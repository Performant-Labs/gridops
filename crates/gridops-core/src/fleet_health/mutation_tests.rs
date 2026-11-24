//! Mutation tests for #192 and #193 (#246): each test here kills mutants
//! `cargo mutants` found surviving in `score.rs`, `concerns.rs` and
//! `speed.rs`, or pins an edge the issue names (the concern bands, the 0 and
//! 100 clamps, the reweighting of every subset of missing components, the
//! rounding of the busy-share gate, the speed index's minimum run counts and
//! its `ratio > 1.0` filter, every concern threshold one unit either side).
//! `docs/handoffs/mutation-192-193.md` lists every survivor and what killed
//! it, or why it is equivalent.
//!
//! Mounted from `fleet_health/mod.rs` under `#[cfg(test)]`. The contract
//! tests (`*_contract_tests.rs`) stay #197's and are not edited.

use std::collections::BTreeMap;

use super::{
    FleetHealthInputs, FleetLiveStats, HOUR_MILLIS, HealthTargets, HealthWindow, Histogram,
    HostInfo, HourlyHostRow, HourlyJobRow, JobCategory, RollupHost, ScoreComponent, ScoredFleet,
    ScoredHost, Severity, SpeedResult, concerns, score,
};

const GIB: u64 = 1024 * 1024 * 1024;
/// A whole hour, the `now` most tests read as of.
const T: i64 = 1_800_000_000_000;
const MINUTE: i64 = 60_000;

/// The fleet's components and weights, in the score table's order.
const FLEET_WEIGHTS: [(&str, f64); 5] = [
    ("queue-wait", 40.0),
    ("start-reliability", 20.0),
    ("infra-failures", 15.0),
    ("capacity-fit", 15.0),
    ("availability", 10.0),
];

/// A machine's components and weights, in the score table's order.
const HOST_WEIGHTS: [(&str, f64); 5] = [
    ("availability", 25.0),
    ("start-reliability", 25.0),
    ("relative-speed", 25.0),
    ("headroom", 15.0),
    ("busy-share", 10.0),
];

fn component(key: &str, weight: f64, score: Option<u8>) -> ScoreComponent {
    ScoreComponent {
        key: key.to_owned(),
        label: key.to_owned(),
        weight,
        score,
        deduction: None,
    }
}

fn close(got: f64, expected: f64) -> bool {
    (got - expected).abs() < 1e-9
}

// ---------------------------------------------------------------------------
// Reweighting: every subset of missing components.
// ---------------------------------------------------------------------------

/// For each of the 32 subsets of scored components: a missing one weighs 0,
/// a scored one `weight × 100 ÷ scored weight`, the weights sum to 100, and
/// the score is the weighted mean rounded once; none scored is `None`, with
/// the table's weights kept.
fn every_subset_reweights(table: &[(&str, f64); 5]) {
    // Distinct scores, so a weight given to the wrong component shows,
    // with both clamps (0 and 100), and no subset whose mean is exactly .5.
    let scores: [u8; 5] = [62, 70, 8, 0, 100];
    for mask in 0_u32..32 {
        let present = |index: usize| mask & (1 << index) != 0;
        let components: Vec<ScoreComponent> = table
            .iter()
            .enumerate()
            .map(|(index, &(key, weight))| {
                component(key, weight, present(index).then_some(scores[index]))
            })
            .collect();
        let combined = score::combine(&components);
        let sum: f64 = combined.components.iter().map(|c| c.weight).sum();
        assert!(close(sum, 100.0), "subset {mask:05b}: weights sum to {sum}");
        if mask == 0 {
            assert_eq!(combined.score, None, "nothing scored");
            for (got, &(key, weight)) in combined.components.iter().zip(table) {
                assert!(
                    close(got.weight, weight),
                    "{key} keeps {weight} with nothing scored"
                );
            }
            continue;
        }
        // Integer arithmetic: the table's weights are whole numbers.
        let (mut numerator, mut denominator) = (0_u64, 0_u64);
        for (index, &(_, weight)) in table.iter().enumerate() {
            if present(index) {
                numerator += u64::from(scores[index]) * weight as u64;
                denominator += weight as u64;
            }
        }
        assert_ne!(
            (2 * numerator) % (2 * denominator),
            denominator,
            "subset {mask:05b} lands on .5: pick other scores"
        );
        let expected = ((2 * numerator + denominator) / (2 * denominator)) as u8;
        assert_eq!(combined.score, Some(expected), "subset {mask:05b}");
        for (index, (got, &(key, weight))) in combined.components.iter().zip(table).enumerate() {
            let want = if present(index) {
                weight * 100.0 / denominator as f64
            } else {
                0.0
            };
            assert!(
                close(got.weight, want),
                "subset {mask:05b}: {key} weighs {} not {want}",
                got.weight
            );
            assert_eq!(got.key, key, "the order is kept");
        }
    }
}

