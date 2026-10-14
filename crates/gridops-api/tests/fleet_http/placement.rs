//! A runner's whole fleet life against the API binary and the real agent:
//! profile, enrollment, approval, activation, admission, prepare, start with
//! one-time registration material, live console output, inspection and
//! cleanup. Only the container runtime is a stand-in.

use std::sync::{Arc, Mutex};

use gridops_core::{
    Vault,
    fleet::{
        FleetService, Principal, SubmitIntent, WorkloadKind,
        ids::{Generation, PlacementId, WorkloadId},
        placement,
        protocol::{
            agent::{AgentOperation, LocalResourceIdentity, StartRunner},
            inventory::ObservedBackend,
            outcomes::{FailureCode, SuccessEvidence},
            primitives::WireTimestamp,
        },
    },
};
use gridops_host_agent::{
    client::Client,
    dispatch::Trust,
    helper::{HelperRequest, HelperResponse, RunnerOutput, RuntimeHelper},
    journal::Journal,
    supervisor::{Stop, Supervisor},
};
use secrecy::ExposeSecret as _;
use tokio::{io::AsyncWriteExt as _, sync::oneshot};

use super::*;

/// Plays a Docker host whose runner exits as soon as it's inspected.
#[derive(Default)]
struct FakeDocker {
    bootstraps: Mutex<Vec<String>>,
    operations: Mutex<Vec<&'static str>>,
    /// The started container's console, written by the test.
    console: Mutex<Option<tokio::io::DuplexStream>>,
}

impl RuntimeHelper for FakeDocker {
    async fn call(&self, request: HelperRequest) -> HelperResponse {
        let kind = request.operation.database_kind();
        if let Ok(mut operations) = self.operations.lock() {
            operations.push(kind);
        }
        let container = || LocalResourceIdentity::Docker {
            object_id: gridops_core::fleet::ids::UpstreamText::new("fake-container")
                .unwrap_or_else(|_| unreachable!()),
        };
        let evidence = match (&request.operation, request.environment_id) {
            (AgentOperation::PrepareEnvironment(_), Some(prepared_environment_id)) => {
                SuccessEvidence::PreparedEnvironment {
                    prepared_environment_id,
                }
            }
            (AgentOperation::StartRunner(start), Some(_)) => {
                let Some(material) = &request.bootstrap else {
                    return HelperResponse::Failed {
                        code: FailureCode::Configuration,
                        retryable: false,
                    };
                };
                if let Ok(mut bootstraps) = self.bootstraps.lock() {
                    bootstraps.push(material.0.clone());
                }
                SuccessEvidence::RunnerStarted {
                    prepared_environment_id: start.prepared_environment_id,
                    execution_profile_id: start.execution_profile_id,
                    profile_revision: start.profile_revision,
                    local_identity: container(),
                }
            }
            // The runner has already exited.
            (AgentOperation::Inspect { .. }, _) => {
                return HelperResponse::Failed {
                    code: FailureCode::ResourceUnavailable,
                    retryable: false,
                };
            }
            (AgentOperation::Cleanup { expected_resource }, _) => {
                let now = WireTimestamp::from_millis(gridops_core::now_millis())
                    .unwrap_or_else(|_| unreachable!());
                let reference =
                    || -> gridops_core::fleet::protocol::outcomes::DiagnosticReference {
                        serde_json::from_value(json!({"id": Uuid::new_v4(), "code": "runtime"}))
                            .unwrap_or_else(|_| unreachable!())
                    };
                SuccessEvidence::Cleaned {
                    local_identity: expected_resource.clone(),
                    local_absence: serde_json::from_value(json!({
                        "resource": expected_resource, "observed_at": now,
                        "evidence": reference(),
                    }))
                    .unwrap_or_else(|_| unreachable!()),
                    delayed_start_exclusion: serde_json::from_value(json!({
                        "resource": expected_resource, "excluded_at": now,
                        "evidence": reference(),
                    }))
                    .unwrap_or_else(|_| unreachable!()),
                }
            }
            _ => {
                return HelperResponse::Failed {
                    code: FailureCode::Unsupported,
                    retryable: false,
                };
            }
        };
        HelperResponse::Succeeded {
            evidence: Box::new(evidence),
        }
    }

