mod agent;
mod builtin_host;
mod fleet;

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context as _, Result, bail};
use futures_util::StreamExt as _;
use gridops_core::{
    BitbucketClient, BitbucketRunnerTarget, Config, GitHubClient, GitHubWorkflowJob,
    GitHubWorkflowRun, JitRequest, ProviderCapacity, RepositoryCapacity, RunnerTarget, Vault,
    WorkflowJobPage, WorkflowRunPage, assigned_queued_jobs, associate_runner_with_job,
    connect_database, effective_runner_labels, next_runner_provider, next_runner_repository,
    now_millis, provider_capacities, provider_capacity_deficit, repository_capacities,
    repository_capacity_deficit, scale_up_target,
};
use reqwest::{Method, StatusCode};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::{FromRow, Row as _, SqliteConnection, SqlitePool};
use tokio::io::AsyncWriteExt as _;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _};

const MAX_ARCHIVED_LOG_BYTES: i64 = 100 * 1_024 * 1_024;

#[derive(Clone)]
struct Reconciler {
    config: Config,
    database: SqlitePool,
    github: GitHubClient,
    bitbucket: BitbucketClient,
    vault: Vault,
    http: reqwest::Client,
    /// The pass under way (its start, in milliseconds), sent with admissions
    /// so a pool waiting for a macOS VM slot keeps its place in line.
    pass: Arc<AtomicI64>,
}

#[derive(Debug, FromRow)]
struct Pool {
    id: String,
    installation_id: i64,
    account_login: String,
    scope: String,
    name: String,
    state: String,
    mode: String,
    provider: String,
    providers: String,
    labels: String,
    docker_image: String,
    tart_image: String,
    /// "vm" or "native": how the macOS agent should execute this pool's runners.
    /// Ignored by the Docker provider.
    macos_runtime: String,
    desired_count: i64,
    min_count: i64,
    max_count: i64,
    cpu_limit: f64,
    memory_limit_mb: i64,
    runner_group_id: i64,
    ephemeral: bool,
    paused: bool,
    autoscaling_enabled: bool,
    queue_scale_factor: i64,
    idle_timeout_minutes: i64,
    configuration_version: i64,
    provision_failure_count: i64,
    provision_retry_at: Option<i64>,
    provision_circuit_open: bool,
    /// The pool's last start was deferred (the host had no room, or the Mac
    /// no VM slot) and nothing has happened to it since: a wait reason is
    /// recorded in `runner_pool_waits`.
    wait_recorded: bool,
}

impl Pool {
    /// Whether the pool is pausing before its next start, after a failure,
    /// in its circuit's cool-down, or after a deferred start.
    fn retry_deferred(&self, now: i64) -> bool {
        self.provision_retry_at
            .is_some_and(|retry_at| retry_at > now)
    }
}

#[derive(Clone, Debug, FromRow)]
struct Runner {
    id: String,
    installation_id: i64,
    name: String,
    container_id: Option<String>,
    github_runner_id: Option<i64>,
    ci_platform: String,
    bitbucket_connection_id: Option<String>,
    bitbucket_runner_uuid: Option<String>,
    target_repository_id: Option<i64>,
    repository_owner: Option<String>,
    repository_name: Option<String>,
    provider: String,
    status: String,
    busy: bool,
    last_job_id: Option<i64>,
    configuration_version: i64,
    updated_at: i64,
    /// Placed by the fleet scheduler on an enrolled machine, not by the
    /// manager. Repository and provider rebalancing leaves it alone.
    fleet: bool,
}

#[derive(Debug, FromRow)]
struct BitbucketPoolConnection {
    id: String,
    workspace: String,
    workspace_uuid: String,
    access_token_key: String,
}

#[derive(Debug, FromRow)]
struct Repository {
    id: i64,
    installation_id: i64,
    owner: String,
    name: String,
    full_name: String,
}

#[derive(Debug, FromRow)]
struct PendingGitHubCleanup {
    id: String,
    installation_id: i64,
    target_owner: String,
    target_repository: Option<String>,
    github_runner_id: Option<i64>,
    runner_name: String,
    attempts: i64,
}

#[derive(Debug, Deserialize)]
struct ManagerRunners {
    runners: Vec<ManagedRunner>,
}

