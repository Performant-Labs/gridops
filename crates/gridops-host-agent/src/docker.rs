//! Linux Docker execution through the local Docker CLI.
//!
//! Runs GitHub Actions runner containers for fleet placements. Containers are
//! named and labelled after their prepared environment, so every later
//! operation targets exactly the object this agent created. Only images on the
//! operator's allowlist can be prepared. The runner's registration material is
//! written to the container's stdin and never appears in its environment,
//! arguments or `docker inspect`.

use std::{process::Stdio, time::Duration};

use gridops_core::fleet::{
    domain::{Architecture, RuntimeKind},
    ids::{DiagnosticId, UpstreamText},
    protocol::{
        agent::{AgentOperation, LocalResourceIdentity, PrepareEnvironment, RunnerMode, StopMode},
        inventory::ObservedBackend,
        outcomes::{
            DelayedStartExclusionClaim, DiagnosticCode, DiagnosticReference, FailureCode,
            LocalAbsenceClaim, SuccessEvidence,
        },
        primitives::WireTimestamp,
    },
};
use serde::Deserialize;
use tokio::{io::AsyncWriteExt as _, process::Command};
use uuid::Uuid;

use crate::helper::{Bootstrap, HelperRequest, HelperResponse, RuntimeHelper};

const LABEL: &str = "io.gridops.fleet.environment";

pub struct DockerRuntime {
    images: Vec<String>,
    network: Option<String>,
    socket: Option<String>,
}

/// Registration material produced by the reconciler.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Material {
    GithubJit {
        jit_config: String,
    },
    GithubRegistration {
        token: String,
        url: String,
        name: String,
        labels: Vec<String>,
        group: Option<String>,
    },
}

impl DockerRuntime {
    /// `images` is the operator's allowlist; `socket` is the host Docker
    /// socket path to share when a trusted profile asks for it.
    pub const fn new(images: Vec<String>, network: Option<String>, socket: Option<String>) -> Self {
        Self {
            images,
            network,
            socket,
        }
    }

    async fn prepare(
        &self,
        request: &HelperRequest,
        prepare: &PrepareEnvironment,
    ) -> HelperResponse {
        let Some(prepared_environment_id) = request.environment_id else {
            return failed(FailureCode::Configuration, false);
        };
        if prepare.runtime_kind != RuntimeKind::Docker {
            return failed(FailureCode::Unsupported, false);
        }
        let image = String::from(prepare.image.clone());
        if !self.images.contains(&image) {
            return failed(FailureCode::Permission, false);
        }
        if docker(&["image", "inspect", &image]).await.is_err()
            && docker(&["pull", &image]).await.is_err()
        {
            return failed(FailureCode::Dependency, true);
        }
        succeeded(SuccessEvidence::PreparedEnvironment {
            prepared_environment_id,
        })
    }

