## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** The claim that `host_commands_poll_idx(host_id, state, cursor)` exists with exactly that column order and that a top-level `c.state IN ('pending','delivered')` makes SQLite use it with `state=?` — the schema/migration defining the index (file:line in migrations, or the `CREATE INDEX` DDL) would settle it. The measured plan table asserts the outcome but the index definition itself is not shown.
- **[NV-2]** The claim that `settle_unreceived` reads `state IN ('delivered','uncertain')` and runs at every handshake, and that `host_commands_poll_idx` can bound it — the source of `settle_unreceived` (file:line) would settle whether 'uncertain' rows are indexable under the same index and whether the done-when assertion (`uses host_commands_poll_idx with state=?`) is achievable for it.
- **[NV-3]** The claim that sqlx 0.9 logs slow statements at WARN over 1 s by default on target `sqlx::query`, and that `log_slow_statements(LevelFilter::Warn, 100 ms)` is the correct API — the sqlx 0.9 `SqliteConnectOptions` source/docs excerpt would settle it. Note the default threshold is a library-behavior hypothesis the brief's part 2 depends on.
- **[NV-4]** The claim that heartbeat/inventory/command-result endpoints have idempotent-replay rules (same operation id + sequence is a replay, not a conflict) in the registry — the registry replay/idempotency code (file:line) would settle it. The brief itself hedges ("Verify each endpoint's replay behaviour… before relying on it"), so this is acknowledged-unverified, not proven.
- **[NV-5]** The claim that `crates/gridops-api/tests/fleet_http/recovery.rs::host_recovers_when_a_delivered_command_never_reaches_it` exists and pins the delivered-command settle-on-reconnect behavior — the test source would settle it; the done-when requires it pass "unchanged," which is only checkable against the real file.
- **[NV-6]** The claim that `POLL_STEP` is 500 ms, `busy_timeout` is 5 s in `db.rs`, and the api maps code 5 to 503 `DependencyUnavailable` — file:line excerpts would settle each; none are shown.
- **[NV-7]** The claim that logs (`ship_logs`) and output already never end the session — the `supervisor.rs` error-handling code would settle it; part 3 relies on it being already true.

### WARN findings
- **[W-1]** The agent test's lease-expiry arm ("when the 503s outlast the observation lease, the agent ends the session and reconnects") is timing-dependent and unbounded in wall-clock time: the lease duration and backoff (1–30 s full jitter) are not bounded in the brief. Recommend the brief state the test's maximum lease/backoff configuration so the test doesn't take minutes in CI.
- **[W-2]** Part 1's sweep scope is open-ended ("about 21 statements" plus every statement inside any registry `BEGIN IMMEDIATE` on the hot paths, plus the reconciler fleet pass). The done-when ("add the same kind of assertion for every other hot-path statement the sweep changed") inherits that open-endedness. Recommend enumerating the statements to be swept, or defining a mechanical criterion for "hot path," so the sweep table can be checked for completeness at the diff gate.
- **[W-3]** The planner regression test asserts `state=?` in the plan, but a rewritten sargable form might legitimately produce `state IN (...)` plan text rather than `state=?` depending on SQLite version. Recommend the criterion allow the equivalent plan shape or pin the SQLite version.
- **[W-4]** The slow-statement logging test "must fail on today's code" — a property of a test relative to a code state, not directly runnable as written. Recommend restating as: the EnvFilter-construction function's output admits `sqlx::query` at WARN, and the connect options carry the 100 ms threshold (the brief's second alternative is the testable one; lead with it).
- **[W-5]** Part 3's retry window is defined by "the observation lease the server last granted," but the brief gives no lease duration, no source location for how it's granted, and no handling of the case where no lease has been granted yet (retry before first heartbeat?). Recommend the brief specify the lease source and the no-lease-yet behavior.

### NIT findings
- **[NIT-1]** The brief says migration numbering must be "the next free number after master's 0047" while also shipping with epic #136's migration 0047 in the same deploy — worth stating explicitly that if #136 adds a further migration the number must be re-derived at handoff.
- **[NIT-2]** "The PR should name it so the operator can remove it" (Mars cron) is not reflected in the done-when list; either add "PR description names the cron" to done-when or accept it as untestable.
- **[NIT-3]** Part 2 says "Keep each process's filter construction in one testable function if it is not one already" — conditional shape instructions belong to the diff gate, not the brief; harmless but noise.

### Verdict
PASS — no BLOCK findings; implementation may proceed.