#[derive(Debug, Deserialize)]
struct ManagedRunner {
    id: String,
    state: String,
    #[serde(default)]
    labels: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct CreatedRunner {
    id: String,
    name: String,
    state: String,
}

enum ProvisionAttempt {
    Provisioned,
    /// No queued job needs another runner, so none was started.
    NoDemand,
    /// The host had no safe capacity. `full_provider` names the provider whose
    /// runner budget is exhausted, so idle runners of that provider could make
    /// room.
    Deferred {
        reason: Option<String>,
        full_provider: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "gridops_reconciler=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    let config = Config::from_env()?;
    let database = connect_database(&config).await?;
    let github = GitHubClient::new(config.clone())?;
    let bitbucket = BitbucketClient::new()?;
    let vault = Vault::from_config(&config)?;
    let reconciler = Reconciler {
        config,
        database,
        github,
        bitbucket,
        vault,
        http: reqwest::Client::builder()
            .user_agent("GridOps reconciler/0.1")
            .build()?,
        pass: Arc::default(),
    };

    tracing::info!("GridOps Rust reconciler started");
    // Agent runs take minutes, so they get their own task rather than holding
    // up runner reconciliation.
    tokio::spawn(agent::worker(reconciler.clone()));
    // Syncing every repository from GitHub takes over a minute; inside the
    // pass it held up every runner waiting to be told to start.
    tokio::spawn(github_sync_worker(reconciler.clone()));
    loop {
        let started_at = now_millis();
        reconciler.pass.store(started_at, Ordering::Relaxed);
        let started = std::time::Instant::now();
        if let Err(error) = reconcile(&reconciler).await {
            tracing::error!(error = ?error, "reconciliation pass failed");
        }
        let elapsed = started.elapsed();
        if let Err(error) = record_pass(&reconciler.database, started_at, elapsed).await {
            tracing::warn!(error = ?error, "could not record reconciliation pass duration");
        }
        let interval = setting_i64(&reconciler.database, "reconcileIntervalSeconds", 30)
            .await
            .clamp(5, 3_600);
        tokio::time::sleep(Duration::from_secs(interval as u64).saturating_sub(elapsed)).await;
    }
}

/// Polls GitHub for workflow runs and jobs on its own schedule
/// (`githubSyncIntervalSeconds`), so a slow sync never delays a pass.
async fn github_sync_worker(app: Reconciler) {
    loop {
        if let Err(error) = maybe_sync_github(&app).await {
            tracing::warn!(error = ?error, "periodic GitHub workflow sync failed");
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Runs a sync write again, up to three times a second apart, when another
/// writer held the database past its busy timeout. The GitHub sync runs beside
/// reconciler passes, so without this a long write in a pass would drop a page
/// GitHub had already sent.
async fn retry_when_busy<T, F, Fut>(mut write: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    for _ in 0..3 {
        match write().await {
            Err(error) if database_busy(&error) => {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            result => return result,
        }
    }
    write().await
}

/// Whether the database was locked (`SQLITE_BUSY`, in any of its extended forms).
fn database_busy(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<sqlx::Error>()
        .and_then(sqlx::Error::as_database_error)
        .and_then(|error| error.code())
        .and_then(|code| code.parse::<i32>().ok())
        .is_some_and(|code| code & 0xff == 5)
}

/// Keeps how long a pass took, for the overview.
async fn record_pass(database: &SqlitePool, started_at: i64, elapsed: Duration) -> Result<()> {
    sqlx::query("INSERT INTO reconciler_passes (started_at,duration_ms) VALUES (?,?)")
        .bind(started_at)
        .bind(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .execute(database)
        .await?;
    Ok(())
}

async fn reconcile(app: &Reconciler) -> Result<()> {
    if let Err(error) = retry_github_runner_cleanup(app).await {
        tracing::warn!(error = ?error, "deferred GitHub runner cleanup pass failed");
    }
    if let Err(error) = fleet::tick(app).await {
        tracing::warn!(error = ?error, "fleet reconciliation pass failed");
    }
    let provisioning_paused = setting_bool(&app.database, "provisioningPaused", false).await;
    let builtin_enabled = gridops_core::builtin_host::is_enabled(&app.database).await;
    if let Err(error) = gridops_core::builtin_host::sync_pool_states(&app.database).await {
        tracing::warn!(error = ?error, "could not update pools that depend on the built-in runner host");
    }
    // Whether the built-in host may be used and its budget, every pass, so a
    // manager that restarted or missed a change catches up.
    let policy =
        gridops_core::builtin_host::manager_policy(&app.database, provisioning_paused).await;
    if let Err(error) = manager_request::<Value>(app, Method::PUT, "v1/policy", Some(policy)).await
    {
        tracing::warn!(error = ?error, "could not synchronize runner provisioning policy");
    }
    let managed = match manager_request::<ManagerRunners>(app, Method::GET, "v1/runners", None)
        .await
    {
        Ok(value) => value.runners,
        Err(error) => {
            tracing::warn!(error = ?error, "runner manager unavailable; skipping runner reconciliation");
            cleanup_retention(app).await?;
            return Ok(());
        }
    };
    if let Err(error) = cleanup_orphaned_manager_containers(app, &managed).await {
        tracing::warn!(error = ?error, "could not complete orphaned runner container cleanup");
    }
    let container_states = managed
        .into_iter()
        .map(|runner| (runner.id, runner.state))
        .collect::<HashMap<_, _>>();
    let pools = load_pools(&app.database).await?;
    for pool in pools {
        if let Err(error) = reconcile_pool(
            app,
            &pool,
            &container_states,
            provisioning_paused,
            builtin_enabled,
        )
        .await
        {
            tracing::error!(pool_id = %pool.id, pool = %pool.name, error = ?error, "pool reconciliation failed");
            system_event(
                app,
                Some(&pool.id),
                "error",
                "Pool reconciliation failed",
                &error.to_string(),
                json!({}),
            )
            .await?;
        }
    }
    cleanup_retention(app).await?;
    // Agent logs are kept for a day, including for machines that went quiet.
    sqlx::query("DELETE FROM fleet_agent_logs WHERE received_at < ?")
        .bind(now_millis() - gridops_core::fleet::registry::logs::AGENT_LOG_RETENTION_MILLIS)
        .execute(&app.database)
        .await?;
    sqlx::query("DELETE FROM oauth_states WHERE expires_at < ?")
        .bind(now_millis())
        .execute(&app.database)
        .await?;
    sqlx::query("DELETE FROM github_app_manifest_states WHERE expires_at < ?")
        .bind(now_millis())
        .execute(&app.database)
        .await?;
    sqlx::query("DELETE FROM ai_oauth_attempts WHERE expires_at < ?")
        .bind(now_millis())
        .execute(&app.database)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE expires_at < ?")
        .bind(now_millis())
        .execute(&app.database)
        .await?;
    Ok(())
}

async fn load_pools(database: &SqlitePool) -> Result<Vec<Pool>> {
    Ok(sqlx::query_as::<_, Pool>(
        r#"SELECT p.id,p.installation_id,i.account_login,p.scope,p.name,p.state,p.mode,p.provider,p.providers,p.labels,p.docker_image,p.tart_image,p.macos_runtime,p.desired_count,p.min_count,
          p.max_count,CAST(p.cpu_limit AS REAL) AS cpu_limit,p.memory_limit_mb,
          p.runner_group_id,p.ephemeral,p.paused,
          p.autoscaling_enabled,p.queue_scale_factor,p.idle_timeout_minutes,p.configuration_version,
          p.provision_failure_count,p.provision_retry_at,p.provision_circuit_open,
          EXISTS(SELECT 1 FROM runner_pool_waits w WHERE w.pool_id=p.id) AS wait_recorded FROM runner_pools p
          JOIN installations i ON i.id=p.installation_id
          WHERE p.state != 'deleting' ORDER BY p.created_at"#,
    )
    .fetch_all(database)
    .await?)
}

async fn reconcile_pool(
    app: &Reconciler,
    pool: &Pool,
    container_states: &HashMap<String, String>,
    provisioning_paused: bool,
    builtin_enabled: bool,
) -> Result<()> {
    let known_runners = runners(app, &pool.id).await?;
    for runner in &known_runners {
        if let Some(container_id) = &runner.container_id {
            let docker_state = container_states
                .get(container_id)
                .map(String::as_str)
                .unwrap_or("missing");
            let status = match docker_state {
                "running" if runner.busy => "busy",
                "running" => "online",
                "paused" => "paused",
                "exited" | "dead" if runner.status == "stopped" => "stopped",
                "exited" | "dead" | "missing" => "failed",
                value => value,
            };
            let heartbeat = now_millis();
            sqlx::query(
                "UPDATE runners SET status=?,last_heartbeat_at=?,updated_at=CASE WHEN status<>? THEN ? ELSE updated_at END WHERE id=?",
            )
                .bind(status)
                .bind(heartbeat)
                .bind(status)
                .bind(heartbeat)
                .bind(&runner.id)
                .execute(&app.database)
                .await?;
        }
    }

    let failed = runners(app, &pool.id)
        .await?
        .into_iter()
        .filter(|runner| runner.status == "failed")
        .collect::<Vec<_>>();
    for runner in &failed {
        delete_runner(app, pool, runner).await?;
    }

    // With the built-in host off, idle runners on it are wound down; busy ones
    // finish their job first. Nothing replaces them there.
    let mut retired = 0;
    if !builtin_enabled {
        for runner in runners(app, &pool.id)
            .await?
            .iter()
            .filter(|runner| builtin_host::retire_when_off(builtin_enabled, runner))
        {
            if delete_runner(app, pool, runner).await? {
                retired += 1;
            }
        }
    }
    // A pool that only uses Docker and has no fleet platform has nowhere to
    // run while the host is off.
    let host_blocked = !builtin_enabled
        && gridops_core::builtin_host::depends_on_host(&app.database, &pool.id).await?;
    if host_blocked && pool.state != gridops_core::builtin_host::POOL_STATE && !pool.paused {
        system_event(
            app,
            Some(&pool.id),
            "warning",
            "Cannot run: built-in runner host is off",
            gridops_core::builtin_host::POOL_REASON,
            json!({ "code": gridops_core::builtin_host::DISABLED_CODE }),
        )
        .await?;
    }

    let retry_deferred = pool.retry_deferred(now_millis());
    // An open circuit carries a cool-down in `provision_retry_at`; once it
    // passes, one attempt probes whether the cause has cleared.
    let provisioning_blocked = provisioning_paused || retry_deferred || host_blocked;
    let current = runners(app, &pool.id).await?;
    let mut rotated = 0;
    if !provisioning_blocked
        && let Some(stale) = current.iter().find(|runner| {
            runner.ci_platform != "bitbucket" && !runner.busy && runner_needs_update(pool, runner)
        })
    {
        if delete_runner(app, pool, stale).await? {
            rotated = 1;
        }
    }

    let current = runners(app, &pool.id).await?;
    if !provisioning_blocked {
        maybe_scale_up(app, pool, &current).await?;
    }
    maybe_scale_down(app, pool, &current).await?;
    let desired = if pool.paused {
        0
    } else {
        sqlx::query("SELECT desired_count FROM runner_pools WHERE id=?")
            .bind(&pool.id)
            .fetch_one(&app.database)
            .await?
            .get::<i64, _>("desired_count")
            .clamp(pool.min_count, pool.max_count)
    };
    let mut rebalanced = if provisioning_blocked {
        0
    } else {
        rebalance_repository_capacity(app, pool, desired).await?
    };
    let refreshed = runners(app, &pool.id).await?;
    let active = refreshed
        .iter()
        .filter(|runner| active_status(&runner.status))
        .collect::<Vec<_>>();
    let mut provisioned = 0;
    let mut removed = 0;
    let mut capacity_deferred = false;
    if !provisioning_blocked && active.len() < desired as usize {
        for _ in active.len()..desired as usize {
            let mut attempt = provision(app, pool, builtin_enabled).await?;
            // An idle runner parked on a repository without work does not make
            // the host full: reclaim its slot and retry straight away.
            // Rebalancing waits for the pool to reach its desired size, which
            // a full host never allows, so it cannot free the slot itself.
            if let ProvisionAttempt::Deferred {
                full_provider: Some(provider),
                ..
            } = &attempt
            {
                let current = runners(app, &pool.id).await?;
                if evict_surplus_runner(app, pool, &current, Some(provider)).await? > 0 {
                    rebalanced += 1;
                    attempt = provision(app, pool, builtin_enabled).await?;
                }
            }
            match attempt {
                ProvisionAttempt::Provisioned => provisioned += 1,
                ProvisionAttempt::NoDemand => break,
                ProvisionAttempt::Deferred {
                    reason,
                    full_provider,
                } => {
                    defer_pool_capacity(
                        &app.database,
                        pool,
                        reason.as_deref(),
                        full_provider.as_deref(),
                    )
                    .await?;
                    capacity_deferred = true;
                    break;
                }
            }
        }
    } else if active.len() > desired as usize {
        let excess = active.len() - desired as usize;
        for runner in active
            .into_iter()
            .filter(|runner| runner.ci_platform != "bitbucket" && !runner.busy)
            .take(excess)
        {
            if delete_runner(app, pool, runner).await? {
                removed += 1;
            }
        }
    }
    if !provisioning_blocked && evict_idle_fleet_runner(app, pool).await? {
        removed += 1;
    }
    let final_runners = runners(app, &pool.id).await?;
    let active_count = final_runners
        .iter()
        .filter(|runner| active_status(&runner.status))
        .count();
    let outdated_count = final_runners
        .iter()
        .filter(|runner| runner_needs_update(pool, runner))
        .count();
    let state = pool_state(&PassOutcome {
        provisioning_paused,
        host_blocked,
        capacity_deferred,
        retry_deferred,
        outdated: outdated_count,
        active: active_count,
        desired: usize::try_from(desired).unwrap_or_default(),
        ..PassOutcome::for_pool(pool, now_millis())
    });
    finish_pool_pass(&app.database, &pool.id, state).await?;
    record_capacity_sample(app, pool, &final_runners).await?;
    if provisioned > 0 || removed > 0 || rotated > 0 || rebalanced > 0 || retired > 0 {
        system_event(
            app,
            Some(&pool.id),
            "info",
            "Pool reconciled",
            &format!("Provisioned {provisioned}, removed {removed}, rotated {rotated}, rebalanced {rebalanced}, retired from the built-in host {retired}, active {active_count}"),
            json!({ "desired": desired, "active": active_count, "provisioned": provisioned, "removed": removed, "rotated": rotated, "rebalanced": rebalanced, "retiredFromBuiltinHost": retired, "outdated": outdated_count }),
        )
        .await?;
    }
    Ok(())
}

/// What a pass found, for the state the pool is shown in.
#[derive(Clone, Copy, Debug, Default)]
struct PassOutcome {
    paused: bool,
    provisioning_paused: bool,
    host_blocked: bool,
    circuit_open: bool,
    failure_count: i64,
    /// This pass's start was deferred: no room on the host, or no VM slot.
    capacity_deferred: bool,
    /// The pool is pausing before its next start (see `Pool::retry_deferred`).
    retry_deferred: bool,
    /// The pool's last start, in an earlier pass, was deferred.
    wait_recorded: bool,
    outdated: usize,
    active: usize,
    desired: usize,
}

impl PassOutcome {
    /// What the pool row says; the pass fills in what it found.
    fn for_pool(pool: &Pool, now: i64) -> Self {
        Self {
            paused: pool.paused,
            circuit_open: pool.provision_circuit_open,
            failure_count: pool.provision_failure_count,
            retry_deferred: pool.retry_deferred(now),
            wait_recorded: pool.wait_recorded,
            ..Self::default()
        }
    }
}

/// The state a pool is shown in at the end of a pass.
///
/// A real block comes first: the pool is paused, provisioning is paused, or
/// the built-in host it depends on is off. Then a wait: the pool's latest
/// start was deferred, in this pass or (while it pauses before asking again)
/// in an earlier one. A wait is not a failure, so it is shown whatever the
/// pool's earlier failures, even with its circuit open (#102): the circuit
/// stays open, and the pool page says so, until a start succeeds or fails.
/// Only then do an open circuit (`blocked`) and a failure's pause (`backoff`).
fn pool_state(outcome: &PassOutcome) -> &'static str {
    let waiting = outcome.capacity_deferred
        || (outcome.retry_deferred && (outcome.failure_count == 0 || outcome.wait_recorded));
    if outcome.paused {
        "paused"
    } else if outcome.provisioning_paused {
        "provisioning-paused"
    } else if outcome.host_blocked {
        gridops_core::builtin_host::POOL_STATE
    } else if waiting {
        "waiting"
    } else if outcome.circuit_open {
        "blocked"
    } else if outcome.retry_deferred {
        "backoff"
    } else if outcome.outdated > 0 {
        "updating"
    } else if outcome.active > outcome.desired {
        "draining"
    } else {
        "active"
    }
}

/// Stores the state the pass ended in; the wait reason is kept only while
/// the pool is waiting.
async fn finish_pool_pass(database: &SqlitePool, pool_id: &str, state: &str) -> Result<()> {
    sqlx::query("UPDATE runner_pools SET state=?,updated_at=? WHERE id=?")
        .bind(state)
        .bind(now_millis())
        .bind(pool_id)
        .execute(database)
        .await?;
    if state != "waiting" {
        set_pool_wait_reason(database, pool_id, None).await?;
    }
    Ok(())
}

async fn maybe_scale_up(app: &Reconciler, pool: &Pool, runners: &[Runner]) -> Result<()> {
    if !pool.autoscaling_enabled || pool.paused {
        return Ok(());
    }
    let queued = assigned_queued_jobs(&app.database, &pool.id).await?;
    if queued == 0 {
        return Ok(());
    }
    let busy = i64::try_from(
        runners
            .iter()
            .filter(|runner| runner.busy && active_status(&runner.status))
            .count(),
    )
    .unwrap_or(i64::MAX);
    let aggregate_target = scale_up_target(
        pool.desired_count,
        busy,
        queued,
        pool.queue_scale_factor,
        pool.max_count,
    );
    let repository_placement_needed = if pool.scope == "repository" {
        repository_capacities(&app.database, &pool.id)
            .await?
            .iter()
            .map(|capacity| repository_capacity_deficit(capacity, pool.queue_scale_factor).max(0))
            .sum::<i64>()
    } else {
        0
    };
    let providers = serde_json::from_str::<Vec<String>>(&pool.providers).unwrap_or_default();
    let labels = serde_json::from_str::<Vec<String>>(&pool.labels).unwrap_or_default();
    let provider_placement_needed =
        provider_capacities(&app.database, &pool.id, &providers, &labels)
            .await?
            .iter()
            .map(|capacity| provider_capacity_deficit(capacity, pool.queue_scale_factor).max(0))
            .sum::<i64>();
    let active = i64::try_from(
        runners
            .iter()
            .filter(|runner| active_status(&runner.status))
            .count(),
    )
    .unwrap_or(i64::MAX);
    let target = aggregate_target
        .max(active.saturating_add(repository_placement_needed))
        .max(active.saturating_add(provider_placement_needed))
        .min(pool.max_count);
    if target <= pool.desired_count {
        return Ok(());
    }
    let now = now_millis();
    sqlx::query("UPDATE runner_pools SET desired_count=?,state='scaling',updated_at=? WHERE id=?")
        .bind(target)
        .bind(now)
        .bind(&pool.id)
        .execute(&app.database)
        .await?;
    system_event(
        app,
        Some(&pool.id),
        "info",
        "Autoscale requested",
        &format!(
            "{queued} queued jobs raised desired capacity from {} to {target}",
            pool.desired_count
        ),
        json!({ "queuedJobs": queued, "desiredCount": target }),
    )
    .await?;
    system_audit(
        app,
        "runner_pool.autoscaled",
        "runner_pool",
        &pool.id,
        json!({ "queuedJobs": queued, "desiredCount": target, "source": "polling" }),
    )
    .await?;
    Ok(())
}

async fn rebalance_repository_capacity(app: &Reconciler, pool: &Pool, desired: i64) -> Result<i64> {
    if pool.paused {
        return Ok(0);
    }
    let current = runners(app, &pool.id).await?;
    let active = current
        .iter()
        .filter(|runner| active_status(&runner.status))
        .count();
    if active < usize::try_from(desired).unwrap_or(usize::MAX) {
        return Ok(0);
    }
    evict_surplus_runner(app, pool, &current, None).await
}

/// How long a fleet runner must sit idle before queued fleet work may take its
/// slot, so a runner that has just come online keeps the job it was made for.
const FLEET_IDLE_EVICTION_GRACE_MS: i64 = 2 * 60 * 1_000;

/// Deletes one idle fleet runner parked on a repository with no queued jobs
/// for its OS,
/// while fleet work in the pool waits for host capacity. Rebalancing leaves
/// fleet runners alone, and the fleet scheduler queues rather than reporting a
/// full host, so without this idle runners hold a machine indefinitely.
async fn evict_idle_fleet_runner(app: &Reconciler, pool: &Pool) -> Result<bool> {
    let Some(idle) = idle_fleet_runner_to_evict(&app.database, &pool.id, now_millis()).await?
    else {
        return Ok(false);
    };
    let current = runners(app, &pool.id).await?;
    let Some(runner) = current.iter().find(|runner| runner.id == idle) else {
        return Ok(false);
    };
    tracing::info!(pool = %pool.id, runner = %runner.name, "evicting an idle fleet runner for queued work");
    delete_runner(app, pool, runner).await
}

/// The idle fleet runner of `pool_id` that [`evict_idle_fleet_runner`] would
/// release: one parked at least the grace period on a repository with no
/// queued job for its OS, while fleet work in the pool waits.
async fn idle_fleet_runner_to_evict(
    database: &SqlitePool,
    pool_id: &str,
    now: i64,
) -> Result<Option<String>> {
    Ok(sqlx::query_scalar(
        r#"SELECT r.id FROM runners r
          WHERE r.pool_id=? AND r.status IN ('online','idle') AND r.busy=0 AND r.updated_at<?
            AND EXISTS (SELECT 1 FROM workload_intents own WHERE own.workload_id=r.id)
            AND EXISTS (SELECT 1 FROM workload_intents waiting
                         WHERE waiting.pool_id=r.pool_id AND waiting.status='queued')
            AND NOT EXISTS (SELECT 1 FROM workflow_jobs job JOIN workflow_runs run ON run.id=job.run_id
                             WHERE run.repository_id=r.target_repository_id AND job.status IN ('queued','waiting')
                               AND run.status!='completed'
                               AND (instr(lower(job.labels),'"'||lower(r.os)||'"')>0
                                    OR (instr(lower(job.labels),'"linux"')=0 AND instr(lower(job.labels),'"windows"')=0
                                        AND instr(lower(job.labels),'"macos"')=0)))
          ORDER BY r.updated_at LIMIT 1"#,
    )
    .bind(pool_id)
    .bind(now - FLEET_IDLE_EVICTION_GRACE_MS)
    .fetch_optional(database)
    .await?)
}

/// Deletes one idle runner that has no work, when a repository or provider is
/// short of runners, so the freed slot can be provisioned where it is needed.
/// With `provider`, only that provider's runners are considered.
async fn evict_surplus_runner(
    app: &Reconciler,
    pool: &Pool,
    current: &[Runner],
    provider: Option<&str>,
) -> Result<i64> {
    let capacities = if pool.scope == "repository" {
        repository_capacities(&app.database, &pool.id).await?
    } else {
        Vec::new()
    };
    let providers = serde_json::from_str::<Vec<String>>(&pool.providers).unwrap_or_default();
    let labels = serde_json::from_str::<Vec<String>>(&pool.labels).unwrap_or_default();
    let provider_capacities =
        provider_capacities(&app.database, &pool.id, &providers, &labels).await?;
    // Fleet runners are placed by the fleet scheduler; the provider and
    // repository counts here don't describe them.
    let current = current
        .iter()
        .filter(|runner| !runner.fleet)
        .cloned()
        .collect::<Vec<_>>();
    let Some(runner) = surplus_runner(
        &current,
        &capacities,
        &provider_capacities,
        &providers,
        pool.queue_scale_factor,
        provider,
    ) else {
        return Ok(0);
    };
    Ok(i64::from(delete_runner(app, pool, runner).await?))
}

fn surplus_runner<'a>(
    runners: &'a [Runner],
    capacities: &[RepositoryCapacity],
    provider_capacities: &[ProviderCapacity],
    providers: &[String],
    factor: i64,
    provider: Option<&str>,
) -> Option<&'a Runner> {
    let repository_needs_capacity = capacities
        .iter()
        .any(|capacity| repository_capacity_deficit(capacity, factor) > 0);
    let provider_needs_capacity = provider_capacities
        .iter()
        .any(|capacity| provider_capacity_deficit(capacity, factor) > 0);
    if !repository_needs_capacity && !provider_needs_capacity {
        return None;
    }
    let surplus = capacities
        .iter()
        .filter(|capacity| {
            capacity.active
                > capacity
                    .busy
                    .saturating_add(capacity.queued.saturating_mul(factor))
        })
        .map(|capacity| capacity.repository_id)
        .collect::<HashSet<_>>();
    let provider_surplus = provider_capacities
        .iter()
        .filter(|capacity| {
            capacity.active
                > capacity
                    .busy
                    .saturating_add(capacity.queued.saturating_mul(factor))
        })
        .map(|capacity| capacity.provider.as_str())
        .collect::<HashSet<_>>();
    let membership = capacities
        .iter()
        .map(|capacity| capacity.repository_id)
        .collect::<HashSet<_>>();
    runners.iter().find(|runner| {
        runner.ci_platform != "bitbucket"
            && !runner.busy
            && active_status(&runner.status)
            && provider.is_none_or(|provider| runner.provider == provider)
            && (provider_surplus.contains(runner.provider.as_str())
                || !providers.contains(&runner.provider)
                || runner.target_repository_id.is_some_and(|repository_id| {
                    surplus.contains(&repository_id) || !membership.contains(&repository_id)
                }))
    })
}

async fn maybe_scale_down(app: &Reconciler, pool: &Pool, runners: &[Runner]) -> Result<()> {
    if !pool.autoscaling_enabled || pool.paused || pool.desired_count <= pool.min_count {
        return Ok(());
    }
    // Bitbucket does not publish a per-step busy signal to GridOps. Keep its
    // persistent runner until an operator explicitly removes or pauses it.
    if runners
        .iter()
        .any(|runner| runner.ci_platform == "bitbucket")
    {
        return Ok(());
    }
    if runners.iter().any(|runner| runner.busy) {
        return Ok(());
    }
    let queued = assigned_queued_jobs(&app.database, &pool.id).await?;
    if queued > 0 {
        return Ok(());
    }
    let last_activity = runners
        .iter()
        .map(|runner| runner.updated_at)
        .max()
        .unwrap_or(0);
    if !idle_period_elapsed(now_millis(), pool.idle_timeout_minutes, last_activity) {
        return Ok(());
    }
    sqlx::query("UPDATE runner_pools SET desired_count=?,state='scaling',updated_at=? WHERE id=?")
        .bind(pool.min_count)
        .bind(now_millis())
        .bind(&pool.id)
        .execute(&app.database)
        .await?;
    system_audit(
        app,
        "runner_pool.autoscaled_down",
        "runner_pool",
        &pool.id,
        json!({ "desiredCount": pool.min_count }),
    )
    .await?;
    Ok(())
}

async fn retry_github_runner_cleanup(app: &Reconciler) -> Result<()> {
    let now = now_millis();
    let pending = sqlx::query_as::<_, PendingGitHubCleanup>(
        r#"SELECT id,installation_id,target_owner,target_repository,github_runner_id,
          runner_name,attempts FROM github_runner_cleanup
          WHERE next_attempt_at<=? ORDER BY next_attempt_at LIMIT 25"#,
    )
    .bind(now)
    .fetch_all(&app.database)
    .await?;
    for cleanup in pending {
        let result = async {
            let token = installation_token(app, cleanup.installation_id)
                .await?
                .context("GitHub App credentials are unavailable")?;
            let target = match cleanup.target_repository.as_deref() {
                Some(repository) => RunnerTarget::Repository {
                    owner: &cleanup.target_owner,
                    repository,
                },
                None => RunnerTarget::Organization {
                    organization: &cleanup.target_owner,
                },
            };
            let github_runner_id = match cleanup.github_runner_id {
                Some(id) => Some(id),
                None => app
                    .github
                    .runner_by_name(target, &token, &cleanup.runner_name)
                    .await?
                    .map(|runner| runner.id),
            };
            if let Some(github_runner_id) = github_runner_id {
                let path = match cleanup.target_repository.as_deref() {
                    Some(repository) => format!(
                        "/repos/{}/{repository}/actions/runners/{github_runner_id}",
                        cleanup.target_owner
                    ),
                    None => format!(
                        "/orgs/{}/actions/runners/{github_runner_id}",
                        cleanup.target_owner
                    ),
                };
                app.github.delete(&path, &token).await?;
            }
            Result::<()>::Ok(())
        }
        .await;
        match result {
            Ok(()) => {
                sqlx::query("DELETE FROM github_runner_cleanup WHERE id=?")
                    .bind(&cleanup.id)
                    .execute(&app.database)
                    .await?;
                system_audit(
                    app,
                    "runner.github_cleanup_completed",
                    "runner",
                    &cleanup.id,
                    json!({ "attempts": cleanup.attempts + 1 }),
                )
                .await?;
            }
            Err(error) => {
                let message = error.to_string().chars().take(2_000).collect::<String>();
                let attempts = cleanup.attempts.saturating_add(1);
                let next_attempt_at = now.saturating_add(github_cleanup_backoff_ms(attempts));
                sqlx::query(
                    "UPDATE github_runner_cleanup SET attempts=?,last_error=?,next_attempt_at=?,updated_at=? WHERE id=?",
                )
                .bind(attempts)
                .bind(&message)
                .bind(next_attempt_at)
                .bind(now)
                .bind(&cleanup.id)
                .execute(&app.database)
                .await?;
                tracing::warn!(runner_id = %cleanup.id, attempts, error = %message, "GitHub runner cleanup remains deferred");
            }
        }
    }
    Ok(())
}

fn github_cleanup_backoff_ms(attempts: i64) -> i64 {
    let exponent = u32::try_from(attempts.saturating_sub(1).clamp(0, 7)).unwrap_or(7);
    30_000_i64.saturating_mul(1_i64 << exponent).min(3_600_000)
}

async fn record_capacity_sample(app: &Reconciler, pool: &Pool, runners: &[Runner]) -> Result<()> {
    let online = runners
        .iter()
        .filter(|runner| matches!(runner.status.as_str(), "online" | "idle" | "busy"))
        .count();
    let busy = runners.iter().filter(|runner| runner.busy).count();
    let available = online.saturating_sub(busy);
    let queued = assigned_queued_jobs(&app.database, &pool.id).await?;
    let recorded_at = now_millis().div_euclid(60_000) * 60_000;
    sqlx::query(
        r#"INSERT INTO capacity_samples (id,installation_id,pool_id,available,busy,queued,recorded_at)
          VALUES (?,?,?,?,?,?,?) ON CONFLICT(pool_id,recorded_at) DO UPDATE SET
          available=excluded.available,busy=excluded.busy,queued=excluded.queued"#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(pool.installation_id)
    .bind(&pool.id)
    .bind(i64::try_from(available).unwrap_or(i64::MAX))
    .bind(i64::try_from(busy).unwrap_or(i64::MAX))
    .bind(queued)
    .bind(recorded_at)
    .execute(&app.database)
    .await?;
    Ok(())
}

async fn maybe_sync_github(app: &Reconciler) -> Result<()> {
    let now = now_millis();
    let interval_seconds = setting_i64(&app.database, "githubSyncIntervalSeconds", 60)
        .await
        .clamp(30, 3_600);
    let last_sync = setting_i64(&app.database, "lastGithubSyncAt", 0).await;
    if now.saturating_sub(last_sync) < interval_seconds * 1_000 {
        return Ok(());
    }
    sqlx::query(
        r#"INSERT INTO settings (key,value,updated_at) VALUES ('lastGithubSyncAt',?,?)
           ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at"#,
    )
    .bind(now.to_string())
    .bind(now)
    .execute(&app.database)
    .await?;
    if let Err(error) = sync_organization_pool_repositories(app).await {
        tracing::warn!(error = ?error, "organization repository discovery failed");
    }
    let repositories = sqlx::query_as::<_, Repository>(
        r#"SELECT repo.id,repo.installation_id,repo.owner,repo.name,repo.full_name
           FROM repositories repo JOIN installations i ON i.id=repo.installation_id
           WHERE repo.archived=0 AND i.suspended_at IS NULL
             AND (EXISTS (SELECT 1 FROM runner_pool_repositories membership
                    WHERE membership.repository_id=repo.id)
               OR EXISTS (SELECT 1 FROM runner_pools p WHERE p.repository_id=repo.id)
               OR EXISTS (SELECT 1 FROM workflow_runs wr WHERE wr.repository_id=repo.id)
               OR EXISTS (SELECT 1 FROM runner_pools op WHERE op.scope='organization'
                    AND op.installation_id=repo.installation_id AND op.state!='deleted'))
           ORDER BY repo.id"#,
    )
    .fetch_all(&app.database)
    .await?;
    let mut tokens = HashMap::<i64, Option<String>>::new();
    for repository in repositories {
        let token = if let Some(token) = tokens.get(&repository.installation_id) {
            token.clone()
        } else {
            let token = installation_token(app, repository.installation_id).await?;
            tokens.insert(repository.installation_id, token.clone());
            token
        };
        let Some(token) = token else {
            continue;
        };
        if let Err(error) = sync_repository_workflows(app, &repository, &token).await {
            tracing::warn!(repository = %repository.full_name, error = ?error, "GitHub workflow polling failed for repository");
        }
    }
    confirm_jobs(app, &mut tokens).await
}

/// Populates `repositories` for an organization-scoped pool's installation.
///
/// An organization-scoped pool never selects individual repositories, so nothing calls the
/// API-driven repository sync for the repos it covers. Under webhook delivery this self-heals:
/// every `workflow_run`/`workflow_job` webhook carries the full repository object, and the first
/// one for any new repo creates its `repositories` row (see `upsert_repository_from_webhook` in
/// `gridops-api`). Under polling there is no such event, ever, so without this, a `repositories`
/// row for a repo under this installation would never exist in the first place — and even once
/// it does (this function), it still needs the 4th `OR` branch added to the candidate query below
/// (an organization-scoped pool match by installation) to actually become a polling candidate;
/// the first three branches all assume either explicit repo selection or a repo that's already
/// been polled once, neither of which is true for a brand new repo under an organization scope.
async fn sync_organization_pool_repositories(app: &Reconciler) -> Result<()> {
    if app.config.webhook_delivery_active() {
        return Ok(());
    }
    let installation_ids: Vec<i64> = sqlx::query_scalar(
        r#"SELECT DISTINCT p.installation_id FROM runner_pools p
           JOIN installations i ON i.id=p.installation_id
           WHERE p.scope='organization' AND p.state!='deleted' AND i.suspended_at IS NULL"#,
    )
    .fetch_all(&app.database)
    .await?;
    for installation_id in installation_ids {
        let Some(token) = installation_token(app, installation_id).await? else {
            continue;
        };
        if let Err(error) = sync_installation_repositories(app, installation_id, &token).await {
            tracing::warn!(installation_id, error = ?error, "organization repository discovery failed for installation");
        }
    }
    Ok(())
}

async fn sync_installation_repositories(
    app: &Reconciler,
    installation_id: i64,
    token: &str,
) -> Result<()> {
    let first_page: gridops_core::github::RepositoryPage = app
        .github
        .get("/installation/repositories?per_page=100&page=1", token)
        .await?;
    let expected_total = first_page.total_count;
    let mut repositories = first_page.repositories;
    let last_page = expected_total
        .saturating_add(99)
        .div_euclid(100)
        .clamp(1, 100);
    for page in 2..=last_page {
        let page_data: gridops_core::github::RepositoryPage = app
            .github
            .get(
                &format!("/installation/repositories?per_page=100&page={page}"),
                token,
            )
            .await?;
        repositories.extend(page_data.repositories);
    }
    let now = now_millis();
    for repository in repositories {
        let (owner, name) = repository
            .full_name
            .split_once('/')
            .unwrap_or(("unknown", &repository.full_name));
        sqlx::query(
            r#"INSERT INTO repositories (
              id,installation_id,owner,name,full_name,private,archived,default_branch,
              html_url,last_synced_at,created_at,updated_at
            ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)
            ON CONFLICT(id) DO UPDATE SET installation_id=excluded.installation_id,
              owner=excluded.owner,name=excluded.name,full_name=excluded.full_name,
              private=excluded.private,archived=excluded.archived,default_branch=excluded.default_branch,
              html_url=excluded.html_url,
              last_synced_at=excluded.last_synced_at,updated_at=excluded.updated_at"#,
        )
        .bind(repository.id)
        .bind(installation_id)
        .bind(owner)
        .bind(name)
        .bind(&repository.full_name)
        .bind(repository.private)
        .bind(repository.archived)
        .bind(&repository.default_branch)
        .bind(&repository.html_url)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&app.database)
        .await?;
    }
    Ok(())
}

async fn sync_repository_workflows(
    app: &Reconciler,
    repository: &Repository,
    token: &str,
) -> Result<()> {
    let response: WorkflowRunPage = app
        .github
        .get(
            &format!(
                "/repos/{}/{}/actions/runs?per_page=50",
                repository.owner, repository.name
            ),
            token,
        )
        .await?;
    let now = now_millis();
    let mut job_run_ids = Vec::new();
    let mut seen_run_ids = HashSet::new();
    for run in response
        .workflow_runs
        .iter()
        .filter(|run| matches!(run.status.as_str(), "queued" | "in_progress"))
        .chain(response.workflow_runs.iter().take(5))
    {
        if seen_run_ids.insert(run.id) {
            job_run_ids.push(run.id);
        }
    }
    let runs = &response.workflow_runs;
    retry_when_busy(|| async move {
        let mut transaction = app.database.begin().await?;
        for run in runs {
            upsert_workflow_run(&mut transaction, repository.id, run.clone(), now).await?;
        }
        transaction.commit().await?;
        anyhow::Ok(())
    })
    .await?;
    for run_id in job_run_ids {
        if let Err(error) = sync_run_jobs(app, repository, run_id, token).await {
            tracing::warn!(repository = %repository.full_name, run_id, error = ?error, "GitHub workflow job polling failed");
        }
    }
    refresh_stale_runs(app, repository, token, &seen_run_ids).await
}

/// Runs are only listed while they are among a repository's 50 newest, and
/// completed runs outside the newest five never have their jobs re-polled. A
/// run that finishes after falling out of that window would otherwise keep
/// its jobs `queued` in `GridOps` forever, so a few are revisited each sync.
async fn refresh_stale_runs(
    app: &Reconciler,
    repository: &Repository,
    token: &str,
    just_synced: &HashSet<i64>,
) -> Result<()> {
    let stale = stale_run_ids(&app.database, repository.id, just_synced).await?;
    for run_id in stale {
        let result = app
            .github
            .get::<GitHubWorkflowRun>(
                &format!(
                    "/repos/{}/{}/actions/runs/{run_id}",
                    repository.owner, repository.name
                ),
                token,
            )
            .await;
        match result {
            Ok(run) => {
                let completed = run.status == "completed";
                let mut transaction = app.database.begin().await?;
                upsert_workflow_run(&mut transaction, repository.id, run, now_millis()).await?;
                transaction.commit().await?;
                match sync_run_jobs(app, repository, run_id, token).await {
                    Ok(()) => {
                        if completed {
                            cancel_orphaned_jobs(&app.database, run_id).await?;
                        }
                    }
                    Err(error) => {
                        if completed && github_not_found(&error) {
                            // GitHub no longer lists the run's jobs at all.
                            cancel_orphaned_jobs(&app.database, run_id).await?;
                        } else {
                            tracing::warn!(repository = %repository.full_name, run_id, error = ?error, "GitHub stale workflow job refresh failed");
                        }
                    }
                }
            }
            Err(error) if github_not_found(&error) => {
                close_missing_run(&app.database, run_id).await?;
            }
            Err(error) => {
                tracing::warn!(repository = %repository.full_name, run_id, error = ?error, "GitHub stale workflow run refresh failed");
            }
        }
    }
    Ok(())
}

const STALE_RUN_REFRESH_LIMIT: usize = 10;

/// Unfinished runs (or runs with unfinished jobs) that the latest listing did
/// not cover, least recently refreshed first so every one is eventually seen.
async fn stale_run_ids(
    database: &SqlitePool,
    repository_id: i64,
    just_synced: &HashSet<i64>,
) -> Result<Vec<i64>> {
    let candidates = sqlx::query_scalar::<_, i64>(
        r#"SELECT wr.id FROM workflow_runs wr
           WHERE wr.repository_id=? AND (wr.status<>'completed' OR EXISTS (
             SELECT 1 FROM workflow_jobs wj WHERE wj.run_id=wr.id AND wj.status<>'completed'))
           ORDER BY wr.updated_at,wr.id LIMIT ?"#,
    )
    .bind(repository_id)
    .bind(i64::try_from(STALE_RUN_REFRESH_LIMIT + just_synced.len()).unwrap_or(i64::MAX))
    .fetch_all(database)
    .await?;
    Ok(candidates
        .into_iter()
        .filter(|run_id| !just_synced.contains(run_id))
        .take(STALE_RUN_REFRESH_LIMIT)
        .collect())
}

/// A queued job under a run GitHub reports completed can never run: GitHub
/// finished (or deleted) the run without ever reporting a conclusion for that
/// job. Mark it cancelled so queue views and autoscaling stop counting it.
async fn cancel_orphaned_jobs(database: &SqlitePool, run_id: i64) -> Result<()> {
    let now = now_millis();
    let changed = sqlx::query(
        "UPDATE workflow_jobs
            SET status='completed',conclusion=COALESCE(conclusion,'cancelled'),
                completed_at=COALESCE(completed_at,?),updated_at=?
          WHERE run_id=? AND status<>'completed'",
    )
    .bind(now)
    .bind(now)
    .bind(run_id)
    .execute(database)
    .await?
    .rows_affected();
    if changed > 0 {
        tracing::info!(
            run_id,
            jobs = changed,
            "cancelled orphaned queued jobs of a completed run"
        );
    }
    Ok(())
}

fn github_not_found(error: &anyhow::Error) -> bool {
    error.to_string().contains("(404 Not Found)")
}

/// GitHub deleted the run (retention or manual deletion), so none of its
/// unfinished jobs can still run.
async fn close_missing_run(database: &SqlitePool, run_id: i64) -> Result<()> {
    let now = now_millis();
    let mut transaction = database.begin().await?;
    sqlx::query(
        "UPDATE workflow_runs SET status='completed',conclusion=COALESCE(conclusion,'cancelled'),completed_at=COALESCE(completed_at,?),updated_at=? WHERE id=?",
    )
    .bind(now)
    .bind(now)
    .bind(run_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE workflow_jobs SET status='completed',conclusion=COALESCE(conclusion,'cancelled'),completed_at=COALESCE(completed_at,?),updated_at=? WHERE run_id=? AND status<>'completed'",
    )
    .bind(now)
    .bind(now)
    .bind(run_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn upsert_workflow_run(
    connection: &mut SqliteConnection,
    repository_id: i64,
    run: GitHubWorkflowRun,
    now: i64,
) -> Result<()> {
    let updated_at = parse_github_date(Some(&run.updated_at)).unwrap_or(now);
    sqlx::query(
            r#"INSERT INTO workflow_runs (
              id,repository_id,workflow_id,workflow_name,run_number,run_attempt,event,status,
              conclusion,head_branch,head_sha,actor_login,html_url,started_at,completed_at,
              github_created_at,github_updated_at,created_at,updated_at
            ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
            ON CONFLICT(id) DO UPDATE SET repository_id=excluded.repository_id,
              workflow_id=excluded.workflow_id,workflow_name=excluded.workflow_name,
              run_number=excluded.run_number,run_attempt=excluded.run_attempt,event=excluded.event,
              status=excluded.status,conclusion=excluded.conclusion,head_branch=excluded.head_branch,
              head_sha=excluded.head_sha,actor_login=excluded.actor_login,html_url=excluded.html_url,
              started_at=excluded.started_at,completed_at=excluded.completed_at,
              github_created_at=excluded.github_created_at,
              github_updated_at=excluded.github_updated_at,updated_at=excluded.updated_at
            WHERE excluded.run_attempt>=workflow_runs.run_attempt"#,
        )
        .bind(run.id)
        .bind(repository_id)
        .bind(run.workflow_id)
        .bind(
            run.name
                .or(run.display_title)
                .unwrap_or_else(|| "Workflow".into()),
        )
        .bind(run.run_number)
        .bind(run.run_attempt)
        .bind(run.event)
        .bind(&run.status)
        .bind(run.conclusion)
        .bind(run.head_branch)
        .bind(run.head_sha)
        .bind(run.actor.map(|actor| actor.login))
        .bind(run.html_url)
        .bind(parse_github_date(run.run_started_at.as_deref()))
        .bind((run.status == "completed").then_some(updated_at))
        .bind(parse_github_date(Some(&run.created_at)).unwrap_or(now))
        .bind(updated_at)
        .bind(now)
        .bind(now)
        .execute(&mut *connection)
        .await?;
    // A newer attempt means the earlier ones are over (#33).
    gridops_core::finish_earlier_attempts(&mut *connection, run.id, now).await?;
    Ok(())
}

/// Records every job of `run_id` as GitHub lists it, the jobs of earlier
/// attempts included (`filter=all`). With GitHub's default, only the newest
/// attempt's jobs are listed, so after a re-run an earlier attempt's job whose
/// completion had been missed was never seen again and stayed queued or in
/// progress (#33).
async fn sync_run_jobs(
    app: &Reconciler,
    repository: &Repository,
    run_id: i64,
    token: &str,
) -> Result<()> {
    for page in 1..=100 {
        let response: WorkflowJobPage = app
            .github
            .get(
                &format!(
                    "/repos/{}/{}/actions/runs/{run_id}/jobs?filter=all&per_page=100&page={page}",
                    repository.owner, repository.name
                ),
                token,
            )
            .await?;
        let final_page = response.jobs.len() < 100;
        let now = now_millis();
        let jobs = &response.jobs;
        retry_when_busy(|| async move {
            let mut transaction = app.database.begin().await?;
            for job in jobs {
                upsert_workflow_job(&mut transaction, job, now).await?;
            }
            gridops_core::finish_earlier_attempts(&mut *transaction, run_id, now).await?;
            transaction.commit().await?;
            anyhow::Ok(())
        })
        .await?;
        for job in &response.jobs {
            associate_runner_with_job(
                &app.database,
                job.id,
                &job.status,
                job.runner_id,
                job.runner_name.as_deref(),
                now,
            )
            .await?;
        }
        if final_page {
            break;
        }
    }
    Ok(())
}

/// Records a job as GitHub reported it at `now`.
async fn upsert_workflow_job(
    connection: &mut SqliteConnection,
    job: &GitHubWorkflowJob,
    now: i64,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO workflow_jobs (
          id,run_id,name,status,conclusion,runner_id,runner_name,runner_group_id,
          runner_group_name,labels,html_url,started_at,completed_at,created_at,updated_at
        ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
        ON CONFLICT(id) DO UPDATE SET run_id=excluded.run_id,name=excluded.name,
          status=excluded.status,conclusion=excluded.conclusion,runner_id=excluded.runner_id,
          runner_name=excluded.runner_name,runner_group_id=excluded.runner_group_id,
          runner_group_name=excluded.runner_group_name,labels=excluded.labels,
          html_url=excluded.html_url,started_at=excluded.started_at,
          completed_at=excluded.completed_at,updated_at=excluded.updated_at"#,
    )
    .bind(job.id)
    .bind(job.run_id)
    .bind(&job.name)
    .bind(&job.status)
    .bind(&job.conclusion)
    .bind(job.runner_id)
    .bind(&job.runner_name)
    .bind(job.runner_group_id)
    .bind(&job.runner_group_name)
    .bind(serde_json::to_string(&job.labels)?)
    .bind(&job.html_url)
    .bind(parse_github_date(job.started_at.as_deref()))
    .bind(parse_github_date(job.completed_at.as_deref()))
    .bind(parse_github_date(job.started_at.as_deref()).unwrap_or(now))
    .bind(now)
    .execute(&mut *connection)
    .await?;
    gridops_core::record_job_report(&mut *connection, job.id, job.run_attempt, now).await?;
    Ok(())
}

/// How long a job may be open with no word from GitHub before the job check
/// asks GitHub about it, and how long before it asks again: the age at which
/// the Overview raises "Jobs have been waiting for over five minutes", so the
/// job that alert names has been confirmed with GitHub by the time it shows,
/// or within a sync of it. Webhooks and the poll keep jobs of the runs they
/// see confirmed, so in a healthy queue the check asks about almost nothing.
const JOB_CHECK_AGE_MS: i64 = 5 * 60 * 1_000;

/// Most jobs the check asks GitHub about per sync, one request each. At the
/// default sync interval (60 s) that is at most 600 requests an hour, 1,200
/// at the shortest (30 s), against the 5,000 an hour an installation token is
/// allowed at least; the requests are made one after another, never in
/// parallel. A backlog of stale rows clears at this rate too.
const JOB_CHECK_LIMIT: i64 = 10;

/// How long after a job was finished here without a conclusion the job check
/// still asks GitHub for one: the window of the Overview's per-machine pass
/// rates, the one place such a job is counted (as neither passed nor failed).
/// The check reaches such a job within a few syncs, so the window only
/// matters when GitHub cannot be asked for that long, or keeps answering
/// without a conclusion; then the job stops taking a place in the check.
const CONCLUSION_CHECK_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1_000;

#[derive(Debug, FromRow)]
struct UnconfirmedJob {
    id: i64,
    installation_id: i64,
    owner: String,
    name: String,
    /// Already `completed` here, without a conclusion: the check is only for
    /// how it ended.
    closed: bool,
}

/// Jobs GitHub has said nothing about for [`JOB_CHECK_AGE_MS`] and that were
/// not asked about within it either, at most `limit`. Open jobs come first:
/// jobs of unfinished runs before jobs of finished ones (only the first can
/// hold up the queue), waiting jobs before running ones, oldest first, so the
/// job the queue alert names is checked first. The room left goes to jobs
/// finished here without a conclusion in the last
/// [`CONCLUSION_CHECK_WINDOW_MS`] (an earlier attempt's job finished when a
/// re-run appeared), oldest first. Two queries rather than one with `OR`, so
/// each is served by its own partial index instead of a scan of every job.
async fn unconfirmed_jobs(
    database: &SqlitePool,
    now: i64,
    limit: i64,
) -> Result<Vec<UnconfirmedJob>> {
    let before = now - JOB_CHECK_AGE_MS;
    let mut jobs = sqlx::query_as::<_, UnconfirmedJob>(
        r#"SELECT wj.id,repo.installation_id,repo.owner,repo.name,0 AS closed
           FROM workflow_jobs wj
           JOIN workflow_runs wr ON wr.id=wj.run_id
           JOIN repositories repo ON repo.id=wr.repository_id
           JOIN installations i ON i.id=repo.installation_id
           LEFT JOIN workflow_job_sync sync ON sync.job_id=wj.id
           WHERE wj.status<>'completed' AND i.suspended_at IS NULL
             AND wj.created_at<? AND COALESCE(sync.confirmed_at,0)<?
             AND COALESCE(sync.checked_at,0)<?
           ORDER BY CASE WHEN wr.status<>'completed' THEN 0 ELSE 1 END,
             CASE WHEN wj.status='in_progress' THEN 1 ELSE 0 END,
             wj.created_at,wj.id
           LIMIT ?"#,
    )
    .bind(before)
    .bind(before)
    .bind(before)
    .bind(limit)
    .fetch_all(database)
    .await?;
    let room = limit.saturating_sub(i64::try_from(jobs.len())?);
    if room > 0 {
        jobs.extend(
            sqlx::query_as::<_, UnconfirmedJob>(
                r#"SELECT wj.id,repo.installation_id,repo.owner,repo.name,1 AS closed
                   FROM workflow_jobs wj
                   JOIN workflow_runs wr ON wr.id=wj.run_id
                   JOIN repositories repo ON repo.id=wr.repository_id
                   JOIN installations i ON i.id=repo.installation_id
                   LEFT JOIN workflow_job_sync sync ON sync.job_id=wj.id
                   WHERE wj.status='completed' AND wj.conclusion IS NULL
                     AND COALESCE(wj.completed_at,wj.updated_at)>=?
                     AND i.suspended_at IS NULL
                     AND wj.created_at<? AND COALESCE(sync.confirmed_at,0)<?
                     AND COALESCE(sync.checked_at,0)<?
                   ORDER BY wj.created_at,wj.id
                   LIMIT ?"#,
            )
            .bind(now - CONCLUSION_CHECK_WINDOW_MS)
            .bind(before)
            .bind(before)
            .bind(before)
            .bind(room)
            .fetch_all(database)
            .await?,
        );
    }
    Ok(jobs)
}

/// Asks GitHub (`GET /repos/{owner}/{repo}/actions/jobs/{id}`) about open jobs
/// it has not reported on lately and records the answer (#33, #34). This
/// catches what the source fix cannot: a completion missed for any other
/// reason, rows recorded before attempts were stored, and runs the poll no
/// longer lists. A job GitHub no longer has (404) can never run and is
/// cancelled, as a deleted run's jobs are. `tokens` caches installation tokens
/// for the sync.
///
/// With the room the open jobs leave, it also asks about jobs finished here
/// without a conclusion (an earlier attempt's job finished when a re-run
/// appeared), so how they ended is recorded even when their run is not polled
/// again. Once such a job has a conclusion it is not asked about again.
async fn confirm_jobs(app: &Reconciler, tokens: &mut HashMap<i64, Option<String>>) -> Result<()> {
    let jobs = unconfirmed_jobs(&app.database, now_millis(), JOB_CHECK_LIMIT).await?;
    for job in jobs {
        let token = if let Some(token) = tokens.get(&job.installation_id) {
            token.clone()
        } else {
            let token = installation_token(app, job.installation_id)
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(installation_id = job.installation_id, error = ?error, "no GitHub token to check jobs with");
                    None
                });
            tokens.insert(job.installation_id, token.clone());
            token
        };
        let Some(token) = token else {
            continue;
        };
        let now = now_millis();
        sqlx::query(
            "INSERT INTO workflow_job_sync (job_id,checked_at) VALUES (?,?)
             ON CONFLICT(job_id) DO UPDATE SET checked_at=excluded.checked_at",
        )
        .bind(job.id)
        .bind(now)
        .execute(&app.database)
        .await?;
        let result = app
            .github
            .get::<GitHubWorkflowJob>(
                &format!("/repos/{}/{}/actions/jobs/{}", job.owner, job.name, job.id),
                &token,
            )
            .await;
        match result {
            Ok(remote) => {
                let mut connection = app.database.acquire().await?;
                upsert_workflow_job(&mut connection, &remote, now).await?;
                gridops_core::finish_earlier_attempts(&mut *connection, remote.run_id, now).await?;
                drop(connection);
                if job.closed {
                    // The job was over here already, so the answer says
                    // nothing about its runner now: the runner may have
                    // taken the new attempt's job since.
                    tracing::info!(job_id = job.id, conclusion = ?remote.conclusion, "recorded how a job finished here ended");
                } else {
                    associate_runner_with_job(
                        &app.database,
                        remote.id,
                        &remote.status,
                        remote.runner_id,
                        remote.runner_name.as_deref(),
                        now,
                    )
                    .await?;
                    tracing::info!(job_id = job.id, status = %remote.status, "checked an open job with GitHub");
                }
            }
            Err(error) if github_not_found(&error) => {
                sqlx::query(
                    "UPDATE workflow_jobs SET status='completed',conclusion=COALESCE(conclusion,'cancelled'),completed_at=COALESCE(completed_at,?),updated_at=? WHERE id=? AND (status<>'completed' OR conclusion IS NULL)",
                )
                .bind(now)
                .bind(now)
                .bind(job.id)
                .execute(&app.database)
                .await?;
                gridops_core::record_job_report(&app.database, job.id, None, now).await?;
                tracing::info!(job_id = job.id, "cancelled a job GitHub no longer has");
            }
            Err(error) => {
                tracing::warn!(job_id = job.id, error = ?error, "could not check a job with GitHub");
            }
        }
    }
    Ok(())
}

fn parse_github_date(value: Option<&str>) -> Option<i64> {
    value
        .and_then(|date| chrono::DateTime::parse_from_rfc3339(date).ok())
        .map(|date| date.timestamp_millis())
}

async fn next_bitbucket_connection(
    app: &Reconciler,
    pool_id: &str,
) -> Result<Option<BitbucketPoolConnection>> {
    Ok(sqlx::query_as(
        r#"SELECT connection.id,connection.workspace,connection.workspace_uuid,connection.access_token_key
           FROM runner_pool_bitbucket_connections membership
           JOIN bitbucket_connections connection ON connection.id=membership.connection_id
           WHERE membership.pool_id=?
             AND NOT EXISTS (
               SELECT 1 FROM runners runner
               WHERE runner.pool_id=membership.pool_id
                 AND runner.ci_platform='bitbucket'
                 AND runner.bitbucket_connection_id=connection.id
                 AND runner.deleted_at IS NULL
                 AND runner.status IN ('starting','online','idle','busy','paused','stopped')
             )
           ORDER BY membership.created_at,connection.id LIMIT 1"#,
    )
    .bind(pool_id)
    .fetch_optional(&app.database)
    .await?)
}

/// Bitbucket labels for a pool's runner. Bitbucket requires `self.hosted` and
/// the runner's OS on every self-hosted runner (it resolves the runner type
/// from them and refuses the registration otherwise), and steps target runners
/// with `runs-on: [self.hosted, macos, ...]`. `GridOps` runs Bitbucket runners
/// only on the macOS host, so the OS is always `macos`.
fn bitbucket_runner_labels(pool_name: &str, labels: &[String]) -> Result<Vec<String>> {
    let mut output = vec![
        "self.hosted".to_owned(),
        "macos".to_owned(),
        format!("gridops.{}", pool_name.replace('-', ".")),
    ];
    output.extend(
        labels
            .iter()
            .filter(|label| label.as_str() != pool_name)
            .cloned(),
    );
    output.sort();
    output.dedup();
    anyhow::ensure!(
        output.len() <= 10
            && output.iter().all(|label| {
                !label.is_empty()
                    && label.chars().all(|character| {
                        character.is_ascii_lowercase()
                            || character.is_ascii_digit()
                            || character == '.'
                    })
            }),
        "Bitbucket runner labels must use lowercase letters, numbers, and dots; at most 10 are allowed."
    );
    Ok(output)
}

/// Starts one runner for the pool, on the built-in host or a fleet machine as
/// the pool's fleet platforms and placement say.
async fn provision(
    app: &Reconciler,
    pool: &Pool,
    builtin_enabled: bool,
) -> Result<ProvisionAttempt> {
    use gridops_core::fleet::targets::Placement;
    let profiles = fleet::profiles(app, &pool.id).await?;
    let Some((profile, behind)) = fleet::choose_profile(app, pool, &profiles).await? else {
        return provision_on_host(app, pool, builtin_enabled).await;
    };
    // With the built-in host off a Linux platform runs on machines only,
    // whatever placement was saved.
    let os = profile.input.execution_os;
    let placement = builtin_host::effective_placement(builtin_enabled, os, profile.input.placement);
    // A fleet runner without queued work only sits on a repository holding
    // machine capacity, and the desired count can stay high while a job no
    // platform can run stays queued. Above the pool's minimum, start fleet
    // runners only for platforms that are behind.
    if !behind && placement == Placement::Machines {
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM runners WHERE pool_id=? AND deleted_at IS NULL
               AND status IN ('starting','online','idle','busy','paused','stopped')",
        )
        .bind(&pool.id)
        .fetch_one(&app.database)
        .await?;
        if active >= pool.min_count {
            return Ok(ProvisionAttempt::NoDemand);
        }
    }
    let host =
        fleet::host_serves(pool, profile) && builtin_host::host_may_serve(builtin_enabled, os);
    match placement {
        Placement::Machines => fleet::provision(app, pool, &profiles, profile).await,
        Placement::MachinesFirst => {
            if !host || fleet::machine_ready(app, pool, profile).await? {
                fleet::provision(app, pool, &profiles, profile).await
            } else {
                provision_on_host(app, pool, builtin_enabled).await
            }
        }
        Placement::HostFirst => {
            if !host {
                return fleet::provision(app, pool, &profiles, profile).await;
            }
            let attempt = provision_on_host(app, pool, builtin_enabled).await?;
            if matches!(attempt, ProvisionAttempt::Deferred { .. })
                && fleet::machine_ready(app, pool, profile).await?
            {
                return fleet::provision(app, pool, &profiles, profile).await;
            }
            Ok(attempt)
        }
    }
}

async fn provision_on_host(
    app: &Reconciler,
    pool: &Pool,
    builtin_enabled: bool,
) -> Result<ProvisionAttempt> {
    let runner_id = uuid::Uuid::new_v4().to_string();
    let suffix = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
    let runner_name = format!("{}-{suffix}", pool.name);
    let target_repository = if pool.scope == "repository" {
        Some(
            next_runner_repository(&app.database, &pool.id, pool.queue_scale_factor)
                .await?
                .context("runner pool has no available repositories")?,
        )
    } else {
        None
    };
    let mut providers = serde_json::from_str::<Vec<String>>(&pool.providers).unwrap_or_default();
    if providers.is_empty() {
        providers.push(pool.provider.clone());
    }
    // The manager would refuse Docker while the host is off; do not ask.
    let providers = builtin_host::host_providers(builtin_enabled, providers);
    if providers.is_empty() {
        return Ok(ProvisionAttempt::NoDemand);
    }
    let labels = serde_json::from_str::<Vec<String>>(&pool.labels).unwrap_or_default();
    let pending_bitbucket_connection = next_bitbucket_connection(app, &pool.id).await?;
    let provider =
        if pending_bitbucket_connection.is_some() && providers.iter().any(|item| item == "tart") {
            "tart".to_owned()
        } else {
            next_runner_provider(
                &app.database,
                &pool.id,
                target_repository
                    .as_ref()
                    .map(|repository| repository.repository_id),
                &providers,
                &labels,
                pool.queue_scale_factor,
            )
            .await?
            .context("runner pool has no configured providers")?
        };
    let bitbucket_connection = (provider == "tart")
        .then_some(pending_bitbucket_connection)
        .flatten();
    let plan = gridops_core::ProvisioningPlan::for_selected_provider(
        &provider,
        bitbucket_connection.is_some(),
        &pool.mode,
        pool.ephemeral,
        &pool.macos_runtime,
        &pool.docker_image,
        &pool.tart_image,
    );
    let platform = plan.platform;
    let runner_mode = plan.mode.as_str();
    let runner_ephemeral = plan.ephemeral;
    let runtime = plan.runtime.as_str();
    let image = plan.image.as_str();
    let admission = reserve_runner_capacity(
        app,
        &runner_id,
        &pool.id,
        &provider,
        runtime,
        pool.cpu_limit,
        pool.memory_limit_mb,
    )
    .await?;
    let Some(capacity_lease) = admission.lease_id else {
        return Ok(ProvisionAttempt::Deferred {
            full_provider: host_capacity_exhausted(admission.reason.as_deref()).then_some(provider),
            reason: admission.reason,
        });
    };
    let now = now_millis();
    let (runner_os, runner_architecture) = if provider == "tart" {
        ("macOS", "ARM64")
    } else {
        ("linux", gridops_core::runner_arch_label())
    };
    if let Err(error) = sqlx::query("INSERT INTO runners (id,pool_id,target_repository_id,name,provider,ci_platform,bitbucket_connection_id,os,architecture,status,ephemeral,configuration_version,runtime,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?,?,'starting',?,?,?,?,?)")
        .bind(&runner_id)
        .bind(&pool.id)
        .bind((platform == "github").then(|| target_repository.as_ref().map(|repository| repository.repository_id)).flatten())
        .bind(&runner_name)
        .bind(&provider)
        .bind(platform)
        .bind(bitbucket_connection.as_ref().map(|connection| &connection.id))
        .bind(runner_os)
        .bind(runner_architecture)
        .bind(runner_ephemeral)
        .bind(pool.configuration_version)
        .bind(runtime)
        .bind(now)
        .bind(now)
        .execute(&app.database)
        .await
    {
        if gridops_core::failure_cleanup(true, false, false).release_capacity {
            release_runner_capacity(app, &capacity_lease).await;
        }
        return Err(error.into());
    }

    let result = async {
        let mut request = json!({
            "runnerId": runner_id,
            "poolId": pool.id,
            "name": runner_name,
            "image": image,
            "mode": runner_mode,
            "provider": provider,
            "runtime": runtime,
            "platform": platform,
            "labels": &labels,
            "cpuLimit": pool.cpu_limit,
            "memoryLimitMb": pool.memory_limit_mb,
            "network": app.config.runner_network(),
            "capacityLease": &capacity_lease,
            "pullImage": setting_bool(&app.database, "autoUpdateImages", false).await,
        });
        let (github_runner_id, bitbucket_runner_uuid) = if let Some(connection) = &bitbucket_connection {
            let access_token = runtime_secret(app, &connection.access_token_key)
                .await?
                .context("Bitbucket workspace credentials are unavailable")?;
            let runner = app
                .bitbucket
                .create_runner(
                    &BitbucketRunnerTarget::Workspace { workspace: connection.workspace.clone() },
                    &access_token,
                    &runner_name,
                    &bitbucket_runner_labels(&pool.name, &labels)?,
                )
                .await?;
            let oauth_secret = runner.oauth_client.secret.context(
                "Bitbucket did not return the one-time runner OAuth secret",
            )?;
            request["bitbucket"] = json!({
                "accountUuid": connection.workspace_uuid,
                "runnerUuid": runner.uuid,
                "oauthClientId": runner.oauth_client.id,
            });
            request["bitbucketOauthClientSecret"] = Value::String(oauth_secret);
            (None, Some(runner.uuid))
        } else {
            let target_installation_id = target_repository
                .as_ref()
                .map_or(pool.installation_id, |repository| repository.installation_id);
            let token = installation_token(app, target_installation_id)
                .await?
                .context("GitHub App credentials are required for autonomous reconciliation")?;
            let target = match &target_repository {
                Some(repository) => RunnerTarget::Repository {
                    owner: &repository.owner,
                    repository: &repository.name,
                },
                None => RunnerTarget::Organization {
                    organization: &pool.account_login,
                },
            };
            if pool.ephemeral {
                let jit = app
                    .github
                    .generate_jit_config(
                        target,
                        &token,
                        &JitRequest {
                            name: runner_name.clone(),
                            runner_group_id: pool.runner_group_id,
                            labels: effective_runner_labels(&provider, &labels),
                            work_folder: "_work".into(),
                        },
                    )
                    .await?;
                request["jitConfig"] = Value::String(jit.encoded_jit_config);
                (Some(jit.runner.id), None)
            } else {
                let registration = app
                    .github
                    .generate_registration_token(target, &token)
                    .await?;
                request["registrationToken"] = Value::String(registration.token);
                request["registrationUrl"] = Value::String(runner_registration_url(
                    &pool.account_login,
                    target_repository.as_ref(),
                ));
                if pool.scope == "organization" && pool.runner_group_id != 1 {
                    let group = app
                        .github
                        .runner_group_name(&pool.account_login, pool.runner_group_id, &token)
                        .await?;
                    request["runnerGroup"] = Value::String(group);
                }
                (None, None)
            }
        };
        if let Some(github_runner_id) = github_runner_id {
            sqlx::query("UPDATE runners SET github_runner_id=?,updated_at=? WHERE id=?")
                .bind(github_runner_id)
                .bind(now_millis())
                .bind(&runner_id)
                .execute(&app.database)
                .await?;
        }
        let created = match manager_request::<CreatedRunner>(app, Method::POST, "v1/runners", Some(request)).await {
            Ok(created) => created,
            Err(error) => {
                if gridops_core::failure_cleanup(true, bitbucket_runner_uuid.is_some(), false)
                    .remove_provider_runner
                    && let (Some(connection), Some(bitbucket_runner_uuid)) = (&bitbucket_connection, &bitbucket_runner_uuid)
                    && let Ok(Some(access_token)) = runtime_secret(app, &connection.access_token_key).await
                {
                    let _ = app.bitbucket.delete_runner(
                        &BitbucketRunnerTarget::Workspace { workspace: connection.workspace.clone() },
                        &access_token,
                        bitbucket_runner_uuid,
                    ).await;
                }
                return Err(error);
            }
        };
        let updated = now_millis();
        sqlx::query("UPDATE runners SET github_runner_id=?,bitbucket_runner_uuid=?,container_id=?,container_name=?,status=?,registered_at=?,last_heartbeat_at=?,updated_at=? WHERE id=?")
            .bind(github_runner_id)
            .bind(&bitbucket_runner_uuid)
            .bind(&created.id)
            .bind(&created.name)
            .bind(if created.state == "running" { "online" } else { "starting" })
            .bind(updated)
            .bind(updated)
            .bind(updated)
            .bind(&runner_id)
            .execute(&app.database)
            .await?;
        Result::<()>::Ok(())
    }
        .await;
    if let Err(error) = result {
        if gridops_core::failure_cleanup(true, false, false).release_capacity {
            release_runner_capacity(app, &capacity_lease).await;
        }
        let message = error.to_string().chars().take(2_000).collect::<String>();
        let manager_cleanup = match cleanup_unrecorded_manager_runner(app, &runner_id).await {
            Ok(Some(container_id)) => json!({ "status": "removed", "containerId": container_id }),
            Ok(None) => json!({ "status": "not-found" }),
            Err(cleanup_error) => {
                tracing::warn!(runner_id = %runner_id, error = ?cleanup_error, "could not recover manager container after failed provisioning");
                json!({ "status": "failed", "error": cleanup_error.to_string().chars().take(500).collect::<String>() })
            }
        };
        sqlx::query("UPDATE runners SET status='failed',failure_reason=?,updated_at=? WHERE id=?")
            .bind(&message)
            .bind(now_millis())
            .bind(&runner_id)
            .execute(&app.database)
            .await?;
        system_event(
            app,
            Some(&pool.id),
            "error",
            "Runner provisioning failed",
            &message,
            json!({ "runnerId": runner_id, "managerContainerCleanup": manager_cleanup }),
        )
        .await?;
        if deferred_manager_error(&error) {
            return Ok(ProvisionAttempt::Deferred {
                full_provider: host_capacity_exhausted(Some(&message)).then_some(provider),
                reason: Some(message),
            });
        }
        record_provision_failure(&app.database, pool).await?;
        return Err(error);
    }
    sqlx::query(
        "UPDATE runner_pools SET provision_failure_count=0,provision_retry_at=NULL,provision_circuit_open=0 WHERE id=?",
    )
    .bind(&pool.id)
    .execute(&app.database)
    .await?;
    set_pool_wait_reason(&app.database, &pool.id, None).await?;
    system_audit(
        app,
        "runner.provisioned",
        "runner",
        &runner_id,
        json!({ "poolId": pool.id, "provider": provider, "repositoryId": target_repository.as_ref().map(|repository| repository.repository_id) }),
    )
    .await?;
    Ok(ProvisionAttempt::Provisioned)
}

/// Deregisters and retires a runner. Returns `false`, leaving it in place
/// marked busy, when GitHub says it is running a job and its runtime isn't
/// known to be gone: its busy flag was missed, as happens when the
/// `in_progress` webhook arrives while the API is restarting, and removing it
/// would kill the job.
async fn delete_runner(app: &Reconciler, pool: &Pool, runner: &Runner) -> Result<bool> {
    if runner.container_id.is_some()
        && let Err(error) = archive_runner_logs(app, pool, runner).await
    {
        tracing::warn!(runner_id = %runner.id, error = ?error, "could not archive runner logs");
    }
    if runner.ci_platform == "bitbucket" {
        // A runner whose registration failed never reached Bitbucket, so there
        // is nothing to deregister. Requiring a UUID here used to fail every
        // pass and wedge the whole pool behind one failed runner.
        if let Some(runner_uuid) = bitbucket_registration(runner) {
            deregister_bitbucket_runner(app, runner, runner_uuid).await?;
        }
    } else {
        let target = match (&runner.repository_owner, &runner.repository_name) {
            (Some(owner), Some(repository)) => RunnerTarget::Repository { owner, repository },
            _ => RunnerTarget::Organization {
                organization: &pool.account_login,
            },
        };
        match installation_token(app, runner.installation_id).await {
            Ok(Some(token)) => {
                let github_cleanup = async {
                    let github_runner_id = match runner.github_runner_id {
                        Some(id) => Some(id),
                        None => app
                            .github
                            .runner_by_name(target, &token, &runner.name)
                            .await?
                            .map(|runner| runner.id),
                    };
                    let Some(github_runner_id) = github_runner_id else {
                        return Result::<bool>::Ok(true);
                    };
                    let path = match (&runner.repository_owner, &runner.repository_name) {
                        (Some(owner), Some(repository)) => format!(
                            "/repos/{owner}/{repository}/actions/runners/{github_runner_id}"
                        ),
                        _ => format!(
                            "/orgs/{}/actions/runners/{github_runner_id}",
                            pool.account_login
                        ),
                    };
                    app.github.remove_runner(&path, &token).await
                }
                .await;
                match github_cleanup {
                    Ok(false) if runner.status != "failed" => {
                        tracing::info!(runner_id = %runner.id, "GitHub says the runner is running a job; keeping it");
                        let now = now_millis();
                        sqlx::query(
                            "UPDATE runners SET busy=1,status='busy',updated_at=? WHERE id=? AND deleted_at IS NULL",
                        )
                        .bind(now)
                        .bind(&runner.id)
                        .execute(&app.database)
                        .await?;
                        return Ok(false);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(runner_id = %runner.id, error = ?error, "could not remove GitHub runner registration");
                    }
                }
            }
            Ok(None) => {
                tracing::warn!(runner_id = %runner.id, "GitHub App credentials unavailable during runner cleanup");
            }
            Err(error) => {
                tracing::warn!(runner_id = %runner.id, error = ?error, "GitHub installation unavailable during runner cleanup");
            }
        }
    }
    if let Some(container_id) = &runner.container_id {
        remove_manager_container(app, container_id).await?;
    }
    let now = now_millis();
    sqlx::query("UPDATE runners SET status='deleted',busy=0,deleted_at=?,updated_at=? WHERE id=?")
        .bind(now)
        .bind(now)
        .bind(&runner.id)
        .execute(&app.database)
        .await?;
    system_audit(
        app,
        "runner.deleted",
        "runner",
        &runner.id,
        json!({ "poolId": pool.id }),
    )
    .await?;
    Ok(true)
}

/// The Bitbucket registration to remove when deleting `runner`, if it ever
/// registered.
fn bitbucket_registration(runner: &Runner) -> Option<&str> {
    runner
        .bitbucket_runner_uuid
        .as_deref()
        .filter(|uuid| !uuid.is_empty())
}

async fn deregister_bitbucket_runner(
    app: &Reconciler,
    runner: &Runner,
    runner_uuid: &str,
) -> Result<()> {
    let connection_id = runner
        .bitbucket_connection_id
        .as_deref()
        .context("Bitbucket runner is missing its workspace connection")?;
    let connection = sqlx::query_as::<_, BitbucketPoolConnection>(
        "SELECT id,workspace,workspace_uuid,access_token_key FROM bitbucket_connections WHERE id=?",
    )
    .bind(connection_id)
    .fetch_optional(&app.database)
    .await?
    .context("Bitbucket workspace connection no longer exists")?;
    let access_token = runtime_secret(app, &connection.access_token_key)
        .await?
        .context("Bitbucket workspace credentials are unavailable")?;
    app.bitbucket
        .delete_runner(
            &BitbucketRunnerTarget::Workspace {
                workspace: connection.workspace,
            },
            &access_token,
            runner_uuid,
        )
        .await
}

async fn remove_manager_container(app: &Reconciler, container_id: &str) -> Result<()> {
    let token = app
        .config
        .manager_token()
        .context("GRIDOPS_MANAGER_TOKEN is required")?
        .expose_secret();
    let url = app
        .config
        .manager_url()
        .join(&format!("v1/runners/{container_id}"))?;
    let response = app.http.delete(url).bearer_auth(token).send().await?;
    let status = response.status();
    if status.is_success() || status == StatusCode::NOT_FOUND {
        return Ok(());
    }
    let detail = response.text().await.unwrap_or_default();
    bail!(
        "runner manager deletion failed ({status}): {}",
        detail.chars().take(500).collect::<String>()
    )
}

async fn cleanup_unrecorded_manager_runner(
    app: &Reconciler,
    runner_id: &str,
) -> Result<Option<String>> {
    let managed = manager_request::<ManagerRunners>(app, Method::GET, "v1/runners", None).await?;
    let Some(container_id) = managed
        .runners
        .into_iter()
        .find(|runner| {
            runner
                .labels
                .get("io.gridops.runner-id")
                .is_some_and(|id| id == runner_id)
        })
        .map(|runner| runner.id)
    else {
        return Ok(None);
    };
    remove_manager_container(app, &container_id).await?;
    Ok(Some(container_id))
}

async fn cleanup_orphaned_manager_containers(
    app: &Reconciler,
    managed: &[ManagedRunner],
) -> Result<()> {
    let live_runner_ids =
        sqlx::query_scalar::<_, String>("SELECT id FROM runners WHERE deleted_at IS NULL")
            .fetch_all(&app.database)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
    for runner in orphaned_manager_containers(managed, &live_runner_ids) {
        let Some(runner_id) = runner.labels.get("io.gridops.runner-id") else {
            continue;
        };
        let pool_id = runner.labels.get("io.gridops.pool-id").map(String::as_str);
        match remove_manager_container(app, &runner.id).await {
            Ok(()) => {
                tracing::info!(container_id = %runner.id, runner_id, "removed orphaned runner container");
                system_event(
                    app,
                    pool_id,
                    "warning",
                    "Orphaned runner container removed",
                    "GridOps removed an unreferenced stopped runner container before it could consume host capacity.",
                    json!({ "runnerId": runner_id, "containerId": runner.id, "state": runner.state }),
                )
                .await?;
            }
            Err(error) => {
                tracing::warn!(container_id = %runner.id, runner_id, error = ?error, "could not remove orphaned runner container");
            }
        }
    }
    Ok(())
}

fn orphaned_manager_containers<'a>(
    managed: &'a [ManagedRunner],
    live_runner_ids: &HashSet<String>,
) -> Vec<&'a ManagedRunner> {
    managed
        .iter()
        .filter(|runner| {
            matches!(runner.state.as_str(), "created" | "exited" | "dead")
                && runner
                    .labels
                    .get("io.gridops.managed")
                    .is_some_and(|managed| managed == "true")
                && runner
                    .labels
                    .get("io.gridops.runner-id")
                    .is_some_and(|runner_id| !live_runner_ids.contains(runner_id))
        })
        .collect()
}

