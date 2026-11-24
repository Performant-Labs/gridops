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
//!   default from a hook), which reads sqlx's statement log: a statement is
//!   timed from its first progress callback or row change to its log line,
//!   and
//!   sqlx's slow-statement warning names it. Other threads, and the process's
//!   global subscriber, are left alone;
//! - [`check`] ends once every statement it saw start has written its log
//!   line (sqlx writes it when the statement's cursor is dropped, which can be
//!   after the caller has its row), and fails if one has not within 10 s.
//!
//! What is timed is the work, not the machine (#247). Wall-clock time counts
//! the time a loaded machine keeps the work's threads off the CPU, or its
//! disk keeps a write waiting, as if the work had spent it: under load the
//! same synthetic week failed with "held the write lock 71 ms" and passed on
//! a rerun. So:
//! - a statement is timed by the CPU time the connection's worker thread and
//!   the thread that called [`check`] used while it ran (Linux's
//!   `/proc/<thread>/schedstat`): what the statement did, whatever the
//!   machine was doing;
//! - a write transaction is timed by wall-clock time less the time those two
//!   threads spent runnable and waiting for a CPU (`run_delay`, the second
//!   field of `schedstat`) and less the time the worker spent blocked in the
//!   kernel inside statements that return no rows and read nothing from the
//!   disk (a write's disk I/O, which a busy disk stretches; a scan's reads
//!   from a cold database still count), and never less than the CPU time the
//!   two used.
//!
//! So a full scan counts in full in both (it holds the lock as long on a busy
//! machine as on an idle one), and a transaction still counts the caller's
//! work between statements, a lock held while the caller sleeps or waits on
//! anything but a CPU, and a worker blocked inside a statement that returns
//! rows (it may be waiting for the caller to take them). Where the kernel has
//! no `schedstat` (macOS), both are wall-clock time, as before. [`work_time`]
//! times any work the transaction's way.
//!
//! Limits, by design: a write transaction is timed from its first row
//! change, so a `BEGIN IMMEDIATE` held before any write, or a write to a
//! `WITHOUT ROWID` table (no update hook), is not timed; a statement that
//! has run fewer than [`PROGRESS_OPS`] instructions and changed no row is
//! timed by the worker's CPU since the previous statement's log line, so a
//! statement that is slow only because it waited (for the lock, a CPU or
//! the disk) is not seen as slow (its writer's transaction is). A check sees
//! a worker's statements only once one of its hooks has run on that worker
//! during the check: a read that changes no row and runs under
//! [`PROGRESS_OPS`] instructions as the first statement on a connection in
//! a check (`SELECT COUNT(*)` alone, one `OP_Count` that can walk a whole
//! table) logs to that thread's own default and is not seen. Two threads waiting
//! for a CPU at once both have their waits taken off, which the CPU-time
//! floor bounds. Only the worker's and the calling thread's waits are taken
//! off: a check whose work runs its statements from other threads (a
//! multi-thread runtime moving the task) keeps those threads' waits.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fmt::{self, Debug},
    future::Future,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
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

/// The longest a statement may take: `db.rs`'s slow-statement threshold,
/// after which sqlx logs the warning this module reads.
pub const SLOW_STATEMENT_LIMIT: Duration = Duration::from_millis(100);

/// How many SQLite VM instructions between progress callbacks.
pub const PROGRESS_OPS: i32 = 1_000;

/// The longest [`check`] waits after the work for the log lines of the
/// statements it saw start.
#[cfg(not(test))]
const LOG_WAIT: Duration = Duration::from_secs(10);
#[cfg(test)]
const LOG_WAIT: Duration = Duration::from_secs(1);

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
    /// The `schedstat` file of each thread running a check on this database.
    drivers: Mutex<Vec<PathBuf>>,
    /// The `schedstat` file of each connection's worker thread, for
    /// [`work_time`].
    workers: Mutex<Vec<PathBuf>>,
    /// Statements marked by [`statement_started`] whose log line has not
    /// come yet.
    in_flight: AtomicUsize,
    /// Every statement [`statement_started`] marked.
    #[cfg(test)]
    marked: AtomicUsize,
}

