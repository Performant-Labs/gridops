//! The relative speed index (#193): how long a machine takes to run the jobs
//! the rest of the fleet also runs, at one runner size, against the fleet's
//! median. 1.0 is typical; 1.5 is half again as slow.
//!
//! The rules (`docs/design/fleet-health.md`): a job key is repository +
//! workflow name + job name with a trailing shard counter stripped; only
//! successful final-attempt runs completed inside the window count; the
//! index is the run-count-weighted geometric mean of (own median ÷ fleet
//! median) over keys with at least 5 runs on the machine at that size and 5
//! on the rest of the fleet; fewer than 3 such keys is `None`.
//!
//! #197 fixes the signatures; #193 implements them.

use std::collections::BTreeMap;

use anyhow::Context as _;
use sqlx::SqlitePool;

use super::{HealthWindow, SpeedKey, SpeedResult};

/// The least successful runs a key needs on the machine at that size, and on
/// the rest of the fleet, before it counts towards an index.
const MIN_RUNS: usize = 5;

/// The least qualifying keys a (machine, runner size) needs for an index at
/// all; fewer is "not enough data" (`index: None`).
const MIN_KEYS: usize = 3;

/// How many of the keys that push an index up most a result carries.
const TOP_KEYS: usize = 5;

/// The key a job's runs are compared under: repository, workflow and job
/// name, the name's trailing shard counter (` (n/m)`, ` [n/m]` or ` n/m`)
/// stripped so the shards of one suite compare together.
#[must_use]
pub fn job_key(repository: &str, workflow: &str, job_name: &str) -> String {
    let name = strip_shard_suffix(job_name);
    format!("{repository} / {workflow} / {name}")
}

/// A job name trimmed, without its trailing shard counter if it has one.
fn strip_shard_suffix(job_name: &str) -> &str {
    let trimmed = job_name.trim();
    match split_shard_counter(trimmed) {
        Some((name, _)) => name.trim(),
        None => trimmed,
    }
}

/// Splits a trimmed job name into the name before its trailing shard counter
/// and the counter itself, one of ` (n/m)`, ` [n/m]` or ` n/m` with `n` and
/// `m` any run of digits. A counter that is not digits over digits
/// (` (a/b)`) or not trailing (`unit 3/4 extra`) is part of the name.
fn split_shard_counter(trimmed: &str) -> Option<(&str, &str)> {
    // What opens the counter: a bracketed one ends the name with its closing
    // bracket, a bare one is separated from the name by a space.
    let opened_by = match trimmed.as_bytes().last()? {
        b')' => b'(',
        b']' => b'[',
        _ => b' ',
    };
    let body = if opened_by == b' ' {
        trimmed
    } else {
        &trimmed[..trimmed.len() - 1]
    };
    let opened_at = body.rfind(opened_by as char)?;
    let counter = &body[opened_at + 1..];
    let (shard, of) = counter.split_once('/')?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    (digits(shard) && digits(of)).then_some((&body[..opened_at], counter))
}

