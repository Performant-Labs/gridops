//! The fixture fleet and the fake source tests inject. Behind `test-support`:
//! a release build never compiles it (#204).
//!
//! [`fixture`] is `src/features/fleet-health/fixture.json` (the UI's
//! fixture and the `OpenAPI` 200 example are generated from it, and
//! `fleet_health_contract.rs` holds all three equal). [`fixture_inputs`] is
//! the scoring input behind it: the five fixture machines over the 24 hours
//! before [`FIXTURE_NOW`], built so #192's engine finds the fixture's
//! windowed concerns.
//!
//! The fixture's job rollup folds the day into one hour per (machine,
//! category), plus one backlog hour on `io`: scores and concerns only read
//! sums over the window, so the fold changes none of them. The hourly usage
//! the page charts spreads the same totals over the day.
//!
//! [`FakeFleetHealthSource`] serves the fixture's two halves and has the
//! seams #194's cache tests need: call counts, failing on demand, and
//! holding calls until released.

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use tokio::sync::watch;

use super::{
    CategoryUsage, Concern, ConcernSubject, FleetHealth, FleetHealthInputs, FleetHealthSource,
    FleetLiveStats, FleetSummary, HOUR_MILLIS, HealthTargets, HealthWindow, Histogram, HostHealth,
    HostInfo, HostLiveStats, HostSpeed, HourlyHostRow, HourlyJobRow, HourlyUsage, JobCategory,
    LiveHealth, OldestQueued, PoolQueue, RollupHost, ScoreComponent, Severity, SpeedKey,
    SpeedResult, TrendPoint, WindowedHealth, assemble, score,
};

/// When the fixture was generated: 2026-11-20 23:00 UTC.
pub const FIXTURE_NOW: i64 = 1_795_215_600_000;

const MINUTE: i64 = 60_000;
const GIB: u64 = 1024 * 1024 * 1024;

const IO: &str = "3f6c1d2e-8a41-4b7c-9e05-1a2b3c4d5e01";
const JUPITER: &str = "3f6c1d2e-8a41-4b7c-9e05-1a2b3c4d5e02";
const CALLISTO: &str = "3f6c1d2e-8a41-4b7c-9e05-1a2b3c4d5e03";
const EUROPA: &str = "3f6c1d2e-8a41-4b7c-9e05-1a2b3c4d5e04";
const MACBOOK: &str = "3f6c1d2e-8a41-4b7c-9e05-1a2b3c4d5e05";
const POOL: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

/// The fixture machine with the worst relative speed (`callisto`).
pub const SLOW_HOST: &str = CALLISTO;
/// The fixture machine with the worst start reliability (`Jupiter`).
pub const FAILING_STARTS_HOST: &str = JUPITER;

/// The main hour of the folded rollup, and the hour of the queue backlog.
const MAIN_HOUR: i64 = FIXTURE_NOW - 12 * HOUR_MILLIS;
const BACKLOG_HOUR: i64 = FIXTURE_NOW - 3 * HOUR_MILLIS;

/// The day's shape: each hour's share (of 69) of a machine's CPU-hours,
/// oldest first.
const DAY_SHAPE: [u32; 24] = [
    1, 1, 1, 1, 1, 1, 2, 3, 4, 5, 6, 6, 5, 6, 6, 5, 4, 3, 2, 2, 1, 1, 1, 1,
];

fn hosts() -> Vec<HostInfo> {
    [
        (IO, "io", 22_800, 65_536, 500 * GIB),
        (JUPITER, "Jupiter", 19_200, 49_152, 1024 * GIB),
        (CALLISTO, "callisto", 8_000, 16_384, 200 * GIB),
        (EUROPA, "europa", 12_800, 32_768, 512 * GIB),
        (MACBOOK, "MacBookPro", 0, 16_384, 1024 * GIB),
    ]
    .into_iter()
    .map(|(id, name, cpu, memory, disk)| HostInfo {
        id: id.to_owned(),
        name: name.to_owned(),
        cpu_budget_millis: cpu,
        memory_budget_mib: memory,
        disk_total_bytes: Some(disk),
    })
    .collect()
}

fn host_name(id: &str) -> &'static str {
    match id {
        IO => "io",
        JUPITER => "Jupiter",
        CALLISTO => "callisto",
        EUROPA => "europa",
        _ => "MacBookPro",
    }
}

#[allow(clippy::too_many_arguments, reason = "one fixture row per call")]
fn job_row(
    hour: i64,
    host: &str,
    category: JobCategory,
    (job_count, completed_count, infra_failure_count): (u64, u64, u64),
    duration_ms_sum: i64,
    cpu_seconds_sum: f64,
    wait_histogram: Histogram,
    duration_histogram: Histogram,
) -> HourlyJobRow {
    HourlyJobRow {
        hour,
        host: RollupHost::Managed(host.to_owned()),
        pool_id: POOL.to_owned(),
        category,
        job_count,
        completed_count,
        infra_failure_count,
        duration_ms_sum,
        cpu_seconds_sum,
        wait_histogram,
        duration_histogram,
    }
}

