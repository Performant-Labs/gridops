//! The GitLab branch of the Docker runtime (#147), driven against a stand-in
//! `docker` CLI that plays a gitlab-runner container through each way its
//! Start can end: GitLab accepts the token (the script prints the readiness
//! marker and keeps running), refuses it (exit 77, at once or a few seconds
//! in), can't be asked (`GITLAB_EXIT_VERIFY_UNAVAILABLE`), the script fails
//! otherwise, or nothing is ever said.
//!
//! The stand-in keeps its container's state in files beside it and answers
//! `inspect` by filling in whatever Go template it is given, so these tests
//! pin what the runtime reports, not which `docker` calls it makes. A
//! template it can't fill is written to `unsupported` and shown on failure.

use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, ensure};
use gridops_core::{
    fleet::{
        Architecture, CiTarget, ExecutionOs, ExternalId, GitlabConnectionId, ResourceRequest,
        RuntimeKind,
        ids::{BootstrapReference, PreparedEnvironmentId, ProfileId, Revision},
        protocol::{
            agent::{
                AgentOperation, ExecutionConfiguration, GitlabRunnerMaterial,
                LocalResourceIdentity, PrepareEnvironment, PreparedWorkload, RunnerMode,
                StartRunner,
            },
            outcomes::{DiagnosticCode, FailureCode, SuccessEvidence},
            primitives::{ImageReference, WireTimestamp},
        },
    },
    provisioning::{
        GITLAB_EXIT_VERIFY_UNAVAILABLE, GITLAB_READY_MARKER, GITLAB_RUNNER_IMAGE,
        GITLAB_START_READY_TIMEOUT_SECS, gitlab_runner_env, gitlab_runner_script,
    },
};
use gridops_host_agent::{
    docker::DockerRuntime,
    helper::{Bootstrap, HelperRequest, HelperResponse, RuntimeHelper},
};
use serde_json::json;

/// The id the stand-in gives the container it creates.
const CONTAINER: &str = "c0ffee-gitlab-runner";
const TOKEN: &str = "glrt-fleet-test-token-0001";
const URL: &str = "https://gitlab.example.com";
/// The pool's runner image, which the server chose (B-1).
const POOL_IMAGE: &str = "registry.example/gitlab-runner:pool-pinned";
const SOCKET: &str = "/var/run/docker.sock";
/// The reconciler's start deadline less its delivery margin (R2 B-2).
const START_BUDGET: Duration = Duration::from_secs(25);

/// A `docker` CLI with one container, whose state lives in files beside it:
/// `container` (its id, while it exists), `name`, `label`, `scenario`
/// (`<kind> <exit code> <seconds>`: `ready` runs and prints the marker,
/// `silent` runs and prints nothing, `exit` runs for the seconds given
/// after its start, then exits with the code), `marker` and `started`.
/// It records `calls`, `create-args` (NUL-separated), `stdin`, `pulled` and
/// `removed`.
const STAND_IN: &str = r#"#!/usr/bin/env bash
d=$(cd "$(dirname "$0")" && pwd)
printf '%s\n' "$*" >> "$d/calls"
id=$(cat "$d/container" 2>/dev/null)
name=$(cat "$d/name" 2>/dev/null)

ours() { [ -f "$d/container" ] && { [ "$1" = "$id" ] || [ "$1" = "$name" ]; }; }
# status running exit-code
state() {
  [ -f "$d/container" ] || return 1
  if [ ! -f "$d/started" ]; then echo "created false 0"; return 0; fi
  if [ -f "$d/stopped" ]; then echo "exited false 137"; return 0; fi
  read -r kind code delay < "$d/scenario"
  if [ "$kind" = exit ] && [ $(( $(date +%s) - $(cat "$d/started") )) -ge "$delay" ]; then
    echo "exited false $code"
  else
    echo "running true 0"
  fi
}
running() { set -- $(state); [ "$2" = true ]; }
output() {
  [ -f "$d/started" ] || return 0
  read -r kind code delay < "$d/scenario"
  echo "Runtime platform                                    arch=amd64 os=linux"
  if [ "$kind" = ready ]; then cat "$d/marker"; fi
  return 0
}
missing() { echo "Error: No such object: $1" >&2; exit 1; }

