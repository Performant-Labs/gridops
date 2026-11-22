# Handoff-S: Phase 8 - SQLITE_BUSY reconnect storm: bounded hot-path queries, lock-holder visibility, in-session retry (#179)

**Date:** 2026-11-21
**Branch:** claude/lock-storm-179-17e061 (head f9a0b4d; diff base fe08e3d)
**Issue:** #179
**Brief:** docs/handoffs/179-brief.md
**Handoff-T reviewed:** docs/handoffs/179/handoff-T-green.md (and handoff-T-red.md)
**Handoff-A reviewed:** docs/handoffs/179/handoff-A.md (up-front, PASS) and handoff-A-dup.md (anti-duplication, PASS)
**Handoff-F reviewed:** docs/handoffs/179/handoff-F.md (and evidence.md, decisions.md)
**Operator-facing report:** N/A (no visual surface)

## A precondition
Confirmed: handoff-A.md is PASS (0 block, 5 warn, all folded into T and F). handoff-A-dup.md is PASS (no duplication; 3 warn, none drift).

## T precondition
Confirmed: handoff-T-green.md reports "Blocking issues: None". Its one failing test is environmental (see Known issues).

## Visual-diff-tool precondition
N/A. The change has no UI surface (brief: "no UI"). No browser or pixel-diff gate applies, and none was run.

