//! The scoring engine (#192): pure functions from the window's rollups to a
//! 0–100 score per component, per machine and for the fleet. No I/O here, ever
//! (`the_scoring_engine_does_no_io`).
//!
//! #197 fixes the components' keys, labels and weights and the signatures;
//! every component scores `None` ("not enough data") until #192 implements
//! the two score tables of `docs/design/fleet-health.md`.

use super::{
    CombinedScore, DiskHeadroom, FleetHealthInputs, HISTOGRAM_BUCKETS, HealthTargets,
    ScoreComponent, ScoredFleet, ScoredHost,
};

const PENDING: &str = "Not enough data: scoring arrives with #192.";

fn pending(key: &str, label: &str, weight: f64) -> ScoreComponent {
    ScoreComponent {
        key: key.to_owned(),
        label: label.to_owned(),
        weight,
        score: None,
        deduction: Some(PENDING.to_owned()),
    }
}

/// Queue wait, from the window's p90 queue wait in milliseconds.
#[must_use]
pub fn fleet_queue_wait(p90_ms: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    let _ = (p90_ms, targets);
    pending("queue-wait", "Queue wait", 40.0)
}

/// Start reliability, from the share of runner starts that succeeded.
#[must_use]
pub fn fleet_start_reliability(success: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    let _ = (success, targets);
    pending("start-reliability", "Start reliability", 20.0)
}

/// Infrastructure failures, from their share of completed jobs.
#[must_use]
pub fn fleet_infra_failures(share: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    let _ = (share, targets);
    pending("infra-failures", "Infrastructure failures", 15.0)
}

/// Capacity fit, from the share of hours jobs waited while CPU sat idle.
#[must_use]
pub fn fleet_capacity_fit(misfit: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    let _ = (misfit, targets);
    pending("capacity-fit", "Capacity fit", 15.0)
}

/// Availability, from the budget-weighted share of machine-minutes online.
#[must_use]
pub fn fleet_availability(online: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    let _ = (online, targets);
    pending("availability", "Availability", 10.0)
}

/// A machine's availability, from its share of minutes online.
#[must_use]
pub fn host_availability(online: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    let _ = (online, targets);
    pending("availability", "Availability", 25.0)
}

/// A machine's start reliability.
#[must_use]
pub fn host_start_reliability(success: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    let _ = (success, targets);
    pending("start-reliability", "Start reliability", 25.0)
}

/// A machine's relative speed, from its speed index (#193).
#[must_use]
pub fn host_relative_speed(index: Option<f64>, targets: &HealthTargets) -> ScoreComponent {
    let _ = (index, targets);
    pending("relative-speed", "Relative speed", 25.0)
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
    let _ = (disk, memory_used, targets);
    pending("headroom", "Headroom", 15.0)
}

/// A machine's busy share (CPU-seconds reserved over its budget), deducted
/// only while the fleet's queue-wait component is under 100.
#[must_use]
pub fn host_busy_share(
    busy: Option<f64>,
    fleet_queue: Option<u8>,
    targets: &HealthTargets,
) -> ScoreComponent {
    let _ = (busy, fleet_queue, targets);
    pending("busy-share", "Busy share", 10.0)
}

/// The weighted score of `components`, a component without a score spreading
/// its weight over the others in proportion. `None` when none has a score.
#[must_use]
pub fn combine(components: &[ScoreComponent]) -> CombinedScore {
    CombinedScore {
        score: None,
        components: components.to_vec(),
    }
}

/// The `quantile` (0..=1) of a bucket histogram, interpolated inside the
/// bucket it falls in, in milliseconds. `None` for an empty histogram.
#[must_use]
pub fn percentile(histogram: &[u64; HISTOGRAM_BUCKETS], quantile: f64) -> Option<f64> {
    let _ = (histogram, quantile);
    None
}

/// Every score for the window.
#[must_use]
pub fn score_fleet(inputs: &FleetHealthInputs) -> ScoredFleet {
    let t = &inputs.targets;
    let fleet = vec![
        fleet_queue_wait(None, t),
        fleet_start_reliability(None, t),
        fleet_infra_failures(None, t),
        fleet_capacity_fit(None, t),
        fleet_availability(None, t),
    ];
    let hosts = inputs
        .hosts
        .iter()
        .map(|host| {
            let components = vec![
                host_availability(None, t),
                host_start_reliability(None, t),
                host_relative_speed(None, t),
                host_headroom(None, None, t),
                host_busy_share(None, None, t),
            ];
            let combined = combine(&components);
            ScoredHost {
                host_id: host.id.clone(),
                name: host.name.clone(),
                score: combined.score,
                components: combined.components,
                busy_share: None,
            }
        })
        .collect();
    let combined = combine(&fleet);
    ScoredFleet {
        score: combined.score,
        components: combined.components,
        hosts,
    }
}
