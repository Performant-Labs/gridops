# Decision journal: #183 retire a GitLab connection's fleet targets on removal

## Phase 3 Architecture (A, up-front plan review) - 2026-11-22T08:41:00-08:00
- **Decided:** BLOCK, with 2 block and 6 warn findings (`docs/handoffs/183/handoff-A.md`).
  - **Block 1 [correctness]: item 2 refuses removal on `admitted` intents.**
    - An intent never leaves `admitted`. It is set at `admission.rs:1195`, in the same transaction that inserts the placement and the allocation. Its only other status change is `queued`→`cancelled` (`service.rs:138`), and only `delete_pool` deletes intents.
    - Refusing on `admitted` would make any connection whose target has surviving history impossible to remove, which is the #183 bug again. It also contradicts item 1 and test 1.
    - Fix: refuse on `queued` intents, plus the `LIVE_WORKLOADS`-shaped placement and allocation check keyed by target.
  - **Block 2 [correctness]: test 1 asserts history that `delete_pool` removes.** `delete_pool` (`targets.rs:810-898`) removes the pool's placements, intents, placement events and profile links by design, and `fleet_schema.rs:769` pins that.
    - Fix: split test 1.
      - (a) The issue's case: the pool is deleted, and only the grant remains.
      - (b) The history case: the pool drops the connection through the pool update (`resources.rs:1883-1895`), so the history and the enabled profile links survive.
  - **The four decisions the brief left to the architect:**
    1. **The retire nulls `gitlab_connection_id` in the same UPDATE, and the FK stays NO ACTION.** The CHECK accepts a NULL connection only when `retired_at IS NOT NULL`. The trigger allows only the retire transition, so a retired row is final.
    2. **`canonical_key` is unchanged and stays globally UNIQUE.** The key embeds the connection's UUID v4.
    3. **One `fleet.target.retired` fleet event per retired target.** It has a `target_id`, no host, and is written by a direct INSERT in the transaction.
    4. **No `DELETE /api/v1/fleet/targets/{id}`.** A grant on a retired target answers 404 `NotFound`, like a retired host.
  - **The warns:**
    - (3) one core function in `fleet::targets` takes the caller's `&mut SqliteConnection`; the handler switches to `BEGIN IMMEDIATE`;
    - (4) put retired guards at the paths every caller goes through (`list_targets`, `set_host_grant`, `ensure_target_identity_for_pool`, `link_pool_target`, the `save_pool_fleet` upsert, the key path), not into each admission CTE;
    - (5) share one placement-liveness fragment with `LIVE_WORKLOADS`;
    - (6) a retired key resolves to `None` in `gitlab_connection_target`;
    - (7) 0048 appends `retired_at` last, keeps the trigger's name and message, bumps `revision` on revoked grants, and `FLEET_ADDED_COLUMNS` gains the column;
    - (8) the HTTP tests' home is `control_plane_contracts.rs:2345`.
