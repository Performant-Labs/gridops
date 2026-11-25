//! #204 item 6: the fleet health timings on a copy of a production database.
//! Ignored: it runs only by hand, with the copy's path in
//! `GRIDOPS_FLEET_HEALTH_DB_COPY` (a copy: the run writes the rollups, the
//! categories and the snapshots into it, as the first deploy would):
//!
//! ```text
//! GRIDOPS_FLEET_HEALTH_DB_COPY=/path/to/copy.sqlite \
//!   cargo test -p gridops-api --bin gridops-api db_copy_timings -- --ignored --nocapture
//! ```
//!
//! It times, as of the copy's newest sample plus 30 s:
//! - `host_live_reads` on its own (#234, #240);
//! - the first full catch-up: the reconciler's fleet health pass
//!   (`gridops_reconciler::fleet_health_pass`) until the backfill and both
//!   rollups have nothing left, while another task reads the page's live and
//!   24 h halves every second, as the page would during a deploy;
//! - `GET /api/v1/fleet/health` through the real router and handler, cold
//!   (a new health state: no response or speed cache) and cached (within the
//!   30 s), for each window.
//!
//! Every write transaction is timed from its first row change to its commit
//! (wall clock, on a quiet machine), sqlx's `slow statement` warnings (over
//! 100 ms) are counted, and so are `SQLITE_BUSY` errors, per phase. The
//! report is printed.

