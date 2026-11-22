//! Pruning old inventory snapshots (#152).
//!
//! Every agent inventory report adds a row to `fleet_inventory_snapshots`,
//! about 4 KB each and thousands a day, and nothing reads a row once it is no
//! longer a session's current snapshot: every reader joins the snapshot at the
//! session's `current_inventory_sequence` and requires the session not to have
//! ended. So a row may go once it is older than the retention window, unless
//! it is the current snapshot of a session that has not ended.
//!
//! The prune runs under `SQLite`'s single write lock, which the agents' hot
//! paths wait on for at most the busy timeout. Each batch is therefore one
//! autocommit statement of at most `batch_size` rows, found through the
//! `received_at` index so it reads only rows older than the cutoff, with a
//! pause between batches that lets the API's writers (another process) take
//! the lock. One call deletes at most `batch_size * max_batches` rows; the rest
//! goes on the next call.

use std::{ops::RangeInclusive, time::Duration};

use sqlx::SqlitePool;

/// The system setting holding how many days of snapshots are kept.
pub const RETENTION_DAYS_KEY: &str = "inventorySnapshotRetentionDays";
/// Days kept when the setting is not stored.
pub const DEFAULT_RETENTION_DAYS: i64 = 7;
/// The days the setting accepts; a stored value outside them is clamped.
pub const RETENTION_DAYS: RangeInclusive<i64> = 1..=3_650;

/// Rows one batch deletes at most: about 2 MB of row data, a few
/// milliseconds of write lock.
pub const BATCH_SIZE: i64 = 500;
/// Batches one call runs at most (20,000 rows), bounding each reconcile pass.
pub const MAX_BATCHES: u32 = 40;
/// The pause between batches, at least one batch's measured lock hold.
/// `SQLite`'s busy handler polls with sleeps of 1 to 100 ms, so a waiter in
/// another process needs the lock to stay free for a while, not just a yield.
pub const BATCH_PAUSE: Duration = Duration::from_millis(25);

/// One batch, binding `(cutoff_millis, batch_size)`: the oldest rows received
/// before the cutoff that are not an open session's current snapshot.
/// Ordering by `received_at` lets the planner walk the `received_at` index,
/// so a batch never reads the recent rows.
pub const PRUNE_BATCH: &str = "DELETE FROM fleet_inventory_snapshots
 WHERE rowid IN (SELECT snap.rowid FROM fleet_inventory_snapshots snap
                  WHERE snap.received_at < ?
                    AND NOT EXISTS (SELECT 1 FROM fleet_observation_sessions s
                                     WHERE s.host_id=snap.host_id
                                       AND s.session_id=snap.authority_session_id
                                       AND s.current_inventory_sequence=snap.inventory_sequence
                                       AND s.ended_at IS NULL)
                  ORDER BY snap.received_at LIMIT ?)";

/// The cutoff for keeping `days` days back from `now_millis`, with `days`
/// clamped to [`RETENTION_DAYS`].
#[must_use]
pub fn cutoff_millis(now_millis: i64, days: i64) -> i64 {
    let days = days.clamp(*RETENTION_DAYS.start(), *RETENTION_DAYS.end());
    now_millis - days * 86_400_000
}

/// Deletes snapshots received before `cutoff_millis` that are not the current
/// snapshot of a session that has not ended, oldest first, in batches of
/// `batch_size`, stopping after `max_batches` batches or at the first batch
/// that is not full. Returns the number of rows deleted. A `batch_size` below
/// 1 deletes nothing.
///
/// Each batch is its own autocommit statement, so the write lock is released
/// between batches; [`BATCH_PAUSE`] is slept between them.
///
/// # Errors
///
/// The database error of a failing batch. Batches before it stay deleted.
pub async fn prune_inventory_snapshots(
    pool: &SqlitePool,
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
        let rows = sqlx::query(PRUNE_BATCH)
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