#[test]
fn every_subset_of_missing_fleet_components_reweights_to_100() {
    every_subset_reweights(&FLEET_WEIGHTS);
}

#[test]
fn every_subset_of_missing_machine_components_reweights_to_100() {
    every_subset_reweights(&HOST_WEIGHTS);
}

// ---------------------------------------------------------------------------
// Concern bands and thresholds, one unit either side.
// ---------------------------------------------------------------------------

#[test]
fn the_concern_bands_hold_at_every_edge_and_one_either_side() {
    for (score, expected) in [
        (0, Some(Severity::Critical)),
        (48, Some(Severity::Critical)),
        (49, Some(Severity::Critical)),
        (50, Some(Severity::Warning)),
        (51, Some(Severity::Warning)),
        (68, Some(Severity::Warning)),
        (69, Some(Severity::Warning)),
        (70, None),
        (71, None),
        (100, None),
    ] {
        let fleet = concerns::fleet_component(&component("queue-wait", 40.0, Some(score)));
        assert_eq!(
            fleet.map(|c| c.severity),
            expected,
            "fleet component {score}"
        );
        let host = ScoredHost {
            host_id: "h".to_owned(),
            name: "h".to_owned(),
            score: Some(score),
            components: vec![component("availability", 100.0, Some(score))],
            busy_share: None,
        };
        let got = concerns::host_score(&host);
        assert_eq!(got.map(|c| c.severity), expected, "machine {score}");
    }
}

#[test]
fn each_concern_threshold_holds_one_unit_either_side() {
    let info = HostInfo {
        id: "h".to_owned(),
        name: "h".to_owned(),
        cpu_budget_millis: 4_000,
        memory_budget_mib: 1_000,
        disk_total_bytes: None,
    };
    let slow = |index: f64| SpeedResult {
        host_id: "h".to_owned(),
        cpu_millis: 2_000,
        index: Some(index),
        top_keys: Vec::new(),
    };
    for (index, fires) in [
        (1.29, false),
        (1.299_999, false),
        (1.3, true),
        (1.300_001, true),
        (1.31, true),
    ] {
        assert_eq!(
            concerns::host_slow(&info, &slow(index)).is_some(),
            fires,
            "host-slow at {index}"
        );
    }

    let idle = |busy: f64| ScoredHost {
        host_id: "h".to_owned(),
        name: "h".to_owned(),
        score: Some(90),
        components: Vec::new(),
        busy_share: Some(busy),
    };
    for (busy, queue, fires) in [
        (0.29, 69, true),
        (0.299_999, 69, true),
        (0.30, 69, false),
        (0.300_001, 69, false),
        (0.29, 68, true),
        (0.29, 70, false),
        (0.29, 71, false),
        (0.0, 0, true),
    ] {
        assert_eq!(
            concerns::host_idle_while_queued(&idle(busy), Some(queue)).is_some(),
            fires,
            "idle at busy {busy}, queue {queue}"
        );
    }

    for (share, fires) in [
        (0.19, false),
        (0.199_999, false),
        (0.20, false),
        (0.200_001, true),
        (0.21, true),
    ] {
        assert_eq!(
            concerns::category_other(Some(share)).is_some(),
            fires,
            "category-other at {share}"
        );
    }
}