/// Every (machine, runner size) that ran jobs in the window ending at `now`:
/// each one's index is the run-count-weighted geometric mean of its median
/// duration ÷ the fleet median over the keys it shares with the fleet, with
/// the [`TOP_KEYS`] keys that push it up most, or `index: None` (and no keys)
/// with fewer than [`MIN_KEYS`] shared keys: not enough data.
///
/// One query walks the window's successful final-attempt jobs joined to the
/// placements that ran them, keeping every duration, so both medians are
/// true ones, not histogram estimates. It runs once per hour of the window
/// (`completed_at`, the indexed bound): one statement over a whole week's
/// jobs is itself a slow one (394 ms on the synthetic week the contract test
/// runs), against #179's 100 ms rule, which the contract test enforces on
/// exactly this read. Nothing is cached here: #204 caches the result.
///
/// # Errors
/// The read failed, or a row's column did not fit its type.
pub async fn speed_indexes(
    db: &SqlitePool,
    window: HealthWindow,
    now: i64,
) -> anyhow::Result<Vec<SpeedResult>> {
    let start = now - window.millis();

    // Durations per (machine, runner size) per key, and per key over the
    // whole fleet for the fleet medians.
    let mut machines: BTreeMap<(String, u32), BTreeMap<String, Vec<i64>>> = BTreeMap::new();
    let mut fleet: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    let mut slice_start = start;
    while slice_start < now {
        let slice_end = (slice_start + super::HOUR_MILLIS).min(now);
        for (repository, workflow, name, host_id, cpu_millis, durations) in
            grouped_durations(db, slice_start, slice_end).await?
        {
            let key = job_key(&repository, &workflow, &name);
            let cpu_millis = u32::try_from(cpu_millis)
                .context("a placement's cpu_millis is not a runner size")?;
            let durations = parse_durations(&durations)?;
            machines
                .entry((host_id, cpu_millis))
                .or_default()
                .entry(key.clone())
                .or_default()
                .extend(durations.iter().copied());
            fleet.entry(key).or_default().extend(durations);
        }
        slice_start = slice_end;
    }
    for durations in fleet.values_mut() {
        durations.sort_unstable();
    }

    let mut results = Vec::with_capacity(machines.len());
    for ((host_id, cpu_millis), keys) in machines {
        // The keys that qualify: this machine at this size ran them at least
        // `MIN_RUNS` times, and the rest of the fleet did too.
        let mut qualifying: Vec<(f64, SpeedKey)> = Vec::new();
        for (key, durations) in &keys {
            let host_runs = durations.len();
            let fleet_runs = fleet[key].len();
            if host_runs < MIN_RUNS || fleet_runs - host_runs < MIN_RUNS {
                continue;
            }
            let mut sorted = durations.clone();
            sorted.sort_unstable();
            let host_median = median(&sorted);
            let fleet_median = median(&fleet[key]);
            if host_median <= 0 || fleet_median <= 0 {
                // A median of no measurable time — sub-second jobs at GitHub's
                // second granularity, or clock skew — on the machine or the
                // fleet: dividing by it, or logging it, is not an index.
                continue;
            }
            let ratio = host_median as f64 / fleet_median as f64;
            let count = |runs: usize, what: &str| -> anyhow::Result<u32> {
                u32::try_from(runs).with_context(|| format!("too many runs to count: {what}"))
            };
            qualifying.push((
                ratio,
                SpeedKey {
                    key: key.clone(),
                    host_median_ms: host_median,
                    fleet_median_ms: fleet_median,
                    host_runs: count(host_runs, "on the machine")?,
                    fleet_runs: count(fleet_runs, "on the fleet")?,
                },
            ));
        }

        // The weighted geometric mean: average the logs of the ratios,
        // weighted by run count, then exponentiate.
        let index = if qualifying.len() < MIN_KEYS {
            None
        } else {
            let weight = qualifying
                .iter()
                .map(|(_, key)| f64::from(key.host_runs))
                .sum::<f64>();
            let logs = qualifying
                .iter()
                .map(|(ratio, key)| ratio.ln() * f64::from(key.host_runs))
                .sum::<f64>();
            Some((logs / weight).exp())
        };

        // With no index there is nothing for keys to push up; with one, the
        // keys slower than the fleet (a ratio above 1.0: only those push it
        // up, #239 item 10), worst first, ties by key.
        let top_keys = if index.is_none() {
            Vec::new()
        } else {
            qualifying.retain(|(ratio, _)| *ratio > 1.0);
            qualifying.sort_by(|(ratio, key), (other, other_key)| {
                other
                    .total_cmp(ratio)
                    .then_with(|| key.key.cmp(&other_key.key))
            });
            qualifying.truncate(TOP_KEYS);
            qualifying.into_iter().map(|(_, key)| key).collect()
        };
        results.push(SpeedResult {
            host_id,
            cpu_millis,
            index,
            top_keys,
        });
    }
    Ok(results)
}