impl DatabaseState {
    fn drivers(&self) -> Vec<PathBuf> {
        self.drivers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn workers(&self) -> Vec<PathBuf> {
        self.workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

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
    /// The statement this worker thread is running, from its first progress
    /// callback while a check is armed until its log line.
    static STATEMENT: RefCell<Option<Mark>> = const { RefCell::new(None) };
    /// This worker thread's times at its last statement log line, or when a
    /// check began routing its events here.
    static LAST_LOG: Cell<Option<ThreadTimes>> = const { Cell::new(None) };
    /// The time this worker thread has spent blocked in the kernel inside
    /// statements that returned no rows, ended ones only.
    static WRITES_BLOCKED: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

/// Called from every hook, on the connection's worker thread: remembers
/// which database the thread serves, and routes its events to this module
/// while (and only while) a check is running on it.
fn on_worker(state: &Arc<DatabaseState>) {
    let armed = state.armed();
    WORKER.with(|worker| {
        let mut worker = worker.borrow_mut();
        let entry = worker.get_or_insert_with(|| {
            if let Some(path) = this_thread_schedstat() {
                state
                    .workers
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(path);
            }
            (Arc::clone(state), None)
        });
        match (armed, entry.1.is_some()) {
            (true, false) => {
                // A statement marked before this check began is not its.
                statement_ended(state);
                LAST_LOG.set(thread_times(Path::new(THIS_THREAD)));
                entry.1 = Some(tracing::dispatcher::set_default(dispatch()));
            }
            (false, true) => {
                statement_ended(state);
                entry.1 = None;
            }
            _ => {}
        }
    });
}

fn worker_database() -> Option<Arc<DatabaseState>> {
    WORKER.with(|worker| worker.borrow().as_ref().map(|(state, _)| Arc::clone(state)))
}

/// How long a thread has run, and how long it has waited, runnable, for a
/// CPU: the first two fields of Linux's `/proc/<thread>/schedstat`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ThreadTimes {
    cpu: Duration,
    waited: Duration,
}

/// The calling thread's `schedstat` file.
const THIS_THREAD: &str = "/proc/thread-self/schedstat";

/// The times in `schedstat`, or `None` where there is no such file.
fn thread_times(schedstat: &Path) -> Option<ThreadTimes> {
    parse_schedstat(&std::fs::read_to_string(schedstat).ok()?)
}

fn parse_schedstat(text: &str) -> Option<ThreadTimes> {
    let mut fields = text.split_ascii_whitespace();
    let cpu = fields.next()?.parse().ok()?;
    let waited = fields.next()?.parse().ok()?;
    Some(ThreadTimes {
        cpu: Duration::from_nanos(cpu),
        waited: Duration::from_nanos(waited),
    })
}

/// The `schedstat` file of the calling thread under its own id, so another
/// thread can read it.
fn this_thread_schedstat() -> Option<PathBuf> {
    let task = std::fs::read_link("/proc/thread-self").ok()?;
    Some(Path::new("/proc").join(task).join("schedstat"))
}

/// A moment, and the times of the threads some work runs on, from which
/// [`Mark::busy`] times the work.
struct Mark {
    at: Instant,
    threads: Vec<(PathBuf, ThreadTimes)>,
    /// On a worker thread: its blocked time in writes so far
    /// ([`writes_blocked`]).
    writes_blocked: Duration,
    /// On a worker thread: the bytes it had read from storage.
    disk_reads: Option<u64>,
}

impl Mark {
    /// Now, for the threads whose `schedstat` files are `threads`.
    fn of(threads: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            at: Instant::now(),
            threads: threads
                .into_iter()
                .filter_map(|path| thread_times(&path).map(|times| (path, times)))
                .collect(),
            writes_blocked: Duration::ZERO,
            disk_reads: None,
        }
    }

    /// Now, on a worker thread of `state`'s database: the worker (first) and
    /// the threads running checks on it. Read it on the same worker.
    fn on_worker(state: &DatabaseState) -> Self {
        let mut mark = Self::of(std::iter::once(PathBuf::from(THIS_THREAD)).chain(state.drivers()));
        mark.writes_blocked = writes_blocked();
        mark.disk_reads = disk_reads();
        mark
    }

    /// On the marked worker thread: the bytes it has read from storage
    /// since the mark.
    fn read_from_disk(&self) -> Option<u64> {
        Some(disk_reads()?.saturating_sub(self.disk_reads?))
    }

    /// The times of the marked threads now, then and now, with `joined` the
    /// threads that started since the mark (all of whose times are since).
    fn times(&self, joined: &[PathBuf]) -> Vec<(ThreadTimes, ThreadTimes)> {
        let marked = self
            .threads
            .iter()
            .filter_map(|(path, then)| thread_times(path).map(|now| (*then, now)));
        let joined = joined
            .iter()
            .filter(|path| self.threads.iter().all(|(marked, _)| marked != *path))
            .filter_map(|path| thread_times(path).map(|now| (ThreadTimes::default(), now)));
        marked.chain(joined).collect()
    }

    /// On the marked worker thread: how long it has spent blocked in the
    /// kernel since the mark, out of `elapsed`.
    fn worker_blocked(&self, elapsed: Duration) -> Duration {
        match self.threads.first() {
            Some((path, then)) if path == Path::new(THIS_THREAD) => {
                thread_times(path).map_or(Duration::ZERO, |now| blocked(elapsed, *then, now))
            }
            _ => Duration::ZERO,
        }
    }

    /// On the marked worker thread: how long the work took since the mark,
    /// `elapsed` by the clock, less what [`busy`] takes off and the time the
    /// worker was blocked in writes in between.
    fn busy_on_worker(&self, elapsed: Duration) -> Duration {
        let writes = writes_blocked().saturating_sub(self.writes_blocked);
        busy(elapsed.saturating_sub(writes), self.times(&[]).into_iter())
    }
}

/// The time a thread spent neither running nor waiting for a CPU, out of
/// `elapsed`, between its times `then` and `now`.
fn blocked(elapsed: Duration, then: ThreadTimes, now: ThreadTimes) -> Duration {
    elapsed
        .saturating_sub(now.cpu.saturating_sub(then.cpu))
        .saturating_sub(now.waited.saturating_sub(then.waited))
}

/// `elapsed` less the waits for a CPU in each thread's `(then, now)` times,
/// and never less than the CPU time the threads used (up to `elapsed`).
///
/// Two threads can wait at once (the worker preempted just after it woke
/// the caller), and then both waits are taken off; the floor keeps the
/// work's own CPU time in full whatever the waits add up to.
fn busy(elapsed: Duration, times: impl Iterator<Item = (ThreadTimes, ThreadTimes)>) -> Duration {
    let (waits, worked) = times.fold(
        (Duration::ZERO, Duration::ZERO),
        |(waits, worked), (then, now)| {
            (
                waits.saturating_add(now.waited.saturating_sub(then.waited)),
                worked.saturating_add(now.cpu.saturating_sub(then.cpu)),
            )
        },
    );
    elapsed.saturating_sub(waits).max(worked.min(elapsed))
}

/// The work of a statement nothing marked, `elapsed` by sqlx's clock: the
/// worker's CPU time since its previous log line (`last_log`), up to
/// `elapsed`. Nothing marks a statement that changes no row and runs under
/// [`PROGRESS_OPS`] instructions, but one instruction can walk a whole table
/// (`SELECT COUNT(*)` is a single `OP_Count`), so its CPU still counts; a
/// statement that only waited (for the lock, a CPU or the disk) used none.
/// The worker's CPU between log lines also holds its unlogged `BEGIN` and
/// `COMMIT`, which only ever adds. Without `schedstat`, the clock.
fn unmarked_work(
    elapsed: Duration,
    last_log: Option<ThreadTimes>,
    now: Option<ThreadTimes>,
) -> Duration {
    match (last_log, now) {
        (Some(last_log), Some(now)) => now.cpu.saturating_sub(last_log.cpu).min(elapsed),
        _ => elapsed,
    }
}

/// How much of a statement's `blocked` time (the worker neither running nor
/// waiting for a CPU) is the machine's: all of it for a statement that
/// returned no rows and read nothing from the disk, whose stall is the disk
/// busy with others' writes; none otherwise. A statement that returns rows
/// may be waiting for the caller to take them, and one that read from the
/// disk (a scan on a cold database) was doing its own work.
fn excused(blocked: Duration, rows_returned: Option<u64>, read_from_disk: Option<u64>) -> Duration {
    if rows_returned == Some(0) && read_from_disk == Some(0) {
        blocked
    } else {
        Duration::ZERO
    }
}

/// The bytes the calling thread has read from storage (`read_bytes` in
/// Linux's `/proc/thread-self/io`), or `None` where there is no such file.
fn disk_reads() -> Option<u64> {
    std::fs::read_to_string("/proc/thread-self/io")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("read_bytes:"))?
        .trim()
        .parse()
        .ok()
}

