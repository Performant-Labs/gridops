//! A machine's physical budget is the room that may be filled on it. An
//! administrator, or a token with `fleet:operate`, raises or lowers it through
//! the API; it can never exceed what the machine reports, and it is audited.

use super::{
    api_tokens::{member_cookie, member_fixture, mint, token_get},
    placement::{
        FakeDocker, Machine, active_machine, admitted_placement, placement_state, seed_pool,
    },
    *,
};
use gridops_core::{
    api_tokens::ApiTokenScope,
    fleet::{
        FleetService, Principal, SubmitIntent, WorkloadKind,
        ids::{Generation, WorkloadId},
    },
};
use std::sync::Arc;

fn admin_put(f: &Fixture, path: &str) -> reqwest::RequestBuilder {
    f.client
        .put(f.url(path))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin)
}

async fn budget_row(f: &Fixture, host: &str) -> Result<(i64, i64, i64)> {
    Ok(sqlx::query_as(
        "SELECT cpu_millis,memory_mib,disk_bytes FROM host_resource_domains
          WHERE host_id=? AND parent_domain_id IS NULL",
    )
    .bind(host)
    .fetch_one(&f.database)
    .await?)
}

async fn reported_row(f: &Fixture, host: &str) -> Result<(i64, i64, i64)> {
    Ok(sqlx::query_as(
        "SELECT o.cpu_millis,o.memory_mib,COALESCE(o.disk_bytes,0)
           FROM fleet_observed_domains o JOIN host_resource_domains d ON d.id=o.domain_id
          WHERE d.host_id=? AND d.parent_domain_id IS NULL",
    )
    .bind(host)
    .fetch_one(&f.database)
    .await?)
}

async fn stop(machine: Machine) -> Result<()> {
    let Machine { stop, running, .. } = machine;
    let _ = stop.send(());
    running.await??;
    Ok(())
}

#[tokio::test]
async fn a_machines_budget_is_set_through_the_api_within_what_it_reports() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let machine = active_machine(&f, &f.origin, Arc::new(FakeDocker::default())).await?;
    let host = machine.host.to_string();
    let (reported_cpu, reported_memory, _) = reported_row(&f, &host).await?;
    let (_, _, disk_before) = budget_row(&f, &host).await?;

    // Lowering the budget takes effect at once and leaves disk alone.
    let lowered = admin_put(&f, &format!("/hosts/{host}/budget"))
        .json(&json!({"cpuMillis": 1500, "memoryMib": 3072}))
        .send()
        .await?;
    assert_eq!(lowered.status(), StatusCode::OK);
    let body: Value = lowered.json().await?;
    assert_eq!(body["budget"]["cpuMillis"], 1500);
    assert_eq!(body["budget"]["memoryMib"], 3072);
    assert_eq!(body["budget"]["diskBytes"], disk_before);
    assert_eq!(
        budget_row(&f, &host).await?,
        (1500, 3072, disk_before),
        "stored budget"
    );

    // The machine page and API read the same figures back.
    let detail: Value = f
        .client
        .get(f.url(&format!("/hosts/{host}")))
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(detail["item"]["physicalBudget"]["cpuMillis"], 1500);
    assert_eq!(detail["item"]["physicalBudget"]["memoryMib"], 3072);

    // One dimension alone changes only that dimension.
    let memory_only = admin_put(&f, &format!("/hosts/{host}/budget"))
        .json(&json!({"memoryMib": 2048}))
        .send()
        .await?;
    assert_eq!(memory_only.status(), StatusCode::OK);
    assert_eq!(budget_row(&f, &host).await?, (1500, 2048, disk_before));

    // Raising up to exactly what the machine reports is allowed.
    let full = admin_put(&f, &format!("/hosts/{host}/budget"))
        .json(&json!({"cpuMillis": reported_cpu, "memoryMib": reported_memory}))
        .send()
        .await?;
    assert_eq!(full.status(), StatusCode::OK);
    assert_eq!(
        budget_row(&f, &host).await?,
        (reported_cpu, reported_memory, disk_before)
    );

    // One audit record per change, naming the figures before and after.
    let audited: Vec<(String, String)> = sqlx::query_as(
        "SELECT target_id,metadata FROM audit_events
          WHERE action='fleet.host.budget_changed' ORDER BY created_at,rowid",
    )
    .fetch_all(&f.database)
    .await?;
    assert_eq!(audited.len(), 3);
    assert!(audited.iter().all(|(target, _)| *target == host));
    let first: Value = serde_json::from_str(&audited[0].1)?;
    assert_eq!(first["after"]["cpuMillis"], 1500);
    assert_eq!(
        first["before"]["cpuMillis"].as_i64(),
        Some(reported_cpu / 5 * 4)
    );

    stop(machine).await
}