    async fn start(&self, request: &HelperRequest) -> HelperResponse {
        let (
            Some(environment),
            Some(prepared),
            Some(Bootstrap(material)),
            AgentOperation::StartRunner(start),
        ) = (
            request.environment_id,
            request.prepared.as_ref(),
            request.bootstrap.as_ref(),
            &request.operation,
        )
        else {
            return failed(FailureCode::Configuration, false);
        };
        let Ok(material) = serde_json::from_str::<Material>(material) else {
            return failed(FailureCode::Configuration, false);
        };
        let name = format!("gridops-{environment}");
        // A retried start finds the container it already created.
        if let Ok(id) = docker(&["inspect", "-f", "{{.Id}}", &name]).await {
            return started(start, id);
        }
        let (script, secret, env) = match (&material, prepared.mode) {
            (Material::GithubJit { jit_config }, RunnerMode::Ephemeral) => (
                "set -euo pipefail\nIFS= read -r S\n[ -n \"$S\" ]\nexec /home/runner/run.sh --jitconfig \"$S\"",
                jit_config.clone(),
                Vec::new(),
            ),
            (
                Material::GithubRegistration {
                    token,
                    url,
                    name,
                    labels,
                    group,
                },
                RunnerMode::Persistent,
            ) => (
                "set -euo pipefail\nIFS= read -r S\nargs=(--unattended --url \"$GRIDOPS_REGISTRATION_URL\" --token \"$S\" --name \"$GRIDOPS_RUNNER_NAME\" --labels \"$GRIDOPS_RUNNER_LABELS\" --work _work --replace --disableupdate)\nif [ -n \"${GRIDOPS_RUNNER_GROUP:-}\" ]; then args+=(--runnergroup \"$GRIDOPS_RUNNER_GROUP\"); fi\n./config.sh \"${args[@]}\"\nunset S\nexec ./run.sh",
                token.clone(),
                vec![
                    format!("GRIDOPS_REGISTRATION_URL={url}"),
                    format!("GRIDOPS_RUNNER_NAME={name}"),
                    format!("GRIDOPS_RUNNER_LABELS={}", labels.join(",")),
                    format!("GRIDOPS_RUNNER_GROUP={}", group.clone().unwrap_or_default()),
                ],
            ),
            _ => return failed(FailureCode::Configuration, false),
        };
        let resources = &prepared.resource_request;
        let mut args = vec![
            "create".to_owned(),
            "-i".into(),
            "--name".into(),
            name.clone(),
            "--label".into(),
            format!("{LABEL}={environment}"),
            "--cpus".into(),
            format!("{:.3}", f64::from(resources.cpu_millis()) / 1000.0),
            "--memory".into(),
            format!("{}m", resources.memory_mib()),
            "--pids-limit".into(),
            "4096".into(),
            "--cap-drop".into(),
            "ALL".into(),
            "--security-opt".into(),
            "no-new-privileges:true".into(),
            "--log-opt".into(),
            "max-size=20m".into(),
        ];
        if let Some(network) = prepared
            .configuration
            .network
            .as_ref()
            .map(|network| network.as_str().to_owned())
            .or_else(|| self.network.clone())
        {
            args.extend(["--network".into(), network]);
        }
        if request.docker_socket {
            let Some(socket) = &self.socket else {
                return failed(FailureCode::Permission, false);
            };
            args.extend(["-v".into(), format!("{socket}:/var/run/docker.sock")]);
        }
        for variable in env {
            args.extend(["-e".into(), variable]);
        }
        args.extend([
            String::from(prepared.image.clone()),
            "/bin/bash".into(),
            "-lc".into(),
            script.into(),
        ]);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let Ok(id) = docker(&args).await else {
            return failed(FailureCode::Runtime, true);
        };
        if feed_secret(&id, &secret).await.is_err() {
            let _ = docker(&["rm", "-f", &id]).await;
            return failed(FailureCode::Runtime, true);
        }
        started(start, id)
    }

    async fn owned(&self, resource: &LocalResourceIdentity) -> Option<String> {
        let LocalResourceIdentity::Docker { object_id } = resource else {
            return None;
        };
        let id = object_id.as_str();
        let label = docker(&[
            "inspect",
            "-f",
            &format!("{{{{index .Config.Labels \"{LABEL}\"}}}}"),
            id,
        ])
        .await
        .ok()?;
        (!label.is_empty()).then(|| id.to_owned())
    }
}