/// The CPU time the threads used between each `(then, now)` of their
/// times, up to `elapsed`.
fn worked(elapsed: Duration, times: impl Iterator<Item = (ThreadTimes, ThreadTimes)>) -> Duration {
    times
        .map(|(then, now)| now.cpu.saturating_sub(then.cpu))
        .fold(Duration::ZERO, Duration::saturating_add)
        .min(elapsed)
}

/// This worker thread's blocked time that is the machine's ([`excused`]),
/// the statement it is running included as one that returns no rows (its
/// rows are not known before its log line, and a write transaction ends
/// inside its last statement).
fn writes_blocked() -> Duration {
    let running = STATEMENT.with_borrow(|statement| {
        statement.as_ref().map_or(Duration::ZERO, |mark| {
            excused(
                mark.worker_blocked(mark.at.elapsed()),
                Some(0),
                mark.read_from_disk(),
            )
        })
    });
    WRITES_BLOCKED.get().saturating_add(running)
}

/// From the progress handler and the update hook: marks the start of the
/// worker's statement, if it is not marked yet.
fn statement_started(state: &DatabaseState) {
    if state.armed() && STATEMENT.with_borrow(Option::is_none) {
        let mark = Mark::on_worker(state);
        state.in_flight.fetch_add(1, Ordering::SeqCst);
        #[cfg(test)]
        state.marked.fetch_add(1, Ordering::SeqCst);
        STATEMENT.set(Some(mark));
    }
}

