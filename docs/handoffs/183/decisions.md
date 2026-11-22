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
