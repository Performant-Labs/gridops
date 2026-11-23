//! The scoring engine (#192): pure functions from the window's rollups to a
//! 0–100 score per component, per machine and for the fleet. No I/O here,
//! ever (`the_scoring_engine_does_no_io`).
//!
//! Every component is linear between its "100 at" and "0 at" values
//! (`HealthTargets`, `docs/design/fleet-health.md`'s two score tables),
//! clamped to 0–100 and rounded only at the end; a component with too little
//! data has `score: None` and a "not enough data" deduction, and [`combine`]
//! spreads its weight over the others. [`score_fleet`] folds a window's
//! rollup rows into the measured values the component functions score.

use crate::disk::{DiskAlarm, alarm_for};

use super::{
    CombinedScore, DiskHeadroom, FleetHealthInputs, HISTOGRAM_BUCKETS,
    HISTOGRAM_LOWER_BOUNDS_MILLIS, HOUR_MILLIS, HealthTargets, Histogram, HourlyHostRow,
    HourlyJobRow, ScoreComponent, ScoredFleet, ScoredHost,
};

/// A gibibyte, the unit the disk floors read in.
const GIB: u64 = 1024 * 1024 * 1024;

/// A value scored 100 at `good` and 0 at `bad`, more being better.
fn higher(value: f64, good: f64, bad: f64) -> f64 {
    if (good - bad).abs() <= f64::EPSILON {
        return if value >= good { 100.0 } else { 0.0 };
    }
    ((value - bad) / (good - bad) * 100.0).clamp(0.0, 100.0)
}

/// A value scored 100 at `good` and 0 at `bad`, less being better.
fn lower(value: f64, good: f64, bad: f64) -> f64 {
    if (good - bad).abs() <= f64::EPSILON {
        return if value <= good { 100.0 } else { 0.0 };
    }
    ((bad - value) / (bad - good) * 100.0).clamp(0.0, 100.0)
}

/// A duration in milliseconds as the deduction sentences read it: `45 s`,
/// `1 min`, `7 min 12 s`, `1 h 5 min`.
pub(super) fn duration(ms: f64) -> String {
    let total_seconds = (ms.max(0.0) / 1000.0).round() as i64;
    let (minutes, seconds) = (total_seconds / 60, total_seconds % 60);
    match (minutes / 60, minutes % 60, seconds) {
        (0, 0, _) => format!("{seconds} s"),
        (0, minutes, 0) => format!("{minutes} min"),
        (0, minutes, seconds) => format!("{minutes} min {seconds} s"),
        (hours, 0, _) => format!("{hours} h"),
        (hours, minutes, _) => format!("{hours} h {minutes} min"),
    }
}

/// A share as the deduction sentences read it, always one decimal: `97.4%`.
pub(super) fn percent(share: f64) -> String {
    format!("{:.1}%", share * 100.0)
}

/// A share as the sentences read it, without a trailing `.0`: `45%`, `29.9%`.
pub(super) fn whole_percent(share: f64) -> String {
    let value = share * 100.0;
    if (value - value.round()).abs() < 1e-9 {
        format!("{}%", value.round() as i64)
    } else {
        format!("{value:.1}%")
    }
}

/// A byte count as `15 GiB` / `102.4 GiB`.
fn gib(bytes: u64) -> String {
    let value = bytes as f64 / GIB as f64;
    if (value - value.round()).abs() < 0.05 {
        format!("{value:.0} GiB")
    } else {
        format!("{value:.1} GiB")
    }
}

/// One component: `measured` scored through `scored` (linear, clamped),
/// rounded to `u8`, with the deduction sentence `"{body}: −N."` unless it
/// scored 100. `None` is "not enough data". The points lost are
/// `round(weight × (100 − raw) / 100)`, from the unrounded score, so a
/// component just under 100 can lose a rounded `0` points and still say so
/// (`−0.`, as the fixture's own "Speed index 1.01 …: −0." does).
fn component<T: Copy>(
    key: &str,
    label: &str,
    weight: f64,
    measured: Option<T>,
    scored: impl FnOnce(T) -> f64,
    body: impl FnOnce(T) -> String,
    no_data: &str,
) -> ScoreComponent {
    let Some(value) = measured else {
        return ScoreComponent {
            key: key.to_owned(),
            label: label.to_owned(),
            weight,
            score: None,
            deduction: Some(format!("Not enough data: {no_data}.")),
        };
    };
    let raw = scored(value);
    let score = raw.round() as u8;
    let lost = (weight * (100.0 - raw) / 100.0).round() as i64;
    ScoreComponent {
        key: key.to_owned(),
        label: label.to_owned(),
        weight,
        score: Some(score),
        deduction: (score < 100).then(|| format!("{}: \u{2212}{lost}.", body(value))),
    }
}

