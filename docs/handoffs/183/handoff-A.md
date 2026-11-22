# Handoff-A: Phase 3 - Retire a GitLab connection's fleet targets on removal (#183)  (up-front plan review)

**Date:** 2026-11-22
**Branch:** claude/gitlab-183-retire-target (head 44624e2; `git diff origin/master -- crates src migrations` is empty, so no code exists yet)
**Brief reviewed:** docs/handoffs/183-brief.md   **Reuse map:** there is no `docs/handoffs/183/survey.md`. The brief names its own analogues: `LIVE_WORKLOADS` / `delete_pool` in `fleet/targets.rs`, and 0047's table-rebuild style. I also used the soft-retire and soft-delete patterns for fleet hosts (`registry/retire.rs`, `registry/delete.rs`, `migrations/0040_fleet_host_delete.sql`). Recommendation: EXTEND.   **Wireframe:** N/A (a copy change only)
**Verdict:** BLOCK

## Summary
BLOCK, with 2 block findings and 6 warns. The plan's overall shape is right and matches the codebase: a retired state on the target row, revoking grants, history rows that stay, and refusals checked inside the delete's own transaction. Fleet hosts already work this way.

Two parts of the brief can't both be true given how the code works today:
- **Block 1.** Item 2 refuses removal while an intent on the target is `admitted`. An intent never leaves `admitted`, even after its runner finishes. So any connection whose target still has fleet history could never be removed, which is the same bug as #183.
- **Block 2.** Test 1 expects that pool's history to survive "delete the pool". `delete_pool` deletes exactly that history, by design.

