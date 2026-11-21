## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** `host_commands_poll_idx(host_id, state, cursor)` already exists in the schema with that exact column order (the brief's plan table shows SQLite using it, and the fix relies on reaching it). — Evidence: the `CREATE INDEX` statement in migrations up to 0047, or the live schema of the prod DB backup.

**[NV-2]** sqlx's connection options in `connect_database_path` support `log_slow_statements(LevelFilter::Warn, Duration)` chained as specified, and "leave normal statements at their current level" is achievable via the same options API. — Evidence: sqlx `ConnectOptions` trait source / docs for the pinned sqlx version in `Cargo.lock`.

**[NV-3]** The host agent can compute "when the observation lease the server last granted would expire" — i.e. the lease TTL/expiry is available client-side from the handshake/heartbeat responses. — Evidence: the session/handshake response struct in `crates/gridops-host-agent/src/supervisor.rs` and the corresponding api response payload.

**[NV-4]** The observation-lease/idempotent-replay claims: heartbeat/inventory with same sequence+digest is a replay not a conflict, and `Unavailable` maps from 5xx/429/transport as the brief states. — Evidence: registry replay/idempotency code in `crates/gridops-core/src/fleet/registry/` and the `ClientError` mapping in the host-agent HTTP client. (The brief itself flags this as "verify each endpoint's replay behaviour" — correctly self-hedged.)

**[NV-5]** 401 maps to `CredentialRejected` in the host-agent loop (asserted in the agent test's done-when). — Evidence: error mapping in `crates/gridops-host-agent/src/supervisor.rs` / client error enum.

### WARN findings
**[W-1]** Done-when for slow logging accepts "a unit test or doc comment naming the threshold" — a doc comment is not an executable criterion and would let the requirement ship untested. Recommend requiring the unit test (or at minimum a test that the option is set on the built options).

**[W-2]** The held-lock test requires holding a write lock longer than `busy_timeout` (5 s per `db.rs`), so the test will take at least ~5–6 s of wall time. Recommend stating whether the test DB may use a shorter `busy_timeout` to keep suite time bounded, or explicitly accepting the duration.

**[W-3]** The sweep scope ("about 21 statements outside tests", reconciler pass, placement) is broad; the done-when only requires planner assertions for "every other hot-path statement the sweep changed" without defining "hot-path statement". Recommend the brief name the enumerated list (or require the PR to include it) so coverage of the sweep is checkable rather than discretionary.

**[W-4]** Missing error path: part 3 doesn't specify behaviour when a retried request's lease-expiry deadline is reached mid-backoff with the server still returning 503 while other operations (e.g. poll vs heartbeat) are pending — i.e., is the session torn down immediately, or do in-flight retries finish first? Recommend one sentence pinning the teardown rule.

### NIT findings
**[NIT-1]** "Out of scope" says "Part 3 removes the reconnects a busy DB causes" — slightly overstated: an outage longer than the observation lease still ends the session by design. "Defers them past transient BUSY windows" would be accurate.

**[NIT-2]** The migration instruction "Use the next free migration number after master's 0047" races with epic #136 shipping in the same deploy — recommend stating the tie-break (e.g. renumber at rebase if both add migrations).

### Verdict
PASS — no BLOCK findings; implementation may proceed.