async fn runners(app: &Reconciler, pool_id: &str) -> Result<Vec<Runner>> {
    Ok(sqlx::query_as::<_, Runner>(
        r#"SELECT runner.id,COALESCE(repo.installation_id,pool.installation_id) AS installation_id,
           runner.name,runner.container_id,runner.github_runner_id,runner.ci_platform,
           runner.bitbucket_connection_id,runner.bitbucket_runner_uuid,
           runner.target_repository_id,repo.owner AS repository_owner,repo.name AS repository_name,
           runner.provider,runner.status,runner.busy,runner.last_job_id,runner.configuration_version,runner.updated_at,
           EXISTS(SELECT 1 FROM workload_intents intent WHERE intent.workload_id=runner.id) AS fleet
           FROM runners runner JOIN runner_pools pool ON pool.id=runner.pool_id
           LEFT JOIN repositories repo ON repo.id=runner.target_repository_id
           WHERE runner.pool_id=? AND runner.deleted_at IS NULL ORDER BY runner.created_at DESC"#,
    )
    .bind(pool_id)
    .fetch_all(&app.database)
    .await?)
}

async fn cleanup_retention(app: &Reconciler) -> Result<()> {
    let now = now_millis();
    let webhook_cutoff =
        now - setting_i64(&app.database, "webhookRetentionDays", 90).await * 86_400_000;
    let audit_cutoff =
        now - setting_i64(&app.database, "auditRetentionDays", 365).await * 86_400_000;
    let capacity_cutoff = now - 31 * 86_400_000;
    let expired_logs =
        sqlx::query("SELECT path FROM log_streams WHERE expires_at IS NOT NULL AND expires_at < ?")
            .bind(now)
            .fetch_all(&app.database)
            .await?;
    for row in expired_logs {
        let filename = row.get::<String, _>("path");
        if let Some(path) = safe_log_path(app, &filename) {
            match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(filename, error = ?error, "could not remove expired log file");
                }
            }
        }
    }
    sqlx::query("DELETE FROM log_streams WHERE expires_at IS NOT NULL AND expires_at < ?")
        .bind(now)
        .execute(&app.database)
        .await?;
    // Fleet output whose runner ended without the agent closing its stream
    // (it restarted, or lost the session) still counts against the budget.
    sqlx::query(
        "UPDATE log_streams SET complete=1 WHERE complete=0 AND updated_at < ?
           AND id IN (SELECT o.log_stream_id FROM fleet_output_streams o
             JOIN workload_placements p ON p.id=o.placement_id
             WHERE p.state IN ('stopped','cleaned','failed'))",
    )
    .bind(now - 3_600_000)
    .execute(&app.database)
    .await?;
    enforce_log_storage_budget(app).await?;
    sqlx::query("DELETE FROM webhook_deliveries WHERE received_at < ?")
        .bind(webhook_cutoff)
        .execute(&app.database)
        .await?;
    sqlx::query("DELETE FROM audit_events WHERE created_at < ?")
        .bind(audit_cutoff)
        .execute(&app.database)
        .await?;
    sqlx::query("DELETE FROM capacity_samples WHERE recorded_at < ?")
        .bind(capacity_cutoff)
        .execute(&app.database)
        .await?;
    // Machine samples back the overview's workload chart, which shows up to 30 days.
    sqlx::query("DELETE FROM host_samples WHERE received_at < ?")
        .bind(capacity_cutoff)
        .execute(&app.database)
        .await?;
    sqlx::query("DELETE FROM reconciler_passes WHERE started_at < ?")
        .bind(capacity_cutoff)
        .execute(&app.database)
        .await?;
    Ok(())
}

