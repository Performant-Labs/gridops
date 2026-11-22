# Handoff-A-dup: Phase 7 - Retire a GitLab connection's fleet targets on removal (#183)  (anti-duplication gate)

**Date:** 2026-11-22
**Branch:** claude/gitlab-183-retire-target
**Diff base:** 37fdd0b (A plan review)   **Diff head:** f383b74
**Reuse map:** none (docs/handoffs/183/survey.md does not exist); analogues named in the brief and handoff-A.md: `LIVE_WORKLOADS` / `delete_pool` in `fleet/targets.rs`, 0047's table rebuild, `registry/delete.rs` revoke pattern.
**Verdict:** PASS

## Summary
F extended the analogous objects. The retire function lives in `fleet::targets` beside `delete_pool`. The liveness clause is shared with `LIVE_WORKLOADS` through one `live_placement!()` fragment. The revoke follows `registry/delete.rs`. The migration follows 0047. Each retired-target guard is one inline `retired_at IS NULL` predicate in existing SQL. There is no new route, module, helper layer or parallel path.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `docs/handoffs/183/handoff-F.md` ("Tests that look wrong") | Says the stale test `a_connection_a_fleet_target_references_is_refused_with_409_not_500` was not edited, but the diff deletes it (correct, since it pinned the removed behaviour). Prose only. | No action for the code; note only. |
| 2 | warn | `migrations/0048_fleet_target_retire.sql` (trigger WHEN) | The trigger allows a retire that keeps `gitlab_connection_id`, which would leave a retired row FK-blocking its connection. Reachable only by ad-hoc SQL; production always nulls it. Consistent with A decision §1. | Optional follow-up: require NULL connection on a GitLab retire. |

No duplication. Checked:
- The retire function is in `fleet::targets`, takes the caller's `&mut SqliteConnection`, and is called by the existing handler `delete_gitlab_connection`. The handler dropped its own `fleet_ci_targets` COUNT refusal and gained no fleet SQL.
- `LIVE_WORKLOADS` and `LIVE_CONNECTION_WORK` share one terminal-state list (`live_placement!()`), as the brief required. `LIVE_WORKLOADS` behaviour is unchanged.
- Key path: `target_id_for_key`, `ensure_target` (public signature unchanged) and `gitlab_connection_target` are extended with a private `ensure_live_target`. This is not a second target-creation path.
- Guards are inline predicates in the existing SQL (`list_targets`, `set_host_grant`, `ensure_target_identity_for_pool`, `link_pool_target`, `save_pool_fleet`), as A warn 3 asked. There is no `is_retired()` helper.
- There is no new route, and the FK stays NO ACTION (A decision §1).
- No unplanned edits to shared or other-agent code. The edited files are all within the brief's list.

## Notes for F
None (PASS).