cmd=$1; shift
if [ "$cmd" = container ]; then cmd=$1; shift; fi
case "$cmd" in
  version) echo linux/amd64 ;;
  info) echo stand-in-engine ;;
  pull) printf '%s\n' "$1" >> "$d/pulled" ;;
  image) exit 0 ;;
  create)
    printf '%s\0' "$@" > "$d/create-args"
    prev=
    for a in "$@"; do
      case "$prev" in
        --name) printf '%s' "$a" > "$d/name" ;;
        --label) case "$a" in io.gridops.fleet.environment=*) printf '%s' "${a#*=}" > "$d/label" ;; esac ;;
      esac
      prev=$a
    done
    echo c0ffee-gitlab-runner > "$d/container"
    echo c0ffee-gitlab-runner ;;
  start)
    ref=${!#}; ours "$ref" || missing "$ref"
    date +%s > "$d/started"
    attached=false
    for a in "$@"; do case "$a" in -i|--interactive|-a|--attach|-ai|-ia) attached=true ;; esac; done
    if $attached; then
      IFS= read -r line && printf '%s\n' "$line" >> "$d/stdin"
      output
      # Attached until the container ends, as Docker's own `start -i` is.
      while running; do sleep 0.1; done
      set -- $(state); exit "${3:-0}"
    fi
    echo "$ref" ;;
  inspect)
    fmt=; refs=()
    while [ $# -gt 0 ]; do
      case "$1" in
        -f|--format) fmt=$2; shift 2 ;;
        --format=*) fmt=${1#--format=}; shift ;;
        --type) shift 2 ;;
        --type=*) shift ;;
        *) refs+=("$1"); shift ;;
      esac
    done
    for ref in "${refs[@]}"; do ours "$ref" || missing "$ref"; done
    set -- $(state); status=$1 run=$2 code=$3
    label=$(cat "$d/label" 2>/dev/null)
    js="{\"Status\":\"$status\",\"Running\":$run,\"ExitCode\":$code}"
    if [ -z "$fmt" ]; then
      echo "[{\"Id\":\"$id\",\"Name\":\"/$name\",\"State\":$js,\"Config\":{\"Labels\":{\"io.gridops.fleet.environment\":\"$label\"}}}]"
      exit 0
    fi
    t=$fmt
    t=${t//'{{.Id}}'/"$id"}
    t=${t//'{{.Name}}'/"/$name"}
    t=${t//'{{.State.Running}}'/"$run"}
    t=${t//'{{.State.ExitCode}}'/"$code"}
    t=${t//'{{.State.Status}}'/"$status"}
    t=${t//'{{json .State}}'/"$js"}
    t=${t//'{{json .State.ExitCode}}'/"$code"}
    t=${t//'{{index .Config.Labels "io.gridops.fleet.environment"}}'/"$label"}
    t=${t//'{{range .Mounts}}{{.Source}}{{"\n"}}{{end}}'/}
    t=${t//'{{"\n"}}'/$'\n'}
    case "$t" in
      *'{{'*) printf '%s\n' "$fmt" >> "$d/unsupported"; echo "stand-in docker: unsupported template $fmt" >&2; exit 2 ;;
    esac
    printf '%s\n' "$t" ;;
  logs)
    ref=${!#}; ours "$ref" || missing "$ref"
    follow=false
    for a in "$@"; do case "$a" in -f|--follow) follow=true ;; esac; done
    output
    if $follow; then while running; do sleep 0.1; done; fi ;;
  wait)
    ours "$1" || missing "$1"
    while running; do sleep 0.1; done
    set -- $(state); echo "$3" ;;
  rm)
    ref=${!#}; ours "$ref" || { echo "Error: No such container: $ref" >&2; exit 1; }
    printf 'rm %s\n' "$*" >> "$d/removed"
    rm -f "$d/container" "$d/started" "$d/stopped"
    echo "$ref" ;;
  kill|stop)
    ref=${!#}; ours "$ref" || missing "$ref"
    touch "$d/stopped"; echo "$ref" ;;
  *)
    printf '%s\n' "$cmd $*" >> "$d/unsupported"
    echo "stand-in docker: unsupported command $cmd" >&2; exit 2 ;;
esac
"#;

/// In the rerun with the stand-in first on PATH: its directory. Otherwise
/// reruns this binary's test `test` that way and answers `None`.
fn stand_in(test: &str) -> Result<Option<PathBuf>> {
    if let Some(directory) = std::env::var_os("STAND_IN_GITLAB_DOCKER") {
        return Ok(Some(PathBuf::from(directory)));
    }
    // The runtime runs `docker` from PATH, which a test can't change for its
    // own process, so it reruns itself with the stand-in first.
    let directory =
        std::env::temp_dir().join(format!("gridops-gitlab-docker-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory)?;
    let script = directory.join("docker");
    fs::write(&script, STAND_IN)?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
    let output = Command::new(std::env::current_exe()?)
        .args(["--exact", test, "--nocapture"])
        .env(
            "PATH",
            format!("{}:{}", directory.display(), std::env::var("PATH")?),
        )
        .env("STAND_IN_GITLAB_DOCKER", &directory)
        .output()?;
    let unsupported = fs::read_to_string(directory.join("unsupported")).unwrap_or_default();
    fs::remove_dir_all(&directory)?;
    ensure!(
        output.status.success(),
        "{test} failed with the stand-in Docker\n--- stdout\n{}\n--- stderr\n{}\n--- docker calls the stand-in could not answer\n{unsupported}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    Ok(None)
}

fn gitlab_target() -> Result<CiTarget> {
    Ok(CiTarget::GitlabProject {
        connection_id: GitlabConnectionId::new("gitlab-connection-1")?,
        project_id: ExternalId::new(42)?,
    })
}

fn github_target() -> Result<CiTarget> {
    Ok(CiTarget::GithubRepository {
        installation_id: ExternalId::new(1)?,
        repository_id: ExternalId::new(1)?,
    })
}

fn prepared(target: CiTarget, mode: RunnerMode, docker_socket: bool) -> Result<PrepareEnvironment> {
    Ok(PrepareEnvironment {
        execution_profile_id: ProfileId::try_new(uuid::Uuid::new_v4())?,
        profile_revision: Revision::new(1)?,
        workload: PreparedWorkload::CiRunner,
        target,
        runtime_kind: RuntimeKind::Docker,
        execution_os: ExecutionOs::Linux,
        architecture: Architecture::X64,
        mode,
        image: ImageReference::try_from(POOL_IMAGE.to_owned())?,
        labels: Vec::new(),
        configuration: ExecutionConfiguration {
            requires_docker_socket: docker_socket,
            ..ExecutionConfiguration::default()
        },
        resource_request: ResourceRequest::new(1_000, 1_024, 0)?,
        allocation_ids: Vec::new(),
    })
}

fn material(url: &str) -> Result<String> {
    Ok(serde_json::to_string(
        &GitlabRunnerMaterial::GitlabRunner {
            token: TOKEN.to_owned(),
            url: url.to_owned(),
            wait_timeout_seconds: 600,
        },
    )?)
}

fn runtime() -> DockerRuntime {
    DockerRuntime::new(None, Some(SOCKET.to_owned()))
}

/// A Start of the environment `prepared`, with `bootstrap` as its material.
fn start(prepared: PrepareEnvironment, bootstrap: &str) -> Result<HelperRequest> {
    let environment = PreparedEnvironmentId::try_new(uuid::Uuid::new_v4())?;
    let start = StartRunner {
        prepared_environment_id: environment,
        execution_profile_id: prepared.execution_profile_id,
        profile_revision: prepared.profile_revision,
        bootstrap_reference: BootstrapReference::try_new(uuid::Uuid::new_v4())?,
        bootstrap_expires_at: WireTimestamp::from_millis(gridops_core::now_millis())?,
        resource_request: prepared.resource_request,
        allocation_ids: Vec::new(),
    };
    Ok(HelperRequest {
        environment_id: Some(environment),
        docker_socket: prepared.configuration.requires_docker_socket,
        prepared: Some(prepared),
        bootstrap: Some(Bootstrap(bootstrap.to_owned())),
        operation: AgentOperation::StartRunner(start),
    })
}

/// How the container behaves once started.
fn scenario(directory: &Path, kind: &str, code: i32, seconds: u64) -> Result<()> {
    fs::write(
        directory.join("scenario"),
        format!("{kind} {code} {seconds}\n"),
    )?;
    fs::write(directory.join("marker"), format!("{GITLAB_READY_MARKER}\n"))?;
    Ok(())
}

fn created_args(directory: &Path) -> Vec<String> {
    fs::read(directory.join("create-args"))
        .map(|bytes| {
            bytes
                .split(|byte| *byte == 0)
                .filter(|arg| !arg.is_empty())
                .map(|arg| String::from_utf8_lossy(arg).into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// The container was created and is gone again.
fn removed(directory: &Path) -> bool {
    directory.join("removed").exists() && !directory.join("container").exists()
}

fn failed_with(
    response: &HelperResponse,
    code: FailureCode,
    retryable: bool,
    diagnostic: Option<DiagnosticCode>,
) -> bool {
    matches!(
        response,
        HelperResponse::Failed { code: actual, retryable: actual_retryable, diagnostic: actual_diagnostic }
            if *actual == code
                && *actual_retryable == retryable
                && actual_diagnostic.as_ref().map(|reference| reference.code) == diagnostic
    )
}

/// The value after `flag` in `args`, wherever it appears.
fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
    args.windows(2)
        .any(|pair| pair[0] == flag && pair[1] == value)
}

/// W-5: the Docker executor needs the host's daemon, so a GitLab profile
/// that doesn't ask for the Docker socket is refused at Prepare, before
/// anything is pulled (or any token minted). GitHub is unchanged.
#[tokio::test]
async fn a_gitlab_prepare_without_the_docker_socket_is_refused() -> Result<()> {
    let Some(directory) = stand_in("a_gitlab_prepare_without_the_docker_socket_is_refused")? else {
        return Ok(());
    };
    let runtime = runtime();
    let prepare = |prepared: PrepareEnvironment| -> Result<HelperRequest> {
        Ok(HelperRequest {
            environment_id: Some(PreparedEnvironmentId::try_new(uuid::Uuid::new_v4())?),
            docker_socket: prepared.configuration.requires_docker_socket,
            prepared: None,
            bootstrap: None,
            operation: AgentOperation::PrepareEnvironment(prepared),
        })
    };

    let refused = runtime
        .call(prepare(prepared(
            gitlab_target()?,
            RunnerMode::Ephemeral,
            false,
        )?)?)
        .await;
    ensure!(
        failed_with(&refused, FailureCode::Configuration, false, None),
        "{refused:?}"
    );
    ensure!(
        !directory.join("pulled").exists(),
        "a refused prepare pulled an image"
    );

    let accepted = runtime
        .call(prepare(prepared(
            gitlab_target()?,
            RunnerMode::Ephemeral,
            true,
        )?)?)
        .await;
    ensure!(
        matches!(accepted, HelperResponse::Succeeded { .. }),
        "{accepted:?}"
    );
    let github = runtime
        .call(prepare(prepared(
            github_target()?,
            RunnerMode::Ephemeral,
            false,
        )?)?)
        .await;
    ensure!(
        matches!(github, HelperResponse::Succeeded { .. }),
        "{github:?}"
    );
    Ok(())
}

/// W-4 and brief review W-2: material that isn't GitLab's, a Persistent
/// GitLab runner and a URL that isn't http(s) are configuration failures,
/// refused before any container exists, so none reaches the verify call.
#[tokio::test]
async fn a_gitlab_start_with_unusable_material_creates_nothing() -> Result<()> {
    let Some(directory) = stand_in("a_gitlab_start_with_unusable_material_creates_nothing")? else {
        return Ok(());
    };
    let runtime = runtime();
    let github_material = json!({"kind": "github_jit", "jit_config": "jit-token"}).to_string();
    for (case, request) in [
        (
            "GitHub material",
            start(
                prepared(gitlab_target()?, RunnerMode::Ephemeral, true)?,
                &github_material,
            )?,
        ),
        (
            "material that does not parse",
            start(
                prepared(gitlab_target()?, RunnerMode::Ephemeral, true)?,
                r#"{"kind":"gitlab_runner"}"#,
            )?,
        ),
        (
            "persistent mode",
            start(
                prepared(gitlab_target()?, RunnerMode::Persistent, true)?,
                &material(URL)?,
            )?,
        ),
        (
            "an ftp URL",
            start(
                prepared(gitlab_target()?, RunnerMode::Ephemeral, true)?,
                &material("ftp://gitlab.example.com")?,
            )?,
        ),
        (
            "a URL that does not parse",
            start(
                prepared(gitlab_target()?, RunnerMode::Ephemeral, true)?,
                &material("gitlab example")?,
            )?,
        ),
    ] {
        let response = runtime.call(request).await;
        ensure!(
            failed_with(&response, FailureCode::Configuration, false, None),
            "{case}: {response:?}"
        );
        ensure!(
            !directory.join("create-args").exists(),
            "{case}: a container was created"
        );
    }
    Ok(())
}

/// GitLab accepts the token: the runner is started from the pool's image
/// with the shared script, its non-secret inputs as environment variables,
/// the Docker socket and no GitHub work folder. The token goes only to its
/// stdin: no argument of `docker create`, so neither `docker inspect` nor
/// the container's main process environment can show it.
#[tokio::test]
async fn a_verified_gitlab_runner_starts_with_its_token_only_on_stdin() -> Result<()> {
    let Some(directory) = stand_in("a_verified_gitlab_runner_starts_with_its_token_only_on_stdin")?
    else {
        return Ok(());
    };
    scenario(&directory, "ready", 0, 0)?;
    let request = start(
        prepared(gitlab_target()?, RunnerMode::Ephemeral, true)?,
        &material(URL)?,
    )?;
    let began = Instant::now();
    let response = runtime().call(request).await;
    let took = began.elapsed();

    let HelperResponse::Succeeded { evidence } = &response else {
        anyhow::bail!("{response:?}");
    };
    let SuccessEvidence::RunnerStarted {
        local_identity: LocalResourceIdentity::Docker { object_id },
        ..
    } = evidence.as_ref()
    else {
        anyhow::bail!("{evidence:?}");
    };
    ensure!(object_id.as_str() == CONTAINER, "{object_id:?}");
    ensure!(
        took < Duration::from_secs(10),
        "a ready runner took {took:?} to start"
    );
    ensure!(!removed(&directory), "a started runner was removed");

    let stdin = fs::read_to_string(directory.join("stdin")).unwrap_or_default();
    ensure!(stdin == format!("{TOKEN}\n"), "stdin was {stdin:?}");

    let args = created_args(&directory);
    ensure!(
        !args.iter().any(|arg| arg.contains(TOKEN)),
        "the token is in the container's metadata: {args:?}"
    );
    for variable in gitlab_runner_env(URL, 600) {
        ensure!(
            has_pair(&args, "-e", &variable),
            "missing -e {variable}: {args:?}"
        );
    }
    ensure!(
        has_pair(&args, "-v", &format!("{SOCKET}:/var/run/docker.sock")),
        "the Docker executor has no Docker socket: {args:?}"
    );
    ensure!(
        !args
            .iter()
            .any(|arg| arg.contains(gridops_core::fleet::SHARED_WORK_ROOT)),
        "a GitLab runner got the GitHub shared work folder: {args:?}"
    );
    ensure!(args.iter().any(|arg| arg == "--init"), "{args:?}");
    // The image is the pool's; the agent holds no image of its own (B-1).
    let image = args
        .iter()
        .position(|arg| arg == POOL_IMAGE)
        .context(format!("not started from the pool's image: {args:?}"))?;
    ensure!(!args.iter().any(|arg| arg == GITLAB_RUNNER_IMAGE));
    ensure!(
        args.get(image + 1).is_some_and(|arg| arg.ends_with("bash"))
            && args.last() == Some(&gitlab_runner_script()),
        "not the shared GitLab runner script: {:?}",
        &args[image..]
    );
    Ok(())
}

/// One Start that the container ends before it is ready: what the runtime
/// reports, and that it removed the container, since nothing else can (only
/// a successful Start binds the container to its environment).
async fn failed_start(
    directory: &Path,
    exit_code: i32,
    after_seconds: u64,
    code: FailureCode,
    retryable: bool,
    diagnostic: Option<DiagnosticCode>,
) -> Result<()> {
    scenario(directory, "exit", exit_code, after_seconds)?;
    let request = start(
        prepared(gitlab_target()?, RunnerMode::Ephemeral, true)?,
        &material(URL)?,
    )?;
    let began = Instant::now();
    let response = runtime().call(request).await;
    let took = began.elapsed();
    ensure!(
        failed_with(&response, code, retryable, diagnostic),
        "exit {exit_code} after {after_seconds}s: {response:?}"
    );
    ensure!(
        removed(directory),
        "exit {exit_code} after {after_seconds}s left its container"
    );
    ensure!(took < START_BUDGET, "the start took {took:?}");
    Ok(())
}

/// B-2 and A's warning 1: GitLab refuses the token at once, inside the
/// stdin feed: a non-retryable configuration failure saying so, not a
/// retryable runtime failure.
#[tokio::test]
async fn a_token_refused_at_once_fails_the_start_as_refused() -> Result<()> {
    let Some(directory) = stand_in("a_token_refused_at_once_fails_the_start_as_refused")? else {
        return Ok(());
    };
    failed_start(
        &directory,
        77,
        0,
        FailureCode::Configuration,
        false,
        Some(DiagnosticCode::RegistrationRefused),
    )
    .await
}

/// B-2: the refusal comes a few seconds in, during the readiness wait.
#[tokio::test]
async fn a_token_refused_during_the_wait_fails_the_start_as_refused() -> Result<()> {
    let Some(directory) = stand_in("a_token_refused_during_the_wait_fails_the_start_as_refused")?
    else {
        return Ok(());
    };
    failed_start(
        &directory,
        77,
        3,
        FailureCode::Configuration,
        false,
        Some(DiagnosticCode::RegistrationRefused),
    )
    .await
}

/// R2 B-2: GitLab couldn't be asked: today's retryable runtime failure.
#[tokio::test]
async fn an_unanswered_verify_fails_the_start_as_retryable() -> Result<()> {
    let Some(directory) = stand_in("an_unanswered_verify_fails_the_start_as_retryable")? else {
        return Ok(());
    };
    failed_start(
        &directory,
        GITLAB_EXIT_VERIFY_UNAVAILABLE,
        2,
        FailureCode::Runtime,
        true,
        None,
    )
    .await
}

/// B-2: any other exit during Start is today's retryable runtime failure.
#[tokio::test]
async fn any_other_exit_fails_the_start_as_retryable() -> Result<()> {
    let Some(directory) = stand_in("any_other_exit_fails_the_start_as_retryable")? else {
        return Ok(());
    };
    failed_start(&directory, 1, 0, FailureCode::Runtime, true, None).await
}

/// R2 B-2: a runner that neither says it is ready nor exits is given up on
/// after `GITLAB_START_READY_TIMEOUT_SECS`, inside the start deadline, as a
/// retryable runtime failure, and removed.
#[tokio::test]
async fn a_runner_that_never_gets_ready_is_given_up_within_the_deadline() -> Result<()> {
    let Some(directory) =
        stand_in("a_runner_that_never_gets_ready_is_given_up_within_the_deadline")?
    else {
        return Ok(());
    };
    scenario(&directory, "silent", 0, 0)?;
    let request = start(
        prepared(gitlab_target()?, RunnerMode::Ephemeral, true)?,
        &material(URL)?,
    )?;
    let began = Instant::now();
    let response = runtime().call(request).await;
    let took = began.elapsed();
    ensure!(
        failed_with(&response, FailureCode::Runtime, true, None),
        "{response:?}"
    );
    ensure!(
        removed(&directory),
        "a runner that never got ready was left"
    );
    ensure!(
        took >= Duration::from_secs(GITLAB_START_READY_TIMEOUT_SECS - 1) && took < START_BUDGET,
        "gave up after {took:?}"
    );
    Ok(())
}

/// B-2 and B-3: an exited GitLab runner always inspects as gone, so the
/// reconciler cleans it up, saying why when the reason is the runner's: idle
/// expiry (75) or a late refusal (77, D147-2). A completed runner (0) or one
/// that failed otherwise carries no diagnostic, as a GitHub runner's exit
/// does today, and a GitHub runner is never given a GitLab reason.
#[tokio::test]
async fn an_exited_gitlab_runner_inspects_as_gone_saying_why() -> Result<()> {
    let Some(directory) = stand_in("an_exited_gitlab_runner_inspects_as_gone_saying_why")? else {
        return Ok(());
    };
    let runtime = runtime();
    fs::write(directory.join("name"), "gridops-inspected")?;
    fs::write(directory.join("label"), "inspected-environment")?;
    let resource: LocalResourceIdentity =
        serde_json::from_value(json!({"kind": "docker", "object_id": CONTAINER}))?;
    for (platform, ending, expected) in [
        (
            "gitlab",
            "exit 75 0",
            Some(Some(DiagnosticCode::IdleExpired)),
        ),
        (
            "gitlab",
            "exit 77 0",
            Some(Some(DiagnosticCode::RegistrationRefused)),
        ),
        ("gitlab", "exit 0 0", Some(None)),
        ("gitlab", "exit 3 0", Some(None)),
        ("gitlab", "ready 0 0", None),
        ("github", "exit 75 0", Some(None)),
        ("github", "exit 77 0", Some(None)),
    ] {
        fs::write(directory.join("container"), CONTAINER)?;
        fs::write(directory.join("started"), "1")?;
        fs::write(directory.join("scenario"), format!("{ending}\n"))?;
        let target = if platform == "gitlab" {
            gitlab_target()?
        } else {
            github_target()?
        };
        let prepared = prepared(target, RunnerMode::Ephemeral, platform == "gitlab")?;
        let response = runtime
            .call(HelperRequest {
                environment_id: Some(PreparedEnvironmentId::try_new(uuid::Uuid::new_v4())?),
                docker_socket: prepared.configuration.requires_docker_socket,
                prepared: Some(prepared),
                bootstrap: None,
                operation: AgentOperation::Inspect {
                    expected_resource: resource.clone(),
                },
            })
            .await;
        match expected {
            None => ensure!(
                matches!(response, HelperResponse::Succeeded { .. }),
                "{platform} {ending}: {response:?}"
            ),
            Some(diagnostic) => ensure!(
                failed_with(
                    &response,
                    FailureCode::ResourceUnavailable,
                    false,
                    diagnostic
                ),
                "{platform} {ending}: {response:?}"
            ),
        }
    }
    Ok(())
}

/// D6: the Docker backend offers GitLab project and group runners, one job
/// each, beside its GitHub runners.
#[tokio::test]
async fn the_docker_backend_offers_ephemeral_gitlab_runners() -> Result<()> {
    let Some(_directory) = stand_in("the_docker_backend_offers_ephemeral_gitlab_runners")? else {
        return Ok(());
    };
    let backends = serde_json::to_value(runtime().backends().await)?;
    let offered = backends[0]["capabilities"]["supported_runners"]
        .as_array()
        .context(format!("no supported runners: {backends}"))?
        .iter()
        .map(|runner| {
            (
                runner["target_kind"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                runner["mode"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    let expected = [
        ("github_repository", "ephemeral"),
        ("github_repository", "persistent"),
        ("github_organization", "ephemeral"),
        ("github_organization", "persistent"),
        ("gitlab_project", "ephemeral"),
        ("gitlab_group", "ephemeral"),
    ]
    .map(|(kind, mode)| (kind.to_owned(), mode.to_owned()))
    .into_iter()
    .collect::<std::collections::BTreeSet<_>>();
    ensure!(offered == expected, "{offered:?}");
    Ok(())
}