/// The successful final-attempt jobs completed in `[from, to)`, as one row
/// per (repository, workflow, job name, machine, runner size) with the
/// group's durations comma-joined: the query stays a single pass over the
/// join whatever the window's volume. The name is grouped as stored; the
/// shard suffix is stripped in [`job_key`], in Rust. A job's machine is its
/// runner's newest `runners` row and that row's newest placement, as #191's
/// rollup reads it, so duplicate rows never count a job twice.
///
/// # Errors
/// The read failed.
async fn grouped_durations(
    db: &SqlitePool,
    from: i64,
    to: i64,
) -> anyhow::Result<Vec<(String, String, String, String, i64, String)>> {
    let rows = sqlx::query_as::<_, (String, String, String, String, i64, String)>(
        r#"SELECT repo.full_name, run.workflow_name, job.name, p.host_id, p.cpu_millis,
                  GROUP_CONCAT(job.completed_at - job.started_at) AS durations
             FROM workflow_jobs job
             JOIN workflow_runs run ON run.id = job.run_id
             JOIN repositories repo ON repo.id = run.repository_id
             JOIN runners r ON r.id = (
                    SELECT cand.id FROM runners cand
                     WHERE cand.github_runner_id = job.runner_id
                     ORDER BY cand.created_at DESC, cand.id DESC LIMIT 1)
             JOIN workload_placements p ON p.id = (
                    SELECT latest.id FROM workload_placements latest
                     WHERE latest.runner_id = r.id
                     ORDER BY latest.generation DESC, latest.updated_at DESC, latest.id DESC
                     LIMIT 1)
             LEFT JOIN workflow_job_sync sync ON sync.job_id = job.id
            WHERE job.status = 'completed'
              AND job.conclusion = 'success'
              AND job.completed_at >= ?
              AND job.completed_at < ?
              AND job.started_at IS NOT NULL
              AND (sync.run_attempt IS NULL
                   OR sync.run_attempt >= COALESCE(run.run_attempt, 1))
              AND NOT EXISTS (
                    SELECT 1 FROM workflow_jobs later
                      JOIN workflow_job_sync later_sync ON later_sync.job_id = later.id
                     WHERE later.run_id = job.run_id
                       AND later_sync.run_attempt > sync.run_attempt)
            GROUP BY repo.full_name, run.workflow_name, job.name, p.host_id, p.cpu_millis"#,
    )
    .bind(from)
    .bind(to)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// The comma-joined durations of one group, as `GROUP_CONCAT` wrote them.
fn parse_durations(joined: &str) -> anyhow::Result<Vec<i64>> {
    joined
        .split(',')
        .map(|duration| {
            duration
                .parse::<i64>()
                .with_context(|| format!("a duration that is not a number: {duration:?}"))
        })
        .collect()
}

/// The median of `durations`, which must be sorted: the middle value, or the
/// mean of the two middle ones rounded to a whole millisecond.
fn median(sorted: &[i64]) -> i64 {
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[middle]
    } else {
        let (low, high) = (sorted[middle - 1], sorted[middle]);
        ((low + high) as f64 / 2.0).round() as i64
    }
}

#[cfg(test)]
mod tests {
    //! #193's own tests, beyond the contract tests' rules: the median's even
    //! case, the run-count weighting, the top keys' order and cap, the
    //! half-open window, and the final-attempt filter, whose re-runs the
    //! contract tests never seed.

    use std::path::PathBuf;

    use anyhow::Result;
    use sqlx::SqlitePool;

    use super::{HealthWindow, SpeedResult, job_key, median, speed_indexes};
    use crate::connect_database_path;
    use crate::fleet_health::testdata::{self, JobSeed, SeededFleet};

    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 24 * HOUR;
    /// The end of the window every test reads.
    const NOW: i64 = 1_800_000_000_000;

    struct Harness {
        directory: PathBuf,
        db: SqlitePool,
        fleet: SeededFleet,
        next_id: i64,
    }

    impl Harness {
        async fn new() -> Result<Self> {
            let directory =
                std::env::temp_dir().join(format!("gridops-speed-own-{}", uuid::Uuid::new_v4()));
            let db = connect_database_path(&directory.join("db.sqlite")).await?;
            let fleet = testdata::seed_fleet(&db).await?;
            Ok(Self {
                directory,
                db,
                fleet,
                next_id: 1,
            })
        }

