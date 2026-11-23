//! The database speed check (#197 section 7): [`check`] runs some work and
//! fails when any statement on the pool takes over 100 ms (the #179
//! slow-statement log, `db.rs`) or any write transaction holds the write lock
//! over [`WRITE_LOCK_LIMIT`]. Wave 1's rollup, backfill and host rollup
//! contract tests run their passes under it.
//!
//! How it sees the database, test builds only (`test-support`):
//! - `connect_database_path` instruments every connection it opens
//!   ([`instrument`]): SQLite's progress handler, update hook, commit hook and
//!   rollback hook, which run on the connection's own worker thread;
//! - a write transaction is timed from its first row change (update hook) to
//!   its commit or rollback;
//! - while a check is running on its database, a worker thread routes its
//!   `tracing` events to this module's dispatcher (set as that thread's
//!   default from a hook), which records sqlx's slow-statement warning. Other
//!   threads, and the process's global subscriber, are left alone.
//!
//! Limits, by design: a write transaction is timed from its first row
//! change, so a `BEGIN IMMEDIATE` held before any write, or a write to a
//! `WITHOUT ROWID` table (no update hook), is not timed; a statement is only
//! seen once SQLite has run [`PROGRESS_OPS`] instructions of it on that
//! worker, so a statement that is slow only because it waited for the lock
//! is not seen as slow (its writer's transaction is).

use std::{
    cell::RefCell,
    collections::HashMap,
    fmt::{self, Debug},
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, PoisonError},
    time::{Duration, Instant},
};

use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use tracing::{
    Dispatch, Event, Metadata,
    dispatcher::DefaultGuard,
    field::{Field, Visit},
    span,
    subscriber::Interest,
};

/// The longest a write transaction may hold the write lock.
pub const WRITE_LOCK_LIMIT: Duration = Duration::from_millis(50);

/// How many SQLite VM instructions between progress callbacks.
pub const PROGRESS_OPS: i32 = 1_000;

/// How long [`check`] waits after the work for the last statement's log
/// line (sqlx writes it when the statement's cursor is dropped, which can be
/// just after the caller has its result).
const SETTLE: Duration = Duration::from_millis(25);

const SLOW_TARGET: &str = "sqlx::query";

#[derive(Default)]
struct Observations {
    armed: usize,
    found: Vec<String>,
}

/// One database file's instrumentation.
#[derive(Default)]
struct DatabaseState {
    observations: Mutex<Observations>,
}

impl DatabaseState {
    fn armed(&self) -> bool {
        self.lock().armed > 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Observations> {
        self.observations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, observation: String) {
        let mut observations = self.lock();
        if observations.armed > 0 {
            observations.found.push(observation);
        }
    }
}

fn databases() -> &'static Mutex<HashMap<PathBuf, Arc<DatabaseState>>> {
    static DATABASES: OnceLock<Mutex<HashMap<PathBuf, Arc<DatabaseState>>>> = OnceLock::new();
    DATABASES.get_or_init(Mutex::default)
}

fn database(path: &Path) -> Option<Arc<DatabaseState>> {
    databases()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(path)
        .cloned()
}

thread_local! {
    /// The database this worker thread serves, and this module's dispatcher
    /// guard while a check is running on it.
    static WORKER: RefCell<Option<(Arc<DatabaseState>, Option<DefaultGuard>)>> =
        const { RefCell::new(None) };
}

/// Called from every hook, on the connection's worker thread: remembers
/// which database the thread serves, and routes its events to this module
/// while (and only while) a check is running on it.
fn on_worker(state: &Arc<DatabaseState>) {
    let armed = state.armed();
    WORKER.with(|worker| {
        let mut worker = worker.borrow_mut();
        let entry = worker.get_or_insert_with(|| (Arc::clone(state), None));
        match (armed, entry.1.is_some()) {
            (true, false) => entry.1 = Some(tracing::dispatcher::set_default(dispatch())),
            (false, true) => entry.1 = None,
            _ => {}
        }
    });
}

fn worker_database() -> Option<Arc<DatabaseState>> {
    WORKER.with(|worker| worker.borrow().as_ref().map(|(state, _)| Arc::clone(state)))
}

/// The per-connection write-transaction timer.
struct WriteTimer {
    state: Arc<DatabaseState>,
    first_write: Mutex<Option<Instant>>,
}

impl WriteTimer {
    fn first_write(&self) -> std::sync::MutexGuard<'_, Option<Instant>> {
        self.first_write
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn wrote(&self) {
        on_worker(&self.state);
        self.first_write().get_or_insert_with(Instant::now);
    }

