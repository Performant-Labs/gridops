# Handoff-A-dup: Phase 7 - GitLab shared contract (#137)  (anti-duplication gate)

**Date:** 2026-11-20
**Branch:** claude/gitlab-137-contract-fa3bbe
**Diff base:** 78a1266 (A plan PASS)   **Diff head:** 2afc773
**Reuse map:** docs/handoffs/137-brief.md (Scope, A-1..A-3, R2-1 and the deviation table D1-D13)
**Verdict:** PASS

## Summary
F extended every analogous object the brief named and built no parallel path. Each new object (`gitlab/mod.rs`, `gitlab/fake.rs`, `GitlabConnectionId`, `GitlabRunnerMaterial`, `0046_gitlab.sql`, `ApiErrorBody`) is one the brief places and justifies in writing.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `crates/gridops-core/src/fleet/targets.rs` (`ensure_gitlab_target`) | A GitLab-only helper beside `ensure_target` instead of widening the GitHub tuple. The public entry point is still `ensure_target`, the insert/SELECT tail is shared via `target_id_for_key`, and F gives a reason (avoids touching every GitHub bind). Acceptable; the two INSERTs are the only duplicated shape. | None required. Fold into one insert if a third platform lands. |
| 2 | warn | `crates/gridops-tart-agent/src/main.rs` | Private copies of `GitlabBootstrap`, the GITLAB_EXIT_* constants and `GITLAB_EXIT_REASONS`. Deliberate: the brief (R2-1) says the Tart agent has no `gridops-core` dependency and pins a copy test in each crate. | None. |
| 3 | warn | `crates/gridops-manager/src/main.rs` | `GitlabBootstrap` is a private manager struct, not a shared core type. Brief-pinned (two private camelCase `ProvisionRunner` structs); the manager reuses the core exit-code constants as the brief requires. | None. |

Checked and clean:
- **`CiTarget` / `CiPlatform` / `TargetKind`:** the variants were added in place. `is_bitbucket` now names its variants explicitly, and the exhaustive matches (`canonical_key`, `From<&CiTarget> for TargetKind`) gained arms in the existing functions.
- **`GitlabConnectionId`:** the same shape as `BitbucketConnectionId` (the brief's "alongside"), re-exported from `fleet/mod.rs`.
- **`target_from_db`:** the GitLab arm sits in the existing `match`, with no second loader.
- **`DiagnosticCode`:** the two variants were added to the existing enum. `FailureCode` is unchanged.
- **`ProvisioningPlan::for_selected_provider`:** rewritten in place to take `CiPlatform` (D9). Both callers (`gridops-api/src/resources.rs`, `gridops-reconciler`) were changed mechanically, with no second planner.
- **`SECRET_PREFIXES`:** extended with `glrt-` (`glpat-` was already there). No new redactor.
- **OpenAPI:** the existing `gridops-v1.json` was extended and no second spec file was added. `ApiErrorBody` is brief-specified (matches `gridops-api/src/error.rs`) and is not a duplicate of the fleet `ErrorResponse`, which has a different shape.
- **Gitlab client module:** `gitlab/mod.rs` sits beside `bitbucket.rs` and follows its credentials-per-call pattern. It reuses `ExternalId`, `CiTarget` and `GitlabConnectionId` from `fleet`, and redefines none of them.
- **Migration:** 0046 mirrors the 0019 (`runner_pool_*_connections`) and 0043 (active-jobs index) shapes, as the brief directs.
- **Drive-by edits to shared code:** none. Every touched file is in the brief's scope. `db.rs` has only the table-count test repair.
- **Handlers / AppState / Reconciler:** untouched, as R2-W3 requires.

No duplication; extension is clean.

## Notes for F
None (PASS).