## Preview / spec sanity check
The brief was checked against its own conventions and against the code before auditing. No source-of-truth defect found, so no ADVISORY-HOLD.
- The one wording gap (brief says `lease_expires_at` lives in `Authority`; A's R3 warn 1 recommends a local) is a wording preference, not a contradiction. The brief's behavioural requirement is unaffected. See Scope check.
- The brief's old "about 21 statements" wording was already corrected to the 25 grep hits at R3.

## Visual diff results
N/A.

## Spec compliance

Audit method: read the full production diff (`git diff fe08e3d..HEAD`, 10 production files, 18 files with tests), the 11 new tests, F's sweep table and evidence.md, and re-ran the verification myself. Independent checks below are mine, not copied from F or T.

### Part 1: bounded queries on the lock-holding hot paths

| Requirement | Compliant | How verified |
|---|---|---|
| `cancel_undeliverable` and the poll delivery select use `host_commands_poll_idx` via a top-level `state IN ('pending','delivered')` | YES | `commands.rs` `NEXT_DELIVERY`, `UNDELIVERABLE`. Plan test green. My own `EXPLAIN QUERY PLAN` on a scratch DB with all 47 migrations (sqlite 3.54.1): `SEARCH c USING INDEX host_commands_poll_idx (host_id=? AND state=?)` for both, `USE TEMP B-TREE FOR ORDER BY` only over open rows. |
| Same rows, same order, same cancellation and failure semantics | YES | By construction (each added conjunct is implied by the unchanged disjunction under it; bind order unchanged). **Independent randomized proof:** 30,000 commands over 3 hosts in all 8 states, random sessions/epochs/deadlines/attempt counts and a random half of the delivered ones with attempt rows. 360 old-vs-new comparisons (180 parameter sets times the two selects, ordered by cursor), 324 non-empty, **0 mismatches**. The core behaviour test `a_long_history_changes_nothing_poll_cancels_or_delivers` also passes. |
| Sweep the class (every `FROM/JOIN host_commands`, every hot-path in-lock statement, other history tables, reconciler fleet pass) | YES | F states its grep and count. I re-ran `grep -rnE '(FROM\|JOIN) host_commands\b' crates --include='*.rs'` minus test files: **25 hits**, matching F's 25 rows and the brief's per-file breakdown. I also grepped every statement on `host_command_attempts`, `host_command_results`, `fleet_inventory_snapshots`, `host_samples`, `fleet_events`, `fleet_observation_sessions` outside tests and checked the ones F's table does not list individually (`workload.rs:130` runs on the pool for a chart, `queries.rs:1327` and `:1534` are read-snapshot reads, `registry/mod.rs:1289` is an admin session-keyed lookup, `queries.rs:1126/1346` are keyed by session and sequence). None is an in-lock agent hot-path or reconciler-pass history scan. |
| Add a migration only where a predicate cannot reach an existing index | YES (none needed) | No migration in the diff. `LAST_FAILURE` reaches `fleet_events_host_cursor_idx`. |
| Each asserted or changed statement in a `pub(super)` const; no new `pub` items, no test hooks | YES | Diff scan: every new item is `pub(super)` or private; `client.rs` unchanged. |
| `settle_unreceived` in the plan test, no change | YES | SQL moved into `UNRECEIVED` verbatim (diff confirms). Covered by the plan test. |
| Keep the read-only `open` pre-check in `poll_commands`; no new lock-avoidance protocol | YES | Untouched. |
| Sweep finding `LAST_FAILURE` (retry.rs) is equivalent | YES | Schema check: `fleet_events` has FK `(placement_id,host_id,target_id)` to `workload_placements(id,host_id,target_id)` plus `CHECK (placement_id IS NULL OR (host_id IS NOT NULL AND target_id IS NOT NULL))`, so every event with a `placement_id` carries a non-null host that equals the placement's. `workload_placements.host_id` is `NOT NULL` and is in the immutability trigger's column list (migration 0022:439). The `row.get::<String,_>("host_id")` therefore cannot fail and the added `host_id=?` finds the same row. |
| Sweep table handed off with plans before and after and a verdict per statement | YES | handoff-F.md "Sweep table", 25 grep rows plus the in-lock hot-path rows plus "seen and deliberately left". |

### Part 2: lock-holder visibility

| Requirement | Compliant | Notes |
|---|---|---|
| `log_slow_statements(Warn, 100 ms)` in `connect_database_path` | YES | `db.rs`, named const `SLOW_STATEMENT`. |
| `sqlx::query=warn` in the api's and the reconciler's default EnvFilter, each in one testable function | YES | `default_log_filter()` in both `main.rs`; `main` uses it when `RUST_LOG` is unset or invalid. Manager untouched, as the brief says. |
| `log` as a direct core dependency at the version sqlx pulls in | YES | Declared via `[workspace.dependencies]` (A warn 5); `Cargo.lock` gains one line, version 0.4.33, nothing new downloaded. |
| CHANGELOG: `RUST_LOG` must include `sqlx::query=warn`; name migrations and retention deletes | YES | "Upgrading" entry names both expected warning sources and the `RUST_LOG` rule; "Fixed" entry describes the storm fix. |
| Slow-statement test in the api crate, fails on today's code | YES | `log_filter_tests.rs` installs a process-global subscriber built from `default_log_filter()` (A warn 4: sqlx emits on its worker thread), drives a real pool from `connect_database_path`, asserts target `sqlx::query`, level WARN, `slow_threshold` `100ms`, and that non-slow statements stay out. T-red recorded it RED (logged nothing) on today's code. |

### Part 3: amplification (in-session retry)

| Requirement | Compliant | Notes |
|---|---|---|
| Heartbeat and inventory refresh retried on `Unavailable` with the existing `backoff()` until a deadline | YES | `resend_while_leased`, next to `backoff()`; wraps only the `self.client.*` call, so `internal()` errors still end the session at once. |
| Deadline read once, at first failure, never re-read; past it the session ends as today | YES | `lease_expires_at` is passed by value; last wait clipped to `min(backoff, deadline - now)`; `left <= 0` returns the error. |
| Identical request on every resend (same operation id, sequence, body, `observed_at`) | YES | Request is built once outside the closure. The test compares the raw bodies byte for byte across the 503s and the success. |
| Replay rules exist server-side for both endpoints | YES | I re-read `registry/inventory.rs:91-142` (inventory: same sequence and digest answered from the stored `fleet_inventory_snapshots` row, otherwise `SequenceConflict`) and `:275-300` (heartbeat: same sequence and digest answered with the stored receipt and `current_heartbeat_lease_expires_at`, otherwise `HeartbeatConflict`). No endpoint needed the "no replay rule" fallback. |
| Command poll and command result still end the session on `Unavailable` | YES | Untouched code paths. `host_recovers_when_a_delivered_command_never_reaches_it` is unchanged and passes; `a_result_the_api_cannot_take_still_ends_the_session` pins the result case. |
| 401 still stops the loop as `CredentialRejected`; `Refused` not retried | YES | `result => return result` for every non-`Unavailable` outcome; `a_heartbeat_refused_as_unauthenticated_still_stops_the_agent`. |
| Helper attempt count separate from `run`'s `failures`; handshake, first inventory, `deliver_results`, rotation untouched | YES | Local `attempts`; diff shows those call sites unchanged. |
| Logs and output get no second retry layer | YES | `ship_logs` and `output.rs` untouched. |
| `client.rs` stays transport only | YES | Zero diff lines. |
| Module doc comment updated | YES | Rewritten at the top of `supervisor.rs`. |
| `lease_expires_at` stored in `Authority` | NO, documented deviation | Kept as a local of the same name in the `heartbeats` block, seeded from the first inventory lease and updated from every successful refresh and heartbeat reply. See Scope check. |

### Done-when checklist

| Done-when item | Status | Evidence |
|---|---|---|
| Planner regression test (core), `pub(super)` consts, large history, poll index with `state=?`, never `(host_id=?)` alone, plus every other changed statement | PASS | `query_plan_tests.rs`: two plan tests (three `host_commands` consts; `LAST_FAILURE` with its own, correctly different assertion). A's warn 2 was followed: the negative clause forbids any step ending `(host_id=?)` and any `SCAN`, not one index name. |
| Behaviour tests: existing unchanged; new large-history test shows same cancellations, failures, delivery order | PASS | Existing poll/cancel tests live in `fleet_http` (A warn 3) and pass unchanged. New core test seeds a dozen open commands among 20k finished ones, across every branch of both disjunctions, plus a session-ended case. |
| Agent test: real Supervisor, Proxy/Faults 503 injection, no new fake server, no production lease knob | PASS | `busy_database.rs`; `Faults` extended in place (fail next N for a path, record body and arrival `Instant`, shorten leases in replies). No production or Supervisor-builder change for tests. |
| Heartbeat arm and inventory arm separate; one handshake, same session id, byte-identical body, gaps at least 1 s (900 ms allowance) | PASS | `resent_unchanged("/heartbeat")` and `resent_unchanged("/inventory")`, each its own test. |
| Lease-expiry arm: second handshake only after the deadline, new session id | PASS | `a_heartbeat_outage_that_outlasts_the_lease_ends_the_session`. |
| 401 stops; result 503 still ends the session | PASS | Both guard tests. |
| Held-lock test with no open command, hold past `busy_timeout` | PASS | `a_database_held_past_the_busy_timeout_leaves_the_session_alone`: asserts zero open commands, a 503 really happened during the hold (so it cannot pass vacuously), one handshake, same session id, no new `fleet_observation_sessions` row, and a successful heartbeat after release. |
| Slow-statement logging test fails on today's code | PASS | See Part 2. |
| CHANGELOG entry under Unreleased | PASS | |
| fmt, clippy, four-crate tests | PASS, one pre-existing macOS-only failure | See Quality audit. |

### Independent verification run by S
All with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, in this worktree:
- `cargo fmt --all -- --check`: clean.
- `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (CI's toolchain and flags): clean.
- `cargo test -p gridops-core --lib`: 310 passed, 0 failed (includes the 3 new plan/behaviour tests).
- `cargo test -p gridops-api --bin gridops-api log_filter` and `cargo test -p gridops-reconciler --bin gridops-reconciler log_filter`: 1 passed each.
- `cargo test -p gridops-host-agent --lib`: 8 passed.
- `cargo test -p gridops-api --test fleet_http -- busy_database:: recovery:: inventory::idle_command_poll`: 21 passed, 0 failed (all 14 `recovery::` tests unchanged, including `host_recovers_when_a_delivered_command_never_reaches_it` and `an_api_restart_leaves_a_running_runner_alone`).
- `cargo test -p gridops-api --test fleet_http` (full): 149 passed, 1 failed, the macOS-only `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` (matches F and T; see Known issues).
- Randomized old-vs-new equivalence of `NEXT_DELIVERY` and `UNDELIVERABLE` (scratch DB, scripts in the session scratchpad, not committed): 360 comparisons, 0 mismatches.

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | N/A | No endpoint, protocol or 503-mapping change (brief: keep the api's mapping). |
| Error handling | PASS | `Unavailable` retried only for heartbeat and inventory refresh; every other error path unchanged; deadline bounds the retry so it cannot loop forever (undecodable 2xx maps to `Unavailable` and is bounded the same way). |
| UI/UX match to spec | N/A | No UI. |
| Accessibility | N/A | No UI. |
| Architecture gate | PASS | A: PASS, A-dup: PASS. Their warns are all either applied (A R3 warns 1-5) or recorded as a deviation (see Scope check). No conflict between S findings and A's verdict. |
| Code organization | PASS | SQL in the registry, retry beside `backoff()`, filters per process, `log` through the workspace table. No dead code, TODO or debug output in the diff. One `#[allow(clippy::too_many_arguments)]` on a test helper (`History::raw`), the same pattern the crate already uses in `queries.rs`. |
| Security | PASS | No new input or auth surface. The 100 ms WARN logs statement text, not bound values (sqlx logs SQL, not binds), and the repo's dynamic SQL still binds its values. The resend reuses the same credential and request; no secret is logged by the new `tracing::warn!` (it logs the client error text, delay and attempt count). |
| Performance | PASS | Hot-path selects now plan as `SEARCH ... host_commands_poll_idx (host_id=? AND state=?)`, bounded to open rows. `LAST_FAILURE` is bounded to one machine's events (newest first, `LIMIT 1`) instead of a whole-table scan. See advisory 3 for the residual bound. |
| Visual regression | N/A | No UI. |
| Naming consistency | PASS | `NEXT_DELIVERY`, `UNDELIVERABLE`, `UNRECEIVED`, `LAST_FAILURE`, `SLOW_STATEMENT`, `default_log_filter`, `resend_while_leased` read plainly and follow the `STARTED`/`LIVE_WORKLOADS` precedent. |
| Test quality (`testing/test-quality.md` §7) | PASS | See below. No "delete or merge" finding. |

### Test-quality audit (rubric §7, all 11 new tests)

| Test | One behaviour | Fails in isolation for the right reason | Tier / non-duplicative | Behavioural |
|---|---|---|---|---|
| core `hot_path_command_statements_read_only_open_commands_through_the_poll_index` | The three in-lock `host_commands` selects reach only open rows | Yes (T mutation: dropping the `state` conjunct from `UNDELIVERABLE` fails it) | Core, cheapest tier for a plan | A plan test is structural by nature; the brief mandates it and it protects a performance behaviour, not an implementation detail |
| core `the_start_retry_reads_one_hosts_events_not_the_whole_table` | `LAST_FAILURE` is host-bounded | Yes (T mutation: un-host it fails it) | Own fixture, own (correctly different) assertion | Same as above |
| core `a_long_history_changes_nothing_poll_cancels_or_delivers` | Same cancellations, failures and delivery order with a 20k history | Yes (T behaviour-guard mutations: DESC order and a dropped epoch branch each fail it); passes on old and new code by design | Core; the api-tier poll/cancel tests it complements are unchanged and not re-proven | Effects only (states, attempts, placement events), no call counts |
| api `a_statement_running_past_100_ms_is_logged_as_a_slow_statement_warning` | A >100 ms statement through `connect_database_path` logs WARN on `sqlx::query` | Yes; RED on old code (logged nothing); asserts `slow_threshold=100ms` so neither the 1 s default nor a 0 threshold passes | Api crate (only place with the filter); one test, not one per layer | Behavioural; adapts the statement length so it does not depend on machine speed |
| reconciler `the_default_filter_admits_slow_statement_warnings_and_not_every_statement` | The reconciler's default filter admits `sqlx::query` WARN only | Yes (RED on old code) | Unit tier, scoped subscriber; does not re-prove the 100 ms threshold the api test owns | Behavioural |
| `a_heartbeat_the_api_cannot_answer_is_resent_unchanged_in_the_same_session` | Heartbeat 503s resent unchanged, same session, backing off | Yes (T mutation: no retry fails it) | Integration through the real Supervisor and api, the only tier that can prove it | Behavioural |
| `an_inventory_refresh_the_api_cannot_answer_is_resent_unchanged_in_the_same_session` | Same for the inventory refresh | Yes (same mutation) | A separate call site of the helper, so not a duplicate; shares one helper (`resent_unchanged`), not copy-paste | Behavioural |
| `a_heartbeat_outage_that_outlasts_the_lease_ends_the_session` | Retry is bounded by the lease captured at the first failure | Yes (same mutation) | Integration | Behavioural |
| `a_heartbeat_refused_as_unauthenticated_still_stops_the_agent` | A 401 is never retried | Green by design (guard); would fail if the helper retried non-`Unavailable` | Integration | Behavioural |
| `a_result_the_api_cannot_take_still_ends_the_session` | A result 503 still ends the session | Green by design (guard); would fail on an in-session result retry | Integration | Behavioural |
| `a_database_held_past_the_busy_timeout_leaves_the_session_alone` | The real incident: a held write lock past `busy_timeout` does not end the session | Yes (RED on today's code); checks a 503 really occurred during the hold | Integration; its unique value over the heartbeat arm is the real DB lock and the api's real BUSY-to-503 mapping | Behavioural |

Suite proportionality: eleven tests for a three-part feature fix (plan, behaviour, filter, six agent arms), each mapped to a Done-when item. No assertion-free, tautological, unreachable-outcome, duplicate-signal, mock-shaped, snapshot or coverage-padding test. The two guard tests are green by design and say so. Observations that are not findings: the plan test seeds 20k rows that do not change an unanalysed plan (the brief requires the seed and A warn 2 noted it); the inventory arm takes about 35 s of wall clock because the refresh fires 30 s into a session (the brief prescribes this).

## Scope check
- **Delivered:** all three parts, the CHANGELOG, and every Done-when test. No UI, no deploy, no migration, no schema or protocol change, no 503-mapping change, no server-side BUSY retry, no retention work, no cron change. All the brief's "out of scope (decided)" items are untouched.
- **One extra change, inside scope:** `LAST_FAILURE` in `fleet/retry.rs`. It is a statement on the reconciler's fleet pass that runs under `BEGIN IMMEDIATE` and filtered `fleet_events` (a history table) with no host bound; the brief's "fix the class" sweep clause requires exactly this. It adds no migration, and T added its plan assertion in Phase 7. Not over-delivery.
- **One deviation from the brief's wording:** `lease_expires_at` is a local in the heartbeat loop, not an `Authority` field. F documented it; A's R3 warn 1 recommended it (the field would be stale for the heartbeat that follows a successful refresh, and only this loop reads it); A-dup recorded it as not drift; T found behaviour matches. The brief's behavioural requirement (a deadline separate from the work-authority `expires_at`, read once at first failure) is met. I accept it. O should align the brief's wording or say so in the PR.
- **Existing recovery tests:** no amendment was needed. No test body in `recovery.rs` changed; only the shared `Faults`/`Proxy`/`within`/`agent_session` items became `pub(super)` and `Faults` grew the injection, lease-shortening and request-log fields the brief asks for.

## Known issues (not caused by this change)
`fleet_http release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on a macOS development machine (I reproduced it: 149 passed, 1 failed). `probe::host_os()` is `cfg!(target_os = "macos")`, so on a Mac the fixture's "Linux" machine enrolls as macOS and the preflight passes `macos-machine`. The diff touches none of `probe.rs`, the preflight or enrollment, and the test passes on CI's Linux runners. Not a regression.

## Verdict

PASS - all acceptance criteria met, spec-compliant, quality acceptable. Ready for O to commit.

## Advisory notes
Non-blocking. Items 1 and 2 are for the PR description.
1. **Say which part is proven.** Parts 1 and 2 are proven (the query plans, the Mars timings of 176 ms and 375 ms against 1 ms, and the randomized equivalence above). Part 3 is defensive and was not reproduced on Mars.
2. **Part 3 does not prevent every storm reconnect, by design.** The command poll still ends the session on `Unavailable` (the 2026-10-14 lockout reason in the brief). A poll with an open command still takes the write lock, so part 1 (bounded queries) is what actually removes the busy database; part 3 only stops a heartbeat or inventory 503 from adding a new session on top of it. The PR should name the Mars host cron (`/etc/cron.hourly/free` with `drop_caches`, and `fstrim`, both at :17) as the cause of the hourly cold-cache bursts and ask the operator to remove it. It should also name the intended new WARNs from migrations at connect and from the reconciler's unbatched retention deletes (`main.rs` ~3343).
3. **`LAST_FAILURE` is bounded to one machine's events, not to one row.** That is enough today because the lock-free pre-check lets through only placements that failed within the 5-minute retry window. `fleet_events` has no retention (issue #152 area); if per-machine event volume grows large, an index on `(host_id,placement_id,kind,cursor)` would give a true bound. No action now.
4. **`delete_pool`'s `DELETE FROM fleet_events WHERE placement_id IN (...)` (`targets.rs:849`) is a whole-table scan.** It is admin-only and not on an agent or reconciler hot path, so it is correctly left alone; worth noting for #152.
5. **Clock comparison.** The agent compares the server's wall-clock lease with its own clock (`gridops_core::now_millis()`). Skew only shifts the resend window; it cannot loop forever, because a server-side lease expiry answers `SessionConflict` (`Refused`), which ends the session. F's choice (no monotonic conversion) is reasonable and consistent with the rest of the agent.
6. **Test-suite cost.** The inventory arm adds about 35 s and the held-lock test about 8 s to `fleet_http`; the full suite's wall clock was about 90 s on this loaded machine. Acceptable for what they prove.
