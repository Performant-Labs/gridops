//! The windowed areas of concern (#192): pure rules over the scores, the
//! speed results and the window's job mix. The live concerns (offline,
//! start gate, disk, unmatched, oldest queued, uncertain) are #201's
//! `collect_live`. No I/O here, ever (`the_scoring_engine_does_no_io`).
//!
//! #197 fixes the rules' signatures; every rule returns `None` until #192
//! implements the concern table of `docs/design/fleet-health.md`.

use super::{
    Concern, FleetHealthInputs, HostInfo, ScoreComponent, ScoredFleet, ScoredHost, SpeedResult,
};

/// `fleet-component:<key>`: critical under 50, warning under 70.
#[must_use]
pub fn fleet_component(component: &ScoreComponent) -> Option<Concern> {
    let _ = component;
    None
}

/// `host-score:<host>`: the same bands, naming the worst component.
#[must_use]
pub fn host_score(host: &ScoredHost) -> Option<Concern> {
    let _ = host;
    None
}

/// `host-slow:<host>:<cpu millis>`: a speed index of 1.3 or more at one
/// runner size, naming the keys behind it.
#[must_use]
pub fn host_slow(host: &HostInfo, speed: &SpeedResult) -> Option<Concern> {
    let _ = (host, speed);
    None
}

/// `host-idle-while-queued:<host>`: busy under 30% while the fleet's
/// queue-wait component is under 70.
#[must_use]
pub fn host_idle_while_queued(host: &ScoredHost, fleet_queue: Option<u8>) -> Option<Concern> {
    let _ = (host, fleet_queue);
    None
}

/// `category-other`: more than 20% of the window's jobs are `other`.
#[must_use]
pub fn category_other(other_share: Option<f64>) -> Option<Concern> {
    let _ = other_share;
    None
}

/// Every windowed concern, in rule order.
#[must_use]
pub fn windowed(inputs: &FleetHealthInputs, scored: &ScoredFleet) -> Vec<Concern> {
    let _ = (inputs, scored);
    Vec::new()
}