/// Each fleet component's concern has its own title; an unknown key falls
/// back to its label and score.
#[test]
fn each_fleet_component_concern_has_its_own_title() {
    for (key, title) in [
        ("queue-wait", "Jobs wait too long to start"),
        ("start-reliability", "Runner starts fail too often"),
        ("infra-failures", "Jobs fail on the infrastructure"),
        ("capacity-fit", "The fleet runs idle while jobs wait"),
        ("availability", "Too much of the fleet is offline"),
        ("something-else", "something-else scores 40"),
    ] {
        let concern = concerns::fleet_component(&component(key, 10.0, Some(40)));
        assert_eq!(concern.map(|c| c.title).as_deref(), Some(title), "{key}");
    }
}

fn job_row(hour: i64, host: RollupHost, category: JobCategory, jobs: u64) -> HourlyJobRow {
    HourlyJobRow {
        hour,
        host,
        pool_id: "pool".to_owned(),
        category,
        job_count: jobs,
        completed_count: jobs,
        infra_failure_count: 0,
        duration_ms_sum: 0,
        cpu_seconds_sum: 0.0,
        wait_histogram: Histogram::default(),
        duration_histogram: Histogram::default(),
    }
}

fn no_score() -> ScoredFleet {
    ScoredFleet {
        score: None,
        components: Vec::new(),
        hosts: Vec::new(),
    }
}

fn concern_inputs(now: i64, hourly: Vec<HourlyJobRow>) -> FleetHealthInputs {
    FleetHealthInputs {
        window: HealthWindow::Day,
        now,
        hosts: Vec::new(),
        hourly,
        host_hourly: Vec::new(),
        speed: Vec::new(),
        live: FleetLiveStats::default(),
        host_live: Vec::new(),
        targets: HealthTargets::default(),
    }
}

/// The `category-other` share counts the rows of `[now − 24 h, now)` as of
/// a real `now`: a row an hour before the window, 9,999 `test` jobs, must
/// not dilute the window's 50% `other`, and the first hour of the window
/// counts.
#[test]
fn the_other_share_reads_the_window_as_of_a_real_now() {
    let concerns = concerns::windowed(
        &concern_inputs(
            T,
            vec![
                job_row(
                    T - 24 * HOUR_MILLIS,
                    RollupHost::Unmanaged,
                    JobCategory::Other,
                    50,
                ),
                job_row(
                    T - HOUR_MILLIS,
                    RollupHost::Unmanaged,
                    JobCategory::Test,
                    50,
                ),
                job_row(
                    T - 25 * HOUR_MILLIS,
                    RollupHost::Unmanaged,
                    JobCategory::Test,
                    9_999,
                ),
            ],
        ),
        &no_score(),
    );
    let keys: Vec<&str> = concerns.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(keys, ["category-other"]);
    assert!(
        concerns[0].detail.starts_with("50% of the window's jobs"),
        "{}",
        concerns[0].detail
    );
}

/// A window without a job has no `other` share, so no concern (and no
/// `NaN%` sentence).
#[test]
fn a_window_without_jobs_raises_no_other_concern() {
    assert_eq!(
        concerns::windowed(&concern_inputs(T, Vec::new()), &no_score()),
        Vec::new()
    );
}

// ---------------------------------------------------------------------------
// The busy-share gate reads the rounded queue-wait score.
// ---------------------------------------------------------------------------