    async fn output(&self, _resource: LocalResourceIdentity) -> Option<RunnerOutput> {
        let console = self.console.lock().ok()?.take()?;
        Some(Box::pin(console))
    }

    async fn backends(&self) -> Vec<ObservedBackend> {
        serde_json::from_value(json!([{
            "local_key":"docker","known_id":null,"domain_local_key":"root",
            "runtime_kind":"docker","execution_os":"linux","architecture":"arm64",
            "readiness":"ready","reason":null,"runtime_identity":"docker-fake",
            "capabilities":{
                "schema_version":1,"runtime_kind":"docker","execution_os":"linux","architecture":"arm64",
                "supported_runners":[{"target_kind":"github_repository","mode":"ephemeral"}],
                "environments":[{"reference":"runner:fixture","readiness":"ready","version":null}],
                "resources":{"cpu":"hard_limit","memory":"hard_limit","disk":"estimate"},
                "interactive":"unsupported","docker_socket":"unsupported"
            }
        }]))
        .unwrap_or_default()
    }
}

async fn seed_pool(f: &Fixture) -> Result<()> {
    sqlx::raw_sql(
        "INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
           VALUES (1,1,'octo-org','Organization','Organization','selected',1,1);
         INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
           VALUES (10,1,'octo-org','gridops','octo-org/gridops',0,'master','https://github.com/octo-org/gridops',1,1,1);
         INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,provider,providers,labels,image,docker_image,tart_image,created_at,updated_at)
           VALUES ('fleet-pool',1,10,'fleet','repository','docker','[\"docker\"]','[\"self-hosted\"]','runner:fixture','runner:fixture','macos',1,1);
         INSERT INTO runner_pool_repositories (pool_id,repository_id,created_at) VALUES ('fleet-pool',10,1);
         INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('fleet-user',1,'admin',1);",
    )
    .execute(&f.database)
    .await?;
    Ok(())
}

