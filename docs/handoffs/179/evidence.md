# Evidence: #179

## F (Phase 5)

- **Fact:** Every registry write path, including `poll_commands`' cancel and delivery step, the heartbeat and the handshake, runs in a `BEGIN IMMEDIATE` transaction that holds `SQLite`'s single write lock until commit.
  **Source:** `crates/gridops-core/src/fleet/registry/mod.rs:2045-2047`
  **Verbatim excerpt:**
  > async fn begin(&self) -> Result<Transaction<'_, Sqlite>, RegistryError> {
  >     Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
  > }

- **Fact:** The heartbeat path calls `cancel_undeliverable` (so `UNDELIVERABLE` runs on every heartbeat, not only on polls).
  **Source:** `crates/gridops-core/src/fleet/registry/inventory.rs:321`
  **Verbatim excerpt:**
  > super::commands::cancel_undeliverable(&mut tx, &session.host_id.to_string(), now).await?;

- **Fact:** A `fleet_events` row with a `placement_id` always carries that placement's own `host_id` (composite foreign key plus CHECK), so adding `host_id=?` bound to the placement's host to `LAST_FAILURE` selects the same rows.
  **Source:** `migrations/0022_fleet_foundation.sql:400-401`
  **Verbatim excerpt:**
  > FOREIGN KEY (placement_id,host_id,target_id) REFERENCES workload_placements(id,host_id,target_id),
  > CHECK (placement_id IS NULL OR (host_id IS NOT NULL AND target_id IS NOT NULL))

- **Fact:** `retry_failed_start` answers old failed placements from a read outside the write lock, so the in-lock `LAST_FAILURE` read runs only for a failed placement whose runner is still `starting`.
  **Source:** `crates/gridops-core/src/fleet/retry.rs:106-124`
  **Verbatim excerpt:**
  > // Most callers ask about placements that failed long ago; answer them
  > // without taking the write lock.
  > let Some((state, runner_status)) = sqlx::query_as::<_, (String, Option<String>)>(
  >     "SELECT p.state,r.status FROM workload_placements p
  >        LEFT JOIN runners r ON r.id=p.workload_id AND r.deleted_at IS NULL
  >       WHERE p.id=?",
  > )
  > .bind(&id)
  > .fetch_optional(pool)
  > .await?
  > else {
  >     return Ok(RetryOutcome::NotRetried(NotRetried::NotFound));
  > };
  > if state != "failed" {
  >     return Ok(RetryOutcome::NotRetried(NotRetried::NotFailed));
  > }
  > if runner_status.as_deref() != Some("starting") {
  >     return Ok(RetryOutcome::NotRetried(NotRetried::RunnerNotWanted));
  > }

- **Fact:** A resent heartbeat with the current sequence and the same digest is answered from the stored receipt, including the stored lease; any other reuse of a sequence is `HeartbeatConflict`.
  **Source:** `crates/gridops-core/src/fleet/registry/inventory.rs:275-302`
  **Verbatim excerpt:**
  > if sequence_i64 <= session.current_heartbeat_sequence {
  >     if sequence_i64 == session.current_heartbeat_sequence
  >         && session.current_heartbeat_digest.as_deref() == Some(digest.as_str())
  >     {
  >         let received = session
  >             .current_heartbeat_received_at
  >             .ok_or(InventoryError::HeartbeatConflict)?;
  >         let receipt_state = session
  >             .current_heartbeat_state
  >             .ok_or(InventoryError::HeartbeatConflict)?;
  >         let rotation = pending_rotation(&mut tx, &session, now).await?;
  >         let authority = work_authority(&mut tx, &session, None).await?;
  >         let response = heartbeat_response(
  >             &request,
  >             &session,
  >             rotation,
  >             authority,
  >             received,
  >             session
  >                 .current_heartbeat_lease_expires_at
  >                 .ok_or(InventoryError::HeartbeatConflict)?,
  >             receipt_state,
  >         )?;
  >         tx.commit().await?;
  >         return Ok(response);
  >     }
  >     return Err(InventoryError::HeartbeatConflict);
  > }

