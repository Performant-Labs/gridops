//! Native GitHub Actions runners directly on the host (macOS or Linux).
//!
//! Each prepared environment gets a private copy of a verified actions-runner
//! release under the agent's state directory. A started runner runs in its own
//! process group with the registration secret on stdin. Its identity is the
//! process ID plus the process start time, so a later stop or cleanup can
//! confirm it's still the same process before signalling it.
//!
//! Native execution shares the host with everything else, so admission only
//! places it where an administrator granted native trust.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context as _, Result};
use gridops_core::fleet::{
    domain::RuntimeKind,
    ids::UpstreamText,
    protocol::{
        agent::{AgentOperation, LocalResourceIdentity, RunnerMode, StopMode},
        inventory::ObservedBackend,
        outcomes::{FailureCode, SuccessEvidence},
    },
};
use serde::Deserialize;
use sha2::Digest as _;
use tokio::{io::AsyncWriteExt as _, process::Command, sync::Mutex};

use crate::helper::{Bootstrap, HelperRequest, HelperResponse, RuntimeHelper};

/// The environment reference native profiles use.
pub const NATIVE_ENVIRONMENT: &str = "native:actions-runner";

pub struct NativeRuntime {
    root: PathBuf,
    release_lock: Mutex<()>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Material {
    GithubJit {
        jit_config: String,
    },
    #[serde(other)]
    Unsupported,
}

impl NativeRuntime {
    /// `root` holds cached releases and per-environment runner copies.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            release_lock: Mutex::new(()),
        }
    }

    fn environment(&self, id: &str) -> PathBuf {
        self.root.join("runners").join(id)
    }

    async fn prepare(&self, request: &HelperRequest) -> HelperResponse {
        let (Some(id), AgentOperation::PrepareEnvironment(prepare)) =
            (request.environment_id, &request.operation)
        else {
            return failed(FailureCode::Configuration, false);
        };
        if prepare.runtime_kind != RuntimeKind::NativeProcess {
            return failed(FailureCode::Unsupported, false);
        }
        if prepare.mode != RunnerMode::Ephemeral {
            return failed(FailureCode::Unsupported, false);
        }
        let release = match self.release().await {
            Ok(release) => release,
            Err(error) => {
                tracing::warn!(error = ?error, "actions-runner release unavailable");
                return failed(FailureCode::Dependency, true);
            }
        };
        if let Err(error) = copy_release(&release, &self.environment(&id.to_string())).await {
            tracing::warn!(error = ?error, "could not stage native runner");
            return failed(FailureCode::Runtime, true);
        }
        succeeded(SuccessEvidence::PreparedEnvironment {
            prepared_environment_id: id,
        })
    }

    async fn start(&self, request: &HelperRequest) -> HelperResponse {
        let (Some(id), Some(Bootstrap(material)), AgentOperation::StartRunner(start)) = (
            request.environment_id,
            request.bootstrap.as_ref(),
            &request.operation,
        ) else {
            return failed(FailureCode::Configuration, false);
        };
        let Ok(Material::GithubJit { jit_config }) = serde_json::from_str::<Material>(material)
        else {
            return failed(FailureCode::Configuration, false);
        };
        let root = self.environment(&id.to_string());
        // A retried start finds the runner it already launched.
        if let Some(identity) = recorded_identity(&root).await {
            if alive(&identity).await {
                return started(start, identity);
            }
        }
        match launch(&root, &jit_config).await {
            Ok(identity) => started(start, identity),
            Err(error) => {
                tracing::warn!(error = ?error, "native runner failed to start");
                failed(FailureCode::Runtime, true)
            }
        }
    }

    async fn release(&self) -> Result<PathBuf> {
        let _guard = self.release_lock.lock().await;
        let releases = self.root.join("releases");
        let (version, url, digest) = latest_release().await?;
        let target = releases.join(&version);
        let entry = if cfg!(windows) { "run.cmd" } else { "run.sh" };
        if tokio::fs::try_exists(target.join(entry))
            .await
            .unwrap_or(false)
        {
            return Ok(target);
        }
        download(&url, digest.as_deref(), &target).await?;
        Ok(target)
    }
}

impl RuntimeHelper for NativeRuntime {
    async fn call(&self, request: HelperRequest) -> HelperResponse {
        match &request.operation {
            AgentOperation::PrepareEnvironment(_) => self.prepare(&request).await,
            AgentOperation::StartRunner(_) => self.start(&request).await,
            AgentOperation::Inspect { expected_resource } => {
                if alive(expected_resource).await {
                    succeeded(SuccessEvidence::Inspected {
                        local_identity: Some(expected_resource.clone()),
                    })
                } else {
                    failed(FailureCode::ResourceUnavailable, false)
                }
            }
            AgentOperation::Stop {
                expected_resource,
                mode,
            } => {
                if alive(expected_resource).await {
                    signal(expected_resource, matches!(mode, StopMode::CancelBusy)).await;
                }
                succeeded(SuccessEvidence::Stopped {
                    local_identity: expected_resource.clone(),
                })
            }
            AgentOperation::Cleanup { expected_resource } => {
                if alive(expected_resource).await {
                    signal(expected_resource, true).await;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    if alive(expected_resource).await {
                        return failed(FailureCode::Runtime, true);
                    }
                }
                if let Some(id) = request.environment_id {
                    let _ = tokio::fs::remove_dir_all(self.environment(&id.to_string())).await;
                }
                crate::docker::cleaned(expected_resource)
            }
            _ => failed(FailureCode::Unsupported, false),
        }
    }

