//! A runner whose start fails on one machine is started again on another,
//! and a machine that keeps failing starts stops being chosen for a while.
//! The motivating fault is a machine that drops packets: it looks healthy,
//! but a share of the runners placed on it never start. Here one machine's
//! runtime fails every start, as the fake runtime is told to; the other is
//! healthy. The agents and the API are real.

use std::sync::{Arc, atomic::Ordering};

use gridops_core::fleet::{
    FleetService, Principal,
    ids::PlacementId,
    retry::{self, NotRetried, RetryOutcome},
};

use super::{
    placement::{
        FakeDocker, Machine, active_machine, admitted_placement, eventually, issue_start,
        placement_state, prepared_placement, seed_pool,
    },
    *,
};

/// A runtime whose every start fails.
fn failing_runtime() -> Arc<FakeDocker> {
    let runtime = FakeDocker::default();
    runtime.fail_starts.store(true, Ordering::SeqCst);
    Arc::new(runtime)
}

/// A healthy and a failing machine. Enrolling the second saves the pool's
/// profile again, which replaces the platform the first one recorded.
async fn two_machines(f: &Fixture) -> Result<(Machine, Machine)> {
    let mut flaky = active_machine(f, &f.origin, failing_runtime()).await?;
    let healthy = active_machine(f, &f.origin, Arc::new(FakeDocker::default())).await?;
    flaky.profile_id.clone_from(&healthy.profile_id);
    // The flaky machine has more room, so a new runner goes there first.
    budget(f, &flaky, 8).await?;
    budget(f, &healthy, 4).await?;
    Ok((flaky, healthy))
}

/// Gives the machine room for `runners` runners of the pool, so the roomier
/// machine is the one admission tries first.
async fn budget(f: &Fixture, machine: &Machine, runners: i64) -> Result<()> {
    sqlx::query(
        "UPDATE host_resource_domains SET cpu_millis=?,revision=revision+1
          WHERE host_id=? AND parent_domain_id IS NULL",
    )
    .bind(runners * 1000)
    .bind(machine.host.to_string())
    .execute(&f.database)
    .await?;
    Ok(())
}

async fn host_of(f: &Fixture, placement: PlacementId) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT host_id FROM workload_placements WHERE id=?")
            .bind(placement.to_string())
            .fetch_one(&f.database)
            .await?,
    )
}

/// Starts the prepared placement and waits for the machine's answer to fail it.
async fn fail_start(f: &Fixture, placement: PlacementId, workload: Uuid) -> Result<()> {
    issue_start(f, placement, workload, 30_000).await?;
    eventually("failed start", || async {
        Ok((placement_state(f, placement).await? == "failed").then_some(()))
    })
    .await
}

/// The placement of `runner` once admission has placed it.
async fn placement_of(f: &Fixture, runner: &str) -> Result<Option<PlacementId>> {
    Ok(
        sqlx::query_scalar::<_, String>("SELECT id FROM workload_placements WHERE workload_id=?")
            .bind(runner)
            .fetch_optional(&f.database)
            .await?
            .map(|id| id.parse())
            .transpose()?,
    )
}

async fn runner_status(f: &Fixture, runner: &str) -> Result<String> {
    Ok(sqlx::query_scalar("SELECT status FROM runners WHERE id=?")
        .bind(runner)
        .fetch_one(&f.database)
        .await?)
}

async fn retried_events(f: &Fixture, placement: PlacementId) -> Result<Vec<(String, Value)>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT host_id,detail_json FROM fleet_events
          WHERE placement_id=? AND kind='placement_retried' ORDER BY cursor",
    )
    .bind(placement.to_string())
    .fetch_all(&f.database)
    .await?;
    rows.into_iter()
        .map(|(host, detail)| Ok((host, serde_json::from_str(&detail)?)))
        .collect()
}

async fn host_read(f: &Fixture, machine: &Machine) -> Result<Value> {
    let read: Value = f
        .client
        .get(f.url(&format!("/hosts/{}", machine.host)))
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(read["item"].clone())
}

