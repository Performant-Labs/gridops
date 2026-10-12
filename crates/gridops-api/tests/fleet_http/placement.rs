//! A runner's whole fleet life against the API binary and the real agent:
//! profile, enrollment, approval, activation, admission, prepare, start with
//! one-time registration material, inspection and cleanup. Only the container
//! runtime is a stand-in.

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
    helper::{HelperRequest, HelperResponse, RuntimeHelper},
    journal::Journal,
    supervisor::{Stop, Supervisor},
};
use tokio::sync::oneshot;

use super::*;

/// Plays a Docker host whose runner exits as soon as it's inspected.
#[derive(Default)]
struct FakeDocker {
    bootstraps: Mutex<Vec<String>>,
    operations: Mutex<Vec<&'static str>>,
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
async fn admitted_runner_is_prepared_started_inspected_and_cleaned_on_its_host() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;

    let response = f
        .client
        .put(f.url("/pools/fleet-pool/profile"))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin)
        .json(&json!({
            "enabled": true, "runtimeKind": "docker", "executionOs": "linux",
            "architecture": "arm64", "mode": "ephemeral", "image": "runner:fixture",
            "labels": ["self-hosted"], "cpuMillis": 1000, "memoryMib": 1024, "diskBytes": 0,
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

    let service = FleetService::new(f.database.clone());
    let profile_id = profile["item"]["id"].as_str().context("profile id")?;
    service
        .submit_intent(
            &Principal::Reconciler,
            SubmitIntent {
                idempotency_key: "runner:placement-test".into(),
                pool_id: "fleet-pool".into(),
                profile_id: profile_id.parse()?,
                target_id: target.parse()?,
                workload_id: WorkloadId::try_new(Uuid::new_v4())?,
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
