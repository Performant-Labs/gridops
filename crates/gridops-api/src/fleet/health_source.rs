//! The database source of the fleet health page (#204): the windowed half
//! from `gridops_core::fleet_health::window` (the hourly rollups for the
//! hours past their grace, the later hours computed live, the scores and
//! windowed concerns, the usage by category and the trend from the
//! snapshots) with #193's speed indexes cached 5 minutes per window, and the
//! live half from `health_live::collect_live`.
//!
//! #240's rules hold here too: the live read takes no lock of the source's
//! (it reads the database and returns, and nothing of it is kept), and the
//! answers don't depend on how many windows are asked at once.
//!
//! The speed index reads raw jobs (the 7 d one about 0.4 s on production's
//! data, against the page's 300 ms), so it is kept off the request path: a
//! refresher ([`DbFleetHealthSource::start_refresher`], started with the API)
//! reads every window's indexes every [`SPEED_REFRESH_EVERY`], one window at
//! a time, and a request is served the window's last indexes whatever their
//! age, refreshing them behind the call when they are over
//! [`SPEED_TTL_MILLIS`] old (should the refresher fall behind). A request
//! reads inline only when the window has never been read: the first page
//! before the refresher's first round has finished. One read per window runs
//! at a time; it holds only that window's read lock, which no other window
//! and no live read takes, and which a request takes only when there is
//! nothing to serve.

use std::{
    sync::{
        Arc, Mutex as SyncMutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use futures_util::future::BoxFuture;
use gridops_core::{
    fleet::registry::RegistryClock,
    fleet_health::{
        FleetHealthSource, HealthWindow, LiveHealth, SpeedResult, WindowedHealth, speed, window,
    },
};
use sqlx::SqlitePool;
use tokio::{sync::Mutex, task::JoinHandle};

use super::health_live::collect_live;

/// How long a window's speed indexes are served without a refresh behind
/// the call (the epic's "cached for 5 minutes").
const SPEED_TTL_MILLIS: i64 = 5 * 60 * 1_000;

/// How often the refresher reads every window's indexes: under
/// [`SPEED_TTL_MILLIS`], so a request never finds them due.
pub(crate) const SPEED_REFRESH_EVERY: Duration = Duration::from_mins(4);

/// A window's speed indexes and the time they were read as of.
type CachedSpeed = (i64, Arc<Vec<SpeedResult>>);

/// Reads a window's speed indexes: `speed::speed_indexes`, or a test's.
pub(crate) type SpeedReader = Arc<
    dyn Fn(SqlitePool, HealthWindow, i64) -> BoxFuture<'static, anyhow::Result<Vec<SpeedResult>>>
        + Send
        + Sync,
>;

/// One window's speed cache.
#[derive(Debug, Default)]
struct SpeedCache {
    /// Only locked to read or write it, never across an await.
    entry: SyncMutex<Option<CachedSpeed>>,
    /// Held by whoever reads the window's indexes, so one read runs at a
    /// time and a caller that waited on it finds the result.
    reading: Mutex<()>,
    /// A refresh is under way: a second one is skipped, not queued.
    refreshing: AtomicBool,
}

impl SpeedCache {
    /// The last indexes read, if any, and their age at `now`.
    fn last(&self, now: i64) -> Option<(i64, Arc<Vec<SpeedResult>>)> {
        let entry = self.entry.lock().unwrap_or_else(PoisonError::into_inner);
        entry
            .as_ref()
            .map(|(at, results)| (now - at, Arc::clone(results)))
    }

    /// Keeps `results`, read as of `at`. A refresh keeps a later read the
    /// cache already holds (`only_newer`); a request's own read replaces
    /// whatever is there.
    fn store(&self, at: i64, results: Arc<Vec<SpeedResult>>, only_newer: bool) {
        let mut entry = self.entry.lock().unwrap_or_else(PoisonError::into_inner);
        if !only_newer || entry.as_ref().is_none_or(|(kept, _)| *kept <= at) {
            *entry = Some((at, results));
        }
    }
}

/// Clears a window's `refreshing` flag when its refresh ends, however it
/// ends (a panic or a cancelled task included).
struct Refreshing<'a>(&'a AtomicBool);

impl Drop for Refreshing<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// The speed cache of each window.
#[derive(Debug, Default)]
struct SpeedCaches {
    hour: SpeedCache,
    day: SpeedCache,
    week: SpeedCache,
}

impl SpeedCaches {
    const fn window(&self, window: HealthWindow) -> &SpeedCache {
        match window {
            HealthWindow::Hour => &self.hour,
            HealthWindow::Day => &self.day,
            HealthWindow::Week => &self.week,
        }
    }
}

/// Reads the fleet's health from the database.
#[derive(Clone)]
pub(crate) struct DbFleetHealthSource {
    db: SqlitePool,
    speed: Arc<SpeedCaches>,
    reader: SpeedReader,
    /// Speed reads a request waited for (nothing cached for its window).
    inline_reads: Arc<AtomicU64>,
}

impl std::fmt::Debug for DbFleetHealthSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DbFleetHealthSource")
            .field("speed", &self.speed)
            .finish_non_exhaustive()
    }
}

