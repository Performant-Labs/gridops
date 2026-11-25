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
//! data, against the page's 300 ms), so a window's indexes are served from
//! its cache: under [`SPEED_TTL_MILLIS`] old as they are, and up to
//! [`SPEED_STALE_MILLIS`] old while one background read refreshes them for
//! the next call. Only a window with none, or one older than that, waits for
//! the read; [`DbFleetHealthSource::warm`] fills every window when the API
//! starts. One read per window runs at a time; it holds only that window's
//! read lock, which no other window and no live read takes.

use std::sync::{
    Arc, Mutex as SyncMutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use gridops_core::fleet_health::{
    FleetHealthSource, HealthWindow, LiveHealth, SpeedResult, WindowedHealth, speed, window,
};
use sqlx::SqlitePool;
use tokio::sync::Mutex;

use super::health_live::collect_live;

/// How long a window's speed indexes are served without reading them again
/// (the epic's "cached for 5 minutes").
const SPEED_TTL_MILLIS: i64 = 5 * 60 * 1_000;

/// How long past that they are still served while a background read
/// refreshes them; older ones are read before answering.
const SPEED_STALE_MILLIS: i64 = 10 * 60 * 1_000;

/// A window's speed indexes and the time they were read as of.
type CachedSpeed = (i64, Arc<Vec<SpeedResult>>);

/// One window's speed cache.
#[derive(Debug, Default)]
struct SpeedCache {
    /// Only locked to read or write it, never across an await.
    entry: SyncMutex<Option<CachedSpeed>>,
    /// Held by whoever reads the window's indexes, so one read runs at a
    /// time and a caller that waited on it finds the result.
    reading: Mutex<()>,
    /// A background refresh is under way.
    refreshing: AtomicBool,
}

impl SpeedCache {
    /// The cached indexes at `now` and their age, while they may be served
    /// (not over [`SPEED_STALE_MILLIS`] old, nor from after `now`).
    fn servable(&self, now: i64) -> Option<(i64, Arc<Vec<SpeedResult>>)> {
        let entry = self.entry.lock().unwrap_or_else(PoisonError::into_inner);
        entry.as_ref().and_then(|(at, results)| {
            let age = now - at;
            (0..SPEED_STALE_MILLIS)
                .contains(&age)
                .then(|| (age, Arc::clone(results)))
        })
    }

    /// Keeps `results`, read as of `at`, unless the cache holds a later read.
    fn store(&self, at: i64, results: Arc<Vec<SpeedResult>>) {
        let mut entry = self.entry.lock().unwrap_or_else(PoisonError::into_inner);
        if entry.as_ref().is_none_or(|(kept, _)| *kept <= at) {
            *entry = Some((at, results));
        }
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
#[derive(Debug, Clone)]
pub(crate) struct DbFleetHealthSource {
    db: SqlitePool,
    speed: Arc<SpeedCaches>,
}

impl DbFleetHealthSource {
    pub(crate) fn new(db: SqlitePool) -> Self {
        Self {
            db,
            speed: Arc::default(),
        }
    }

    /// Reads every window's speed indexes as of `now` in the background,
    /// so the first page after the API starts doesn't wait on them. Needs a
    /// Tokio runtime; without one it does nothing.
    pub(crate) fn warm(&self, now: i64) {
        for window in HealthWindow::ALL {
            if self.speed.window(window).servable(now).is_none() {
                self.refresh_behind(window, now);
            }
        }
    }

    /// `window`'s speed indexes at `now`: the cached ones while they may be
    /// served (starting a background refresh once they are over
    /// [`SPEED_TTL_MILLIS`] old), else read before answering.
    async fn speed(&self, window: HealthWindow, now: i64) -> anyhow::Result<Arc<Vec<SpeedResult>>> {
        let cache = self.speed.window(window);
        if let Some((age, results)) = cache.servable(now) {
            if age >= SPEED_TTL_MILLIS {
                self.refresh_behind(window, now);
            }
            return Ok(results);
        }
        let _reading = cache.reading.lock().await;
        if let Some((_, results)) = cache.servable(now) {
            return Ok(results);
        }
        let results = Arc::new(speed::speed_indexes(&self.db, window, now).await?);
        cache.store(now, Arc::clone(&results));
        Ok(results)
    }

    /// Starts one background read of `window`'s indexes as of `now`, unless
    /// one is under way. A failure is logged; the next call tries again.
    fn refresh_behind(&self, window: HealthWindow, now: i64) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self
            .speed
            .window(window)
            .refreshing
            .swap(true, Ordering::SeqCst)
        {
            return;
        }
        let (db, caches) = (self.db.clone(), Arc::clone(&self.speed));
        runtime.spawn(async move {
            let cache = caches.window(window);
            {
                let _reading = cache.reading.lock().await;
                match speed::speed_indexes(&db, window, now).await {
                    Ok(results) => cache.store(now, Arc::new(results)),
                    Err(error) => {
                        tracing::warn!(error = ?error, %window, "fleet health speed index refresh failed");
                    }
                }
            }
            cache.refreshing.store(false, Ordering::SeqCst);
        });
    }

    /// Waits for `window`'s background refresh, if one is under way.
    #[cfg(test)]
    pub(super) async fn speed_refreshed(&self, window: HealthWindow) {
        let cache = self.speed.window(window);
        while cache.refreshing.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
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

    use super::DbFleetHealthSource;

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

    /// The speed indexes are cached per window: under 5 minutes old they are
    /// served as they are; from 5 to 10 minutes old they are still served,
    /// and read again in the background for the next call, so a page refresh
    /// never waits on the 7 d index (about 0.4 s on production's data); with
    /// none, or one over 10 minutes old, the call reads them itself. A
    /// window's cache is its own.
    #[tokio::test]
    async fn the_speed_indexes_are_served_from_cache_and_refreshed_behind() -> Result<()> {
        let harness = Harness::new().await?;
        let source = DbFleetHealthSource::new(harness.db.clone());
        let before = callisto_index(&source.windowed(HealthWindow::Day, NOW).await?);
        // callisto gets twice as slow again.
        harness.jobs(1_000, NOW - 2 * HOUR, 2).await?;
        let cached = callisto_index(&source.windowed(HealthWindow::Day, NOW + 4 * MINUTE).await?);
        assert_eq!(cached, before, "cached within 5 minutes");
        let other = callisto_index(
            &source
                .windowed(HealthWindow::Week, NOW + 4 * MINUTE)
                .await?,
        );
        assert_ne!(other, before, "the 7 d window computes its own");

        // 6 minutes on: still the cached indexes, while they are read again
        // behind the call.
        let stale = callisto_index(&source.windowed(HealthWindow::Day, NOW + 6 * MINUTE).await?);
        assert_eq!(stale, before, "served while it is refreshed");
        source.speed_refreshed(HealthWindow::Day).await;
        let refreshed =
            callisto_index(&source.windowed(HealthWindow::Day, NOW + 6 * MINUTE).await?);
        assert_ne!(refreshed, before, "the refresh read the new jobs");

        // Over 10 minutes old, or from a later time than the call (a clock
        // that moved back), the call reads them itself.
        let fresh = DbFleetHealthSource::new(harness.db.clone());
        let first = callisto_index(&fresh.windowed(HealthWindow::Day, NOW).await?);
        assert_eq!(first, refreshed);
        harness.jobs(2_000, NOW - HOUR, 3).await?;
        let late = callisto_index(&fresh.windowed(HealthWindow::Day, NOW + 11 * MINUTE).await?);
        assert_ne!(late, first, "read inline past 10 minutes");
        let back = callisto_index(&fresh.windowed(HealthWindow::Day, NOW + 5 * MINUTE).await?);
        assert_eq!(
            back, late,
            "recomputed at the earlier time from the same jobs"
        );
        harness.finish().await
    }

    /// Warming reads every window's speed indexes in the background, so the
    /// first page after the API starts doesn't wait on them.
    #[tokio::test]
    async fn warming_fills_every_windows_speed_cache() -> Result<()> {
        let harness = Harness::new().await?;
        let source = DbFleetHealthSource::new(harness.db.clone());
        source.warm(NOW);
        for window in HealthWindow::ALL {
            source.speed_refreshed(window).await;
            assert!(source.cached_speed_at(window).is_some(), "{window}");
        }
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