The brief left four decisions to the architect. They are settled below, so T and F can pin them.

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | block | Item 2: "Also refuse on queued or admitted intents on those targets" | pattern consistency (liveness predicate) | **An `admitted` intent is permanent history, not live work.**<br>- An intent's status changes in only two places. `queued`→`admitted` happens at `admission.rs:1195`, in the same transaction that inserts the placement (`:1096`) and the allocation (`:1128`). `queued`→`cancelled` happens at `service.rs:138`.<br>- Nothing moves an intent out of `admitted` once its placement is `stopped`, `cleaned` or `failed`. Only `delete_pool` deletes intents (`targets.rs:877`).<br>- Every existing "is work still on a machine" check reads the placement and its allocation, never the intent status. That covers `LIVE_WORKLOADS` (`targets.rs:797-801`, which the brief itself names as the shape), the host delete (`registry/delete.rs:83-98`) and the host retire (`registry/retire.rs:136-150`).<br>- As written, a connection whose target was used by a pool that later dropped it can never be removed. That repeats #183, and it contradicts item 1 ("history keeps its references") and test 1 (204 while old intents still reference the target). | Refuse while:<br>(a) a placement on the connection's targets is not in `stopped`/`cleaned`/`failed`, or still has an unreleased allocation. This is `LIVE_WORKLOADS` keyed by `p.target_id` instead of `i.pool_id`;<br>(b) an intent on those targets is `status='queued'`, meaning not yet placed. The reconciler cancels these after `QUEUE_TIMEOUT` (`gridops-reconciler/src/fleet.rs:297-320`).<br><br>An admitted intent is live exactly when its placement is, and (a) already covers that. |
| 2 | block | Tests, first bullet: "Delete the pool, then delete the connection … its grants and profile links are revoked … the old placements, intents and events still reference it" | pattern consistency (pool delete contract) | **This test can't pass against the current `delete_pool`, and it mustn't be made to.**<br>- The pool DELETE route (`resources.rs:2874`) calls `delete_pool` (`targets.rs:810-898`). By design, that deletes the pool's placements, intents, the events tied to those placements, allocations, commands and `profile_ci_targets` rows. That behaviour is pinned by `fleet_schema.rs:769` (`deleting_a_pool_takes_its_finished_fleet_history_with_it`).<br>- After the pool is deleted, only the host grant (and any events with no placement) still reference the target. So "profile links are revoked" and "old placements, intents and events still reference it" can't hold for that pool's rows.<br>- Making them hold would mean changing `delete_pool`, an unplanned edit to shared code. It also can't be done: `workload_intents.pool_id` is a NO ACTION FK to `runner_pools`. | Split the test in two.<br>**(a) The issue's case.** The pool is deleted and only the host grant remains. Deleting the connection returns 204. Assert:<br>- the target row exists and is retired, with a NULL `gitlab_connection_id` (decision 1);<br>- the grant is revoked (`revoked_at` set, `revision` bumped);<br>- `GET /api/v1/fleet/targets` doesn't list it;<br>- a `fleet.target.retired` event exists (decision 3).<br>**(b) The history case.** A pool's fleet workload on the target has finished: the placement is terminal, the allocation is released, and the intent stays `admitted`. The pool then drops the connection through the pool update, which replaces memberships and leaves `profile_ci_targets` enabled (`resources.rs:1883-1895`). Deleting the connection then returns 204. Assert:<br>- the old placement, intent and events still reference the target;<br>- the profile link is revoked (`enabled=0`, `revoked_at` set);<br>- the grant is revoked. |
| 3 | warn | Item 2 doesn't say which layer runs the retire. Today the handler holds the fleet SQL (`gitlab.rs:403-414`) and opens a deferred transaction (`gitlab.rs:378`) | layering; cross-cutting (transactions) | **Fleet-table writes belong in `gridops_core::fleet`, not in the API resource.** The pattern already exists: an API handler opens `begin_with("BEGIN IMMEDIATE")` and passes its connection into a core function. See `webhooks/gitlab.rs:358-359` calling `gitlab::jobs::record_gitlab_job(conn: &mut SqliteConnection, …)` (`gitlab/jobs.rs:202-206`). | - Add **one** core function in `fleet::targets`, beside `LIVE_WORKLOADS` and `delete_pool`. It takes the caller's `&mut SqliteConnection` and the connection id. Inside it:<br>&nbsp;&nbsp;1. read the blockers;<br>&nbsp;&nbsp;2. retire the targets and null their connection;<br>&nbsp;&nbsp;3. revoke grants, session grants and profile links;<br>&nbsp;&nbsp;4. insert the fleet events.<br>- It returns a typed result: the retired ids and counts, or the blockers with pool and host names.<br>- The handler:<br>&nbsp;&nbsp;- switches to `begin_with("BEGIN IMMEDIATE")`;<br>&nbsp;&nbsp;- keeps the pool and runner refusals in their current order;<br>&nbsp;&nbsp;- drops its own `fleet_ci_targets` COUNT refusal;<br>&nbsp;&nbsp;- calls the core function;<br>&nbsp;&nbsp;- formats the 409 the way `gitlab.rs` formats its other refusals;<br>&nbsp;&nbsp;- writes `gitlab.connection_deleted` after commit as today, adding the metadata keys `retiredTargetIds`, `hostGrantsRevoked`, `sessionGrantsRevoked` and `profileLinksRevoked`.<br>- Add no new fleet SQL to `gitlab.rs`. |
| 4 | warn | Item 3: "Every query that offers, grants, admits or places against a target must ignore retired targets", plus the named path list | abstraction level; pattern consistency | **Put the retired check at the few places every path goes through, not into each of about 20 queries.** Two facts make this possible:<br>- Admission already requires an unrevoked grant and an enabled, unrevoked profile link (`admission.rs:622, 642-646`). It also re-runs `service::recheck_intent_authorization` (`admission.rs:233`), which reaches `ensure_target_identity_for_pool` (`service.rs:736`). That one function also guards submit (`:733`) and operation access (`:392`).<br>- Only GitLab targets get retired. `ensure_target_admin` (`registry/mod.rs:2065-2145`) has no GitLab arm, so a GitLab target can never be in an enrollment scope.<br><br>**Not in the brief's list:** the `profile_ci_targets` upsert in `save_pool_fleet` (`targets.rs:701-709`) sets `enabled=1,revoked_at=NULL` again, the same way `link_pool_target` does.<br><br>**Two named paths don't exist:**<br>- No production code writes `host_session_target_grants`. Only `admission.rs:825` reads it, and it already checks `st.revoked_at IS NULL`. `handshake.rs` contains no target code at all.<br>- `authorization.rs` has no SQL. | Add `retired_at IS NULL` in these places:<br>- `list_targets` (`targets.rs:213-218`);<br>- `set_host_grant`'s existence check (`registry/mod.rs:1171-1173`). That returns `NotFound` → 404, exactly as a retired host in the same statement does;<br>- `ensure_target_identity_for_pool`;<br>- `link_pool_target`;<br>- the `save_pool_fleet` upsert;<br>- the key path (warn 6).<br><br>Leave the admission and diagnosis CTEs unedited. In the sweep table, mark them "covered by revocation + `ensure_target_identity_for_pool`". Mark these "N/A, no GitLab path":<br>- enrollment issue, approve and recovery;<br>- the handshake;<br>- `authorization.rs`.<br><br>Mark these "history read, keep": the `readable_targets` CTE (`queries.rs:824-850`), the host and event queries (`queries.rs:885-954, 1638`), `placement.rs:48` and `host_grants` (which already filters on `g.revoked_at IS NULL`). |
| 5 | warn | Item 2: "the same shape as `LIVE_WORKLOADS`" | abstraction level (duplication) | **A second, target-keyed copy of the predicate would mean two lists of terminal states to keep in sync.** `delete_pool` already builds SQL from shared `macro_rules!` fragments (`targets.rs:823-833`). | Factor the placement-liveness clause into one fragment: `p.state NOT IN ('stopped','cleaned','failed') OR EXISTS (unreleased allocation)`. Use it in both the pool-keyed `LIVE_WORKLOADS` and the new target-keyed check, so the terminal-state list exists once. `LIVE_WORKLOADS`'s behaviour must not change. |
| 6 | warn | Item 3: "`ensure_target` / `gitlab_connection_target` / `sync_gitlab_targets` must never revive a retired row … guard the key path" | pattern consistency | **The only revive path is the key lookup.** `ensure_gitlab_target` runs `INSERT … ON CONFLICT(canonical_key) DO NOTHING` and then `target_id_for_key` (`targets.rs:185-206`). So if an existing key belongs to a retired row, the code returns that retired id.<br><br>That can only happen in a race. The connection id is a UUID v4 (`gitlab.rs:143`) and is built into the key (`domain.rs:248-259`), and no route retires a target without deleting its connection (decision 4). | - Resolve keys only to unretired rows.<br>- When a GitLab key resolves to a retired row, `gitlab_connection_target` returns `Ok(None)`, which is its documented "connection does not exist" contract (`targets.rs:72-77`). It never inserts.<br>- `sync_gitlab_targets`, `pool_targets` and the reconciler's `gitlab_lifecycle::fleet_target` (`gitlab_lifecycle.rs:549`) then skip the target with no further change.<br>- The GitHub `ensure_target` path is unchanged. |
| 7 | warn | Item 1: migration 0048 "in 0047's style" | naming and file structure; pattern consistency | **0048 has to fit the tests that already pin 0047's output.** The `fleet_schema.rs` 0046→latest upgrade test runs 0048 too. It checks:<br>- that columns are only appended (`assert_added_columns`, through `FLEET_ADDED_COLUMNS` at `:253-270`);<br>- that the trigger keeps its name (`:876-878`);<br>- that `fleet_ci_targets` FKs are NO ACTION (`:898-906`). | - Copy every 0047 column in order and **append `retired_at INTEGER` last**. Keep the table STRICT.<br>- Keep the trigger name `fleet_ci_targets_immutable_identity` and the message `'CI target identity is immutable'`.<br>- Give the FK guard table its own name, `_fleet_0048_fk_guard`.<br>- When the retire revokes rows, bump `revision` on `host_target_grants` and `host_session_target_grants`, as the host delete does (`registry/delete.rs:165-171`).<br>- `FLEET_ADDED_COLUMNS` gains `("fleet_ci_targets", retired_at, INTEGER, None)`. That is expected upkeep of an existing test constant, not a change in behaviour.<br>- The new migration test should mirror `gitlab_upgrade_from_0046_keeps_the_fleet_graph_and_every_reference` (`:826`), using `run_to(47)` and the same seven `TARGET_REFERRERS`. |
| 8 | warn | Tests: "api `fleet_http` / gitlab resource tests" (brief gate NIT-1) | naming and file structure (test homes) | **The GitLab connection DELETE HTTP contract isn't in `fleet_http`.** It's at `crates/gridops-api/tests/control_plane_contracts.rs:2345` (`…delete_answers_404_and_409`). That test checks every response against `docs/openapi/gridops-v1.json` through `openapi_schema::check_response`. | Put the HTTP 204/409 cases beside that test. Put the target and grant state assertions in `crates/gridops-core/tests/gitlab_pool_fleet.rs`, and the migration test in `fleet_schema.rs`. The OpenAPI edit only changes the DELETE `description`. The 204/404/409 response schemas stay as they are. |