impl DbFleetHealthSource {
    pub(crate) fn new(db: SqlitePool) -> Self {
        Self {
            db,
            speed: Arc::default(),
            reader: Arc::new(|db, window, now| {
                Box::pin(async move { speed::speed_indexes(&db, window, now).await })
            }),
            inline_reads: Arc::default(),
        }
    }

    /// Replaces the speed reader (a test's, which can fail or panic).
    #[cfg(test)]
    pub(crate) fn with_speed_reader(mut self, reader: SpeedReader) -> Self {
        self.reader = reader;
        self
    }

    /// Starts the refresher: every window's indexes now, then every `every`,
    /// one window after another (never two reads of a window at once), as of
    /// `clock`. Each round's refreshes run as their own task, so one that
    /// fails or panics only skips its window until the next round. It runs
    /// until the returned task is aborted (the API: for as long as it runs).
    pub(crate) fn start_refresher(
        &self,
        clock: Arc<dyn RegistryClock>,
        every: Duration,
    ) -> JoinHandle<()> {
        let source = self.clone();
        tokio::spawn(async move {
            loop {
                for window in HealthWindow::ALL {
                    let round = source.clone();
                    let now = clock.now_millis();
                    // A panic ends only that window's refresh (its flag is
                    // cleared by `Refreshing`); the round goes on.
                    let _ = tokio::spawn(async move { round.refresh(window, now).await }).await;
                }
                tokio::time::sleep(every).await;
            }
        })
    }

    /// `window`'s speed indexes at `now`: the last ones read, whatever their
    /// age, refreshed behind the call when over [`SPEED_TTL_MILLIS`] old or
    /// read as of a later time; read before answering only when there are
    /// none.
    async fn speed(&self, window: HealthWindow, now: i64) -> anyhow::Result<Arc<Vec<SpeedResult>>> {
        let cache = self.speed.window(window);
        if let Some((age, results)) = cache.last(now) {
            if !(0..SPEED_TTL_MILLIS).contains(&age) {
                self.refresh_behind(window, now);
            }
            return Ok(results);
        }
        let _reading = cache.reading.lock().await;
        if let Some((_, results)) = cache.last(now) {
            return Ok(results);
        }
        self.inline_reads.fetch_add(1, Ordering::SeqCst);
        let results = Arc::new((self.reader)(self.db.clone(), window, now).await?);
        cache.store(now, Arc::clone(&results), false);
        Ok(results)
    }

    /// One read of `window`'s indexes as of `now` into its cache, unless a
    /// refresh of the window is under way. A failure is logged; the next
    /// round, or the next stale request, tries again.
    async fn refresh(&self, window: HealthWindow, now: i64) {
        let cache = self.speed.window(window);
        if cache.refreshing.swap(true, Ordering::SeqCst) {
            return;
        }
        let _refreshing = Refreshing(&cache.refreshing);
        let _reading = cache.reading.lock().await;
        match (self.reader)(self.db.clone(), window, now).await {
            Ok(results) => cache.store(now, Arc::new(results), true),
            Err(error) => {
                tracing::warn!(error = ?error, %window, "fleet health speed index refresh failed");
            }
        }
    }

