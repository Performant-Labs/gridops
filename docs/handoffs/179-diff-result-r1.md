## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** crates/gridops-core/src/db.rs:40 — the claim that sqlx 0.9 emits the slow-statement event on target `sqlx::query` at WARN with a field `slow_threshold` rendered as the string `"100ms"` (the api test asserts this exact field/string) — settle with `sqlx-core-0.9.0/src/logger.rs` (the summary formatting of `slow_threshold`) and a live run of the test.

**[NV-2]** crates/gridops-core/src/fleet/registry/commands.rs:48-77 — the claim that SQLite renders the added top-level `c.state IN ('pending','delivered')` as `SEARCH … host_commands_poll_idx (host_id=? AND state=?)` (not `(host_id=?)` alone, and no `SCAN`) for these exact statements against the 0022/0041 index DDL — settle with `EXPLAIN QUERY PLAN` of the three consts on a DB with all migrations applied (the green test run claims it, but the plan text itself is not in this prompt).

**[NV-3]** crates/gridops-api/tests/fleet_http/busy_database.rs:260-283 — the claim that a heartbeat (and/or an inventory refresh) reliably falls inside the 7.5 s `BEGIN IMMEDIATE` hold so that the "no heartbeat failed … proves nothing" assertion is not flaky — settle with the fixture's agent heartbeat interval (`HEARTBEAT_INTERVAL_MILLIS` is 15 000 per handshake.rs:27; the interval is `min(lease heartbeat_ms, self.heartbeat)`, and the test-side `self.heartbeat` override is not shown). If the effective interval is ≥ 8 s the hold window can contain no heartbeat at all and the test fails or proves nothing roughly half the time.

**[NV-4]** crates/gridops-host-agent/src/supervisor.rs:615-655 — the claim that comparing the server wall-clock `WireTimestamp` lease against `gridops_core::now_millis()` cannot hang or loop forever (disclosed by the author; the SessionConflict→Refused backstop is asserted, not shown) — settle with the registry's lease-expiry error path in `load_current_session`/`heartbeat` showing an expired lease maps to a `Refused`-class code, not `Unavailable`.

**[NV-5]** crates/gridops-core/src/fleet/retry.rs:157-165 — the claim that `fleet_events.host_id` is guaranteed non-NULL and equal to the placement's host for every row with `placement_id=?` — the shown FK at 0022:400-401 supports it, but only if FKs were enforced at insert time for all pre-existing rows; settle with a `PRAGMA foreign_key_check` result on prod/Mars data (read-only) confirming no orphaned `(placement_id,host_id,target_id)` triples exist.

**[NV-6]** crates/gridops-api/src/log_filter_tests.rs:70-80 — the claim that `EXPLAIN`-style ad-hoc SQL via `sqlx::AssertSqlSafe` returns a `detail` column and that installing a process-global subscriber in one test does not perturb other tests in the api bin that capture tracing output — settle with the full api bin test run (claimed green) and the sqlx `Row` column naming for `EXPLAIN QUERY PLAN`.

### WARN findings
**[W-1]** crates/gridops-api/tests/fleet_http/busy_database.rs:247-283 — the held-lock test's self-check assertion makes the test fail (rather than pass vacuously) when no heartbeat lands in the 7.5 s window; with a 15 s heartbeat interval this is a coin flip unless the fixture shortens the interval below the hold. Recommend holding across at least one guaranteed heartbeat cadence, or arming the fault window after observing one heartbeat's timing.

**[W-2]** crates/gridops-host-agent/src/supervisor.rs:287 — deviation from the brief's "store it in `Authority` as `lease_expires_at`" (kept as a loop-local). The behavior matches the brief's requirement and the deviation is disclosed, but it forecloses other consumers (e.g. logs/output paths) from ever seeing the lease, and couples the deadline to the heartbeat loop's structure. Recommend recording the deviation in the PR description so the reviewer of the next agent change knows the invariants live in `session`, not `Authority`.

**[W-3]** crates/gridops-api/tests/fleet_http/busy_database.rs:126-142 — `resent_unchanged` allows a 90 s wait and only filters attempts with `seen.at >= armed`, but does not bound the total attempt count; a slow machine where backoff reaches 30 s could produce fewer than 4 attempts within the window and time out rather than fail informatively. Acceptable, but a longer `wait_for` limit or an explicit timeout message naming the observed attempt count would aid diagnosis.

### NIT findings
**[NIT-1]** CHANGELOG.md (first hunk) — the `sqlx::query=warn` / 100 ms note appears to be added to the "Known problems"-style bullet list rather than an Upgrading/Changed section; check the section header it landed under.

**[NIT-2]** crates/gridops-core/src/fleet/registry/commands.rs:67-68 — doc comment says UNDELIVERABLE "Binds the host, now, and the latest deadline within the delivery margin"; it actually binds `now` and `now + DELIVERY_MARGIN_MILLIS` (the pending cutoff). Minor wording mismatch with the call site.

### Verdict
PASS — no BLOCK findings; testing may proceed.