fn hourly() -> Vec<HourlyJobRow> {
    use JobCategory::{Build, Check, Deploy, Other, Test};
    let h = MAIN_HOUR;
    vec![
        job_row(
            h,
            IO,
            Test,
            (110, 104, 0),
            39_930_000,
            286_797.2,
            [34, 29, 17, 11, 13, 6, 0, 0, 0],
            [0, 0, 0, 11, 44, 44, 11, 0, 0],
        ),
        job_row(
            h,
            IO,
            Build,
            (60, 60, 0),
            25_920_000,
            156_434.8,
            [19, 15, 10, 7, 6, 3, 0, 0, 0],
            [0, 0, 0, 6, 18, 24, 12, 0, 0],
        ),
        job_row(
            h,
            IO,
            Check,
            (50, 48, 0),
            3_850_000,
            130_362.4,
            [16, 14, 7, 5, 6, 2, 0, 0, 0],
            [0, 5, 20, 20, 5, 0, 0, 0, 0],
        ),
        job_row(
            h,
            IO,
            Other,
            (80, 80, 0),
            2_200_000,
            208_579.8,
            [25, 20, 13, 8, 9, 5, 0, 0, 0],
            [24, 32, 16, 8, 0, 0, 0, 0, 0],
        ),
        job_row(
            BACKLOG_HOUR,
            IO,
            Test,
            (40, 38, 0),
            14_520_000,
            104_289.9,
            [0, 0, 0, 0, 10, 10, 15, 5, 0],
            [0, 0, 0, 4, 16, 16, 4, 0, 0],
        ),
        job_row(
            h,
            JUPITER,
            Test,
            (100, 93, 2),
            36_300_000,
            127_606.2,
            [31, 26, 15, 11, 12, 5, 0, 0, 0],
            [0, 0, 0, 10, 40, 40, 10, 0, 0],
        ),
        job_row(
            h,
            JUPITER,
            Build,
            (40, 37, 3),
            17_280_000,
            51_042.5,
            [12, 11, 7, 4, 4, 2, 0, 0, 0],
            [0, 0, 0, 4, 12, 16, 8, 0, 0],
        ),
        job_row(
            h,
            JUPITER,
            Check,
            (40, 40, 0),
            3_080_000,
            51_042.5,
            [13, 10, 6, 4, 5, 2, 0, 0, 0],
            [0, 4, 16, 16, 4, 0, 0, 0, 0],
        ),
        job_row(
            h,
            JUPITER,
            Deploy,
            (20, 20, 0),
            4_680_000,
            25_521.2,
            [7, 5, 3, 2, 2, 1, 0, 0, 0],
            [0, 0, 0, 4, 12, 4, 0, 0, 0],
        ),
        job_row(
            h,
            JUPITER,
            Other,
            (60, 60, 0),
            1_650_000,
            76_563.7,
            [18, 16, 10, 6, 7, 3, 0, 0, 0],
            [18, 24, 12, 6, 0, 0, 0, 0, 0],
        ),
        job_row(
            h,
            CALLISTO,
            Test,
            (70, 66, 0),
            25_410_000,
            165_888.0,
            [22, 18, 10, 8, 8, 4, 0, 0, 0],
            [0, 0, 0, 7, 28, 28, 7, 0, 0],
        ),
        job_row(
            h,
            CALLISTO,
            Build,
            (25, 25, 0),
            10_860_000,
            59_245.7,
            [8, 7, 4, 2, 3, 1, 0, 0, 0],
            [0, 0, 0, 2, 8, 10, 5, 0, 0],
        ),
        job_row(
            h,
            CALLISTO,
            Check,
            (30, 30, 0),
            2_310_000,
            71_094.9,
            [9, 7, 5, 3, 4, 2, 0, 0, 0],
            [0, 3, 12, 12, 3, 0, 0, 0, 0],
        ),
        job_row(
            h,
            CALLISTO,
            Other,
            (50, 50, 0),
            1_375_000,
            118_491.4,
            [16, 13, 8, 6, 5, 2, 0, 0, 0],
            [15, 20, 10, 5, 0, 0, 0, 0, 0],
        ),
        job_row(
            h,
            EUROPA,
            Test,
            (80, 77, 0),
            29_040_000,
            98_304.0,
            [25, 22, 12, 8, 9, 4, 0, 0, 0],
            [0, 0, 0, 8, 32, 32, 8, 0, 0],
        ),
        job_row(
            h,
            EUROPA,
            Build,
            (25, 25, 0),
            10_860_000,
            30_720.0,
            [8, 6, 4, 2, 3, 2, 0, 0, 0],
            [0, 0, 0, 2, 8, 10, 5, 0, 0],
        ),
        job_row(
            h,
            EUROPA,
            Check,
            (30, 30, 0),
            2_310_000,
            36_864.0,
            [9, 7, 5, 4, 4, 1, 0, 0, 0],
            [0, 3, 12, 12, 3, 0, 0, 0, 0],
        ),
        job_row(
            h,
            EUROPA,
            Deploy,
            (30, 30, 0),
            7_020_000,
            36_864.0,
            [9, 8, 5, 3, 3, 2, 0, 0, 0],
            [0, 0, 0, 6, 18, 6, 0, 0, 0],
        ),
        job_row(
            h,
            EUROPA,
            Other,
            (60, 60, 0),
            1_650_000,
            73_728.0,
            [19, 16, 9, 6, 7, 3, 0, 0, 0],
            [18, 24, 12, 6, 0, 0, 0, 0, 0],
        ),
    ]
}