## Architect decisions (the brief left these to the architect; they bind T and F)

1. **FK and CHECK: null the connection id when retiring. The FK stays NO ACTION.**
   - **What 0048 contains.**
     - The two GitLab arms of the CHECK change from `gitlab_connection_id IS NOT NULL` to `(gitlab_connection_id IS NOT NULL OR retired_at IS NOT NULL)`. The project and group id clauses are unchanged.
     - The trigger aborts every UPDATE except one retire transition:
       - `OLD.retired_at IS NULL AND NEW.retired_at IS NOT NULL`;
       - every other column `IS OLD`;
       - except `gitlab_connection_id`, which may stay the same or become NULL.

       So nothing can change a retired row again, which also refuses un-retiring.
     - Optionally, split the "retired is final" part into a second trigger, `fleet_ci_targets_retired_is_final`, following `fleet_hosts_deleted_is_final` in 0040.
   - **What the retire does.** It sets `retired_at` and `gitlab_connection_id=NULL` in one UPDATE, before the `DELETE FROM gitlab_connections`, in the same transaction.
   - **I checked this in sqlite3.** All of the following hold:
     - deleting a connection that still has an unretired target fails with `FOREIGN KEY constraint failed`;
     - retire, then delete, succeeds;
     - un-retiring, re-retiring, or changing an identity column during the retire aborts with the identity message;
     - a live GitLab row with no connection fails the CHECK.
   - **Why not `ON DELETE SET NULL`.**
     - (i) It breaks two pinned tests:
       - `a_gitlab_connection_a_fleet_target_references_cannot_be_deleted` (`fleet_schema.rs:1176-1222`) expects `FOREIGN KEY constraint failed`;
       - the upgrade test's NO ACTION assertion (`:898-906`).
     - (ii) SQLite runs the SET NULL action as an UPDATE of the child row, and that fires `BEFORE UPDATE` triggers. I confirmed this in sqlite3: the action aborted on an immutable trigger. So the trigger would need a second allowed transition.
     - (iii) The 0022 header says "Ownership history uses restrictive FKs; retirement is a state, not deletion", and 0047 §5 chose NO ACTION for this FK on purpose.
   - **What this keeps.** The FK stays the backstop: a connection can't be deleted while any of its targets is unretired.
