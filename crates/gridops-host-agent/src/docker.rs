//! Linux Docker execution through the local Docker CLI.
//!
//! Runs GitHub Actions and GitLab runner containers for fleet placements.
//! Containers are named and labelled after their prepared environment, so
//! every later operation targets exactly the object this agent created.
//! `GridOps` chooses the image: each platform names it. The runner's
//! registration material is written to the container's stdin and never
//! appears in its environment, arguments or `docker inspect`.

use std::{
    process::Stdio,
    time::{Duration, Instant},
};

use gridops_core::{
    fleet::{
        domain::{Architecture, RuntimeKind},
        ids::{DiagnosticId, PreparedEnvironmentId, UpstreamText},
        protocol::{
            agent::{
                AgentOperation, GitlabRunnerMaterial, LocalResourceIdentity, PrepareEnvironment,
                RunnerMode, StartRunner, StopMode,
            },
            inventory::ObservedBackend,
            outcomes::{
                DelayedStartExclusionClaim, DiagnosticCode, DiagnosticReference, FailureCode,
                LocalAbsenceClaim, SuccessEvidence,
            },
            primitives::WireTimestamp,
        },
    },
    provisioning::{
        GITLAB_EXIT_IDLE_EXPIRED, GITLAB_EXIT_REGISTRATION_REFUSED, GITLAB_READY_MARKER,
        GITLAB_START_READY_TIMEOUT_SECS, gitlab_runner_env, gitlab_runner_script,
        instrumented_runner_script, valid_gitlab_url,
    },
};
use serde::Deserialize;
use tokio::{io::AsyncWriteExt as _, process::Command};
use uuid::Uuid;

use crate::helper::{
    Bootstrap, ChildOutput, HelperRequest, HelperResponse, RunnerOutput, RuntimeHelper,
    StartRemains,
};

const LABEL: &str = "io.gridops.fleet.environment";
/// How many times, 3 s apart, an interrupted start waits for Docker to answer.
const DOCKER_WAIT_ATTEMPTS: u32 = 20;