        /// `runs` successful runs of `name` on `host` at `cpu_millis`, each
        /// lasting `duration` ms, completing inside the last day.
        async fn runs(
            &mut self,
            host: &str,
            cpu_millis: u32,
            name: &str,
            runs: usize,
            duration: i64,
        ) -> Result<()> {
            self.runs_at(host, cpu_millis, name, runs, duration, NOW - 2 * HOUR)
                .await
        }

        /// As [`Harness::runs`], completing at `completed` (epoch ms).
        async fn runs_at(
            &mut self,
            host: &str,
            cpu_millis: u32,
            name: &str,
            runs: usize,
            duration: i64,
            completed: i64,
        ) -> Result<()> {
            let host_id = self.fleet.host(host)?.id.clone();
            let mut jobs = Vec::new();
            for _ in 0..runs {
                let id = self.next_id;
                self.next_id += 1;
                jobs.push(JobSeed::completed(
                    id,
                    name,
                    Some(&host_id),
                    completed - duration - 5_000,
                    completed - duration,
                    completed,
                ));
            }
            for job in &mut jobs {
                job.cpu_millis = cpu_millis;
            }
            testdata::insert_jobs(&self.db, &self.fleet, &jobs).await
        }

        /// One successful job of `name` on `host` at 2 CPUs, part of run
        /// `run_id` at `attempt`, lasting `duration` ms and completing at
        /// `completed`.
        async fn attempt(
            &mut self,
            host: &str,
            name: &str,
            run_id: i64,
            attempt: i64,
            duration: i64,
            completed: i64,
        ) -> Result<i64> {
            let id = self.next_id;
            self.next_id += 1;
            let host_id = self.fleet.host(host)?.id.clone();
            let mut job = JobSeed::completed(
                id,
                name,
                Some(&host_id),
                completed - duration - 5_000,
                completed - duration,
                completed,
            );
            job.run_id = run_id;
            job.run_attempt = attempt;
            testdata::insert_jobs(&self.db, &self.fleet, std::slice::from_ref(&job)).await?;
            Ok(id)
        }

        async fn indexes(&self) -> Result<Vec<SpeedResult>> {
            speed_indexes(&self.db, HealthWindow::Day, NOW).await
        }