async fn eventually<T, F, Fut>(what: &str, mut probe: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>>>,
{
    for _ in 0..200 {
        if let Some(value) = probe().await? {
            return Ok(value);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bail!("timed out waiting for {what}")
}

async fn placement_state(f: &Fixture, placement: PlacementId) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT state FROM workload_placements WHERE id=?")
            .bind(placement.to_string())
            .fetch_one(&f.database)
            .await?,
    )
}

#[tokio::test]
async fn admitted_runner_is_prepared_started_streamed_and_cleaned_on_its_host() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;

    let response = f
        .client
        .put(f.url("/pools/fleet-pool/profile"))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin)
        .json(&json!({
            "enabled": true, "mode": "ephemeral", "cpuMillis": 1000, "memoryMib": 1024, "diskBytes": 0,            "platforms": [{"runtimeKind": "docker", "executionOs": "linux", "architecture": "arm64", "image": "runner:fixture"}],
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let profile: Value = response.json().await?;
    let target = profile["item"]["targetIds"][0]
        .as_str()
        .context("profile target")?
        .to_owned();

    let issued: Value = f
        .admin_post("/enrollments")
        .header("idempotency-key", "placement-enrollment")
        .json(&json!({"targetIds": [target]}))
        .send()
        .await?
        .json()
        .await?;
    let runtime = Arc::new(FakeDocker::default());
    let (mut console, output) = tokio::io::duplex(64 * 1024);
    *runtime
        .console
        .lock()
        .map_err(|_| anyhow::anyhow!("poisoned"))? = Some(output);
    let journal =
        Journal::open(&f.directory.join(format!("agent-{}.sqlite", Uuid::new_v4()))).await?;
    let supervisor = Supervisor::new(
        Client::new(&f.origin)?,
        journal.clone(),
        "fleet-host".into(),
    )
    .with_heartbeat(Duration::from_millis(200))
    .with_measurements(|| gridops_host_agent::probe::Measurement {
        memory_total_mib: Some(16_384),
        memory_used_mib: Some(4_096),
        cpu_used_millis: Some(500),
        uptime_seconds: Some(60),
    })
    .with_runtime(Arc::clone(&runtime), Trust::default());
    supervisor
        .enroll(issued["code"].as_str().context("code")?, None)
        .await?;
    let host = journal.identity().await?.context("identity")?.host_id;
    let (stop, stopped) = oneshot::channel::<()>();
    let running = tokio::spawn(async move {
        supervisor
            .run(async {
                let _ = stopped.await;
            })
            .await
    });

    let read = eventually("inventory", || async {
        let read: Value = f
            .client
            .get(f.url(&format!("/hosts/{host}")))
            .header(header::COOKIE, &f.cookie)
            .send()
            .await?
            .json()
            .await?;
        Ok((read["item"]["approvalEvidence"]["state"] == "available").then_some(read))
    })
    .await?;
    let evidence = &read["item"]["approvalEvidence"];
    let approved = f
        .admin_post(&format!("/hosts/{host}/approve"))
        .json(&json!({
            "expectedRevision": evidence["expectedRevision"],
            "expectedInventoryRevision": evidence["expectedInventoryRevision"],
            "expectedInventoryDigest": evidence["expectedInventoryDigest"],
            "targetIds": [target],
        }))
        .send()
        .await?;
    assert_eq!(approved.status(), StatusCode::OK);
    let backend: String = sqlx::query_scalar("SELECT id FROM host_backends WHERE host_id=?")
        .bind(host.to_string())
        .fetch_one(&f.database)
        .await?;
    let enabled = f
        .admin_post(&format!("/hosts/{host}/backends/{backend}/enabled"))
        .json(&json!({"enabled": true}))
        .send()
        .await?;
    assert_eq!(enabled.status(), StatusCode::OK);
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM fleet_hosts WHERE id=?")
        .bind(host.to_string())
        .fetch_one(&f.database)
        .await?;
    let activated = f
        .admin_post(&format!("/hosts/{host}/lifecycle"))
        .json(&json!({"expectedRevision": revision, "lifecycle": "active"}))
        .send()
        .await?;
    assert_eq!(activated.status(), StatusCode::OK);
    eventually("work authority", || async {
        let expires: Option<i64> =
            sqlx::query_scalar("SELECT authority_expires_at FROM fleet_hosts WHERE id=?")
                .bind(host.to_string())
                .fetch_one(&f.database)
                .await?;
        Ok(expires.map(|_| ()))
    })
    .await?;

    // As the reconciler does: the runner row exists before placement, and
    // must not count against a pool that allows exactly one runner.
    let workload = Uuid::new_v4();
    sqlx::query("UPDATE runner_pools SET max_count=1 WHERE id='fleet-pool'")
        .execute(&f.database)
        .await?;
    sqlx::query(
        "INSERT INTO runners (id,pool_id,name,provider,ci_platform,os,architecture,status,ephemeral,configuration_version,created_at,updated_at)
         VALUES (?,'fleet-pool','fleet-runner','docker','github','linux','ARM64','starting',1,1,1,1)",
    )
    .bind(workload.to_string())
    .execute(&f.database)
    .await?;
    let profile_id = profile["item"]["platforms"][0]["id"]
        .as_str()
        .context("profile id")?;
    // A pool set to "machines first" would place its next runner here.
    assert!(
        gridops_core::fleet::targets::machine_ready(&f.database, "fleet-pool", profile_id).await?
    );
    let service = FleetService::new(f.database.clone());
    service
        .submit_intent(
            &Principal::Reconciler,
            SubmitIntent {
                idempotency_key: "runner:placement-test".into(),
                pool_id: "fleet-pool".into(),
                profile_id: profile_id.parse()?,
                target_id: target.parse()?,
                workload_id: WorkloadId::try_new(workload)?,
                generation: Generation::new(1)?,
                kind: WorkloadKind::CiRunner,
                agent_run_id: None,
                expected_profile_revision: Some(1),
            },
        )
        .await?;
    let report = service.admit_queued(&Principal::Reconciler, 10).await?;
    let reason: Option<String> =
        sqlx::query_scalar("SELECT reason_code FROM workload_intents LIMIT 1")
            .fetch_one(&f.database)
            .await?;
    assert_eq!(report.admitted, 1, "not admitted: {reason:?}");
    let placement_id: PlacementId =
        sqlx::query_scalar::<_, String>("SELECT id FROM workload_placements")
            .fetch_one(&f.database)
            .await?
            .parse()?;

    eventually("prepared environment", || async {
        Ok((placement_state(&f, placement_id).await? == "registering").then_some(()))
    })
    .await?;
    let prepared = placement::prepared_environment(&f.database, placement_id)
        .await?
        .context("prepared environment")?;
    let prepare = placement::prepared_with(&f.database, placement_id)
        .await?
        .context("prepare command")?;
    let vault = Vault::new([0; 32], SESSION_SECRET.as_bytes().to_vec());
    let reference = placement::store_bootstrap(
        &f.database,
        &vault,
        placement_id,
        r#"{"kind":"github_jit","jit_config":"one-time-jit"}"#,
        60_000,
    )
    .await?;
    let start = AgentOperation::StartRunner(StartRunner {
        prepared_environment_id: prepared,
        execution_profile_id: prepare.execution_profile_id,
        profile_revision: prepare.profile_revision,
        bootstrap_reference: reference,
        bootstrap_expires_at: WireTimestamp::from_millis(gridops_core::now_millis() + 60_000)?,
        resource_request: prepare.resource_request,
        allocation_ids: prepare.allocation_ids,
    });
    placement::mark_starting(&f.database, placement_id, &workload.to_string()).await?;
    placement::issue_command(&f.database, placement_id, start, "start:test", 55_000)
        .await?
        .context("start command")?;
    eventually("running runner", || async {
        Ok((placement_state(&f, placement_id).await? == "running").then_some(()))
    })
    .await?;
    assert!(
        runtime
            .bootstraps
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?
            .iter()
            .any(|material| material.contains("one-time-jit"))
    );
    let erased: Option<String> =
        sqlx::query_scalar("SELECT sealed FROM fleet_bootstrap_material WHERE placement_id=?")
            .bind(placement_id.to_string())
            .fetch_one(&f.database)
            .await?;
    assert!(erased.is_none(), "registration material outlived its start");

    // The runner picks up a job; its console output shows in the job log
    // view while the job runs.
    let now = gridops_core::now_millis();
    sqlx::query("UPDATE users SET access_token=? WHERE id='fleet-user'")
        .bind(vault.seal("fixture-token")?)
        .execute(&f.database)
        .await?;
    sqlx::raw_sql(
        "INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
           VALUES (70,10,'CI',7,'push','in_progress','abc','https://github.com/octo-org/gridops/actions/runs/70',1,1,1,1);
         INSERT INTO workflow_jobs (id,run_id,name,status,html_url,started_at,created_at,updated_at)
           VALUES (700,70,'test','in_progress','https://github.com/octo-org/gridops/actions/runs/70/job/700',1,1,1);",
    )
    .execute(&f.database)
    .await?;
    sqlx::query("UPDATE runners SET current_job_id=700 WHERE id=?")
        .bind(workload.to_string())
        .execute(&f.database)
        .await?;
    let job_view = || async {
        let view: Value = f
            .client
            .get(format!("{}/api/v1/workflow-jobs/700/logs", f.origin))
            .header(header::COOKIE, &f.cookie)
            .send()
            .await?
            .json()
            .await?;
        anyhow::Ok(view)
    };
    let first = "2026-10-14T10:00:00.000000000Z [GRIDOPS JOB LOG 1.log]\n\
        2026-10-14T10:00:01.000000000Z 2026-10-14T10:00:01.0000000Z ##[group]Run cargo test\n\
        2026-10-14T10:00:02.000000000Z 2026-10-14T10:00:02.0000000Z    Compiling gridops-core v0.1.0\n";
    console.write_all(first.as_bytes()).await?;
    let view = eventually("live runner output", || async {
        let view = job_view().await?;
        Ok(view["steps"]
            .to_string()
            .contains("Compiling gridops-core")
            .then_some(view))
    })
    .await?;
    assert_eq!(view["source"], "runner");
    assert_eq!(view["status"], "in_progress");
    let stream = || async {
        anyhow::Ok(
            sqlx::query_as::<_, (i64, i64, bool, Option<i64>)>(
                "SELECT o.received_bytes,s.size_bytes,s.complete,s.job_id FROM fleet_output_streams o
                   JOIN log_streams s ON s.id=o.log_stream_id WHERE o.placement_id=?",
            )
            .bind(placement_id.to_string())
            .fetch_one(&f.database)
            .await?,
        )
    };
    let shipped = i64::try_from(first.len())?;
    assert_eq!(stream().await?, (shipped, shipped, false, None));

    // A replayed chunk is appended once, and only the placement's own host,
    // in its live session, can write the placement's output.
    let identity = journal.identity().await?.context("identity")?;
    let credential = identity.credential.expose_secret().to_owned();
    let session: String = sqlx::query_scalar("SELECT agent_session_id FROM fleet_hosts WHERE id=?")
        .bind(host.to_string())
        .fetch_one(&f.database)
        .await?;
    let chunk = |session: &str, data: &str| {
        json!({
            "authority_session_id": session, "placement_id": placement_id,
            "offset": 0, "data": base64::engine::general_purpose::STANDARD.encode(data),
        })
    };
    let replay: Value = inventory::agent_post(&f, &host.to_string(), &credential, "output")
        .json(&chunk(&session, first))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(replay, json!({"received": shipped, "finished": false}));
    error(
        inventory::agent_post(&f, &host.to_string(), &credential, "output")
            .json(&chunk(&Uuid::new_v4().to_string(), "forged\n"))
            .send()
            .await?,
        StatusCode::FORBIDDEN,
        "forbidden",
    )
    .await?;
    let (other, other_credential) = inventory::enrolled(&f).await?;
    let other_session = Uuid::new_v4().to_string();
    sqlx::query("UPDATE fleet_hosts SET agent_session_id=?,authority_expires_at=? WHERE id=?")
        .bind(&other_session)
        .bind(now + 60_000)
        .bind(&other)
        .execute(&f.database)
        .await?;
    error(
        inventory::agent_post(&f, &other, &other_credential, "output")
            .json(&chunk(&other_session, "forged\n"))
            .send()
            .await?,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await?;
    assert_eq!(stream().await?, (shipped, shipped, false, None));

    // When the runner stops, the rest arrives and the stream is kept as the
    // job's runner log.
    let last =
        "2026-10-14T10:00:03.000000000Z 2026-10-14T10:00:03.0000000Z test result: ok. 12 passed\n";
    console.write_all(last.as_bytes()).await?;
    drop(console);
    let total = shipped + i64::try_from(last.len())?;
    eventually("finished output", || async {
        Ok((stream().await? == (total, total, true, Some(700))).then_some(()))
    })
    .await?;
    assert!(
        job_view().await?["steps"]
            .to_string()
            .contains("test result: ok")
    );

    let container = LocalResourceIdentity::Docker {
        object_id: gridops_core::fleet::ids::UpstreamText::new("fake-container")?,
    };
    placement::issue_command(
        &f.database,
        placement_id,
        AgentOperation::Inspect {
            expected_resource: container.clone(),
        },
        "inspect:test",
        25_000,
    )
    .await?
    .context("inspect command")?;
    eventually("exit observed", || async {
        Ok(
            placement::reported_gone(&f.database, placement_id, "inspect")
                .await?
                .then_some(()),
        )
    })
    .await?;
    placement::issue_command(
        &f.database,
        placement_id,
        AgentOperation::Cleanup {
            expected_resource: container,
        },
        "cleanup:test",
        55_000,
    )
    .await?
    .context("cleanup command")?;
    eventually("cleaned placement", || async {
        Ok((placement_state(&f, placement_id).await? == "cleaned").then_some(()))
    })
    .await?;
    let held: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM capacity_allocations WHERE placement_id=? AND state!='released'",
    )
    .bind(placement_id.to_string())
    .fetch_one(&f.database)
    .await?;
    assert_eq!(held, 0, "capacity was not released");
    // A placement that was told to start is never released by abandonment;
    // only cleanup can prove its runner gone.
    assert!(!placement::abandon_unstarted(&f.database, placement_id).await?);
    assert_eq!(
        *runtime
            .operations
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?,
        ["prepare_environment", "start_runner", "inspect", "cleanup"]
    );

    let _ = stop.send(());
    assert_eq!(running.await??, Stop::Shutdown);
    Ok(())
}

#[tokio::test]
async fn pool_runs_on_several_platforms_and_keeps_unchanged_ones() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let save = |platforms: Value| {
        f.client
            .put(f.url("/pools/fleet-pool/profile"))
            .header(header::COOKIE, &f.cookie)
            .header(header::ORIGIN, &f.origin)
            .json(&json!({
                "enabled": true, "mode": "ephemeral", "cpuMillis": 1000, "memoryMib": 1024,
                "diskBytes": 0, "platforms": platforms,
            }))
            .send()
    };
    // Without an image, a Docker platform runs the pool's own Docker image.
    let linux = json!({"runtimeKind": "docker", "executionOs": "linux", "architecture": "arm64"});
    let windows = json!({"runtimeKind": "native_process", "executionOs": "windows", "architecture": "x64", "image": "native:actions-runner"});
    let saved: Value = save(json!([linux, windows])).await?.json().await?;
    let platforms = saved["item"]["platforms"].as_array().context("platforms")?;
    assert_eq!(platforms.len(), 2);
    assert_eq!(platforms[0]["image"], "runner:fixture");
    let (linux_id, windows_id) = (platforms[0]["id"].clone(), platforms[1]["id"].clone());

    // The same platform twice is refused.
    assert_eq!(
        save(json!([linux, linux])).await?.status(),
        StatusCode::BAD_REQUEST
    );

    // Dropping Windows archives it; Linux keeps its ID and revision.
    let mut kept = linux.clone();
    kept["id"] = linux_id.clone();
    let saved: Value = save(json!([kept])).await?.json().await?;
    assert_eq!(saved["item"]["platforms"].as_array().map(Vec::len), Some(1));
    assert_eq!(saved["item"]["platforms"][0]["id"], linux_id);
    assert_eq!(saved["item"]["platforms"][0]["revision"], 1);
    let archived: (bool, Option<i64>) =
        sqlx::query_as("SELECT enabled,archived_at FROM pool_execution_profiles WHERE id=?")
            .bind(windows_id.as_str().context("windows id")?)
            .fetch_one(&f.database)
            .await?;
    assert!(!archived.0 && archived.1.is_some());

    // Matching labels: a Windows x64 job needs the Windows platform.
    let requested = [
        "self-hosted".to_owned(),
        "Windows".to_owned(),
        "X64".to_owned(),
    ];
    assert!(gridops_core::fleet::targets::platform_supports_labels(
        gridops_core::fleet::ExecutionOs::Windows,
        gridops_core::fleet::Architecture::X64,
        &requested,
        &[],
    ));
    assert!(!gridops_core::fleet::targets::platform_supports_labels(
        gridops_core::fleet::ExecutionOs::Linux,
        gridops_core::fleet::Architecture::Arm64,
        &requested,
        &[],
    ));
    Ok(())
}

#[tokio::test]
async fn waiting_runners_explain_what_is_missing() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let profile: Value = f
        .client
        .put(f.url("/pools/fleet-pool/profile"))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin)
        .json(&json!({
            "enabled": true, "mode": "ephemeral", "cpuMillis": 1000, "memoryMib": 1024, "diskBytes": 0,            "platforms": [{"runtimeKind": "docker", "executionOs": "linux", "architecture": "arm64", "image": "runner:fixture"}],
        }))
        .send()
        .await?
        .json()
        .await?;
    let service = FleetService::new(f.database.clone());
    service
        .submit_intent(
            &Principal::Reconciler,
            SubmitIntent {
                idempotency_key: "runner:waiting".into(),
                pool_id: "fleet-pool".into(),
                profile_id: profile["item"]["platforms"][0]["id"]
                    .as_str()
                    .context("profile")?
                    .parse()?,
                target_id: profile["item"]["targetIds"][0]
                    .as_str()
                    .context("target")?
                    .parse()?,
                workload_id: WorkloadId::try_new(Uuid::new_v4())?,
                generation: Generation::new(1)?,
                kind: WorkloadKind::CiRunner,
                agent_run_id: None,
                expected_profile_revision: Some(1),
            },
        )
        .await?;
    assert_eq!(
        service
            .admit_queued(&Principal::Reconciler, 10)
            .await?
            .admitted,
        0
    );
    let queue: Value = f
        .client
        .get(f.url("/pools/fleet-pool/queue"))
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(queue["items"][0]["reason"], "no_active_machine");
    // With no machine able to take it, "machines first" uses the built-in host.
    assert!(
        !gridops_core::fleet::targets::machine_ready(
            &f.database,
            "fleet-pool",
            profile["item"]["platforms"][0]["id"]
                .as_str()
                .context("profile")?
        )
        .await?
    );
    Ok(())
}