    async fn backends(&self) -> Vec<ObservedBackend> {
        let os = if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(target_os = "linux") {
            "linux"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else {
            return Vec::new();
        };
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        };
        serde_json::from_value(serde_json::json!([{
            "local_key": "native",
            "known_id": null,
            "domain_local_key": crate::probe::ROOT_DOMAIN,
            "runtime_kind": "native_process",
            "execution_os": os,
            "architecture": arch,
            "readiness": "ready",
            "reason": null,
            "runtime_identity": format!("native-{os}-{arch}"),
            "capabilities": {
                "schema_version": 1,
                "runtime_kind": "native_process",
                "execution_os": os,
                "architecture": arch,
                "supported_runners": [
                    {"target_kind": "github_repository", "mode": "ephemeral"},
                    {"target_kind": "github_organization", "mode": "ephemeral"}
                ],
                "environments": [{"reference": NATIVE_ENVIRONMENT, "readiness": "preparable", "version": null}],
                "resources": {"cpu": "reservation", "memory": "reservation", "disk": "estimate"},
                "interactive": "unsupported",
                "docker_socket": "unsupported",
            }
        }]))
        .unwrap_or_default()
    }
}

/// The shell that reads the secret line from stdin and execs the runner, in
/// its own process group (Unix) or without a console window (Windows).
#[cfg(not(windows))]
fn runner_command() -> Command {
    let mut command = Command::new("/bin/bash");
    command.args([
        "-c",
        "set -euo pipefail\nIFS= read -r S\n[ -n \"$S\" ]\nexec ./run.sh --jitconfig \"$S\"",
    ]);
    #[cfg(unix)]
    command.process_group(0);
    command
}

#[cfg(windows)]
fn runner_command() -> Command {
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("powershell");
    command
        // Job steps run as PowerShell scripts, which Windows blocks by
        // default. Like GitHub's hosted runners, allow them for this runner's
        // process tree only; the machine's own policy is unchanged.
        .env("PSExecutionPolicyPreference", "Bypass")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "$s=[Console]::In.ReadLine(); if (-not $s) { exit 1 }; & .\\run.cmd --jitconfig $s; exit $LASTEXITCODE",
        ])
        .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    command
}

/// Starts the runner in its own process group and feeds it the secret.
async fn launch(root: &Path, jit_config: &str) -> Result<LocalResourceIdentity> {
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("gridops-runner.log"))?;
    let mut command = runner_command();
    command
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    let mut child = command.spawn()?;
    let pid = child.id().context("runner has no process ID")?;
    let mut stdin = child.stdin.take().context("runner stdin unavailable")?;
    stdin
        .write_all(format!("{jit_config}\n").as_bytes())
        .await?;
    stdin.shutdown().await?;
    drop(stdin);
    // Reap the child when it exits so it doesn't linger as a zombie.
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    let marker = start_time(pid).await.context("runner exited immediately")?;
    let identity = LocalResourceIdentity::NativeProcess {
        boot_id: UpstreamText::new(crate::probe::boot_id())?,
        process_id: gridops_core::fleet::protocol::primitives::BoundedInteger::new(u64::from(pid))?,
        start_marker: UpstreamText::new(marker)?,
    };
    tokio::fs::write(
        root.join(".gridops-identity"),
        serde_json::to_vec(&identity)?,
    )
    .await?;
    Ok(identity)
}

async fn recorded_identity(root: &Path) -> Option<LocalResourceIdentity> {
    let bytes = tokio::fs::read(root.join(".gridops-identity")).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Whether the process is the one this agent started: same boot, same PID,
/// same start time.
async fn alive(identity: &LocalResourceIdentity) -> bool {
    let LocalResourceIdentity::NativeProcess {
        boot_id,
        process_id,
        start_marker,
    } = identity
    else {
        return false;
    };
    let Ok(pid) = u32::try_from(process_id.get()) else {
        return false;
    };
    boot_id.as_str() == crate::probe::boot_id()
        && start_time(pid).await.as_deref() == Some(start_marker.as_str())
}

async fn start_time(pid: u32) -> Option<String> {
    #[cfg(windows)]
    let script = format!(
        "(Get-Process -Id {pid} -ErrorAction Stop).StartTime.ToUniversalTime().ToString('o')"
    );
    #[cfg(windows)]
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script.as_str()])
        .output()
        .await
        .ok()?;
    #[cfg(not(windows))]
    let output = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .await
        .ok()?;
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (output.status.success() && !value.is_empty()).then_some(value)
}