/// One machine's day: minutes online in each hour (oldest first), its
/// memory p95 and least free disk, and the main hour's starts, start
/// failures and uncertain placements.
struct HostDay {
    id: &'static str,
    cpu_budget_millis: u32,
    online: [u32; 24],
    memory_p95_mib: u32,
    min_disk_free_bytes: u64,
    start_attempts: u32,
    start_failures: &'static [(&'static str, u32)],
    uncertain: u32,
}

fn online(offline_hours: &[(usize, u32)]) -> [u32; 24] {
    let mut minutes = [60; 24];
    for (hour, online) in offline_hours {
        if let Some(slot) = minutes.get_mut(*hour) {
            *slot = *online;
        }
    }
    minutes
}

fn host_hourly() -> Vec<HourlyHostRow> {
    let days = [
        HostDay {
            id: IO,
            cpu_budget_millis: 22_800,
            online: online(&[]),
            memory_p95_mib: 26_214,
            min_disk_free_bytes: 300 * GIB,
            start_attempts: 40,
            start_failures: &[],
            uncertain: 1,
        },
        HostDay {
            id: JUPITER,
            cpu_budget_millis: 19_200,
            online: online(&[(4, 24), (16, 24)]),
            memory_p95_mib: 29_491,
            min_disk_free_bytes: 600 * GIB,
            start_attempts: 24,
            start_failures: &[("agent_session_lost", 3), ("start_deadline_expired", 6)],
            uncertain: 0,
        },
        HostDay {
            id: CALLISTO,
            cpu_budget_millis: 8_000,
            online: online(&[(9, 36)]),
            memory_p95_mib: 14_418,
            min_disk_free_bytes: 15 * GIB,
            start_attempts: 25,
            start_failures: &[("agent_reported", 1)],
            uncertain: 0,
        },
        HostDay {
            id: EUROPA,
            cpu_budget_millis: 12_800,
            online: online(&[(6, 0)]),
            memory_p95_mib: 16_384,
            min_disk_free_bytes: 300 * GIB,
            start_attempts: 30,
            start_failures: &[],
            uncertain: 0,
        },
    ];
    let start = FIXTURE_NOW - HealthWindow::Day.millis();
    let mut rows = Vec::new();
    for day in &days {
        for (index, minutes) in (0_i64..).zip(day.online) {
            let hour = start + index * HOUR_MILLIS;
            let main = hour == MAIN_HOUR;
            let sampled = minutes > 0;
            rows.push(HourlyHostRow {
                hour,
                host_id: day.id.to_owned(),
                online_minutes: minutes,
                cpu_used_p50_millis: sampled.then_some(day.cpu_budget_millis * 6 / 10),
                cpu_used_p95_millis: sampled.then_some(day.cpu_budget_millis * 8 / 10),
                memory_used_p95_mib: sampled.then_some(day.memory_p95_mib),
                min_disk_free_bytes: sampled.then_some(day.min_disk_free_bytes),
                start_attempts: if main { day.start_attempts } else { 0 },
                start_failures: if main {
                    day.start_failures
                        .iter()
                        .map(|(code, count)| ((*code).to_owned(), *count))
                        .collect()
                } else {
                    BTreeMap::new()
                },
                uncertain_placements: if main { day.uncertain } else { 0 },
            });
        }
    }
    // MacBookPro sent no samples: a row with 0 minutes and no usage.
    for index in 0..24 {
        rows.push(HourlyHostRow {
            hour: start + index * HOUR_MILLIS,
            host_id: MACBOOK.to_owned(),
            online_minutes: 0,
            cpu_used_p50_millis: None,
            cpu_used_p95_millis: None,
            memory_used_p95_mib: None,
            min_disk_free_bytes: None,
            start_attempts: 0,
            start_failures: BTreeMap::new(),
            uncertain_placements: 0,
        });
    }
    rows
}

