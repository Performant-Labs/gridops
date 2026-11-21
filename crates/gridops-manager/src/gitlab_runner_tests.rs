//! GitLab runners on the manager's built-in Docker host (#147, D10 and
//! W-6), end to end through the real router in front of a stand-in Docker
//! engine (`stand_in_docker`) that records what a start sent it and lists the
//! containers a test gives it.

use gridops_core::provisioning::{gitlab_runner_env, gitlab_runner_script};

use super::*;
use crate::stand_in_docker::{self, Images, Recorded, StandInContainer};

const TOKEN: &str = "gitlab-runner-test-manager-token";
const RUNNER_TOKEN: &str = "glrt-manager-route-token-0001";
const LEASE: &str = "lease-gitlab-1";
const GITLAB_URL: &str = "https://gitlab.example.com";

/// The real manager router in front of `docker`, holding the capacity lease
/// the GitLab start below claims, with the runners' Docker socket when
/// `socket` is set.
async fn manager(docker: &FsPath, socket: Option<SharedDockerSocket>) -> anyhow::Result<String> {
    let state = ManagerState {
        docker: Docker::connect_with_socket(
            &docker.to_string_lossy(),
            10,
            bollard::API_DEFAULT_VERSION,
        )?,
        tart: None,
        token: TOKEN.into(),
        limits: ManagerLimits {
            host_name: "docker-host".into(),
            available_cpus: 8.0,
            total_memory_mb: 16_384,
            cpu_budget: 6.0,
            memory_budget_mb: 12_288,
            max_runners: 3,
            // No disk floor: these tests are not about the host's disk.
            min_free_disk_mb: 0,
            min_free_disk_percent: 0,
            runner_network: "gridops-runners".into(),
            log_max_size: "20m".into(),
            log_max_files: 5,
            runner_pids_limit: 1_024,
            runner_swap_percent: 100,
            budget_origin: BudgetOrigin::default(),
        },
        shared_docker_socket: socket,
        reservations: Arc::new(Mutex::new(HashMap::from([(
            LEASE.to_owned(),
            CapacityReservation {
                runner_id: "runner-gitlab-1".into(),
                pool_id: "pool-gitlab".into(),
                provider: "docker".into(),
                cpu_limit: 2.0,
                memory_limit_mb: 2_048,
                vm_slot: false,
                expires_at: Instant::now() + Duration::from_mins(5),
            },
        )]))),
        provisioning_paused: Arc::new(AtomicBool::new(false)),
        builtin_host: builtin_host::BuiltinHost::new(true, Some(true)),
        provision_locks: ProvisionLocks::default(),
        vm_slots: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base_url = format!("http://{}", listener.local_addr()?);
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(base_url)
}

fn socket() -> SharedDockerSocket {
    SharedDockerSocket {
        host_path: "/var/run/docker.sock".into(),
        group_id: 999,
    }
}

/// `POST /v1/runners` for a GitLab runner on Docker, as #139 sends it.
fn start_gitlab_runner(manager: &str) -> reqwest::RequestBuilder {
    reqwest::Client::new()
        .post(format!("{manager}/v1/runners"))
        .bearer_auth(TOKEN)
        .json(&json!({
            "runnerId": "runner-gitlab-1",
            "poolId": "pool-gitlab",
            "name": "pool-gitlab-abcd1234",
            "image": "registry.example/gitlab-runner:pool-pinned",
            "mode": "ephemeral",
            "provider": "docker",
            "platform": "gitlab",
            "runtime": "vm",
            "cpuLimit": 2.0,
            "memoryLimitMb": 2_048,
            "network": "gridops-runners",
            "capacityLease": LEASE,
            "gitlab": {"url": GITLAB_URL, "waitTimeoutSeconds": 600},
            "gitlabRunnerToken": RUNNER_TOKEN,
        }))
}

/// D10 and W-6: the container runs the shared GitLab script with its
/// non-secret inputs in its environment, is labelled as GitLab's, shares the
/// runners' Docker socket, and gets the token only on its stdin: in nothing
/// `docker inspect` shows.
#[tokio::test]
async fn a_gitlab_runner_starts_labelled_with_its_token_only_on_stdin() -> anyhow::Result<()> {
    let recorded = Recorded::default();
    let docker = stand_in_docker::start(Images {
        recorded: recorded.clone(),
        ..Default::default()
    })?;
    let manager = manager(&docker, Some(socket())).await?;
    let response = start_gitlab_runner(&manager).send().await?;
    let status = response.status();
    let text = response.text().await?;
    let _ = std::fs::remove_file(&docker);
    assert_eq!(status, StatusCode::CREATED, "{text}");

    let creates = recorded
        .creates
        .lock()
        .map_err(|_| anyhow::anyhow!("poisoned"))?
        .clone();
    let [created] = creates.as_slice() else {
        anyhow::bail!("expected one container: {creates:?}");
    };
    assert!(
        !created.to_string().contains(RUNNER_TOKEN),
        "the token is in the container's metadata: {created}"
    );
    assert_eq!(
        created["Labels"]["io.gridops.platform"], "gitlab",
        "{created}"
    );
    assert_eq!(
        created["Cmd"].as_array().and_then(|cmd| cmd.last()),
        Some(&json!(gitlab_runner_script())),
        "not the shared GitLab runner script: {}",
        created["Cmd"]
    );
    let env = created["Env"].as_array().cloned().unwrap_or_default();
    for variable in gitlab_runner_env(GITLAB_URL, 600) {
        assert!(
            env.contains(&json!(variable)),
            "missing {variable}: {env:?}"
        );
    }
    assert!(
        created["HostConfig"]["Binds"].as_array().is_some_and(
            |binds| binds.contains(&json!("/var/run/docker.sock:/var/run/docker.sock"))
        ),
        "the Docker executor has no Docker socket: {created}"
    );
    let stdin = recorded
        .stdin
        .lock()
        .map_err(|_| anyhow::anyhow!("poisoned"))?
        .clone();
    assert_eq!(String::from_utf8_lossy(&stdin), format!("{RUNNER_TOKEN}\n"));
    Ok(())
}

/// W-6: without a Docker socket for runners, the Docker executor has no
/// daemon to run jobs on, so the start is refused and nothing is created.
#[tokio::test]
async fn a_gitlab_runner_is_refused_without_a_runner_docker_socket() -> anyhow::Result<()> {
    let recorded = Recorded::default();
    let docker = stand_in_docker::start(Images {
        recorded: recorded.clone(),
        ..Default::default()
    })?;
    let manager = manager(&docker, None).await?;
    let response = start_gitlab_runner(&manager).send().await?;
    let status = response.status();
    let text = response.text().await?;
    let _ = std::fs::remove_file(&docker);
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert!(!text.contains(RUNNER_TOKEN), "{text}");
    assert!(
        recorded
            .creates
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?
            .is_empty()
    );
    Ok(())
}

/// D10 (R2-1 item 3 of #137) and W-6: an exited GitLab runner is listed with
/// why it ended, read from its container's exit status through
/// `GITLAB_EXIT_REASONS`. A status outside the table, a running GitLab
/// runner and any GitHub runner carry no `exitReason` at all.
#[tokio::test]
async fn exited_gitlab_runners_are_listed_with_why_they_ended() -> anyhow::Result<()> {
    let runner = |id: &str, platform: Option<&str>, state, exit_code| {
        let mut labels = vec![
            ("io.gridops.managed", "true".to_owned()),
            ("io.gridops.runner-id", format!("runner-{id}")),
            ("io.gridops.pool-id", "pool-gitlab".to_owned()),
            ("io.gridops.mode", "ephemeral".to_owned()),
        ];
        if let Some(platform) = platform {
            labels.push(("io.gridops.platform", platform.to_owned()));
        }
        StandInContainer {
            id: id.to_owned(),
            state,
            exit_code,
            labels,
        }
    };
    let docker = stand_in_docker::start(Images {
        containers: vec![
            runner("a0000000000000000001", Some("gitlab"), "exited", 0),
            runner("a0000000000000000002", Some("gitlab"), "exited", 75),
            runner("a0000000000000000003", Some("gitlab"), "exited", 77),
            runner("a0000000000000000004", Some("gitlab"), "exited", 3),
            runner("a0000000000000000005", Some("gitlab"), "running", 0),
            runner("a0000000000000000006", None, "exited", 75),
        ],
        ..Default::default()
    })?;
    let manager = manager(&docker, Some(socket())).await?;
    let response = reqwest::Client::new()
        .get(format!("{manager}/v1/runners?partial=true"))
        .bearer_auth(TOKEN)
        .send()
        .await?;
    let status = response.status();
    let body = response.json::<Value>().await?;
    let _ = std::fs::remove_file(&docker);
    assert_eq!(status, StatusCode::OK, "{body}");
    let reasons = body["runners"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|runner| {
            (
                runner["id"].as_str().unwrap_or_default().to_owned(),
                runner.get("exitReason").cloned(),
            )
        })
        .collect::<HashMap<_, _>>();
    let expected = HashMap::from([
        ("a0000000000000000001".to_owned(), Some(json!("completed"))),
        (
            "a0000000000000000002".to_owned(),
            Some(json!("idle_expired")),
        ),
        (
            "a0000000000000000003".to_owned(),
            Some(json!("registration_refused")),
        ),
        ("a0000000000000000004".to_owned(), None),
        ("a0000000000000000005".to_owned(), None),
        ("a0000000000000000006".to_owned(), None),
    ]);
    assert_eq!(reasons, expected, "{body}");
    Ok(())
}
