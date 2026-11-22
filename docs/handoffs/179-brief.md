# [Fleet: SQLITE_BUSY makes the api 503 agents into a reconnect storm](https://github.com/Performant-Labs/gridops/issues/179)

Prod incident on Mars, 2026-11-21, 17:12–18:43 UTC (9:12–10:43 AM PST), plus a 2-minute burst at :17 of every hour all day. This fix ships in the same deploy as epic #136 (master `fe08e3d`, migration 0047). It is a correctness and performance fix with no UI.

## Root cause (proven on the prod DB, read-only)

Two agent requests take SQLite's write lock (`BEGIN IMMEDIATE`, `RegistryService::begin` in `crates/gridops-core/src/fleet/registry/mod.rs`). While holding it, they run queries whose cost grows with **every command ever issued to the host**:

1. **`cancel_undeliverable`** (`crates/gridops-core/src/fleet/registry/commands.rs` ~line 735). It runs on every heartbeat (`inventory.rs`, heartbeat path) and on every poll that finds an open command. Its `WHERE c.host_id=? AND ((c.state='delivered' AND …) OR (c.state='pending' AND …))` has no top-level `state` conjunct, so SQLite uses only the `host_id` prefix of `host_commands_host_kind_issued_idx(host_id,kind,issued_at)`. It reads every command row for the host and runs `json_extract` on each payload.
2. **The delivery `SELECT` in `poll_commands`** (`commands.rs` ~line 91) has the same shape (`c.state='pending' OR (c.state='delivered' AND …)`). It scans the host's history the same way and adds a temp B-tree for `ORDER BY c.cursor`.

Measured on Mars (warm cache) for host `6b94…` with 59,136 command rows:

| Query | Plan | Time |
|---|---|---|
| `cancel_undeliverable` select as written | `SEARCH c USING INDEX host_commands_host_kind_issued_idx (host_id=?)` | 176 ms |
| same plus `AND c.state IN ('pending','delivered')` | `SEARCH c USING INDEX host_commands_poll_idx (host_id=? AND state=?)` | 1 ms |
| poll delivery select as written | `host_kind_issued_idx (host_id=?)` + temp B-tree | 375 ms |

The indexes are `CREATE INDEX host_commands_poll_idx ON host_commands(host_id,state,cursor);` (`migrations/0022_fleet_foundation.sql:319`) and `CREATE INDEX host_commands_host_kind_issued_idx ON host_commands(host_id,kind,issued_at);` (`migrations/0041_placement_start_retry.sql:18`). The two selects are at `registry/commands.rs:91-97` (poll delivery) and `:741-750` (`cancel_undeliverable`, function at `:735`).

So a poll with an open command holds the write lock for about 550 ms. `commands::poll` in `crates/gridops-api/src/fleet/commands.rs` repeats it every 500 ms (`POLL_STEP`) for up to 25 s. With roughly 200 `inspect` commands per 5 minutes, a host almost always has an open command. One busy host alone keeps the write lock held more than 100% of the time, and four hosts do this. Every other writer (api, reconciler, manager) waits out `busy_timeout` (5 s, `crates/gridops-core/src/db.rs:32`) and fails with code 5. The api turns that into a 503 `DependencyUnavailable` (`crates/gridops-api/src/fleet/error.rs:69-76` maps every `RegistryError::Database` to `DependencyUnavailable`, and `:146` answers it as `SERVICE_UNAVAILABLE`). The poll loop is `crates/gridops-api/src/fleet/commands.rs:24` (`POLL_STEP` = 500 ms), `:30` (`wait_ms` bounded to 25 000) and `:54-57`.

**Why now:** `host_commands` has no retention. It held 176,828 rows (254 MB), and the daily volume rose from about 2–7k rows a day before 11-20 to 23k on 11-20 and 65k on 11-21, as fleet CI volume grew. The scan cost grows with the table, so the lock duty cycle crossed saturation on the morning of 11-21.

