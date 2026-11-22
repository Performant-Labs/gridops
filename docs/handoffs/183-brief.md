# [GitLab: a connection used on the fleet can never be removed](https://github.com/Performant-Labs/gridops/issues/183)

Part of epic #136. Found on prod (Mars) on 2026-11-21, while cleaning up after the GitLab production check. The connection "gridops-e2e (prod check)" (`044f513b-…`) can't be removed because of fleet target `b429d80e-d590-4c84-96f7-ff63afafd235`. That target's pool is deleted, and its only host grant (on callisto) has schedule, native and Docker socket all off.

## The problem (as it is on master `5e122c1`)

- `DELETE /api/v1/platform-connections/gitlab/{id}` (`crates/gridops-api/src/resources/gitlab.rs`, `delete` handler ~line 365-450) refuses with 409 while any `fleet_ci_targets` row has `gitlab_connection_id` = the connection: "… is a CI target of the fleet (N target). Remove it from the fleet first." Nothing in the API or UI can remove or retire a fleet target. The fleet router has only `GET /api/v1/fleet/targets`.
- The connection does not even need to have been "used". `list_targets` (`crates/gridops-core/src/fleet/targets.rs:210`) calls `sync_gitlab_targets`, which creates a target for **every** GitLab connection. So once anyone opens the fleet targets list, every GitLab connection becomes undeletable. `pool_targets` / `gitlab_connection_target` also create them on demand.
- Schema facts (migration `0047_gitlab.sql` §5, which rebuilt the table):
  - `fleet_ci_targets.gitlab_connection_id` is `REFERENCES gitlab_connections(id)` with the default NO ACTION, so deleting the connection fails while a target row names it.
  - The CHECK requires `gitlab_connection_id IS NOT NULL` for `gitlab_project` and `gitlab_group`.
  - `canonical_key` is UNIQUE.
  - Trigger `fleet_ci_targets_immutable_identity` aborts **every** UPDATE of the table.
- Rows referencing a target (all from `0022_fleet_foundation.sql`): `host_target_grants` (has `revoked_at`), `host_session_target_grants` (has `revoked_at`), `profile_ci_targets` (has `enabled`, `revoked_at`), `workload_intents`, `workload_placements`, `fleet_events`, `external_runner_observations`.

## What to build

1. **Retire, don't delete.** Give `fleet_ci_targets` a retired state (for example `retired_at INTEGER`) in a new migration, `0048`. Follow 0047's style for a table rebuild (`PRAGMA foreign_keys` off and on around it, re-create the trigger, no `IF NOT EXISTS`). A retired GitLab target must no longer block deleting its connection, and fleet history must keep its references to the target row. That means `workload_intents`, `workload_placements`, `fleet_events` and `external_runner_observations` keep their `target_id` values and the target row stays.
   - The FK and CHECK decision is the architect's. One option: make the GitLab FK `ON DELETE SET NULL`, and let the CHECK accept a NULL connection only on a retired row. The other option: null the connection column when retiring, in the same transaction.
   - The immutable-identity trigger must keep refusing changes to identity columns (`kind`, `canonical_key`, the external ids). It may allow only the retire transition, and must not allow un-retiring.
   - Say in the plan what happens to `canonical_key` uniqueness. A new connection gets a new id, so a new key, but confirm it.
2. **Removing the connection retires its targets** in the same `BEGIN IMMEDIATE` transaction as the delete:
   - Keep today's refusals and their order. Refuse while pools still use the connection, and while live runners registered through it exist (`runners.deleted_at IS NULL`).
   - Add a refusal while any workload on one of its targets may still be on a machine. A target counts as live when one of its placements (`p.target_id`) is not in `stopped`/`cleaned`/`failed` or still holds an unreleased capacity allocation, or when an intent on it has `status='queued'`. Share one placement-liveness SQL fragment with `LIVE_WORKLOADS` in `targets.rs`, so the list of terminal states lives in one place.
   - An **admitted** intent is history, not live work. Its placement check covers it, so it must not block. The 409 names what is still running, so an operator knows to stop or drain it first. Nothing is cancelled silently.
   - Otherwise, retire every target of the connection. Revoke its `host_target_grants`, `host_session_target_grants` and `profile_ci_targets` rows (set `revoked_at`, and `enabled=0` where it applies), then delete the connection and its secrets as today.
   - Write the audit event `gitlab.connection_deleted` as today, with the retired target ids and the counts of grants revoked.
   - Emit one `fleet_events` row per retired target, if that event kind fits the existing event conventions. The architect decides.
3. **Retired targets are invisible to new work.** Every query that offers, grants, admits or places against a target must ignore retired targets. That covers `list_targets` and `GET /api/v1/fleet/targets`, grant writes, admission, placement, `link_pool_target`, `pool_targets`, `save_pool_fleet`, and the reconciler's fleet pass. Guard them at the choke points listed under Architecture decisions §6, not in each query.
   - `ensure_target` / `gitlab_connection_target` / `sync_gitlab_targets` must never revive a retired row. The connection is gone, so there's nothing to sync, but guard the key path.
   - Granting a host on a retired target must answer a clear 4xx error, not succeed.
   - **Sweep the class:** grep every `fleet_ci_targets` and `target_id` read in `crates/` (outside tests). In the result handoff, list each one with its verdict (filters retired / history read, keep / not applicable).
