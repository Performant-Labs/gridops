# Handoff-F: Phase 5 - Runner rows and dashboard take the pool platform from the registry (#169)

**Date:** 2026-11-21
**Branch:** claude/gitlab-169-runner-rows
**Issue:** #169

## What was done
- `crates/gridops-api/src/resources.rs`:
  - `runners`: selects `p.scope AS pool_scope` and `gc.name AS gitlab_connection_name`, and emits `poolScope` and `gitlabConnectionName` on each item. `gitlabConnectionName` is null for a GitHub or Bitbucket runner, or when the connection is gone.
  - `runner_pools`: `total_runners` now counts `CASE WHEN r.id IS NOT NULL AND r.deleted_at IS NULL`, so an empty pool reports 0.
- `src/features/operations/operations.functions.ts`: `Runner` gains the optional fields `poolScope?: string` and `gitlabConnectionName?: string | null`. An older server omits them.
- `src/features/platform-connections/types.ts`:
  - new named type `RunnerDestinationView = Pick<PoolDestinationView, "text" | "title">`;
  - `PoolBindings` gains the **required** members `runnerDestination(runner)` and `scopeLabel(pool)`. The doc comment on `scopeLabel` explains the `capitalize` span.
- `src/features/platform-connections/github.tsx`:
  - `runnerDestination` returns `runner.repository ?? runner.accountLogin` for both text and title (today's output);
  - `scopeLabel` returns the raw scope.
- `src/features/platform-connections/gitlab.tsx`:
  - `connectionAddress` and `connectionLabel` are widened to a structural `ConnectionParts` with nullable fields;
  - `runnerDestination` = `connectionLabel({...runner fields}) || "GitLab"`;
  - `scopeLabel` returns "GitLab".
- `src/features/platform-connections/registry.tsx`:
  - `unknownPoolPlatform` supplies `runnerDestination` ("—" / "Unknown destination") and `scopeLabel` ("Unknown");
  - `platformForPool` accepts `{ scope?: string | null }`. An absent scope returns GitHub's pool platform; an unknown string still returns the neutral fallback.
- `src/features/runners/runner-row.tsx`: a local `runnerDestination(runner)` calls `platformForPool({ scope: runner.poolScope }).pool.runnerDestination(runner)`. Its result is used in the details-sheet "Repository" row, on the narrow line (`currentJobName ?? destination.text`) and in the right-hand column (text and title).
- `src/routes/_app/index.tsx`: `PoolsSection` shows `platformForPool(pool).pool.scopeLabel(pool)` in place of `pool.scope`, inside the unchanged `capitalize` span, and imports `platformForPool` from the registry.

## Design decisions
- **Absent-scope default lives in the registry** (A warn 1). `platformForPool` treats `undefined`/`null` as GitHub's, as `platformForDeliveryId` already does. I rejected `runner.poolScope ?? "repository"` in the row: it would put a GitHub scope id in a shared component, and the new `BRANCHING` pattern forbids it.
  - The GitHub constant is narrowed with a `hasPoolBindings` type guard (`absentScopePoolPlatform`) rather than spread into a copy, so the descriptor keeps its identity. The guard falls back to `unknownPoolPlatform` only if GitHub ever lost its `pool` key.
  - An empty-string scope is treated as unknown, not absent.
- **Both hook members are required** (A warn 4). The type forces GitHub, GitLab and the unknown fallback to supply them, so the row and the dashboard call them with no `?.`.
- **GitLab reuses `connectionLabel`/`connectionAddress`** (A warn 3), widened to a structural nullable type instead of adding a second join beside `runnerLink`. `GitlabPoolConnection` is still assignable, so pool list and pool detail output is unchanged.
- **The `total_runners` fix is the brief's second form**, `r.id IS NOT NULL AND r.deleted_at IS NULL`. It keeps the `COUNT(CASE …)` shape of the sibling columns. The sweep for other `LEFT JOIN runners` counts relies on A's confirmed check (`:342` DISTINCT, the others compare columns that are NULL on a missing runner), so I made no other count change.
- The row computes the destination once per component (`RunnerRow` and `RunnerDetails`) and does not compute it separately for each place.

## Reuse / extend-vs-new
- Extended #159's `PoolBindings` key (types.ts), `platformForPool` and `unknownPoolPlatform` (registry.tsx), GitLab's `connectionAddress`/`connectionLabel` (gitlab.tsx), the existing `runners` handler (with its existing `LEFT JOIN gitlab_connections gc`), and the `runner_pools` SQL in place.
- I created no new object or parallel path. `RunnerDestinationView` is a `Pick` of the existing `PoolDestinationView`.

## Architecture notes for A
- **Public interface change:** `PoolBindings` has two new required members, and `platformForPool`'s parameter is widened to `{ scope?: string | null }`. The widening is backward compatible for every existing caller.
- **Wire contract:** `/api/v1/runners` items gain `poolScope` and `gitlabConnectionName`. There is no OpenAPI change, because `/api/v1/runners` is not in the spec (per A).
- **Dependency direction:** `routes/_app/index.tsx` now imports the platform registry, which is route → feature. No new packages.

## Deviations from spec / wireframe
None. Two copy choices follow D's defaults:
- the details-sheet label stays "Repository" (D open question 1);
- the GitLab address keeps its scheme, as T assumed and the tests pin.

## Tier 1 self-check (incl. tests now GREEN)
- `npm run typecheck`: clean.
- `npm run lint`: clean (`--max-warnings=0`).
- `npx vitest run`: `Test Files 53 passed (53)`, `Tests 602 passed (602)`. All 13 RED Vitest tests are now GREEN, with no regressions.
- `cargo fmt --check`: clean.
- `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api --bins -- the_runners_list_carries_each_runners_pool_scope_and_gitlab_connection_name the_pool_list_counts_no_runners_for_an_empty_pool`: `2 passed; 0 failed`.
- `cargo test -p gridops-api --no-fail-fast`:
  - unittests (main.rs): `125 passed; 0 failed`;
  - `control_plane_contracts`: `36 passed; 0 failed`;
  - `fleet_http`: `143 passed; 1 failed`. The failure is `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` and is host-environmental (see Known issues).
- `cargo clippy -p gridops-api --all-targets -- -D warnings`: fails only on the 5 pre-existing `clippy::unused-async-trait-impl` sites T recorded, all in files this run does not touch:
  - `fleet/boundary.rs:78`, `fleet/enrollment.rs:46` and `resources/route_security.rs:16`;
  - `gridops-host-agent` `helper.rs:188` and `native.rs:200`.

  With that lint and `clippy::assert-is-empty` allowed, clippy finishes clean.

## Evidence appendix
`docs/handoffs/169/evidence.md` has six facts:
- `runner_pools.scope` and `gitlab_connections.name` are NOT NULL;
- the existing `LEFT JOIN gitlab_connections gc` and `LEFT JOIN runners r`;
- the `platformForDeliveryId` GitHub-default precedent;
- a Bitbucket target requires a GitHub target.

## Tests that look wrong (for T)
None.

## Known issues
- `fleet_http::release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on this Mac, and the cause is not this diff:
  - The fixture's "Linux machine" runs the real host agent, which probes the build host's OS with `cfg!(target_os = "macos")` (`crates/gridops-host-agent/src/probe.rs:17`). On a Mac it reports macOS ARM64, so the preflight passes `macos-machine`. The test's output shows `PASS  macos-machine: macOS ARM64 machine ready … fleet-host`.
  - `scripts/release-preflight.sh` calls only `/api/v1/fleet…` endpoints. This diff changes only the `runners` and `runner_pools` handlers.
  - It should pass on Linux CI.
- The headless UI walkthrough in the brief's Done-when (`/runners`, a GitLab pool's detail page, `/`) is a Tier 2/3 check owned by T and U/S. I did not run it.

## Files changed
- crates/gridops-api/src/resources.rs
- src/features/operations/operations.functions.ts
- src/features/platform-connections/types.ts
- src/features/platform-connections/github.tsx
- src/features/platform-connections/gitlab.tsx
- src/features/platform-connections/registry.tsx
- src/features/runners/runner-row.tsx
- src/routes/_app/index.tsx