#[test]
fn the_busy_share_gate_deducts_at_99_and_not_at_100_or_without_a_queue_score() {
    let targets = HealthTargets::default();
    assert_eq!(
        score::host_busy_share(Some(0.0), Some(99), &targets).score,
        Some(0)
    );
    assert_eq!(
        score::host_busy_share(Some(0.0), Some(100), &targets).score,
        Some(100)
    );
    assert_eq!(
        score::host_busy_share(Some(0.0), None, &targets).score,
        Some(100)
    );
}

// ---------------------------------------------------------------------------
// The speed index (#193): the minimum run counts and the `ratio > 1.0` filter.
// ---------------------------------------------------------------------------

mod speed_index {
    use anyhow::Result;
    use sqlx::SqlitePool;

    use crate::fleet_health::{
        HealthWindow, SpeedResult, speed,
        testdata::{self, JobSeed, SeededFleet},
    };

    const HOUR: i64 = 3_600_000;
    const NOW: i64 = 1_800_000_000_000;

    struct Harness {
        directory: std::path::PathBuf,
        db: SqlitePool,
        fleet: SeededFleet,
        next_id: i64,
    }

    impl Harness {
        async fn new() -> Result<Self> {
            let directory = std::env::temp_dir()
                .join(format!("gridops-speed-mutation-{}", uuid::Uuid::new_v4()));
            let db = crate::connect_database_path(&directory.join("db.sqlite")).await?;
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
            let host_id = self.fleet.host(host)?.id.clone();
            let mut jobs = Vec::new();
            for _ in 0..runs {
                let id = self.next_id;
                self.next_id += 1;
                let completed = NOW - HOUR - id * 1_000;
                let mut job = JobSeed::completed(
                    id,
                    name,
                    Some(&host_id),
                    completed - duration - 5_000,
                    completed - duration,
                    completed,
                );
                job.cpu_millis = cpu_millis;
                jobs.push(job);
            }
            testdata::insert_jobs(&self.db, &self.fleet, &jobs).await
        }

        async fn result(&self, host: &str, cpu_millis: u32) -> Result<SpeedResult> {
            let id = self.fleet.host(host)?.id.clone();
            speed::speed_indexes(&self.db, HealthWindow::Day, NOW)
                .await?
                .into_iter()
                .find(|result| result.host_id == id && result.cpu_millis == cpu_millis)
                .ok_or_else(|| anyhow::anyhow!("no speed result for {host} at {cpu_millis}"))
        }

        /// Closes the pool and removes the database, reporting a failure to
        /// remove it. A harness dropped without `close` is cleaned up by
        /// [`Drop`].
        async fn close(self) -> Result<()> {
            self.db.close().await;
            std::fs::remove_dir_all(&self.directory)?;
            Ok(())
        }
    }

    impl Drop for Harness {
        /// The database goes whether the test passed or not: after `close`
        /// there is nothing left to remove; after a failed assertion or an
        /// early `?`, this removes it (best effort, as a drop cannot fail).
        fn drop(&mut self) {
            if self.directory.exists() {
                let _ = std::fs::remove_dir_all(&self.directory);
            }
        }
    }

    /// A test that fails before `close` (an assertion, or an early `?`)
    /// still removes its database: a mutation run fails tests by design,
    /// and each failure must not leave a directory behind.
    #[tokio::test]
    async fn a_harness_dropped_without_close_removes_its_database() -> Result<()> {
        let harness = Harness::new().await?;
        let directory = harness.directory.clone();
        assert!(
            directory.exists(),
            "the harness made {}",
            directory.display()
        );
        drop(harness);
        assert!(
            !directory.exists(),
            "{} is left behind",
            directory.display()
        );
        Ok(())
    }

    /// A shard counter is digits over digits on both sides of the slash:
    /// one side alone being digits leaves the name as it is.
    #[test]
    fn a_counter_with_digits_on_one_side_only_is_part_of_the_name() {
        for name in [
            "unit (a/4)",
            "unit (1/b)",
            "unit [x/2]",
            "unit [2/x]",
            "unit a/4",
            "unit 1/b",
            "unit (/4)",
            "unit (1/)",
        ] {
            assert_eq!(
                speed::job_key("acme/app", "CI", name),
                format!("acme/app / CI / {name}"),
                "{name:?}"
            );
        }
        assert_eq!(
            speed::job_key("acme/app", "CI", "unit (1/4)"),
            "acme/app / CI / unit"
        );
    }