2. **`canonical_key`: unchanged, and stays globally UNIQUE (not a partial index).** Retiring keeps the key. A GitLab key embeds the connection id (`domain.rs:248-259`), and connection ids are `Uuid::new_v4()` (`gitlab.rs:143`). A connection re-created for the same project or group therefore gets a new key, with no collision.
3. **Fleet events: yes, one row per retired target.**
   - Kind `fleet.target.retired`, named like `fleet.host.retired`.
   - `target_id` is set, `host_id` and `placement_id` are NULL, and `actor_user_id` is the deleting user.
   - `detail_json` carries the connection id and the per-target revoke counts.
   - Host-less fleet events already exist (`fleet.enrollment.issued` and `fleet.enrollment.revoked`, `registry/mod.rs:376-379, 429-432`), and the schema indexes `(target_id,cursor)`.
   - Write the row with a direct INSERT inside the retire transaction, as `failure.rs:204-216` does. Don't use registry's private `audit_and_event`, which would also write a second audit row next to the API's `gitlab.connection_deleted`.
   - No reader shows target-only events today (the event queries filter by `host_id`), so this is a durable record only.
4. **No `DELETE /api/v1/fleet/targets/{id}`.**
   - The connection delete covers the issue's need.
   - Retiring while the connection lives would leave that connection's `canonical_key` owned by a retired row. `ensure_target` would then resolve to the retired id forever. Avoiding that would need a partial unique index or a revive path, and the brief forbids reviving.
   - Granting a host on a retired target answers 404 `NotFound`, the same way a retired host already does in the same check (warn 4).

