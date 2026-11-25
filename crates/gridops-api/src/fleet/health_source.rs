//! The database source of the fleet health page (#204): the windowed half
//! from `gridops_core::fleet_health::window` (the hourly rollups for the
//! hours past their grace, the later hours computed live, the scores and
//! windowed concerns, the usage by category and the trend from the
//! snapshots) with #193's speed indexes cached 5 minutes per window, and the
//! live half from `health_live::collect_live`.
//!
//! #240's rules hold here too: the live read takes no lock of the source's
//! (it reads the database and returns, and nothing of it is kept), and the
//! answers don't depend on how many windows are asked at once. A window's
//! speed cache is locked only by a computation of that window, while it
//! reads or refreshes the indexes, so concurrent misses of one window share
//! one read and other windows never wait on it.

use std::sync::Arc;

use async_trait::async_trait;
use gridops_core::fleet_health::{
    FleetHealthSource, HealthWindow, LiveHealth, SpeedResult, WindowedHealth, speed, window,
};
use sqlx::SqlitePool;
use tokio::sync::Mutex;

use super::health_live::collect_live;

/// How long a window's speed indexes are served without reading them again
/// (the epic's "cached for 5 minutes": the speed index reads raw jobs).
const SPEED_TTL_MILLIS: i64 = 5 * 60 * 1_000;

/// A window's speed indexes and when they were read.
type CachedSpeed = Option<(i64, Arc<Vec<SpeedResult>>)>;

/// The speed cache of each window, each behind its own lock.
#[derive(Debug, Default)]
struct SpeedCaches {
    hour: Mutex<CachedSpeed>,
    day: Mutex<CachedSpeed>,
    week: Mutex<CachedSpeed>,
}

impl SpeedCaches {
    const fn window(&self, window: HealthWindow) -> &Mutex<CachedSpeed> {
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

    /// `window`'s speed indexes at `now`: the cached ones while under 5
    /// minutes old, else read again. A cache from a later time than `now`
    /// (a clock moved back) is read again too.
    async fn speed(&self, window: HealthWindow, now: i64) -> anyhow::Result<Arc<Vec<SpeedResult>>> {
        let mut cache = self.speed.window(window).lock().await;
        if let Some((at, results)) = cache.as_ref() {
            if (0..SPEED_TTL_MILLIS).contains(&(now - at)) {
                return Ok(Arc::clone(results));
            }
        }
        let results = Arc::new(speed::speed_indexes(&self.db, window, now).await?);
        *cache = Some((now, Arc::clone(&results)));
        Ok(results)
    }

    /// Holds every window's speed cache until the guards drop: a test's way
    /// to show what does not wait on them.
    #[cfg(test)]
    async fn hold_speed_caches(&self) -> [tokio::sync::MutexGuard<'_, CachedSpeed>; 3] {
        [
            self.speed.hour.lock().await,
            self.speed.day.lock().await,
            self.speed.week.lock().await,
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

    /// The speed indexes are cached 5 minutes per window: jobs that would
    /// move an index don't until the cache expires, and a window's cache
    /// is its own.
    #[tokio::test]
    async fn the_speed_indexes_are_cached_for_5_minutes_per_window() -> Result<()> {
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
        let fresh = callisto_index(&source.windowed(HealthWindow::Day, NOW + 5 * MINUTE).await?);
        assert_ne!(fresh, before, "read again after 5 minutes");
        // A clock that moves back is not a fresh cache.
        let back = callisto_index(&source.windowed(HealthWindow::Day, NOW).await?);
        assert_eq!(
            back, fresh,
            "recomputed at the earlier time from the same jobs"
        );
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
