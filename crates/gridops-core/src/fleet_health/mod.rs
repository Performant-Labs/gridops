//! Fleet health (#197, epic #198): the shared contract every wave-1 issue
//! builds on. `docs/design/fleet-health.md` is the source of truth for every
//! rule, weight, threshold and wire field named here; these types are its
//! Rust form, `src/features/fleet-health/types.ts` its TypeScript form, and
//! `src/features/fleet-health/fixture.json` (the fake source's response) ties
//! the two together.
//!
//! The modules, and who fills them in:
//! - [`category`]: the job classifier (implemented here);
//! - [`classifier_cache`]: one classifier per process, and the seam the job
//!   writers bind `category` through (#190);
//! - [`score`] and [`concerns`]: the scoring engine and the windowed concerns,
//!   pure (#192);
//! - [`speed`]: the relative speed index (#193);
//! - [`fake`]: the fixture source the endpoint serves until #204;
//! - `testdata` and `speed_check` (behind `test-support`): the seeded fleet,
//!   the synthetic week and the database speed check the contract tests use.
//!
//! Every type here belongs to #197: a wave-1 issue uses them as they are.

pub mod category;
pub mod classifier_cache;
pub mod concerns;
pub mod fake;
pub mod score;
pub mod speed;
#[cfg(any(test, feature = "test-support"))]
pub mod speed_check;
#[cfg(any(test, feature = "test-support"))]
pub mod testdata;

// The wave-1 contract tests in this crate. They are #197's whatever issue
// fills them in (D-3(e)): a wave-1 agent removes its markers and edits
// nothing here.
#[cfg(test)]
mod classifier_cache_contract_tests;
#[cfg(test)]
mod score_contract_tests;
#[cfg(test)]
mod speed_contract_tests;
// #246: the tests that kill the mutants `cargo mutants` found surviving in
// `score.rs`, `concerns.rs` and `speed.rs`.
#[cfg(test)]
mod mutation_tests;

use std::{collections::BTreeMap, fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// The host and pool key of a job no managed placement ran
/// (`fleet_health_hourly.host_id` and `pool_id`).
pub const UNMANAGED: &str = "unmanaged";

/// Milliseconds in an hour; every rollup hour is a multiple of it.
pub const HOUR_MILLIS: i64 = 3_600_000;

/// How long an ended hour waits before the rollups write it: no hour, listed
/// or caught up, is rolled before `hour_end + ROLLUP_GRACE_MILLIS` (#239), so
/// a job or sample recorded late (a missed webhook the poll finds, an agent
/// that reports late) still counts in its own hour. The one copy of the rule:
/// #204's source reads the hourly tables only for hours past it and computes
/// every later hour of the window live.
pub const ROLLUP_GRACE_MILLIS: i64 = 15 * 60_000;

/// The number of fixed histogram buckets (`fleet_health_hourly`).
pub const HISTOGRAM_BUCKETS: usize = 9;

/// The lower bound of each histogram bucket, in milliseconds: 0–10 s,
/// 10–30 s, 30–60 s, 1–2, 2–5, 5–10, 10–20, 20–40 and 40+ min. A value
/// belongs to the last bucket whose lower bound it reaches.
pub const HISTOGRAM_LOWER_BOUNDS_MILLIS: [i64; HISTOGRAM_BUCKETS] = [
    0, 10_000, 30_000, 60_000, 120_000, 300_000, 600_000, 1_200_000, 2_400_000,
];

/// A histogram of wait or duration counts in [`HISTOGRAM_BUCKETS`] buckets.
pub type Histogram = [u64; HISTOGRAM_BUCKETS];

/// The bucket `millis` falls in, lower bound inclusive. A negative value
/// (clock skew between GitHub and `GridOps`) counts as no wait.
#[must_use]
pub fn histogram_bucket(millis: i64) -> usize {
    HISTOGRAM_LOWER_BOUNDS_MILLIS
        .iter()
        .rposition(|lower| millis >= *lower)
        .unwrap_or(0)
}

/// What kind of work a job is (#190). Stored jobs are never `fix-agent`:
/// #201 counts fix-agent work from fix placements, so it only appears in the
/// live counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobCategory {
    Test,
    Build,
    Check,
    Deploy,
    FixAgent,
    Other,
}