/// The statement marked on this worker has ended (or is no longer the
/// check's).
fn statement_ended(state: &DatabaseState) -> Option<Mark> {
    let mark = STATEMENT.take();
    if mark.is_some() {
        // A check that gave up waiting has already set the count to zero.
        let _ = state
            .in_flight
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_sub(1)
            });
    }
    mark
}

/// Runs `work` and returns how long it took by the rule of [`check`]'s
/// timers: wall-clock time less the time the calling thread and the worker
/// threads of `db`'s connections waited for a CPU, and never less than their
/// CPU time. For a time budget that must not depend on how busy the machine
/// is (the synthetic week's 20 s). On a pool not opened by
/// `connect_database_path` in a test build, only the calling thread's
/// waits are taken off.
pub async fn work_time<T>(db: &SqlitePool, work: impl Future<Output = T>) -> (T, Duration) {
    let state = database(db.connect_options().get_filename());
    let workers = || {
        state
            .as_ref()
            .map_or_else(Vec::new, |state| state.workers())
    };
    let mark = Mark::of(this_thread_schedstat().into_iter().chain(workers()));
    let value = work.await;
    let took = busy(mark.at.elapsed(), mark.times(&workers()).into_iter());
    (value, took)
}

/// The per-connection write-transaction timer.
struct WriteTimer {
    state: Arc<DatabaseState>,
    first_write: Mutex<Option<Mark>>,
}