    /// Three keys, each run `host_runs` times on Jupiter at twice the
    /// duration and `rest_runs` times on io: Jupiter's index at 2 CPUs.
    async fn three_keys(host_runs: usize, rest_runs: usize) -> Result<SpeedResult> {
        let mut harness = Harness::new().await?;
        for (name, duration) in [("unit", 120_000_i64), ("lint", 60_000), ("e2e", 600_000)] {
            harness.runs("io", 2_000, name, rest_runs, duration).await?;
            harness
                .runs("Jupiter", 2_000, name, host_runs, 2 * duration)
                .await?;
        }
        let result = harness.result("Jupiter", 2_000).await?;
        harness.close().await?;
        Ok(result)
    }

    /// Exactly 5 runs on the machine and exactly 5 on the rest of the fleet
    /// qualify a key; 4 on either side do not. With three keys at the edge
    /// the index exists; one run fewer on either side leaves no key.
    #[tokio::test]
    async fn five_runs_on_each_side_qualify_a_key_and_four_do_not() -> Result<()> {
        let edge = three_keys(5, 5).await?;
        // Each fleet median is the mean of 5 runs at 1x and 5 at 2x, 1.5x:
        // every ratio is 2 / 1.5.
        let index = edge
            .index
            .ok_or_else(|| anyhow::anyhow!("5 and 5 runs gave no index"))?;
        assert!((index - 2.0 / 1.5).abs() < 1e-3, "index {index}");
        assert_eq!(edge.top_keys.len(), 3, "every key is slower than the fleet");
        for key in &edge.top_keys {
            assert_eq!((key.host_runs, key.fleet_runs), (5, 10), "{}", key.key);
        }
        assert_eq!(three_keys(4, 5).await?.index, None, "4 runs on the machine");
        assert_eq!(three_keys(5, 4).await?.index, None, "4 runs on the rest");
        Ok(())
    }

    /// `topKeys` holds a key only when its ratio is above 1.0: a key at
    /// exactly the fleet's median is left out, one a millisecond slower is
    /// in.
    #[tokio::test]
    async fn top_keys_leave_out_a_ratio_of_exactly_1_and_keep_one_just_above() -> Result<()> {
        let mut harness = Harness::new().await?;
        // Five runs on Jupiter and on io each: the fleet median is the mean
        // of the two middle runs, so equal durations give a ratio of 1.0.
        for (name, typical, own) in [
            ("unit", 120_000_i64, 120_000_i64),
            ("lint", 60_000, 60_002),
            ("e2e", 600_000, 300_000),
        ] {
            harness.runs("io", 2_000, name, 5, typical).await?;
            harness.runs("Jupiter", 2_000, name, 5, own).await?;
        }
        let result = harness.result("Jupiter", 2_000).await?;
        let keys: Vec<String> = result.top_keys.iter().map(|key| key.key.clone()).collect();
        assert_eq!(keys, [speed::job_key("acme/app", "CI", "lint")]);
        harness.close().await
    }
}

// ---------------------------------------------------------------------------
// Scores: the clamps and the targets' edges.
// ---------------------------------------------------------------------------