/// Admits whatever is queued and returns the queued intent's reason, if any
/// intent is still waiting.
async fn admit(f: &Fixture) -> Result<Option<String>> {
    FleetService::new(f.database.clone())
        .admit_queued(&Principal::Reconciler, 10)
        .await?;
    Ok(sqlx::query_scalar(
        "SELECT reason_code FROM workload_intents WHERE status='queued' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_optional(&f.database)
    .await?
    .flatten())
}

#[tokio::test]
async fn a_start_that_fails_on_one_machine_is_retried_on_the_healthy_one() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let (flaky, healthy) = two_machines(&f).await?;

    let workload = Uuid::new_v4();
    let first = prepared_placement(&f, &flaky, workload).await?;
    assert_eq!(host_of(&f, first).await?, flaky.host.to_string());
    fail_start(&f, first, workload).await?;

    let RetryOutcome::Retried(retry) = retry::retry_failed_start(&f.database, first).await? else {
        bail!("the failed start was not retried");
    };
    assert_eq!(retry.attempt, 2);
    // The runner that failed is retired; its replacement waits for a machine.
    assert_eq!(runner_status(&f, &workload.to_string()).await?, "failed");
    assert_eq!(runner_status(&f, &retry.runner_id).await?, "starting");
    assert_eq!(admit(&f).await?, None);
    let second = placement_of(&f, &retry.runner_id)
        .await?
        .context("retry placed")?;
    assert_eq!(host_of(&f, second).await?, healthy.host.to_string());

    // The retry is recorded on the machine where the start failed.
    let events = retried_events(&f, first).await?;
    assert_eq!(events.len(), 1, "{events:?}");
    let (host, detail) = &events[0];
    assert_eq!(host, &flaky.host.to_string());
    assert_eq!(detail["attempt"], 2);
    assert_eq!(detail["max_attempts"], 3);
    assert_eq!(detail["failure_reason"], "agent_reported");
    assert_eq!(detail["retry_runner_id"], retry.runner_id);
    assert_eq!(detail["avoided_host_ids"], json!([flaky.host.to_string()]));

    // A failed placement is retried once however often the reconciler asks.
    assert!(matches!(
        retry::retry_failed_start(&f.database, first).await?,
        RetryOutcome::NotRetried(NotRetried::RunnerNotWanted | NotRetried::AlreadyRetried)
    ));
    assert_eq!(retried_events(&f, first).await?.len(), 1);
    let runners: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM runners WHERE pool_id='fleet-pool'")
            .fetch_one(&f.database)
            .await?;
    assert_eq!(runners, 2);
    Ok(())
}

#[tokio::test]
async fn retries_go_back_to_the_only_machine_and_stop_after_the_last_attempt() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let flaky = active_machine(&f, &f.origin, failing_runtime()).await?;
    budget(&f, &flaky, 8).await?;

    let workload = Uuid::new_v4();
    let mut placement = prepared_placement(&f, &flaky, workload).await?;
    let mut runner = workload;
    for attempt in 2..=3 {
        fail_start(&f, placement, runner).await?;
        let RetryOutcome::Retried(retry) =
            retry::retry_failed_start(&f.database, placement).await?
        else {
            bail!("attempt {attempt} was not made");
        };
        assert_eq!(retry.attempt, attempt);
        // No other machine can take it, and this one is not set aside yet,
        // so the retry comes back here.
        assert_eq!(admit(&f).await?, None);
        placement = placement_of(&f, &retry.runner_id)
            .await?
            .context("retry placed")?;
        assert_eq!(host_of(&f, placement).await?, flaky.host.to_string());
        eventually("prepared retry", || async {
            Ok((placement_state(&f, placement).await? == "registering").then_some(()))
        })
        .await?;
        runner = retry.runner_id.parse()?;
    }
    fail_start(&f, placement, runner).await?;
    assert!(matches!(
        retry::retry_failed_start(&f.database, placement).await?,
        RetryOutcome::NotRetried(NotRetried::AttemptsExhausted)
    ));
    // Nothing replaces it; the reconciler retires the runner as before.
    assert_eq!(runner_status(&f, &runner.to_string()).await?, "starting");
    Ok(())
}

#[tokio::test]
async fn a_start_that_is_not_proven_gone_is_never_retried() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let flaky = active_machine(&f, &f.origin, failing_runtime()).await?;
    let placement = prepared_placement(&f, &flaky, Uuid::new_v4()).await?;
    // A placement marked failed, with a start failure recorded, but without
    // proof that nothing of it can run on the machine: a second placement
    // could run alongside it.
    sqlx::query("UPDATE workload_placements SET state='failed' WHERE id=?")
        .bind(placement.to_string())
        .execute(&f.database)
        .await?;
    sqlx::query(
        "INSERT INTO fleet_events (id,host_id,placement_id,target_id,kind,detail_json,created_at)
         SELECT ?,host_id,id,target_id,'placement_failed',?,? FROM workload_placements WHERE id=?",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(json!({"reason": "start_deadline_expired", "message": "Timed out."}).to_string())
    .bind(gridops_core::now_millis())
    .bind(placement.to_string())
    .execute(&f.database)
    .await?;
    assert!(matches!(
        retry::retry_failed_start(&f.database, placement).await?,
        RetryOutcome::NotRetried(NotRetried::NotProvenAbsent)
    ));
    // An uncertain placement may still be starting: not retried either.
    sqlx::query("UPDATE workload_placements SET state='uncertain' WHERE id=?")
        .bind(placement.to_string())
        .execute(&f.database)
        .await?;
    assert!(matches!(
        retry::retry_failed_start(&f.database, placement).await?,
        RetryOutcome::NotRetried(NotRetried::NotFailed)
    ));
    let intents: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workload_intents")
        .fetch_one(&f.database)
        .await?;
    assert_eq!(intents, 1);
    Ok(())
}

