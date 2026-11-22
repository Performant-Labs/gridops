# Handoff-A: Phase 3 - Retire a GitLab connection's fleet targets on removal (#183)  (up-front plan review, R2)

**Date:** 2026-11-22
**Branch:** claude/gitlab-183-retire-target (head aef7a18; `git diff origin/master -- crates src migrations` is empty, so no code exists yet)
**Brief reviewed:** docs/handoffs/183-brief.md (amended in aef7a18)   **Reuse map:** there is no `docs/handoffs/183/survey.md`. The brief names its own analogues: `LIVE_WORKLOADS` / `delete_pool` in `fleet/targets.rs`, 0047's table-rebuild style, and the fleet-host retire and delete precedents (`registry/retire.rs`, `registry/delete.rs`, `migrations/0040_fleet_host_delete.sql`). Recommendation: EXTEND.   **Wireframe:** N/A (a copy change only)
**Verdict:** PASS

## Summary
PASS, with 0 block findings and 4 warns. Both R1 blocks are fixed in the amended brief:
- **R1 block 1 (admitted intents) is resolved.** Item 2 now refuses only on a live placement or unreleased allocation keyed by `p.target_id`, or on a `queued` intent. It says outright that an `admitted` intent is history, and it shares one liveness fragment with `LIVE_WORKLOADS`.
- **R1 block 2 (history after `delete_pool`) is resolved.** The test is split. (a) deletes the pool and asserts only the surviving grant. (b) drops the connection through the pool update and asserts the history.

The four architect decisions and the R1 warns are now binding in the brief ("Architecture decisions" §1-7). I re-checked each one against the source at `aef7a18`, and all of them still hold.