4. **No separate target route.** Removing the connection is the operator's way to retire its targets (decision §4).
5. **API and UI say what happens.**
   - Update the OpenAPI description of the GitLab connection DELETE: it retires the connection's fleet targets and revokes their machine grants, and fleet history is kept.
   - Update the delete confirmation copy in `src/features/platform-connections/` to say the same in one sentence. This is a copy change only, with no new UI surface.
   - Update `docs/gitlab-runners.md` with a short "Removing a connection" section: what blocks removal, and the order to follow (delete or empty the pool, let its runners leave service, then remove the connection).
   - Add a CHANGELOG entry.

## Architecture decisions (from plan review R1, binding)

The full detail is in `docs/handoffs/183/handoff-A.md` and `docs/handoffs/183/decisions.md`.

1. **Retiring a target.**
   - The retire is a single UPDATE. It sets `retired_at` and nulls `gitlab_connection_id`. The FK stays NO ACTION, and `ON DELETE SET NULL` is rejected.
   - The CHECK lets a GitLab row have a NULL connection only when `retired_at IS NOT NULL`.
   - The trigger refuses every UPDATE except that one retire transition, so a retired row never changes again. The trigger keeps its name and message.
   - `retired_at` is appended as the last column, and `FLEET_ADDED_COLUMNS` gains it.
2. `canonical_key` stays unchanged and globally UNIQUE. A GitLab key embeds the connection's UUID v4, so a re-created connection gets a new key.
3. Write one `fleet_events` row per retired target. Its kind is `fleet.target.retired`, `target_id` is set and there is no host. Insert it in the same transaction.
4. There is no `DELETE /api/v1/fleet/targets/{id}` route. A grant on a retired target answers 404 NotFound, the same as a retired host.
5. **One core function.**
   - Put it in `fleet::targets`, taking the caller's `&mut SqliteConnection` (the pattern is `webhooks/gitlab.rs:358` → `gitlab::jobs::record_gitlab_job`). It does the liveness refusal, the retire, the revokes (bump `revision` on revoked host and session grants, as `registry/delete.rs:165-171` does) and the events.
   - The handler switches to `begin_with("BEGIN IMMEDIATE")` and drops its `fleet_ci_targets` COUNT refusal. It gains no new fleet SQL.
6. **Retired-target guards go at the choke points:**
   - `list_targets`
   - `set_host_grant`'s existence check
   - `ensure_target_identity_for_pool`, which covers submit, the admission re-checks and operation access
   - `link_pool_target`
   - the key path: `gitlab_connection_target` returns `Ok(None)` when its key resolves to a retired row, and never inserts
   - the `save_pool_fleet` profile-link upsert (`targets.rs:701-709`)

   No production code writes `host_session_target_grants`. `authorization.rs` has no SQL. Enrollment has no GitLab arm.
7. **Where the tests go.** HTTP DELETE tests go next to `crates/gridops-api/tests/control_plane_contracts.rs:2345`, which validates responses against the OpenAPI schema.

## Tests (required)

- Core (`crates/gridops-core/tests/gitlab_pool_fleet.rs` or a new file), and/or api `fleet_http` / gitlab resource tests:
  - **(a) The issue's case (prod's shape):** a GitLab pool on fleet machines, with a host grant on the connection's target. Delete the pool, then delete the connection. `delete_pool` already removes the pool's own placements, intents and profile links by design, and that stays unchanged, so only the grant remains. Expect all of this:
    - 204
    - the target row still exists, retired, with a NULL connection
    - the grant is revoked, and its revision is bumped
    - `GET /api/v1/fleet/targets` no longer lists the target
    - one `fleet.target.retired` event
  - **(b) The history case:** a GitLab pool has finished placements on the target. Remove the connection from the pool through the pool update, which leaves `profile_ci_targets` enabled. Then delete the connection. Expect all of this:
    - 204
    - the old placements, intents and events still reference the target
    - the profile link and the grant are revoked
  - A connection whose target was created only by `list_targets` (never used) can be deleted.
  - Deleting a connection that still has a live placement or queued intent on its target is refused with 409, a message naming what is running, and no change.
  - The pool and live-runner refusals are unchanged.
  - A retired target is not offered for a grant, refuses a grant write, and is never admitted or placed.
  - The migration test: the `fleet_schema.rs` style rebuild keeps existing rows and FKs, and the trigger still refuses identity updates and un-retiring.
- Keep every existing test passing. Constraints for local runs: `CARGO_BUILD_JOBS=2`, `RUST_TEST_THREADS=2`, the shared target dir from the untracked `.cargo/config.toml` (never set `CARGO_TARGET_DIR`), and `-p <crate>` runs.

## Out of scope

- Deploying to Mars, or touching prod data. The prod connection is removed by an operator after deploy.
- #182 (the v0.2.0 release).
- Retiring GitHub or Bitbucket targets.
- Retention or GC of retired targets.