/// A target whose "100 at" equals its "0 at" (a setting, so possible) is a
/// step: 100 at or past the target, 0 short of it, never a division by 0.
#[test]
fn a_target_with_equal_good_and_bad_values_is_a_step_not_a_division_by_zero() {
    let targets = HealthTargets {
        start_success_good: 0.9,
        start_success_bad: 0.9,
        infra_failure_good: 0.02,
        infra_failure_bad: 0.02,
        ..HealthTargets::default()
    };
    // More is better.
    for (share, expected) in [(0.9, 100), (0.95, 100), (0.89, 0), (0.0, 0)] {
        assert_eq!(
            score::fleet_start_reliability(Some(share), &targets).score,
            Some(expected),
            "start success {share}"
        );
    }
    // Less is better.
    for (share, expected) in [(0.02, 100), (0.01, 100), (0.0, 100), (0.03, 0)] {
        assert_eq!(
            score::fleet_infra_failures(Some(share), &targets).score,
            Some(expected),
            "infra failures {share}"
        );
    }
}

/// When disk and memory score the same, headroom explains it with the disk
/// sentence: the memory sentence replaces it only when memory is strictly
/// lower.
#[test]
fn headroom_names_the_disk_when_disk_and_memory_tie() {
    let targets = HealthTargets::default();
    // 1 GiB free of 100 GiB is past the 5 GiB critical floor, and memory at
    // 120% is past 100%: both clamp to exactly 0.
    let disk = Some(super::DiskHeadroom {
        total_bytes: 100 * GIB,
        min_free_bytes: GIB,
    });
    let tied = score::host_headroom(disk, Some(1.2), &targets);
    assert_eq!(tied.score, Some(0));
    assert!(
        tied.deduction
            .as_deref()
            .is_some_and(|sentence| sentence.starts_with("The least free disk was 1 GiB")),
        "{:?}",
        tied.deduction
    );
    // Memory a hair lower than the disk takes over.
    let disk = Some(super::DiskHeadroom {
        total_bytes: 100 * GIB,
        min_free_bytes: 10 * GIB,
    });
    let lower = score::host_headroom(disk, Some(0.81), &targets);
    assert!(
        lower
            .deduction
            .as_deref()
            .is_some_and(|sentence| sentence.starts_with("Memory p95 was 81%")),
        "{:?}",
        lower.deduction
    );
}

/// Components that are scored but weigh nothing leave nothing to divide
/// by: no score, and every weight stays a number (0), not `NaN`.
#[test]
fn scored_components_that_weigh_nothing_give_no_score_and_no_nan_weight() {
    let combined = score::combine(&[
        component("weightless", 0.0, Some(50)),
        component("missing", 0.0, None),
    ]);
    assert_eq!(combined.score, None);
    for part in &combined.components {
        assert!(
            close(part.weight, 0.0),
            "{} weighs {}",
            part.key,
            part.weight
        );
    }
}

/// The 0th percentile is the lower bound of the first bucket holding a job:
/// empty buckets before it are skipped, not interpolated into (`NaN`).
#[test]
fn the_0th_percentile_skips_empty_leading_buckets() {
    let mut waits: Histogram = Histogram::default();
    waits[2] = 5;
    waits[4] = 5;
    assert_eq!(score::percentile(&waits, 0.0), Some(30_000.0));
    // A negative quantile clamps to 0.
    assert_eq!(score::percentile(&waits, -1.0), Some(30_000.0));
}

// ---------------------------------------------------------------------------
// `score_fleet`: what counts as data, at each edge.
// ---------------------------------------------------------------------------

fn host_info(id: &str, cpu: u32, memory: u32, disk: Option<u64>) -> HostInfo {
    HostInfo {
        id: id.to_owned(),
        name: id.to_owned(),
        cpu_budget_millis: cpu,
        memory_budget_mib: memory,
        disk_total_bytes: disk,
    }
}

fn host_row(hour: i64, host: &str, online: u32) -> HourlyHostRow {
    HourlyHostRow {
        hour,
        host_id: host.to_owned(),
        online_minutes: online,
        cpu_used_p50_millis: None,
        cpu_used_p95_millis: None,
        memory_used_p95_mib: None,
        min_disk_free_bytes: None,
        start_attempts: 0,
        start_failures: BTreeMap::new(),
        uncertain_placements: 0,
    }
}