**Hourly trigger (host, not code):** Mars runs `/etc/cron.hourly/free` (`echo 1 > /proc/sys/vm/drop_caches`) and `/etc/cron.hourly/fstrim` (`fstrim /`) at :17 past every hour. That evicts the 2.9 GB DB from the page cache, so the same scans run cold, and it produced the hourly 2-minute code=5 bursts at :17–:18 all day, including after the reconciler restarted at 19:11. This is host configuration and **out of scope for the code change**. The PR should name it so the operator can remove it.

**Amplification (proven in code):** the host agent (`crates/gridops-host-agent/src/supervisor.rs`, `run`/`session`) ends its session on any error, including `ClientError::Unavailable` (5xx/429/transport). (`run`, `supervisor.rs:181-207`, stops only on `Unauthenticated` and reconnects on every other error; `session`, `:210-396`, propagates every client error with `?`. `client.rs:252-276` maps 401 to `Unauthenticated`, other non-5xx non-429 errors to `Refused`, and 5xx, 429 and transport errors to `Unavailable`. `internal()`, `supervisor.rs:622-624`, wraps local errors as `Unavailable` too.) It then reconnects with a new session: a handshake, a full inventory snapshot and samples. The old session's pending `prepare_environment`/`start_runner` commands then fail as `agent_session_lost` through `cancel_undeliverable`, and the reconciler issues new commands. During the storm, sessions created went from 0–1 to about 30 per 5 minutes, so each 503 added write load.

## Scope

### 1. Bounded queries on the lock-holding hot paths (the fix)