#[tokio::test]
async fn pools_take_turns_when_work_waits() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    sqlx::raw_sql(
        "INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,provider,providers,labels,image,docker_image,tart_image,created_at,updated_at)
           VALUES ('other-pool',1,10,'other','repository','docker','[\"docker\"]','[]','runner:fixture','runner:fixture','macos',1,1);
         INSERT INTO runner_pool_repositories (pool_id,repository_id,created_at) VALUES ('other-pool',10,1);",
    )
    .execute(&f.database)
    .await?;
    let mut profiles = Vec::new();
    for pool in ["fleet-pool", "other-pool"] {
        let profile: Value = f
            .client
            .put(f.url(&format!("/pools/{pool}/profile")))
            .header(header::COOKIE, &f.cookie)
            .header(header::ORIGIN, &f.origin)
            .json(&json!({
                "enabled": true, "mode": "ephemeral", "cpuMillis": 1000, "memoryMib": 1024, "diskBytes": 0,                "platforms": [{"runtimeKind": "docker", "executionOs": "linux", "architecture": "arm64", "image": "runner:fixture"}],
            }))
            .send()
            .await?
            .json()
            .await?;
        profiles.push((pool, profile));
    }
    let service = FleetService::new(f.database.clone());
    // The busy pool queues three runners before the other pool queues one.
    for (index, (pool, profile)) in [&profiles[0], &profiles[0], &profiles[0], &profiles[1]]
        .into_iter()
        .enumerate()
    {
        service
            .submit_intent(
                &Principal::Reconciler,
                SubmitIntent {
                    idempotency_key: format!("runner:turn-{index}"),
                    pool_id: (*pool).into(),
                    profile_id: profile["item"]["platforms"][0]["id"]
                        .as_str()
                        .context("profile")?
                        .parse()?,
                    target_id: profile["item"]["targetIds"][0]
                        .as_str()
                        .context("target")?
                        .parse()?,
                    workload_id: WorkloadId::try_new(Uuid::new_v4())?,
                    generation: Generation::new(1)?,
                    kind: WorkloadKind::CiRunner,
                    agent_run_id: None,
                    expected_profile_revision: Some(1),
                },
            )
            .await?;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // Two admission attempts: one per pool, not the busy pool's first two.
    service.admit_queued(&Principal::Reconciler, 2).await?;
    let considered: Vec<String> = sqlx::query_scalar(
        "SELECT pool_id FROM workload_intents WHERE reason_code IS NOT NULL ORDER BY pool_id",
    )
    .fetch_all(&f.database)
    .await?;
    assert_eq!(considered, ["fleet-pool", "other-pool"]);
    Ok(())
}