/// A day of rows for `host`, online every minute.
fn day_rows(host: &str) -> Vec<HourlyHostRow> {
    (1..=24)
        .map(|back| host_row(T - back * HOUR_MILLIS, host, 60))
        .collect()
}

fn day_inputs(
    hosts: Vec<HostInfo>,
    hourly: Vec<HourlyJobRow>,
    host_hourly: Vec<HourlyHostRow>,
) -> FleetHealthInputs {
    FleetHealthInputs {
        window: HealthWindow::Day,
        now: T,
        hosts,
        hourly,
        host_hourly,
        speed: Vec::new(),
        live: FleetLiveStats::default(),
        host_live: Vec::new(),
        targets: HealthTargets::default(),
    }
}

/// The score of `key` among `components`.
fn part(components: &[ScoreComponent], key: &str) -> Option<u8> {
    components
        .iter()
        .find(|component| component.key == key)
        .and_then(|component| component.score)
}

/// With `min_jobs` set to 0, a window without a job or a start is still not
/// a measurement: start reliability and infrastructure failures have no
/// score (not a `0 ÷ 0` scored as 0).
#[test]
fn a_min_jobs_of_0_does_not_score_0_starts_or_0_jobs() {
    let mut inputs = day_inputs(
        vec![host_info("a", 4_000, 1_000, None)],
        Vec::new(),
        day_rows("a"),
    );
    inputs.targets.min_jobs = 0;
    let scored = score::score_fleet(&inputs);
    for key in ["queue-wait", "start-reliability", "infra-failures"] {
        assert_eq!(part(&scored.components, key), None, "fleet {key}");
    }
    assert_eq!(
        part(&scored.hosts[0].components, "start-reliability"),
        None,
        "machine start reliability"
    );
    assert_eq!(part(&scored.components, "availability"), Some(100));
}

/// A fleet whose only machine has a CPU budget of 0, or no machine at all,
/// has no budget to weigh availability by: no score, not `0 ÷ 0` scored 0.
#[test]
fn a_fleet_without_cpu_budget_has_no_availability_score() {
    let laptop = score::score_fleet(&day_inputs(
        vec![host_info("laptop", 0, 1_000, None)],
        Vec::new(),
        day_rows("laptop"),
    ));
    assert_eq!(part(&laptop.components, "availability"), None, "budget 0");
    let empty = score::score_fleet(&day_inputs(Vec::new(), Vec::new(), Vec::new()));
    assert_eq!(part(&empty.components, "availability"), None, "no machine");
    assert_eq!(empty.score, None);
}

/// `jobs` jobs on `host` in `hour`, every one of them having waited in the
/// histogram bucket `bucket`.
fn waited(hour: i64, host: &str, jobs: u64, bucket: usize) -> HourlyJobRow {
    let mut row = job_row(
        hour,
        RollupHost::Managed(host.to_owned()),
        JobCategory::Test,
        jobs,
    );
    row.wait_histogram[bucket] = jobs;
    row
}