async fn enforce_log_storage_budget(app: &Reconciler) -> Result<()> {
    let budget_bytes = setting_i64(&app.database, "logStorageBudgetMb", 4_096)
        .await
        .clamp(100, 1_048_576)
        .saturating_mul(1_024 * 1_024);
    let mut retained_bytes = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(size_bytes),0) FROM log_streams WHERE complete=1",
    )
    .fetch_one(&app.database)
    .await?;
    if retained_bytes <= budget_bytes {
        return Ok(());
    }
    let logs = sqlx::query(
        "SELECT id,path,size_bytes FROM log_streams WHERE complete=1 ORDER BY created_at,id",
    )
    .fetch_all(&app.database)
    .await?;
    for row in logs {
        if retained_bytes <= budget_bytes {
            break;
        }
        let id = row.get::<String, _>("id");
        let filename = row.get::<String, _>("path");
        let size_bytes = row.get::<i64, _>("size_bytes").max(0);
        let Some(path) = safe_log_path(app, &filename) else {
            tracing::warn!(
                id,
                filename,
                "refusing to remove an invalid retained-log path"
            );
            continue;
        };
        match tokio::fs::remove_file(path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(id, filename, error = ?error, "could not remove retained log while enforcing storage budget");
                continue;
            }
        }
        sqlx::query("DELETE FROM log_streams WHERE id=?")
            .bind(&id)
            .execute(&app.database)
            .await?;
        retained_bytes = retained_bytes.saturating_sub(size_bytes);
    }
    Ok(())
}