impl WriteTimer {
    fn first_write(&self) -> std::sync::MutexGuard<'_, Option<Mark>> {
        self.first_write
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn wrote(&self) {
        on_worker(&self.state);
        // A write statement starts at its first row change at the latest,
        // however few instructions it runs.
        statement_started(&self.state);
        self.first_write()
            .get_or_insert_with(|| Mark::on_worker(&self.state));
    }

    fn ended(&self, outcome: &str) {
        on_worker(&self.state);
        if let Some(started) = self.first_write().take() {
            let held = started.busy_on_worker(started.at.elapsed());
            if held > WRITE_LOCK_LIMIT {
                self.state.record(format!(
                    "a write transaction held the write lock {} ms, less waits for the machine (over {} ms), before its {outcome}",
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
                statement_started(&state);
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
/// [`SLOW_STATEMENT_LIMIT`] or any write transaction held the write lock
/// over [`WRITE_LOCK_LIMIT`] while it ran, both timed as the module says.
/// The work's own error, if any, is returned as it is.
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
    let driver = this_thread_schedstat();
    if let Some(driver) = &driver {
        state
            .drivers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(driver.clone());
    }
    let outcome = work.await;
    // The last statement's log line can come after the caller has its row,
    // later still when the worker is kept off the CPU.
    let waiting = Instant::now();
    while state.in_flight.load(Ordering::SeqCst) > 0 && waiting.elapsed() < LOG_WAIT {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let unlogged = state.in_flight.swap(0, Ordering::SeqCst);
    if unlogged > 0 {
        // Far over the statement limit, or the check is not seeing the
        // statement log at all: either way not a pass. The next check
        // starts from zero rather than waiting for them again.
        state.record(format!(
            "{unlogged} statement(s) had no log line {} s after the work ended",
            LOG_WAIT.as_secs()
        ));
    }
    if let Some(driver) = &driver {
        let mut drivers = state.drivers.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(index) = drivers.iter().position(|path| path == driver) {
            drivers.swap_remove(index);
        }
    }
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

/// This module's dispatcher, registered with an inert second one.
///
/// tracing-core works out whether a callsite is of interest from every
/// registered dispatcher, except while only one is registered: then it asks
/// the default of whichever thread reaches the callsite first
/// (`Rebuilder::JustOne`). sqlx's statement log is usually first reached
/// outside a check, on a worker whose default is the process's (none in a
/// test), and was then never of interest again: the check stopped seeing
/// statements (#247). With two registered, every callsite asks both, and
/// this module's says it wants the statement log.
fn dispatch() -> &'static Dispatch {
    static DISPATCH: OnceLock<(Dispatch, Dispatch)> = OnceLock::new();
    &DISPATCH
        .get_or_init(|| (Dispatch::new(Inert), Dispatch::new(SlowStatements)))
        .1
}

/// A dispatcher interested in nothing, registered only so that there are
/// two ([`dispatch`]).
struct Inert;

impl tracing::Subscriber for Inert {
    fn register_callsite(&self, _metadata: &'static Metadata<'static>) -> Interest {
        Interest::never()
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::OFF)
    }

    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        false
    }

    fn new_span(&self, _span: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }

    fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}

    fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}

    fn event(&self, _event: &Event<'_>) {}

    fn enter(&self, _span: &span::Id) {}

    fn exit(&self, _span: &span::Id) {}
}

/// Reads sqlx's statement log for the worker thread's database: every line
/// ends the statement [`statement_started`] marked, and the slow-statement
/// warning is recorded when the statement did over [`SLOW_STATEMENT_LIMIT`]
/// of CPU work. It is only ever a
/// worker thread's default while a check runs on its database, and it is
/// interested in nothing else.
struct SlowStatements;

/// sqlx's statement log, at any level, and the `enabled!` check it makes
/// before it (a hint callsite, not an event).
fn is_statement_log(metadata: &Metadata<'_>) -> bool {
    !metadata.is_span() && metadata.target() == SLOW_TARGET
}

#[derive(Default)]
struct Fields {
    message: String,
    summary: String,
    elapsed: String,
    elapsed_secs: Option<f64>,
    rows_returned: Option<u64>,
}

impl Visit for Fields {
    fn record_f64(&mut self, field: &Field, value: f64) {
        if field.name() == "elapsed_secs" {
            self.elapsed_secs = Some(value);
        } else {
            self.record_debug(field, &value);
        }
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "rows_returned" {
            self.rows_returned = Some(value);
        } else {
            self.record_debug(field, &value);
        }
    }

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

impl SlowStatements {
    /// The work of the statement whose log line this is: the CPU time the
    /// worker and the threads running checks used since it was marked, up
    /// to sqlx's `elapsed`; a statement nothing marked, by
    /// [`unmarked_work`]. Where the kernel has no `schedstat`, every
    /// statement keeps sqlx's `elapsed`.
    fn took(
        started: Option<&Mark>,
        fields: &Fields,
        last_log: Option<ThreadTimes>,
        now: Option<ThreadTimes>,
    ) -> Duration {
        let Some(elapsed) = fields
            .elapsed_secs
            .and_then(|secs| Duration::try_from_secs_f64(secs).ok())
        else {
            return Duration::MAX;
        };
        let Some(started) = started else {
            return unmarked_work(elapsed, last_log, now);
        };
        let writes = excused(
            started.worker_blocked(started.at.elapsed()),
            fields.rows_returned,
            started.read_from_disk(),
        );
        WRITES_BLOCKED.set(WRITES_BLOCKED.get().saturating_add(writes));
        let times = started.times(&[]);
        if times.is_empty() {
            // No `schedstat` (macOS): the clock.
            return elapsed;
        }
        worked(elapsed, times.into_iter())
    }
}

impl tracing::Subscriber for SlowStatements {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if is_statement_log(metadata) {
            Interest::sometimes()
        } else {
            Interest::never()
        }
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::DEBUG)
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        is_statement_log(metadata)
    }

    fn new_span(&self, _span: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }

    fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}

    fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}