#[tokio::test]
async fn a_budget_beyond_the_machine_or_empty_is_refused_and_changes_nothing() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let machine = active_machine(&f, &f.origin, Arc::new(FakeDocker::default())).await?;
    let host = machine.host.to_string();
    let before = budget_row(&f, &host).await?;
    let (reported_cpu, reported_memory, _) = reported_row(&f, &host).await?;

    for body in [
        json!({}),
        json!({"cpuMillis": 0}),
        json!({"memoryMib": 0}),
        json!({"cpuMillis": reported_cpu + 1}),
        json!({"memoryMib": reported_memory + 1}),
        // Disk is not reported by this fixture, so there is nothing to bound it by.
        json!({"diskBytes": 1_000_000_000_000_u64}),
        json!({"cpuMillis": 1000, "unknown": 1}),
        json!({"cpuMillis": -5}),
    ] {
        let response = admin_put(&f, &format!("/hosts/{host}/budget"))
            .json(&body)
            .send()
            .await?;
        error(response, StatusCode::BAD_REQUEST, "invalid_request").await?;
    }
    assert_eq!(budget_row(&f, &host).await?, before);
    let audited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_events WHERE action='fleet.host.budget_changed'",
    )
    .fetch_one(&f.database)
    .await?;
    assert_eq!(audited, 0);

    let missing = admin_put(&f, &format!("/hosts/{}/budget", Uuid::new_v4()))
        .json(&json!({"cpuMillis": 1000}))
        .send()
        .await?;
    error(missing, StatusCode::NOT_FOUND, "not_found").await?;
    stop(machine).await
}

#[tokio::test]
async fn only_administrators_and_operate_tokens_may_change_a_budget() -> Result<()> {
    let f = member_fixture().await?;
    seed_pool(&f).await?;
    let machine = active_machine(&f, &f.origin, Arc::new(FakeDocker::default())).await?;
    let host = machine.host.to_string();
    let before = budget_row(&f, &host).await?;
    let path = format!("/hosts/{host}/budget");

    // A member of the organisation is refused.
    let cookie = member_cookie(&f).await?;
    let member = f
        .client
        .put(f.url(&path))
        .header(header::COOKIE, &cookie)
        .header(header::ORIGIN, &f.origin)
        .json(&json!({"cpuMillis": 1000}))
        .send()
        .await?;
    error(member, StatusCode::FORBIDDEN, "forbidden").await?;

    // Not signed in at all.
    let anonymous = f
        .client
        .put(f.url(&path))
        .header(header::ORIGIN, &f.origin)
        .json(&json!({"cpuMillis": 1000}))
        .send()
        .await?;
    error(anonymous, StatusCode::UNAUTHORIZED, "unauthenticated").await?;

    // A read-only token names the scope it lacks.
    let reader = mint(&f, "reader", &[ApiTokenScope::FleetRead]).await?;
    let denied = f
        .client
        .put(f.url(&path))
        .header(header::ORIGIN, &f.origin)
        .bearer_auth(&reader)
        .json(&json!({"cpuMillis": 1000}))
        .send()
        .await?;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let denied: Value = denied.json().await?;
    assert_eq!(denied["details"]["requiredScope"], "fleet:operate");
    assert_eq!(budget_row(&f, &host).await?, before);

    // A token with fleet:operate may, and the audit names the token.
    let operator = mint(&f, "budget-bot", &[ApiTokenScope::FleetOperate]).await?;
    let allowed = f
        .client
        .put(f.url(&path))
        .header(header::ORIGIN, &f.origin)
        .bearer_auth(&operator)
        .json(&json!({"cpuMillis": 1000}))
        .send()
        .await?;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(budget_row(&f, &host).await?.0, 1000);
    let label: String = sqlx::query_scalar(
        "SELECT actor_label FROM audit_events WHERE action='fleet.host.budget_changed'",
    )
    .fetch_one(&f.database)
    .await?;
    assert!(label.contains("token: budget-bot"), "{label}");
    // The read scope still reads the new figure.
    let read: Value = token_get(&f, &format!("/hosts/{host}"), &reader)
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(read["item"]["physicalBudget"]["cpuMillis"], 1000);
    stop(machine).await
}

