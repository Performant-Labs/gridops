//! #152: old inventory snapshots are pruned in bounded batches.
//!
//! Every reader of `fleet_inventory_snapshots` reads only the current
//! snapshot of a session that has not ended (`current_inventory_sequence`
//! with `ended_at IS NULL`), so a row may go once it is past the retention
//! window unless it is that row. The prune holds `SQLite`'s single write lock
//! one batch at a time, so a single call never deletes more than
//! `batch_size * max_batches` rows; the rest goes on the next call.

use std::time::Duration;

use crate::fleet::registry::snapshot_retention::{
    BATCH_PAUSE, BATCH_SIZE, MAX_BATCHES, prune_inventory_snapshots,
};

use super::*;

const NOW: i64 = 1_800_000_000_000;
const DAY: i64 = 86_400_000;
/// Seven days back: the default window.
const CUTOFF: i64 = NOW - 7 * DAY;
const OLD: i64 = CUTOFF - DAY;
const RECENT: i64 = NOW - DAY;

/// An enrolled host, named by the enrollment it was issued under.
async fn host(f: &Fixture, name: &str) -> Result<HostId> {
    Ok(f.consume(&f.issue_empty(name).await?).await?.host_id)
}

/// An observation session of `host` whose current inventory is `current`,
/// ended when `ended` is set. A host has at most one session that has not
/// ended.
async fn session(f: &Fixture, host: HostId, id: &str, current: i64, ended: bool) -> Result<()> {
    sqlx::query(
        "INSERT INTO fleet_observation_sessions
         (session_id,host_id,host_epoch,boot_id,control_plane_incarnation,negotiated_protocol,
          initial_credential_id,initial_credential_generation,current_credential_id,
          current_credential_generation,initial_request_id,initial_body_digest,state,
          created_at,last_seen_at,ended_at,lease_expires_at,handshake_received_at,
          handshake_lease_expires_at,handshake_state,current_inventory_sequence)
         SELECT ?,h.id,h.epoch,'boot-1','inc-1',1,c.id,c.generation,c.id,c.generation,
                ?,?,'observing',1,1,?,1,1,1,'observing',?
           FROM fleet_hosts h JOIN host_credentials c ON c.host_id=h.id AND c.revoked_at IS NULL
          WHERE h.id=? ORDER BY c.generation DESC LIMIT 1",
    )
    .bind(id)
    .bind(format!("request-{id}"))
    .bind(inventory_digest())
    .bind(ended.then_some(2_i64))
    .bind(current)
    .bind(host.to_string())
    .execute(&f.pool)
    .await?;
    Ok(())
}

/// Snapshot `sequence` of a session, received at `received_at`.
async fn snapshot(
    f: &Fixture,
    host: HostId,
    session: &str,
    sequence: i64,
    received_at: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO fleet_inventory_snapshots
         (host_id,authority_session_id,host_epoch,control_plane_incarnation,inventory_sequence,
          digest,snapshot_json,received_at,receipt_lease_expires_at,receipt_state,
          inventory_revision,domain_mappings_json,backend_mappings_json,interactive_mappings_json)
         SELECT ?,?,epoch,'inc-1',?,?,'{}',?,?,'reconciling',1,'[]','[]','[]'
           FROM fleet_hosts WHERE id=?",
    )
    .bind(host.to_string())
    .bind(session)
    .bind(sequence)
    .bind(inventory_digest())
    .bind(received_at)
    .bind(received_at + 60_000)
    .bind(host.to_string())
    .execute(&f.pool)
    .await?;
    Ok(())
}

/// Every snapshot left, as `(session, sequence)`, in that order.
async fn remaining(f: &Fixture) -> Result<Vec<(String, i64)>> {
    Ok(sqlx::query_as(
        "SELECT authority_session_id,inventory_sequence FROM fleet_inventory_snapshots
          ORDER BY authority_session_id,inventory_sequence",
    )
    .fetch_all(&f.pool)
    .await?)
}

fn rows(expected: &[(&str, i64)]) -> Vec<(String, i64)> {
    expected
        .iter()
        .map(|(session, sequence)| ((*session).to_owned(), *sequence))
        .collect()
}

