# Handoff-A-dup: Phase 7 - Pool list and pool detail take the destination from the platform registry (#159)  (anti-duplication gate)

**Date:** 2026-11-21
**Branch:** claude/gitlab-159-pool-destination
**Diff base:** e022a5d   **Diff head:** a3c448d
**Reuse map:** none (no survey.md); the brief's Scope section names the analogues, and handoff-A.md (Phase 3, PASS) confirmed them.
**Verdict:** PASS

## Summary
F extended the objects the plan named and built no parallel path:
- the `PlatformDescriptor` contract (one optional `pool` key, in the style of `connections` / `platformOptions`);
- the registry (`platformForPool` beside `getPlatform` / `platformForDeliveryId`);
- `resources/gitlab.rs` (`pools_gitlab_connections` beside `pool_gitlab_connections`).

`RepositoryList` moved into `github.tsx` as the brief's named refactor-to-extend. `readOnlySummary` is untouched.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | info | `crates/gridops-api/src/resources/gitlab.rs:506-556` | The batched and per-pool queries share `POOL_CONNECTIONS` and `POOL_CONNECTION_ORDER` and map rows through `pool_connection_json`, so the SQL cannot drift. The batch uses the house `IN (SELECT value FROM json_each(?))` pattern. | None. |
| 2 | info | `src/features/platform-connections/registry.tsx:51-84` | The unknown-scope fallback is registry-owned and reuses `unknownPlatform` as its base. No hook reads `name`, so the raw scope is never echoed. Pools do not fall back to GitHub; `platformForDeliveryId`'s fallback is unchanged. | None. |
| 3 | warn | `src/features/runners/runner-row.tsx:46,109,127`; `src/routes/_app/index.tsx:279` | Carried from handoff-A finding 5, not introduced by this diff: the runner row and the dashboard pool row still show a GitLab pool under the installation's GitHub login or as "Gitlab". The new `platformForPool` / `getPlatform` seams are the right place to fix them. | O: file a follow-up in epic #136 if none exists. Do not fix with a platform branch. |
| 4 | warn | `src/features/platform-connections/gitlab.tsx:260-270` | `PoolDestination` renders a connection chip (`bg-panel-subtle rounded-md px-2 py-1 text-xs`) in the same style as GitHub's `RepositoryList` row, written separately. It is a different shape (two lines) in a different descriptor, so it is not a parallel path. | None needed; revisit only if a third platform adds the same chip. |

No duplication; extension is clean. No drift introduced: layering (`types.ts` imports `RunnerPool` from `operations.functions` in the direction it already imports `Runner` / `SettingsPage`; `registry.tsx` imports `PropertyRow` from `components/page`, which imports no feature code), naming (`platformForPool`, `PoolBindings`, `PoolDestinationView` match the neighbours), and the source-guard test (routes join `SHARED`, new tokens join `BRANCHING`, per-route registry-import assertion) all follow existing patterns. No unplanned edits to shared or other-agent-owned code: the diff touches only the files the brief names.

## Notes for F
Not required (PASS).