- **Assumed:**
  - With no `survey.md`, the analogues the brief names (`LIVE_WORKLOADS` / `delete_pool`, 0047's rebuild) and the fleet-host retire and delete precedents serve as the Reuse map.
  - QUEUE_TIMEOUT expiry (`reconciler fleet.rs:297-320`) is what clears stale `queued` intents, so refusing on `queued` is transient.
- **Hedged:**
  - The "retired is final" part can be a second trigger, following 0040, or folded into the identity trigger's `WHEN`. Either is consistent, so I left the choice to F.
  - The `fleet.target.retired` event has no reader today: every event query filters by `host_id`. It is a durable record only.
  - The two blocks stop the run for a human amendment. I chose block over warn because both would have T pin tests that can't pass without an unplanned change to `delete_pool`, or would ship the #183 bug class again.
- **Evidence:**
  - **Files read:**
    - the brief and the brief gate result (r1);
    - `migrations/0022_fleet_foundation.sql`, `0040_fleet_host_delete.sql` and `0047_gitlab.sql`;
    - `fleet/targets.rs` (sync, ensure, list, save, liveness, delete, link, `pool_targets`);
    - `fleet/domain.rs` (`canonical_key`);
    - `fleet/service.rs` (`cancel_queued`, operation access, intent policy, target identity, target access);
    - `fleet/admission.rs` (the admit query, `available_session`, diagnose, `admit_operation`, the event insert);
    - `fleet/registry/{retire,delete}.rs` and `registry/mod.rs` (`set_host_grant`, `host_grants`, approve, recovery scope, `ensure_target_admin`, `ensure_scope_admin`, `audit_and_event`);
    - `registry/queries.rs` (`readable_targets` and the event readers);
    - `fleet/failure.rs`, `gitlab/jobs.rs` and `fleet/mod.rs`;
    - in `gridops-api`: `resources/gitlab.rs` (delete, create), `resources.rs` (pool update, pool delete), `webhooks/gitlab.rs` (BEGIN IMMEDIATE into core), `fleet/profiles.rs` and `fleet/error.rs`;
    - reconciler `fleet.rs` (provision, `expire_queued`);
    - `src/features/platform-connections/gitlab.tsx:378`;
    - `docs/openapi/gridops-v1.json` (the DELETE description);
    - `crates/gridops-core/tests/fleet_schema.rs` (the upgrade tests, FK pins, immutability, pool-delete history);
    - `crates/gridops-api/tests/control_plane_contracts.rs:2345`.
  - **Checks run:**
    - **SQLite trigger and FK behaviour, checked in `sqlite3`.** A SET NULL FK action fires the child's BEFORE UPDATE trigger and aborts. The chosen CHECK and trigger keep `FOREIGN KEY constraint failed` for a live target, allow retire then delete, and refuse un-retiring, re-retiring and identity edits.
    - **No code exists yet.** `git diff origin/master -- crates src migrations` is empty.
    - **No live worktree overlaps,** according to `git worktree list` and each worktree's status.

## Phase 3 Architecture (A, up-front plan review, R2) - 2026-11-22T08:49:00-08:00
- **Decided:** PASS, with 0 block and 4 warn findings (`docs/handoffs/183/handoff-A.md`, R2). The brief amended in `aef7a18` fixes both R1 blocks.
  - **R1 block 1 is resolved.** Item 2 refuses on a live placement or an unreleased allocation keyed by `p.target_id`, or on a `queued` intent, and it shares one fragment with `LIVE_WORKLOADS`. An `admitted` intent is explicitly history.
  - **R1 block 2 is resolved.** The test is split into (a), the issue's case where only the grant survives `delete_pool`, and (b), the history case via the pool update.
  - **Warns:**
    1. Build §1 and §2 still carry option and "architect decides" wording. The binding decisions §1 and §3 override it, and T and F follow the decisions.
    2. The session-grant revoke stays in the core function. "No production code writes" means only that nothing inserts them.
    3. The `link_pool_target` and `save_pool_fleet` guards are backstops for GitLab. Add every guard as an inline `retired_at IS NULL` predicate, with no new helper layer.
    4. Share the liveness clause through a `macro_rules!` fragment and `concat!`, as `delete_pool` does, because `LIVE_WORKLOADS` is a `const &str`.
- **Assumed:** with no `survey.md`, the analogues the brief names still serve as the Reuse map. They are `LIVE_WORKLOADS`/`delete_pool`, 0047's rebuild, and the fleet-host retire and delete.
- **Hedged:** warn 1 does not block. The brief labels the decisions section binding, so the leftover build text can't pick the design on its own. It could still mislead a reader, which is why it is a warn.
- **Evidence:**
  - **Re-read at `aef7a18`:**
    - `resources/gitlab.rs:369-450`;
    - `targets.rs:59-230, 551-758, 795-960`;
    - `service.rs:736-830`;
    - `registry/mod.rs:1155-1206`;
    - `registry/delete.rs:150-180`;
    - `registry/queries.rs:820-850`;
    - `failure.rs:200-220`;
    - `webhooks/gitlab.rs:350-365`;
    - `gitlab/jobs.rs:202-206`;
    - `resources.rs:1840-1930`;
    - `0022` (the grant tables, `profile_ci_targets`, `fleet_events`, every FK into `fleet_ci_targets`);
    - `0047` §5;
    - `fleet_schema.rs:240-275, 600-620, 855-880`;
    - `control_plane_contracts.rs:2345-2440`;
    - `fleet/error.rs:78`.
  - **Checks run:**
    - **No code exists yet.** `git diff origin/master -- crates src migrations` is empty.
    - **No overlapping 0048 migration.** No worktree has one, and every other worktree is clean.
    - **Nothing pins the removed refusal message.** "is a CI target of the fleet" appears only in the handler.
    - **Only the admission reader touches session grants.** `host_session_target_grants` appears in production code only at `admission.rs:825`, which reads it.

## Phase 4 Tests RED (T) - 2026-11-22T09:11:00-08:00
- **Decided:** the RED is valid (`docs/handoffs/183/handoff-T-red.md`). 10 failing tests, each failing for the right reason. The diff is test files only: `control_plane_contracts.rs`, `gitlab_pool_fleet.rs`, `fleet/admission_tests.rs` and `fleet_schema.rs`.
  - **HTTP (next to `:2345`, OpenAPI-checked):** (a) the prod case retires the target and revokes the grant; (b) history is kept and the profile link is revoked; the refusals run in order pool → runner → queued intent / running placement / unreleased allocation, with no change after each 409 and a 204 once drained; the existing DELETE test also covers the never-used target that `list_targets` creates.
  - **Core:** the key path, `list_targets`, `pool_targets` and `save_pool_fleet` never revive or offer a retired target. `ensure_target_identity_for_pool` answers `NotFound` for one, and `link_pool_target` never links one.
  - **Schema:** `FLEET_ADDED_COLUMNS` gains `retired_at`; the 0047→latest rebuild keeps rows and FKs; the retire transition is allowed and final; 0048 has an FK guard.
  - **Pinned contract details F must meet:**
    - the audit keys `retiredTargetIds`, `hostGrantsRevoked`, `sessionGrantsRevoked`, `profileLinksRevoked`;
    - `fleet.target.retired` with no host and the deleting user as actor;
    - grant `revision+1` on revoke;
    - a grant on a retired target answers 404 `not_found`;
    - the live-work 409 contains the running workload's pool name;
    - refusal order pool → runner → live work;
    - exactly one `migrations/0048_*.sql` with a `*_fk_guard`.
- **Assumed:**
  - "A message naming what is running" means the operator-facing pool name of the live workload. The test asserts only that the name is present, not the wording.
  - The new live-work refusal comes after today's two refusals, from "Keep today's refusals and their order … Add a refusal".
  - In the HTTP tests, two DB writes stand in for real paths: removing the pool's membership with SQL stands in for the pool update, which does the same replace at `resources.rs:1883-1895`; `delete_pool` is called directly rather than through the pool route.
- **Hedged:**
  - The copy, OpenAPI description, docs and CHANGELOG items get no automated test (wording tests are brittle, and none exist today). S checks them.
  - `link_pool_target` and the GitLab arm of admission already refuse a NULL-connection row. So for those two, the RED comes from the missing schema, and the tests act as regression guards. The `NotFound` assertion is what proves the identity-check guard exists.
  - I did not run a schema-only stub migration in the repo to prove the core guard assertions fail once the schema exists, because that would mean writing production SQL. I checked their logic by reading the code instead, and I replayed the schema test's statements against A's design in `sqlite3`.
- **Evidence:**
  - **Commands:**
    - `cargo test -p gridops-core`: `fleet_schema` 13 pass / 4 fail, `gitlab_pool_fleet` 7/1, lib 310/1, all other binaries pass.
    - `cargo test -p gridops-api`: `control_plane_contracts` 35/4; `fleet_http` 149/1 (`release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`, which fails only on a macOS host because the test agent reports its compile-time OS (`release_preflight.rs:63-65`); not touched by this diff).
    - `cargo fmt --check` is clean.
    - `cargo +1.98.1 clippy --all-targets --locked -D warnings` (the CI flags) is clean for both crates.
  - **Checks run:**
    - A temporary test-only probe, since deleted, confirmed the later-step seeds: the liveness transitions, the event insert, grant route 200 and 404 `{"code":"not_found"}`, the target list, and `delete_pool` from the test process.
    - A `sqlite3` replay of A's CHECK and trigger matched all 10 refusals and 2 successes in the retire test.

## Phase 6 Implement (F) - 2026-11-22T09:28:00-08:00
- **Decided:** implemented against T's RED (`docs/handoffs/183/handoff-F.md`). All 10 RED tests are GREEN.
  - **0048** rebuilds `fleet_ci_targets` in 0047's style and appends `retired_at`. The GitLab CHECK arms accept a NULL connection only when retired. The FK stays NO ACTION. The single identity trigger (same name and message) allows only the retire transition, so a retired row is final.
  - **One core function,** `fleet::targets::retire_gitlab_connection_targets(&mut SqliteConnection, …)`. It refuses on live work (pool names) through `LIVE_CONNECTION_WORK`, which shares the `live_placement!()` fragment with `LIVE_WORKLOADS`. Otherwise it revokes host and session grants (`revision+1`) and profile links, retires each target with one UPDATE, and writes a `fleet.target.retired` event per target.
  - **The handler** uses `BEGIN IMMEDIATE` and keeps the pool refusal, then the runner refusal. It drops the target COUNT refusal and adds the four audit keys.
  - **Guards** are inline `retired_at IS NULL` predicates in `target_id_for_key` (the key path), `list_targets`, the `save_pool_fleet` upsert, `link_pool_target`, `ensure_target_identity_for_pool` and `set_host_grant`. There is no new helper.
  - **Copy:** OpenAPI DELETE text, the `removeConfirm` sentence, the "Removing a connection" docs section, and the CHANGELOG.
- **Assumed:**
  - Revoking every profile link on the target with `revoked_at IS NULL` (including ones only disabled by `save_pool_fleet`) is the intended "revoke".
  - Naming the live work's pools is what an operator needs; T pins only the pool name.
- **Hedged:**
  - The trigger also allows retiring a non-GitLab row. Nothing does it (out of scope), so I didn't add a kind clause the brief didn't ask for.
  - `gitlab_tests.rs:1222` (pre-existing) pins the removed refusal and now fails. It is flagged for T as test-only. I did not edit it.
- **Evidence:**
  - `cargo test -p gridops-core`: all pass (lib 311, `fleet_schema` 17, `gitlab_pool_fleet` 8).
  - `-p gridops-api`: `control_plane_contracts` 39/39. The only failures are the stale unit test above and the known macOS-only `release_preflight`.
  - `-p gridops-reconciler`: 125/125.
  - fmt is clean, and clippy 1.98.1 with the CI flags is clean.
  - `npm run build` (incl. tsc), eslint, vitest 624/624 and the redocly lint (67 warnings, the same as master) all pass.
  - The sweep table of every production `fleet_ci_targets` / `target_id` read is in `handoff-F.md`. Source facts are in `evidence.md`.

## Phase 7 Verify GREEN (T) - 2026-11-22T10:00:00-08:00
- **Decided:** the suite is GREEN with no blocking Tier 2 issue. I repaired two stale tests (test-only):
  - deleted `gitlab_tests.rs` `a_connection_a_fleet_target_references_is_refused_with_409_not_500`, because it is superseded by the HTTP tests;
  - updated `fleet/openapi.rs` `the_gitlab_operations_are_specified_and_all_are_served` to pin the new DELETE 409 description. F did not flag this one.
- **Assumed:** the `release_preflight` failure is the macOS-only one recorded in T-red (not touched by this diff).
- **Hedged:** the frontend checks were not re-run, because only a copy string changed. F's results are accepted.
- **Evidence:**
  - full `cargo test` for core, api and reconciler is green except the one known macOS-only failure;
  - fmt and clippy are clean;
  - a temporary mutation of the `list_targets` and key-path filters failed `a_retired_gitlab_target_is_never_revived_offered_or_relinked`, and production was restored.

## Phase 7 Architecture (A, anti-duplication gate) - 2026-11-22
- **Decided:** PASS, with 0 block and 2 warn findings (`docs/handoffs/183/handoff-A-dup.md`). F extended the named analogues and built no parallel path:
  - the retire function sits in `fleet::targets` beside `delete_pool`;
  - the liveness clause comes from one `live_placement!()` fragment shared with `LIVE_WORKLOADS`;
  - the guards are inline `retired_at IS NULL` predicates;
  - the migration follows 0047;
  - there is no new route and no new helper layer.
- **Assumed:** the stale-test deletion in `gitlab_tests.rs` is intended (T-green). The F handoff prose saying it was not edited is out of date.
- **Hedged:** the trigger permits a retire that keeps the connection id. It is reachable only by ad-hoc SQL and matches decision §1. Tightening it is left as optional follow-up work.