- **Fact:** A resent inventory with the current sequence and the same digest is answered from the stored `fleet_inventory_snapshots` row; any other reuse is `SequenceConflict`.
  **Source:** `crates/gridops-core/src/fleet/registry/inventory.rs:91-110`
  **Verbatim excerpt:**
  > if sequence_i64 <= session.current_inventory_sequence {
  >     if sequence_i64 == session.current_inventory_sequence
  >         && session.current_inventory_digest.as_deref() == Some(digest.as_str())
  >     {
  >         let row = sqlx::query(
  >             "SELECT digest,received_at,receipt_lease_expires_at,receipt_state,inventory_revision,
  >                     domain_mappings_json,backend_mappings_json,interactive_mappings_json
  >                FROM fleet_inventory_snapshots
  >               WHERE host_id=? AND authority_session_id=? AND inventory_sequence=?",
  >         )
  >         .bind(session.host_id.to_string())
  >         .bind(session.authority_session_id.to_string())
  >         .bind(sequence_i64)
  >         .fetch_optional(&mut *tx)
  >         .await?
  >         .ok_or(InventoryError::SequenceConflict)?;
  >         let stored_digest: String = row.get("digest");
  >         if stored_digest != digest {
  >             return Err(InventoryError::SequenceConflict);
  >         }

- **Fact:** The agent's client maps a 5xx, a 429, a transport error and an undecodable 2xx body to `Unavailable`, a 401 to `Unauthenticated`, and any other refusal to `Refused`; the in-session resend retries only `Unavailable`.
  **Source:** `crates/gridops-host-agent/src/client.rs:252-277`
  **Verbatim excerpt:**
  > async fn send<T: DeserializeOwned>(request: reqwest::RequestBuilder) -> Result<T, ClientError> {
  >     let response = request
  >         .send()
  >         .await
  >         .map_err(|error| ClientError::Unavailable(error.without_url().to_string()))?;
  >     let status = response.status();
  >     if status.is_success() {
  >         let body = response
  >             .bytes()
  >             .await
  >             .map_err(|error| ClientError::Unavailable(error.without_url().to_string()))?;
  >         // A reply with no body (204) is read as null. Treating it as a
  >         // failure made the agent resend every log batch it had delivered.
  >         let body = if body.is_empty() { &b"null"[..] } else { &body };
  >         return serde_json::from_slice(body)
  >             .map_err(|error| ClientError::Unavailable(error.to_string()));
  >     }
  >     if status == StatusCode::UNAUTHORIZED {
  >         return Err(ClientError::Unauthenticated);
  >     }
  >     match response.json::<FleetErrorResponse>().await {
  >         Ok(error) if !status.is_server_error() && status != StatusCode::TOO_MANY_REQUESTS => {
  >             Err(ClientError::Refused(error.code))
  >         }
  >         _ => Err(ClientError::Unavailable(status.to_string())),
  >     }
  > }

- **Fact:** `backoff()` never returns less than `MIN_BACKOFF` (1 s) and is capped at `MAX_BACKOFF` (30 s); the resend helper reuses it, clipped to the lease deadline.
  **Source:** `crates/gridops-host-agent/src/supervisor.rs:606-613`
  **Verbatim excerpt:**
  > /// Full-jitter exponential backoff from 1 s, capped at 30 s.
  > fn backoff(failures: u32) -> Duration {
  >     let ceiling = MIN_BACKOFF
  >         .saturating_mul(2_u32.saturating_pow(failures.min(5)))
  >         .min(MAX_BACKOFF);
  >     let millis = u64::try_from(ceiling.as_millis()).unwrap_or(u64::MAX);
  >     Duration::from_millis(rand::rng().random_range(MIN_BACKOFF.as_millis() as u64..=millis))
  > }

- **Fact:** The agent's reported OS is fixed at compile time, so on a macOS development machine the fleet_http agent enrolls as macOS (why `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails locally on macOS, independently of this change).
  **Source:** `crates/gridops-host-agent/src/probe.rs:16-17`
  **Verbatim excerpt:**
  > pub const fn host_os() -> ExecutionOs {
  >     if cfg!(target_os = "macos") {