#[tokio::test]
async fn a_machine_that_keeps_failing_starts_is_set_aside_and_says_why() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let (flaky, healthy) = two_machines(&f).await?;

    // The flaky machine is the roomiest, so it takes runner after runner.
    for _ in 0..3 {
        let workload = Uuid::new_v4();
        let placement = prepared_placement(&f, &flaky, workload).await?;
        assert_eq!(host_of(&f, placement).await?, flaky.host.to_string());
        fail_start(&f, placement, workload).await?;
    }
    // After three failed starts it stops being chosen.
    let placement = admitted_placement(&f, &healthy, Uuid::new_v4()).await?;
    assert_eq!(host_of(&f, placement).await?, healthy.host.to_string());

    let read = host_read(&f, &flaky).await?;
    assert_eq!(read["startGate"]["failures"], 3);
    assert_eq!(read["startGate"]["windowMinutes"], 15);
    assert_eq!(
        read["startGate"]["message"],
        "Not accepting new runners: 3 start failures in the last 15 minutes."
    );
    assert!(read["startGate"]["until"].is_string());
    // Being set aside changes nothing else about the machine.
    assert_eq!(read["lifecycle"], "active");
    assert_eq!(host_read(&f, &healthy).await?["startGate"], Value::Null);
    Ok(())
}

#[tokio::test]
async fn a_machine_set_aside_is_reconsidered_after_a_success_or_the_cool_down() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let runtime = failing_runtime();
    let only = active_machine(&f, &f.origin, Arc::clone(&runtime)).await?;
    budget(&f, &only, 8).await?;

    // One runner is prepared before the failures and started after them.
    let waiting_workload = Uuid::new_v4();
    let waiting = prepared_placement(&f, &only, waiting_workload).await?;
    for _ in 0..3 {
        let workload = Uuid::new_v4();
        let placement = prepared_placement(&f, &only, workload).await?;
        fail_start(&f, placement, workload).await?;
    }
    // The only machine is set aside: the next runner waits, and says why.
    sqlx::query("UPDATE runner_pools SET max_count=max_count+1 WHERE id='fleet-pool'")
        .execute(&f.database)
        .await?;
    let queued = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runners (id,pool_id,name,provider,ci_platform,os,architecture,status,ephemeral,configuration_version,created_at,updated_at)
         VALUES (?,'fleet-pool','fleet-runner','docker','github','linux','ARM64','starting',1,1,1,1)",
    )
    .bind(queued.to_string())
    .execute(&f.database)
    .await?;
    FleetService::new(f.database.clone())
        .submit_intent(
            &Principal::Reconciler,
            gridops_core::fleet::SubmitIntent {
                idempotency_key: format!("runner:{queued}"),
                pool_id: "fleet-pool".into(),
                profile_id: only.profile_id.parse()?,
                target_id: only.target.parse()?,
                workload_id: gridops_core::fleet::ids::WorkloadId::try_new(queued)?,
                generation: gridops_core::fleet::ids::Generation::new(1)?,
                kind: gridops_core::fleet::WorkloadKind::CiRunner,
                agent_run_id: None,
                expected_profile_revision: Some(1),
            },
        )
        .await?;
    assert_eq!(admit(&f).await?.as_deref(), Some("machines_failing_starts"));
    assert_eq!(host_read(&f, &only).await?["startGate"]["failures"], 3);

    // A start that succeeds there shows the machine works again.
    runtime.fail_starts.store(false, Ordering::SeqCst);
    issue_start(&f, waiting, waiting_workload, 30_000).await?;
    eventually("successful start", || async {
        Ok((placement_state(&f, waiting).await? == "running").then_some(()))
    })
    .await?;
    assert_eq!(host_read(&f, &only).await?["startGate"], Value::Null);
    assert_eq!(admit(&f).await?, None);
    assert!(placement_of(&f, &queued.to_string()).await?.is_some());

    // Three more failures set it aside again; once they are older than the
    // window, it is reconsidered without anything else happening.
    runtime.fail_starts.store(true, Ordering::SeqCst);
    for _ in 0..3 {
        let workload = Uuid::new_v4();
        let placement = prepared_placement(&f, &only, workload).await?;
        fail_start(&f, placement, workload).await?;
    }
    assert_eq!(host_read(&f, &only).await?["startGate"]["failures"], 3);
    sqlx::query(
        "UPDATE fleet_events SET created_at=created_at-16*60000
          WHERE host_id=? AND kind='placement_failed'",
    )
    .bind(only.host.to_string())
    .execute(&f.database)
    .await?;
    assert_eq!(host_read(&f, &only).await?["startGate"], Value::Null);
    Ok(())
}
