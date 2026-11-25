//! The relative speed index (#193): how long a machine takes to run the jobs
//! the rest of the fleet also runs, at one runner size, against the fleet's
//! median. 1.0 is typical; 1.5 is half again as slow.
//!
//! The rules (`docs/design/fleet-health.md`): a job key is repository +
//! workflow name + job name with a trailing shard counter stripped (a GitLab
//! job's, `gitlab:` + instance + project path + job name, #253); only
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

/// The key a GitLab job's runs are compared under (#253):
/// `gitlab:<instance>/<project path> / <job name>`, the shard counter
/// stripped as in [`job_key`] (GitLab names a `parallel` job's copies
/// `name 1/4`, `name 2/4`, …). The instance is the connection's `base_url`
/// without its scheme or trailing `/`. The `gitlab:` prefix keeps it from
/// ever equalling a GitHub key, whose repository (`owner/name`) holds no
/// `:`, though a GitLab job name may hold ` / ` (`deploy / prod` against
/// GitHub workflow `deploy`, job `prod`); the instance keeps two GitLab
/// instances' projects of the same path apart.
#[must_use]
pub fn gitlab_job_key(base_url: &str, project_path: &str, job_name: &str) -> String {
    let instance = base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest)
        .trim_end_matches('/');
    let name = strip_shard_suffix(job_name);
    format!("gitlab:{instance}/{project_path} / {name}")
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
/// One query walks the window's successful final-attempt GitHub jobs joined
/// to the placements that ran them, keeping every duration, so both medians
/// are true ones, not histogram estimates, and a second does the same for
/// the successful GitLab jobs (#253). Each runs once per hour of the window
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
        let github = grouped_durations(db, slice_start, slice_end)
            .await?
            .into_iter()
            .map(
                |(repository, workflow, name, host_id, cpu_millis, durations)| {
                    (
                        job_key(&repository, &workflow, &name),
                        host_id,
                        cpu_millis,
                        durations,
                    )
                },
            );
        let gitlab = grouped_gitlab_durations(db, slice_start, slice_end)
            .await?
            .into_iter()
            .map(
                |(base_url, project_path, name, host_id, cpu_millis, durations)| {
                    (
                        gitlab_job_key(&base_url, &project_path, &name),
                        host_id,
                        cpu_millis,
                        durations,
                    )
                },
            );
        for (key, host_id, cpu_millis, durations) in github.chain(gitlab) {
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
/// rollup reads it, so duplicate rows never count a job twice. Only a job on
/// a machine that is approved, not deleted, and existed when the job
/// completed counts (#262, [`super::machines`]).
///
/// # Errors
/// The read failed.
async fn grouped_durations(
    db: &SqlitePool,
    from: i64,
    to: i64,
) -> anyhow::Result<Vec<(String, String, String, String, i64, String)>> {
    let rows = sqlx::query_as::<_, (String, String, String, String, i64, String)>(concat!(
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
             JOIN fleet_hosts fh ON fh.id = p.host_id
              AND "#,
        crate::fleet_machine_approved!("fh"),
        " AND ",
        crate::fleet_machine_existed_at!("fh", "job.completed_at"),
        r#"
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
    ))
    .bind(from)
    .bind(to)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// The successful GitLab jobs completed in `[from, to)` (#253), grouped as
/// [`grouped_durations`] groups GitHub's, by (instance `base_url`, project
/// path, job name, machine, runner size), through
/// `gitlab_jobs_completed_idx`. A GitLab job
/// is its own only attempt (as #191's rollup reads it); `status='success'`
/// is its success. A job without a name, a path or a start (a row written
/// before migration 0050) has no key or duration and is left out. A job
/// recorded under several connections of one GitLab instance (a project's
/// and its group's) counts once, by its row with the lowest id (looked up on
/// `gitlab_jobs_connection_job_unique` per connection of the instance). Its
/// machine is the newest `runners` row with its
/// `gitlab_runner_id` on the same instance, and that row's newest placement,
/// on a machine [`grouped_durations`] would count (#262).
///
/// # Errors
/// The read failed.
async fn grouped_gitlab_durations(
    db: &SqlitePool,
    from: i64,
    to: i64,
) -> anyhow::Result<Vec<(String, String, String, String, i64, String)>> {
    let rows = sqlx::query_as::<_, (String, String, String, String, i64, String)>(concat!(
        r#"SELECT connection.base_url, job.project_path, job.job_name, p.host_id, p.cpu_millis,
                  GROUP_CONCAT(job.completed_at - job.started_at) AS durations
             FROM gitlab_jobs job
             JOIN gitlab_connections connection ON connection.id = job.connection_id
             JOIN runners r ON r.id = (
                    SELECT cand.id FROM runners cand
                      JOIN gitlab_connections cand_connection
                        ON cand_connection.id = cand.gitlab_connection_id
                     WHERE cand.gitlab_runner_id = job.gitlab_runner_id
                       AND cand_connection.base_url = connection.base_url
                     ORDER BY cand.created_at DESC, cand.id DESC LIMIT 1)
             JOIN workload_placements p ON p.id = (
                    SELECT latest.id FROM workload_placements latest
                     WHERE latest.runner_id = r.id
                     ORDER BY latest.generation DESC, latest.updated_at DESC, latest.id DESC
                     LIMIT 1)
             JOIN fleet_hosts fh ON fh.id = p.host_id
              AND "#,
        crate::fleet_machine_approved!("fh"),
        " AND ",
        crate::fleet_machine_existed_at!("fh", "job.completed_at"),
        r#"
            WHERE job.completed_at >= ?
              AND job.completed_at < ?
              AND job.status = 'success'
              AND job.started_at IS NOT NULL
              AND job.job_name IS NOT NULL
              AND job.project_path IS NOT NULL
              AND NOT EXISTS (
                    SELECT 1 FROM gitlab_connections other_connection
                      JOIN gitlab_jobs other
                        ON other.connection_id = other_connection.id
                       AND other.job_id = job.job_id
                     WHERE other_connection.base_url = connection.base_url
                       AND other.id < job.id)
            GROUP BY connection.base_url, job.project_path, job.job_name, p.host_id,
                     p.cpu_millis"#,
    ))
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

    use super::{HealthWindow, SpeedResult, gitlab_job_key, job_key, median, speed_indexes};
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

    /// #253 item 5: GitLab jobs are speed samples too, keyed by
    /// `project path / job name` (shard counter stripped), so a machine that
    /// runs only GitLab jobs has an index. Only successful jobs with a name,
    /// a path and a start count, and a job recorded under two connections of
    /// one GitLab instance (a project's and its group's) counts once.
    #[tokio::test]
    async fn gitlab_jobs_are_speed_samples_keyed_by_project_path_and_job_name() -> Result<()> {
        let mut harness = Harness::new().await?;
        let mut jobs = Vec::new();
        for (name, duration) in [
            ("rspec 1/4", 120_000_i64),
            ("lint", 60_000),
            ("build", 600_000),
        ] {
            for (host, factor) in [("io", 1), ("europa", 1), ("callisto", 2)] {
                let host_id = harness.fleet.host(host)?.id.clone();
                for run in 0..5_i64 {
                    let id = harness.next_id;
                    harness.next_id += 1;
                    let completed = NOW - 2 * HOUR - run * 60_000;
                    let mut job = JobSeed::completed(
                        id,
                        name,
                        Some(&host_id),
                        completed - factor * duration - 5_000,
                        completed - factor * duration,
                        completed,
                    );
                    job.gitlab = true;
                    jobs.push(job);
                }
            }
        }
        // Left out: a failed job, and jobs with no name or no path (written
        // before migration 0050), each far slower than the rest.
        for (id, status) in [
            (9_001_i64, "failed"),
            (9_002, "nameless"),
            (9_003, "pathless"),
        ] {
            let host_id = harness.fleet.host("callisto")?.id.clone();
            let mut job = JobSeed::completed(
                id,
                "lint",
                Some(&host_id),
                NOW - 3 * HOUR - 10_000_000,
                NOW - 3 * HOUR - 9_000_000,
                NOW - 3 * HOUR,
            );
            job.gitlab = true;
            if status == "failed" {
                job.conclusion = Some("failure".to_owned());
            }
            jobs.push(job);
        }
        testdata::insert_jobs(&harness.db, &harness.fleet, &jobs).await?;
        sqlx::query("UPDATE gitlab_jobs SET job_name=NULL WHERE job_id=9002")
            .execute(&harness.db)
            .await?;
        sqlx::query("UPDATE gitlab_jobs SET project_path=NULL WHERE job_id=9003")
            .execute(&harness.db)
            .await?;
        // Every callisto job also seen through the group connection.
        sqlx::query(
            "INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
             VALUES ('group-connection','group','https://gitlab.test','group',2,'acme','group.token','group.webhook',1,1)",
        )
        .execute(&harness.db)
        .await?;
        sqlx::query(
            "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,gitlab_runner_id,created_at,updated_at,category,job_name,project_path,started_at,completed_at)
             SELECT id||'-group','group-connection',project_id,job_id,pipeline_id,status,tag_list,status_changed_at,gitlab_runner_id,created_at,updated_at,category,job_name,project_path,started_at,completed_at
               FROM gitlab_jobs",
        )
        .execute(&harness.db)
        .await?;

        let results = harness.indexes().await?;
        let callisto = harness.result(&results, "callisto", 2_000)?;
        let index = callisto
            .index
            .ok_or_else(|| anyhow::anyhow!("callisto has no index: {callisto:?}"))?;
        assert!(
            (index - 2.0).abs() < 1e-9,
            "callisto runs at half speed: {index}"
        );
        let keys: Vec<(String, u32, u32)> = callisto
            .top_keys
            .iter()
            .map(|key| (key.key.clone(), key.host_runs, key.fleet_runs))
            .collect();
        let mut expected: Vec<(String, u32, u32)> = ["build", "lint", "rspec"]
            .into_iter()
            .map(|name| {
                (
                    gitlab_job_key("https://gitlab.test", "acme/app", name),
                    5,
                    15,
                )
            })
            .collect();
        expected.sort();
        assert_eq!(keys, expected, "(key, machine runs, fleet runs)");
        let io = harness.result(&results, "io", 2_000)?;
        assert_eq!(io.index, Some(1.0), "io runs at the fleet median: {io:?}");
        harness.close().await
    }

    /// Adds a second GitLab instance, `https://gitlab.other`, with a project
    /// connection, and moves `host`'s GitLab runners to it (with
    /// `move_jobs`, their jobs too).
    async fn other_instance(harness: &Harness, host: &str, move_jobs: bool) -> Result<()> {
        sqlx::query(
            "INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
             VALUES ('other-instance','other','https://gitlab.other','project',1,'acme/app','other.token','other.webhook',1,1)",
        )
        .execute(&harness.db)
        .await?;
        let host_id = harness.fleet.host(host)?.id.clone();
        if move_jobs {
            sqlx::query(
                "UPDATE gitlab_jobs SET connection_id='other-instance'
                  WHERE gitlab_runner_id IN (SELECT gitlab_runner_id FROM runners WHERE id IN (
                        SELECT runner_id FROM workload_placements WHERE host_id=?))",
            )
            .bind(&host_id)
            .execute(&harness.db)
            .await?;
        }
        sqlx::query(
            "UPDATE runners SET gitlab_connection_id='other-instance' WHERE id IN (
                    SELECT runner_id FROM workload_placements WHERE host_id=?)",
        )
        .bind(&host_id)
        .execute(&harness.db)
        .await?;
        Ok(())
    }

    /// `runs` successful GitLab runs of each of `names` on `host` at 2
    /// CPUs, each lasting `duration` ms.
    async fn gitlab_runs(
        harness: &mut Harness,
        host: &str,
        names: &[&str],
        runs: i64,
        duration: i64,
    ) -> Result<()> {
        let host_id = harness.fleet.host(host)?.id.clone();
        let mut jobs = Vec::new();
        for name in names {
            for run in 0..runs {
                let id = harness.next_id;
                harness.next_id += 1;
                let completed = NOW - 2 * HOUR - run * 60_000;
                let mut job = JobSeed::completed(
                    id,
                    name,
                    Some(&host_id),
                    completed - duration - 5_000,
                    completed - duration,
                    completed,
                );
                job.gitlab = true;
                jobs.push(job);
            }
        }
        testdata::insert_jobs(&harness.db, &harness.fleet, &jobs).await
    }

    #[test]
    fn gitlab_keys_name_their_instance_and_never_equal_a_github_key() {
        assert_eq!(
            gitlab_job_key("https://gitlab.test/", "acme/app", "rspec 2/4"),
            "gitlab:gitlab.test/acme/app / rspec"
        );
        assert_ne!(
            gitlab_job_key("https://gitlab.test", "acme/app", "deploy / prod"),
            job_key("acme/app", "deploy", "prod")
        );
        assert_ne!(
            gitlab_job_key("https://gitlab.test", "acme/app", "unit"),
            gitlab_job_key("https://gitlab.other", "acme/app", "unit")
        );
        assert_eq!(
            gitlab_job_key("https://gitlab.test", "acme/app", "unit"),
            gitlab_job_key("http://gitlab.test", "acme/app", "unit"),
            "one instance whatever its scheme"
        );
    }

    /// #253 review: a GitLab key never equals a GitHub key, though a GitLab
    /// job name may hold " / ": GitLab project `acme/app`'s job `k1 / x` is
    /// not GitHub repository `acme/app`'s workflow `k1` job `x`. callisto
    /// runs only the GitLab jobs, the rest of the fleet only the GitHub ones,
    /// so callisto shares no key with the fleet and has no index.
    #[tokio::test]
    async fn a_gitlab_key_never_equals_a_github_key() -> Result<()> {
        let mut harness = Harness::new().await?;
        let names = ["k1 / x", "k2 / x", "k3 / x"];
        gitlab_runs(&mut harness, "callisto", &names, 5, 240_000).await?;
        for host in ["io", "europa"] {
            let host_id = harness.fleet.host(host)?.id.clone();
            let mut jobs = Vec::new();
            for workflow in ["k1", "k2", "k3"] {
                for _ in 0..5 {
                    let id = harness.next_id;
                    harness.next_id += 1;
                    let completed = NOW - 2 * HOUR;
                    let mut job = JobSeed::completed(
                        id,
                        "x",
                        Some(&host_id),
                        completed - 125_000,
                        completed - 120_000,
                        completed,
                    );
                    job.workflow = workflow.to_owned();
                    jobs.push(job);
                }
            }
            testdata::insert_jobs(&harness.db, &harness.fleet, &jobs).await?;
        }
        let results = harness.indexes().await?;
        let callisto = harness.result(&results, "callisto", 2_000)?;
        assert_eq!(
            callisto.index, None,
            "no key shared with GitHub: {callisto:?}"
        );
        harness.close().await
    }

    /// #253 review: two GitLab instances never share a key, though their
    /// projects have the same path: callisto runs `acme/app`'s jobs on
    /// another instance than the rest of the fleet, so it has no index.
    #[tokio::test]
    async fn gitlab_instances_never_share_a_key() -> Result<()> {
        let mut harness = Harness::new().await?;
        let names = ["unit", "lint", "build"];
        gitlab_runs(&mut harness, "callisto", &names, 5, 240_000).await?;
        gitlab_runs(&mut harness, "io", &names, 5, 120_000).await?;
        gitlab_runs(&mut harness, "europa", &names, 5, 120_000).await?;
        other_instance(&harness, "callisto", true).await?;
        let results = harness.indexes().await?;
        let callisto = harness.result(&results, "callisto", 2_000)?;
        assert_eq!(
            callisto.index, None,
            "another instance's keys: {callisto:?}"
        );
        let io = harness.result(&results, "io", 2_000)?;
        assert_eq!(io.index, Some(1.0), "io's fleet is europa alone: {io:?}");
        harness.close().await
    }

    /// #253 review: a GitLab job's runner is the one with its
    /// `gitlab_runner_id` on the job's own instance: callisto's runners are
    /// registered on another instance than its jobs', so its jobs have no
    /// machine and are not speed samples.
    #[tokio::test]
    async fn a_gitlab_job_runs_on_a_runner_of_its_own_instance() -> Result<()> {
        let mut harness = Harness::new().await?;
        let names = ["unit", "lint", "build"];
        for (host, duration) in [("callisto", 240_000), ("io", 120_000), ("europa", 120_000)] {
            gitlab_runs(&mut harness, host, &names, 5, duration).await?;
        }
        other_instance(&harness, "callisto", false).await?;
        let results = harness.indexes().await?;
        let callisto = harness.result(&results, "callisto", 2_000);
        assert!(callisto.is_err(), "callisto ran nothing: {results:?}");
        harness.close().await
    }

    /// #262: the speed index's machine set is the fleet machines'. A revoked
    /// machine's runs are left out of the fleet medians and get no index; a
    /// machine retired after its runs keeps them, and a run its machine
    /// completed after its retirement (or before its creation) is left out.
    #[tokio::test]
    async fn only_runs_on_a_fleet_machine_while_it_existed_count() -> Result<()> {
        let mut harness = Harness::new().await?;
        for key in ["k1", "k2", "k3"] {
            harness.runs("io", 2_000, key, 5, 100_000).await?;
            harness.runs("Jupiter", 2_000, key, 5, 100_000).await?;
            // callisto is slow at every key, then revoked.
            harness.runs("callisto", 2_000, key, 5, 400_000).await?;
            // europa, retired an hour after these runs, is twice as slow.
            harness.runs("europa", 2_000, key, 5, 200_000).await?;
        }
        let (callisto, europa, io) = (
            harness.fleet.host("callisto")?.id.clone(),
            harness.fleet.host("europa")?.id.clone(),
            harness.fleet.host("io")?.id.clone(),
        );
        sqlx::query(
            "UPDATE fleet_hosts SET enrollment_state='revoked',lifecycle_state='retired',
                    retired_at=?1 WHERE id=?2",
        )
        .bind(NOW - HOUR)
        .bind(&callisto)
        .execute(&harness.db)
        .await?;
        sqlx::query("UPDATE fleet_hosts SET lifecycle_state='retired',retired_at=? WHERE id=?")
            .bind(NOW - HOUR)
            .bind(&europa)
            .execute(&harness.db)
            .await?;
        // A run io "completed" before io existed is not io's.
        harness
            .runs_at("io", 2_000, "k1", 1, 900_000, NOW - 20 * HOUR)
            .await?;
        sqlx::query("UPDATE fleet_hosts SET created_at=? WHERE id=?")
            .bind(NOW - 10 * HOUR)
            .bind(&io)
            .execute(&harness.db)
            .await?;
        let results = harness.indexes().await?;
        assert!(
            results.iter().all(|result| result.host_id != callisto),
            "a revoked machine has no index: {results:?}"
        );
        // Each key: io and Jupiter at 100 s, europa at 200 s; the fleet
        // median over those 15 runs is 100 s, so europa's index is 2.0
        // (callisto's runs would make it 150 s, and the index 1.33).
        let europa_result = harness.result(&results, "europa", 2_000)?;
        assert!(
            europa_result
                .index
                .is_some_and(|index| (index - 2.0).abs() < 1e-9),
            "europa {:?}",
            europa_result.index
        );
        let fleet_runs: Vec<(String, u32)> = europa_result
            .top_keys
            .iter()
            .map(|key| (key.key.clone(), key.fleet_runs))
            .collect();
        assert_eq!(fleet_runs.len(), 3, "{fleet_runs:?}");
        assert!(
            fleet_runs.iter().all(|(_, runs)| *runs == 15),
            "neither callisto's runs nor the one before io existed count: {fleet_runs:?}"
        );
        harness.close().await
    }
}
