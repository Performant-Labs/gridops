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
- **Fix the class, not just these two.** Sweep every `FROM host_commands` / `JOIN host_commands` statement (there are about 21 outside tests) and every other statement that runs inside a registry `BEGIN IMMEDIATE` on the agent hot paths: handshake, inventory, heartbeat, poll, result, logs, output and credentials. Also sweep the reconciler's fleet pass (`crates/gridops-reconciler/src/fleet.rs`, `crates/gridops-core/src/fleet/placement.rs`). Any of them that filters a large, unbounded history table (`host_commands`, `host_command_attempts`, `host_command_results`, `fleet_inventory_snapshots`, `host_samples`, `fleet_events`, `fleet_observation_sessions`) without an index that bounds it to the open or current rows gets the same treatment. Add an index through a new migration only where a predicate cannot reach an existing index. Use the next free migration number after master's 0047, and make it `CREATE INDEX IF NOT EXISTS`.
- Keep the existing read-only pre-check in `poll_commands` (the `open` EXISTS in a read transaction). Do not add a new lock-avoidance protocol; with bounded queries the write transaction lasts about 1 ms.

### 2. Lock-holder visibility (issue's first "done when")

- In `connect_database_path` (`crates/gridops-core/src/db.rs`), enable sqlx slow-statement logging: `log_slow_statements(LevelFilter::Warn, 1 s)`. Leave normal statements at their current level. All three processes share this function, so a statement that holds the lock for more than 1 s is named in that process's log.

### 3. A busy DB must not turn into a reconnect storm (amplification)

- In the host agent, an `Unavailable` error on a request **inside an established session** must not end the session: heartbeat, inventory refresh, command poll, command result, logs and output. Retry that same request with the existing full-jitter `backoff` (1 s to 30 s) while the session can still be valid, meaning before the observation lease the server last granted would expire. End the session and reconnect only when the lease would expire, on `Unauthenticated`, or on `Refused`, exactly as today.
- A retry must resend **the identical request** (same operation id, sequence number and body), so the server's existing idempotent-replay rules apply. A heartbeat or inventory with the same sequence and digest is a replay, not a conflict. Do not rebuild the report with a fresh `observed_at`. Verify each endpoint's replay behaviour in the registry before relying on it. If one endpoint has no replay rule, say so and keep today's behaviour for that endpoint only.
- The handshake and the first inventory of a new session keep today's reconnect behaviour.
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

- **Planner regression test (core):** seed a host with a large command history, at least 20k terminal (`succeeded`/`failed`/`cancelled`) commands plus a few `pending`/`delivered` ones. `EXPLAIN QUERY PLAN` the exact SQL strings that `cancel_undeliverable` and `poll_commands` execute (expose them as consts or a test hook). Assert that each one uses `host_commands_poll_idx` with `state=?` and never `host_commands_host_kind_issued_idx (host_id=?)`. Add the same kind of assertion for every other hot-path statement the sweep changed.
- **Behaviour tests (core):** existing `cancel_undeliverable` and poll tests still pass unchanged. A new test with a large terminal history plus open commands shows the same cancellations, placement failures and delivery order as before.
- **Agent test (host-agent):** a fake server answers a heartbeat (and separately a poll) with 503 several times and then succeeds. The agent sends exactly one handshake, keeps the same `authority_session_id`, resends the identical request body, and backs off between attempts. When the 503s outlast the observation lease, the agent ends the session and reconnects as today. A 401 still stops the loop as `CredentialRejected`.
- **Held-lock test:** a test holds a `BEGIN IMMEDIATE` write lock on the test DB from a second connection for longer than `busy_timeout`, drives an agent session against the api in that window, and shows the session survives: no second handshake and no new `fleet_observation_sessions` row. Then release the lock and the agent carries on. If driving the real api from host-agent tests is not practical, compose it at the api crate with a fake agent loop and say so.
- Slow-statement logging is enabled in `connect_database_path`, with a unit test or doc comment naming the threshold.
- `CHANGELOG.md` entry under Unreleased.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test -p gridops-core -p gridops-api -p gridops-host-agent -p gridops-reconciler` pass.
