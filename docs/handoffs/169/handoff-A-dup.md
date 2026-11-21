# Handoff-A-dup: Phase 7 - Runner rows and dashboard take the pool platform from the registry (#169)  (anti-duplication gate)

**Date:** 2026-11-21
**Branch:** claude/gitlab-169-runner-rows
**Diff base:** acd2afd542fa17668f907b358080b35c5b899383   **Diff head:** 83f0a4855cf25eecc3e2aa91c93ca39f246178c6
**Reuse map:** none (no survey.md); the analogues are named in docs/handoffs/169-brief.md and docs/handoffs/169/handoff-A.md
**Verdict:** PASS

## Summary
F extended the analogous objects and built no parallel path. The two hooks sit on #159's single `PoolBindings` key, resolve through the existing `platformForPool`, and GitLab reuses `connectionLabel`/`connectionAddress`. The API change adds columns to the existing `runners` handler and fixes `runner_pools` SQL in place. All five Phase 3 warns were adopted.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | none | - | No duplication; extension is clean. | - |

Checked:
- **PoolBindings:** `runnerDestination` and `scopeLabel` are required members of the existing key (`types.ts`). `RunnerDestinationView` is a `Pick` of `PoolDestinationView`, not a new shape.
- **Registry:** `platformForPool` is widened (`scope?: string | null`). The absent-scope default lives in `registry.tsx` (`absentScopePoolPlatform`), as `platformForDeliveryId` does. An unknown scope still gets `unknownPoolPlatform`, which now supplies both hooks.
- **GitLab:** `connectionAddress`/`connectionLabel` are widened to a structural `ConnectionParts` (`gitlab.tsx`). `runnerDestination` calls them, with no second join. GitHub's hook returns today's `repository ?? accountLogin`.
- **Components:** `runner-row.tsx` has one local helper, `runnerDestination`, that calls the registry. The row has no platform or scope comparison, and the three destination sites use it. `getPlatform(runner.platform)` is untouched. `routes/_app/index.tsx` calls `platformForPool(pool).pool.scopeLabel(pool)`; the dashboard has no other consumer.
- **API:** `poolScope` and `gitlabConnectionName` are added to the existing `runners` SELECT and JSON, reusing the existing `LEFT JOIN gitlab_connections gc`. `total_runners` is fixed inline (`r.id IS NOT NULL AND r.deleted_at IS NULL`). No new handler or helper.
- **Tests:** the source guard extends the existing `SHARED` and `BRANCHING` lists in `tests/platform-neutral-source.test.ts` (no parallel `describe`). The Rust tests go in `resources/gitlab_tests.rs`, and the Vitest tests in `tests/runner-row-platform.test.ts`. `tests/dashboard-pool-scope.test.ts` is a new file for the dashboard route; this is a new surface with no existing home, and it follows the `pool-destination.test.ts` loader-mock pattern. It is not drift.
- **Layering and direction:** route to feature registry, and `types.ts` already imported `Runner`. No lower-to-higher import, and GitLab-specific code stays in `gitlab.tsx`.
- **Coexistence:** the diff touches only files in the brief's scope. No drive-by edits to shared or other-agent code.

## Notes for F
Not required (PASS).