/// Queue wait, from the window's p90 queue wait in milliseconds.
#[must_use]
pub fn fleet_queue_wait(p90_ms: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    component(
        "queue-wait",
        "Queue wait",
        40.0,
        p90_ms,
        |p90| {
            lower(
                p90,
                targets.queue_wait_p90_good_ms,
                targets.queue_wait_p90_bad_ms,
            )
        },
        |p90| {
            format!(
                "Queue p90 was {} against a {} target",
                duration(p90),
                duration(targets.queue_wait_p90_good_ms)
            )
        },
        &format!(
            "fewer than {} jobs completed in the window",
            targets.min_jobs
        ),
    )
}

/// The shared shape of the fleet's and a machine's start reliability.
fn starts(success: Option<f64>, targets: &HealthTargets, weight: f64) -> ScoreComponent {
    component(
        "start-reliability",
        "Start reliability",
        weight,
        success,
        |share| higher(share, targets.start_success_good, targets.start_success_bad),
        |share| {
            format!(
                "{} of runner starts succeeded against a {} target",
                percent(share),
                whole_percent(targets.start_success_good)
            )
        },
        &format!(
            "fewer than {} runner starts in the window",
            targets.min_jobs
        ),
    )
}

/// Start reliability, from the share of runner starts that succeeded.
#[must_use]
pub fn fleet_start_reliability(success: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    starts(success, targets, 20.0)
}

/// Infrastructure failures, from their share of completed jobs.
#[must_use]
pub fn fleet_infra_failures(share: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    component(
        "infra-failures",
        "Infrastructure failures",
        15.0,
        share,
        |share| lower(share, targets.infra_failure_good, targets.infra_failure_bad),
        |share| {
            format!(
                "{} of completed jobs failed on the infrastructure against a {} target",
                percent(share),
                whole_percent(targets.infra_failure_good)
            )
        },
        &format!(
            "fewer than {} jobs completed in the window",
            targets.min_jobs
        ),
    )
}

/// Capacity fit, from the share of hours jobs waited while CPU sat idle.
#[must_use]
pub fn fleet_capacity_fit(misfit: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    component(
        "capacity-fit",
        "Capacity fit",
        15.0,
        misfit,
        |share| {
            lower(
                share,
                targets.capacity_misfit_good,
                targets.capacity_misfit_bad,
            )
        },
        |share| {
            format!(
                "{} of the window's hours waited over a minute while the fleet sat under half its CPU budget, against a {} target",
                percent(share),
                whole_percent(targets.capacity_misfit_good)
            )
        },
        &format!(
            "fewer than {} jobs completed in the window",
            targets.min_jobs
        ),
    )
}

/// Availability, from the budget-weighted share of machine-minutes online.
#[must_use]
pub fn fleet_availability(online: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    component(
        "availability",
        "Availability",
        10.0,
        online,
        |share| {
            higher(
                share,
                targets.fleet_availability_good,
                targets.fleet_availability_bad,
            )
        },
        |share| {
            format!(
                "The fleet's CPU budget was online {} of the window against a {} target",
                percent(share),
                whole_percent(targets.fleet_availability_good)
            )
        },
        "no machine samples in the window",
    )
}

/// A machine's availability, from its share of minutes online.
#[must_use]
pub fn host_availability(online: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    component(
        "availability",
        "Availability",
        25.0,
        online,
        |share| {
            higher(
                share,
                targets.host_availability_good,
                targets.host_availability_bad,
            )
        },
        |share| {
            format!(
                "Online {} of the window against a {} target",
                percent(share),
                whole_percent(targets.host_availability_good)
            )
        },
        "no samples from this machine in the window",
    )
}

/// A machine's start reliability.
#[must_use]
pub fn host_start_reliability(success: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    starts(success, targets, 25.0)
}

/// A machine's relative speed, from its speed index (#193).
#[must_use]
pub fn host_relative_speed(index: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    component(
        "relative-speed",
        "Relative speed",
        25.0,
        index,
        |index| lower(index, targets.speed_index_good, targets.speed_index_bad),
        |index| {
            format!(
                "Speed index {index:.2} against a {:.2} target",
                targets.speed_index_good
            )
        },
        "fewer than 3 job keys qualify",
    )
}