fn key(job: &str) -> String {
    super::speed::job_key("acme/app", "CI", job)
}

fn speed_key(job: &str, host_median_ms: i64, fleet_median_ms: i64, runs: (u32, u32)) -> SpeedKey {
    SpeedKey {
        key: key(job),
        host_median_ms,
        fleet_median_ms,
        host_runs: runs.0,
        fleet_runs: runs.1,
    }
}

fn speed() -> Vec<SpeedResult> {
    let result =
        |host: &str, cpu_millis: u32, index: Option<f64>, top_keys: Vec<SpeedKey>| SpeedResult {
            host_id: host.to_owned(),
            cpu_millis,
            index,
            top_keys,
        };
    vec![
        result(
            IO,
            1_500,
            None,
            vec![speed_key("lint", 66_000, 60_000, (3, 140))],
        ),
        result(
            IO,
            2_000,
            Some(1.01),
            vec![
                speed_key("e2e", 606_000, 600_000, (22, 90)),
                speed_key("unit", 121_000, 120_000, (40, 160)),
                speed_key("build", 300_000, 300_000, (30, 100)),
            ],
        ),
        result(
            JUPITER,
            2_000,
            Some(0.97),
            vec![
                speed_key("unit", 120_000, 120_000, (25, 175)),
                speed_key("e2e", 588_000, 600_000, (18, 94)),
                speed_key("build", 285_000, 300_000, (20, 110)),
            ],
        ),
        result(
            CALLISTO,
            2_000,
            Some(1.42),
            vec![
                speed_key("e2e", 870_000, 600_000, (14, 98)),
                speed_key("build", 405_000, 300_000, (9, 121)),
                speed_key("unit", 162_000, 120_000, (20, 180)),
            ],
        ),
        result(
            EUROPA,
            3_000,
            Some(1.08),
            vec![
                speed_key("e2e", 660_000, 600_000, (12, 100)),
                speed_key("unit", 126_000, 120_000, (16, 184)),
                speed_key("lint", 63_000, 60_000, (14, 129)),
            ],
        ),
    ]
}

fn running(counts: &[(JobCategory, u32)]) -> BTreeMap<JobCategory, u32> {
    counts.iter().copied().collect()
}

fn fleet_live() -> FleetLiveStats {
    use JobCategory::{Build, Check, FixAgent, Other, Test};
    FleetLiveStats {
        queued_total: 7,
        queued_by_category: running(&[(Test, 4), (Build, 1), (Other, 2)]),
        queued_by_pool: vec![PoolQueue {
            pool_id: POOL.to_owned(),
            pool_name: "linux-docker".to_owned(),
            queued: 4,
        }],
        queued_unmatched: 3,
        oldest_queued: Some(OldestQueued {
            job_id: 90_001,
            job_name: "integration tests (2/4)".to_owned(),
            repository: "acme/app".to_owned(),
            queued_at: FIXTURE_NOW - 7 * MINUTE,
            wait_ms: 7 * MINUTE,
        }),
        running_total: 9,
        running_by_category: running(&[(Test, 6), (Build, 1), (Check, 1), (FixAgent, 1)]),
    }
}

fn hosts_live() -> Vec<HostLiveStats> {
    use JobCategory::{Build, Check, FixAgent, Test};
    let stats = |host: &str,
                 online: bool,
                 last_seen_ago: Option<i64>,
                 counts: &[(JobCategory, u32)],
                 reserved: (u32, u32),
                 used: Option<(u32, u32, u64)>,
                 start_failures_last_hour: u32| HostLiveStats {
        host_id: host.to_owned(),
        online,
        last_seen_at: last_seen_ago.map(|ago| FIXTURE_NOW - ago),
        running_by_category: running(counts),
        cpu_reserved_millis: reserved.0,
        memory_reserved_mib: reserved.1,
        cpu_used_millis: used.map(|used| used.0),
        memory_used_mib: used.map(|used| used.1),
        disk_free_bytes: used.map(|used| used.2),
        start_failures_last_hour,
    };
    vec![
        stats(
            IO,
            true,
            Some(20_000),
            &[(Test, 3), (Build, 1), (FixAgent, 1)],
            (9_000, 18_432),
            Some((7_400, 21_000, 300 * GIB)),
            0,
        ),
        stats(
            JUPITER,
            true,
            Some(15_000),
            &[(Test, 1), (Check, 1)],
            (4_000, 8_192),
            Some((3_100, 9_000, 600 * GIB)),
            3,
        ),
        stats(
            CALLISTO,
            true,
            Some(25_000),
            &[(Test, 2)],
            (4_000, 8_192),
            Some((3_500, 12_000, 16 * GIB)),
            0,
        ),
        stats(
            EUROPA,
            false,
            Some(12 * MINUTE),
            &[],
            (0, 0),
            Some((800, 4_000, 300 * GIB)),
            0,
        ),
        stats(MACBOOK, false, None, &[], (0, 0), None, 0),
    ]
}