impl JobCategory {
    /// Every category, in display order.
    pub const ALL: [Self; 6] = [
        Self::Test,
        Self::Build,
        Self::Check,
        Self::Deploy,
        Self::FixAgent,
        Self::Other,
    ];

    /// The categories a job row or a rule may hold (every one but
    /// `fix-agent`).
    pub const STORED: [Self; 5] = [
        Self::Test,
        Self::Build,
        Self::Check,
        Self::Deploy,
        Self::Other,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Build => "build",
            Self::Check => "check",
            Self::Deploy => "deploy",
            Self::FixAgent => "fix-agent",
            Self::Other => "other",
        }
    }

    /// The stored category of a column value; NULL (`None`) counts as
    /// `other`, as every reader of a not-yet-classified job counts it.
    #[must_use]
    pub fn from_column(value: Option<&str>) -> Self {
        value
            .and_then(|value| value.parse().ok())
            .unwrap_or(Self::Other)
    }
}

impl fmt::Display for JobCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A spelling that is not one of [`JobCategory::ALL`] or [`HealthWindow::ALL`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown value {0:?}")]
pub struct UnknownValue(pub String);

impl FromStr for JobCategory {
    type Err = UnknownValue;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|category| category.as_str() == value)
            .ok_or_else(|| UnknownValue(value.to_owned()))
    }
}

/// The window the page and the endpoint report over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HealthWindow {
    #[serde(rename = "1h")]
    Hour,
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
}

impl HealthWindow {
    pub const ALL: [Self; 3] = [Self::Hour, Self::Day, Self::Week];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hour => "1h",
            Self::Day => "24h",
            Self::Week => "7d",
        }
    }

    /// The window's length in milliseconds.
    #[must_use]
    pub const fn millis(self) -> i64 {
        match self {
            Self::Hour => HOUR_MILLIS,
            Self::Day => 24 * HOUR_MILLIS,
            Self::Week => 7 * 24 * HOUR_MILLIS,
        }
    }
}

impl fmt::Display for HealthWindow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for HealthWindow {
    type Err = UnknownValue;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|window| window.as_str() == value)
            .ok_or_else(|| UnknownValue(value.to_owned()))
    }
}

/// How bad a concern is. Ordered worst first: `Critical < Warning < Info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

/// What a concern is about. On the wire: `{"kind": "fleet"}`,
/// `{"kind": "host", "id": "<host id>"}` or `{"kind": "pool", "id": "<pool
/// id>"}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub enum ConcernSubject {
    Fleet,
    Host(String),
    Pool(String),
}

/// One area of concern. `key` is stable for the same condition (for example
/// `host-offline:<host id>`), so the page keeps a row in place across
/// refreshes; `title` is short; `detail` is one sentence with the numbers;
/// `link` is where in the app it is explained; `since` (epoch ms) is when the
/// condition started, when known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Concern {
    pub key: String,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub subject: ConcernSubject,
    pub link: Option<String>,
    pub since: Option<i64>,
}

/// One part of a score. `weight` is its share of 100 after any
/// not-enough-data reweighting; `score` is `None` when there is not enough
/// data; `deduction` is the sentence that explains the points lost (or why
/// there is no score), `None` at 100.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScoreComponent {
    pub key: String,
    pub label: String,
    pub weight: f64,
    pub score: Option<u8>,
    pub deduction: Option<String>,
}

/// The windowed usage of one job category, for the fleet or one machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CategoryUsage {
    pub category: JobCategory,
    /// Final-attempt jobs completed in the window.
    pub jobs: u32,
    /// CPU-hours reserved by those jobs (`cpu_millis / 1000 × hours`).
    pub cpu_hours: f64,
    /// Estimated from the duration histograms; `None` without jobs.
    pub median_duration_ms: Option<i64>,
    /// Estimated from the queue-wait histograms; `None` without jobs.
    pub p90_queue_wait_ms: Option<i64>,
}

/// CPU-hours by category in one hour of the window, for the usage chart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HourlyUsage {
    pub hour: i64,
    pub cpu_hours: BTreeMap<JobCategory, f64>,
}