/// Lowering a budget below what runners already hold leaves them alone and
/// stops new placements; raising it again lets the next one in.
#[tokio::test]
async fn a_lowered_budget_stops_new_placements_but_not_running_ones() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let machine = active_machine(&f, &f.origin, Arc::new(FakeDocker::default())).await?;
    let host = machine.host.to_string();
    let path = format!("/hosts/{host}/budget");
    let (reported_cpu, _, _) = reported_row(&f, &host).await?;
    // The profile asks 1000 millicores and 1024 MiB per runner; room for one.
    let first = admitted_placement(&f, &machine, Uuid::new_v4()).await?;
    let resize = |body: Value| {
        let request = admin_put(&f, &path).json(&body);
        async move { anyhow::Ok(request.send().await?.status()) }
    };
    assert_eq!(resize(json!({"cpuMillis": 1000})).await?, StatusCode::OK);

    // A second runner for the same pool, with the pool allowed two.
    let workload = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runners (id,pool_id,name,provider,ci_platform,os,architecture,status,ephemeral,configuration_version,created_at,updated_at)
         VALUES (?,'fleet-pool','fleet-runner-2','docker','github','linux','ARM64','starting',1,1,1,1)",
    )
    .bind(workload.to_string())
    .execute(&f.database)
    .await?;
    sqlx::query("UPDATE runner_pools SET max_count=2 WHERE id='fleet-pool'")
        .execute(&f.database)
        .await?;
    let service = FleetService::new(f.database.clone());
    service
        .submit_intent(
            &Principal::Reconciler,
            SubmitIntent {
                idempotency_key: format!("runner:{workload}"),
                pool_id: "fleet-pool".into(),
                profile_id: machine.profile_id.parse()?,
                target_id: machine.target.parse()?,
                workload_id: WorkloadId::try_new(workload)?,
                generation: Generation::new(1)?,
                kind: WorkloadKind::CiRunner,
                agent_run_id: None,
                expected_profile_revision: Some(1),
            },
        )
        .await?;
    let reason = || async {
        anyhow::Ok(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT reason_code FROM workload_intents WHERE workload_id=?",
            )
            .bind(workload.to_string())
            .fetch_one(&f.database)
            .await?,
        )
    };
    // Exactly full: nothing more fits.
    assert_eq!(
        service
            .admit_queued(&Principal::Reconciler, 10)
            .await?
            .admitted,
        0
    );
    assert_eq!(
        reason().await?.as_deref(),
        Some("physical_resource_exhausted")
    );

    // Below what the first runner holds: accepted, the runner is untouched,
    // and the second still waits.
    assert_eq!(resize(json!({"cpuMillis": 500})).await?, StatusCode::OK);
    assert_eq!(
        service
            .admit_queued(&Principal::Reconciler, 10)
            .await?
            .admitted,
        0
    );
    assert_eq!(
        reason().await?.as_deref(),
        Some("physical_resource_exhausted")
    );
    assert_ne!(placement_state(&f, first).await?, "released");

    // Room for two (when the machine has two cores to give): the next one is placed.
    if reported_cpu >= 2000 {
        assert_eq!(resize(json!({"cpuMillis": 2000})).await?, StatusCode::OK);
        assert_eq!(
            service
                .admit_queued(&Principal::Reconciler, 10)
                .await?
                .admitted,
            1
        );
    }
    stop(machine).await
}