- Rewrite `cancel_undeliverable`'s select and the poll delivery select so that SQLite uses `host_commands_poll_idx(host_id,state,cursor)`, with a top-level `c.state IN ('pending','delivered')` (or an equivalent sargable form). Results must be identical: same rows, same order (`ORDER BY c.cursor`), and the same cancellation and failure semantics.
- **Fix the class, not just these two.** Sweep every `FROM host_commands` / `JOIN host_commands` statement (there are about 21 outside tests) and every other statement that runs inside a registry `BEGIN IMMEDIATE` on the agent hot paths: handshake, inventory, heartbeat, poll, result, logs, output and credentials. Also sweep the reconciler's fleet pass (`crates/gridops-reconciler/src/fleet.rs`, `crates/gridops-core/src/fleet/placement.rs`). Any of them that filters a large, unbounded history table (`host_commands`, `host_command_attempts`, `host_command_results`, `fleet_inventory_snapshots`, `host_samples`, `fleet_events`, `fleet_observation_sessions`) without an index that bounds it to the open or current rows gets the same treatment. Add an index through a new migration only where a predicate cannot reach an existing index; the expectation is that none is needed. If one is, use the next free number after master's 0047 and follow the existing migrations' style (none of them uses `IF NOT EXISTS`).
- Put each statement that the plan test asserts or the sweep changes (only those) in a `pub(super)` const. The existing private `STARTED` (`registry/unconfirmed.rs:67`) and `LIVE_WORKLOADS` (`fleet/targets.rs:795`) consts show the pattern.
- The existing poll and cancel behaviour tests are in the api crate (`fleet_http`), for example `recovery.rs:392` `host_recovers_when_a_delivered_command_never_reaches_it` and `inventory.rs:797` `idle_command_poll_answers_while_another_writer_holds_the_database`. Keep them passing. Add no `pub` items and no test hooks.
- `settle_unreceived` already has a top-level `state IN ('delivered','uncertain')` (`registry/commands.rs:541-544`; architecture review R2 confirmed it is bounded). SQLite renders a top-level `state IN (...)` on this index as `SEARCH c USING INDEX host_commands_poll_idx (host_id=? AND state=?)` (the Mars measurement above, and a local `sqlite3` `EXPLAIN QUERY PLAN` of `settle_unreceived`'s `WHERE` against the 0022 and 0041 index DDL), so the plan-test assertion wording holds for it. Include it in the plan test, but it needs no change.
- Hand-off: the result handoff includes a sweep table listing each statement, its plan before and after, and its verdict. Completeness method: the table has one row for every hit of `grep -rnE '(FROM|JOIN) host_commands\b' crates --include='*.rs'` outside test files (`*_tests.rs`, `tests.rs`, `tests/`), which gives 25 lines at brief time (`api/src/resources.rs` 3, `fleet/placement.rs` 6, `registry/commands.rs` 6, `registry/delete.rs` 1, `registry/retire.rs` 1, `registry/unconfirmed.rs` 3, `fleet/retry.rs` 1, `fleet/start_health.rs` 1, `fleet/targets.rs` 3), plus every statement run between `self.begin()` and `commit` in the hot-path registry functions named above and every statement on the other history tables named above in those functions and in the reconciler's fleet pass. The handoff states the grep it ran and its hit count.
- Keep the existing read-only pre-check in `poll_commands` (the `open` EXISTS in a read transaction). Do not add a new lock-avoidance protocol; with bounded queries the write transaction lasts about 1 ms.

### 2. Lock-holder visibility (issue's first "done when")

- sqlx 0.9 already logs slow statements at WARN over 1 s by default, on the `sqlx::query` log target (`sqlx-core-0.9.0/src/connection.rs:202-206`: `slow_statements_level: LevelFilter::Warn`, `slow_statements_duration: Duration::from_secs(1)`; `src/logger.rs:108-122`: `target: "sqlx::query"`). The api's default EnvFilter (`gridops=info,tower_http=info`) and the reconciler's (`gridops_reconciler=info`) drop that target, so prod never saw them. compose.yaml and .env.example set no `RUST_LOG`. The manager never opens the DB and is out of scope.
- In `connect_database_path` (`crates/gridops-core/src/db.rs`), set `log_slow_statements(LevelFilter::Warn, 100 ms)`. The measured lock holders took 176 ms and 375 ms warm, so 1 s would only catch cold-cache runs. After part 1 the hot-path statements take about 1 ms, so 100 ms stays quiet in normal operation.
- Add `sqlx::query=warn` to the default EnvFilter in `crates/gridops-api/src/main.rs` and in `crates/gridops-reconciler/src/main.rs`. Keep each process's filter construction in one testable function if it is not one already.
- Add a CHANGELOG note: anyone who sets `RUST_LOG` must include `sqlx::query=warn` to see slow statements.
- `log_slow_statements` takes `log::LevelFilter`, which sqlx does not re-export (`sqlx-core-0.9.0/src/connection.rs:8` `use log::LevelFilter;`, `:266` `fn log_slow_statements(self, level: LevelFilter, duration: Duration) -> Self;` on `ConnectOptions`; no `pub use log` in the `sqlx-0.9.0`, `sqlx-core-0.9.0` or `sqlx-sqlite-0.9.0` sources). `Cargo.lock` pins `log` 0.4.33. Add `log` as a direct dependency of gridops-core, at the version sqlx already pulls in.
- Put the slow-statement capture test in the api crate, next to the filter function. Core has no dev-dependencies.
- At 100 ms these will also log WARN: the reconciler's unbatched retention deletes (`main.rs` ~3282-3340) and the migrations run at connect. That is intended visibility. Name both in the CHANGELOG and the PR. "Quiet" applies to the agent hot paths only.

### 3. A busy DB must not turn into a reconnect storm (amplification)

- In the host agent, an `Unavailable` error on a **heartbeat or an inventory refresh** inside an established session must not end the session. These are the requests the agent makes most often, and they run in the loop that renews the lease. Retry the same request with the existing full-jitter `backoff` (1 s to 30 s) until a deadline.
  - The deadline is read **once**, when the request first fails, and never re-read: it is the observation lease expiry the server last granted. Store it in `Authority` as `lease_expires_at`, a field separate from the work-authority `expires_at`.
  - Past the deadline, return the error, and the session ends as today.
- **Command results are not retried in-session (decided).** `record_command_result` does not tie a result to a session, and a poison result answers 503 every time. A retry would loop forever while heartbeats kept granting work authority. A failed result still ends the session, and `deliver_results` with the 24 h `RESULT_REDELIVERY_WINDOW` heals it, as today.
- A retry must resend **the identical request** (same operation id, sequence number and body), so the server's existing idempotent-replay rules apply. A heartbeat or inventory with the same sequence and digest is a replay, not a conflict. Do not rebuild the report with a fresh `observed_at`. Verify each endpoint's replay behaviour in the registry before relying on it. If one endpoint has no replay rule, say so and keep today's behaviour for that endpoint only. (Checked for this brief: both endpoints have one. `submit_inventory`, `registry/inventory.rs:91-142`, answers a resend with the current sequence and the same digest from the stored `fleet_inventory_snapshots` row, and any other reuse is `SequenceConflict`. `heartbeat`, `:275-300`, answers a resend with the current sequence and the same digest with the stored receipt, including the stored `current_heartbeat_lease_expires_at`, and any other reuse is `HeartbeatConflict`. A resend whose first attempt never committed is simply a new, higher sequence.)
- **A command poll that fails with `Unavailable` still ends the session, as today.** The poll has no replay rule: a command the server marked delivered in a lost reply would stay `delivered` until its deadline, then go `uncertain` and withhold work authority, which is the 2026-10-14 lockout. Today the reconnect's `settle_unreceived` settles that command, and `crates/gridops-api/tests/fleet_http/recovery.rs` `host_recovers_when_a_delivered_command_never_reaches_it` pins it. That test must stay unchanged and passing.
- **When the session ends (exhaustive).** The only new behaviour in part 3 is that a heartbeat or an inventory refresh inside an established session is retried on `Unavailable`. Everything else ends the session exactly as today. The session ends and the agent reconnects (or stops) in each of these cases, and in no others:
  1. A heartbeat or an inventory refresh still fails with `Unavailable` when the `lease_expires_at` captured at its first failure has passed.
  2. Any request answers `Unauthenticated`: the loop stops as `CredentialRejected` (`supervisor.rs:196-198`), as today.
  3. Any request answers `Refused`: no retry, as today.
  4. A command poll fails with `Unavailable` (decided above).
  5. A command result fails with `Unavailable` (decided above).
  6. Any other error that ends a session today still does: the handshake, the first inventory and `deliver_results` of a new session, the credential-rotation calls (`exchange_rotation`, `acknowledge_rotation`), and local `internal()` errors, which the retry helper does not wrap.
- Logs (`ship_logs`) and output (`output.rs`) already never end the session. They are already compliant: add no second retry layer there. (`ship_logs`, `supervisor.rs:398-423`, puts a failed batch back and breaks; `output::ship`, `output.rs:48-50`, never returns, and `:187-194` treats `Unavailable`, `Unauthenticated` and `Forbidden`/`RateLimited` refusals as wait-and-retry.)
- Implementation shape:
  - Use one retry helper in `supervisor.rs`, next to `backoff()`.
  - Wrap only the `self.client.*` call, because `ClientError::Unavailable` also carries local `internal()` errors.
  - Keep the helper's attempt count separate from the reconnect `failures` counter.
  - Keep the observation lease expiry the server last granted in the existing `Authority` watch struct.
  - `client.rs` stays transport only.
  - Update the module doc comment ("Any failure drops the session…").
- The handshake and the first inventory of a new session keep today's reconnect behaviour.
- **Existing recovery tests** (`crates/gridops-api/tests/fleet_http/recovery.rs`): all keep their assertions. The poll and the result still end the session, so they should pass unchanged. If one waits for a new session and now times out because the session correctly survives an outage shorter than the lease, amending it is authorised. That covers `an_api_restart_leaves_a_running_runner_alone`. The amendment asserts that the session survives, keeps every runner, allocation and operation assertion, and is listed in the handoff. Only doc comments may change in `host_recovers_when_a_started_runner_result_is_lost_past_its_deadline` and `an_administrator_repairs_a_placement_the_agent_cannot_confirm`.
- Do not change the api's 503 mapping and do not add server-side BUSY retries. Once the queries are bounded, the agent-side retry is enough.

### Out of scope (decided)

- **Re-issuing a prepare or start command that a reconnect left undelivered** (an item in the issue's list). Part 3 removes the reconnects a busy DB causes, and part 1 removes the busy DB. A real session loss (agent restart or reboot) still fails the placement exactly as today, and the reconciler retries it. This PR does not change that.
- **Retention for `host_commands`.** After part 1 no hot path reads history. Retention is #152's area (table growth) and needs FK-aware deletes across attempts and results.
- **The Mars host cron** (`/etc/cron.hourly/free`, `fstrim`). This is host configuration and the operator removes it. The PR description names it.

## Constraints

- Build and test with `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2` (`.claude/settings.local.json`). Never set `CARGO_TARGET_DIR` and never commit `.cargo/`. Prefer `-p <crate>` test runs.
- Work headlessly. There is no UI surface.
- No deploy; the orchestrating session deploys.
- The PR description must say which part is proven (parts 1 and 2: the query plans and the timings above) and which is defensive (part 3).

## Done when

- **Planner regression test (core):** an in-crate registry test module, included from `registry/tests.rs` and shaped like `tests/retention.rs`. Seed a host with a large command history, at least 20k terminal (`succeeded`/`failed`/`cancelled`) commands plus a few `pending`/`delivered` ones. `EXPLAIN QUERY PLAN` the `pub(super)` const SQL that `cancel_undeliverable`, `poll_commands` and `settle_unreceived` execute. Assert that each one uses `host_commands_poll_idx` with `state=?` and never `host_commands_host_kind_issued_idx (host_id=?)`. Add the same kind of assertion for every other hot-path statement the sweep changed.
- **Behaviour tests (core):** existing `cancel_undeliverable` and poll tests still pass unchanged. A new test with a large terminal history plus open commands shows the same cancellations, placement failures and delivery order as before.
- **Agent test:** write it in `recovery.rs`, or move Proxy/Faults into a shared `fleet_http` module. Use the real Supervisor plus the Proxy/Faults 503 injection, and do not build a new fake server. The server answers a heartbeat (and separately an inventory refresh) with 503 several times and then succeeds.
  - Lease-expiry arm: the test proxy rewrites `lease.expires_at` in heartbeat and inventory replies to a short value. Add no production lease knob and no test-only Supervisor builder.
  - Heartbeat arm and inventory-refresh arm (each asserted separately): across the 503s and the success the agent sends exactly one handshake, keeps the same `authority_session_id`, and resends the byte-identical request body (same operation id, sequence and `observed_at`).
  - Backoff, observed at the proxy (extend `Faults` to fail the next N requests for a path and to record each request's body and arrival `Instant`): consecutive attempts of the same body are at least 1 s apart (`MIN_BACKOFF`, `supervisor.rs:48`; `backoff()` at `:602-608` never returns less), allowing about 100 ms for scheduling. A test heartbeat interval of 200 ms would show gaps far below that if the agent resent without backing off.
  - Lease-expiry arm: the 503s outlast the rewritten lease deadline, so the agent ends the session and reconnects as today. Assert exactly one handshake before the deadline passes and a second handshake after it, with a new `authority_session_id`.
  - If the implementer finds that an endpoint has no replay rule after all (both have one at brief time; see part 3), that endpoint's arm asserts today's behaviour instead: the 503 ends the session and the agent sends a second handshake. The handoff names the endpoint and the reason.
  - A 401 still stops the loop as `CredentialRejected`.
  - A command result that gets a 503 still ends the session, as today.
- **Held-lock test** (in `fleet_http`, with the held-writer pattern from `inventory.rs` 793-811, `idle_command_poll_answers_while_another_writer_holds_the_database`, which opens `f.database.begin_with("BEGIN IMMEDIATE")` at `:800`): with **no open command** (so the poll stays read-only), hold a `BEGIN IMMEDIATE` write lock from a second connection for longer than `busy_timeout`. Drive the real Supervisor against the api in that window and show the session survives: no second handshake and no new `fleet_observation_sessions` row. Release the lock and the agent carries on. Bound the hold to a little past `busy_timeout` (for example 6-8 s). That is far inside the 90 s observation lease (`OBSERVATION_LEASE_MILLIS`, `registry/inventory.rs:37`), so this test needs no lease rewrite, and after the release the next retry comes within the backoff ceiling of the attempts made so far (1, 2, 4, 8 s for attempts 0-3), so the test finishes well inside the 45 s `within` helper (`recovery.rs:114-126`).
- **Slow-statement logging test**, which must fail on today's code: the api's and the reconciler's default EnvFilter admit `sqlx::query` at WARN. Also test that `connect_database_path`'s options carry the 100 ms threshold, or that a deliberately slow statement emits a WARN on the `sqlx::query` target. The behaviour-level test (a statement over 100 ms through a pool from `connect_database_path` emits a WARN event on the `sqlx::query` target) is the one to write: `connect_database_path` builds its `SqliteConnectOptions` inline and returns only the pool (`db.rs:23-43`), so inspecting the options from the api crate would need a new `pub` item.
- `CHANGELOG.md` entry under Unreleased.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test -p gridops-core -p gridops-api -p gridops-host-agent -p gridops-reconciler` pass.

## Source material read

Read for the round 1 brief-review amendment (in full unless a range is given):

- `docs/handoffs/179-brief.md`; `docs/handoffs/179/decisions.md`; `docs/handoffs/179/handoff-A.md`; `git log -p` of the brief.
- `crates/gridops-host-agent/src/supervisor.rs` (in full); `crates/gridops-host-agent/src/client.rs` (error variants and `send` mapping, located by grep); `crates/gridops-host-agent/src/output.rs` (`:1-20`, and its `Err` arms located by grep).
- `crates/gridops-core/src/fleet/registry/commands.rs` (`:20-140`, `:520-560`, `:720-786`); `crates/gridops-core/src/fleet/registry/inventory.rs` (`:30-350`, sequence/replay helpers by grep `:400-510`); `crates/gridops-core/src/fleet/registry/mod.rs` (test-module wiring); `crates/gridops-core/src/fleet/registry/tests.rs` (module list); `crates/gridops-core/src/fleet/registry/unconfirmed.rs:67`; `crates/gridops-core/src/fleet/targets.rs:795`; `crates/gridops-core/src/fleet/registry/handshake.rs:26`.
- `crates/gridops-core/src/db.rs` (`:20-40`); `crates/gridops-core/Cargo.toml`.
- `crates/gridops-api/src/fleet/commands.rs` (`POLL_STEP` and poll loop); `crates/gridops-api/src/fleet/error.rs` (`:60-80`, `:130-148`); `crates/gridops-api/src/main.rs` (EnvFilter, `:38`, `:70-73`); `crates/gridops-reconciler/src/main.rs` (EnvFilter, `:46`, `:431`).
- `crates/gridops-api/tests/fleet_http/recovery.rs` (`:1-130`, `:585-680`, test list by grep); `crates/gridops-api/tests/fleet_http/inventory.rs` (`:785-812`).
- `migrations/0022_fleet_foundation.sql:319`; `migrations/0041_placement_start_retry.sql:18`.
- `compose.yaml` and `.env.example` (grep for `RUST_LOG`: none); `CHANGELOG.md` (`## Unreleased` at `:11`); `Cargo.lock` (`log` 0.4.33).
- Third-party: `sqlx-core-0.9.0/src/connection.rs` (`:8`, `:196-216`, `:262-271`), `sqlx-core-0.9.0/src/logger.rs` (`:1-40`, `:108-122`); `sqlx-0.9.0` and `sqlx-sqlite-0.9.0` sources (grep for a `log` re-export: none).
- Not read for this round: `crates/gridops-reconciler/src/fleet.rs`, `crates/gridops-core/src/fleet/placement.rs`, `crates/gridops-core/tests/retention.rs` (exists), and the rest of `crates/gridops-reconciler/src/main.rs`. No finding in this round needed them; the sweep in part 1 reads them.