        fn result<'a>(
            &self,
            results: &'a [SpeedResult],
            host: &str,
            cpu_millis: u32,
        ) -> Result<&'a SpeedResult> {
            let id = &self.fleet.host(host)?.id;
            results
                .iter()
                .find(|result| &result.host_id == id && result.cpu_millis == cpu_millis)
                .ok_or_else(|| anyhow::anyhow!("no speed result for {host} at {cpu_millis}"))
        }

        async fn close(self) -> Result<()> {
            self.db.close().await;
            std::fs::remove_dir_all(self.directory)?;
            Ok(())
        }
    }

    #[test]
    fn the_median_is_the_middle_value_or_the_mean_of_the_middle_two() {
        assert_eq!(median(&[3]), 3);
        assert_eq!(median(&[2, 4]), 3);
        assert_eq!(median(&[1, 2, 3, 4, 5]), 3);
        // (3 + 4) / 2 rounds away from zero to a whole millisecond.
        assert_eq!(median(&[1, 2, 3, 4, 5, 6]), 4);
    }

    /// Jupiter leans 40 of its 50 runs on one key at 1.5x, so the index is
    /// exp(40 x ln 1.5 / 50) = 1.383, not the unweighted (1 x 1 x 1.5)^(1/3)
    /// = 1.145.
    #[tokio::test]
    async fn the_index_weights_each_key_by_its_run_count() -> Result<()> {
        let mut harness = Harness::new().await?;
        for (name, duration) in [("unit", 120_000_i64), ("lint", 60_000)] {
            for host in ["io", "europa", "callisto"] {
                harness.runs(host, 2_000, name, 6, duration).await?;
            }
        }
        // The fleet runs "build" 100 times elsewhere, so Jupiter's 40 runs at
        // 1.5x stay the minority its ratio is measured against.
        for (host, runs) in [("io", 34), ("europa", 33), ("callisto", 33)] {
            harness.runs(host, 2_000, "build", runs, 300_000).await?;
        }
        harness.runs("Jupiter", 2_000, "unit", 5, 120_000).await?;
        harness.runs("Jupiter", 2_000, "lint", 5, 60_000).await?;
        harness.runs("Jupiter", 2_000, "build", 40, 450_000).await?;
        let results = harness.indexes().await?;
        let index = harness
            .result(&results, "Jupiter", 2_000)?
            .index
            .ok_or_else(|| anyhow::anyhow!("Jupiter has no index"))?;
        assert!(
            (index - 1.383_2).abs() <= 0.01,
            "Jupiter's index is {index}"
        );
        harness.close().await
    }

    #[tokio::test]
    async fn top_keys_are_the_five_worst_and_none_without_an_index() -> Result<()> {
        let mut harness = Harness::new().await?;
        let keys = [
            ("unit", 120_000_i64, 1.1_f64),
            ("lint", 60_000, 1.1),
            ("clippy", 240_000, 1.0),
            ("build", 300_000, 1.4),
            ("e2e", 600_000, 1.5),
            ("deploy", 180_000, 1.6),
        ];
        for (name, typical, factor) in keys {
            for host in ["io", "europa", "callisto"] {
                harness.runs(host, 2_000, name, 6, typical).await?;
            }
            harness
                .runs("Jupiter", 2_000, name, 6, (typical as f64 * factor) as i64)
                .await?;
        }
        // europa at 4 CPUs shares only two keys with the fleet (which runs
        // neither of them anywhere else).
        harness.runs("europa", 4_000, "docs", 6, 30_000).await?;
        harness.runs("europa", 4_000, "rare", 6, 30_000).await?;
        let results = harness.indexes().await?;
        let slow = harness.result(&results, "Jupiter", 2_000)?;
        let index = slow
            .index
            .ok_or_else(|| anyhow::anyhow!("Jupiter has no index"))?;
        // The geometric mean of the six ratios, all keys equal in weight.
        assert!(
            (index - 1.263_4).abs() <= 0.01,
            "Jupiter's index is {index}"
        );
        // Worst first, the 1.1x tie broken by key, the 1.0x key truncated out.
        let expected = ["deploy", "e2e", "build", "lint", "unit"];
        let top_keys = slow
            .top_keys
            .iter()
            .map(|key| key.key.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            top_keys,
            expected
                .iter()
                .map(|name| job_key("acme/app", "CI", name))
                .collect::<Vec<_>>(),
            "at most five keys, of six run, worst first"
        );
        for (key, name) in slow.top_keys.iter().zip(expected) {
            let &(_, typical, factor) = keys
                .iter()
                .find(|(seed, _, _)| *seed == name)
                .ok_or_else(|| anyhow::anyhow!("no seed for {name}"))?;
            assert_eq!(
                key.host_median_ms,
                (typical as f64 * factor) as i64,
                "{name}"
            );
            assert_eq!(key.fleet_median_ms, typical, "{name}");
            assert_eq!(key.host_runs, 6);
            assert_eq!(key.fleet_runs, 24);
        }
        let few = harness.result(&results, "europa", 4_000)?;
        assert_eq!(few.index, None, "two keys give no index");
        assert!(few.top_keys.is_empty(), "nothing pushes up no index");
        harness.close().await
    }

    #[tokio::test]
    async fn the_window_is_half_open_on_the_indexed_completion_time() -> Result<()> {
        let mut harness = Harness::new().await?;
        for (name, duration) in [
            ("unit", 120_000_i64),
            ("lint", 60_000),
            ("e2e", 600_000),
            ("build", 300_000),
            ("instant", 0),
        ] {
            for host in ["io", "europa", "callisto"] {
                harness.runs(host, 2_000, name, 6, duration).await?;
            }
        }
        // "unit" completes exactly at the window's start: inside. "lint"
        // exactly at its end: outside, at twice the fleet's median.
        harness
            .runs_at("Jupiter", 2_000, "unit", 6, 120_000, NOW - DAY)
            .await?;
        harness
            .runs_at("Jupiter", 2_000, "lint", 6, 120_000, NOW)
            .await?;
        harness.runs("Jupiter", 2_000, "e2e", 6, 600_000).await?;
        harness.runs("Jupiter", 2_000, "build", 6, 300_000).await?;
        // "flash" records no time on Jupiter alone, "instant" nowhere: a
        // median of no measurable time says nothing about speed, whichever
        // side of the division it is on.
        harness.runs("io", 2_000, "flash", 6, 40_000).await?;
        harness.runs("europa", 2_000, "flash", 6, 40_000).await?;
        harness.runs("callisto", 2_000, "flash", 6, 40_000).await?;
        harness.runs("Jupiter", 2_000, "flash", 6, 0).await?;
        harness.runs("Jupiter", 2_000, "instant", 6, 0).await?;
        let results = harness.indexes().await?;
        let index = harness
            .result(&results, "Jupiter", 2_000)?
            .index
            .ok_or_else(|| anyhow::anyhow!("Jupiter has no index"))?;
        assert!(
            (index - 1.0).abs() <= 0.01,
            "the boundary or the timeless key leaked in: {index}"
        );
        harness.close().await
    }

    /// Every key of Jupiter's runs at twice the fleet's median, so the index
    /// is exactly 2.0 — and every way an earlier attempt could leak back in
    /// turns one key's median into the mean of its two durations, 1.5.
    #[tokio::test]
    async fn only_the_final_attempt_of_a_re_run_counts() -> Result<()> {
        let mut harness = Harness::new().await?;
        for (name, duration) in [("unit", 120_000_i64), ("lint", 60_000), ("e2e", 600_000)] {
            for host in ["io", "europa", "callisto"] {
                harness.runs(host, 2_000, name, 6, duration).await?;
            }
        }
        // Five re-runs of "unit" (runs 700-704), their run rows still at
        // attempt 1: only the later job's own attempt rules the attempt 1
        // jobs out.
        for run in 700..705 {
            harness
                .attempt("Jupiter", "unit", run, 1, 120_000, NOW - 2 * HOUR)
                .await?;
            harness
                .attempt("Jupiter", "unit", run, 2, 240_000, NOW - 2 * HOUR)
                .await?;
        }
        sqlx::query("UPDATE workflow_runs SET run_attempt=1 WHERE id BETWEEN 700 AND 704")
            .execute(&harness.db)
            .await?;
        // Five plain "lint" runs at 2x, plus five attempt 1 jobs of runs whose
        // row knows attempt 2 (900-904) but whose attempt 2 jobs are not
        // stored: the run row alone rules them out.
        harness.runs("Jupiter", 2_000, "lint", 5, 120_000).await?;
        for run in 900..905 {
            harness
                .attempt("Jupiter", "lint", run, 1, 60_000, NOW - 2 * HOUR)
                .await?;
        }
        sqlx::query("UPDATE workflow_runs SET run_attempt=2 WHERE id BETWEEN 900 AND 904")
            .execute(&harness.db)
            .await?;
        // "e2e" at 2x, its fifth run with an attempt not known: unknown is not
        // earlier, so it counts, and without it the key misses the minimum.
        harness.runs("Jupiter", 2_000, "e2e", 4, 1_200_000).await?;
        harness.runs("Jupiter", 2_000, "e2e", 1, 1_200_000).await?;
        sqlx::query("UPDATE workflow_job_sync SET run_attempt=NULL WHERE job_id=?")
            .bind(harness.next_id - 1)
            .execute(&harness.db)
            .await?;
        let results = harness.indexes().await?;
        let index = harness
            .result(&results, "Jupiter", 2_000)?
            .index
            .ok_or_else(|| anyhow::anyhow!("Jupiter has no index"))?;
        assert!(
            (index - 2.0).abs() <= 0.01,
            "an earlier attempt ruled in, or an unknown one out: {index}"
        );
        harness.close().await
    }
    /// Item 10: `topKeys` are the keys that push an index up, so only keys
    /// slower than the fleet (a ratio above 1.0) are listed. Jupiter's index
    /// comes from three keys at 1.2x, 1.0x and 0.8x; only the 1.2x one pushes
    /// it up.
    #[tokio::test]
    async fn top_keys_hold_only_keys_slower_than_the_fleet() -> Result<()> {
        let mut harness = Harness::new().await?;
        for (name, typical, factor) in [
            ("unit", 120_000_i64, 1.2_f64),
            ("lint", 60_000, 1.0),
            ("clippy", 240_000, 0.8),
        ] {
            for host in ["io", "europa", "callisto"] {
                harness.runs(host, 2_000, name, 6, typical).await?;
            }
            harness
                .runs("Jupiter", 2_000, name, 6, (typical as f64 * factor) as i64)
                .await?;
        }
        let results = harness.indexes().await?;
        let jupiter = harness.result(&results, "Jupiter", 2_000)?;
        assert!(jupiter.index.is_some(), "three keys give an index");
        let top_keys: Vec<String> = jupiter.top_keys.iter().map(|key| key.key.clone()).collect();
        assert_eq!(top_keys, [job_key("acme/app", "CI", "unit")]);
        harness.close().await
    }

    /// Re-registers the runner of every `name` job on `host_id`: a second,
    /// older `runners` row with the same `github_runner_id` (0013 dropped the
    /// unique index), with its own placement on the same machine.
    async fn older_runner_rows(db: &SqlitePool, host_id: &str, name: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO runners (id,pool_id,github_runner_id,name,status,busy,ephemeral,
                                  registered_at,created_at,updated_at,ci_platform)
             SELECT r.id||'-old',r.pool_id,r.github_runner_id,r.name,r.status,r.busy,r.ephemeral,
                    r.registered_at,r.created_at-60000,r.updated_at,r.ci_platform
               FROM runners r JOIN workload_placements p ON p.runner_id=r.id
              WHERE p.host_id=? AND length(p.id)=36 AND p.runner_id IN (
                    SELECT r.id FROM runners r
                      JOIN workflow_jobs j ON j.runner_id=r.github_runner_id
                     WHERE j.name=?)",
        )
        .bind(host_id)
        .bind(name)
        .execute(db)
        .await?;
        copy_placements(db, (host_id, name), "-old", "-old", 0, "-old").await
    }

    /// Gives the runner of every `name` job on `host_id` a second, newer
    /// placement (generation 2) on the same machine:
    /// `workload_placements_runner_idx` is not unique.
    async fn newer_placements(db: &SqlitePool, host_id: &str, name: &str) -> Result<()> {
        copy_placements(db, (host_id, name), "-g2", "", 1, "").await
    }

    /// Copies the placements of the `jobs` (machine id, job name), with their
    /// operations and intents. Only seeded placements are copied (a seeded
    /// id is 36 characters; a copy's carries a suffix): ids suffixed
    /// `suffix`, the workload id
    /// suffixed `workload`, the generation raised by `generation_step`, and
    /// the runner id suffixed `runner`.
    async fn copy_placements(
        db: &SqlitePool,
        jobs: (&str, &str),
        suffix: &str,
        workload: &str,
        generation_step: i64,
        runner: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO fleet_operations (id,kind,principal_kind,principal_scope,idempotency_key,
                                           request_hash,status,created_at,updated_at)
             SELECT o.id||?,o.kind,o.principal_kind,o.principal_scope,o.idempotency_key||?,
                    o.request_hash,o.status,o.created_at,o.updated_at
               FROM fleet_operations o JOIN workload_placements p ON p.intent_id=o.id
              WHERE p.host_id=? AND length(p.id)=36 AND p.runner_id IN (
                    SELECT r.id FROM runners r
                      JOIN workflow_jobs j ON j.runner_id=r.github_runner_id
                     WHERE j.name=?)",
        )
        .bind(suffix)
        .bind(suffix)
        .bind(jobs.0)
        .bind(jobs.1)
        .execute(db)
        .await?;
        sqlx::query(
            "INSERT INTO workload_intents (id,workload_id,generation,kind,pool_id,profile_id,
                                           target_id,expected_profile_revision,status,
                                           created_at,updated_at)
             SELECT i.id||?,i.workload_id||?,i.generation+?,i.kind,i.pool_id,i.profile_id,
                    i.target_id,i.expected_profile_revision,i.status,i.created_at,i.updated_at
               FROM workload_intents i JOIN workload_placements p ON p.intent_id=i.id
              WHERE p.host_id=? AND length(p.id)=36 AND p.runner_id IN (
                    SELECT r.id FROM runners r
                      JOIN workflow_jobs j ON j.runner_id=r.github_runner_id
                     WHERE j.name=?)",
        )
        .bind(suffix)
        .bind(workload)
        .bind(generation_step)
        .bind(jobs.0)
        .bind(jobs.1)
        .execute(db)
        .await?;
        sqlx::query(
            "INSERT INTO workload_placements (id,intent_id,workload_id,generation,host_id,
                    backend_id,host_epoch,profile_revision,target_id,config_snapshot_json,
                    cpu_millis,memory_mib,disk_bytes,state,runner_id,created_at,updated_at)
             SELECT p.id||?,p.intent_id||?,p.workload_id||?,p.generation+?,p.host_id,
                    p.backend_id,p.host_epoch,p.profile_revision,p.target_id,
                    p.config_snapshot_json,p.cpu_millis,p.memory_mib,p.disk_bytes,p.state,
                    p.runner_id||?,p.created_at,p.updated_at+1
               FROM workload_placements p
              WHERE p.host_id=? AND length(p.id)=36 AND p.runner_id IN (
                    SELECT r.id FROM runners r
                      JOIN workflow_jobs j ON j.runner_id=r.github_runner_id
                     WHERE j.name=?)",
        )
        .bind(suffix)
        .bind(suffix)
        .bind(workload)
        .bind(generation_step)
        .bind(runner)
        .bind(jobs.0)
        .bind(jobs.1)
        .execute(db)
        .await?;
        Ok(())
    }

    /// Item 11: a job is counted once, on its newest runner row's newest
    /// placement, as #191's rollup counts it. Jupiter's "unit" runners were
    /// re-registered (two `runners` rows per runner id) and its "lint"
    /// runners have two placements each; read naively, each of those jobs
    /// counts twice, on the machine and in the fleet.
    #[tokio::test]
    async fn a_job_counts_once_whatever_duplicate_runner_or_placement_rows_it_has() -> Result<()> {
        let mut harness = Harness::new().await?;
        for (name, duration) in [("unit", 120_000_i64), ("lint", 60_000), ("e2e", 600_000)] {
            for host in ["io", "europa", "callisto"] {
                harness.runs(host, 2_000, name, 6, duration).await?;
            }
            harness
                .runs("Jupiter", 2_000, name, 6, 2 * duration)
                .await?;
        }
        let jupiter = harness.fleet.host("Jupiter")?.id.clone();
        older_runner_rows(&harness.db, &jupiter, "unit").await?;
        newer_placements(&harness.db, &jupiter, "lint").await?;
        let duplicated: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM workload_placements WHERE id LIKE '%-old' OR id LIKE '%-g2'",
        )
        .fetch_one(&harness.db)
        .await?;
        assert_eq!(duplicated, 12, "the seed made its duplicates");

        let results = harness.indexes().await?;
        let result = harness.result(&results, "Jupiter", 2_000)?;
        let runs: Vec<(String, u32, u32)> = result
            .top_keys
            .iter()
            .map(|key| (key.key.clone(), key.host_runs, key.fleet_runs))
            .collect();
        let expected: Vec<(String, u32, u32)> = ["e2e", "lint", "unit"]
            .into_iter()
            .map(|name| (job_key("acme/app", "CI", name), 6, 24))
            .collect();
        assert_eq!(runs, expected, "(key, machine runs, fleet runs)");
        harness.close().await
    }
}