/// Stops the runner's whole process tree.
#[cfg(windows)]
async fn signal(identity: &LocalResourceIdentity, kill: bool) {
    if let LocalResourceIdentity::NativeProcess { process_id, .. } = identity {
        let pid = process_id.get().to_string();
        let mut args = vec!["/PID", pid.as_str(), "/T"];
        if kill {
            args.push("/F");
        }
        let _ = Command::new("taskkill").args(args).status().await;
    }
}

/// Signals the runner's whole process group.
#[cfg(not(windows))]
async fn signal(identity: &LocalResourceIdentity, kill: bool) {
    if let LocalResourceIdentity::NativeProcess { process_id, .. } = identity {
        let _ = Command::new("kill")
            .args([
                if kill { "-KILL" } else { "-TERM" },
                "--",
                &format!("-{}", process_id.get()),
            ])
            .status()
            .await;
    }
}

async fn latest_release() -> Result<(String, String, Option<String>)> {
    let client = reqwest::Client::builder()
        .user_agent("GridOps host agent")
        .timeout(Duration::from_secs(30))
        .build()?;
    let release: serde_json::Value = client
        .get("https://api.github.com/repos/actions/runner/releases/latest")
        .header("accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let version = release["tag_name"]
        .as_str()
        .context("release tag")?
        .trim_start_matches('v')
        .to_owned();
    anyhow::ensure!(
        !version.is_empty()
            && version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')),
        "release tag is malformed"
    );
    let (platform, extension) = if cfg!(target_os = "macos") {
        ("osx", "tar.gz")
    } else if cfg!(target_os = "windows") {
        ("win", "zip")
    } else {
        ("linux", "tar.gz")
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let name = format!("actions-runner-{platform}-{arch}-{version}.{extension}");
    let asset = release["assets"]
        .as_array()
        .and_then(|assets| assets.iter().find(|asset| asset["name"] == name.as_str()))
        .with_context(|| format!("release has no {name}"))?;
    Ok((
        version,
        asset["browser_download_url"]
            .as_str()
            .context("asset URL")?
            .to_owned(),
        asset["digest"]
            .as_str()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .map(str::to_owned),
    ))
}

async fn download(url: &str, digest: Option<&str>, target: &Path) -> Result<()> {
    let staging = target.with_extension("partial");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging).await?;
    let archive = reqwest::Client::builder()
        .user_agent("GridOps host agent")
        .timeout(Duration::from_mins(10))
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    if let Some(expected) = digest {
        let actual: String =
            sha2::Sha256::digest(&archive)
                .iter()
                .fold(String::new(), |mut out, byte| {
                    use std::fmt::Write as _;
                    let _ = write!(out, "{byte:02x}");
                    out
                });
        anyhow::ensure!(
            actual.eq_ignore_ascii_case(expected),
            "release digest mismatch"
        );
    }
    // Windows 10 and later ship bsdtar, which also extracts the zip release.
    let tarball = staging.join(if cfg!(windows) {
        "actions-runner.zip"
    } else {
        "actions-runner.tar.gz"
    });
    tokio::fs::write(&tarball, &archive).await?;
    let status = Command::new("tar")
        .arg("-xf")
        .arg(&tarball)
        .arg("-C")
        .arg(&staging)
        .status()
        .await?;
    anyhow::ensure!(status.success(), "could not extract the release");
    tokio::fs::remove_file(&tarball).await?;
    let _ = tokio::fs::remove_dir_all(target).await;
    tokio::fs::rename(&staging, target).await?;
    Ok(())
}

/// Copies the release into the environment.
#[cfg(windows)]
async fn copy_release(release: &Path, root: &Path) -> Result<()> {
    fn copy(from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let target = to.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }
    let _ = tokio::fs::remove_dir_all(root).await;
    let (from, to) = (release.to_path_buf(), root.to_path_buf());
    tokio::task::spawn_blocking(move || copy(&from, &to)).await??;
    Ok(())
}

/// Copies the release into the environment (an APFS clone on macOS).
#[cfg(not(windows))]
async fn copy_release(release: &Path, root: &Path) -> Result<()> {
    let _ = tokio::fs::remove_dir_all(root).await;
    if let Some(parent) = root.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let clone = if cfg!(target_os = "macos") {
        "-Rc"
    } else {
        "-R"
    };
    let status = Command::new("cp")
        .arg(clone)
        .arg(release)
        .arg(root)
        .status()
        .await?;
    if !status.success() {
        let status = Command::new("cp")
            .arg("-R")
            .arg(release)
            .arg(root)
            .status()
            .await?;
        anyhow::ensure!(status.success(), "could not stage the runner");
    }
    Ok(())
}

fn started(
    start: &gridops_core::fleet::protocol::agent::StartRunner,
    local_identity: LocalResourceIdentity,
) -> HelperResponse {
    succeeded(SuccessEvidence::RunnerStarted {
        prepared_environment_id: start.prepared_environment_id,
        execution_profile_id: start.execution_profile_id,
        profile_revision: start.profile_revision,
        local_identity,
    })
}

fn succeeded(evidence: SuccessEvidence) -> HelperResponse {
    HelperResponse::Succeeded {
        evidence: Box::new(evidence),
    }
}

const fn failed(code: FailureCode, retryable: bool) -> HelperResponse {
    HelperResponse::Failed { code, retryable }
}
