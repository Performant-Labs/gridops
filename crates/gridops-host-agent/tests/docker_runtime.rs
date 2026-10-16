//! Drives the Docker runtime against a stand-in `docker` CLI, to pin down
//! what it reports about a runner when Docker itself can't answer.

use std::{fs, os::unix::fs::PermissionsExt as _, path::Path, process::Command};

use anyhow::{Result, ensure};
use gridops_core::fleet::protocol::{agent::AgentOperation, outcomes::FailureCode};
use gridops_host_agent::{
    docker::DockerRuntime,
    helper::{HelperRequest, HelperResponse, RuntimeHelper},
};
use serde_json::json;

/// Answers `docker inspect` as Docker would in the state named by the
/// `state` file beside it; `$3` is the `-f` template, if any.
const STAND_IN: &str = r#"#!/bin/sh
case "$(cat "$(dirname "$0")/state")" in
  running) running=true ;;
  exited) running=false ;;
  gone) echo 'Error: No such object: runner' >&2; exit 1 ;;
  *) echo 'Cannot connect to the Docker daemon. Is the docker daemon running?' >&2; exit 1 ;;
esac
case "$3" in
  *State.Running*Labels*) echo "$running environment" ;;
  *State.Running*) echo "$running" ;;
  *Labels*) echo environment ;;
  *) echo '[{}]' ;;
esac
"#;

#[tokio::test]
async fn a_runner_is_reported_gone_only_when_docker_says_so() -> Result<()> {
    let Some(directory) = std::env::var_os("STAND_IN_DOCKER") else {
        // The runtime runs `docker` from PATH, which a test can't change for
        // its own process, so it reruns itself with the stand-in first.
        let directory =
            std::env::temp_dir().join(format!("gridops-docker-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory)?;
        let script = directory.join("docker");
        fs::write(&script, STAND_IN)?;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
        let status = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "a_runner_is_reported_gone_only_when_docker_says_so",
            ])
            .env(
                "PATH",
                format!("{}:{}", directory.display(), std::env::var("PATH")?),
            )
            .env("STAND_IN_DOCKER", &directory)
            .status()?;
        fs::remove_dir_all(&directory)?;
        ensure!(status.success(), "the run with the stand-in Docker failed");
        return Ok(());
    };
    let directory = Path::new(&directory);
    let runtime = DockerRuntime::new(Vec::new(), None, None);
    let resource = serde_json::from_value(json!({"kind": "docker", "object_id": "runner"}))?;
    let request = |operation| HelperRequest {
        environment_id: None,
        docker_socket: false,
        prepared: None,
        bootstrap: None,
        operation,
    };
    let failed = |code, retryable| Some(HelperResponse::Failed { code, retryable });
    for (state, inspect, cleanup) in [
        ("running", None, None),
        (
            "exited",
            failed(FailureCode::ResourceUnavailable, false),
            None,
        ),
        (
            "gone",
            failed(FailureCode::ResourceUnavailable, false),
            None,
        ),
        // Busy or restarting Docker: neither an exit nor proof of absence.
        (
            "unreachable",
            failed(FailureCode::Runtime, true),
            failed(FailureCode::Runtime, true),
        ),
    ] {
        fs::write(directory.join("state"), state)?;
        let inspected = runtime
            .call(request(AgentOperation::Inspect {
                expected_resource: Clone::clone(&resource),
            }))
            .await;
        match &inspect {
            None => ensure!(
                matches!(inspected, HelperResponse::Succeeded { .. }),
                "{state}: {inspected:?}"
            ),
            Some(expected) => ensure!(&inspected == expected, "{state}: {inspected:?}"),
        }
        if state == "gone" || state == "unreachable" {
            let cleaned = runtime
                .call(request(AgentOperation::Cleanup {
                    expected_resource: Clone::clone(&resource),
                }))
                .await;
            match &cleanup {
                None => ensure!(
                    matches!(cleaned, HelperResponse::Succeeded { .. }),
                    "{state} cleanup: {cleaned:?}"
                ),
                Some(expected) => ensure!(&cleaned == expected, "{state} cleanup: {cleaned:?}"),
            }
        }
    }
    Ok(())
}