    /// [`Self::refresh`] on its own task, for a request that found its
    /// window's indexes due. Needs a Tokio runtime; without one it does
    /// nothing.
    fn refresh_behind(&self, window: HealthWindow, now: i64) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self.speed.window(window).refreshing.load(Ordering::SeqCst) {
            return;
        }
        let source = self.clone();
        runtime.spawn(async move { source.refresh(window, now).await });
    }

    /// Speed reads requests waited for.
    #[cfg(test)]
    pub(super) fn inline_speed_reads(&self) -> u64 {
        self.inline_reads.load(Ordering::SeqCst)
    }

    /// Whether a refresh of `window` is under way.
    #[cfg(test)]
    pub(super) fn is_refreshing(&self, window: HealthWindow) -> bool {
        self.speed.window(window).refreshing.load(Ordering::SeqCst)
    }

    /// Waits for `window`'s refresh, if one is under way.
    #[cfg(test)]
    pub(super) async fn speed_refreshed(&self, window: HealthWindow) {
        while self.is_refreshing(window) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// When `window`'s cached indexes were read as of, if any.
    #[cfg(test)]
    pub(super) fn cached_speed_at(&self, window: HealthWindow) -> Option<i64> {
        self.speed
            .window(window)
            .entry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|(at, _)| *at)
    }

    /// Reads every window's indexes as of `now` before returning: what one
    /// round of the refresher does, for a test that needs them current.
    #[cfg(test)]
    pub(super) async fn refresh_all(&self, now: i64) {
        for window in HealthWindow::ALL {
            self.speed_refreshed(window).await;
            self.refresh(window, now).await;
        }
    }

    /// Holds every window's speed read lock until the guards drop: a test's
    /// way to show what does not wait on them.
    #[cfg(test)]
    async fn hold_speed_caches(&self) -> [tokio::sync::MutexGuard<'_, ()>; 3] {
        [
            self.speed.hour.reading.lock().await,
            self.speed.day.reading.lock().await,
            self.speed.week.reading.lock().await,
        ]
    }
}

#[async_trait]
impl FleetHealthSource for DbFleetHealthSource {
    async fn windowed(&self, window: HealthWindow, now: i64) -> anyhow::Result<WindowedHealth> {
        let speed = self.speed(window, now).await?;
        window::windowed_health(&self.db, window, now, speed.as_ref().clone()).await
    }

    async fn live(&self, now: i64) -> anyhow::Result<LiveHealth> {
        let (fleet, hosts, concerns) = collect_live(&self.db, now).await?;
        Ok(LiveHealth {
            fleet,
            hosts,
            concerns,
        })
    }
}

#[cfg(test)]
mod tests {
    //! #204 item 1: the source's answers, its 5-minute speed cache, and
    //! #240's rules for it: it holds no lock across the live read, and gives
    //! the same answers whether a window is asked alone or with the others
    //! at the same moment.

    use std::{path::PathBuf, sync::Arc, time::Duration};

    use anyhow::Result;
    use gridops_core::{
        connect_database_path,
        fleet_health::{
            FleetHealthSource, HealthWindow,
            testdata::{self, JobSeed, SampleSeed, SeededFleet},
        },
    };
    use sqlx::SqlitePool;

    use super::{DbFleetHealthSource, SpeedReader};
    use gridops_core::fleet_health::speed;

    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 3_600_000;
    /// Ten minutes past a whole hour: the hour in progress and the one in
    /// its grace are both live.
    const NOW: i64 = 1_800_000_000_000 + 10 * MINUTE;

    struct Harness {
        directory: PathBuf,
        db: SqlitePool,
        fleet: SeededFleet,
    }

    impl Harness {
        /// Three machines run the same keys for a day, callisto at twice the
        /// others' durations, with a sample a minute on every machine.
        async fn new() -> Result<Self> {
            let directory = std::env::temp_dir()
                .join(format!("gridops-health-source-{}", uuid::Uuid::new_v4()));
            let db = connect_database_path(&directory.join("gridops.sqlite")).await?;
            let fleet = testdata::seed_fleet(&db).await?;
            let harness = Self {
                directory,
                db,
                fleet,
            };
            harness.jobs(1, NOW - 20 * HOUR, 1).await?;
            for host in &harness.fleet.hosts {
                let samples: Vec<SampleSeed> = (0..24 * 60)
                    .map(|minute| SampleSeed::at(NOW - 24 * HOUR + minute * MINUTE))
                    .collect();
                testdata::insert_samples(&harness.db, host, &samples).await?;
            }
            Ok(harness)
        }