**Checked and consistent (no finding):**
- **The retired-state pattern** matches the fleet-host precedent:
  - `fleet_hosts.retired_at` in 0022;
  - the soft delete in 0040, which keeps history and refuses revival with a trigger;
  - grant revocation in `registry/delete.rs:165-171`.
- **The refusal order** (pools, then live runners, then the new live-fleet-work refusal) keeps the existing two refusals and their messages. No existing test pins the removed "is a CI target of the fleet" message. I grepped `crates/` and `src/`.
- **Coexistence.** No live worktree edits these files.
  - `cool-davinci-1830ec` (#143) has a clean tree.
  - `gridops-134` has a clean tree and is stale.
  - No other branch adds a migration 0048.

## Notes for O
Amend `docs/handoffs/183-brief.md`, then start a **fresh** run. Don't use `resumeFromRunId`.

1. **Item 2, second sub-bullet.** Replace "Also refuse on queued or admitted intents on those targets" with: "Also refuse while an intent on those targets is still `queued` (not yet placed). An `admitted` intent never leaves that state; its liveness is its placement's, which the placement/allocation check above covers."
2. **Tests, first bullet.** Replace it with the two cases in finding 2:
   - (a) The pool is deleted, then the connection is deleted, and the response is 204. Assert the retired row, the revoked grant, that the target is hidden from the list, and the `fleet.target.retired` event. Don't assert that pool's placements, intents or profile links: `delete_pool` removes them by design.
   - (b) A pool's finished fleet workload is on the target. The pool drops the connection through the pool update, the connection is deleted, and the response is 204. Assert the history rows still reference the target, and that the profile link and the grant are revoked.
3. **Recommended: copy the four architect decisions above into the brief**, so the brief alone pins them:
   - null the connection id on retire, and keep the FK NO ACTION;
   - keep the key;
   - add the `fleet.target.retired` event;
   - add no fleet-target DELETE route, and refuse a grant on a retired target with 404.

   Also name the `save_pool_fleet` profile-link upsert in item 3's path list.

Warns 3-8 are implementation guidance and need no change to the brief. A checks them at the Phase 7 anti-duplication gate.

## Patterns referenced
- `crates/gridops-core/src/fleet/targets.rs`:
  - `:59-207`: `sync_gitlab_targets`, `gitlab_connection_target`, `ensure_target`, `ensure_gitlab_target`, `target_id_for_key`;
  - `:210-230`: `list_targets`;
  - `:551-758`: `save_pool_fleet`;
  - `:795-934`: `LIVE_WORKLOADS`, `delete_pool`, `link_pool_target`.
- The fleet-host precedents:
  - `crates/gridops-core/src/fleet/registry/retire.rs`, `registry/delete.rs` and `migrations/0040_fleet_host_delete.sql`: a retired or soft-deleted state, named blockers, grant revocation with `revision+1`, and revival refused by a trigger;
  - `registry/mod.rs:1155-1206` (`set_host_grant`) and `:2065-2160` (`ensure_target_admin`, `ensure_scope_admin`).
- `migrations/0022_fleet_foundation.sql` (the `fleet_ci_targets` referrers and the intent and placement state machines) and `migrations/0047_gitlab.sql` (the rebuild style and the FK guard).
- `crates/gridops-core/src/fleet/admission.rs:233, 610-720, 815-830, 1096-1223` and `fleet/service.rs:128-150, 364-470, 730-830`: the intent lifecycle and the policy choke points.
- `crates/gridops-api/src/resources/gitlab.rs:369-453` (the current delete), `crates/gridops-api/src/webhooks/gitlab.rs:357-359` (BEGIN IMMEDIATE passed into core), and `crates/gridops-core/tests/fleet_schema.rs:253-270, 769, 816-947, 1176-1222` (the pinned migration and FK contracts).
