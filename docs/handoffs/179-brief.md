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

So a poll with an open command holds the write lock for about 550 ms. `commands::poll` in `crates/gridops-api/src/fleet/commands.rs` repeats it every 500 ms (`POLL_STEP`) for up to 25 s. With roughly 200 `inspect` commands per 5 minutes, a host almost always has an open command. One busy host alone keeps the write lock held more than 100% of the time, and four hosts do this. Every other writer (api, reconciler, manager) waits out `busy_timeout` (5 s, `crates/gridops-core/src/db.rs`) and fails with code 5. The api turns that into a 503 `DependencyUnavailable`.

**Why now:** `host_commands` has no retention. It held 176,828 rows (254 MB), and the daily volume rose from about 2–7k rows a day before 11-20 to 23k on 11-20 and 65k on 11-21, as fleet CI volume grew. The scan cost grows with the table, so the lock duty cycle crossed saturation on the morning of 11-21.

**Hourly trigger (host, not code):** Mars runs `/etc/cron.hourly/free` (`echo 1 > /proc/sys/vm/drop_caches`) and `/etc/cron.hourly/fstrim` (`fstrim /`) at :17 past every hour. That evicts the 2.9 GB DB from the page cache, so the same scans run cold, and it produced the hourly 2-minute code=5 bursts at :17–:18 all day, including after the reconciler restarted at 19:11. This is host configuration and **out of scope for the code change**. The PR should name it so the operator can remove it.

**Amplification (proven in code):** the host agent (`crates/gridops-host-agent/src/supervisor.rs`, `run`/`session`) ends its session on any error, including `ClientError::Unavailable` (5xx/429/transport). It then reconnects with a new session: a handshake, a full inventory snapshot and samples. The old session's pending `prepare_environment`/`start_runner` commands then fail as `agent_session_lost` through `cancel_undeliverable`, and the reconciler issues new commands. During the storm, sessions created went from 0–1 to about 30 per 5 minutes, so each 503 added write load.

## Scope

### 1. Bounded queries on the lock-holding hot paths (the fix)

- Rewrite `cancel_undeliverable`'s select and the poll delivery select so that SQLite uses `host_commands_poll_idx(host_id,state,cursor)`, with a top-level `c.state IN ('pending','delivered')` (or an equivalent sargable form). Results must be identical: same rows, same order (`ORDER BY c.cursor`), and the same cancellation and failure semantics.
- **Fix the class, not just these two.** Sweep every `FROM host_commands` / `JOIN host_commands` statement (there are about 21 outside tests) and every other statement that runs inside a registry `BEGIN IMMEDIATE` on the agent hot paths: handshake, inventory, heartbeat, poll, result, logs, output and credentials. Also sweep the reconciler's fleet pass (`crates/gridops-reconciler/src/fleet.rs`, `crates/gridops-core/src/fleet/placement.rs`). Any of them that filters a large, unbounded history table (`host_commands`, `host_command_attempts`, `host_command_results`, `fleet_inventory_snapshots`, `host_samples`, `fleet_events`, `fleet_observation_sessions`) without an index that bounds it to the open or current rows gets the same treatment. Add an index through a new migration only where a predicate cannot reach an existing index; the expectation is that none is needed. If one is, use the next free number after master's 0047 and follow the existing migrations' style (none of them uses `IF NOT EXISTS`).
- Put each statement that the plan test asserts or the sweep changes (only those) in a `pub(super)` const. The existing private `STARTED` and `LIVE_WORKLOADS` consts show the pattern.
- The existing poll and cancel behaviour tests are in the api crate (`fleet_http`). Keep them passing. Add no `pub` items and no test hooks.
- `settle_unreceived` already has a top-level `state IN (...)` (architecture review R2 confirmed it is bounded). Include it in the plan test, but it needs no change.
- Hand-off: the result handoff includes a sweep table listing each statement, its plan before and after, and its verdict.
- Keep the existing read-only pre-check in `poll_commands` (the `open` EXISTS in a read transaction). Do not add a new lock-avoidance protocol; with bounded queries the write transaction lasts about 1 ms.

### 2. Lock-holder visibility (issue's first "done when")

