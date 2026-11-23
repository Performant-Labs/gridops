//! Pruning retained tables in bounded batches (#152, #222).
//!
//! The reconcile pass deletes rows older than each table's retention window.
//! The deletes run under `SQLite`'s single write lock, which the API's writers
//! (another process) wait on for at most the busy timeout. One unbounded
//! `DELETE` held it for up to 13 s on Mars while it read all 628k machine
//! samples, and the API answered with 503s (#222).
//!
//! So each batch is one autocommit statement of at most `batch_size` rows,
//! found through the table's cutoff-column index, oldest first, so it reads
//! only rows older than the cutoff. A pause between batches lets the other
//! writers take the lock, and one call deletes at most
//! `batch_size * max_batches` rows; the rest goes on the next call. No
//! transaction spans batches.
//!
//! The statements are a closed set of `const`s, one per [`RetentionTable`]:
//! no SQL here is built from runtime input.

use std::time::Duration;

use sqlx::SqlitePool;

/// Rows one batch deletes at most: a few milliseconds of write lock.
pub const BATCH_SIZE: i64 = 500;
/// Batches one call runs at most (20,000 rows), bounding each reconcile pass.
pub const MAX_BATCHES: u32 = 40;
/// The pause between batches, at least one batch's measured lock hold.
/// `SQLite`'s busy handler polls with sleeps of 1 to 100 ms, so a waiter in
/// another process needs the lock to stay free for a while, not just a yield.
pub const BATCH_PAUSE: Duration = Duration::from_millis(25);

/// A table the reconcile pass prunes by age, through [`prune`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetentionTable {
    /// `webhook_deliveries`, by `received_at`.
    WebhookDeliveries,
    /// `audit_events`, by `created_at`.
    AuditEvents,
    /// `capacity_samples`, by `recorded_at`.
    CapacitySamples,
    /// `host_samples`, by `received_at` (index added by migration 0051).
    HostSamples,
    /// `reconciler_passes`, by `started_at`.
    ReconcilerPasses,
    /// `fleet_health_hourly`, by `hour` (its primary key leads with it).
    FleetHealthHourly,
    /// `fleet_host_hourly`, by `hour` (its primary key leads with it).
    FleetHostHourly,
    /// `fleet_health_snapshots`, by `recorded_at` (its primary key leads with it).
    FleetHealthSnapshots,
}

impl RetentionTable {
    /// Every retained table, each once.
    pub const ALL: [Self; 8] = [
        Self::WebhookDeliveries,
        Self::AuditEvents,
        Self::CapacitySamples,
        Self::HostSamples,
        Self::ReconcilerPasses,
        Self::FleetHealthHourly,
        Self::FleetHostHourly,
        Self::FleetHealthSnapshots,
    ];

    /// The table's name, for log fields.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WebhookDeliveries => "webhook_deliveries",
            Self::AuditEvents => "audit_events",
            Self::CapacitySamples => "capacity_samples",
            Self::HostSamples => "host_samples",
            Self::ReconcilerPasses => "reconciler_passes",
            Self::FleetHealthHourly => "fleet_health_hourly",
            Self::FleetHostHourly => "fleet_host_hourly",
            Self::FleetHealthSnapshots => "fleet_health_snapshots",
        }
    }

    /// One batch of the table's prune, binding `(cutoff_millis, batch_size)`:
    /// the oldest rows whose cutoff column is before the cutoff. Ordering by
    /// that column (not by `rowid`, which plans as a full scan) lets the
    /// planner walk the column's index, so a batch never reads recent rows.
    #[must_use]
    pub const fn prune_batch(self) -> &'static str {
        match self {
            Self::WebhookDeliveries => {
                "DELETE FROM webhook_deliveries WHERE rowid IN (SELECT rowid FROM webhook_deliveries
                  WHERE received_at < ? ORDER BY received_at LIMIT ?)"
            }
            Self::AuditEvents => {
                "DELETE FROM audit_events WHERE rowid IN (SELECT rowid FROM audit_events
                  WHERE created_at < ? ORDER BY created_at LIMIT ?)"
            }
            Self::CapacitySamples => {
                "DELETE FROM capacity_samples WHERE rowid IN (SELECT rowid FROM capacity_samples
                  WHERE recorded_at < ? ORDER BY recorded_at LIMIT ?)"
            }
            Self::HostSamples => {
                "DELETE FROM host_samples WHERE rowid IN (SELECT rowid FROM host_samples
                  WHERE received_at < ? ORDER BY received_at LIMIT ?)"
            }
            Self::ReconcilerPasses => {
                "DELETE FROM reconciler_passes WHERE rowid IN (SELECT rowid FROM reconciler_passes
                  WHERE started_at < ? ORDER BY started_at LIMIT ?)"
            }
            Self::FleetHealthHourly => {
                "DELETE FROM fleet_health_hourly WHERE rowid IN (SELECT rowid FROM fleet_health_hourly
                  WHERE hour < ? ORDER BY hour LIMIT ?)"
            }
            Self::FleetHostHourly => {
                "DELETE FROM fleet_host_hourly WHERE rowid IN (SELECT rowid FROM fleet_host_hourly
                  WHERE hour < ? ORDER BY hour LIMIT ?)"
            }
            Self::FleetHealthSnapshots => {
                "DELETE FROM fleet_health_snapshots WHERE rowid IN (SELECT rowid FROM fleet_health_snapshots
                  WHERE recorded_at < ? ORDER BY recorded_at LIMIT ?)"
            }
        }
    }
}

/// Deletes the rows of `table` older than `cutoff_millis`, oldest first, in
/// batches of `batch_size`, stopping after `max_batches` batches or at the
/// first batch that is not full. Returns the number of rows deleted. A
/// `batch_size` below 1 deletes nothing.
///
/// Each batch is its own autocommit statement, so the write lock is released
/// between batches; [`BATCH_PAUSE`] is slept between them.
///
/// # Errors
///
/// The database error of a failing batch. Batches before it stay deleted.
pub async fn prune(
    pool: &SqlitePool,
    table: RetentionTable,
    cutoff_millis: i64,
    batch_size: i64,
    max_batches: u32,
) -> Result<u64, sqlx::Error> {
    delete_in_batches(
        pool,
        table.prune_batch(),
        cutoff_millis,
        batch_size,
        max_batches,
    )
    .await
}

/// The batch loop every bounded prune shares: runs `statement`, which binds
/// `(cutoff_millis, batch_size)` and deletes at most `batch_size` rows, until
/// a batch comes back short or `max_batches` have run, sleeping
/// [`BATCH_PAUSE`] between batches. `&'static str` keeps the statements a
/// closed set of constants.
pub(crate) async fn delete_in_batches(
    pool: &SqlitePool,
    statement: &'static str,
    cutoff_millis: i64,
    batch_size: i64,
    max_batches: u32,
) -> Result<u64, sqlx::Error> {
    // A negative LIMIT is no limit in SQLite: never let one through.
    let Ok(full @ 1..) = u64::try_from(batch_size) else {
        return Ok(0);
    };
    let mut deleted = 0;
    for batch in 0..max_batches {
        if batch > 0 {
            tokio::time::sleep(BATCH_PAUSE).await;
        }
        let rows = sqlx::query(statement)
            .bind(cutoff_millis)
            .bind(batch_size)
            .execute(pool)
            .await?
            .rows_affected();
        deleted += rows;
        if rows < full {
            break;
        }
    }
    Ok(deleted)
}