async fn archive_runner_logs(app: &Reconciler, pool: &Pool, runner: &Runner) -> Result<()> {
    let Some(container_id) = runner.container_id.as_deref() else {
        return Ok(());
    };
    if sqlx::query_scalar::<_, String>(
        "SELECT id FROM log_streams WHERE runner_id=? AND source=? AND complete=1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&runner.id)
    .bind(&runner.provider)
    .fetch_optional(&app.database)
    .await?
    .is_some()
    {
        return Ok(());
    }
    let token = app
        .config
        .manager_token()
        .context("GRIDOPS_MANAGER_TOKEN is required")?
        .expose_secret();
    let url = app
        .config
        .manager_url()
        .join(&format!("v1/runners/{container_id}/logs?tail=100000"))?;
    let response = app.http.get(url).bearer_auth(token).send().await?;
    let status = response.status();
    if !status.is_success() {
        let detail = response.text().await.unwrap_or_default();
        bail!(
            "runner log download failed ({status}): {}",
            detail.chars().take(300).collect::<String>()
        );
    }

    tokio::fs::create_dir_all(app.config.log_directory()).await?;
    let stream_id = uuid::Uuid::new_v4().to_string();
    let filename = format!("{stream_id}.log");
    let path = safe_log_path(app, &filename).context("generated log path is invalid")?;
    let write_result = async {
        let mut file = tokio::fs::File::create(&path).await?;
        let mut stream = response.bytes_stream();
        let mut checksum = Sha256::new();
        let mut size_bytes = 0_i64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let remaining = usize::try_from(MAX_ARCHIVED_LOG_BYTES - size_bytes).unwrap_or(0);
            if remaining == 0 {
                break;
            }
            let retained = &chunk[..chunk.len().min(remaining)];
            checksum.update(retained);
            size_bytes =
                size_bytes.saturating_add(i64::try_from(retained.len()).unwrap_or(i64::MAX));
            file.write_all(retained).await?;
        }
        file.flush().await?;
        Result::<(i64, String)>::Ok((size_bytes, hex::encode(checksum.finalize())))
    }
    .await;
    let (size_bytes, checksum) = match write_result {
        Ok(result) => result,
        Err(error) => {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(error);
        }
    };
    let now = now_millis();
    let retention_days = setting_i64(&app.database, "logRetentionDays", 30).await;
    let repository = runner
        .repository_owner
        .as_ref()
        .zip(runner.repository_name.as_ref())
        .map(|(owner, repository)| format!("{owner}/{repository}"));
    let inserted = sqlx::query(
        r#"INSERT INTO log_streams (
          id,runner_id,job_id,installation_id,runner_name,pool_name,repository,source,path,
          size_bytes,complete,checksum,expires_at,created_at,updated_at
        ) VALUES (?,?,?,?,?,?,?,?,?, ?,1,?,?,?,?)"#,
    )
    .bind(&stream_id)
    .bind(&runner.id)
    .bind(runner.last_job_id)
    .bind(runner.installation_id)
    .bind(&runner.name)
    .bind(&pool.name)
    .bind(repository)
    .bind(&runner.provider)
    .bind(&filename)
    .bind(size_bytes)
    .bind(checksum)
    .bind(now + retention_days * 86_400_000)
    .bind(now)
    .bind(now)
    .execute(&app.database)
    .await;
    if let Err(error) = inserted {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(error.into());
    }
    Ok(())
}

fn safe_log_path(app: &Reconciler, filename: &str) -> Option<std::path::PathBuf> {
    let candidate = std::path::Path::new(filename);
    if candidate.is_absolute()
        || candidate.components().count() != 1
        || candidate.file_name().and_then(|value| value.to_str()) != Some(filename)
        || filename.len() > 128
        || !filename
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return None;
    }
    Some(app.config.log_directory().join(filename))
}

async fn manager_request<T: serde::de::DeserializeOwned>(
    app: &Reconciler,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<T> {
    let token = app
        .config
        .manager_token()
        .context("GRIDOPS_MANAGER_TOKEN is required")?
        .expose_secret();
    let url = app
        .config
        .manager_url()
        .join(path.trim_start_matches('/'))?;
    let mut request = app.http.request(method, url).bearer_auth(token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        if status == StatusCode::NOT_FOUND {
            bail!("runner manager object was not found");
        }
        bail!(manager_failure(status, &text));
    }
    // A 204 (a removed sandbox or a released lease) has no body; callers that
    // ignore the result read it as null.
    if text.trim().is_empty() {
        return Ok(serde_json::from_value(Value::Null)?);
    }
    Ok(serde_json::from_str(&text)?)
}

/// What a failed manager request is recorded as: its status, the manager's
/// error code and its reason, which is never left empty.
fn manager_failure(status: StatusCode, text: &str) -> String {
    let payload = serde_json::from_str::<Value>(text).ok();
    let code = payload
        .as_ref()
        .and_then(|value| value.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("manager_error");
    let message = payload
        .as_ref()
        .and_then(|value| value.get("error"))
        .and_then(Value::as_str)
        .unwrap_or(text)
        .trim();
    let message = if message.is_empty() {
        "the runner manager answered without saying why"
    } else {
        message
    };
    format!(
        "runner manager request failed ({status}) [{code}]: {}",
        message.chars().take(500).collect::<String>()
    )
}

struct CapacityAdmission {
    lease_id: Option<String>,
    reason: Option<String>,
}

async fn reserve_runner_capacity(
    app: &Reconciler,
    runner_id: &str,
    pool_id: &str,
    provider: &str,
    runtime: &str,
    cpu_limit: f64,
    memory_limit_mb: i64,
) -> Result<CapacityAdmission> {
    let response = manager_request::<Value>(
        app,
        Method::POST,
        "v1/admissions",
        Some(json!({
            "runnerId": runner_id,
            "poolId": pool_id,
            "provider": provider,
            "runtime": runtime,
            "cpuLimit": cpu_limit,
            "memoryLimitMb": memory_limit_mb,
            "reconcilePass": app.pass.load(Ordering::Relaxed),
        })),
    )
    .await;
    match response {
        Ok(value) => Ok(CapacityAdmission {
            lease_id: value
                .get("leaseId")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            reason: None,
        }),
        Err(error) if deferred_manager_error(&error) => Ok(CapacityAdmission {
            lease_id: None,
            reason: Some(error.to_string()),
        }),
        Err(error) => Err(error),
    }
}

async fn release_runner_capacity(app: &Reconciler, lease_id: &str) {
    if let Err(error) = manager_request::<Value>(
        app,
        Method::DELETE,
        &format!("v1/admissions/{lease_id}"),
        None,
    )
    .await
    {
        tracing::warn!(lease_id, error = ?error, "could not release runner capacity reservation");
    }
}

/// Manager refusals that mean "not now" rather than a fault: the pool waits
/// and they never count towards its provisioning circuit. Lacking a macOS VM
/// slot is one, whether admission refused the start (`vm_slot_busy`) or the
/// Tart agent's safety-net wait ran out (`vm_slot_unavailable`, #95).
fn deferred_manager_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    [
        "[host_capacity_exhausted]",
        "[host_disk_guardrail]",
        "[provisioning_paused]",
        "[vm_slot_busy]",
        "[vm_slot_unavailable]",
    ]
    .iter()
    .any(|code| message.contains(code))
}

fn host_capacity_exhausted(reason: Option<&str>) -> bool {
    reason.is_some_and(|reason| reason.contains("[host_capacity_exhausted]"))
}

/// How a pool waits when its next runner was deferred.
#[derive(Debug, PartialEq, Eq)]
struct PoolWait {
    /// When to ask again; `None` asks on the very next pass.
    retry_after_ms: Option<i64>,
    /// The manager's reason, without the transport prefix, for the pool page.
    reason: Option<String>,
}

/// A Tart VM start refused for lack of one of the Mac's VM slots, or of room
/// in `GridOps`'s own budget on the Mac (`full_provider` is `tart`), asks again
/// on the next pass, so the pool keeps its place in the manager's line for
/// the Mac (a pool that misses a whole pass leaves it). Other waits pause
/// half a minute.
fn pool_wait(reason: Option<&str>, full_provider: Option<&str>) -> PoolWait {
    let in_tart_line = reason.is_some_and(|reason| reason.contains("[vm_slot_busy]"))
        || (full_provider == Some("tart") && host_capacity_exhausted(reason));
    let retry_after_ms = if in_tart_line { None } else { Some(30_000) };
    PoolWait {
        retry_after_ms,
        reason: reason.map(|reason| {
            reason
                .split_once("]: ")
                .map_or(reason, |(_, message)| message)
                .chars()
                .take(2_000)
                .collect()
        }),
    }
}

async fn defer_pool_capacity(
    database: &SqlitePool,
    pool: &Pool,
    reason: Option<&str>,
    full_provider: Option<&str>,
) -> Result<()> {
    let wait = pool_wait(reason, full_provider);
    let retry_at = wait
        .retry_after_ms
        .map(|delay| now_millis().saturating_add(delay));
    sqlx::query(
        "UPDATE runner_pools SET state='waiting',provision_retry_at=?,updated_at=? WHERE id=?",
    )
    .bind(retry_at)
    .bind(now_millis())
    .bind(&pool.id)
    .execute(database)
    .await?;
    set_pool_wait_reason(database, &pool.id, wait.reason.as_deref()).await?;
    if pool.state != "waiting" {
        let mut metadata = json!({ "retryAt": retry_at });
        if let Some(reason) = reason {
            metadata["reason"] = Value::String(reason.chars().take(2_000).collect());
        }
        pool_event(
            database,
            Some(&pool.id),
            "warning",
            "Runner provisioning waiting",
            "The runner host has no safe capacity available; GridOps will retry without exceeding host limits.",
            metadata,
        )
        .await?;
    }
    Ok(())
}

/// Records why the pool waits, or clears it with `None`.
async fn set_pool_wait_reason(
    database: &SqlitePool,
    pool_id: &str,
    reason: Option<&str>,
) -> Result<()> {
    match reason {
        Some(reason) => {
            sqlx::query(
                "INSERT INTO runner_pool_waits (pool_id,reason,updated_at) VALUES (?,?,?)
                 ON CONFLICT(pool_id) DO UPDATE SET reason=excluded.reason,updated_at=excluded.updated_at",
            )
            .bind(pool_id)
            .bind(reason)
            .bind(now_millis())
            .execute(database)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM runner_pool_waits WHERE pool_id=?")
                .bind(pool_id)
                .execute(database)
                .await?;
        }
    }
    Ok(())
}

async fn record_provision_failure(database: &SqlitePool, pool: &Pool) -> Result<()> {
    let attempts = pool.provision_failure_count.saturating_add(1);
    let (circuit_open, delay_ms) = provision_failure_policy(attempts);
    let retry_at = now_millis().saturating_add(delay_ms);
    sqlx::query(
        "UPDATE runner_pools SET provision_failure_count=?,provision_retry_at=?,provision_circuit_open=?,state=?,updated_at=? WHERE id=?",
    )
    .bind(attempts)
    .bind(retry_at)
    .bind(circuit_open)
    .bind(if circuit_open { "blocked" } else { "backoff" })
    .bind(now_millis())
    .bind(&pool.id)
    .execute(database)
    .await?;
    // A start that failed ends any wait before it: the pause now is the
    // failure's, and the pool is shown backing off or blocked.
    set_pool_wait_reason(database, &pool.id, None).await?;
    if circuit_open && !pool.provision_circuit_open {
        pool_event(
            database,
            Some(&pool.id),
            "error",
            "Provisioning circuit opened",
            "GridOps paused provisioning for this pool after three consecutive failures and retries automatically every 15 minutes. Retry the pool to try sooner.",
            json!({ "attempts": attempts, "retryAt": retry_at }),
        )
        .await?;
    }
    Ok(())
}

const PROVISION_CIRCUIT_COOLDOWN_MS: i64 = 15 * 60 * 1_000;

/// Whether the pool's circuit is open after `attempts` consecutive failures,
/// and how long to wait before the next attempt.
fn provision_failure_policy(attempts: i64) -> (bool, i64) {
    if attempts >= 3 {
        (true, PROVISION_CIRCUIT_COOLDOWN_MS)
    } else {
        (false, provision_backoff_ms(attempts))
    }
}

fn provision_backoff_ms(attempts: i64) -> i64 {
    let exponent = u32::try_from(attempts.saturating_sub(1).clamp(0, 10)).unwrap_or(10);
    30_000_i64.saturating_mul(2_i64.saturating_pow(exponent))
}

async fn setting_i64(database: &SqlitePool, key: &str, fallback: i64) -> i64 {
    sqlx::query("SELECT value FROM settings WHERE key=?")
        .bind(key)
        .fetch_optional(database)
        .await
        .ok()
        .flatten()
        .and_then(|row| serde_json::from_str::<i64>(row.get::<&str, _>("value")).ok())
        .unwrap_or(fallback)
}

async fn setting_bool(database: &SqlitePool, key: &str, fallback: bool) -> bool {
    sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
        .bind(key)
        .fetch_optional(database)
        .await
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str::<bool>(&value).ok())
        .unwrap_or(fallback)
}

fn runner_registration_url(
    account_login: &str,
    repository: Option<&gridops_core::RepositoryCapacity>,
) -> String {
    match repository {
        Some(repository) => format!(
            "https://github.com/{}/{}",
            repository.owner, repository.name
        ),
        None => format!("https://github.com/{account_login}"),
    }
}

async fn installation_token(app: &Reconciler, installation_id: i64) -> Result<Option<String>> {
    let app_id = match runtime_secret(app, "github.app_id").await? {
        Some(value) => Some(value),
        None => app.config.github_app_id().map(ToOwned::to_owned),
    };
    let private_key = match runtime_secret(app, "github.app_private_key").await? {
        Some(value) => Some(value),
        None => app
            .config
            .github_app_private_key()
            .map(|value| value.expose_secret().to_owned()),
    };
    let Some((app_id, private_key)) = app_id.zip(private_key) else {
        return Ok(None);
    };
    app.github
        .installation_token_with_credentials(installation_id, &app_id, &private_key)
        .await
        .map(Some)
}

async fn runtime_secret(app: &Reconciler, key: &str) -> Result<Option<String>> {
    let sealed = sqlx::query_scalar::<_, String>("SELECT value FROM runtime_secrets WHERE key=?")
        .bind(key)
        .fetch_optional(&app.database)
        .await?;
    sealed
        .map(|value| {
            app.vault
                .open(&value)
                .context("could not decrypt runtime secret")
        })
        .transpose()
}

async fn system_event(
    app: &Reconciler,
    pool_id: Option<&str>,
    level: &str,
    event: &str,
    message: &str,
    metadata: Value,
) -> Result<()> {
    pool_event(&app.database, pool_id, level, event, message, metadata).await
}