/// One point of the fleet score trend (#204's snapshots).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrendPoint {
    pub at: i64,
    pub score: Option<u8>,
}

/// The fleet's windowed score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FleetSummary {
    pub score: Option<u8>,
    pub components: Vec<ScoreComponent>,
    /// Oldest first.
    pub trend: Vec<TrendPoint>,
    pub categories: Vec<CategoryUsage>,
}

/// One job key behind a speed index (#193): this machine's median at the
/// runner size against the fleet median, with both run counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeedKey {
    pub key: String,
    pub host_median_ms: i64,
    pub fleet_median_ms: i64,
    pub host_runs: u32,
    pub fleet_runs: u32,
}

/// #193's output for one (machine, runner size). `index` is `None` when
/// fewer than 3 job keys qualify ("not enough data"); `top_keys` are the (at
/// most 5) keys that push it up most.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeedResult {
    pub host_id: String,
    pub cpu_millis: u32,
    pub index: Option<f64>,
    pub top_keys: Vec<SpeedKey>,
}

/// A machine's speed index at one runner size, as the page shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostSpeed {
    pub cpu_millis: u32,
    pub index: Option<f64>,
    pub top_keys: Vec<SpeedKey>,
}

impl From<SpeedResult> for HostSpeed {
    fn from(result: SpeedResult) -> Self {
        Self {
            cpu_millis: result.cpu_millis,
            index: result.index,
            top_keys: result.top_keys,
        }
    }
}

/// What is true of one machine right now (#201).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostLiveStats {
    pub host_id: String,
    /// A sample in the last 5 minutes.
    pub online: bool,
    /// The newest sample's `observed_at`; `None` without any.
    pub last_seen_at: Option<i64>,
    /// Runners running a job now, by the job's category (NULL as `other`),
    /// plus running fix-agent placements as `fix-agent`.
    pub running_by_category: BTreeMap<JobCategory, u32>,
    /// Charged allocations on the machine's root domain.
    pub cpu_reserved_millis: u32,
    pub memory_reserved_mib: u32,
    /// From the latest root sample (`/fleet/hosts`' `latestRootMeasurement`).
    pub cpu_used_millis: Option<u32>,
    pub memory_used_mib: Option<u32>,
    pub disk_free_bytes: Option<u64>,
    /// Start failures in the last hour: the machine's `placement_failed`
    /// events that `start_health::is_start_failure` puts on it, as the start
    /// gate and the machine rollup count them (not an operator's release or
    /// a runner removed before its start, #253).
    pub start_failures_last_hour: u32,
}

/// The fleet's pool with jobs waiting for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoolQueue {
    pub pool_id: String,
    pub pool_name: String,
    pub queued: u32,
}

/// The job that has waited longest to start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OldestQueued {
    /// The platform's own job id: a GitHub workflow job id or a GitLab job
    /// id (#253), so not unique across platforms.
    pub job_id: i64,
    pub job_name: String,
    /// The GitHub repository, or the GitLab project path.
    pub repository: String,
    pub queued_at: i64,
    pub wait_ms: i64,
}

/// What is true of the fleet right now (#201).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FleetLiveStats {
    pub queued_total: u32,
    /// Adds up to `queued_total`; NULL counts as `other`.
    pub queued_by_category: BTreeMap<JobCategory, u32>,
    /// Waiting and manual jobs by the pool they are under, most first.
    pub queued_by_pool: Vec<PoolQueue>,
    /// Queued jobs no pool can run.
    pub queued_unmatched: u32,
    pub oldest_queued: Option<OldestQueued>,
    pub running_total: u32,
    /// Adds up to `running_total`.
    pub running_by_category: BTreeMap<JobCategory, u32>,
}

/// One machine on the page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostHealth {
    pub id: String,
    pub name: String,
    pub cpu_budget_millis: u32,
    pub memory_budget_mib: u32,
    pub disk_total_bytes: Option<u64>,
    pub score: Option<u8>,
    pub components: Vec<ScoreComponent>,
    /// One entry per runner size it ran at in the window.
    pub speed: Vec<HostSpeed>,
    pub categories: Vec<CategoryUsage>,
    /// Oldest first, one entry per hour of the window with any usage.
    pub usage: Vec<HourlyUsage>,
    /// `None` in the windowed part; [`assemble`] joins the live stats.
    pub live: Option<HostLiveStats>,
}