    fn ended(&self, outcome: &str) {
        on_worker(&self.state);
        if let Some(started) = self.first_write().take() {
            let held = started.elapsed();
            if held > WRITE_LOCK_LIMIT {
                self.state.record(format!(
                    "a write transaction held the write lock {} ms (over {} ms) before its {outcome}",
                    held.as_millis(),
                    WRITE_LOCK_LIMIT.as_millis()
                ));
            }
        }
    }
}

/// Instruments every connection of a pool on the database file `path` (the
/// pool's `get_filename()`). `connect_database_path` calls it in test
/// builds.
pub(crate) fn instrument(options: SqlitePoolOptions, path: &Path) -> SqlitePoolOptions {
    let state = Arc::clone(
        databases()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(path.to_path_buf())
            .or_default(),
    );
    options.after_connect(move |conn, _metadata| {
        let state = Arc::clone(&state);
        Box::pin(async move {
            let timer = Arc::new(WriteTimer {
                state: Arc::clone(&state),
                first_write: Mutex::new(None),
            });
            let mut handle = conn.lock_handle().await?;
            handle.set_progress_handler(PROGRESS_OPS, move || {
                on_worker(&state);
                true
            });
            let wrote = Arc::clone(&timer);
            handle.set_update_hook(move |_| wrote.wrote());
            let committed = Arc::clone(&timer);
            handle.set_commit_hook(move || {
                committed.ended("commit");
                true
            });
            handle.set_rollback_hook(move || timer.ended("rollback"));
            Ok(())
        })
    })
}

/// Runs `work` and fails when, on `db`'s database, any statement took over
/// 100 ms or any write transaction held the write lock over
/// [`WRITE_LOCK_LIMIT`] while it ran. The work's own error, if any, is
/// returned as it is.
///
/// # Errors
/// The work failed; a slow statement or a long write transaction was seen;
/// or `db` was not opened by `connect_database_path` in a test build.
pub async fn check<T>(
    db: &SqlitePool,
    work: impl Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    let path = db.connect_options().get_filename().to_path_buf();
    let state = database(&path).ok_or_else(|| {
        anyhow::anyhow!(
            "speed_check: {} was not opened by connect_database_path with test-support, so it is not instrumented",
            path.display()
        )
    })?;
    {
        let mut observations = state.lock();
        if observations.armed == 0 {
            observations.found.clear();
        }
        observations.armed += 1;
    }
    let outcome = work.await;
    tokio::time::sleep(SETTLE).await;
    let found = {
        let mut observations = state.lock();
        observations.armed = observations.armed.saturating_sub(1);
        if observations.armed == 0 {
            std::mem::take(&mut observations.found)
        } else {
            observations.found.clone()
        }
    };
    let value = outcome?;
    if found.is_empty() {
        Ok(value)
    } else {
        anyhow::bail!(
            "speed_check failed on {}:\n- {}",
            path.display(),
            found.join("\n- ")
        )
    }
}

fn dispatch() -> &'static Dispatch {
    static DISPATCH: OnceLock<Dispatch> = OnceLock::new();
    DISPATCH.get_or_init(|| Dispatch::new(SlowStatements))
}

/// Records sqlx's slow-statement warning for the worker thread's database.
/// It is only ever a worker thread's default while a check runs on its
/// database, and it is interested in nothing else.
struct SlowStatements;

/// sqlx's statement log, and the `enabled!` check it makes before it (a
/// hint callsite, not an event).
fn is_slow_statement_log(metadata: &Metadata<'_>) -> bool {
    !metadata.is_span()
        && metadata.target() == SLOW_TARGET
        && *metadata.level() <= tracing::Level::WARN
}

#[derive(Default)]
struct Fields {
    message: String,
    summary: String,
    elapsed: String,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_debug(field, &value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        let text = format!("{value:?}");
        match field.name() {
            "message" => self.message = text,
            "summary" => text.trim_matches('"').clone_into(&mut self.summary),
            "elapsed" => self.elapsed = text,
            _ => {}
        }
    }
}

impl tracing::Subscriber for SlowStatements {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if is_slow_statement_log(metadata) {
            Interest::sometimes()
        } else {
            Interest::never()
        }
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::WARN)
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        is_slow_statement_log(metadata)
    }

    fn new_span(&self, _span: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }

    fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}

    fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        if !fields.message.contains("slow statement") {
            return;
        }
        if let Some(state) = worker_database() {
            state.record(format!(
                "slow statement ({}, over 100 ms): {}",
                fields.elapsed, fields.summary
            ));
        }
    }

    fn enter(&self, _span: &span::Id) {}

    fn exit(&self, _span: &span::Id) {}
}

impl fmt::Debug for SlowStatements {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SlowStatements")
    }
}