pub struct DockerRuntime {
    network: Option<String>,
    socket: Option<String>,
    /// The last probe failure, so a change is logged once, not every probe.
    last_problem: std::sync::Mutex<Option<String>>,
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
    /// `socket` is the host Docker socket path to share when a trusted
    /// profile asks for it.
    pub fn new(network: Option<String>, socket: Option<String>) -> Self {
        Self {
            network,
            socket,
            last_problem: std::sync::Mutex::new(None),
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
        // GitLab's Docker executor runs every job on the host's daemon, so a
        // GitLab profile that doesn't share the Docker socket can run none.
        if prepare.target.is_gitlab() && !prepare.configuration.requires_docker_socket {
            return failed(FailureCode::Configuration, false);
        }
        let image = String::from(prepare.image.clone());
        // Pull every time, so a tag that moved (`latest`, a rebuilt CI image)
        // is picked up. The copy already here serves if the registry can't be
        // reached.
        if docker(&["pull", &image]).await.is_err()
            && docker(&["image", "inspect", &image]).await.is_err()
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
        if prepared.target.is_gitlab() {
            return self
                .start_gitlab(request, environment, prepared, material, start)
                .await;
        }
        let Ok(material) = serde_json::from_str::<Material>(material) else {
            return failed(FailureCode::Configuration, false);
        };
        let name = format!("gridops-{environment}");
        // A retried start finds the container it already created.
        if let Ok(id) = docker(&["inspect", "-f", "{{.Id}}", &name]).await {
            return started(start, id);
        }
        let (script, secret, mut env) = match (&material, prepared.mode) {
            (Material::GithubJit { jit_config }, RunnerMode::Ephemeral) => (
                instrumented_runner_script(
                    "set -euo pipefail\nIFS= read -r S\n[ -n \"$S\" ]\nrunner_args=(--jitconfig \"$S\")\nunset S",
                    "./run.sh \"${runner_args[@]}\"",
                ),
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
                instrumented_runner_script(
                    "set -euo pipefail\nIFS= read -r S\nargs=(--unattended --url \"$GRIDOPS_REGISTRATION_URL\" --token \"$S\" --name \"$GRIDOPS_RUNNER_NAME\" --labels \"$GRIDOPS_RUNNER_LABELS\" --work \"${GRIDOPS_RUNNER_WORK:-_work}\" --replace --disableupdate)\nif [ -n \"${GRIDOPS_RUNNER_GROUP:-}\" ]; then args+=(--runnergroup \"$GRIDOPS_RUNNER_GROUP\"); fi\n./config.sh \"${args[@]}\"\nunset S",
                    "./run.sh",
                ),
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
        // We bypass the runner image's own entrypoint below (which otherwise exports
        // this itself) because we run its registration/run scripts as root.
        env.push("RUNNER_ALLOW_RUNASROOT=1".into());
        let mut args = self.create_args(&name, environment, prepared);
        if request.docker_socket {
            let Some(socket) = &self.socket else {
                return failed(FailureCode::Permission, false);
            };
            args.extend(["-v".into(), format!("{socket}:/var/run/docker.sock")]);
            // Jobs now drive the host's daemon, which resolves bind-mount paths on the host. Give the
            // runner a work folder with the same absolute path in both places (the server put the same
            // path in the runner's registration).
            let work = gridops_core::fleet::shared_work_folder(environment);
            if let Err(error) = create_shared_work_folder(
                std::path::Path::new(gridops_core::fleet::SHARED_WORK_ROOT),
                std::path::Path::new(&work),
            ) {
                tracing::warn!("Could not create the runner work folder {work}: {error}");
                return failed(FailureCode::Configuration, false);
            }
            args.extend(["-v".into(), format!("{work}:{work}")]);
            env.push(format!("GRIDOPS_RUNNER_WORK={work}"));
        }
        for variable in env {
            args.extend(["-e".into(), variable]);
        }
        args.extend([
            String::from(prepared.image.clone()),
            "/bin/bash".into(),
            "-lc".into(),
            script,
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

    /// `docker create` up to the volumes, environment and command: what every
    /// runner container gets, whatever its platform.
    fn create_args(
        &self,
        name: &str,
        environment: PreparedEnvironmentId,
        prepared: &PrepareEnvironment,
    ) -> Vec<String> {
        let resources = &prepared.resource_request;
        let mut args = vec![
            "create".to_owned(),
            "-i".into(),
            "--name".into(),
            name.to_owned(),
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
            // Dropping ALL also strips root's usual exemption from capability checks, so
            // chown/chmod calls the runner's own setup actions make (e.g. actions/setup-node)
            // fail even as uid 0 without these restored explicitly.
            "--cap-add".into(),
            "CHOWN".into(),
            "--cap-add".into(),
            "DAC_OVERRIDE".into(),
            "--cap-add".into(),
            "FOWNER".into(),
            "--cap-add".into(),
            "SETUID".into(),
            "--cap-add".into(),
            "SETGID".into(),
            "--security-opt".into(),
            "no-new-privileges:true".into(),
            "--log-opt".into(),
            "max-size=20m".into(),
            // An empty entrypoint runs the Cmd below as given. myoung34/github-runner's
            // `/entrypoint.sh` would otherwise take it as arguments and first run a
            // registration from env vars GridOps never sets (bootstrap secrets are piped
            // over stdin instead), which fails before our Cmd runs. GitHub's own image has
            // no entrypoint, so this changes nothing there. gitlab/gitlab-runner's
            // `/entrypoint` would likewise hand the Cmd to gitlab-runner as its arguments.
            "--entrypoint".into(),
            String::new(),
            // Docker's own init reaps the job's orphaned processes, whatever the image
            // ships. dumb-init would do the same job, but GitHub's own image doesn't carry
            // that binary, so PID 1 duty falls to Docker's --init instead.
            "--init".into(),
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
        args
    }

    /// The GitLab branch of Start: `gitlab-runner` with the shared GitLab
    /// runner script, the token on its stdin, then a bounded wait until the
    /// script says GitLab accepted the token or the container ends.
    async fn start_gitlab(
        &self,
        request: &HelperRequest,
        environment: PreparedEnvironmentId,
        prepared: &PrepareEnvironment,
        material: &str,
        start: &StartRunner,
    ) -> HelperResponse {
        let Ok(GitlabRunnerMaterial::GitlabRunner {
            token,
            url,
            wait_timeout_seconds,
        }) = serde_json::from_str::<GitlabRunnerMaterial>(material)
        else {
            return failed(FailureCode::Configuration, false);
        };
        // A GitLab runner takes one job; the URL is checked here as the
        // manager checks it, so a bad one never reaches the verify call.
        if prepared.mode != RunnerMode::Ephemeral || !valid_gitlab_url(&url) {
            return failed(FailureCode::Configuration, false);
        }
        // The Docker executor needs the host's daemon (Prepare refuses a
        // GitLab profile without the socket).
        if !request.docker_socket {
            return failed(FailureCode::Configuration, false);
        }
        let Some(socket) = &self.socket else {
            return failed(FailureCode::Permission, false);
        };
        let name = format!("gridops-{environment}");
        // A retried start finds the container it already created.
        if let Ok(id) = docker(&["inspect", "-f", "{{.Id}}", &name]).await {
            return started(start, id);
        }
        let mut args = self.create_args(&name, environment, prepared);
        // The Docker executor keeps each job's files in its own containers and
        // volumes on the host's daemon, so unlike GitHub's runner this one
        // needs no shared work folder.
        args.extend(["-v".into(), format!("{socket}:/var/run/docker.sock")]);
        for variable in gitlab_runner_env(&url, wait_timeout_seconds) {
            args.extend(["-e".into(), variable]);
        }
        args.extend([
            String::from(prepared.image.clone()),
            "/bin/bash".into(),
            "-c".into(),
            gitlab_runner_script(),
        ]);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let Ok(id) = docker(&args).await else {
            return failed(FailureCode::Runtime, true);
        };
        let ending = start_gitlab_runner(&id, &token).await;
        if ending == GitlabStart::Ready {
            return started(start, id);
        }
        // Only a successful Start binds the container to its environment, so
        // nothing later could remove it: remove it now.
        let _ = docker(&["rm", "-f", &id]).await;
        match ending {
            GitlabStart::Exited(GITLAB_EXIT_REGISTRATION_REFUSED) => failed_because(
                FailureCode::Configuration,
                false,
                diagnostic(DiagnosticCode::RegistrationRefused),
            ),
            _ => failed(FailureCode::Runtime, true),
        }
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

impl DockerRuntime {
    /// Why Docker can't be offered on this host, if it can't.
    pub async fn problem(&self) -> Option<String> {
        docker_backend(self)
            .await
            .err()
            .map(|error| format!("{error:#}"))
    }
}

impl RuntimeHelper for DockerRuntime {
    async fn call(&self, request: HelperRequest) -> HelperResponse {
        match &request.operation {
            AgentOperation::PrepareEnvironment(prepare) => self.prepare(&request, prepare).await,
            AgentOperation::StartRunner(_) => self.start(&request).await,
            AgentOperation::Inspect { expected_resource } => {
                let LocalResourceIdentity::Docker { object_id } = expected_resource else {
                    return failed(FailureCode::Unsupported, false);
                };
                let state = docker(&[
                    "inspect",
                    "-f",
                    &format!("{{{{.State.Running}}}} {{{{index .Config.Labels \"{LABEL}\"}}}}"),
                    object_id.as_str(),
                ])
                .await;
                // A stopped or missing runner is reported as unavailable, which
                // tells the reconciler to retire it and clean it up. Docker
                // failing to answer says nothing about the runner, so it must
                // not read as an exit: that would kill a runner mid-job.
                match state {
                    // Running, and labelled as this agent's environment.
                    Ok(state)
                        if state.split_once(' ').is_some_and(|(running, label)| {
                            running == "true" && !label.is_empty()
                        }) =>
                    {
                        succeeded(SuccessEvidence::Inspected {
                            local_identity: Some(expected_resource.clone()),
                        })
                    }
                    // An exited GitLab runner says why it ended when the
                    // reason is its own: idle expiry or a refused token.
                    Ok(_) => failed_because(
                        FailureCode::ResourceUnavailable,
                        false,
                        match &request.prepared {
                            Some(prepared) if prepared.target.is_gitlab() => {
                                gitlab_exit_diagnostic(object_id.as_str()).await
                            }
                            _ => None,
                        },
                    ),
                    Err(error) if missing(&error) => {
                        failed(FailureCode::ResourceUnavailable, false)
                    }
                    Err(_) => failed(FailureCode::Runtime, true),
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
                    // Read the shared work folder and image before the container goes.
                    let leftover = shared_work_leftover(&id).await;
                    if docker(&["rm", "-f", &id]).await.is_err() {
                        return failed(FailureCode::Runtime, true);
                    }
                    if let Some((folder, image)) = leftover {
                        remove_shared_work_folder(&folder, &image).await;
                    }
                }
                let LocalResourceIdentity::Docker { object_id } = expected_resource else {
                    return failed(FailureCode::Unsupported, false);
                };
                match docker(&["inspect", object_id.as_str()]).await {
                    Err(error) if missing(&error) => cleaned(expected_resource),
                    _ => failed(FailureCode::Runtime, true),
                }
            }
            AgentOperation::InspectHost => succeeded(SuccessEvidence::Inspected {
                local_identity: None,
            }),
            _ => failed(FailureCode::Unsupported, false),
        }
    }

    async fn interrupted_start(&self, request: HelperRequest, _rebooted: bool) -> StartRemains {
        let Some(environment) = request.environment_id else {
            return StartRemains::Unknown;
        };
        let name = format!("gridops-{environment}");
        // A container outlives a reboot, but Docker may still be starting
        // after one, and until it answers nothing can be said.
        for _ in 0..DOCKER_WAIT_ATTEMPTS {
            match docker(&["inspect", "-f", "{{.Id}}", &name]).await {
                Ok(id) => {
                    return UpstreamText::new(id).map_or(StartRemains::Unknown, |object_id| {
                        StartRemains::Runner(LocalResourceIdentity::Docker { object_id })
                    });
                }
                Err(error) if missing(&error) => return StartRemains::Nothing,
                Err(_) => tokio::time::sleep(Duration::from_secs(3)).await,
            }
        }
        StartRemains::Unknown
    }

    /// `docker logs --follow` of a container this agent created, with
    /// Docker's timestamps, as the manager's live logs have them. Only stdout:
    /// it's where the job's console pages are mirrored, and one stream keeps
    /// the output identical every time it's read.
    async fn output(&self, resource: LocalResourceIdentity) -> Option<RunnerOutput> {
        let id = self.owned(&resource).await?;
        ChildOutput::spawn(Command::new("docker").args(["logs", "--follow", "--timestamps", &id]))
    }

    async fn backends(&self) -> Vec<ObservedBackend> {
        let result = docker_backend(self).await;
        let problem = result.as_ref().err().map(|error| format!("{error:#}"));
        if let Ok(mut last) = self.last_problem.lock() {
            if *last != problem {
                match &problem {
                    Some(problem) => tracing::warn!("Docker jobs unavailable: {problem}"),
                    None if last.is_some() => tracing::info!("Docker is available again"),
                    None => {}
                }
                last.clone_from(&problem);
            }
        }
        match result {
            Ok(backend) => vec![backend],
            // Reported as unavailable, so GridOps shows Docker and why it
            // can't be used rather than nothing at all.
            Err(_) => unavailable_backend(self).into_iter().collect(),
        }
    }
}

fn unavailable_backend(runtime: &DockerRuntime) -> Option<ObservedBackend> {
    let arch = if crate::probe::architecture() == Architecture::Arm64 {
        "arm64"
    } else {
        "x64"
    };
    serde_json::from_value(serde_json::json!({
        "local_key": "docker",
        "known_id": null,
        "domain_local_key": crate::probe::ROOT_DOMAIN,
        "runtime_kind": "docker",
        "execution_os": "linux",
        "architecture": arch,
        "readiness": "unavailable",
        "reason": "docker_unreachable",
        "runtime_identity": null,
        "capabilities": {
            "schema_version": 1,
            "runtime_kind": "docker",
            "execution_os": "linux",
            "architecture": arch,
            "supported_runners": [],
            "environments": [],
            "resources": {"cpu": "hard_limit", "memory": "hard_limit", "disk": "estimate"},
            "interactive": "unsupported",
            "docker_socket": if runtime.socket.is_some() { "requires_explicit_grant" } else { "unsupported" },
        }
    }))
    .ok()
}

async fn docker_backend(runtime: &DockerRuntime) -> anyhow::Result<ObservedBackend> {
    let platform = docker(&["version", "--format", "{{.Server.Os}}/{{.Server.Arch}}"]).await?;
    let architecture = match platform.as_str() {
        "linux/arm64" => Architecture::Arm64,
        "linux/amd64" => Architecture::X64,
        other => anyhow::bail!(
            "Docker reports {other:?}; GridOps runs Linux containers, so switch Docker Desktop to Linux containers"
        ),
    };
    let engine = docker(&["info", "--format", "{{.ID}}"]).await?;
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
                {"target_kind": "github_organization", "mode": "persistent"},
                // GitLab runners take one job each (D6).
                {"target_kind": "gitlab_project", "mode": "ephemeral"},
                {"target_kind": "gitlab_group", "mode": "ephemeral"}
            ],
            "environments": [],
            // Preparing pulls whatever image the platform names.
            "any_image": true,
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

/// How a GitLab runner's Start ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitlabStart {
    /// The script printed `GITLAB_READY_MARKER`: GitLab accepted the token.
    Ready,
    /// The container exited before that, with this status.
    Exited(i32),
    /// Neither within `GITLAB_START_READY_TIMEOUT_SECS`.
    NotReady,
}

/// Starts the created GitLab runner container with its stdin attached,
/// writes the token line, and waits, at most
/// `GITLAB_START_READY_TIMEOUT_SECS`, until the script says it is ready or
/// the container exits. Stdin stays attached until then, so the script has
/// read the token whatever the outcome. A refusal can end the container
/// before the token is even written: its exit status is read all the same.
async fn start_gitlab_runner(id: &str, token: &str) -> GitlabStart {
    let deadline = Instant::now() + Duration::from_secs(GITLAB_START_READY_TIMEOUT_SECS);
    let Ok(mut child) = Command::new("docker")
        .args(["start", "--interactive", id])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    else {
        return GitlabStart::NotReady;
    };
    let mut stdin = child.stdin.take();
    if let Some(input) = stdin.as_mut() {
        // A write that fails (the container already ended) is told apart by
        // the exit status below.
        let _ = input.write_all(format!("{token}\n").as_bytes()).await;
        let _ = input.flush().await;
    }
    let ending = loop {
        let state = docker(&["inspect", "-f", "{{.State.Status}} {{.State.ExitCode}}", id]).await;
        let ready = docker(&["logs", id]).await.is_ok_and(|output| {
            output
                .lines()
                .any(|line| line.trim_end() == GITLAB_READY_MARKER)
        });
        if ready {
            break GitlabStart::Ready;
        }
        if let Ok(state) = state {
            if let Some(("exited" | "dead", code)) = state.split_once(' ') {
                break GitlabStart::Exited(code.trim().parse().unwrap_or(-1));
            }
        }
        if Instant::now() >= deadline {
            break GitlabStart::NotReady;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    drop(stdin);
    let _ = child.start_kill();
    let _ = child.wait().await;
    ending
}

/// Why an exited GitLab runner ended, when the reason is the runner's own:
/// idle expiry, or a token GitLab refused after the Start (D147-2).
async fn gitlab_exit_diagnostic(id: &str) -> Option<DiagnosticReference> {
    let state = docker(&[
        "inspect",
        "-f",
        "{{.State.Running}} {{.State.ExitCode}}",
        id,
    ])
    .await
    .ok()?;
    let ("false", code) = state.split_once(' ')? else {
        return None;
    };
    match code.trim().parse::<i32>().ok()? {
        GITLAB_EXIT_IDLE_EXPIRED => diagnostic(DiagnosticCode::IdleExpired),
        GITLAB_EXIT_REGISTRATION_REFUSED => diagnostic(DiagnosticCode::RegistrationRefused),
        _ => None,
    }
}

/// A diagnostic of `code`, with a fresh id as `cleaned` makes its own.
fn diagnostic(code: DiagnosticCode) -> Option<DiagnosticReference> {
    DiagnosticId::try_new(Uuid::new_v4())
        .ok()
        .map(|id| DiagnosticReference { id, code })
}

/// Create a runner's shared work folder under `root`.
///
/// If the Docker daemon is ever asked to bind-mount a path that does not exist yet, it creates it
/// as root, and the agent can then no longer create folders inside it. That happens when a runner
/// that was started without the shared folder (an older agent) is told to use one. The agent owns
/// the directory that holds `root`, so it can move such a root-owned `root` aside and start a fresh
/// one. Nothing of ours can be inside it: the agent could not have created anything there.
fn create_shared_work_folder(
    root: &std::path::Path,
    work: &std::path::Path,
) -> std::io::Result<()> {
    match std::fs::create_dir_all(work) {
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs());
            let mut aside = root.as_os_str().to_owned();
            aside.push(format!(".not-ours-{stamp}"));
            std::fs::rename(root, &aside)?;
            tracing::warn!(
                "Moved the unwritable work root {} aside to {}",
                root.display(),
                std::path::Path::new(&aside).display()
            );
            std::fs::create_dir_all(work)
        }
        other => other,
    }
}

/// The image and shared work folder of a runner container, when it has one.
async fn shared_work_leftover(id: &str) -> Option<(String, String)> {
    let output = docker(&[
        "inspect",
        "-f",
        "{{.Config.Image}}{{\"\\n\"}}{{range .Mounts}}{{.Source}}{{\"\\n\"}}{{end}}",
        id,
    ])
    .await
    .ok()?;
    parse_shared_work_leftover(&output)
}

/// First line is the image; any later line under the shared work root is the folder.
fn parse_shared_work_leftover(inspect: &str) -> Option<(String, String)> {
    let mut lines = inspect.lines();
    let image = lines.next()?.trim();
    let prefix = format!("{}/", gridops_core::fleet::SHARED_WORK_ROOT);
    let folder = lines.map(str::trim).find(|line| {
        line.strip_prefix(prefix.as_str())
            .is_some_and(|rest| !rest.is_empty() && !rest.contains('/') && rest != "..")
    })?;
    (!image.is_empty()).then(|| (folder.to_owned(), image.to_owned()))
}

/// Empty a runner's shared work folder, then remove it. The runner wrote as root, so the files are
/// not the agent's to delete: a throwaway container from the runner's own image (already pulled) does
/// it. Best effort: a failure only leaves disk behind and must not fail the runner's cleanup.
async fn remove_shared_work_folder(folder: &str, image: &str) {
    let mount = format!("{folder}:/w");
    let emptied = docker(&[
        "run",
        "--rm",
        "--pull=never",
        "--network",
        "none",
        "--cap-drop",
        "ALL",
        "--cap-add",
        "DAC_OVERRIDE",
        "--cap-add",
        "FOWNER",
        "--entrypoint",
        "/bin/sh",
        "-v",
        &mount,
        image,
        "-c",
        "find /w -mindepth 1 -delete",
    ])
    .await
    .is_ok();
    if !emptied || std::fs::remove_dir(folder).is_err() {
        tracing::warn!("Could not remove the runner work folder {folder}");
    }
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
    .await?
    .map_err(|error| {
        anyhow::anyhow!(
            "couldn't run `docker` ({error}); is Docker installed and on this user's PATH?"
        )
    })?;
    anyhow::ensure!(
        output.status.success(),
        "`docker {}` failed: {}",
        args.first().unwrap_or(&""),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Whether a failed `docker` call failed because the object doesn't exist.
fn missing(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("no such object") || message.contains("no such container")
}

fn started(start: &StartRunner, id: String) -> HelperResponse {
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

pub(crate) fn cleaned(resource: &LocalResourceIdentity) -> HelperResponse {
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
    failed_because(code, retryable, None)
}

const fn failed_because(
    code: FailureCode,
    retryable: bool,
    diagnostic: Option<DiagnosticReference>,
) -> HelperResponse {
    HelperResponse::Failed {
        code,
        retryable,
        diagnostic,
    }
}

#[cfg(test)]
mod externals_tests;

#[cfg(test)]
mod shared_work_folder_tests {
    use super::{create_shared_work_folder, parse_shared_folders_leftover};

    #[test]
    fn finds_the_image_and_the_shared_folder() {
        let inspect = "harbor.example/pl-runner:1.70.4\n/var/run/docker.sock\n/var/lib/gridops-agent/work/0a1b\n";
        assert_eq!(
            parse_shared_folders_leftover(inspect),
            Some((
                "harbor.example/pl-runner:1.70.4".to_owned(),
                vec!["/var/lib/gridops-agent/work/0a1b".to_owned()]
            ))
        );
    }

    /// #126 item 5: Cleanup finds the runner root beside the work folder, and never the externals
    /// cache, which every runner of the image shares.
    #[test]
    fn finds_the_work_folder_and_the_runner_root_but_never_the_externals_cache() {
        let inspect = "harbor.example/pl-runner:1.70.4\n/var/run/docker.sock\n/var/lib/gridops-agent/work/0a1b\n/var/lib/gridops-agent/runners/0a1b\n/var/lib/gridops-agent/externals/2f529cfa\n";
        let found = parse_shared_folders_leftover(inspect).map(|(image, mut folders)| {
            folders.sort();
            (image, folders)
        });
        assert_eq!(
            found,
            Some((
                "harbor.example/pl-runner:1.70.4".to_owned(),
                vec![
                    "/var/lib/gridops-agent/runners/0a1b".to_owned(),
                    "/var/lib/gridops-agent/work/0a1b".to_owned(),
                ]
            ))
        );
    }

    #[test]
    fn a_runner_without_a_shared_folder_has_nothing_to_remove() {
        assert_eq!(
            parse_shared_folders_leftover("img\n/var/run/docker.sock\n"),
            None
        );
        assert_eq!(
            parse_shared_folders_leftover("img\n/var/lib/gridops-agent/externals/2f529cfa\n"),
            None
        );
        assert_eq!(parse_shared_folders_leftover(""), None);
    }

    #[test]
    fn only_a_direct_child_of_the_work_or_runners_root_is_ever_removed() {
        for source in [
            "/var/lib/gridops-agent/work",
            "/var/lib/gridops-agent/work/",
            "/var/lib/gridops-agent/work/a/b",
            "/var/lib/gridops-agent/work/..",
            "/var/lib/gridops-agent/workaround/x",
            "/var/lib/gridops-agent/runners",
            "/var/lib/gridops-agent/runners/",
            "/var/lib/gridops-agent/runners/a/b",
            "/var/lib/gridops-agent/runners/..",
            "/var/lib/gridops-agent/runnersx/a",
            "/var/lib/gridops-agent/externals/2f529cfa",
            "/var/lib/gridops-agent",
            "/etc",
        ] {
            assert_eq!(
                parse_shared_folders_leftover(&format!("img\n{source}\n")),
                None,
                "{source}"
            );
        }
    }
    #[test]
    fn creates_the_folder_under_a_fresh_root() -> std::io::Result<()> {
        let dir = scratch_dir("fresh")?;
        let root = dir.join("work");
        let work = root.join("env1");
        create_shared_work_folder(&root, &work)?;
        assert!(work.is_dir());
        std::fs::remove_dir_all(&dir)
    }

    #[test]
    fn moves_an_unwritable_root_aside_and_starts_a_fresh_one() -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch_dir("foreign")?;
        let root = dir.join("work");
        std::fs::create_dir_all(&root)?;
        // A root nobody can write to stands in for one the Docker daemon created as root.
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o555))?;
        if std::fs::create_dir(root.join("probe")).is_ok() {
            // Running as root: permissions do not apply, so there is nothing to move aside.
            return std::fs::remove_dir_all(&dir);
        }
        let work = root.join("env1");
        create_shared_work_folder(&root, &work)?;
        assert!(work.is_dir());
        let mut aside = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if name.starts_with("work.not-ours-") {
                aside.push(name);
            }
        }
        assert_eq!(aside.len(), 1, "{aside:?}");
        for name in &aside {
            std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::remove_dir_all(&dir)
    }

    fn scratch_dir(tag: &str) -> std::io::Result<std::path::PathBuf> {
        let dir = std::env::temp_dir().join(format!("gridops-work-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}