/// Capacity fit as of a real `now`, hour by hour: an hour is a misfit when
/// jobs waited a minute or more (the 1–2 min bucket counts, the 30–60 s one
/// does not) while that hour's sampled CPU p50 was strictly under half the
/// budget (exactly half is not idle). The window's hours are the 24 that
/// ended in `[now − 24 h, now)`: the first, at exactly `now − 24 h`, counts,
/// and the hour starting at `now` does not. Two misfits of 24 hours are
/// 8.3%, which scores 67; the sentence's share pins the denominator (2 of
/// 23 would be 8.7%, 2 of 25 8.0%).
#[test]
fn capacity_fit_counts_each_hour_of_a_real_window_on_its_own_rows() {
    let start = T - 24 * HOUR_MILLIS;
    let at = |index: i64| start + index * HOUR_MILLIS;
    let mut rows = Vec::new();
    // Hour 24 starts at `now`: its row is outside the window.
    for index in 0..=24 {
        let mut row = host_row(at(index), "a", 60);
        // 1 CPU of 4 is idle; 2 is exactly half, 3 busy.
        row.cpu_used_p50_millis = Some(match index {
            0 | 5 | 10 | 24 => 1_000,
            20 => 2_000,
            _ => 3_000,
        });
        rows.push(row);
    }
    let inputs = day_inputs(
        vec![host_info("a", 4_000, 1_000, None)],
        vec![
            // Waited 1–2 min while idle: the two misfits, the first at the
            // window's very start.
            waited(at(0), "a", 20, 3),
            waited(at(5), "a", 20, 3),
            // Waited 30–60 s while idle: not over a minute.
            waited(at(10), "a", 20, 2),
            // Waited 1–2 min at exactly half the budget: not idle.
            waited(at(20), "a", 20, 3),
            // A misfit in the hour starting at `now`: not the window's.
            waited(at(24), "a", 20, 3),
        ],
        rows,
    );
    let scored = score::score_fleet(&inputs);
    assert_eq!(part(&scored.components, "capacity-fit"), Some(67));
    let deduction = scored
        .components
        .iter()
        .find(|component| component.key == "capacity-fit")
        .and_then(|component| component.deduction.clone())
        .unwrap_or_default();
    assert!(
        deduction.starts_with("8.3% of the window's hours waited over a minute"),
        "{deduction}"
    );
}

/// On 1h at half past, the window has no ended hour yet: capacity fit has
/// nothing to count, which is not enough data (not `0 ÷ 0`).
#[test]
fn capacity_fit_with_no_ended_hour_in_the_window_has_no_score() {
    let mut row = host_row(T, "a", 30);
    row.cpu_used_p50_millis = Some(100);
    let mut inputs = day_inputs(
        vec![host_info("a", 4_000, 1_000, None)],
        vec![waited(T, "a", 20, 3)],
        vec![row],
    );
    inputs.window = HealthWindow::Hour;
    inputs.now = T + 30 * MINUTE;
    let scored = score::score_fleet(&inputs);
    assert_eq!(part(&scored.components, "capacity-fit"), None);
    // The window still has its 20 jobs: queue wait scores.
    assert!(part(&scored.components, "queue-wait").is_some());
}

/// A machine's headroom reads its least free disk over the window against
/// its root disk's size, and its memory p95 against its memory budget; a
/// disk of size 0 or a memory budget of 0 is no data for that half.
#[test]
fn headroom_reads_the_least_free_disk_and_ignores_a_0_size_or_0_budget() {
    let rows = |host: &str, disk: Option<u64>, memory: Option<u32>| -> Vec<HourlyHostRow> {
        let mut rows = day_rows(host);
        for (index, row) in rows.iter_mut().enumerate() {
            row.min_disk_free_bytes = disk.map(|free| if index == 7 { free } else { 50 * GIB });
            row.memory_used_p95_mib = memory;
        }
        rows
    };
    let mut host_hourly = rows("disk", Some(6 * GIB), None);
    host_hourly.extend(rows("no-disk-size", Some(GIB), None));
    host_hourly.extend(rows("no-memory-budget", None, Some(500)));
    let scored = score::score_fleet(&day_inputs(
        vec![
            host_info("disk", 4_000, 1_000, Some(100 * GIB)),
            host_info("no-disk-size", 4_000, 1_000, Some(0)),
            host_info("no-memory-budget", 4_000, 0, None),
        ],
        Vec::new(),
        host_hourly,
    ));
    let headroom = |index: usize| part(&scored.hosts[index].components, "headroom");
    // 6 GiB free, between the 10 GiB warning and 5 GiB critical floors.
    assert_eq!(headroom(0), Some(20), "the least free disk");
    assert_eq!(headroom(1), None, "a root disk of size 0");
    assert_eq!(headroom(2), None, "a memory budget of 0");
}
