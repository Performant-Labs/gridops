//! The live half of the fleet health response (#201): what is true of each
//! machine and of the queue right now, and the live areas of concern
//! (offline, start gate, disk, unmatched jobs, oldest queued, uncertain
//! placements).
//!
//! #197 fixes the signature. #201 fills it in from the fleet-wide reads #197
//! added for it, with no principal: `gridops_core::fleet::registry::
//! host_live_reads` (D-3(f)) and `crate::resources::fleet_queue_breakdown`
//! (D-3(b)). Until then it reports an empty fleet with no concerns.

use gridops_core::fleet_health::{Concern, FleetLiveStats, HostLiveStats};
use sqlx::SqlitePool;

/// The fleet's and every machine's live stats at `now`, and the live
/// concerns.
///
/// # Errors
/// A database read failed.
#[allow(
    dead_code,
    clippy::unused_async,
    reason = "#201 reads the fleet here, and #204's database source calls it"
)]
pub(crate) async fn collect_live(
    db: &SqlitePool,
    now: i64,
) -> anyhow::Result<(FleetLiveStats, Vec<HostLiveStats>, Vec<Concern>)> {
    let _ = (db, now);
    Ok((FleetLiveStats::default(), Vec::new(), Vec::new()))
}