/// Everything the scoring engine reads for the fixture's 24 hours.
#[must_use]
pub fn fixture_inputs() -> FleetHealthInputs {
    FleetHealthInputs {
        window: HealthWindow::Day,
        now: FIXTURE_NOW,
        hosts: hosts(),
        hourly: hourly(),
        host_hourly: host_hourly(),
        speed: speed(),
        live: fleet_live(),
        host_live: hosts_live(),
        targets: HealthTargets::default(),
    }
}

fn scored(base: ScoreComponent, score: Option<u8>, deduction: Option<&str>) -> ScoreComponent {
    ScoreComponent {
        score,
        deduction: deduction.map(str::to_owned),
        ..base
    }
}

fn fleet_components() -> Vec<ScoreComponent> {
    let t = HealthTargets::default();
    vec![
        scored(
            score::fleet_queue_wait(None, &t),
            Some(61),
            Some("Queue p90 was 4 min 30 s against a 1 min target: −16."),
        ),
        scored(
            score::fleet_start_reliability(None, &t),
            Some(61),
            Some("91.6% of runner starts succeeded against a 99% target: −8."),
        ),
        scored(
            score::fleet_infra_failures(None, &t),
            Some(90),
            Some("0.5% of completed jobs failed on the infrastructure against a 0% target: −2."),
        ),
        scored(score::fleet_capacity_fit(None, &t), Some(100), None),
        scored(
            score::fleet_availability(None, &t),
            Some(87),
            Some(
                "The fleet's CPU budget was online 97.4% of the window against a 100% target: −1.",
            ),
        ),
    ]
}

/// A machine's five components: (score, deduction) in the score table's
/// order (availability, start reliability, relative speed, headroom, busy
/// share).
fn host_components(parts: [(Option<u8>, Option<&str>); 5]) -> Vec<ScoreComponent> {
    let t = HealthTargets::default();
    let [availability, starts, speed, headroom, busy] = parts;
    vec![
        scored(
            score::host_availability(None, &t),
            availability.0,
            availability.1,
        ),
        scored(score::host_start_reliability(None, &t), starts.0, starts.1),
        scored(score::host_relative_speed(None, &t), speed.0, speed.1),
        scored(score::host_headroom(None, None, &t), headroom.0, headroom.1),
        scored(score::host_busy_share(None, None, &t), busy.0, busy.1),
    ]
}

fn usage(
    category: JobCategory,
    jobs: u32,
    cpu_hours: f64,
    median_duration_ms: i64,
    p90_queue_wait_ms: i64,
) -> CategoryUsage {
    CategoryUsage {
        category,
        jobs,
        cpu_hours,
        median_duration_ms: Some(median_duration_ms),
        p90_queue_wait_ms: Some(p90_queue_wait_ms),
    }
}

/// The day's usage chart of one machine: its CPU-hours by category spread
/// over the day's shape, to the hundredth.
fn day_usage(categories: &[CategoryUsage]) -> Vec<HourlyUsage> {
    let start = FIXTURE_NOW - HealthWindow::Day.millis();
    let total: u32 = DAY_SHAPE.iter().sum();
    (0_i64..)
        .zip(DAY_SHAPE)
        .map(|(index, share)| HourlyUsage {
            hour: start + index * HOUR_MILLIS,
            cpu_hours: categories
                .iter()
                .map(|entry| {
                    let hours = entry.cpu_hours * f64::from(share) / f64::from(total);
                    (entry.category, (hours * 100.0).round() / 100.0)
                })
                .collect(),
        })
        .collect()
}

fn host(
    id: &str,
    score: Option<u8>,
    components: Vec<ScoreComponent>,
    categories: Vec<CategoryUsage>,
) -> HostHealth {
    let info = hosts().into_iter().find(|info| info.id == id);
    let (cpu_budget_millis, memory_budget_mib, disk_total_bytes) = info
        .map(|info| {
            (
                info.cpu_budget_millis,
                info.memory_budget_mib,
                info.disk_total_bytes,
            )
        })
        .unwrap_or_default();
    HostHealth {
        id: id.to_owned(),
        name: host_name(id).to_owned(),
        cpu_budget_millis,
        memory_budget_mib,
        disk_total_bytes,
        score,
        components,
        speed: speed()
            .into_iter()
            .filter(|result| result.host_id == id)
            .map(HostSpeed::from)
            .collect(),
        usage: day_usage(&categories),
        categories,
        live: None,
    }
}

fn host_concern(key: &str, host: &str, title: &str, detail: &str) -> Concern {
    Concern {
        key: format!("{key}:{host}"),
        severity: Severity::Warning,
        title: title.to_owned(),
        detail: detail.to_owned(),
        subject: ConcernSubject::Host(host.to_owned()),
        link: Some(format!("/fleet/health#machine-{host}")),
        since: None,
    }
}