/// The free-space floors `disk::alarm_for` warns and escalates at for a disk
/// of `total_bytes`: the smallest frees that clear each alarm, found by
/// search so they stay `alarm_for`'s own. `(warning, critical)`.
fn disk_floors(total_bytes: u64) -> (u64, u64) {
    fn first_without(total_bytes: u64, alarms: impl Fn(u64) -> bool) -> u64 {
        let (mut low, mut high) = (0_u64, total_bytes.max(10 * GIB));
        while low < high {
            let mid = low + (high - low) / 2;
            if alarms(mid) {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        low
    }
    (
        first_without(total_bytes, |free| alarm_for(total_bytes, free).is_some()),
        first_without(total_bytes, |free| {
            alarm_for(total_bytes, free) == Some(DiskAlarm::Critical)
        }),
    )
}

/// A machine's headroom: the lower of its disk score (its least free root
/// disk between `disk::alarm_for`'s floors) and its memory score (p95 used
/// share of its budget).
#[must_use]
pub fn host_headroom(
    disk: Option<DiskHeadroom>,
    memory_used: Option<f64>,
    targets: &HealthTargets,
) -> ScoreComponent {
    let disk = disk.map(|disk| {
        let (warn, critical) = disk_floors(disk.total_bytes);
        (
            higher(disk.min_free_bytes as f64, warn as f64, critical as f64),
            format!(
                "The least free disk was {}, between the {} warning and {} critical floors",
                gib(disk.min_free_bytes),
                gib(warn),
                gib(critical)
            ),
        )
    });
    let memory = memory_used.map(|used| {
        (
            lower(used, targets.memory_used_good, targets.memory_used_bad),
            format!(
                "Memory p95 was {} of budget against a {} target",
                whole_percent(used),
                whole_percent(targets.memory_used_good)
            ),
        )
    });
    let (raw, body) = match (disk, memory) {
        (Some((disk, disk_body)), Some((memory, memory_body))) => {
            if memory < disk {
                (Some(memory), memory_body)
            } else {
                (Some(disk), disk_body)
            }
        }
        (Some((disk, disk_body)), None) => (Some(disk), disk_body),
        (None, Some((memory, memory_body))) => (Some(memory), memory_body),
        (None, None) => (None, String::new()),
    };
    component(
        "headroom",
        "Headroom",
        15.0,
        raw,
        |raw| raw,
        move |_| body,
        "no disk or memory samples in the window",
    )
}

/// A machine's busy share (CPU-seconds reserved over its budget), deducted
/// only while the fleet's queue-wait component is under 100.
#[must_use]
pub fn host_busy_share(
    busy: Option<f64>,
    fleet_queue: Option<u8>,
    targets: &HealthTargets,
) -> ScoreComponent {
    let deducts = fleet_queue.is_some_and(|queue| queue < 100);
    component(
        "busy-share",
        "Busy share",
        10.0,
        busy,
        |busy| {
            if deducts {
                higher(busy, targets.busy_share_good, targets.busy_share_bad)
            } else {
                100.0
            }
        },
        |busy| {
            format!(
                "Busy {} of its CPU budget against a {} target while jobs waited",
                whole_percent(busy),
                whole_percent(targets.busy_share_good)
            )
        },
        "the machine has no CPU budget",
    )
}

/// The weighted score of `components`, a component without a score spreading
/// its weight over the others in proportion. `None` when none has a score.
#[must_use]
pub fn combine(components: &[ScoreComponent]) -> CombinedScore {
    let scored_weight: f64 = components
        .iter()
        .filter_map(|component| component.score.map(|_| component.weight))
        .sum();
    let scaled: Vec<ScoreComponent> = components
        .iter()
        .map(|component| ScoreComponent {
            weight: match component.score {
                Some(_) if scored_weight > 0.0 => component.weight * 100.0 / scored_weight,
                None if scored_weight == 0.0 => component.weight,
                _ => 0.0,
            },
            ..component.clone()
        })
        .collect();
    let score = (scored_weight > 0.0).then(|| {
        let weighted: f64 = scaled
            .iter()
            .filter_map(|component| {
                component
                    .score
                    .map(|score| f64::from(score) * component.weight)
            })
            .sum();
        (weighted / 100.0).round() as u8
    });
    CombinedScore {
        score,
        components: scaled,
    }
}

/// The `quantile` (0..=1) of a bucket histogram, interpolated inside the
/// bucket it falls in, in milliseconds. `None` for an empty histogram. The
/// 40+ min bucket counts as its lower bound: 40 min.
#[must_use]
pub fn percentile(histogram: &[u64; HISTOGRAM_BUCKETS], quantile: f64) -> Option<f64> {
    let total: u64 = histogram.iter().sum();
    if total == 0 {
        return None;
    }
    let target = quantile.clamp(0.0, 1.0) * total as f64;
    let mut before = 0_u64;
    for (index, &count) in histogram.iter().enumerate() {
        if count > 0 && before as f64 + count as f64 >= target {
            let lower = HISTOGRAM_LOWER_BOUNDS_MILLIS[index] as f64;
            let upper = HISTOGRAM_LOWER_BOUNDS_MILLIS
                .get(index + 1)
                .map_or(lower, |bound| *bound as f64);
            let position = ((target - before as f64) / count as f64).clamp(0.0, 1.0);
            return Some(lower + position * (upper - lower));
        }
        before += count;
    }
    None
}

/// The share of `attempts` that succeeded, or `None` under
/// `targets.min_jobs` (which "not enough data" means).
fn start_success(attempts: u32, failures: u32, targets: &HealthTargets) -> Option<f64> {
    (u64::from(attempts) >= targets.min_jobs && attempts > 0)
        .then(|| f64::from(attempts.saturating_sub(failures)) / f64::from(attempts))
}

/// The fleet's budget-weighted share of machine-minutes online. A machine
/// with no rows in the window counts as offline, at its full budget.
fn fleet_online_share(inputs: &FleetHealthInputs, rows: &[&HourlyHostRow]) -> Option<f64> {
    let window_minutes = inputs.window.millis() / 60_000;
    let mut online = 0.0_f64;
    let mut budget = 0.0_f64;
    for host in &inputs.hosts {
        let minutes: u32 = rows
            .iter()
            .filter(|row| row.host_id == host.id)
            .map(|row| row.online_minutes)
            .sum();
        online += f64::from(minutes) * f64::from(host.cpu_budget_millis);
        budget += f64::from(host.cpu_budget_millis);
    }
    (budget > 0.0 && window_minutes > 0).then(|| online / (budget * window_minutes as f64))
}

/// `value` divided by `by`, rounded up (both positive or negative).
fn ceil_div(value: i64, by: i64) -> i64 {
    value / by + i64::from(value % by != 0 && (value >= 0) == (by >= 0))
}

/// The share of the window's hours in which jobs waited over a minute while
/// the fleet's sampled machines sat under half their sampled CPU budget.
fn capacity_misfit(
    inputs: &FleetHealthInputs,
    jobs: &[&HourlyJobRow],
    rows: &[&HourlyHostRow],
) -> Option<f64> {
    let first = ceil_div(inputs.now - inputs.window.millis(), HOUR_MILLIS) * HOUR_MILLIS;
    let hours = (inputs.now - first) / HOUR_MILLIS;
    let mut misfit = 0_u32;
    let mut counted = 0_u32;
    for index in 0..hours {
        let hour = first + index * HOUR_MILLIS;
        counted += 1;
        let waiting: u64 = jobs
            .iter()
            .filter(|row| row.hour == hour)
            .map(|row| {
                row.wait_histogram
                    .iter()
                    .enumerate()
                    .filter(|(bucket, _)| HISTOGRAM_LOWER_BOUNDS_MILLIS[*bucket] >= 60_000)
                    .map(|(_, count)| *count)
                    .sum::<u64>()
            })
            .sum();
        if waiting == 0 {
            continue;
        }
        let mut used = 0.0_f64;
        let mut budget = 0.0_f64;
        for row in rows.iter().filter(|row| row.hour == hour) {
            let Some(p50) = row.cpu_used_p50_millis else {
                continue;
            };
            let Some(host) = inputs.hosts.iter().find(|host| host.id == row.host_id) else {
                continue;
            };
            used += f64::from(p50);
            budget += f64::from(host.cpu_budget_millis);
        }
        if budget > 0.0 && used < budget / 2.0 {
            misfit += 1;
        }
    }
    (counted > 0).then(|| f64::from(misfit) / f64::from(counted))
}

/// Every score for the window.
#[must_use]
pub fn score_fleet(inputs: &FleetHealthInputs) -> ScoredFleet {
    let targets = &inputs.targets;
    let start = inputs.now - inputs.window.millis();
    let in_window = |hour: i64| hour >= start && hour < inputs.now;
    let jobs: Vec<&HourlyJobRow> = inputs
        .hourly
        .iter()
        .filter(|row| in_window(row.hour))
        .collect();
    let hosts: Vec<&HourlyHostRow> = inputs
        .host_hourly
        .iter()
        .filter(|row| in_window(row.hour))
        .collect();

    // The fleet's five components, in the score table's order.
    let total_jobs: u64 = jobs.iter().map(|row| row.job_count).sum();
    let enough_jobs = total_jobs >= targets.min_jobs;
    let mut wait: Histogram = [0; HISTOGRAM_BUCKETS];
    for row in &jobs {
        for (bucket, count) in row.wait_histogram.iter().enumerate() {
            wait[bucket] += count;
        }
    }
    let attempts: u32 = hosts.iter().map(|row| row.start_attempts).sum();
    let failures: u32 = hosts
        .iter()
        .map(|row| row.start_failures.values().sum::<u32>())
        .sum();
    let infra_failures: u64 = jobs.iter().map(|row| row.infra_failure_count).sum();
    let completed: u64 = jobs.iter().map(|row| row.completed_count).sum();
    let queue = fleet_queue_wait(
        enough_jobs.then(|| percentile(&wait, 0.9)).flatten(),
        targets,
    );
    let fleet_components = vec![
        queue.clone(),
        fleet_start_reliability(start_success(attempts, failures, targets), targets),
        fleet_infra_failures(
            (enough_jobs && completed > 0).then(|| infra_failures as f64 / completed as f64),
            targets,
        ),
        fleet_capacity_fit(
            enough_jobs
                .then(|| capacity_misfit(inputs, &jobs, &hosts))
                .flatten(),
            targets,
        ),
        fleet_availability(fleet_online_share(inputs, &hosts), targets),
    ];
    let fleet_queue_score = queue.score;

    let window_seconds = inputs.window.millis() as f64 / 1000.0;
    let scored_hosts = inputs
        .hosts
        .iter()
        .map(|host| {
            let rows: Vec<&HourlyHostRow> = hosts
                .iter()
                .copied()
                .filter(|row| row.host_id == host.id)
                .collect();
            let job_rows: Vec<&HourlyJobRow> = jobs
                .iter()
                .copied()
                .filter(|row| row.host.as_str() == host.id)
                .collect();
            let window_minutes = inputs.window.millis() / 60_000;
            let online_minutes: u32 = rows.iter().map(|row| row.online_minutes).sum();
            let attempts: u32 = rows.iter().map(|row| row.start_attempts).sum();
            let failures: u32 = rows
                .iter()
                .map(|row| row.start_failures.values().sum::<u32>())
                .sum();
            let index = inputs
                .speed
                .iter()
                .filter(|result| result.host_id == host.id)
                .filter_map(|result| result.index)
                .reduce(f64::max);
            let disk = rows
                .iter()
                .filter_map(|row| row.min_disk_free_bytes)
                .min()
                .zip(host.disk_total_bytes)
                .filter(|&(_, total_bytes)| total_bytes > 0)
                .map(|(min_free_bytes, total_bytes)| DiskHeadroom {
                    total_bytes,
                    min_free_bytes,
                });
            let memory = rows
                .iter()
                .filter_map(|row| row.memory_used_p95_mib)
                .max()
                .filter(|_| host.memory_budget_mib > 0)
                .map(|p95| f64::from(p95) / f64::from(host.memory_budget_mib));
            let busy = (host.cpu_budget_millis > 0).then(|| {
                let cpu_seconds: f64 = job_rows.iter().map(|row| row.cpu_seconds_sum).sum();
                cpu_seconds / (f64::from(host.cpu_budget_millis) / 1000.0 * window_seconds)
            });
            let components = vec![
                host_availability(
                    (online_minutes > 0).then(|| f64::from(online_minutes) / window_minutes as f64),
                    targets,
                ),
                host_start_reliability(start_success(attempts, failures, targets), targets),
                host_relative_speed(index, targets),
                host_headroom(disk, memory, targets),
                host_busy_share(busy, fleet_queue_score, targets),
            ];
            let combined = combine(&components);
            ScoredHost {
                host_id: host.id.clone(),
                name: host.name.clone(),
                score: combined.score,
                components: combined.components,
                busy_share: busy,
            }
        })
        .collect();
    let combined = combine(&fleet_components);
    ScoredFleet {
        score: combined.score,
        components: combined.components,
        hosts: scored_hosts,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::fleet_health::{
        FleetHealthInputs, FleetLiveStats, HealthWindow, Histogram, HostInfo, HourlyHostRow,
        HourlyJobRow, JobCategory, RollupHost, SpeedResult,
    };

    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn sentences_format_durations_shares_and_gibibytes() {
        assert_eq!(duration(45_000.0), "45 s");
        assert_eq!(duration(60_000.0), "1 min");
        assert_eq!(duration(432_000.0), "7 min 12 s");
        assert_eq!(duration(2_400_000.0), "40 min");
        assert_eq!(duration(3_900_000.0), "1 h 5 min");
        assert_eq!(percent(0.974), "97.4%");
        assert_eq!(whole_percent(0.45), "45%");
        assert_eq!(whole_percent(0.299), "29.9%");
        assert_eq!(gib(15 * GIB), "15 GiB");
        assert_eq!(gib(103 * GIB / 10), "10.3 GiB");
    }

    #[test]
    fn the_disk_floors_are_the_alarm_levels() {
        // A 100 GiB disk: the absolute floors bind.
        assert_eq!(disk_floors(100 * GIB), (10 * GIB, 5 * GIB));
        // A 500 GiB disk: the percentage floors bind.
        assert_eq!(disk_floors(500 * GIB), (50 * GIB, 25 * GIB));
        // A tiny disk: the floors sit past its own size.
        assert_eq!(disk_floors(4 * GIB), (10 * GIB, 5 * GIB));
    }

    #[test]
    fn percentile_stays_inside_the_histogram_it_crosses() {
        let mut split = [0; HISTOGRAM_BUCKETS];
        split[0] = 4;
        split[1] = 4;
        assert_eq!(percentile(&split, 0.0), Some(0.0));
        // Half of eight falls exactly on the 10 s bucket boundary.
        assert_eq!(percentile(&split, 0.5), Some(10_000.0));
        assert_eq!(percentile(&split, 1.0), Some(30_000.0));
        // A quantile past the end is the top, not past it.
        assert_eq!(percentile(&split, 2.0), Some(30_000.0));
    }

    fn job_row(
        hour: i64,
        host: &str,
        category: JobCategory,
        jobs: (u64, u64, u64),
        cpu_seconds: f64,
        waits: Histogram,
    ) -> HourlyJobRow {
        HourlyJobRow {
            hour,
            host: RollupHost::Managed(host.to_owned()),
            pool_id: "pool".to_owned(),
            category,
            job_count: jobs.0,
            completed_count: jobs.1,
            infra_failure_count: jobs.2,
            duration_ms_sum: 0,
            cpu_seconds_sum: cpu_seconds,
            wait_histogram: waits,
            duration_histogram: [0; HISTOGRAM_BUCKETS],
        }
    }

    fn host_row(
        hour: i64,
        host: &str,
        online: u32,
        p50: Option<u32>,
        memory: Option<u32>,
        disk: Option<u64>,
    ) -> HourlyHostRow {
        HourlyHostRow {
            hour,
            host_id: host.to_owned(),
            online_minutes: online,
            cpu_used_p50_millis: p50,
            cpu_used_p95_millis: p50,
            memory_used_p95_mib: memory,
            min_disk_free_bytes: disk,
            start_attempts: 0,
            start_failures: BTreeMap::new(),
            uncertain_placements: 0,
        }
    }

    fn info(id: &str, cpu: u32, memory: u32, disk: Option<u64>) -> HostInfo {
        HostInfo {
            id: id.to_owned(),
            name: id.to_owned(),
            cpu_budget_millis: cpu,
            memory_budget_mib: memory,
            disk_total_bytes: disk,
        }
    }

    /// A window whose every measured value is hand-computed: `a` full and
    /// healthy, `b` half online with a tight disk and memory, `c` without a
    /// CPU budget, `d` silent, and rows outside the window that must not
    /// count. Hours are `index × HOUR_MILLIS`, the window `[0, 86_400_000)`.
    fn measured() -> FleetHealthInputs {
        let hour = |index: i64| index * HOUR_MILLIS;
        let mut a_failures = BTreeMap::new();
        a_failures.insert("agent_session_lost".to_owned(), 10_u32);
        let host_rows = (0..24)
            .flat_map(|index| {
                let a = {
                    let mut row =
                        host_row(hour(index), "a", 60, Some(4_000), Some(950), Some(60 * GIB));
                    if index == 0 {
                        row.start_attempts = 100;
                        row.start_failures = a_failures.clone();
                        row.uncertain_placements = 7;
                    }
                    row
                };
                let b_online = u32::from(index < 12);
                let mut b = host_row(
                    hour(index),
                    "b",
                    b_online * 60,
                    (b_online == 1).then_some(2_500),
                    (b_online == 1).then_some(if index == 5 { 450 } else { 400 }),
                    (b_online == 1).then_some(8 * GIB),
                );
                if index == 3 {
                    b.start_attempts = 100;
                }
                [a, b]
            })
            .collect();
        let mut b4 = [0; HISTOGRAM_BUCKETS];
        b4[4] = 150;
        let mut b0 = [0; HISTOGRAM_BUCKETS];
        b0[0] = 850;
        let mut b8 = [0; HISTOGRAM_BUCKETS];
        b8[8] = 9_999;
        FleetHealthInputs {
            window: HealthWindow::Day,
            now: HealthWindow::Day.millis(),
            hosts: vec![
                info("a", 10_000, 1_000, Some(100 * GIB)),
                info("b", 5_000, 500, Some(100 * GIB)),
                info("c", 0, 500, None),
                info("d", 5_000, 500, Some(100 * GIB)),
            ],
            hourly: vec![
                job_row(hour(0), "a", JobCategory::Test, (150, 80, 2), 43_200.0, b4),
                job_row(
                    hour(23),
                    "b",
                    JobCategory::Build,
                    (850, 800, 8),
                    108_000.0,
                    b0,
                ),
                // Outside the window: before it and at its end.
                job_row(
                    hour(-1),
                    "a",
                    JobCategory::Other,
                    (9_999, 9_999, 9_999),
                    1.0,
                    b8,
                ),
                job_row(
                    HealthWindow::Day.millis(),
                    "b",
                    JobCategory::Other,
                    (9_999, 9_999, 9_999),
                    1.0,
                    b8,
                ),
            ],
            host_hourly: host_rows,
            speed: vec![
                SpeedResult {
                    host_id: "a".to_owned(),
                    cpu_millis: 1_500,
                    index: Some(1.2),
                    top_keys: Vec::new(),
                },
                SpeedResult {
                    host_id: "a".to_owned(),
                    cpu_millis: 2_000,
                    index: Some(1.5),
                    top_keys: Vec::new(),
                },
                SpeedResult {
                    host_id: "b".to_owned(),
                    cpu_millis: 2_000,
                    index: None,
                    top_keys: Vec::new(),
                },
            ],
            live: FleetLiveStats::default(),
            host_live: Vec::new(),
            targets: HealthTargets::default(),
        }
    }

    #[test]
    fn score_fleet_measures_every_fleet_component_from_the_window() {
        let scored = score_fleet(&measured());
        let summary: Vec<(&str, Option<u8>)> = scored
            .components
            .iter()
            .map(|component| (component.key.as_str(), component.score))
            .collect();
        // p90 of the merged waits (150 in 2–5 min over 1_000 jobs) is 3 min;
        // starts 190/200; infra 10 of the 880 successful; one misfit hour of
        // 24; online minutes weighted 18_000_000/28_800_000.
        assert_eq!(
            summary,
            [
                ("queue-wait", Some(78)),
                ("start-reliability", Some(79)),
                ("infra-failures", Some(77)),
                ("capacity-fit", Some(83)),
                ("availability", Some(0)),
            ]
        );
        assert_eq!(
            scored.components[0].deduction.as_deref(),
            Some("Queue p90 was 3 min against a 1 min target: \u{2212}9.")
        );
        assert_eq!(
            scored.components[1].deduction.as_deref(),
            Some("95.0% of runner starts succeeded against a 99% target: \u{2212}4.")
        );
        assert_eq!(
            scored.components[2].deduction.as_deref(),
            Some(
                "1.1% of completed jobs failed on the infrastructure against a 0% target: \u{2212}3."
            )
        );
        assert_eq!(
            scored.components[3].deduction.as_deref(),
            Some(
                "4.2% of the window's hours waited over a minute while the fleet sat under half its CPU budget, against a 0% target: \u{2212}2."
            )
        );
        assert_eq!(
            scored.components[4].deduction.as_deref(),
            Some(
                "The fleet's CPU budget was online 62.5% of the window against a 100% target: \u{2212}10."
            )
        );
        assert_eq!(scored.score, Some(71));
    }

    #[test]
    fn score_fleet_measures_every_machine_component_from_the_window() {
        let scored = score_fleet(&measured());
        let hosts: BTreeMap<&str, &ScoredHost> = scored
            .hosts
            .iter()
            .map(|host| (host.host_id.as_str(), host))
            .collect();
        let parts = |id: &str| {
            hosts[id]
                .components
                .iter()
                .map(|component| (component.key.as_str(), component.score))
                .collect::<Vec<_>>()
        };

        // `a`: full availability, 90% starts, the worse of its two speed
        // indexes (1.5), memory-bound headroom at 95%, busy 5%.
        assert_eq!(
            parts("a"),
            [
                ("availability", Some(100)),
                ("start-reliability", Some(53)),
                ("relative-speed", Some(50)),
                ("headroom", Some(25)),
                ("busy-share", Some(10)),
            ]
        );
        assert_eq!(hosts["a"].score, Some(56));
        assert_eq!(hosts["a"].busy_share, Some(0.05));
        assert_eq!(
            hosts["a"].components[1].deduction.as_deref(),
            Some("90.0% of runner starts succeeded against a 99% target: \u{2212}12.")
        );
        assert_eq!(
            hosts["a"].components[2].deduction.as_deref(),
            Some("Speed index 1.50 against a 1.00 target: \u{2212}13.")
        );
        assert_eq!(
            hosts["a"].components[3].deduction.as_deref(),
            Some("Memory p95 was 95% of budget against a 80% target: \u{2212}11.")
        );
        assert_eq!(
            hosts["a"].components[4].deduction.as_deref(),
            Some("Busy 5% of its CPU budget against a 50% target while jobs waited: \u{2212}9.")
        );

        // `b`: half online, all starts succeeding, no speed index, the lower
        // of disk (8 GiB of the 10/5 floors) and memory (450 of 500 MiB),
        // busy 25%.
        assert_eq!(
            parts("b"),
            [
                ("availability", Some(0)),
                ("start-reliability", Some(100)),
                ("relative-speed", None),
                ("headroom", Some(50)),
                ("busy-share", Some(50)),
            ]
        );
        assert_eq!(hosts["b"].score, Some(50));
        assert_eq!(hosts["b"].busy_share, Some(0.25));
        assert_eq!(
            hosts["b"].components[0].deduction.as_deref(),
            Some("Online 50.0% of the window against a 100% target: \u{2212}25.")
        );
        assert_eq!(
            hosts["b"].components[3].deduction.as_deref(),
            Some("Memory p95 was 90% of budget against a 80% target: \u{2212}8.")
        );
        // The missing speed index takes no weight; the others still sum to 100.
        assert!(hosts["b"].components[2].weight < 1e-9);
        let weight: f64 = hosts["b"].components.iter().map(|c| c.weight).sum();
        assert!((weight - 100.0).abs() < 1e-6);

        // `c` has no CPU budget; `d` sent nothing: both without a score.
        assert_eq!(hosts["c"].score, None);
        assert_eq!(hosts["c"].busy_share, None);
        assert!(
            hosts["c"]
                .components
                .iter()
                .all(|c| c.score.is_none() && c.deduction.is_some()),
            "a machine without a budget has no component score"
        );
        assert_eq!(hosts["d"].score, Some(0));
        assert_eq!(hosts["d"].busy_share, Some(0.0));
        assert!(
            hosts["d"].components[0]
                .deduction
                .as_deref()
                .is_some_and(|reason| reason.to_lowercase().contains("not enough data"))
        );
    }

    #[test]
    fn a_window_under_min_jobs_scores_only_what_it_can_measure() {
        let mut inputs = measured();
        inputs.hosts.truncate(1); // `a` only
        inputs.host_hourly.retain(|row| row.host_id == "a");
        inputs.hourly.truncate(1); // 150 jobs → 19
        inputs.hourly[0].job_count = 19;
        inputs.hourly[0].completed_count = 19;
        inputs.hourly[0].infra_failure_count = 0;
        inputs.speed.clear();
        for (index, row) in inputs.host_hourly.iter_mut().enumerate() {
            // Five starts across the window: under min_jobs, fleet and host.
            row.start_attempts = u32::from(index == 0) * 5;
            row.start_failures.clear();
        }
        let scored = score_fleet(&inputs);
        let parts = scored
            .components
            .iter()
            .map(|component| (component.key.as_str(), component.score, component.weight))
            .collect::<Vec<_>>();
        assert_eq!(
            parts,
            [
                ("queue-wait", None, 0.0),
                ("start-reliability", None, 0.0),
                ("infra-failures", None, 0.0),
                ("capacity-fit", None, 0.0),
                ("availability", Some(100), 100.0),
            ]
        );
        assert_eq!(scored.score, Some(100));
        let host = &scored.hosts[0];
        assert_eq!(host.components[1].score, None);
        assert!(host.components[1].weight < 1e-9);
        // What the machine can still measure scores: availability 100,
        // headroom 25 and busy 100 over the 50 weight left, so 77.5 → 78.
        assert_eq!(host.score, Some(78));
    }
}
