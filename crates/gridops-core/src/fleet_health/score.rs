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

use super::machines::Life;

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

/// What a machine with a CPU budget of 0 says for each component: it has no
/// score (#239 item 4).
const NO_BUDGET: &str = "the machine has no CPU budget";

/// A component and its unrounded score, which [`combine_measured`] weighs so
/// that only the final score is rounded (#239 item 5). `raw` is `None`
/// exactly when `component.score` is. Private: the wire, [`combine`] and the
/// public component functions keep the rounded `ScoreComponent`.
struct Measured {
    component: ScoreComponent,
    raw: Option<f64>,
    /// The deduction sentence before its points (`None` at 100 or without a
    /// score): [`combine_measured`] adds the points lost from the weight
    /// after reweighting (#257).
    body: Option<String>,
}

/// A deduction sentence: `body` and the points lost, `round(weight × (100 −
/// raw) / 100)` from the unrounded score.
fn deduction(body: &str, weight: f64, raw: f64) -> String {
    let lost = (weight * (100.0 - raw) / 100.0).round() as i64;
    format!("{body}: \u{2212}{lost}.")
}

/// One component: `measured` scored through `scored` (linear, clamped),
/// rounded to `u8`, with the deduction sentence `"{body}: −N."` unless it
/// scored 100. `None` is "not enough data". The points lost are
/// `round(weight × (100 − raw) / 100)`, from the unrounded score, so a
/// component just under 100 can lose a rounded `0` points and still say so
/// (`−0.`, as the fixture's own "Speed index 1.01 …: −0." does).
fn measure<T: Copy>(
    key: &str,
    label: &str,
    weight: f64,
    measured: Option<T>,
    scored: impl FnOnce(T) -> f64,
    body: impl FnOnce(T) -> String,
    no_data: &str,
) -> Measured {
    let Some(value) = measured else {
        return Measured {
            component: ScoreComponent {
                key: key.to_owned(),
                label: label.to_owned(),
                weight,
                score: None,
                deduction: Some(format!("Not enough data: {no_data}.")),
            },
            raw: None,
            body: None,
        };
    };
    let raw = scored(value);
    let score = raw.round() as u8;
    let body = (score < 100).then(|| body(value));
    Measured {
        component: ScoreComponent {
            key: key.to_owned(),
            label: label.to_owned(),
            weight,
            score: Some(score),
            deduction: body.as_deref().map(|body| deduction(body, weight, raw)),
        },
        raw: Some(raw),
        body,
    }
}

/// `part` for a machine with a CPU budget of 0, which has no score: "not
/// enough data", saying it has no budget where the component had data to
/// score.
fn unbudgeted(part: Measured) -> Measured {
    let deduction = if part.raw.is_some() {
        Some(format!("Not enough data: {NO_BUDGET}."))
    } else {
        part.component.deduction
    };
    Measured {
        component: ScoreComponent {
            score: None,
            deduction,
            ..part.component
        },
        raw: None,
        body: None,
    }
}

/// Queue wait, from the window's p90 queue wait in milliseconds.
#[must_use]
pub fn fleet_queue_wait(p90_ms: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    measure_fleet_queue_wait(p90_ms, targets).component
}