use std::{
    path::PathBuf,
    str::FromStr as _,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use gridops_core::{
    BitbucketClient, Config, GitHubClient, Vault,
    fleet::registry::{RegistryClock, host_live_reads},
    fleet_health::{FleetHealthSource, HealthWindow},
};
use sqlx::{
    ConnectOptions as _, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use tower::ServiceExt as _;
use tracing_subscriber::{Layer, layer::SubscriberExt as _, util::SubscriberInitExt as _};
use url::Url;

use super::health_source::DbFleetHealthSource;
use crate::state::AppState;

/// What the hooks and the log layer saw in one phase.
#[derive(Default)]
struct Observed {
    /// The longest write transaction, in microseconds.
    worst_write_us: AtomicU64,
    /// Write transactions over 50 ms.
    writes_over_50ms: AtomicU64,
    write_transactions: AtomicU64,
    slow_statements: AtomicU64,
}

impl Observed {
    fn reset(&self) {
        for counter in [
            &self.worst_write_us,
            &self.writes_over_50ms,
            &self.write_transactions,
            &self.slow_statements,
        ] {
            counter.store(0, Ordering::SeqCst);
        }
    }

    fn line(&self) -> String {
        format!(
            "write transactions {}, worst {:.1} ms, over 50 ms {}, slow statements {}",
            self.write_transactions.load(Ordering::SeqCst),
            self.worst_write_us.load(Ordering::SeqCst) as f64 / 1000.0,
            self.writes_over_50ms.load(Ordering::SeqCst),
            self.slow_statements.load(Ordering::SeqCst),
        )
    }
}

/// Counts sqlx's slow-statement warnings.
struct SlowStatements(Arc<Observed>);

impl<S: tracing::Subscriber> Layer<S> for SlowStatements {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let metadata = event.metadata();
        if *metadata.level() == tracing::Level::WARN && metadata.target().starts_with("sqlx") {
            self.0.slow_statements.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// The copy, opened as `connect_database_path` opens a database (WAL, 5 s
/// busy timeout, slow statements over 100 ms), with every write transaction
/// timed; no migration runs.
async fn open(path: &PathBuf, observed: &Arc<Observed>) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .log_slow_statements(log::LevelFilter::Warn, Duration::from_millis(100));
    let observed = Arc::clone(observed);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .after_connect(move |conn, _| {
            let observed = Arc::clone(&observed);
            Box::pin(async move {
                let first: Arc<Mutex<Option<Instant>>> = Arc::default();
                let mut handle = conn.lock_handle().await?;
                let wrote = Arc::clone(&first);
                handle.set_update_hook(move |_| {
                    let mut first = wrote
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    first.get_or_insert_with(Instant::now);
                });
                let ended = move |first: &Mutex<Option<Instant>>, observed: &Observed| {
                    let started = first
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    if let Some(started) = started {
                        let held = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
                        observed.write_transactions.fetch_add(1, Ordering::SeqCst);
                        observed.worst_write_us.fetch_max(held, Ordering::SeqCst);
                        if held > 50_000 {
                            observed.writes_over_50ms.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                };
                let (committed, commit_observed) = (Arc::clone(&first), Arc::clone(&observed));
                handle.set_commit_hook(move || {
                    ended(&committed, &commit_observed);
                    true
                });
                handle.set_rollback_hook(move || ended(&first, &observed));
                Ok(())
            })
        })
        .connect_with(options)
        .await?;
    Ok(pool)
}

/// Whether `error` is SQLite's `SQLITE_BUSY`, in any extended form.
fn busy(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<sqlx::Error>()
            .and_then(sqlx::Error::as_database_error)
            .and_then(|error| error.code())
            .and_then(|code| code.parse::<i32>().ok())
            .is_some_and(|code| code & 0xff == 5)
    })
}

/// p50, p95 and the worst of `samples` in milliseconds.
fn percentiles(mut samples: Vec<Duration>) -> String {
    samples.sort();
    let at = |quantile: f64| {
        let index = ((samples.len() as f64 * quantile).ceil() as usize).clamp(1, samples.len()) - 1;
        samples[index].as_secs_f64() * 1000.0
    };
    format!(
        "n {}, p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        samples.len(),
        at(0.5),
        at(0.95),
        at(1.0)
    )
}

#[derive(Debug)]
struct FixedClock(AtomicI64);

impl RegistryClock for FixedClock {
    fn now_millis(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// An administrator's session in the copy, for the router.
async fn admin_cookie(db: &SqlitePool, vault: &Vault, now: i64) -> Result<String> {
    let session = "fleet-health-timings-session";
    sqlx::query("INSERT OR IGNORE INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('fleet-health-timings',-204,'fleet-health-timings','admin','',?,?,?)")
        .bind(now).bind(now).bind(now).execute(db).await?;
    sqlx::query("INSERT OR REPLACE INTO sessions (id,token_hash,user_id,expires_at,last_seen_at,created_at) VALUES ('fleet-health-timings',?,'fleet-health-timings',?,?,?)")
        .bind(gridops_core::crypto::hash_token(session))
        .bind(now + 3_600_000)
        .bind(now)
        .bind(now)
        .execute(db)
        .await?;
    Ok(format!(
        "gridops_session={session}.{}",
        vault.sign(session)?
    ))
}

/// The router over `db` with a new health state (empty caches) at `now`,
/// warmed as `main` warms it when `warm`, and the clock it reads.
fn router(
    db: &SqlitePool,
    vault: &Vault,
    now: i64,
    warm: bool,
) -> Result<(axum::Router, Arc<FixedClock>)> {
    let config = Config::from_env()?
        .with_base_url(Url::parse("https://gridops.example")?)
        .with_manager(
            Url::parse("http://127.0.0.1:9")?,
            "timings-manager-credential",
        );
    let clock = Arc::new(FixedClock(AtomicI64::new(now)));
    let state = AppState::new(
        config.clone(),
        db.clone(),
        vault.clone(),
        GitHubClient::new(config)?,
        BitbucketClient::new()?,
    )?
    .with_fleet_health_clock(clock.clone());
    if warm {
        state.fleet_health.warm();
    }
    Ok((
        super::router()
            .merge(crate::session_routes())
            .with_state(state),
        clock,
    ))
}

async fn get(router: &axum::Router, window: HealthWindow, cookie: &str) -> Result<Duration> {
    let request = Request::builder()
        .uri(format!("/api/v1/fleet/health?window={window}"))
        .header(header::ORIGIN, "https://gridops.example")
        .header(header::COOKIE, cookie)
        .body(Body::empty())?;
    let started = Instant::now();
    let response = router.clone().oneshot(request).await?;
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024).await?;
    let elapsed = started.elapsed();
    anyhow::ensure!(
        status == StatusCode::OK,
        "{window}: {status} {}",
        String::from_utf8_lossy(&body)
    );
    Ok(elapsed)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "by hand, on a copy of a production database (GRIDOPS_FLEET_HEALTH_DB_COPY)"]
async fn db_copy_timings() -> Result<()> {
    let path = PathBuf::from(
        std::env::var_os("GRIDOPS_FLEET_HEALTH_DB_COPY")
            .context("GRIDOPS_FLEET_HEALTH_DB_COPY names the copy")?,
    );
    let observed = Arc::new(Observed::default());
    tracing_subscriber::registry()
        .with(
            SlowStatements(Arc::clone(&observed))
                .with_filter(tracing_subscriber::filter::LevelFilter::WARN),
        )
        .try_init()
        .ok();
    let db = open(&path, &observed).await?;
    let newest: i64 = sqlx::query_scalar("SELECT MAX(observed_at) FROM host_samples")
        .fetch_one(&db)
        .await?;
    let now = newest + 30_000;
    println!("copy {}, now {now}", path.display());

    // host_live_reads on its own.
    observed.reset();
    let mut samples = Vec::new();
    for _ in 0..30 {
        let started = Instant::now();
        host_live_reads(&db, now).await?;
        samples.push(started.elapsed());
    }
    println!(
        "host_live_reads: {}; {}",
        percentiles(samples),
        observed.line()
    );

    // The first full catch-up, with the page read alongside.
    observed.reset();
    let busy_errors = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let reader = {
        let (db, busy_errors, done) = (db.clone(), Arc::clone(&busy_errors), Arc::clone(&done));
        tokio::spawn(async move {
            let source = DbFleetHealthSource::new(db);
            let mut reads = Vec::new();
            while !done.load(Ordering::SeqCst) {
                let started = Instant::now();
                let result = async {
                    source.live(now).await?;
                    source.windowed(HealthWindow::Day, now).await
                }
                .await;
                match result {
                    Ok(_) => reads.push(started.elapsed()),
                    Err(error) if busy(&error) => {
                        busy_errors.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(error) => return Err(error),
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Ok::<_, anyhow::Error>(reads)
        })
    };
    let started = Instant::now();
    let mut passes = Vec::new();
    let mut pass_busy = 0_u64;
    loop {
        let pass_started = Instant::now();
        match gridops_reconciler::fleet_health_pass(&db, now).await {
            Ok(()) => {}
            Err(error) if busy(&error) => pass_busy += 1,
            Err(error) => return Err(error),
        }
        passes.push(pass_started.elapsed());
        let unclassified: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM workflow_jobs WHERE category IS NULL)",
        )
        .fetch_one(&db)
        .await?;
        let jobs_next: Option<String> =
            sqlx::query_scalar("SELECT value FROM settings WHERE key='fleet_health.rollup_next'")
                .fetch_optional(&db)
                .await?;
        let caught_up = jobs_next
            .and_then(|next| next.parse::<i64>().ok())
            .is_some_and(|next| next + 3_600_000 + 15 * 60_000 > now);
        if (unclassified == 0 && caught_up) || passes.len() >= 400 {
            break;
        }
    }
    let catch_up = started.elapsed();
    done.store(true, Ordering::SeqCst);
    let reads = reader.await??;
    println!(
        "catch-up: {} passes in {:.1} s; passes {}; {}; SQLITE_BUSY in passes {pass_busy}, in reads {}",
        passes.len(),
        catch_up.as_secs_f64(),
        percentiles(passes),
        observed.line(),
        busy_errors.load(Ordering::SeqCst)
    );
    if !reads.is_empty() {
        println!(
            "  page reads during the catch-up (live + 24 h windowed): {}",
            percentiles(reads)
        );
    }
    let rows: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM fleet_health_hourly),(SELECT COUNT(*) FROM fleet_host_hourly),
                (SELECT COUNT(*) FROM fleet_health_snapshots)",
    )
    .fetch_one(&db)
    .await?;
    println!(
        "  rows: fleet_health_hourly {}, fleet_host_hourly {}, snapshots {}",
        rows.0, rows.1, rows.2
    );

    // The endpoint: cold (a new health state) and cached.
    let vault = Vault::new([7_u8; 32], b"fleet-health-timings-secret".to_vec());
    let cookie = admin_cookie(&db, &vault, now).await?;
    for window in HealthWindow::ALL {
        // A new health state, no cache at all: the API's first page before
        // its warm-up has read the speed indexes.
        observed.reset();
        let mut cold = Vec::new();
        for _ in 0..20 {
            let (router, _) = router(&db, &vault, now, false)?;
            cold.push(get(&router, window, &cookie).await?);
        }
        let cold_line = observed.line();
        // A new health state once its warm-up has read the speed indexes:
        // a response-cache miss on a running API.
        observed.reset();
        let mut warm = Vec::new();
        for _ in 0..20 {
            let (router, _) = router(&db, &vault, now, true)?;
            tokio::time::sleep(Duration::from_millis(1_500)).await;
            warm.push(get(&router, window, &cookie).await?);
        }
        let warm_line = observed.line();
        // A page left open: a miss every 31 s for 10 minutes, the speed
        // indexes refreshed behind the calls.
        observed.reset();
        let (refreshing, clock) = router(&db, &vault, now, true)?;
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        let mut misses = Vec::new();
        for _ in 0..20 {
            clock.0.fetch_add(31_000, Ordering::SeqCst);
            misses.push(get(&refreshing, window, &cookie).await?);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let miss_line = observed.line();
        // Within the 30 s: the cached response.
        observed.reset();
        let mut cached = Vec::new();
        for _ in 0..200 {
            cached.push(get(&refreshing, window, &cookie).await?);
        }
        let cached_line = observed.line();
        let mut speed = Vec::new();
        for _ in 0..10 {
            let started = Instant::now();
            gridops_core::fleet_health::speed::speed_indexes(&db, window, now).await?;
            speed.push(started.elapsed());
        }
        println!(
            "GET {window}:\n  cold, no cache at all     {}; {cold_line}\n  cold, speed warmed        {}; {warm_line}\n  page refresh every 31 s   {}; {miss_line}\n  cached                    {}; {cached_line}\n  speed index alone         {}",
            percentiles(cold),
            percentiles(warm),
            percentiles(misses),
            percentiles(cached),
            percentiles(speed),
        );
    }
    db.close().await;
    Ok(())
}