fn windowed_concerns() -> Vec<Concern> {
    let fleet = |key: &str, title: &str, detail: &str| Concern {
        key: format!("fleet-component:{key}"),
        severity: Severity::Warning,
        title: title.to_owned(),
        detail: detail.to_owned(),
        subject: ConcernSubject::Fleet,
        // The component's row on the fleet score card, as the engine links it
        // (#253).
        link: Some(format!("/fleet/health#fleet-component-{key}")),
        since: None,
    };
    vec![
        fleet(
            "queue-wait",
            "Jobs wait too long to start",
            "Queue wait scores 61: queue p90 was 4 min 30 s against a 1 min target.",
        ),
        fleet(
            "start-reliability",
            "Runner starts fail too often",
            "Start reliability scores 61: 91.6% of runner starts succeeded against a 99% target.",
        ),
        host_concern(
            "host-score",
            JUPITER,
            "Jupiter scores 63",
            "Jupiter's weakest part is Start reliability (0): 62.5% of its runner starts succeeded.",
        ),
        Concern {
            key: format!("host-slow:{CALLISTO}:2000"),
            ..host_concern(
                "host-slow",
                CALLISTO,
                "callisto is slow at 2 CPUs",
                "callisto takes 1.42× as long as the fleet at 2 CPUs; most behind it: acme/app / CI / e2e (14 min 30 s against 10 min).",
            )
        },
        host_concern(
            "host-idle-while-queued",
            JUPITER,
            "Jupiter is idle while jobs wait",
            "Jupiter was busy 20% of its CPU budget while the fleet's queue wait scored 61.",
        ),
        host_concern(
            "host-idle-while-queued",
            EUROPA,
            "europa is idle while jobs wait",
            "europa was busy 25% of its CPU budget while the fleet's queue wait scored 61.",
        ),
        Concern {
            key: "category-other".to_owned(),
            severity: Severity::Info,
            title: "Many jobs have no category".to_owned(),
            detail: "25% of the window's jobs are `other`: a job category rule may be missing."
                .to_owned(),
            subject: ConcernSubject::Fleet,
            link: Some("/settings/general".to_owned()),
            since: None,
        },
    ]
}

/// The windowed half of the fixture. It is the same for every window.
#[must_use]
pub fn windowed_part(window: HealthWindow) -> WindowedHealth {
    use JobCategory::{Build, Check, Deploy, Other, Test};
    let _ = window;
    let none = |reason: &'static str| (None, Some(reason));
    let hosts = vec![
        host(
            IO,
            Some(99),
            host_components([
                (Some(100), None),
                (Some(100), None),
                (
                    Some(99),
                    Some("Speed index 1.01 against a 1.00 target: −0."),
                ),
                (Some(100), None),
                (
                    Some(90),
                    Some("Busy 45% of its CPU budget against a 50% target while jobs waited: −1."),
                ),
            ]),
            vec![
                usage(Test, 150, 108.6, 300_000, 800_000),
                usage(Build, 60, 43.5, 375_000, 210_000),
                usage(Check, 50, 36.2, 60_000, 210_000),
                usage(Other, 80, 57.9, 20_000, 240_000),
            ],
        ),
        host(
            JUPITER,
            Some(63),
            host_components([
                (
                    Some(75),
                    Some("Online 95.0% of the window against a 100% target: −6."),
                ),
                (
                    Some(0),
                    Some("62.5% of runner starts succeeded against a 99% target: −25."),
                ),
                (Some(100), None),
                (Some(100), None),
                (
                    Some(40),
                    Some("Busy 20% of its CPU budget against a 50% target while jobs waited: −6."),
                ),
            ]),
            vec![
                usage(Test, 100, 35.4, 300_000, 225_000),
                usage(Build, 40, 14.2, 375_000, 210_000),
                usage(Check, 40, 14.2, 60_000, 228_000),
                usage(Deploy, 20, 7.1, 210_000, 210_000),
                usage(Other, 60, 21.3, 20_000, 223_000),
            ],
        ),
        host(
            CALLISTO,
            Some(76),
            host_components([
                (
                    Some(92),
                    Some("Online 98.3% of the window against a 100% target: −2."),
                ),
                (
                    Some(84),
                    Some("96.0% of runner starts succeeded against a 99% target: −4."),
                ),
                (
                    Some(58),
                    Some("Speed index 1.42 against a 1.00 target: −11."),
                ),
                (
                    Some(50),
                    Some(
                        "The least free disk was 15 GiB, between the 20 GiB warning and 10 GiB critical floors: −8.",
                    ),
                ),
                (Some(100), None),
            ]),
            vec![
                usage(Test, 70, 46.1, 300_000, 232_000),
                usage(Build, 25, 16.5, 375_000, 210_000),
                usage(Check, 30, 19.7, 60_000, 255_000),
                usage(Other, 50, 32.9, 20_000, 192_000),
            ],
        ),
        host(
            EUROPA,
            Some(88),
            host_components([
                (
                    Some(79),
                    Some("Online 95.8% of the window against a 100% target: −5."),
                ),
                (Some(100), None),
                (
                    Some(92),
                    Some("Speed index 1.08 against a 1.00 target: −2."),
                ),
                (Some(100), None),
                (
                    Some(50),
                    Some("Busy 25% of its CPU budget against a 50% target while jobs waited: −5."),
                ),
            ]),
            vec![
                usage(Test, 80, 27.3, 300_000, 220_000),
                usage(Build, 25, 8.5, 375_000, 270_000),
                usage(Check, 30, 10.2, 60_000, 210_000),
                usage(Deploy, 30, 10.2, 210_000, 240_000),
                usage(Other, 60, 20.5, 20_000, 223_000),
            ],
        ),
        host(
            MACBOOK,
            None,
            host_components([
                none("Not enough data: no samples from this machine in the window."),
                none("Not enough data: no runner starts in the window."),
                none("Not enough data: fewer than 3 job keys qualify."),
                none("Not enough data: no disk or memory samples in the window."),
                none("Not enough data: the machine has no CPU budget."),
            ]),
            Vec::new(),
        ),
    ];
    let trend = [20, 16, 12, 8, 4, 0]
        .into_iter()
        .zip([79, 78, 76, 75, 74, 74])
        .map(|(hours_ago, score)| TrendPoint {
            at: FIXTURE_NOW - hours_ago * HOUR_MILLIS,
            score: Some(score),
        })
        .collect();
    WindowedHealth {
        fleet: FleetSummary {
            score: Some(74),
            components: fleet_components(),
            trend,
            categories: vec![
                usage(Test, 400, 217.5, 300_000, 393_000),
                usage(Build, 150, 82.6, 375_000, 221_000),
                usage(Check, 150, 80.4, 60_000, 224_000),
                usage(Deploy, 50, 17.3, 210_000, 228_000),
                usage(Other, 250, 132.6, 20_000, 223_000),
            ],
        },
        hosts,
        concerns: windowed_concerns(),
    }
}