async fn pool_event(
    database: &SqlitePool,
    pool_id: Option<&str>,
    level: &str,
    event: &str,
    message: &str,
    metadata: Value,
) -> Result<()> {
    sqlx::query("INSERT INTO runner_events (id,pool_id,level,event,message,metadata,created_at) VALUES (?,?,?,?,?,?,?)")
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(pool_id)
        .bind(level)
        .bind(event)
        .bind(message)
        .bind(metadata.to_string())
        .bind(now_millis())
        .execute(database)
        .await?;
    Ok(())
}

async fn system_audit(
    app: &Reconciler,
    action: &str,
    target_type: &str,
    target_id: &str,
    metadata: Value,
) -> Result<()> {
    sqlx::query("INSERT INTO audit_events (id,actor_label,action,target_type,target_id,metadata,created_at) VALUES (?,'system',?,?,?,?,?)")
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(action)
        .bind(target_type)
        .bind(target_id)
        .bind(metadata.to_string())
        .bind(now_millis())
        .execute(&app.database)
        .await?;
    Ok(())
}

fn active_status(status: &str) -> bool {
    matches!(
        status,
        "starting" | "online" | "idle" | "busy" | "paused" | "stopped"
    )
}

fn runner_needs_update(pool: &Pool, runner: &Runner) -> bool {
    runner.configuration_version < pool.configuration_version
}