    fn event(&self, event: &Event<'_>) {
        let Some(state) = worker_database() else {
            return;
        };
        let started = statement_ended(&state);
        let now = thread_times(Path::new(THIS_THREAD));
        let last_log = LAST_LOG.replace(now);
        let mut fields = Fields::default();
        event.record(&mut fields);
        let took = Self::took(started.as_ref(), &fields, last_log, now);
        if !fields.message.contains("slow statement") || took <= SLOW_STATEMENT_LIMIT {
            return;
        }
        state.record(format!(
            "slow statement ({}, {} ms of CPU, over {} ms): {}",
            fields.elapsed,
            took.as_millis(),
            SLOW_STATEMENT_LIMIT.as_millis(),
            fields.summary
        ));
    }

    fn enter(&self, _span: &span::Id) {}

    fn exit(&self, _span: &span::Id) {}
}

impl fmt::Debug for SlowStatements {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SlowStatements")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn times(cpu: u64, waited: u64) -> ThreadTimes {
        ThreadTimes {
            cpu: Duration::from_millis(cpu),
            waited: Duration::from_millis(waited),
        }
    }

    /// A write statement too small for a progress callback is timed too,
    /// so that a busy disk's stall inside it is the machine's.
    #[tokio::test]
    async fn a_write_statement_is_timed_however_small() -> anyhow::Result<()> {
        let directory =
            std::env::temp_dir().join(format!("gridops-speed-check-{}", uuid::Uuid::new_v4()));
        let pool = crate::connect_database_path(&directory.join("db.sqlite")).await?;
        let state = database(pool.connect_options().get_filename())
            .ok_or_else(|| anyhow::anyhow!("the pool is not instrumented"))?;
        // One connection, its schema already read, so the insert below runs
        // a handful of instructions.
        let mut conn = pool.acquire().await?;
        sqlx::query("INSERT INTO settings (key,value,updated_at) VALUES ('warm','1',1)")
            .execute(&mut *conn)
            .await?;
        let before = state.marked.load(Ordering::SeqCst);
        check(&pool, async {
            sqlx::query("INSERT INTO settings (key,value,updated_at) VALUES ('small','1',1)")
                .execute(&mut *conn)
                .await?;
            Ok(())
        })
        .await?;
        assert_eq!(state.marked.load(Ordering::SeqCst) - before, 1);
        assert_eq!(state.in_flight.load(Ordering::SeqCst), 0);
        drop(conn);
        pool.close().await;
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }

    /// A statement too small to be marked (no row change, under
    /// [`PROGRESS_OPS`] instructions) is timed by the worker's CPU
    /// ([`unmarked_work`]): one that is slow by the clock only because it
    /// waited (here for the write lock, as a busy CPU would make it wait too)
    /// used next to none and is not a slow statement.
    #[tokio::test]
    async fn a_statement_too_small_to_mark_is_not_slow_by_the_clock() -> anyhow::Result<()> {
        if thread_times(Path::new(THIS_THREAD)).is_none() {
            // No `schedstat` (macOS): the clock is all there is.
            return Ok(());
        }
        let directory =
            std::env::temp_dir().join(format!("gridops-speed-check-{}", uuid::Uuid::new_v4()));
        let pool = crate::connect_database_path(&directory.join("db.sqlite")).await?;
        let mut holder = pool.acquire().await?;
        let mut waiter = pool.acquire().await?;
        // Both connections have read the schema, so the delete below runs a
        // handful of instructions and changes no row.
        for conn in [&mut holder, &mut waiter] {
            sqlx::query("SELECT COUNT(*) FROM settings")
                .fetch_one(&mut **conn)
                .await?;
        }
        // The write lock, held without a write (which is not timed).
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *holder).await?;
        let release = async {
            tokio::time::sleep(Duration::from_millis(250)).await;
            sqlx::query("ROLLBACK").execute(&mut *holder).await
        };
        let wait = check(&pool, async {
            let started = Instant::now();
            sqlx::query("DELETE FROM settings WHERE key='absent'")
                .execute(&mut *waiter)
                .await?;
            Ok(started.elapsed())
        });
        let (released, waited) = tokio::join!(release, wait);
        released?;
        let waited = waited?;
        assert!(
            waited > SLOW_STATEMENT_LIMIT,
            "the delete waited only {waited:?} for the lock, so this proves nothing"
        );
        drop((holder, waiter));
        pool.close().await;
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn schedstat_is_cpu_time_then_wait_time_in_nanoseconds() {
        assert_eq!(
            parse_schedstat("15000000 40000000 12\n"),
            Some(times(15, 40))
        );
        assert_eq!(parse_schedstat(""), None);
        assert_eq!(parse_schedstat("12 x 3"), None);
    }

    #[test]
    fn the_worker_and_the_callers_waits_for_a_cpu_are_taken_off() {
        // 180 ms by the clock: the worker ran 15 ms and waited 150 ms, the
        // caller waited 10 ms.
        let worker = (times(100, 1_000), times(115, 1_150));
        let caller = (times(0, 500), times(3, 510));
        assert_eq!(
            busy(Duration::from_millis(180), [worker, caller].into_iter()),
            Duration::from_millis(20)
        );
    }

    #[test]
    fn a_sleep_is_not_a_wait_for_a_cpu() {
        // The worker ran 1 ms and waited for nothing while the caller slept.
        let worker = (times(0, 0), times(1, 0));
        assert_eq!(
            busy(Duration::from_millis(150), std::iter::once(worker)),
            Duration::from_millis(150)
        );
    }

    #[test]
    fn never_less_than_the_cpu_time_the_threads_used() {
        // Waits that overlap (the caller runnable while the worker also was)
        // can add up to more than the clock: the work's own CPU time still
        // counts in full.
        let worker = (times(0, 0), times(70, 60));
        let caller = (times(0, 0), times(10, 60));
        assert_eq!(
            busy(Duration::from_millis(140), [worker, caller].into_iter()),
            Duration::from_millis(80)
        );
        // Threads that ran at once count no more than the clock.
        assert_eq!(
            busy(Duration::from_millis(50), [worker, caller].into_iter()),
            Duration::from_millis(50)
        );
    }

    #[test]
    fn blocked_is_the_time_neither_running_nor_waiting_for_a_cpu() {
        // 100 ms by the clock: 30 ms run, 30 ms waiting for a CPU.
        assert_eq!(
            blocked(Duration::from_millis(100), times(10, 20), times(40, 50)),
            Duration::from_millis(40)
        );
        assert_eq!(
            blocked(Duration::from_millis(50), times(0, 0), times(40, 30)),
            Duration::ZERO
        );
    }

    #[test]
    fn a_statement_is_the_cpu_time_its_threads_used() {
        // 300 ms by the clock on a busy CPU: the worker ran 40 ms, the caller
        // 2 ms.
        let worker = (times(100, 0), times(140, 250));
        let caller = (times(0, 0), times(2, 8));
        assert_eq!(
            worked(Duration::from_millis(300), [worker, caller].into_iter()),
            Duration::from_millis(42)
        );
        assert_eq!(
            worked(Duration::from_millis(30), [worker, caller].into_iter()),
            Duration::from_millis(30)
        );
    }

    #[test]
    fn a_statement_nothing_marked_is_the_workers_cpu_since_its_last_log_line() {
        // `SELECT COUNT(*) FROM t` is one VM instruction (`OP_Count`) that
        // walks the whole table, so nothing marks it: 150 ms of the worker's
        // CPU since the previous statement's log line is its work.
        let last = times(1_000, 400);
        let now = times(1_150, 450);
        assert_eq!(
            unmarked_work(Duration::from_millis(210), Some(last), Some(now)),
            Duration::from_millis(150)
        );
        // One that only waited (no CPU) did no work.
        assert_eq!(
            unmarked_work(Duration::from_millis(300), Some(now), Some(now)),
            Duration::ZERO
        );
        // No `schedstat`: the clock.
        assert_eq!(
            unmarked_work(Duration::from_millis(300), None, None),
            Duration::from_millis(300)
        );
    }

    #[test]
    fn a_write_that_read_from_the_disk_keeps_its_stalls() {
        let stalled = Duration::from_millis(30);
        // A write that read nothing from the disk: its stall is the disk
        // being busy with others' writes.
        assert_eq!(excused(stalled, Some(0), Some(0)), stalled);
        // A write that read pages from the disk (a scan on a cold
        // database): those reads are its own work.
        assert_eq!(excused(stalled, Some(0), Some(4_096)), Duration::ZERO);
        // Unknown reads, or a statement that returned rows: nothing excused.
        assert_eq!(excused(stalled, Some(0), None), Duration::ZERO);
        assert_eq!(excused(stalled, Some(3), Some(0)), Duration::ZERO);
    }

    /// A statement the check saw start and never saw log fails the check
    /// once the wait for its log line runs out, rather than passing quietly.
    #[tokio::test]
    async fn a_statement_that_never_logs_fails_the_check() -> anyhow::Result<()> {
        let directory =
            std::env::temp_dir().join(format!("gridops-speed-check-{}", uuid::Uuid::new_v4()));
        let pool = crate::connect_database_path(&directory.join("db.sqlite")).await?;
        let state = database(pool.connect_options().get_filename())
            .ok_or_else(|| anyhow::anyhow!("the pool is not instrumented"))?;
        state.in_flight.fetch_add(1, Ordering::SeqCst);
        let outcome = check(&pool, async { Ok(()) }).await;
        let in_flight = state.in_flight.swap(0, Ordering::SeqCst);
        pool.close().await;
        std::fs::remove_dir_all(directory)?;
        let error = match outcome {
            Ok(()) => anyhow::bail!("a statement that never logged passed the check"),
            Err(error) => format!("{error:#}"),
        };
        assert!(error.contains("no log line"), "{error}");
        assert_eq!(in_flight, 0, "the next check would wait for it again");
        Ok(())
    }

    #[test]
    fn without_schedstat_the_clock_is_kept() {
        assert_eq!(
            busy(Duration::from_millis(70), std::iter::empty()),
            Duration::from_millis(70)
        );
    }
}