/// `GET /api/v1/fleet/health`'s response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FleetHealth {
    pub window: HealthWindow,
    pub generated_at: i64,
    /// The source failed and this is the last good response (#194).
    pub stale: bool,
    pub fleet: FleetSummary,
    pub live: FleetLiveStats,
    /// Worst score first is the page's order, not the wire's: the wire
    /// lists machines in id order.
    pub hosts: Vec<HostHealth>,
    /// Merged and ordered by [`merge_concerns`].
    pub concerns: Vec<Concern>,
}

/// The windowed half of a response: scores, speed, usage and the windowed
/// concerns (#192, #193, #204).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowedHealth {
    pub fleet: FleetSummary,
    pub hosts: Vec<HostHealth>,
    pub concerns: Vec<Concern>,
}

/// The live half of a response (#201's `collect_live`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveHealth {
    pub fleet: FleetLiveStats,
    pub hosts: Vec<HostLiveStats>,
    pub concerns: Vec<Concern>,
}

/// Where the endpoint gets its data. The windowed part is the slow, cached
/// part; the live part is a bounded read of now. #194's handler holds an
/// `Arc<dyn FleetHealthSource>`: [`fake::FakeFleetHealthSource`] until #204
/// plugs in the database source.
#[async_trait]
pub trait FleetHealthSource: Send + Sync {
    async fn windowed(&self, window: HealthWindow, now: i64) -> anyhow::Result<WindowedHealth>;
    async fn live(&self, now: i64) -> anyhow::Result<LiveHealth>;
}

/// One response from its two halves: each machine gets its live stats (by
/// id), and the concerns are merged.
#[must_use]
pub fn assemble(
    window: HealthWindow,
    generated_at: i64,
    windowed: WindowedHealth,
    live: LiveHealth,
) -> FleetHealth {
    let hosts = windowed
        .hosts
        .into_iter()
        .map(|mut host| {
            host.live = live
                .hosts
                .iter()
                .find(|stats| stats.host_id == host.id)
                .cloned();
            host
        })
        .collect();
    FleetHealth {
        window,
        generated_at,
        stale: false,
        fleet: windowed.fleet,
        live: live.fleet,
        hosts,
        concerns: merge_concerns(windowed.concerns, live.concerns),
    }
}

/// The page's concern list: one concern per key (a live concern replaces a
/// windowed one with the same key), worst severity first, then the oldest
/// `since` first, with no `since` after every dated concern of its severity.
/// Ties keep their order (windowed before live).
#[must_use]
pub fn merge_concerns(windowed: Vec<Concern>, live: Vec<Concern>) -> Vec<Concern> {
    let mut merged: Vec<Concern> = windowed
        .into_iter()
        .filter(|concern| live.iter().all(|other| other.key != concern.key))
        .collect();
    merged.extend(live);
    merged.sort_by_key(|concern| {
        (
            concern.severity,
            concern.since.is_none(),
            concern.since.unwrap_or(0),
        )
    });
    merged
}

// The scoring engine's inputs (#192): rollup rows, host rollup rows, speed
// results, live stats and targets. #204's database source fills them; the
// fake builds them for the fixture.

/// The machine a rollup row belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RollupHost {
    Managed(String),
    /// Stored as [`UNMANAGED`] in `host_id` and `pool_id`.
    Unmanaged,
}

impl RollupHost {
    /// The `fleet_health_hourly.host_id` value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Managed(id) => id,
            Self::Unmanaged => UNMANAGED,
        }
    }

    #[must_use]
    pub fn from_column(value: &str) -> Self {
        if value == UNMANAGED {
            Self::Unmanaged
        } else {
            Self::Managed(value.to_owned())
        }
    }
}

/// A `fleet_health_hourly` row (#191).
#[derive(Debug, Clone, PartialEq)]
pub struct HourlyJobRow {
    pub hour: i64,
    pub host: RollupHost,
    /// [`UNMANAGED`] for an unmanaged runner.
    pub pool_id: String,
    pub category: JobCategory,
    pub job_count: u64,
    pub completed_count: u64,
    pub infra_failure_count: u64,
    pub duration_ms_sum: i64,
    pub cpu_seconds_sum: f64,
    pub wait_histogram: Histogram,
    pub duration_histogram: Histogram,
}