#[tokio::test]
async fn the_prune_keeps_each_open_sessions_current_snapshot_and_everything_recent() -> Result<()> {
    let f = Fixture::new(NOW).await?;
    let a = host(&f, "snapshot-retention-a").await?;
    let b = host(&f, "snapshot-retention-b").await?;

    // Host a: its open session's current snapshot is old, and so is the
    // current snapshot of the session before it, which has ended. Both
    // sessions are at sequence 2, so only the session id tells them apart.
    session(&f, a, "a-open", 2, false).await?;
    snapshot(&f, a, "a-open", 1, OLD).await?;
    snapshot(&f, a, "a-open", 2, OLD - DAY).await?;
    session(&f, a, "a-ended", 2, true).await?;
    snapshot(&f, a, "a-ended", 1, OLD - 2 * DAY).await?;
    snapshot(&f, a, "a-ended", 2, OLD - DAY).await?;

    // Host b: an old snapshot, one received exactly at the cutoff (not
    // older than the window), and a recent current one; and an ended session
    // whose snapshots are all recent.
    session(&f, b, "b-open", 3, false).await?;
    snapshot(&f, b, "b-open", 1, OLD).await?;
    snapshot(&f, b, "b-open", 2, CUTOFF).await?;
    snapshot(&f, b, "b-open", 3, RECENT).await?;
    session(&f, b, "b-ended", 2, true).await?;
    snapshot(&f, b, "b-ended", 1, RECENT - DAY).await?;
    snapshot(&f, b, "b-ended", 2, RECENT).await?;

    let deleted = prune_inventory_snapshots(&f.pool, CUTOFF, BATCH_SIZE, MAX_BATCHES).await?;

    assert_eq!(
        remaining(&f).await?,
        rows(&[
            ("a-open", 2),
            ("b-ended", 1),
            ("b-ended", 2),
            ("b-open", 2),
            ("b-open", 3),
        ]),
        "only old rows that are not an open session's current snapshot go"
    );
    assert_eq!(deleted, 4, "the count of rows deleted is returned");
    // The sessions themselves are not touched.
    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM fleet_observation_sessions")
        .fetch_one(&f.pool)
        .await?;
    assert_eq!(sessions, 4);
    f.close().await
}

#[tokio::test]
async fn one_call_deletes_at_most_batch_size_times_max_batches_oldest_first() -> Result<()> {
    const BATCH: i64 = 4;
    let f = Fixture::new(NOW).await?;
    let a = host(&f, "snapshot-retention-batches").await?;
    session(&f, a, "s", 100, false).await?;
    // The open session's current snapshot, the oldest row of all: kept.
    snapshot(&f, a, "s", 100, OLD - 100 * DAY).await?;
    // Fifteen eligible rows. A higher sequence was received earlier, so
    // insertion (rowid) order is the reverse of age.
    for sequence in 1..=15 {
        snapshot(&f, a, "s", sequence, OLD - sequence * DAY).await?;
    }
    let left = |count: i64| {
        let mut expected = (1..=count)
            .map(|sequence| ("s".to_owned(), sequence))
            .collect::<Vec<_>>();
        expected.push(("s".to_owned(), 100));
        expected
    };

    // One batch never deletes more than the batch size, and it takes the
    // oldest rows first.
    assert_eq!(
        prune_inventory_snapshots(&f.pool, CUTOFF, BATCH, 1).await?,
        4
    );
    assert_eq!(remaining(&f).await?, left(11));

    // A call stops after max_batches full batches and says how many went.
    assert_eq!(
        prune_inventory_snapshots(&f.pool, CUTOFF, BATCH, 2).await?,
        8
    );
    assert_eq!(remaining(&f).await?, left(3));

    // The next call takes the rest, then there is nothing left to take.
    assert_eq!(
        prune_inventory_snapshots(&f.pool, CUTOFF, BATCH, 2).await?,
        3
    );
    assert_eq!(remaining(&f).await?, left(0));
    assert_eq!(
        prune_inventory_snapshots(&f.pool, CUTOFF, BATCH, 2).await?,
        0
    );
    assert_eq!(remaining(&f).await?, left(0));
    f.close().await
}

/// The decided bounds of one reconcile pass (#152 brief): 500-row batches,
/// at most 40 of them (20,000 rows), with a pause between batches of at
/// least the measured per-batch lock hold so the API's writers get in.
#[test]
fn a_pass_is_bounded_to_forty_batches_of_five_hundred_with_a_pause_between() {
    assert_eq!(BATCH_SIZE, 500);
    assert_eq!(MAX_BATCHES, 40);
    assert!(BATCH_PAUSE >= Duration::from_millis(25), "{BATCH_PAUSE:?}");
}
