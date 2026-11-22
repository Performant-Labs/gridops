# Evidence: #152 prune fleet_inventory_snapshots

- **Fact:** the reconciler reads a setting as a JSON number and falls back to the given default when it is not stored or doesn't parse, so `inventorySnapshotRetentionDays` defaults to 7 with no stored row.
  **Source:** `crates/gridops-reconciler/src/main.rs:3889-3898`
  **Verbatim excerpt:**
  > async fn setting_i64(database: &SqlitePool, key: &str, fallback: i64) -> i64 {
  >     sqlx::query("SELECT value FROM settings WHERE key=?")
  >         .bind(key)
  >         .fetch_optional(database)
  >         .await
  >         .ok()
  >         .flatten()
  >         .and_then(|row| serde_json::from_str::<i64>(row.get::<&str, _>("value")).ok())
  >         .unwrap_or(fallback)
  > }

- **Fact:** the inventory resend path loads only a session that has not ended, so it can never read the current snapshot of an ended session, which the prune deletes once it is old enough.
  **Source:** `crates/gridops-core/src/fleet/registry/inventory.rs:404-411`
  **Verbatim excerpt:**
  > let row = sqlx::query(
  >     "SELECT session_id,host_epoch,boot_id,control_plane_incarnation,current_credential_id,current_credential_generation,
  >             state,lease_expires_at,current_inventory_sequence,current_inventory_digest,current_inventory_received_at,
  >             current_heartbeat_sequence,current_heartbeat_digest,current_heartbeat_received_at,
  >             current_heartbeat_lease_expires_at,current_heartbeat_state
  >        FROM fleet_observation_sessions WHERE host_id=? AND session_id=? AND ended_at IS NULL",
  > ).bind(authenticated.host_id.to_string()).bind(report.authority_session_id.to_string())
  > .fetch_optional(&mut **tx).await?.ok_or(InventoryError::IdentityConflict)?;

- **Fact:** the resend path reads a stored snapshot only at the session's current sequence; a report at an older sequence is decided from the session row, not from snapshot history.
  **Source:** `crates/gridops-core/src/fleet/registry/inventory.rs:91-93`
  **Verbatim excerpt:**
  > if sequence_i64 <= session.current_inventory_sequence {
  >     if sequence_i64 == session.current_inventory_sequence
  >         && session.current_inventory_digest.as_deref() == Some(digest.as_str())

- **Fact:** the approval-proof reader joins only the current snapshot of a session that has not ended.
  **Source:** `crates/gridops-core/src/fleet/registry/mod.rs:1291-1305`
  **Verbatim excerpt:**
  > JOIN fleet_inventory_snapshots i
  >   ON i.host_id=s.host_id
  >  AND i.authority_session_id=s.session_id
  >  AND i.inventory_sequence=s.current_inventory_sequence
  > JOIN host_credentials c
  >   ON c.host_id=s.host_id
  >  AND c.id=s.current_credential_id
  >  AND c.generation=s.current_credential_generation
  >  AND c.host_epoch=s.host_epoch
  > WHERE s.host_id=?
  >  AND s.session_id=?
  >  AND s.host_epoch=?
  >  AND s.boot_id=?
  >  AND s.control_plane_incarnation=?
  >  AND s.ended_at IS NULL

- **Fact:** the backend-currency reader joins only the current snapshot of a session that has not ended.
  **Source:** `crates/gridops-core/src/fleet/registry/queries.rs:1134-1136`
  **Verbatim excerpt:**
  > JOIN fleet_inventory_snapshots snap ON snap.host_id=s.host_id
  >   AND snap.authority_session_id=s.session_id
  >   AND snap.inventory_sequence=s.current_inventory_sequence
  **Source:** `crates/gridops-core/src/fleet/registry/queries.rs:1150`
  **Verbatim excerpt:**
  > AND s.ended_at IS NULL AND s.state!='conflicted'

- **Fact:** the third reader joins only the current snapshot of a session that has not ended.
  **Source:** `crates/gridops-core/src/fleet/registry/queries.rs:1354-1364`
  **Verbatim excerpt:**
  > JOIN fleet_inventory_snapshots snap ON snap.host_id=s.host_id
  >   AND snap.authority_session_id=s.session_id
  >   AND snap.inventory_sequence=s.current_inventory_sequence
  >   AND snap.host_epoch=s.host_epoch
  >   AND snap.control_plane_incarnation=s.control_plane_incarnation
  >   AND snap.digest=s.current_inventory_digest
  >   AND snap.received_at=s.current_inventory_received_at
  >   AND snap.inventory_revision=h.inventory_revision
  >   AND snap.receipt_state!='conflicted'
  > WHERE s.host_id=? AND s.session_id=? AND s.host_epoch=?
  >   AND s.control_plane_incarnation=? AND s.ended_at IS NULL

- **Fact:** the fourth reader is an `EXISTS` on the current snapshot of a session that has not ended (one line).
  **Source:** `crates/gridops-core/src/fleet/registry/queries.rs:1912`
  **Verbatim excerpt (two fragments of the one line, each copied exactly):**
  > WHERE s.host_id=? AND s.session_id=? AND s.ended_at IS NULL AND s.state!='conflicted'

  > AND EXISTS (SELECT 1 FROM fleet_inventory_snapshots snap WHERE snap.host_id=s.host_id AND snap.authority_session_id=s.session_id AND snap.inventory_sequence=s.current_inventory_sequence

- **Fact:** the triggers on the snapshot table are on INSERT and UPDATE only; none is a DELETE trigger, so the schema allows the prune's DELETE.
  **Source:** `migrations/0026_fleet_inventory.sql:325`, `:335`, `:348`
  **Verbatim excerpt:**
  > BEFORE INSERT ON fleet_inventory_snapshots
  > BEFORE UPDATE ON fleet_inventory_snapshots
  > BEFORE UPDATE OF host_id,authority_session_id,host_epoch,control_plane_incarnation ON fleet_inventory_snapshots

- **Fact:** every connection waits at most 5 s for the write lock, so a single long delete would make the agent hot paths fail with "database is locked".
  **Source:** `crates/gridops-core/src/db.rs:35`
  **Verbatim excerpt:**
  > .busy_timeout(Duration::from_secs(5))