        /// Six runs of each of three keys on io, europa and callisto, from
        /// `from` on, callisto taking `slow` times twice as long.
        async fn jobs(&self, first_id: i64, from: i64, slow: i64) -> Result<()> {
            let mut jobs = Vec::new();
            let mut id = first_id;
            for (host, factor) in [("io", 1), ("europa", 1), ("callisto", 2 * slow)] {
                let host = self.fleet.host(host)?.id.clone();
                for name in ["unit tests", "lint", "build image"] {
                    for run in 0..6 {
                        let completed = from + (id - first_id) * MINUTE;
                        let duration = (60 + run) * 1_000 * factor;
                        jobs.push(JobSeed::completed(
                            id,
                            name,
                            Some(host.as_str()),
                            completed - duration - 30_000,
                            completed - duration,
                            completed,
                        ));
                        id += 1;
                    }
                }
            }
            testdata::insert_jobs(&self.db, &self.fleet, &jobs).await
        }

        async fn finish(self) -> Result<()> {
            self.db.close().await;
            std::fs::remove_dir_all(self.directory)?;
            Ok(())
        }
    }

    fn callisto_index(health: &gridops_core::fleet_health::WindowedHealth) -> Option<f64> {
        health
            .hosts
            .iter()
            .find(|host| host.name == "callisto")
            .and_then(|host| host.speed.first())
            .and_then(|speed| speed.index)
    }

    /// The source reads the database: machines in id order with their
    /// budgets, the windowed scores, and the live half from `collect_live`.
    #[tokio::test]
    async fn it_answers_from_the_database() -> Result<()> {
        let harness = Harness::new().await?;
        let source = DbFleetHealthSource::new(harness.db.clone());
        let windowed = source.windowed(HealthWindow::Day, NOW).await?;
        let ids: Vec<&str> = windowed.hosts.iter().map(|host| host.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "machines in id order");
        assert_eq!(windowed.hosts.len(), harness.fleet.hosts.len());
        let index = callisto_index(&windowed).ok_or_else(|| anyhow::anyhow!("no index"))?;
        assert!(index > 1.3, "callisto is slow: {index}");
        assert!(windowed.fleet.score.is_some());
        let live = source.live(NOW).await?;
        assert_eq!(live.hosts.len(), harness.fleet.hosts.len());
        assert!(live.hosts.iter().all(|host| host.online));
        harness.finish().await
    }

    /// A clock the refresher reads, moved by the test.
    #[derive(Debug)]
    struct Clock(std::sync::atomic::AtomicI64);