fn measure_fleet_queue_wait(p90_ms: Option<f64>, targets: &HealthTargets) -> Measured {
    measure(
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
fn measure_starts(success: Option<f64>, targets: &HealthTargets, weight: f64) -> Measured {
    measure(
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
    measure_starts(success, targets, 20.0).component
}

/// Infrastructure failures, from their share of completed jobs.
#[must_use]
pub fn fleet_infra_failures(share: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    measure_fleet_infra_failures(share, targets).component
}

fn measure_fleet_infra_failures(share: Option<f64>, targets: &HealthTargets) -> Measured {
    measure(
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
    measure_fleet_capacity_fit(misfit, targets).component
}

fn measure_fleet_capacity_fit(misfit: Option<f64>, targets: &HealthTargets) -> Measured {
    measure(
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
    measure_fleet_availability(online, targets).component
}

fn measure_fleet_availability(online: Option<f64>, targets: &HealthTargets) -> Measured {
    measure(
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
    measure_host_availability(online, targets).component
}

fn measure_host_availability(online: Option<f64>, targets: &HealthTargets) -> Measured {
    measure(
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
    measure_starts(success, targets, 25.0).component
}

/// A machine's relative speed, from its speed index (#193).
#[must_use]
pub fn host_relative_speed(index: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    measure_host_relative_speed(index, targets).component
}

fn measure_host_relative_speed(index: Option<f64>, targets: &HealthTargets) -> Measured {
    measure(
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
    measure_host_headroom(disk, memory_used, targets).component
}

fn measure_host_headroom(
    disk: Option<DiskHeadroom>,
    memory_used: Option<f64>,
    targets: &HealthTargets,
) -> Measured {
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
    measure(
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
    measure_host_busy_share(busy, fleet_queue, targets).component
}

fn measure_host_busy_share(
    busy: Option<f64>,
    fleet_queue: Option<u8>,
    targets: &HealthTargets,
) -> Measured {
    let deducts = fleet_queue.is_some_and(|queue| queue < 100);
    measure(
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
        NO_BUDGET,
    )
}

/// The weighted score of `components`, a component without a score spreading
/// its weight over the others in proportion. `None` when none has a score.
#[must_use]
pub fn combine(components: &[ScoreComponent]) -> CombinedScore {
    let parts: Vec<Measured> = components
        .iter()
        .map(|component| Measured {
            component: component.clone(),
            raw: component.score.map(f64::from),
            body: None,
        })
        .collect();
    combine_measured(&parts)
}

/// [`combine`] over the unrounded scores: the weighted sum is rounded once,
/// at the end (#239 item 5). The components keep their rounded scores, with
/// the weights rescaled, and each deduction counts the points lost from the
/// rescaled weight (#257).
fn combine_measured(parts: &[Measured]) -> CombinedScore {
    let scored_weight: f64 = parts
        .iter()
        .filter_map(|part| part.raw.map(|_| part.component.weight))
        .sum();
    let scaled = |part: &Measured| match part.raw {
        Some(_) if scored_weight > 0.0 => part.component.weight * 100.0 / scored_weight,
        None if scored_weight == 0.0 => part.component.weight,
        _ => 0.0,
    };
    let score = (scored_weight > 0.0).then(|| {
        let weighted: f64 = parts
            .iter()
            .filter_map(|part| part.raw.map(|raw| raw * scaled(part)))
            .sum();
        (weighted / 100.0).round() as u8
    });
    CombinedScore {
        score,
        components: parts
            .iter()
            .map(|part| {
                let weight = scaled(part);
                // The points lost from the weight after reweighting, so the
                // deductions add up to what the score lost (#257).
                let deduction = match (&part.body, part.raw) {
                    (Some(body), Some(raw)) => Some(deduction(body, weight, raw)),
                    _ => part.component.deduction.clone(),
                };
                ScoreComponent {
                    weight,
                    deduction,
                    ..part.component.clone()
                }
            })
            .collect(),
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

/// The first hour the window's rows cover: `ceil_hour(now − W)`. Rows are
/// whole hours, so the window's rows run from here to `now`, the hour in
/// progress included.
fn first_hour(inputs: &FleetHealthInputs) -> i64 {
    ceil_div(inputs.now - inputs.window.millis(), HOUR_MILLIS) * HOUR_MILLIS
}

/// The covered time of a machine that lived `life`, the denominator of its time shares (#239
/// item 1): the time the window's rows cover, `[ceil_hour(now − W), now)`
/// (the full window only on the hour, and always over `W − 1 h`), cut to the
/// part the machine existed in (#262, [`Life::span`]). As
/// `(whole minutes, milliseconds)`: the minutes are the unit `online_minutes`
/// counts in, from the one the span starts in to the one it ends in, that
/// one excluded, so a machine online every whole minute is fully available
/// while the minute in progress is still unsampled. `(0, 0)` when it did not
/// exist in it.
///
/// Under the grace-hour contract with #204, the source supplies every hour of
/// it: the hours past their rollup grace from the hourly tables, and the rest
/// (the hour in progress, and the hour that just ended until its grace
/// passes) computed live.
fn host_covered(inputs: &FleetHealthInputs, life: Life) -> (i64, i64) {
    let (start, end) = life.span(first_hour(inputs), inputs.now);
    if start >= end {
        return (0, 0);
    }
    (
        end.div_euclid(60_000) - start.div_euclid(60_000),
        end - start,
    )
}

/// `rows` of machine `id`, which lived `life`, in the hours its life reaches
/// (#262).
fn host_rows<'a>(id: &str, life: Life, rows: &[&'a HourlyHostRow]) -> Vec<&'a HourlyHostRow> {
    rows.iter()
        .copied()
        .filter(|row| row.host_id == id && life.counts_hour(row.hour))
        .collect()
}

/// Every machine the fleet's availability and capacity fit count, as (id,
/// CPU budget, life): the page's, and those retired during the window, which
/// are off the page but count for their time before `retired_at` (#262).
fn fleet_machines(inputs: &FleetHealthInputs) -> Vec<(&str, u32, Life)> {
    inputs
        .hosts
        .iter()
        .map(|host| (host.id.as_str(), host.cpu_budget_millis, host.life()))
        .chain(
            inputs
                .retired_hosts
                .iter()
                .map(|host| (host.id.as_str(), host.cpu_budget_millis, host.life())),
        )
        .collect()
}

/// The fleet's budget-weighted share of machine-minutes online: each
/// machine's online minutes over its own covered minutes (#262), weighted by
/// its CPU budget, the machines retired during the window included. A
/// machine with no rows in the window counts as offline, at its full budget.
/// `None` with no budget or no whole minute covered.
fn fleet_online_share(inputs: &FleetHealthInputs, rows: &[&HourlyHostRow]) -> Option<f64> {
    let mut online = 0.0_f64;
    let mut budget = 0.0_f64;
    for (id, cpu_budget_millis, life) in fleet_machines(inputs) {
        let (covered, _) = host_covered(inputs, life);
        let minutes: u32 = host_rows(id, life, rows)
            .iter()
            .map(|row| row.online_minutes)
            .sum();
        // The sampled minute in progress can put a machine one past the
        // covered minutes; capped, it never offsets another's downtime.
        let minutes = i64::from(minutes).min(covered);
        online += minutes as f64 * f64::from(cpu_budget_millis);
        budget += covered as f64 * f64::from(cpu_budget_millis);
    }
    (budget > 0.0).then(|| online / budget)
}

/// `value` divided by `by`, rounded up (both positive or negative).
fn ceil_div(value: i64, by: i64) -> i64 {
    value / by + i64::from(value % by != 0 && (value >= 0) == (by >= 0))
}

/// The share of the window's hours in which jobs waited over a minute while
/// the fleet's sampled machines sat under half their sampled CPU budget: the
/// page's machines and those retired during the window, each only in the
/// hours its life reaches (#262).
fn capacity_misfit(
    inputs: &FleetHealthInputs,
    jobs: &[&HourlyJobRow],
    rows: &[&HourlyHostRow],
) -> Option<f64> {
    let first = first_hour(inputs);
    let hours = (inputs.now - first) / HOUR_MILLIS;
    let machines = fleet_machines(inputs);
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
            let Some(&(_, cpu_budget_millis, _)) = machines
                .iter()
                .find(|(id, _, life)| *id == row.host_id && life.counts_hour(hour))
            else {
                continue;
            };
            used += f64::from(p50);
            budget += f64::from(cpu_budget_millis);
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
    let queue = measure_fleet_queue_wait(
        enough_jobs.then(|| percentile(&wait, 0.9)).flatten(),
        targets,
    );
    // The busy-share gate reads the rounded queue-wait score, as the concern
    // bands read rounded components (design rule (h)).
    let fleet_queue_score = queue.component.score;
    let fleet_components = vec![
        queue,
        measure_starts(start_success(attempts, failures, targets), targets, 20.0),
        // Of every completed final-attempt job, not only the successes, so an
        // outage where nothing succeeds still scores (#239 item 2).
        measure_fleet_infra_failures(
            (enough_jobs && total_jobs > 0).then(|| infra_failures as f64 / total_jobs as f64),
            targets,
        ),
        measure_fleet_capacity_fit(
            enough_jobs
                .then(|| capacity_misfit(inputs, &jobs, &hosts))
                .flatten(),
            targets,
        ),
        measure_fleet_availability(fleet_online_share(inputs, &hosts), targets),
    ];

    let scored_hosts = inputs
        .hosts
        .iter()
        .map(|host| {
            // Its own covered time and rows: the part of the window it
            // existed in (#262).
            let (covered_minutes, covered_millis) = host_covered(inputs, host.life());
            let covered_seconds = covered_millis as f64 / 1000.0;
            let rows = host_rows(&host.id, host.life(), &hosts);
            let job_rows: Vec<&HourlyJobRow> = jobs
                .iter()
                .copied()
                .filter(|row| row.host.as_str() == host.id && host.life().counts_hour(row.hour))
                .collect();
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
            let budgeted = host.cpu_budget_millis > 0;
            let busy = (budgeted && covered_millis > 0).then(|| {
                let cpu_seconds: f64 = job_rows.iter().map(|row| row.cpu_seconds_sum).sum();
                cpu_seconds / (f64::from(host.cpu_budget_millis) / 1000.0 * covered_seconds)
            });
            // Only no rows at all is missing data: rows with 0 online minutes
            // are a machine that was offline, and score 0 (#239 item 3).
            let online = (!rows.is_empty() && covered_minutes > 0)
                .then(|| f64::from(online_minutes) / covered_minutes as f64);
            let mut parts = vec![
                measure_host_availability(online, targets),
                measure_starts(start_success(attempts, failures, targets), targets, 25.0),
                measure_host_relative_speed(index, targets),
                measure_host_headroom(disk, memory, targets),
                measure_host_busy_share(busy, fleet_queue_score, targets),
            ];
            // A machine with a CPU budget of 0 has no score (#239 item 4).
            if !budgeted {
                parts = parts.into_iter().map(unbudgeted).collect();
            }
            let combined = combine_measured(&parts);
            ScoredHost {
                host_id: host.id.clone(),
                name: host.name.clone(),
                score: combined.score,
                components: combined.components,
                busy_share: busy,
            }
        })
        .collect();
    let combined = combine_measured(&fleet_components);
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
        HourlyJobRow, JobCategory, RetiredHost, RollupHost, SpeedResult,
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
            created_at: None,
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
            retired_hosts: Vec::new(),
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
        // starts 190/200; infra 10 of the 1_000 completed (every final-attempt
        // job, not only the 880 that succeeded: #239 item 2); one misfit hour
        // of 24; online minutes weighted 18_000_000/28_800_000.
        assert_eq!(
            summary,
            [
                ("queue-wait", Some(78)),
                ("start-reliability", Some(79)),
                ("infra-failures", Some(80)),
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
                "1.0% of completed jobs failed on the infrastructure against a 0% target: \u{2212}3."
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
        // Combined unrounded (#239 item 5): 100, 52.63, 50, 25 and 10 at
        // 25/25/25/15/10 is 55.41, so 55; the rounded parts would give 55.5,
        // so 56.
        assert_eq!(hosts["a"].score, Some(55));
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
        // Relative speed has no data, so availability's 25 is 33.3 of the
        // other 75: it loses 33 points, not 25 (#257).
        assert_eq!(
            hosts["b"].components[0].deduction.as_deref(),
            Some("Online 50.0% of the window against a 100% target: \u{2212}33.")
        );
        assert_eq!(
            hosts["b"].components[3].deduction.as_deref(),
            // Its 15 is 20 of the reweighted 100 (#257).
            Some("Memory p95 was 90% of budget against a 80% target: \u{2212}10.")
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
    // #239's scoring fixes. Every test below reads as of a `now` that is
    // not necessarily on the hour: the window's rows cover the whole hours
    // from `ceil_hour(now − W)` to `now`, the hour in progress included, and
    // every time share divides by that covered span.

    /// A whole hour: the base every #239 test offsets `now` from.
    const T: i64 = 1_800_000_000_000;
    const MINUTE: i64 = 60_000;

    /// Inputs for `window` as of `now`: the default targets, no speed and no
    /// live stats.
    fn window_inputs(
        window: HealthWindow,
        now: i64,
        hosts: Vec<HostInfo>,
        hourly: Vec<HourlyJobRow>,
        host_hourly: Vec<HourlyHostRow>,
    ) -> FleetHealthInputs {
        FleetHealthInputs {
            window,
            now,
            hosts,
            retired_hosts: Vec::new(),
            hourly,
            host_hourly,
            speed: Vec::new(),
            live: FleetLiveStats::default(),
            host_live: Vec::new(),
            targets: HealthTargets::default(),
        }
    }

    /// The score of the component `key` in `components` (`Ok(None)` is "not
    /// enough data"), or an error without that component.
    fn part(components: &[ScoreComponent], key: &str) -> Result<Option<u8>, String> {
        components
            .iter()
            .find(|component| component.key == key)
            .map(|component| component.score)
            .ok_or_else(|| format!("no {key} component"))
    }

    /// Item 1: at 10:30 on 1h the window holds only the 10:00 hour, 30
    /// minutes of it. A machine online all of them is fully available, and
    /// no `fleet-component:availability` concern fires on a load that is not
    /// exactly on the hour.
    #[test]
    fn availability_on_1h_divides_by_the_covered_half_hour_not_the_full_window() {
        let inputs = window_inputs(
            HealthWindow::Hour,
            T + 30 * MINUTE,
            vec![info("a", 4_000, 1_000, None)],
            Vec::new(),
            vec![host_row(T, "a", 30, Some(1_000), None, None)],
        );
        let scored = score_fleet(&inputs);
        assert_eq!(
            part(&scored.components, "availability"),
            Ok(Some(100)),
            "fleet"
        );
        assert_eq!(
            part(&scored.hosts[0].components, "availability"),
            Ok(Some(100)),
            "machine"
        );
        let concerns = crate::fleet_health::concerns::windowed(&inputs, &scored);
        assert!(
            concerns
                .iter()
                .all(|concern| concern.key != "fleet-component:availability"),
            "a fully online fleet raised {concerns:?}"
        );
    }

    /// #257: a deduction's points are lost from the component's weight
    /// after reweighting, so a score's deductions add up to what it lost. On
    /// 1h with too few jobs only availability has data: it carries all 100,
    /// and online 27 of 30 minutes (90%, scoring 50) loses 50 points, not the
    /// 5 its own weight of 10 would say. A machine's components likewise.
    #[test]
    fn deductions_count_the_points_lost_after_reweighting() {
        let inputs = window_inputs(
            HealthWindow::Hour,
            T + 30 * MINUTE,
            vec![info("a", 4_000, 1_000, None)],
            Vec::new(),
            vec![host_row(T, "a", 27, Some(1_000), None, None)],
        );
        let scored = score_fleet(&inputs);
        assert_eq!(scored.score, Some(50));
        let availability = scored
            .components
            .iter()
            .find(|component| component.key == "availability");
        assert_eq!(
            availability.and_then(|component| component.deduction.as_deref()),
            Some(
                "The fleet's CPU budget was online 90.0% of the window against a 100% target: \u{2212}50."
            )
        );
        for host in &scored.hosts {
            let lost: i64 = host
                .components
                .iter()
                .filter_map(|component| component.deduction.as_deref())
                .filter_map(|sentence| sentence.rsplit_once('\u{2212}'))
                .filter_map(|(_, points)| points.trim_end_matches('.').parse::<i64>().ok())
                .sum();
            let score = host.score.map_or(100, i64::from);
            assert!(
                (lost - (100 - score)).abs() <= 1,
                "{}: deductions {lost} for a score of {score}",
                host.host_id
            );
        }
    }

    /// Item 1's grace-hour contract: at H + 10 min on 24h the source supplies
    /// the 23 ended hours (rolled, or the one just ended computed live) and
    /// the 10 minutes of the hour in progress. Online every one of those
    /// minutes is availability 100, against the 1390 of 1440 minutes (83)
    /// a full-window denominator reads.
    #[test]
    fn availability_on_24h_at_ten_past_counts_the_covered_23_hours_and_10_minutes() {
        let mut rows: Vec<HourlyHostRow> = (1..=23)
            .map(|back| host_row(T - back * HOUR_MILLIS, "a", 60, Some(1_000), None, None))
            .collect();
        // The hour in progress, its 10 minutes computed live: the same row
        // shape as a rolled one.
        rows.push(host_row(T, "a", 10, Some(1_000), None, None));
        let inputs = window_inputs(
            HealthWindow::Day,
            T + 10 * MINUTE,
            vec![info("a", 4_000, 1_000, None)],
            Vec::new(),
            rows,
        );
        let scored = score_fleet(&inputs);
        assert_eq!(
            part(&scored.components, "availability"),
            Ok(Some(100)),
            "fleet"
        );
        assert_eq!(
            part(&scored.hosts[0].components, "availability"),
            Ok(Some(100)),
            "machine"
        );
    }

    /// A machine's sampled minute in progress can put its `online_minutes`
    /// one past the whole covered minutes; in the fleet's budget-weighted
    /// share that extra minute must not make up for another machine's
    /// downtime. At one minute past the hour on 1h, a machine online for 2
    /// sampled minutes and an equal one down score as one online for 1.
    #[test]
    fn a_machine_past_the_covered_minutes_does_not_offset_another_s_downtime() {
        let fleet_with = |minutes_a: u32| {
            let inputs = window_inputs(
                HealthWindow::Hour,
                T + MINUTE + 30_000,
                vec![info("a", 4_000, 1_000, None), info("b", 4_000, 1_000, None)],
                Vec::new(),
                vec![
                    host_row(T, "a", minutes_a, Some(1_000), None, None),
                    host_row(T, "b", 0, Some(1_000), None, None),
                ],
            );
            part(&score_fleet(&inputs).components, "availability")
        };
        let exact = fleet_with(1);
        assert_ne!(exact, Ok(Some(100)), "half the fleet is down");
        assert_eq!(
            fleet_with(2),
            exact,
            "the extra minute is capped at the covered one"
        );
    }

    /// Item 1 (the plan review's finding 2): `online_minutes` counts whole
    /// minutes with a sample, so availability divides by whole covered
    /// minutes. At H + 5 min 29 s, with the minute in progress not yet
    /// sampled, 5 online minutes of 5 covered is 100; at H + 29 s no whole
    /// minute is covered, which is not enough data, not 0.
    #[test]
    fn availability_divides_by_whole_covered_minutes_and_none_covered_is_no_data() {
        let host = || vec![info("a", 4_000, 1_000, None)];
        let five = score_fleet(&window_inputs(
            HealthWindow::Hour,
            T + 5 * MINUTE + 29_000,
            host(),
            Vec::new(),
            vec![host_row(T, "a", 5, Some(1_000), None, None)],
        ));
        assert_eq!(
            part(&five.components, "availability"),
            Ok(Some(100)),
            "fleet"
        );
        assert_eq!(
            part(&five.hosts[0].components, "availability"),
            Ok(Some(100)),
            "machine"
        );

        let none = score_fleet(&window_inputs(
            HealthWindow::Hour,
            T + 29_000,
            host(),
            Vec::new(),
            vec![host_row(T, "a", 0, None, None, None)],
        ));
        assert_eq!(
            part(&none.components, "availability"),
            Ok(None),
            "fleet, 0 covered minutes"
        );
        assert_eq!(
            part(&none.hosts[0].components, "availability"),
            Ok(None),
            "machine, 0 covered minutes"
        );
    }

    /// Item 1: busy share divides the CPU-seconds reserved by the budget over
    /// the covered seconds. 1800 CPU-seconds on a 2-CPU machine over the
    /// 1800 s covered at 10:30 on 1h is busy half the time, not a quarter.
    #[test]
    fn busy_share_on_1h_divides_by_the_covered_seconds() {
        let scored = score_fleet(&window_inputs(
            HealthWindow::Hour,
            T + 30 * MINUTE,
            vec![info("a", 2_000, 1_000, None)],
            vec![job_row(
                T,
                "a",
                JobCategory::Test,
                (5, 5, 0),
                1_800.0,
                [0; HISTOGRAM_BUCKETS],
            )],
            vec![host_row(T, "a", 30, Some(1_000), None, None)],
        ));
        let busy = scored.hosts[0].busy_share;
        assert!(
            busy.is_some_and(|busy| (busy - 0.5).abs() < 1e-9),
            "busy share {busy:?}"
        );
    }

    /// Item 2: the infrastructure-failure share is of every completed
    /// final-attempt job (`job_count`), not of the successes. In an outage
    /// where nothing succeeds the component scores 0 and its critical concern
    /// fires, instead of going to "not enough data".
    #[test]
    fn infra_failures_divide_by_every_completed_job_so_an_outage_scores_0() {
        let inputs = window_inputs(
            HealthWindow::Day,
            T,
            vec![info("a", 4_000, 1_000, None)],
            vec![job_row(
                T - HOUR_MILLIS,
                "a",
                JobCategory::Test,
                (40, 0, 40),
                0.0,
                {
                    let mut waits = [0; HISTOGRAM_BUCKETS];
                    waits[0] = 40;
                    waits
                },
            )],
            (1..=24)
                .map(|back| host_row(T - back * HOUR_MILLIS, "a", 60, Some(1_000), None, None))
                .collect(),
        );
        let scored = score_fleet(&inputs);
        assert_eq!(part(&scored.components, "infra-failures"), Ok(Some(0)));
        let concerns = crate::fleet_health::concerns::windowed(&inputs, &scored);
        assert!(
            concerns.iter().any(|concern| {
                concern.key == "fleet-component:infra-failures"
                    && concern.severity == crate::fleet_health::Severity::Critical
            }),
            "no critical infra-failures concern in {concerns:?}"
        );
    }

    /// Item 3: only a machine with no rows has no availability data. One whose
    /// rows say 0 online minutes all window (#200 writes those on purpose)
    /// was offline, and scores 0.
    #[test]
    fn a_machine_offline_all_window_scores_availability_0_not_no_data() {
        let scored = score_fleet(&window_inputs(
            HealthWindow::Day,
            T,
            vec![
                info("a", 4_000, 1_000, None),
                info("silent", 4_000, 1_000, None),
            ],
            Vec::new(),
            (1..=24)
                .map(|back| host_row(T - back * HOUR_MILLIS, "a", 0, None, None, None))
                .collect(),
        ));
        let offline = &scored.hosts[0];
        assert_eq!(offline.host_id, "a");
        assert_eq!(part(&offline.components, "availability"), Ok(Some(0)));
        assert_eq!(
            offline.components[0].deduction.as_deref(),
            // Only availability and busy share have data: 25 of 35 is 71.4
            // of the score, all of it lost (#257).
            Some("Online 0.0% of the window against a 100% target: \u{2212}71.")
        );
        // No rows at all is still not enough data.
        let silent = &scored.hosts[1];
        assert_eq!(part(&silent.components, "availability"), Ok(None));
    }

    /// Item 4: a machine with a CPU budget of 0 (`MacBookPro`) has no score:
    /// every component is "not enough data" whatever its rows say, and it
    /// raises no `host-score` concern.
    #[test]
    fn a_machine_with_no_cpu_budget_gets_no_score_and_no_host_score_concern() {
        let mut rows: Vec<HourlyHostRow> = Vec::new();
        for back in 1..=24 {
            rows.push(host_row(
                T - back * HOUR_MILLIS,
                "a",
                60,
                Some(1_000),
                None,
                None,
            ));
            // Half online, memory near its budget: scored, it would be 0.
            let mut laptop = host_row(T - back * HOUR_MILLIS, "laptop", 30, None, Some(990), None);
            laptop.start_attempts = 1;
            rows.push(laptop);
        }
        let inputs = window_inputs(
            HealthWindow::Day,
            T,
            vec![
                info("a", 4_000, 1_000, None),
                info("laptop", 0, 1_000, None),
            ],
            Vec::new(),
            rows,
        );
        let scored = score_fleet(&inputs);
        let laptop = &scored.hosts[1];
        assert_eq!(laptop.host_id, "laptop");
        assert_eq!(laptop.score, None);
        assert_eq!(laptop.busy_share, None);
        for component in &laptop.components {
            assert_eq!(component.score, None, "{} has a score", component.key);
            assert!(
                component.deduction.is_some(),
                "{} says nothing",
                component.key
            );
        }
        let deduction = |host: &crate::fleet_health::ScoredHost, key: &str| {
            host.components
                .iter()
                .find(|component| component.key == key)
                .and_then(|component| component.deduction.clone())
        };
        assert_eq!(
            deduction(laptop, "availability").as_deref(),
            Some("Not enough data: the machine has no CPU budget."),
            "a component that had data says why it has no score"
        );
        let no_speed = deduction(&scored.hosts[0], "relative-speed");
        assert!(
            no_speed.is_some(),
            "a budgeted machine with no speed says so"
        );
        assert_eq!(
            deduction(laptop, "relative-speed"),
            no_speed,
            "a component with no data keeps its own reason"
        );
        let concerns = crate::fleet_health::concerns::windowed(&inputs, &scored);
        assert!(
            concerns
                .iter()
                .all(|concern| concern.key != "host-score:laptop"),
            "a machine without a budget raised {concerns:?}"
        );
    }

    /// Item 5: components are combined unrounded and only the final score is
    /// rounded. Availability 49.65 (1295 of 1440 minutes), headroom 48.55
    /// (memory p95 9029 of 10000 MiB) and busy share 100 over weights
    /// 50/30/20 combine to 59.39, so 59; rounding the components first (50,
    /// 49, 100) gives 59.7, so 60.
    #[test]
    fn components_combine_unrounded_and_only_the_final_score_is_rounded() {
        let rows = (1..=24)
            .map(|back| {
                let minutes = if back == 1 { 53 } else { 54 };
                host_row(
                    T - back * HOUR_MILLIS,
                    "a",
                    minutes,
                    None,
                    Some(9_029),
                    None,
                )
            })
            .collect();
        let scored = score_fleet(&window_inputs(
            HealthWindow::Day,
            T,
            vec![info("a", 4_000, 10_000, None)],
            Vec::new(),
            rows,
        ));
        let host = &scored.hosts[0];
        // The wire still carries each component rounded.
        assert_eq!(part(&host.components, "availability"), Ok(Some(50)));
        assert_eq!(part(&host.components, "headroom"), Ok(Some(49)));
        assert_eq!(part(&host.components, "busy-share"), Ok(Some(100)));
        assert_eq!(host.score, Some(59));
    }

    /// Item 5's preservation rule (design rule (h)), a guard rather than a
    /// fix: the busy-share gate reads the fleet's rounded queue-wait score. A
    /// p90 of 62.16 s is a raw 99.6, rounded 100, so jobs did not wait and an
    /// idle machine loses nothing for it.
    #[test]
    fn the_busy_share_gate_reads_the_rounded_queue_wait_score() {
        // 8_640 jobs under 10 s and 1_000 in 1–2 min: p90 = 60 s + 0.036 min.
        let mut waits = [0; HISTOGRAM_BUCKETS];
        waits[0] = 8_640;
        waits[3] = 1_000;
        let scored = score_fleet(&window_inputs(
            HealthWindow::Day,
            T,
            vec![info("a", 4_000, 1_000, None)],
            vec![job_row(
                T - HOUR_MILLIS,
                "unmanaged-elsewhere",
                JobCategory::Test,
                (9_640, 9_640, 0),
                0.0,
                waits,
            )],
            (1..=24)
                .map(|back| host_row(T - back * HOUR_MILLIS, "a", 60, Some(1_000), None, None))
                .collect(),
        ));
        assert_eq!(part(&scored.components, "queue-wait"), Ok(Some(100)));
        assert_eq!(scored.hosts[0].busy_share, Some(0.0));
        assert_eq!(
            part(&scored.hosts[0].components, "busy-share"),
            Ok(Some(100))
        );
    }

    // #262: a page machine counts only from its `created_at`; a machine
    // retired during the window is off the page but counts for the fleet's
    // availability and capacity fit until its `retired_at`: every time
    // denominator is a machine's own span, and its rows outside it (the
    // 0-minute rows the machine rollup wrote for a retired machine before
    // #262) never count.

    fn retired(id: &str, cpu: u32, retired_at: i64) -> RetiredHost {
        RetiredHost {
            id: id.to_owned(),
            cpu_budget_millis: cpu,
            created_at: None,
            retired_at,
        }
    }

    /// `old` (2 CPUs) was retired at T − 5 h, when `new` (4 CPUs) replaced
    /// it. `old` was offline one hour before its retirement, and its rows
    /// after it are the 0-minute rows a rollup wrote before #262. Only `new`
    /// is scored, but the fleet's availability counts `old`'s 19 hours:
    /// (1,080 × 2,000 + 300 × 4,000) ÷ (1,140 × 2,000 + 300 × 4,000) =
    /// 96.6%, which scores 83 (100 without `old`, lower with its rows after
    /// its retirement).
    #[test]
    fn a_retired_machine_is_off_the_page_but_counts_for_the_fleet_until_it_retired() {
        let mut new = info("new", 4_000, 1_000, None);
        new.created_at = Some(T - 5 * HOUR_MILLIS);
        let mut rows: Vec<HourlyHostRow> = (6..=24)
            .map(|back| {
                let online = if back == 10 { 0 } else { 60 };
                host_row(
                    T - back * HOUR_MILLIS,
                    "old",
                    online,
                    Some(1_500),
                    None,
                    None,
                )
            })
            .collect();
        rows.extend(
            (1..=5).map(|back| host_row(T - back * HOUR_MILLIS, "old", 0, None, None, None)),
        );
        rows.extend(
            (1..=5)
                .map(|back| host_row(T - back * HOUR_MILLIS, "new", 60, Some(3_000), None, None)),
        );
        let mut inputs = window_inputs(HealthWindow::Day, T, vec![new], Vec::new(), rows);
        inputs.retired_hosts = vec![retired("old", 2_000, T - 5 * HOUR_MILLIS)];
        let scored = score_fleet(&inputs);
        assert_eq!(
            part(&scored.components, "availability"),
            Ok(Some(83)),
            "fleet"
        );
        let on_page: Vec<&str> = scored
            .hosts
            .iter()
            .map(|host| host.host_id.as_str())
            .collect();
        assert_eq!(on_page, ["new"]);
        assert_eq!(
            part(&scored.hosts[0].components, "availability"),
            Ok(Some(100))
        );
        let concerns = crate::fleet_health::concerns::windowed(&inputs, &scored);
        assert!(
            concerns.iter().all(|concern| !concern.key.contains("old")),
            "a concern on the retired machine: {concerns:?}"
        );
    }

    /// A machine created at T − 2 h 40 min and online every minute since is
    /// fully available on the day: the hours before it existed are not
    /// offline, even with a 0-minute row for one of them.
    #[test]
    fn a_machine_created_in_the_window_counts_only_from_its_creation() {
        let mut new = info("new", 4_000, 1_000, None);
        new.created_at = Some(T - 2 * HOUR_MILLIS - 40 * MINUTE);
        let scored = score_fleet(&window_inputs(
            HealthWindow::Day,
            T,
            vec![new],
            vec![job_row(
                T - HOUR_MILLIS,
                "new",
                JobCategory::Test,
                (5, 5, 0),
                // 160 min × 4 CPUs, a quarter of the time.
                160.0 * 60.0,
                [0; HISTOGRAM_BUCKETS],
            )],
            vec![
                host_row(T - 10 * HOUR_MILLIS, "new", 0, None, None, None),
                host_row(T - 3 * HOUR_MILLIS, "new", 40, Some(1_000), None, None),
                host_row(T - 2 * HOUR_MILLIS, "new", 60, Some(1_000), None, None),
                host_row(T - HOUR_MILLIS, "new", 60, Some(1_000), None, None),
            ],
        ));
        assert_eq!(
            part(&scored.hosts[0].components, "availability"),
            Ok(Some(100)),
            "machine"
        );
        assert_eq!(
            part(&scored.components, "availability"),
            Ok(Some(100)),
            "fleet"
        );
        let busy = scored.hosts[0].busy_share;
        assert!(
            busy.is_some_and(|busy| (busy - 0.25).abs() < 1e-9),
            "busy share {busy:?}"
        );
    }

    /// Capacity fit counts a retired machine's rows before its retirement
    /// and never after. At T − 8 h only `old` was sampled, 500 of its 2,000:
    /// a misfit. At T − 2 h `new` runs at 2,900 of 4,000, no misfit; `old`'s
    /// row after its retirement (p50 0) would make it one (2,900 under half
    /// of 6,000). So 1 of 24 hours: 4.2%, which scores 83 (100 without
    /// `old`, 67 with its row after its retirement).
    #[test]
    fn capacity_fit_counts_a_retired_machine_only_before_it_retired() {
        let mut new = info("new", 4_000, 1_000, None);
        new.created_at = Some(T - 5 * HOUR_MILLIS);
        let mut rows: Vec<HourlyHostRow> = (6..=24)
            .map(|back| host_row(T - back * HOUR_MILLIS, "old", 60, Some(500), None, None))
            .collect();
        rows.push(host_row(T - 2 * HOUR_MILLIS, "old", 0, Some(0), None, None));
        rows.extend((1..=5).map(|back| {
            let p50 = if back == 2 { 2_900 } else { 3_000 };
            host_row(T - back * HOUR_MILLIS, "new", 60, Some(p50), None, None)
        }));
        let mut waits = [0; HISTOGRAM_BUCKETS];
        waits[3] = 20;
        let jobs = [8, 2]
            .into_iter()
            .map(|back| {
                let host = if back == 8 { "old" } else { "new" };
                job_row(
                    T - back * HOUR_MILLIS,
                    host,
                    JobCategory::Test,
                    (20, 20, 0),
                    0.0,
                    waits,
                )
            })
            .collect();
        let mut inputs = window_inputs(HealthWindow::Day, T, vec![new], jobs, rows);
        inputs.retired_hosts = vec![retired("old", 2_000, T - 5 * HOUR_MILLIS)];
        let scored = score_fleet(&inputs);
        assert_eq!(part(&scored.components, "capacity-fit"), Ok(Some(83)));
        assert_eq!(part(&scored.components, "availability"), Ok(Some(100)));
    }
}