impl RuntimeHelper for DockerRuntime {
    async fn call(&self, request: HelperRequest) -> HelperResponse {
        match &request.operation {
            AgentOperation::PrepareEnvironment(prepare) => self.prepare(&request, prepare).await,
            AgentOperation::StartRunner(_) => self.start(&request).await,
            AgentOperation::Inspect { expected_resource } => {
                let running = match self.owned(expected_resource).await {
                    Some(id) => docker(&["inspect", "-f", "{{.State.Running}}", &id])
                        .await
                        .is_ok_and(|state| state == "true"),
                    None => false,
                };
                // A stopped or missing runner is reported as unavailable, which
                // tells the reconciler to clean it up.
                if running {
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
                if let Some(id) = self.owned(expected_resource).await {
                    let result = match mode {
                        StopMode::IdleOnly => docker(&["stop", "-t", "60", &id]).await,
                        StopMode::CancelBusy => docker(&["kill", &id]).await,
                    };
                    if result.is_err() {
                        return failed(FailureCode::Runtime, true);
                    }
                }
                succeeded(SuccessEvidence::Stopped {
                    local_identity: expected_resource.clone(),
                })
            }
            AgentOperation::Cleanup { expected_resource } => {
                if let Some(id) = self.owned(expected_resource).await {
                    if docker(&["rm", "-f", &id]).await.is_err() {
                        return failed(FailureCode::Runtime, true);
                    }
                }
                let LocalResourceIdentity::Docker { object_id } = expected_resource else {
                    return failed(FailureCode::Unsupported, false);
                };
                if docker(&["inspect", object_id.as_str()]).await.is_ok() {
                    return failed(FailureCode::Runtime, true);
                }
                cleaned(expected_resource)
            }
            AgentOperation::InspectHost => succeeded(SuccessEvidence::Inspected {
                local_identity: None,
            }),
            _ => failed(FailureCode::Unsupported, false),
        }
    }

    async fn backends(&self) -> Vec<ObservedBackend> {
        match docker_backend(self).await {
            Ok(backend) => vec![backend],
            Err(error) => {
                tracing::debug!(%error, "Docker backend unavailable");
                Vec::new()
            }
        }
    }
}

async fn docker_backend(runtime: &DockerRuntime) -> anyhow::Result<ObservedBackend> {
    let platform = docker(&["version", "--format", "{{.Server.Os}}/{{.Server.Arch}}"]).await?;
    let architecture = match platform.as_str() {
        "linux/arm64" => Architecture::Arm64,
        "linux/amd64" => Architecture::X64,
        other => anyhow::bail!("unsupported Docker platform {other}"),
    };
    let engine = docker(&["info", "--format", "{{.ID}}"]).await?;
    let mut environments = Vec::new();
    for image in &runtime.images {
        let ready = docker(&["image", "inspect", image]).await.is_ok();
        environments.push(serde_json::json!({
            "reference": image,
            "readiness": if ready { "ready" } else { "preparable" },
            "version": null,
        }));
    }
    let arch = if architecture == Architecture::Arm64 {
        "arm64"
    } else {
        "x64"
    };
    Ok(serde_json::from_value(serde_json::json!({
        "local_key": "docker",
        "known_id": null,
        "domain_local_key": crate::probe::ROOT_DOMAIN,
        "runtime_kind": "docker",
        "execution_os": "linux",
        "architecture": arch,
        "readiness": "ready",
        "reason": null,
        "runtime_identity": format!("docker-{engine}"),
        "capabilities": {
            "schema_version": 1,
            "runtime_kind": "docker",
            "execution_os": "linux",
            "architecture": arch,
            "supported_runners": [
                {"target_kind": "github_repository", "mode": "ephemeral"},
                {"target_kind": "github_repository", "mode": "persistent"},
                {"target_kind": "github_organization", "mode": "ephemeral"},
                {"target_kind": "github_organization", "mode": "persistent"}
            ],
            "environments": environments,
            "resources": {"cpu": "hard_limit", "memory": "hard_limit", "disk": "estimate"},
            "interactive": "unsupported",
            "docker_socket": if runtime.socket.is_some() { "requires_explicit_grant" } else { "unsupported" },
        }
    }))?)
}

/// Starts the created container with its stdin attached, writes the secret
/// line, and detaches once the container is running.
async fn feed_secret(id: &str, secret: &str) -> anyhow::Result<()> {
    let mut child = Command::new("docker")
        .args(["start", "--interactive", id])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("no stdin"))?;
    stdin.write_all(format!("{secret}\n").as_bytes()).await?;
    stdin.flush().await?;
    for _ in 0..50 {
        if docker(&["inspect", "-f", "{{.State.Running}}", id])
            .await
            .is_ok_and(|state| state == "true")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Give the script a moment to read the line before detaching.
    tokio::time::sleep(Duration::from_millis(500)).await;
    drop(stdin);
    let _ = child.start_kill();
    let _ = child.wait().await;
    let running = docker(&["inspect", "-f", "{{.State.Running}}", id]).await?;
    let exit = docker(&["inspect", "-f", "{{.State.ExitCode}}", id]).await?;
    anyhow::ensure!(
        running == "true" || exit == "0",
        "runner container failed to start"
    );
    Ok(())
}

async fn docker(args: &[&str]) -> anyhow::Result<String> {
    let output = tokio::time::timeout(
        Duration::from_mins(10),
        Command::new("docker")
            .args(args)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    anyhow::ensure!(
        output.status.success(),
        "docker {} failed",
        args.first().unwrap_or(&"")
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn started(
    start: &gridops_core::fleet::protocol::agent::StartRunner,
    id: String,
) -> HelperResponse {
    let Ok(object_id) = UpstreamText::new(id) else {
        return failed(FailureCode::Runtime, false);
    };
    succeeded(SuccessEvidence::RunnerStarted {
        prepared_environment_id: start.prepared_environment_id,
        execution_profile_id: start.execution_profile_id,
        profile_revision: start.profile_revision,
        local_identity: LocalResourceIdentity::Docker { object_id },
    })
}

fn cleaned(resource: &LocalResourceIdentity) -> HelperResponse {
    let (Ok(now), Ok(absence), Ok(exclusion)) = (
        WireTimestamp::from_millis(gridops_core::now_millis()),
        DiagnosticId::try_new(Uuid::new_v4()),
        DiagnosticId::try_new(Uuid::new_v4()),
    ) else {
        return failed(FailureCode::Runtime, true);
    };
    succeeded(SuccessEvidence::Cleaned {
        local_identity: resource.clone(),
        local_absence: LocalAbsenceClaim {
            resource: resource.clone(),
            observed_at: now,
            evidence: DiagnosticReference {
                id: absence,
                code: DiagnosticCode::Runtime,
            },
        },
        delayed_start_exclusion: DelayedStartExclusionClaim {
            resource: resource.clone(),
            excluded_at: now,
            evidence: DiagnosticReference {
                id: exclusion,
                code: DiagnosticCode::Runtime,
            },
        },
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