The warns are small. Two are leftover wording in the build section that the binding decisions override. The other two are guidance for F at the choke points and the shared SQL fragment.

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | Build §1 ("The FK and CHECK decision is the architect's. One option: make the GitLab FK `ON DELETE SET NULL` …") and build §2 ("Emit one `fleet_events` row … if that event kind fits … The architect decides.") | pattern consistency (one source of truth) | **The build section still offers options that the binding decisions have settled.** Decision §1 rejects `ON DELETE SET NULL` and keeps the FK NO ACTION. Decision §3 settles the event. The brief gate flagged the same thing (W-1, NIT-1). A T or F who reads only build §1 could pick the rejected option. That option breaks `a_gitlab_connection_a_fleet_target_references_cannot_be_deleted` (`fleet_schema.rs:1176`) and the NO ACTION pin in the upgrade test (`fleet_schema.rs:894-895`). It also needs a second trigger exception, because a SET NULL action fires `BEFORE UPDATE`. | No brief amendment is needed for this run, because the decisions section says it is binding. T and F: where the build section and "Architecture decisions" differ, follow the decisions. I check this at Phase 7: the FK stays NO ACTION, and the retire is one UPDATE that sets `retired_at` and nulls `gitlab_connection_id`. |
| 2 | warn | Decision §6: "No production code writes `host_session_target_grants`", next to §2 and §5, which revoke those rows | cross-cutting (revocation completeness) | **The sentence means that nothing inserts session grants, so no grant-write choke point is needed for them. It does not mean that the revoke can be skipped.** The only reader is `admission.rs:825`, and it already filters `st.revoked_at IS NULL`. The table has both `revision` and `revoked_at` (`0022_fleet_foundation.sql:140-146`), so the §5 revoke with a `revision+1` bump is valid SQL. In practice it touches 0 rows today. | F: keep the `host_session_target_grants` revoke UPDATE in the core function, shaped like the `host_target_grants` one (`SET revoked_at=?,revision=revision+1 WHERE target_id IN (…) AND revoked_at IS NULL`). Report its count as `sessionGrantsRevoked` in the audit metadata. Don't add a session-grant guard anywhere else. |
| 3 | warn | Decision §6: the guards at `link_pool_target` and at the `save_pool_fleet` profile-link upsert | abstraction level | **For GitLab, both guards are backstops, not the real gate.**<br>- `link_pool_target`'s GitLab arm already needs `gc.id=t.gitlab_connection_id` (`targets.rs:920-926`), and a retired row has a NULL connection.<br>- `save_pool_fleet` gets its targets from `pool_targets` → `gitlab_connection_target` (`targets.rs:613-614, 952-956`), which is the key-path guard.<br>The real gates are the key path, `list_targets`, `set_host_grant`'s existence check (`registry/mod.rs:1170-1178`) and `ensure_target_identity_for_pool` (`service.rs:736`).<br>The risk is over-abstraction: a new `is_retired()` helper or a retired-target layer called from each of these paths. That would be a level of indirection that no neighbouring check uses. Each neighbouring check is a single inline predicate, for example `lifecycle_state!='retired' AND deleted_at IS NULL` in the same `set_host_grant` statement. | Add each guard as one inline `retired_at IS NULL` predicate inside the existing SQL: `list_targets` WHERE, the `set_host_grant` EXISTS, the `ensure_target_identity_for_pool` kind lookup (`SELECT kind … WHERE id=? AND retired_at IS NULL` gives `NotFound`), the `link_pool_target` JOIN, the `save_pool_fleet` upsert, and `target_id_for_key` or its GitLab caller. Add no new helper function or module. |
| 4 | warn | Build §2 / decision §5: "Share one placement-liveness SQL fragment with `LIVE_WORKLOADS`" | pattern consistency (SQL composition) | **`LIVE_WORKLOADS` is a `const &str` (`targets.rs:795-801`, const at `:797`), and `concat!` accepts only literals or macros.** So the fragment can't come out of the const as-is. `delete_pool` already shows how this codebase composes static SQL: module-local `macro_rules!` fragments joined with `concat!` (`targets.rs:823-833`). | Move the clause `p.state NOT IN ('stopped','cleaned','failed') OR EXISTS (SELECT 1 FROM capacity_allocations a WHERE a.placement_id=p.id AND a.state<>'released')` into one `macro_rules!` fragment. Build both `LIVE_WORKLOADS` (pool-keyed, behaviour unchanged) and the target-keyed liveness query from it with `concat!`, so both stay `&'static str`. Don't build either one with a runtime `format!`. |

No block findings. The plan is consistent with existing patterns.

## Re-verified at aef7a18 (no finding)

- **The handler** (`resources/gitlab.rs:369-450`): it uses a deferred `begin()` today (`:378`). The refusal order is pools (`:383-401`), then the `fleet_ci_targets` COUNT (`:402-413`, to be removed), then live runners (`:414-427`). Only that handler contains the string "is a CI target of the fleet". No test pins it.
- **The core-function pattern:** `webhooks/gitlab.rs:358-359` opens `begin_with("BEGIN IMMEDIATE")` and passes `&mut transaction` into `record_gitlab_job(conn: &mut SqliteConnection, …)` (`gitlab/jobs.rs:202-206`).
- **The revoke pattern:** `registry/delete.rs:165-171` runs `UPDATE host_target_grants SET revoked_at=?,revision=revision+1 … AND revoked_at IS NULL`.
- **Test (b) holds.** The pool update replaces `runner_pool_gitlab_connections` (`resources.rs:1883-1895`) without touching `profile_ci_targets`, so the links stay enabled.
- **Test (a) holds.** `delete_pool` (`targets.rs:810-898`) deletes the pool's placements, intents, placement events, allocations and `profile_ci_targets`, and leaves `host_target_grants` alone.
- **`fleet_events` allows a host-less target event.** `host_id` and `placement_id` are nullable, and the composite FK `(placement_id,host_id,target_id)` is skipped when `placement_id` is NULL (`0022:390-402`). `fleet.target.retired` follows the `fleet.<noun>.<verb>` naming of `fleet.host.retired` and `fleet.host.deleted`. The direct-INSERT shape is at `failure.rs:204-216`.
- **Nulling the column is safe for every referrer.** All 7 FKs into `fleet_ci_targets` reference `(id)` only (`0022:101,142,175,219,246,395,407`). No index on `fleet_ci_targets` needs re-creating in 0048 beyond the UNIQUE `canonical_key`. No production code UPDATEs `fleet_ci_targets`, because the trigger refuses it.
- **The 0048 test fit.** `FLEET_ADDED_COLUMNS` (`fleet_schema.rs:253-270`) is where `retired_at` gets appended. The 0046→latest upgrade test exempts `fleet_ci_targets` and its trigger from the definition-equality check (`:865`), and it asserts that the trigger name exists (`:876`).
- **`readable_targets`** (`registry/queries.rs:823-850`) has no GitLab arm, so GitLab targets are admin-only (`?=1`). Retired history stays readable to admins with no change.
- **The 404 mapping.** `RegistryError::NotFound` maps to `ErrorCode::NotFound` (`gridops-api/src/fleet/error.rs:78`), so decision §4's 404 holds.
- **The HTTP test home.** `control_plane_contracts.rs:2345` (`gitlab_connection_list_shows_seeded_connections_and_delete_answers_404_and_409`) checks each response with `openapi_schema::check_response`.
- **Coexistence.** No other worktree (`gridops-134`, `cool-davinci-1830ec`, the main checkout) adds a migration 0048 or has uncommitted changes.

## R1 architect decisions, kept here because the brief points to this file for the detail

The full R1 text is `git show 4c833f4:docs/handoffs/183/handoff-A.md`.

1. **The 0048 CHECK and trigger.** The two GitLab arms change `gitlab_connection_id IS NOT NULL` to `(gitlab_connection_id IS NOT NULL OR retired_at IS NOT NULL)`. The project and group clauses are unchanged. The trigger `fleet_ci_targets_immutable_identity` (same name, same message `'CI target identity is immutable'`) aborts every UPDATE except one transition:
   - `OLD.retired_at IS NULL AND NEW.retired_at IS NOT NULL`;
   - every other column `IS OLD`;
   - except `gitlab_connection_id`, which may stay the same or become NULL.

   So a retired row is final, and un-retiring and re-retiring both abort. As an option, the "retired is final" part can be a second trigger, `fleet_ci_targets_retired_is_final`, following 0040's `fleet_hosts_deleted_is_final`. Append `retired_at INTEGER` last, keep the table STRICT, and name the FK guard `_fleet_0048_fk_guard`. I checked in sqlite3 in R1: a connection with an unretired target still fails with `FOREIGN KEY constraint failed`, and retire-then-delete succeeds.
2. **`canonical_key`** is unchanged and globally UNIQUE. The GitLab key embeds the connection's `Uuid::new_v4()` (`domain.rs:248-259`, `gitlab.rs:143`).
3. **`fleet.target.retired`:** one row per target. `target_id` is set, `host_id` and `placement_id` are NULL, `actor_user_id` is the deleting user, and `detail_json` carries the connection id and the per-target revoke counts. It is a direct INSERT in the transaction. Don't use registry's private `audit_and_event`, which would write a second audit row.
4. **No fleet-target DELETE route.** A grant on a retired target answers 404.
5. **The core function** lives in `fleet::targets` and takes `&mut SqliteConnection`. It returns either the retired ids and revoke counts, or the named blockers. The handler formats the 409, writes `gitlab.connection_deleted` after commit with `retiredTargetIds`, `hostGrantsRevoked`, `sessionGrantsRevoked` and `profileLinksRevoked`, and gains no fleet SQL.
6. **The sweep table** in F's result handoff should mark these:
   - the admission and diagnosis CTEs: "covered by revocation + `ensure_target_identity_for_pool`";
   - enrollment, the handshake and `authorization.rs`: "N/A, no GitLab path";
   - `readable_targets`, the event and host readers, `placement.rs:48` and `host_grants`: "history read, keep".

## Notes for O
None required (PASS). Warns 1-4 are guidance for T and F. I check them at the Phase 7 anti-duplication gate. Warn 1 needs no brief edit for this run. If the brief is reused later, delete the superseded option text in build §1 and §2.

## Patterns referenced
- `crates/gridops-core/src/fleet/targets.rs`:
  - `:59-230`: sync, the key path, `list_targets`;
  - `:551-758`: `save_pool_fleet`;
  - `:795-934`: `LIVE_WORKLOADS`, `delete_pool`'s `macro_rules!` fragments, `link_pool_target`.
- `crates/gridops-core/src/fleet/registry/delete.rs:150-180` and `registry/mod.rs:1155-1206`: the revoke with `revision+1`, and `set_host_grant`'s existence check.
- `crates/gridops-api/src/resources/gitlab.rs:369-450` (the current delete) and `crates/gridops-api/src/webhooks/gitlab.rs:357-359` (BEGIN IMMEDIATE into core).
- `migrations/0022_fleet_foundation.sql` (the referrers, the grant tables, `fleet_events`), `0040_fleet_host_delete.sql` (the final-state trigger) and `0047_gitlab.sql` §5 (the rebuild).
- `crates/gridops-core/tests/fleet_schema.rs:253-270, 600-620, 769, 827-906, 1176` and `crates/gridops-api/tests/control_plane_contracts.rs:2345`.