/// A `fleet_host_hourly` row (#200).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourlyHostRow {
    pub hour: i64,
    pub host_id: String,
    pub online_minutes: u32,
    pub cpu_used_p50_millis: Option<u32>,
    pub cpu_used_p95_millis: Option<u32>,
    pub memory_used_p95_mib: Option<u32>,
    pub min_disk_free_bytes: Option<u64>,
    pub start_attempts: u32,
    /// Machine-fault start failures by `PlacementFailureReason` code.
    pub start_failures: BTreeMap<String, u32>,
    pub uncertain_placements: u32,
}

/// A machine the scores are about: its budgets and root disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInfo {
    pub id: String,
    pub name: String,
    pub cpu_budget_millis: u32,
    pub memory_budget_mib: u32,
    pub disk_total_bytes: Option<u64>,
}

/// A machine's root disk over the window: its size and the least free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskHeadroom {
    pub total_bytes: u64,
    pub min_free_bytes: u64,
}

/// The score targets (#192's "100 at" and "0 at" columns). They are
/// settings: [`Default`] gives the design doc's defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthTargets {
    /// Fewer completed jobs than this is "not enough data" for a job-based
    /// component.
    pub min_jobs: u64,
    pub queue_wait_p90_good_ms: f64,
    pub queue_wait_p90_bad_ms: f64,
    pub start_success_good: f64,
    pub start_success_bad: f64,
    pub infra_failure_good: f64,
    pub infra_failure_bad: f64,
    pub capacity_misfit_good: f64,
    pub capacity_misfit_bad: f64,
    pub fleet_availability_good: f64,
    pub fleet_availability_bad: f64,
    pub host_availability_good: f64,
    pub host_availability_bad: f64,
    pub speed_index_good: f64,
    pub speed_index_bad: f64,
    pub memory_used_good: f64,
    pub memory_used_bad: f64,
    pub busy_share_good: f64,
    pub busy_share_bad: f64,
}

impl Default for HealthTargets {
    fn default() -> Self {
        Self {
            min_jobs: 20,
            queue_wait_p90_good_ms: 60_000.0,
            queue_wait_p90_bad_ms: 600_000.0,
            start_success_good: 0.99,
            start_success_bad: 0.80,
            infra_failure_good: 0.0,
            infra_failure_bad: 0.05,
            capacity_misfit_good: 0.0,
            capacity_misfit_bad: 0.25,
            fleet_availability_good: 1.0,
            fleet_availability_bad: 0.80,
            host_availability_good: 1.0,
            host_availability_bad: 0.80,
            speed_index_good: 1.0,
            speed_index_bad: 2.0,
            memory_used_good: 0.80,
            memory_used_bad: 1.0,
            busy_share_good: 0.50,
            busy_share_bad: 0.0,
        }
    }
}

/// Everything the scoring engine reads for one window.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetHealthInputs {
    pub window: HealthWindow,
    /// The window is `[now - window, now)`.
    pub now: i64,
    pub hosts: Vec<HostInfo>,
    pub hourly: Vec<HourlyJobRow>,
    pub host_hourly: Vec<HourlyHostRow>,
    pub speed: Vec<SpeedResult>,
    pub live: FleetLiveStats,
    pub host_live: Vec<HostLiveStats>,
    pub targets: HealthTargets,
}

/// A score and the components it combines, after reweighting.
#[derive(Debug, Clone, PartialEq)]
pub struct CombinedScore {
    pub score: Option<u8>,
    pub components: Vec<ScoreComponent>,
}

/// One machine's score (#192), with its busy share for the
/// `host-idle-while-queued` concern.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredHost {
    pub host_id: String,
    pub name: String,
    pub score: Option<u8>,
    pub components: Vec<ScoreComponent>,
    pub busy_share: Option<f64>,
}

/// The fleet's score and every machine's.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredFleet {
    pub score: Option<u8>,
    pub components: Vec<ScoreComponent>,
    pub hosts: Vec<ScoredHost>,
}