    impl gridops_core::fleet::registry::RegistryClock for Clock {
        fn now_millis(&self) -> i64 {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    /// Polls `done` every 10 ms for up to 10 s.
    async fn eventually(what: &str, mut done: impl FnMut() -> bool) -> Result<()> {
        for _ in 0..1_000 {
            if done() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        anyhow::bail!("not within 10 s: {what}")
    }

    /// The review of #204: a window's last speed indexes are served however
    /// old they are, so a window nobody asked for in 10 minutes isn't cold
    /// again; one over 5 minutes old (or from after `now`) is refreshed
    /// behind the call. Only a window with nothing cached reads inline.
    #[tokio::test]
    async fn the_last_speed_indexes_are_served_whatever_their_age() -> Result<()> {
        let harness = Harness::new().await?;
        let source = DbFleetHealthSource::new(harness.db.clone());
        let before = callisto_index(&source.windowed(HealthWindow::Day, NOW).await?);
        assert_eq!(source.inline_speed_reads(), 1, "nothing cached: inline");
        // callisto gets twice as slow again.
        harness.jobs(1_000, NOW - 2 * HOUR, 2).await?;

        let late = callisto_index(
            &source
                .windowed(HealthWindow::Day, NOW + 11 * MINUTE)
                .await?,
        );
        assert_eq!(late, before, "11 minutes on, still the cached indexes");
        assert_eq!(source.inline_speed_reads(), 1, "no inline read");
        source.speed_refreshed(HealthWindow::Day).await;
        let refreshed = callisto_index(
            &source
                .windowed(HealthWindow::Day, NOW + 11 * MINUTE)
                .await?,
        );
        assert_ne!(refreshed, before, "refreshed behind the call");

        let back = callisto_index(&source.windowed(HealthWindow::Day, NOW - 5 * MINUTE).await?);
        assert_eq!(
            back, refreshed,
            "a clock moved back is served the cache too"
        );
        assert_eq!(source.inline_speed_reads(), 1);

        source.windowed(HealthWindow::Week, NOW).await?;
        assert_eq!(source.inline_speed_reads(), 2, "the 7 d window had nothing");
        harness.finish().await
    }

    /// The review of #204: the refresher reads every window on its interval,
    /// so no request ever reads inline once it has run, however long a window
    /// goes unasked.
    #[tokio::test]
    async fn the_refresher_keeps_every_window_fresh() -> Result<()> {
        let harness = Harness::new().await?;
        let source = DbFleetHealthSource::new(harness.db.clone());
        let clock = Arc::new(Clock(std::sync::atomic::AtomicI64::new(NOW)));
        let refresher = source.start_refresher(clock.clone(), Duration::from_millis(50));
        eventually("every window cached", || {
            HealthWindow::ALL
                .iter()
                .all(|window| source.cached_speed_at(*window).is_some())
        })
        .await?;
        let before = callisto_index(&source.windowed(HealthWindow::Day, NOW).await?);
        harness.jobs(1_000, NOW - 2 * HOUR, 2).await?;
        clock
            .0
            .store(NOW + 20 * MINUTE, std::sync::atomic::Ordering::SeqCst);
        eventually("the day refreshed at the new time", || {
            source.cached_speed_at(HealthWindow::Day) == Some(NOW + 20 * MINUTE)
        })
        .await?;
        let after = callisto_index(
            &source
                .windowed(HealthWindow::Day, NOW + 20 * MINUTE)
                .await?,
        );
        assert_ne!(after, before);
        for window in HealthWindow::ALL {
            source.windowed(window, NOW + 20 * MINUTE).await?;
        }
        assert_eq!(source.inline_speed_reads(), 0, "no request read inline");
        refresher.abort();
        harness.finish().await
    }

    /// The review of #204: a refresh that panics clears its flag (a drop
    /// guard), so the next one isn't skipped forever; and the refresher
    /// survives it.
    #[tokio::test]
    async fn a_panicking_refresh_clears_its_flag_and_the_refresher_goes_on() -> Result<()> {
        let harness = Harness::new().await?;
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reader: SpeedReader = {
            let calls = Arc::clone(&calls);
            Arc::new(move |db, window, now| {
                let first = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
                Box::pin(async move {
                    assert!(!first, "the first speed read panics");
                    speed::speed_indexes(&db, window, now).await
                })
            })
        };
        let source = DbFleetHealthSource::new(harness.db.clone()).with_speed_reader(reader);
        source.refresh_behind(HealthWindow::Day, NOW);
        eventually("the panicked refresh's flag cleared", || {
            calls.load(std::sync::atomic::Ordering::SeqCst) == 1
                && !source.is_refreshing(HealthWindow::Day)
        })
        .await?;
        let clock = Arc::new(Clock(std::sync::atomic::AtomicI64::new(NOW)));
        let refresher = source.start_refresher(clock, Duration::from_millis(20));
        eventually("every window cached after the panic", || {
            HealthWindow::ALL
                .iter()
                .all(|window| source.cached_speed_at(*window).is_some())
        })
        .await?;
        refresher.abort();
        harness.finish().await
    }

    /// #240: the live read takes no lock of the source's, so it answers
    /// while every window's speed cache is held.
    #[tokio::test]
    async fn the_live_read_holds_no_lock_of_the_source() -> Result<()> {
        let harness = Harness::new().await?;
        let source = DbFleetHealthSource::new(harness.db.clone());
        let _held = source.hold_speed_caches().await;
        let live = tokio::time::timeout(Duration::from_secs(5), source.live(NOW))
            .await
            .map_err(|_| anyhow::anyhow!("the live read waited on the speed cache"))??;
        assert_eq!(live.hosts.len(), harness.fleet.hosts.len());
        harness.finish().await
    }

    /// #240: the windows asked at the same moment answer exactly as each
    /// asked alone, and a live read alongside them is the live read alone.
    #[tokio::test]
    async fn the_windows_at_once_answer_as_each_alone() -> Result<()> {
        let harness = Harness::new().await?;
        let alone = DbFleetHealthSource::new(harness.db.clone());
        let mut expected = Vec::new();
        for window in HealthWindow::ALL {
            expected.push(alone.windowed(window, NOW).await?);
        }
        let expected_live = alone.live(NOW).await?;

        let together = Arc::new(DbFleetHealthSource::new(harness.db.clone()));
        let calls = HealthWindow::ALL.map(|window| {
            let source = Arc::clone(&together);
            tokio::spawn(async move { source.windowed(window, NOW).await })
        });
        let live = {
            let source = Arc::clone(&together);
            tokio::spawn(async move { source.live(NOW).await })
        };
        for (call, expected) in calls.into_iter().zip(expected) {
            assert_eq!(call.await??, expected);
        }
        assert_eq!(live.await??, expected_live);
        harness.finish().await
    }
}