/// The live half of the fixture.
#[must_use]
pub fn live_part() -> LiveHealth {
    let live =
        |key: String, severity, title: &str, detail: &str, subject, link: &str, ago| Concern {
            key,
            severity,
            title: title.to_owned(),
            detail: detail.to_owned(),
            subject,
            link: Some(link.to_owned()),
            since: Some(FIXTURE_NOW - ago),
        };
    let machine = |id: &str| format!("/fleet/machines/{id}");
    LiveHealth {
        fleet: fleet_live(),
        hosts: hosts_live(),
        concerns: vec![
            live(
                format!("host-offline:{EUROPA}"),
                Severity::Critical,
                "europa is offline",
                "europa has sent no sample for 12 min, so nothing can start there.",
                ConcernSubject::Host(EUROPA.to_owned()),
                &machine(EUROPA),
                12 * MINUTE,
            ),
            live(
                "unmatched-jobs".to_owned(),
                Severity::Critical,
                "3 queued jobs match no pool",
                "3 queued jobs ask for labels no runner pool offers; they will wait until one does.",
                ConcernSubject::Fleet,
                "/queued-jobs",
                6 * MINUTE,
            ),
            live(
                format!("start-gate:{JUPITER}"),
                Severity::Warning,
                "Jupiter's starts are paused",
                "3 runner starts failed on Jupiter in 15 min, so GridOps has paused starting runners there.",
                ConcernSubject::Host(JUPITER.to_owned()),
                &machine(JUPITER),
                9 * MINUTE,
            ),
            live(
                format!("disk:{CALLISTO}"),
                Severity::Warning,
                "callisto is low on disk",
                "callisto has 16 GiB free of 200 GiB, under its 20 GiB warning floor.",
                ConcernSubject::Host(CALLISTO.to_owned()),
                &machine(CALLISTO),
                3 * HOUR_MILLIS,
            ),
            live(
                "oldest-queued".to_owned(),
                Severity::Warning,
                "A job has waited 7 min",
                "integration tests (2/4) in acme/app has waited 7 min to start.",
                ConcernSubject::Fleet,
                "/queued-jobs",
                7 * MINUTE,
            ),
            live(
                format!("uncertain:{IO}"),
                Severity::Warning,
                "io has an uncertain placement",
                "1 placement on io is uncertain: GridOps can't tell whether its runner is running.",
                ConcernSubject::Host(IO.to_owned()),
                &machine(IO),
                25 * MINUTE,
            ),
        ],
    }
}