fn idle_period_elapsed(now: i64, idle_timeout_minutes: i64, last_activity: i64) -> bool {
    now.saturating_sub(last_activity) >= idle_timeout_minutes.saturating_mul(60_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridops_core::connect_database_path;
    use std::fs;

    fn pool(configuration_version: i64) -> Pool {
        Pool {
            id: "pool-1".into(),
            installation_id: 1,
            account_login: "octo-org".into(),
            scope: "organization".into(),
            name: "linux".into(),
            state: "active".into(),
            mode: "ephemeral".into(),
            provider: "docker".into(),
            providers: r#"["docker"]"#.into(),
            labels: "[]".into(),
            docker_image: "runner:latest".into(),
            tart_image: "gridops-macos-tahoe-base".into(),
            macos_runtime: "vm".into(),
            desired_count: 1,
            min_count: 0,
            max_count: 10,
            cpu_limit: 2.0,
            memory_limit_mb: 4096,
            runner_group_id: 1,
            ephemeral: true,
            paused: false,
            autoscaling_enabled: true,
            queue_scale_factor: 1,
            idle_timeout_minutes: 5,
            configuration_version,
            provision_failure_count: 0,
            provision_retry_at: None,
            provision_circuit_open: false,
            wait_recorded: false,
        }
    }

    fn runner(configuration_version: i64) -> Runner {
        Runner {
            id: "runner-1".into(),
            installation_id: 1,
            name: "linux-1234".into(),
            container_id: None,
            github_runner_id: None,
            ci_platform: "github".into(),
            bitbucket_connection_id: None,
            bitbucket_runner_uuid: None,
            target_repository_id: None,
            repository_owner: None,
            repository_name: None,
            provider: "docker".into(),
            status: "online".into(),
            busy: false,
            last_job_id: None,
            configuration_version,
            updated_at: 1_000,
            fleet: false,
        }
    }

    #[test]
    fn bitbucket_runners_carry_the_labels_bitbucket_requires() -> Result<()> {
        let labels = bitbucket_runner_labels("toppan", &["gridops".into(), "toppan".into()])?;
        assert_eq!(
            labels,
            ["gridops", "gridops.toppan", "macos", "self.hosted"]
        );
        let deduplicated = bitbucket_runner_labels(
            "ios-builds",
            &["self.hosted".into(), "macos".into(), "xcode16".into()],
        )?;
        assert_eq!(
            deduplicated,
            ["gridops.ios.builds", "macos", "self.hosted", "xcode16"]
        );
        assert!(bitbucket_runner_labels("toppan", &["Self-Hosted".into()]).is_err());
        Ok(())
    }

    #[test]
    fn bitbucket_runners_that_never_registered_need_no_deregistration() {
        let mut failed = runner(1);
        failed.ci_platform = "bitbucket".into();
        failed.status = "failed".into();
        assert_eq!(bitbucket_registration(&failed), None);
        failed.bitbucket_runner_uuid = Some(String::new());
        assert_eq!(bitbucket_registration(&failed), None);
        failed.bitbucket_runner_uuid = Some("{0f3b2c1d-aaaa-bbbb-cccc-123456789abc}".into());
        assert_eq!(
            bitbucket_registration(&failed),
            Some("{0f3b2c1d-aaaa-bbbb-cccc-123456789abc}")
        );
    }

    #[tokio::test]
    async fn superseded_intents_are_cancelled_immediately_for_replanning() -> Result<()> {
        let directory = std::env::temp_dir().join(format!(
            "gridops-superseded-intent-test-{}",
            uuid::Uuid::new_v4()
        ));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO runner_pools (id,installation_id,name,scope,image,created_at,updated_at)
              VALUES ('pool-1',1,'linux','organization','runner:latest',1,1);
            INSERT INTO runners (id,pool_id,name,provider,ci_platform,os,architecture,status,ephemeral,configuration_version,runtime,created_at,updated_at)
              VALUES ('62000000-0000-4000-8000-000000000001','pool-1','linux-1','docker','github','linux','X64','starting',1,1,'vm',1,1);
            INSERT INTO fleet_operations (id,principal_kind,principal_scope,kind,idempotency_key,request_hash,status,created_at,updated_at)
              VALUES ('62000000-0000-4000-8000-000000000001','reconciler','controller:reconciler','submit_workload','runner:62000000-0000-4000-8000-000000000001','hash','queued',1,1);
            INSERT INTO pool_execution_profiles (id,pool_id,name,runtime_kind,execution_os,architecture,mode,image,cpu_millis,memory_mib,disk_bytes,created_at,updated_at)
              VALUES ('40000000-0000-4000-8000-000000000001','pool-1','linux','docker','linux','x64','ephemeral','runner:latest',2000,2048,0,1,1);
            INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
              VALUES (7,1,'octo-org','app','octo-org/app',0,'main','https://github.com/octo-org/app',1,1,1);
            INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,repository_id)
              VALUES ('30000000-0000-4000-8000-000000000001','github_repository','github:repository:1:7',1,7);
            INSERT INTO profile_ci_targets (profile_id,target_id,enabled)
              VALUES ('40000000-0000-4000-8000-000000000001','30000000-0000-4000-8000-000000000001',1);
            INSERT INTO workload_intents (id,workload_id,generation,kind,pool_id,profile_id,target_id,agent_run_id,expected_profile_revision,status,reason_code,created_at,updated_at)
              VALUES ('62000000-0000-4000-8000-000000000001','62000000-0000-4000-8000-000000000001',1,'ci_runner','pool-1','40000000-0000-4000-8000-000000000001','30000000-0000-4000-8000-000000000001',NULL,1,'queued','stale_profile_revision',1,1);
            "#,
        )
        .execute(&database)
        .await?;

        let service = gridops_core::fleet::FleetService::new(database.clone());
        fleet::retire_superseded_intents(&database, &service).await?;

        let intent: (String, Option<String>) = sqlx::query_as(
            "SELECT status,reason_code FROM workload_intents WHERE id='62000000-0000-4000-8000-000000000001'",
        )
        .fetch_one(&database)
        .await?;
        assert_eq!(intent.0, "cancelled");
        let runner: (String, Option<String>) = sqlx::query_as(
            "SELECT status,failure_reason FROM runners WHERE id='62000000-0000-4000-8000-000000000001'",
        )
        .fetch_one(&database)
        .await?;
        assert_eq!(runner.0, "failed");
        assert!(runner.1.is_some_and(|reason| reason.contains("replaced")));
        Ok(())
    }

    #[tokio::test]
    async fn orphaned_queued_jobs_of_a_completed_run_are_cancelled() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("gridops-orphan-jobs-test-{}", uuid::Uuid::new_v4()));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
              VALUES (7,1,'octo-org','app','octo-org/app',0,'main','https://github.com/octo-org/app',1,1,1);
            INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,conclusion,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
              VALUES (500,7,'CI',1,'push','completed','success','abc','https://github.com/octo-org/app/actions/runs/500',1,1,1,1);
            INSERT INTO workflow_jobs (id,run_id,name,status,conclusion,labels,html_url,created_at,updated_at)
              VALUES (9001,500,'done','completed','success','[]','u',1,1),
                     (9002,500,'never ran','queued',NULL,'[]','u',1,1),
                     (9003,500,'in flight','in_progress',NULL,'[]','u',1,1);
            "#,
        )
        .execute(&database)
        .await?;

        cancel_orphaned_jobs(&database, 500).await?;

        let rows = sqlx::query(
            "SELECT id,status,conclusion FROM workflow_jobs WHERE run_id=500 ORDER BY id",
        )
        .fetch_all(&database)
        .await?;
        let summary: Vec<(i64, String, Option<String>)> = rows
            .iter()
            .map(|row| (row.get("id"), row.get("status"), row.get("conclusion")))
            .collect();
        assert_eq!(
            summary,
            vec![
                (9001, "completed".to_owned(), Some("success".to_owned())),
                (9002, "completed".to_owned(), Some("cancelled".to_owned())),
                (9003, "completed".to_owned(), Some("cancelled".to_owned())),
            ]
        );
        // Existing conclusions are never overwritten.
        assert_eq!(summary[0].2.as_deref(), Some("success"));
        Ok(())
    }

    #[tokio::test]
    async fn loads_whole_cpu_limits_stored_as_sqlite_integers() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("gridops-cpu-limit-test-{}", uuid::Uuid::new_v4()));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO runner_pools (id,installation_id,name,scope,image,created_at,updated_at)
              VALUES ('pool-1',1,'linux','organization','runner:latest',1,1);
            "#,
        )
        .execute(&database)
        .await?;

        let storage_class =
            sqlx::query_scalar::<_, String>("SELECT typeof(cpu_limit) FROM runner_pools")
                .fetch_one(&database)
                .await?;
        assert_eq!(storage_class, "integer");
        let pools = load_pools(&database).await?;
        assert_eq!(pools.len(), 1);
        assert!((pools[0].cpu_limit - 2.0).abs() < f64::EPSILON);

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn detects_outdated_runner_generations() {
        assert!(runner_needs_update(&pool(2), &runner(1)));
        assert!(!runner_needs_update(&pool(2), &runner(2)));
        assert!(!runner_needs_update(&pool(2), &runner(3)));
    }

    #[test]
    fn applies_idle_timeout_to_activity_not_heartbeats() {
        let five_minutes = 5 * 60_000;
        assert!(!idle_period_elapsed(1_000 + five_minutes - 1, 5, 1_000));
        assert!(idle_period_elapsed(1_000 + five_minutes, 5, 1_000));
    }

    #[test]
    fn selects_only_stopped_manager_containers_without_live_runner_records() {
        let manager_runner = |id: &str, state: &str, runner_id: &str| ManagedRunner {
            id: id.into(),
            state: state.into(),
            labels: HashMap::from([
                ("io.gridops.managed".into(), "true".into()),
                ("io.gridops.runner-id".into(), runner_id.into()),
            ]),
        };
        let managed = vec![
            manager_runner("created-orphan", "created", "deleted-runner"),
            manager_runner("exited-orphan", "exited", "missing-runner"),
            manager_runner("running-orphan", "running", "missing-running"),
            manager_runner("live-runner", "created", "live-runner"),
        ];
        let live = HashSet::from(["live-runner".to_owned()]);
        let ids = orphaned_manager_containers(&managed, &live)
            .into_iter()
            .map(|runner| runner.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["created-orphan", "exited-orphan"]);
    }

    #[test]
    fn a_full_host_evicts_idle_runners_parked_on_repositories_without_work() {
        let repository = |repository_id: i64, queued: i64, active: i64| RepositoryCapacity {
            repository_id,
            installation_id: 1,
            owner: "octo".into(),
            name: format!("repo-{repository_id}"),
            queued,
            active,
            busy: 0,
            prioritised: 0,
            prioritised_since: None,
        };
        let parked = |id: &str, repository_id: i64| Runner {
            id: id.into(),
            target_repository_id: Some(repository_id),
            ..runner(1)
        };
        // Four idle runners fill the host while repositories 10 and 11 wait.
        let runners = vec![
            parked("runner-12", 12),
            parked("runner-13", 13),
            parked("runner-14", 14),
            parked("runner-15", 15),
        ];
        let mut capacities = vec![repository(10, 2, 0), repository(11, 2, 0)];
        capacities.extend((12..=15).map(|repository_id| repository(repository_id, 0, 1)));
        let providers = vec!["docker".to_owned(), "tart".to_owned()];
        let provider_capacities = vec![
            ProviderCapacity {
                provider: "docker".into(),
                queued: 4,
                active: 4,
                busy: 0,
            },
            ProviderCapacity {
                provider: "tart".into(),
                queued: 0,
                active: 0,
                busy: 0,
            },
        ];

        let evicted = surplus_runner(
            &runners,
            &capacities,
            &provider_capacities,
            &providers,
            1,
            Some("docker"),
        )
        .map(|runner| runner.id.as_str());
        assert_eq!(evicted, Some("runner-12"));
        // Freeing a Docker slot cannot help a full macOS host.
        assert!(
            surplus_runner(
                &runners,
                &capacities,
                &provider_capacities,
                &providers,
                1,
                Some("tart")
            )
            .is_none()
        );

        // Without queued work anywhere, idle runners are left alone.
        let idle = (12..=15)
            .map(|repository_id| repository(repository_id, 0, 1))
            .collect::<Vec<_>>();
        let quiet = vec![ProviderCapacity {
            provider: "docker".into(),
            queued: 0,
            active: 4,
            busy: 0,
        }];
        assert!(surplus_runner(&runners, &idle, &quiet, &providers, 1, Some("docker")).is_none());

        // A runner whose own repository has queued work is never the one evicted.
        let mut claimed = capacities.clone();
        claimed[2].queued = 1;
        let evicted = surplus_runner(
            &runners,
            &claimed,
            &provider_capacities,
            &providers,
            1,
            Some("docker"),
        )
        .map(|runner| runner.id.as_str());
        assert_eq!(evicted, Some("runner-13"));
    }

    #[test]
    fn only_host_capacity_exhaustion_frees_idle_runners() {
        assert!(host_capacity_exhausted(Some(
            "runner manager request failed (429 Too Many Requests) [host_capacity_exhausted]: Host runner limit reached (5/4 including reservations)."
        )));
        assert!(!host_capacity_exhausted(Some(
            "runner manager request failed (429 Too Many Requests) [host_disk_guardrail]: only 100 MB disk space remains"
        )));
        assert!(!host_capacity_exhausted(None));
    }

    #[test]
    fn a_manager_error_is_never_reported_without_a_reason() {
        // A timeout layer answers 408 with no body at all; the operator must
        // still learn what failed.
        assert_eq!(
            manager_failure(StatusCode::REQUEST_TIMEOUT, ""),
            "runner manager request failed (408 Request Timeout) [manager_error]: the runner manager answered without saying why"
        );
        assert_eq!(
            manager_failure(
                StatusCode::REQUEST_TIMEOUT,
                r#"{"code":"tart_start_timeout","error":"the Tart agent did not finish starting the macOS runner within 900 seconds"}"#
            ),
            "runner manager request failed (408 Request Timeout) [tart_start_timeout]: the Tart agent did not finish starting the macOS runner within 900 seconds"
        );
        assert_eq!(
            manager_failure(
                StatusCode::SERVICE_UNAVAILABLE,
                r#"{"code":"vm_slot_unavailable","error":"  "}"#
            ),
            "runner manager request failed (503 Service Unavailable) [vm_slot_unavailable]: the runner manager answered without saying why"
        );
        assert_eq!(
            manager_failure(StatusCode::BAD_GATEWAY, "upstream connect error"),
            "runner manager request failed (502 Bad Gateway) [manager_error]: upstream connect error"
        );
    }

    #[test]
    fn a_tart_start_that_found_no_vm_slot_is_deferred_not_counted_as_a_failed_start() {
        // Changed by #95 (it asserted the opposite after #87): the Tart agent
        // says this after its safety-net wait found every VM slot taken. A
        // start that waited for a slot is not a fault, so it is deferred like
        // a full host and never reaches the provisioning circuit.
        let no_slot = anyhow::anyhow!(
            "runner manager request failed (503 Service Unavailable) [vm_slot_unavailable]: no macOS VM slot became free within 600 seconds; the Mac allows two VMs at once, including VMs of other software"
        );
        assert!(deferred_manager_error(&no_slot));
        assert!(!host_capacity_exhausted(Some(&no_slot.to_string())));
        assert!(deferred_manager_error(&anyhow::anyhow!(
            "runner manager request failed (429 Too Many Requests) [host_capacity_exhausted]: Tart host budget would be exceeded"
        )));
    }

    const VM_SLOT_BUSY: &str = "runner manager request failed (429 Too Many Requests) [vm_slot_busy]: Waiting for a macOS VM slot: position 2 of 2 in the queue. 2 of 2 slots in use (GridOps runners: 1; other software: 1 — runner-warm-1 (ci)).";
    const VM_SLOT_UNAVAILABLE: &str = "runner manager request failed (503 Service Unavailable) [vm_slot_unavailable]: no macOS VM slot became free within 180 seconds; the Mac allows two VMs at once, including VMs of other software";

    #[test]
    fn a_start_refused_for_lack_of_a_vm_slot_is_deferred_not_failed() {
        assert!(deferred_manager_error(&anyhow::anyhow!(VM_SLOT_BUSY)));
        // A busy slot is not a full CPU or memory budget: idle runners of the
        // pool are not evicted for it.
        assert!(!host_capacity_exhausted(Some(VM_SLOT_BUSY)));
    }

    #[test]
    fn a_pool_waiting_for_a_vm_slot_asks_again_next_pass_to_keep_its_place() {
        let wait = pool_wait(Some(VM_SLOT_BUSY), None);
        assert_eq!(wait.retry_after_ms, None);
        assert_eq!(
            wait.reason.as_deref(),
            Some(
                "Waiting for a macOS VM slot: position 2 of 2 in the queue. 2 of 2 slots in use (GridOps runners: 1; other software: 1 — runner-warm-1 (ci))."
            )
        );
        // A Tart VM refused because GridOps's own budget on the Mac is full
        // waits in the same line, so it asks again next pass too.
        let tart_budget = "runner manager request failed (429 Too Many Requests) [host_capacity_exhausted]: Waiting for room on the Tart host: position 1 of 2 in the queue. Host runner limit reached (2/1 including reservations).";
        assert_eq!(
            pool_wait(Some(tart_budget), Some("tart")).retry_after_ms,
            None
        );
        // Other waits keep their half-minute pause.
        let full = pool_wait(
            Some(
                "runner manager request failed (429 Too Many Requests) [host_capacity_exhausted]: Host runner limit reached (5/4 including reservations).",
            ),
            Some("docker"),
        );
        assert_eq!(full.retry_after_ms, Some(30_000));
        assert_eq!(
            full.reason.as_deref(),
            Some("Host runner limit reached (5/4 including reservations).")
        );
        let no_slot_after_waiting = pool_wait(
            Some(
                "runner manager request failed (503 Service Unavailable) [vm_slot_unavailable]: no macOS VM slot became free within 180 seconds; the Mac allows two VMs at once, including VMs of other software",
            ),
            None,
        );
        assert_eq!(no_slot_after_waiting.retry_after_ms, Some(30_000));
        assert_eq!(pool_wait(None, None).reason, None);
    }

    async fn pool_row(
        database: &SqlitePool,
    ) -> Result<(String, i64, bool, Option<i64>, Option<String>)> {
        let row = sqlx::query(
            "SELECT p.state,p.provision_failure_count,p.provision_circuit_open,p.provision_retry_at,w.reason AS provision_wait_reason
             FROM runner_pools p LEFT JOIN runner_pool_waits w ON w.pool_id=p.id WHERE p.id='pool-1'",
        )
        .fetch_one(database)
        .await?;
        Ok((
            row.get("state"),
            row.get("provision_failure_count"),
            row.get("provision_circuit_open"),
            row.get("provision_retry_at"),
            row.get("provision_wait_reason"),
        ))
    }

    #[tokio::test]
    async fn waiting_for_a_vm_slot_never_opens_the_provisioning_circuit_and_says_why() -> Result<()>
    {
        let directory =
            std::env::temp_dir().join(format!("gridops-vm-slot-wait-{}", uuid::Uuid::new_v4()));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO runner_pools (id,installation_id,name,scope,image,created_at,updated_at)
              VALUES ('pool-1',1,'macos','organization','runner:latest',1,1);
            "#,
        )
        .execute(&database)
        .await?;

        // Many passes in a row find no slot: far more than the three failures
        // that open the circuit.
        for _ in 0..6 {
            let pool = load_pools(&database).await?.remove(0);
            defer_pool_capacity(&database, &pool, Some(VM_SLOT_BUSY), None).await?;
        }
        let (state, failures, circuit_open, retry_at, reason) = pool_row(&database).await?;
        assert_eq!(state, "waiting");
        assert_eq!((failures, circuit_open), (0, false));
        assert_eq!(retry_at, None, "the pool asks again on the next pass");
        assert!(
            reason.as_deref().is_some_and(
                |reason| reason.starts_with("Waiting for a macOS VM slot: position 2 of 2")
            ),
            "{reason:?}"
        );
        let events: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM runner_events WHERE pool_id='pool-1' AND event='Runner provisioning waiting'",
        )
        .fetch_one(&database)
        .await?;
        assert_eq!(
            events, 1,
            "one event when the wait begins, not one per pass"
        );

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    // ---- The state a waiting pool is shown in (#102) ----

    #[test]
    fn a_pool_whose_start_waits_for_capacity_is_waiting_whatever_its_failures() {
        let refused_for_a_slot = PassOutcome {
            capacity_deferred: true,
            ..PassOutcome::default()
        };
        assert_eq!(pool_state(&refused_for_a_slot), "waiting");
        // Earlier failures, and a circuit opened by them, do not hide it.
        for (failure_count, circuit_open) in [(1, false), (2, false), (3, true), (7, true)] {
            let outcome = PassOutcome {
                circuit_open,
                failure_count,
                ..refused_for_a_slot
            };
            assert_eq!(pool_state(&outcome), "waiting", "{outcome:?}");
        }
        // Nor during the half-minute pause after a deferred start (the Tart
        // agent's own slot wait ran out): the pool is still waiting.
        for (failure_count, circuit_open) in [(0, false), (2, false), (3, true)] {
            let pausing = PassOutcome {
                circuit_open,
                failure_count,
                retry_deferred: true,
                wait_recorded: true,
                ..PassOutcome::default()
            };
            assert_eq!(pool_state(&pausing), "waiting", "{pausing:?}");
        }
    }

    #[test]
    fn a_wait_does_not_hide_a_real_block_and_a_failure_is_not_a_wait() {
        let waiting = PassOutcome {
            capacity_deferred: true,
            failure_count: 3,
            circuit_open: true,
            ..PassOutcome::default()
        };
        let paused = PassOutcome {
            paused: true,
            ..waiting
        };
        assert_eq!(pool_state(&paused), "paused");
        let provisioning_paused = PassOutcome {
            provisioning_paused: true,
            ..waiting
        };
        assert_eq!(pool_state(&provisioning_paused), "provisioning-paused");
        let host_off = PassOutcome {
            host_blocked: true,
            ..waiting
        };
        assert_eq!(
            pool_state(&host_off),
            gridops_core::builtin_host::POOL_STATE
        );

        // After a failure the pool backs off, or its circuit cools down.
        let backing_off = PassOutcome {
            retry_deferred: true,
            failure_count: 1,
            ..PassOutcome::default()
        };
        assert_eq!(pool_state(&backing_off), "backoff");
        let cooling_down = PassOutcome {
            retry_deferred: true,
            failure_count: 3,
            circuit_open: true,
            ..PassOutcome::default()
        };
        assert_eq!(pool_state(&cooling_down), "blocked");
        // An open circuit with no start waiting (nothing asked this pass) is blocked.
        let idle = PassOutcome {
            failure_count: 3,
            circuit_open: true,
            ..PassOutcome::default()
        };
        assert_eq!(pool_state(&idle), "blocked");
        // A pause set by a deferral with no failures behind it stays waiting.
        let deferred = PassOutcome {
            retry_deferred: true,
            ..PassOutcome::default()
        };
        assert_eq!(pool_state(&deferred), "waiting");
        assert_eq!(pool_state(&PassOutcome::default()), "active");
    }

    async fn seed_macos_pool(
        database: &SqlitePool,
        failures: i64,
        circuit_open: bool,
    ) -> Result<()> {
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO runner_pools (id,installation_id,name,scope,image,created_at,updated_at)
              VALUES ('pool-1',1,'macos','organization','runner:latest',1,1);
            "#,
        )
        .execute(database)
        .await?;
        // The circuit's cool-down has passed, so this pass probes again.
        sqlx::query(
            "UPDATE runner_pools SET provision_failure_count=?,provision_circuit_open=?,provision_retry_at=NULL,state=? WHERE id='pool-1'",
        )
        .bind(failures)
        .bind(circuit_open)
        .bind(if circuit_open { "blocked" } else { "backoff" })
        .execute(database)
        .await?;
        Ok(())
    }

    /// One reconciler pass for a pool whose start is deferred with `reason`,
    /// ending as `reconcile_pool` ends it.
    async fn pass_deferred_with(database: &SqlitePool, reason: &str) -> Result<()> {
        let pool = load_pools(database).await?.remove(0);
        defer_pool_capacity(database, &pool, Some(reason), None).await?;
        let outcome = PassOutcome {
            capacity_deferred: true,
            ..PassOutcome::for_pool(&pool, now_millis())
        };
        finish_pool_pass(database, &pool.id, pool_state(&outcome)).await
    }

    /// One pass in which the pool asks for nothing (it is pausing).
    async fn pass_without_a_start(database: &SqlitePool) -> Result<()> {
        let pool = load_pools(database).await?.remove(0);
        let outcome = PassOutcome::for_pool(&pool, now_millis());
        finish_pool_pass(database, &pool.id, pool_state(&outcome)).await
    }

    async fn waiting_events(database: &SqlitePool) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT COUNT(*) FROM runner_events WHERE pool_id='pool-1' AND event='Runner provisioning waiting'",
        )
        .fetch_one(database)
        .await?)
    }

    /// #102's test: a pool whose circuit is still open from earlier failures
    /// waits in the Mac's VM slot queue. It is shown waiting, with its place
    /// in line, on every pass; the wait counts as no failure, and the circuit
    /// stays open until a start succeeds or fails.
    #[tokio::test]
    async fn a_pool_with_an_open_circuit_waiting_for_a_vm_slot_shows_waiting_and_why() -> Result<()>
    {
        let directory = std::env::temp_dir().join(format!(
            "gridops-slot-wait-circuit-{}",
            uuid::Uuid::new_v4()
        ));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        seed_macos_pool(&database, 3, true).await?;

        for _ in 0..3 {
            pass_deferred_with(&database, VM_SLOT_BUSY).await?;
            let (state, failures, circuit_open, retry_at, reason) = pool_row(&database).await?;
            assert_eq!(state, "waiting");
            assert!(
                reason.as_deref().is_some_and(
                    |reason| reason.starts_with("Waiting for a macOS VM slot: position 2 of 2")
                ),
                "{reason:?}"
            );
            assert_eq!(
                (failures, circuit_open),
                (3, true),
                "a wait is not a failure"
            );
            assert_eq!(retry_at, None, "the pool asks again on the next pass");
        }
        assert_eq!(
            waiting_events(&database).await?,
            1,
            "one event when the wait begins"
        );

        // The agent's own slot wait ran out: the pool pauses half a minute
        // and is still waiting, with its reason, through the pause.
        pass_deferred_with(&database, VM_SLOT_UNAVAILABLE).await?;
        pass_without_a_start(&database).await?;
        let (state, failures, circuit_open, retry_at, reason) = pool_row(&database).await?;
        assert_eq!(state, "waiting");
        assert!(reason.is_some_and(|reason| reason.starts_with("no macOS VM slot became free")));
        assert_eq!((failures, circuit_open), (3, true));
        assert!(retry_at.is_some());

        // A start that then fails ends the wait: the circuit cools down again.
        let pool = load_pools(&database).await?.remove(0);
        record_provision_failure(&database, &pool).await?;
        pass_without_a_start(&database).await?;
        let (state, failures, circuit_open, _, reason) = pool_row(&database).await?;
        assert_eq!(
            (state.as_str(), failures, circuit_open),
            ("blocked", 4, true)
        );
        assert_eq!(reason, None);

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    /// The same with earlier failures but no open circuit: through the pause
    /// after a deferred start it is waiting, not backing off.
    #[tokio::test]
    async fn a_pool_with_earlier_failures_waiting_for_a_vm_slot_is_waiting_not_backing_off()
    -> Result<()> {
        let directory = std::env::temp_dir().join(format!(
            "gridops-slot-wait-failures-{}",
            uuid::Uuid::new_v4()
        ));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        seed_macos_pool(&database, 1, false).await?;

        pass_deferred_with(&database, VM_SLOT_BUSY).await?;
        let (state, failures, _, _, reason) = pool_row(&database).await?;
        assert_eq!((state.as_str(), failures), ("waiting", 1));
        assert!(reason.is_some());

        pass_deferred_with(&database, VM_SLOT_UNAVAILABLE).await?;
        pass_without_a_start(&database).await?;
        let (state, failures, _, _, reason) = pool_row(&database).await?;
        assert_eq!((state.as_str(), failures), ("waiting", 1));
        assert!(reason.is_some_and(|reason| reason.starts_with("no macOS VM slot became free")));

        // A failure's own backoff is still shown as backoff.
        let pool = load_pools(&database).await?.remove(0);
        record_provision_failure(&database, &pool).await?;
        pass_without_a_start(&database).await?;
        let (state, failures, _, _, reason) = pool_row(&database).await?;
        assert_eq!((state.as_str(), failures), ("backoff", 2));
        assert_eq!(reason, None);

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn deferred_github_cleanup_uses_bounded_backoff() {
        assert_eq!(github_cleanup_backoff_ms(1), 30_000);
        assert_eq!(github_cleanup_backoff_ms(2), 60_000);
        assert_eq!(github_cleanup_backoff_ms(8), 3_600_000);
        assert_eq!(github_cleanup_backoff_ms(i64::MAX), 3_600_000);
    }

    #[test]
    fn provisioning_failures_back_off_before_opening_the_circuit() {
        assert_eq!(provision_backoff_ms(1), 30_000);
        assert_eq!(provision_backoff_ms(2), 60_000);
        assert_eq!(provision_backoff_ms(3), 120_000);
    }

    #[test]
    fn open_circuits_cool_down_instead_of_blocking_forever() {
        assert_eq!(provision_failure_policy(1), (false, 30_000));
        assert_eq!(provision_failure_policy(2), (false, 60_000));
        assert_eq!(
            provision_failure_policy(3),
            (true, PROVISION_CIRCUIT_COOLDOWN_MS)
        );
        assert_eq!(
            provision_failure_policy(9),
            (true, PROVISION_CIRCUIT_COOLDOWN_MS)
        );
    }

    #[test]
    fn recognizes_github_not_found_errors() {
        assert!(github_not_found(&anyhow::anyhow!(
            "GitHub API request failed (404 Not Found): {{\"message\":\"Not Found\"}}"
        )));
        assert!(!github_not_found(&anyhow::anyhow!(
            "GitHub API request failed (502 Bad Gateway): Server Error"
        )));
    }

    #[tokio::test]
    async fn stale_runs_are_revisited_until_github_reports_them_finished() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("gridops-reconciler-test-{}", uuid::Uuid::new_v4()));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo','User','User','all',1,1);
            INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
              VALUES
              (10,1,'octo','app','octo/app',0,'main','https://github.com/octo/app',1,1,1),
              (11,1,'octo','other','octo/other',0,'main','https://github.com/octo/other',1,1,1);
            INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,conclusion,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
              VALUES
              (20,10,'CI',1,'push','queued',NULL,'a','https://github.com/octo/app/actions/runs/20',1,1,1,30),
              (21,10,'CI',2,'push','completed','success','b','https://github.com/octo/app/actions/runs/21',1,1,1,10),
              (22,10,'CI',3,'push','completed','success','c','https://github.com/octo/app/actions/runs/22',1,1,1,20),
              (23,10,'CI',4,'push','in_progress',NULL,'d','https://github.com/octo/app/actions/runs/23',1,1,1,5),
              (24,11,'CI',1,'push','queued',NULL,'e','https://github.com/octo/other/actions/runs/24',1,1,1,1);
            INSERT INTO workflow_jobs (id,run_id,name,status,conclusion,labels,html_url,created_at,updated_at)
              VALUES
              (30,20,'build','queued',NULL,'[]','https://github.com/octo/app/actions/runs/20/job/30',1,1),
              (31,21,'build','queued',NULL,'[]','https://github.com/octo/app/actions/runs/21/job/31',1,1),
              (32,22,'build','completed','success','[]','https://github.com/octo/app/actions/runs/22/job/32',1,1),
              (33,23,'build','in_progress',NULL,'[]','https://github.com/octo/app/actions/runs/23/job/33',1,1);
            "#,
        )
        .execute(&database)
        .await?;

        // Run 22 is finished everywhere and run 24 belongs to another
        // repository; run 21 is completed but its job never caught up.
        assert_eq!(
            stale_run_ids(&database, 10, &HashSet::new()).await?,
            vec![23, 21, 20]
        );
        assert_eq!(
            stale_run_ids(&database, 10, &HashSet::from([23])).await?,
            vec![21, 20]
        );

        close_missing_run(&database, 20).await?;
        let (status, conclusion) = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status,conclusion FROM workflow_jobs WHERE id=30",
        )
        .fetch_one(&database)
        .await?;
        assert_eq!(
            (status.as_str(), conclusion.as_deref()),
            ("completed", Some("cancelled"))
        );
        assert_eq!(
            stale_run_ids(&database, 10, &HashSet::new()).await?,
            vec![23, 21]
        );

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn queued_jobs_are_assigned_to_one_label_compatible_pool() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("gridops-reconciler-test-{}", uuid::Uuid::new_v4()));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES
              (1,1,'octo-org','Organization','Organization','all',1,1),
              (2,2,'other-org','Organization','Organization','selected',1,1);
            INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
              VALUES
              (10,1,'octo-org','gridops','octo-org/gridops',0,'master','https://github.com/octo-org/gridops',1,1,1),
              (11,2,'other-org','other','other-org/other',0,'master','https://github.com/other-org/other',1,1,1);
            INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,labels,image,autoscaling_enabled,created_at,updated_at)
              VALUES
              ('pool-a',1,10,'pool-a','repository','["docker","pool-a"]','runner:latest',1,1,1),
              ('pool-b',1,10,'pool-b','repository','["gpu","pool-b"]','runner:latest',1,2,2),
              ('pool-org',1,NULL,'pool-org','organization','["org-only","pool-org"]','runner:latest',1,0,0);
            INSERT INTO runner_pool_repositories (pool_id,repository_id,created_at)
              VALUES ('pool-a',10,1),('pool-a',11,2),('pool-b',10,1);
            INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
              VALUES
              (20,10,'CI',1,'push','queued','abc123','https://github.com/octo-org/gridops/actions/runs/20',1,1,1,1),
              (21,11,'CI',2,'push','queued','def456','https://github.com/octo-org/other/actions/runs/21',1,1,1,1);
            INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
              VALUES
              (30,20,'generic','queued','["self-hosted","linux"]','https://github.com/octo-org/gridops/actions/runs/20/job/30',1,1),
              (31,20,'docker','queued','["self-hosted","docker"]','https://github.com/octo-org/gridops/actions/runs/20/job/31',1,1),
              (32,20,'gpu','queued','["self-hosted","pool-b"]','https://github.com/octo-org/gridops/actions/runs/20/job/32',1,1),
              (33,20,'org','queued','["self-hosted","org-only"]','https://github.com/octo-org/gridops/actions/runs/20/job/33',1,1),
              (34,20,'unmatched','queued','["self-hosted","unmatched"]','https://github.com/octo-org/gridops/actions/runs/20/job/34',1,1),
              (35,20,'wrong-os','queued','["self-hosted","windows"]','https://github.com/octo-org/gridops/actions/runs/20/job/35',1,1);
            INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
              VALUES (36,21,'other','queued','["self-hosted","pool-a"]','https://github.com/octo-org/other/actions/runs/21/job/36',1,1);
            INSERT INTO runners (id,pool_id,target_repository_id,name,status,created_at,updated_at)
              VALUES
              ('runner-a','pool-a',10,'pool-a-one','online',1,1),
              ('runner-b','pool-a',10,'pool-a-two','online',1,1);
            "#,
        )
        .execute(&database)
        .await?;

        let mut pool_a = pool(1);
        pool_a.id = "pool-a".into();
        let mut pool_b = pool(1);
        pool_b.id = "pool-b".into();
        let mut pool_org = pool(1);
        pool_org.id = "pool-org".into();

        assert_eq!(assigned_queued_jobs(&database, &pool_a.id).await?, 3);
        assert_eq!(assigned_queued_jobs(&database, &pool_b.id).await?, 1);
        assert_eq!(assigned_queued_jobs(&database, &pool_org.id).await?, 1);
        let target = next_runner_repository(&database, &pool_a.id, 1)
            .await?
            .context("multi-repository pool should have a target")?;
        assert_eq!(target.repository_id, 11);
        assert_eq!(target.installation_id, 2);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM runner_pool_installations WHERE pool_id='pool-a'",
            )
            .fetch_one(&database)
            .await?,
            2
        );

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn an_idle_pool_scales_for_jobs_a_full_pool_cannot_take() -> Result<()> {
        // Issue #42: the older pool is at its maximum of 4 with every runner
        // busy, and the newer pool, which carries the same label, sits idle.
        let directory =
            std::env::temp_dir().join(format!("gridops-reconciler-test-{}", uuid::Uuid::new_v4()));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
              VALUES (10,1,'octo-org','gridops','octo-org/gridops',0,'master','https://github.com/octo-org/gridops',1,1,1);
            INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,labels,image,desired_count,max_count,autoscaling_enabled,created_at,updated_at)
              VALUES
              ('older',1,NULL,'older','organization','["shared"]','runner:latest',4,4,1,1,1),
              ('newer',1,NULL,'newer','organization','["shared"]','runner:latest',0,10,1,2,2);
            INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
              VALUES (20,10,'CI',1,'push','queued','abc123','https://github.com/octo-org/gridops/actions/runs/20',1,1,1,1);
            INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
              VALUES (30,20,'waiting','queued','["self-hosted","shared"]','https://github.com/octo-org/gridops/actions/runs/20/job/30',1,1);
            INSERT INTO runners (id,pool_id,name,status,busy,created_at,updated_at)
              VALUES
              ('older-1','older','older-1','busy',1,1,1),
              ('older-2','older','older-2','busy',1,1,1),
              ('older-3','older','older-3','busy',1,1,1),
              ('older-4','older','older-4','busy',1,1,1);
            "#,
        )
        .execute(&database)
        .await?;

        assert_eq!(assigned_queued_jobs(&database, "older").await?, 0);
        let queued = assigned_queued_jobs(&database, "newer").await?;
        assert_eq!(queued, 1);
        // What maybe_scale_up asks of the newer pool: desired 0, nothing busy.
        assert_eq!(scale_up_target(0, 0, queued, 1, 10), 1);
        let providers = vec!["docker".to_owned()];
        let labels = vec!["shared".to_owned()];
        let capacity = provider_capacities(&database, "newer", &providers, &labels).await?;
        assert_eq!(
            capacity
                .iter()
                .map(|capacity| provider_capacity_deficit(capacity, 1))
                .sum::<i64>(),
            1
        );

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    /// A fake GitHub: answers each path-and-query in `routes` with its JSON
    /// and anything else with 404, and keeps every request it got, in order.
    async fn fake_github(
        routes: Vec<(String, Value)>,
    ) -> Result<(String, Arc<std::sync::Mutex<Vec<String>>>)> {
        let routes = Arc::new(routes.into_iter().collect::<HashMap<_, _>>());
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = Arc::clone(&seen);
        let app = axum::Router::new().fallback(move |uri: axum::http::Uri| {
            let routes = Arc::clone(&routes);
            let recorded = Arc::clone(&recorded);
            async move {
                let path = uri
                    .path_and_query()
                    .map_or_else(|| uri.path().to_owned(), ToString::to_string);
                recorded
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(path.clone());
                match routes.get(&path) {
                    Some(body) => (axum::http::StatusCode::OK, axum::Json(body.clone())),
                    None => (
                        axum::http::StatusCode::NOT_FOUND,
                        axum::Json(json!({ "message": "Not Found" })),
                    ),
                }
            }
        });
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let url = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok((url, seen))
    }

    /// A reconciler whose GitHub is the fake at `github_url`.
    fn reconciler_with_github(database: SqlitePool, github_url: &str) -> Result<Reconciler> {
        let config = Config::from_env()?.with_github_api_url(reqwest::Url::parse(github_url)?);
        Ok(Reconciler {
            github: GitHubClient::new(config.clone())?,
            config,
            database,
            bitbucket: BitbucketClient::new()?,
            vault: Vault::new([0; 32], b"reconciler-test".to_vec()),
            http: reqwest::Client::new(),
            pass: Arc::default(),
        })
    }

    fn github_job(
        id: i64,
        run_id: i64,
        attempt: i64,
        status: &str,
        conclusion: Option<&str>,
    ) -> Value {
        json!({
            "id": id, "run_id": run_id, "run_attempt": attempt, "name": format!("build-{id}"),
            "status": status, "conclusion": conclusion, "labels": ["self-hosted", "linux"],
            "html_url": format!("https://github.com/octo/app/actions/runs/{run_id}/job/{id}"),
            "started_at": "2026-11-01T14:00:00Z",
            "completed_at": conclusion.map(|_| "2026-11-01T14:08:00Z"),
        })
    }

    const REPAIR_BASE: &str = r#"
        INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
          VALUES (1,1,'octo','Organization','Organization','all',1,1);
        INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
          VALUES (10,1,'octo','app','octo/app',0,'main','https://github.com/octo/app',1,1,1);
    "#;

    async fn job_state(
        database: &SqlitePool,
        id: i64,
    ) -> Result<(String, Option<String>, Option<i64>, Option<i64>)> {
        Ok(sqlx::query_as(
            "SELECT job.status,job.conclusion,sync.run_attempt,sync.confirmed_at
               FROM workflow_jobs job LEFT JOIN workflow_job_sync sync ON sync.job_id=job.id
              WHERE job.id=?",
        )
        .bind(id)
        .fetch_one(database)
        .await?)
    }

    /// #33 through the poll: GitHub lists only the newest attempt's jobs
    /// unless asked for all of them, so an attempt-1 job whose completion was
    /// missed (here a row from before attempts were stored) stayed queued.
    #[tokio::test]
    async fn a_poll_after_a_rerun_records_how_the_earlier_attempt_ended() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("gridops-rerun-poll-test-{}", uuid::Uuid::new_v4()));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            r#"{REPAIR_BASE}
            INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,run_attempt,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
              VALUES (41,10,'CI',7,1,'push','in_progress','abc','https://github.com/octo/app/actions/runs/41',1,1,1,1);
            INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
              VALUES (401,41,'build-401','queued','["self-hosted","linux"]','https://github.com/octo/app/actions/runs/41/job/401',1,1);
            "#
        )))
        .execute(&database)
        .await?;
        let run = json!({
            "id": 41, "workflow_id": 3, "name": "CI", "run_number": 7, "run_attempt": 2,
            "event": "push", "status": "in_progress", "conclusion": null, "head_branch": "main",
            "head_sha": "abc", "html_url": "https://github.com/octo/app/actions/runs/41",
            "run_started_at": "2026-11-01T15:00:00Z", "created_at": "2026-11-01T14:00:00Z",
            "updated_at": "2026-11-01T15:00:00Z",
        });
        let (url, _) = fake_github(vec![
            (
                "/repos/octo/app/actions/runs?per_page=50".into(),
                json!({ "workflow_runs": [run] }),
            ),
            (
                "/repos/octo/app/actions/runs/41/jobs?filter=latest&per_page=100&page=1".into(),
                json!({ "jobs": [github_job(402, 41, 2, "queued", None)] }),
            ),
            (
                "/repos/octo/app/actions/runs/41/jobs?filter=all&per_page=100&page=1".into(),
                json!({ "jobs": [
                    github_job(401, 41, 1, "completed", Some("failure")),
                    github_job(402, 41, 2, "queued", None),
                ] }),
            ),
        ])
        .await?;
        let app = reconciler_with_github(database.clone(), &url)?;
        let repository = Repository {
            id: 10,
            installation_id: 1,
            owner: "octo".into(),
            name: "app".into(),
            full_name: "octo/app".into(),
        };

        sync_repository_workflows(&app, &repository, "installation-token").await?;

        let (status, conclusion, attempt, confirmed) = job_state(&database, 401).await?;
        assert_eq!(
            (status.as_str(), conclusion.as_deref(), attempt),
            ("completed", Some("failure"), Some(1))
        );
        assert!(confirmed.is_some());
        let (status, _, attempt, _) = job_state(&database, 402).await?;
        assert_eq!((status.as_str(), attempt), ("queued", Some(2)));
        // The only job waiting is the live attempt's.
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT id FROM workflow_jobs WHERE status='queued' ORDER BY id"
            )
            .fetch_all(&database)
            .await?,
            [402]
        );

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    /// #33 and #34: jobs open for longer than the check age without word from
    /// GitHub are asked about one by one, the oldest queued job of an
    /// unfinished run first, and GitHub's answer is recorded.
    #[tokio::test]
    async fn the_job_check_asks_github_about_the_oldest_unconfirmed_jobs_first() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("gridops-job-check-test-{}", uuid::Uuid::new_v4()));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        let now = now_millis();
        let ago = |minutes: i64| now - minutes * 60_000;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            r#"{REPAIR_BASE}
            INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,run_attempt,event,status,conclusion,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
              VALUES
              (50,10,'CI',1,1,'push','in_progress',NULL,'a','https://github.com/octo/app/actions/runs/50',1,1,1,1),
              (51,10,'CI',2,1,'push','completed','success','b','https://github.com/octo/app/actions/runs/51',1,1,1,1);
            INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
              VALUES
              (501,50,'phantom','queued','[]','u',{a501},{a501}),
              (502,50,'waiting','queued','[]','u',{a502},{a502}),
              (503,50,'running','in_progress','[]','u',{a503},{a503}),
              (504,50,'just heard of','queued','[]','u',{a504},{a504}),
              (505,50,'just asked','queued','[]','u',{a505},{a505}),
              (506,51,'orphan','queued','[]','u',{a506},{a506}),
              (507,50,'new','queued','[]','u',{a507},{a507});
            -- 501 was recorded before GridOps kept when GitHub reported a job.
            INSERT INTO workflow_job_sync (job_id,confirmed_at,checked_at)
              VALUES
              (502,{a502},NULL),
              (503,{c503},NULL),
              (504,{c504},NULL),
              (505,{a505},{k505}),
              (506,{a506},NULL),
              (507,{a507},NULL);
            "#,
            a501 = ago(180),
            a502 = ago(120),
            a503 = ago(240),
            c503 = ago(60),
            a504 = ago(60),
            c504 = ago(1),
            a505 = ago(10),
            k505 = ago(1),
            a506 = ago(300),
            a507 = ago(2),
        )))
        .execute(&database)
        .await?;
        let (url, seen) = fake_github(vec![
            (
                "/repos/octo/app/actions/jobs/501".into(),
                github_job(501, 50, 1, "completed", Some("failure")),
            ),
            (
                "/repos/octo/app/actions/jobs/502".into(),
                github_job(502, 50, 1, "queued", None),
            ),
            (
                "/repos/octo/app/actions/jobs/506".into(),
                github_job(506, 51, 1, "completed", Some("cancelled")),
            ),
        ])
        .await?;
        let app = reconciler_with_github(database.clone(), &url)?;

        // The cap holds: two at a time, the oldest queued job first.
        assert_eq!(
            unconfirmed_jobs(&database, now, 2)
                .await?
                .iter()
                .map(|job| job.id)
                .collect::<Vec<_>>(),
            [501, 502]
        );

        let mut tokens = HashMap::from([(1, Some("installation-token".to_owned()))]);
        confirm_jobs(&app, &mut tokens).await?;

        assert_eq!(
            *seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            [
                "/repos/octo/app/actions/jobs/501",
                "/repos/octo/app/actions/jobs/502",
                "/repos/octo/app/actions/jobs/503",
                "/repos/octo/app/actions/jobs/506",
            ]
        );
        // GitHub finished the phantom long ago: its real outcome is recorded.
        let (status, conclusion, attempt, confirmed) = job_state(&database, 501).await?;
        assert_eq!(
            (status.as_str(), conclusion.as_deref(), attempt),
            ("completed", Some("failure"), Some(1))
        );
        assert!(confirmed.is_some_and(|at| at >= now));
        // A job GitHub still has queued stays queued, confirmed just now.
        let (status, _, _, confirmed) = job_state(&database, 502).await?;
        assert_eq!(status, "queued");
        assert!(confirmed.is_some_and(|at| at >= now));
        // A job GitHub no longer has can never run.
        let (status, conclusion, _, _) = job_state(&database, 503).await?;
        assert_eq!(
            (status.as_str(), conclusion.as_deref()),
            ("completed", Some("cancelled"))
        );
        let (status, conclusion, _, _) = job_state(&database, 506).await?;
        assert_eq!(
            (status.as_str(), conclusion.as_deref()),
            ("completed", Some("cancelled"))
        );
        // Jobs GitHub reported on, or was asked about, recently, and new
        // jobs, were not asked about.
        for id in [504, 505, 507] {
            assert_eq!(job_state(&database, id).await?.0, "queued");
        }

        // Nothing is asked again before the check age has passed.
        seen.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        confirm_jobs(&app, &mut tokens).await?;
        assert!(
            seen.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        );

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    /// Follow-up to #33: an earlier attempt's job finished here when a re-run
    /// appeared has no conclusion until the poll records it, and keeps none
    /// for good if the run is not polled again. The job check asks GitHub
    /// about it after the open jobs, records the real conclusion, and stops
    /// asking once the job has one.
    #[tokio::test]
    async fn the_job_check_records_the_conclusion_of_a_job_closed_by_a_rerun() -> Result<()> {
        let directory = std::env::temp_dir().join(format!(
            "gridops-job-conclusion-test-{}",
            uuid::Uuid::new_v4()
        ));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        let now = now_millis();
        let ago = |minutes: i64| now - minutes * 60_000;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            r#"{REPAIR_BASE}
            INSERT INTO runner_pools (id,installation_id,name,scope,image,created_at,updated_at)
              VALUES ('pool-1',1,'linux','organization','runner:latest',1,1);
            INSERT INTO runners (id,pool_id,target_repository_id,name,provider,ci_platform,os,architecture,status,busy,current_job_id,ephemeral,configuration_version,runtime,created_at,updated_at)
              VALUES ('63000000-0000-4000-8000-000000000001','pool-1',10,'linux-1','docker','github','linux','X64','busy',1,703,0,1,'vm',1,1);
            INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,run_attempt,event,status,conclusion,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
              VALUES (70,10,'CI',1,2,'push','in_progress',NULL,'a','https://github.com/octo/app/actions/runs/70',1,1,1,1);
            INSERT INTO workflow_jobs (id,run_id,name,status,conclusion,runner_name,labels,html_url,completed_at,created_at,updated_at)
              VALUES
              (701,70,'closed by the re-run','completed',NULL,'linux-1','[]','u',{c701},{a701},{c701}),
              (702,70,'queued','queued',NULL,NULL,'[]','u',NULL,{a702},{a702}),
              (703,70,'running','in_progress',NULL,'linux-1','[]','u',NULL,{a703},{a703}),
              (704,70,'concluded','completed','success',NULL,'[]','u',{c701},{a701},{c701}),
              (705,70,'closed long ago','completed',NULL,NULL,'[]','u',{c705},{a705},{c705}),
              (706,70,'just asked','completed',NULL,NULL,'[]','u',{c701},{a701},{c701}),
              (707,70,'deleted','completed',NULL,NULL,'[]','u',{c701},{a707},{c701}),
              (708,70,'just heard of','completed',NULL,NULL,'[]','u',{c701},{a701},{c701});
            INSERT INTO workflow_job_sync (job_id,run_attempt,confirmed_at,checked_at)
              VALUES
              (701,1,{k701},NULL),
              (702,2,{a702},NULL),
              (703,2,{k703},NULL),
              (704,1,{k701},NULL),
              (705,1,{a705},NULL),
              (706,1,{k701},{k706}),
              (707,1,{k701},NULL),
              (708,1,{k708},NULL);
            "#,
            a701 = ago(60),
            c701 = ago(30),
            k701 = ago(40),
            a702 = ago(20),
            a703 = ago(25),
            k703 = ago(10),
            a705 = ago(8 * 24 * 60 + 60),
            c705 = ago(8 * 24 * 60),
            k706 = ago(1),
            a707 = ago(50),
            k708 = ago(1),
        )))
        .execute(&database)
        .await?;
        let mut concluded = github_job(701, 70, 1, "completed", Some("failure"));
        concluded["runner_name"] = json!("linux-1");
        let mut running = github_job(703, 70, 2, "in_progress", None);
        running["runner_name"] = json!("linux-1");
        let (url, seen) = fake_github(vec![
            ("/repos/octo/app/actions/jobs/701".into(), concluded),
            (
                "/repos/octo/app/actions/jobs/702".into(),
                github_job(702, 70, 2, "queued", None),
            ),
            ("/repos/octo/app/actions/jobs/703".into(), running),
        ])
        .await?;
        let app = reconciler_with_github(database.clone(), &url)?;

        // Open jobs come first and share the cap: with room for two, the
        // older closed jobs wait; with room for three, the oldest one fits.
        let ids = |jobs: Vec<UnconfirmedJob>| jobs.iter().map(|job| job.id).collect::<Vec<_>>();
        assert_eq!(ids(unconfirmed_jobs(&database, now, 2).await?), [702, 703]);
        assert_eq!(
            ids(unconfirmed_jobs(&database, now, 3).await?),
            [702, 703, 701]
        );
        assert_eq!(
            ids(unconfirmed_jobs(&database, now, 10).await?),
            [702, 703, 701, 707]
        );

        let mut tokens = HashMap::from([(1, Some("installation-token".to_owned()))]);
        confirm_jobs(&app, &mut tokens).await?;

        assert_eq!(
            *seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            [
                "/repos/octo/app/actions/jobs/702",
                "/repos/octo/app/actions/jobs/703",
                "/repos/octo/app/actions/jobs/701",
                "/repos/octo/app/actions/jobs/707",
            ]
        );
        // GitHub's conclusion is recorded.
        let (status, conclusion, attempt, confirmed) = job_state(&database, 701).await?;
        assert_eq!(
            (status.as_str(), conclusion.as_deref(), attempt),
            ("completed", Some("failure"), Some(1))
        );
        assert!(confirmed.is_some_and(|at| at >= now));
        // The runner that ran the earlier attempt's job is busy with the new
        // attempt's job; hearing how the old one ended does not free it.
        assert_eq!(
            sqlx::query_as::<_, (i64, Option<i64>)>(
                "SELECT busy,current_job_id FROM runners WHERE name='linux-1'"
            )
            .fetch_one(&database)
            .await?,
            (1, Some(703))
        );
        // A job GitHub no longer has gets the conclusion a deleted job gets.
        let (status, conclusion, _, _) = job_state(&database, 707).await?;
        assert_eq!(
            (status.as_str(), conclusion.as_deref()),
            ("completed", Some("cancelled"))
        );
        // A job with a conclusion, one closed before the window, and ones
        // asked about or reported on by GitHub within the check age were not
        // asked about.
        assert_eq!(
            job_state(&database, 704).await?.1.as_deref(),
            Some("success")
        );
        for id in [705, 706, 708] {
            assert_eq!(job_state(&database, id).await?.1, None);
        }

        // Once a job has a conclusion it is not asked about again, even after
        // the check age; the open jobs still are.
        sqlx::query("UPDATE workflow_job_sync SET confirmed_at=?,checked_at=? WHERE job_id IN (701,702,703,707)")
            .bind(ago(10))
            .bind(ago(10))
            .execute(&database)
            .await?;
        seen.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        confirm_jobs(&app, &mut tokens).await?;
        assert_eq!(
            *seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            [
                "/repos/octo/app/actions/jobs/702",
                "/repos/octo/app/actions/jobs/703",
            ]
        );

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    /// A phantom queued job kept an idle fleet runner from being released for
    /// queued fleet work, since its repository seemed to have work for its
    /// OS. Once the newer attempt finishes the earlier attempt's job, the
    /// runner is the one released.
    #[tokio::test]
    async fn an_idle_fleet_runner_is_released_once_a_stale_earlier_attempt_is_finished()
    -> Result<()> {
        let directory = std::env::temp_dir().join(format!(
            "gridops-rerun-eviction-test-{}",
            uuid::Uuid::new_v4()
        ));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO runner_pools (id,installation_id,name,scope,image,created_at,updated_at)
              VALUES ('pool-1',1,'linux','organization','runner:latest',1,1);
            INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
              VALUES (7,1,'octo-org','app','octo-org/app',0,'main','https://github.com/octo-org/app',1,1,1);
            INSERT INTO pool_execution_profiles (id,pool_id,name,runtime_kind,execution_os,architecture,mode,image,cpu_millis,memory_mib,disk_bytes,created_at,updated_at)
              VALUES ('40000000-0000-4000-8000-000000000001','pool-1','linux','docker','linux','x64','ephemeral','runner:latest',2000,2048,0,1,1);
            INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,repository_id)
              VALUES ('30000000-0000-4000-8000-000000000001','github_repository','github:repository:1:7',1,7);
            INSERT INTO profile_ci_targets (profile_id,target_id,enabled)
              VALUES ('40000000-0000-4000-8000-000000000001','30000000-0000-4000-8000-000000000001',1);
            INSERT INTO runners (id,pool_id,target_repository_id,name,provider,ci_platform,os,architecture,status,busy,ephemeral,configuration_version,runtime,created_at,updated_at)
              VALUES ('62000000-0000-4000-8000-000000000001','pool-1',7,'linux-1','docker','github','linux','X64','online',0,1,1,'vm',1,1);
            INSERT INTO fleet_operations (id,principal_kind,principal_scope,kind,idempotency_key,request_hash,status,created_at,updated_at)
              VALUES
              ('62000000-0000-4000-8000-000000000001','reconciler','controller:reconciler','submit_workload','runner:62000000-0000-4000-8000-000000000001','hash','succeeded',1,1),
              ('62000000-0000-4000-8000-000000000002','reconciler','controller:reconciler','submit_workload','runner:62000000-0000-4000-8000-000000000002','hash','queued',1,1);
            INSERT INTO workload_intents (id,workload_id,generation,kind,pool_id,profile_id,target_id,agent_run_id,expected_profile_revision,status,created_at,updated_at)
              VALUES
              ('62000000-0000-4000-8000-000000000001','62000000-0000-4000-8000-000000000001',1,'ci_runner','pool-1','40000000-0000-4000-8000-000000000001','30000000-0000-4000-8000-000000000001',NULL,1,'admitted',1,1),
              ('62000000-0000-4000-8000-000000000002','62000000-0000-4000-8000-000000000002',1,'ci_runner','pool-1','40000000-0000-4000-8000-000000000001','30000000-0000-4000-8000-000000000001',NULL,1,'queued',1,1);
            INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,run_attempt,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
              VALUES (60,7,'CI',3,1,'push','in_progress','abc','https://github.com/octo-org/app/actions/runs/60',1,1,1,1);
            INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
              VALUES (601,60,'build','queued','["self-hosted","linux"]','https://github.com/octo-org/app/actions/runs/60/job/601',1,1);
            INSERT INTO workflow_job_sync (job_id,run_attempt,confirmed_at) VALUES (601,1,1);
            "#,
        )
        .execute(&database)
        .await?;
        let now = now_millis();
        // The attempt-1 job, still listed as queued, holds the runner.
        assert_eq!(
            idle_fleet_runner_to_evict(&database, "pool-1", now).await?,
            None
        );

        // The poll sees attempt 2 of the run.
        let mut transaction = database.begin().await?;
        upsert_workflow_run(
            &mut transaction,
            7,
            GitHubWorkflowRun {
                id: 60,
                workflow_id: Some(3),
                name: Some("CI".into()),
                display_title: None,
                run_number: 3,
                run_attempt: 2,
                event: "push".into(),
                status: "in_progress".into(),
                conclusion: None,
                head_branch: Some("main".into()),
                head_sha: "abc".into(),
                actor: None,
                html_url: "https://github.com/octo-org/app/actions/runs/60".into(),
                run_started_at: Some("2026-11-01T15:00:00Z".into()),
                created_at: "2026-11-01T14:00:00Z".into(),
                updated_at: "2026-11-01T15:00:00Z".into(),
            },
            now,
        )
        .await?;
        transaction.commit().await?;

        assert_eq!(job_state(&database, 601).await?.0, "completed");
        assert_eq!(
            idle_fleet_runner_to_evict(&database, "pool-1", now)
                .await?
                .as_deref(),
            Some("62000000-0000-4000-8000-000000000001")
        );

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}