- sqlx 0.9 already logs slow statements at WARN over 1 s by default, on the `sqlx::query` log target. The api's default EnvFilter (`gridops=info,tower_http=info`) and the reconciler's (`gridops_reconciler=info`) drop that target, so prod never saw them. compose.yaml and .env.example set no `RUST_LOG`. The manager never opens the DB and is out of scope.
- In `connect_database_path` (`crates/gridops-core/src/db.rs`), set `log_slow_statements(LevelFilter::Warn, 100 ms)`. The measured lock holders took 176 ms and 375 ms warm, so 1 s would only catch cold-cache runs. After part 1 the hot-path statements take about 1 ms, so 100 ms stays quiet in normal operation.
- Add `sqlx::query=warn` to the default EnvFilter in `crates/gridops-api/src/main.rs` and in `crates/gridops-reconciler/src/main.rs`. Keep each process's filter construction in one testable function if it is not one already.
- Add a CHANGELOG note: anyone who sets `RUST_LOG` must include `sqlx::query=warn` to see slow statements.
- `log_slow_statements` takes `log::LevelFilter`, which sqlx does not re-export. Add `log` as a direct dependency of gridops-core, at the version sqlx already pulls in.
- Put the slow-statement capture test in the api crate, next to the filter function. Core has no dev-dependencies.
- At 100 ms these will also log WARN: the reconciler's unbatched retention deletes (`main.rs` ~3282-3340) and the migrations run at connect. That is intended visibility. Name both in the CHANGELOG and the PR. "Quiet" applies to the agent hot paths only.

### 3. A busy DB must not turn into a reconnect storm (amplification)

- In the host agent, an `Unavailable` error on a **heartbeat or an inventory refresh** inside an established session must not end the session. These are the requests the agent makes most often, and they run in the loop that renews the lease. Retry the same request with the existing full-jitter `backoff` (1 s to 30 s) until a deadline.
  - The deadline is read **once**, when the request first fails, and never re-read: it is the observation lease expiry the server last granted. Store it in `Authority` as `lease_expires_at`, a field separate from the work-authority `expires_at`.
  - Past the deadline, return the error, and the session ends as today.
- **Command results are not retried in-session (decided).** `record_command_result` does not tie a result to a session, and a poison result answers 503 every time. A retry would loop forever while heartbeats kept granting work authority. A failed result still ends the session, and `deliver_results` with the 24 h `RESULT_REDELIVERY_WINDOW` heals it, as today. End the session and reconnect only when the lease would expire, on `Unauthenticated`, or on `Refused`, exactly as today.
- A retry must resend **the identical request** (same operation id, sequence number and body), so the server's existing idempotent-replay rules apply. A heartbeat or inventory with the same sequence and digest is a replay, not a conflict. Do not rebuild the report with a fresh `observed_at`. Verify each endpoint's replay behaviour in the registry before relying on it. If one endpoint has no replay rule, say so and keep today's behaviour for that endpoint only.
- **A command poll that fails with `Unavailable` still ends the session, as today.** The poll has no replay rule: a command the server marked delivered in a lost reply would stay `delivered` until its deadline, then go `uncertain` and withhold work authority, which is the 2026-10-14 lockout. Today the reconnect's `settle_unreceived` settles that command, and `crates/gridops-api/tests/fleet_http/recovery.rs` `host_recovers_when_a_delivered_command_never_reaches_it` pins it. That test must stay unchanged and passing.
- Logs (`ship_logs`) and output (`output.rs`) already never end the session. They are already compliant: add no second retry layer there.
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
  - In both cases the agent sends exactly one handshake, keeps the same `authority_session_id`, resends the identical request body, and backs off between attempts.
  - When the 503s outlast the lease deadline, the agent ends the session and reconnects as today.
  - A 401 still stops the loop as `CredentialRejected`.
  - A command result that gets a 503 still ends the session, as today.
- **Held-lock test** (in `fleet_http`, with the held-writer pattern from `inventory.rs` ~793-811): with **no open command** (so the poll stays read-only), hold a `BEGIN IMMEDIATE` write lock from a second connection for longer than `busy_timeout`. Drive the real Supervisor against the api in that window and show the session survives: no second handshake and no new `fleet_observation_sessions` row. Release the lock and the agent carries on.
- **Slow-statement logging test**, which must fail on today's code: the api's and the reconciler's default EnvFilter admit `sqlx::query` at WARN. Also test that `connect_database_path`'s options carry the 100 ms threshold, or that a deliberately slow statement emits a WARN on the `sqlx::query` target.
- `CHANGELOG.md` entry under Unreleased.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test -p gridops-core -p gridops-api -p gridops-host-agent -p gridops-reconciler` pass.