/// The fake source's response for `window` (only `window` differs).
#[must_use]
pub fn fixture(window: HealthWindow) -> FleetHealth {
    assemble(window, FIXTURE_NOW, windowed_part(window), live_part())
}

struct State {
    windowed_calls: AtomicUsize,
    live_calls: AtomicUsize,
    failing: AtomicBool,
    held: watch::Sender<bool>,
}

/// The source tests inject: the fixture's halves, with
/// call counts, failure on demand, and calls held until released.
#[derive(Clone)]
pub struct FakeFleetHealthSource {
    state: Arc<State>,
}

impl Default for FakeFleetHealthSource {
    fn default() -> Self {
        Self {
            state: Arc::new(State {
                windowed_calls: AtomicUsize::new(0),
                live_calls: AtomicUsize::new(0),
                failing: AtomicBool::new(false),
                held: watch::Sender::new(false),
            }),
        }
    }
}

impl std::fmt::Debug for FakeFleetHealthSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FakeFleetHealthSource")
            .field("windowed_calls", &self.windowed_calls())
            .field("live_calls", &self.live_calls())
            .finish_non_exhaustive()
    }
}

impl FakeFleetHealthSource {
    /// Calls of `windowed`, counted when they start.
    #[must_use]
    pub fn windowed_calls(&self) -> usize {
        self.state.windowed_calls.load(Ordering::SeqCst)
    }

    /// Calls of `live`, counted when they start.
    #[must_use]
    pub fn live_calls(&self) -> usize {
        self.state.live_calls.load(Ordering::SeqCst)
    }

    /// Makes every later call fail (`true`) or succeed (`false`).
    pub fn set_failing(&self, failing: bool) {
        self.state.failing.store(failing, Ordering::SeqCst);
    }

    /// Makes every call wait until [`Self::release`].
    pub fn hold(&self) {
        self.state.held.send_replace(true);
    }

    /// Lets held and later calls through.
    pub fn release(&self) {
        self.state.held.send_replace(false);
    }

    async fn pass(&self) -> anyhow::Result<()> {
        let mut held = self.state.held.subscribe();
        held.wait_for(|held| !*held)
            .await
            .map_err(|_| anyhow::anyhow!("the fake fleet health source was dropped"))?;
        if self.state.failing.load(Ordering::SeqCst) {
            anyhow::bail!("the fake fleet health source is failing");
        }
        Ok(())
    }
}

#[async_trait]
impl FleetHealthSource for FakeFleetHealthSource {
    async fn windowed(&self, window: HealthWindow, now: i64) -> anyhow::Result<WindowedHealth> {
        let _ = now;
        self.state.windowed_calls.fetch_add(1, Ordering::SeqCst);
        self.pass().await?;
        Ok(windowed_part(window))
    }

    async fn live(&self, now: i64) -> anyhow::Result<LiveHealth> {
        let _ = now;
        self.state.live_calls.fetch_add(1, Ordering::SeqCst);
        self.pass().await?;
        Ok(live_part())
    }
}

#[cfg(test)]
mod tests {
    //! The fixture agrees with the engine where the engine decides the
    //! wording (PR-Agent 9d29dd7ab80f on #253's PR).

    use super::{HealthWindow, windowed_part};
    use crate::fleet_health::{ScoreComponent, concerns};

    /// Each fleet-component concern links to its component's row on the
    /// fleet score card, as `concerns::fleet_component` links it (#253).
    #[test]
    fn the_fleet_component_concerns_link_to_their_rows() {
        for concern in windowed_part(HealthWindow::Day)
            .concerns
            .iter()
            .filter(|concern| concern.key.starts_with("fleet-component:"))
        {
            let key = concern
                .key
                .trim_start_matches("fleet-component:")
                .to_owned();
            let engine = concerns::fleet_component(&ScoreComponent {
                key: key.clone(),
                label: key.clone(),
                weight: 20.0,
                score: Some(60),
                deduction: None,
            });
            assert_eq!(
                concern.link,
                engine.and_then(|engine| engine.link),
                "{}",
                concern.key
            );
        }
    }

    /// `src/features/fleet-health/fixture.json` from the fake: run with
    /// `GRIDOPS_REGENERATE_FLEET_HEALTH_FIXTURE=1` after changing it (the
    /// contract tests then check the file, and the `OpenAPI` example, against
    /// it).
    #[test]
    #[ignore = "writes src/features/fleet-health/fixture.json"]
    fn regenerate_the_fixture() -> anyhow::Result<()> {
        if std::env::var_os("GRIDOPS_REGENERATE_FLEET_HEALTH_FIXTURE").is_none() {
            return Ok(());
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src/features/fleet-health/fixture.json");
        let json = serde_json::to_string_pretty(&super::fixture(HealthWindow::Day))?;
        std::fs::write(path, json + "\n")?;
        Ok(())
    }
}
