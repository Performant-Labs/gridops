# Handoff-F: Phase 6 - Pool list and pool detail take the destination from the platform registry

**Date:** 2026-11-21
**Branch:** claude/gitlab-159-pool-destination (worktree head f33d411)
**Issue:** #159

## What was done
- `src/features/platform-connections/types.ts`: new `PoolDestinationView` (`{ text, title, leading? }`, in the style of `ConnectionRowView`) and `PoolBindings` (`{ scopes, destination(pool: RunnerPool), Destination: ComponentType<{ pool: RunnerPoolDetail }> }`). These are grouped under one optional `pool?` key on `PlatformDescriptor`.
- `src/features/platform-connections/registry.tsx`: added `platformForPool(pool)`, which returns a `PoolPlatform` (a descriptor whose `pool` is always present). It finds the registered platform whose `pool.scopes` contains the pool's scope. Otherwise it returns a registry-owned `unknownPoolPlatform`, which shows "—" / "Unknown destination" / a dashed placeholder square in the list and Scope "Unknown" in the detail.
- `src/features/platform-connections/github.tsx`: `pool: { scopes: ["repository", "organization"], … }`. The list cell logic and avatar moved here unchanged. The detail rows (Scope, Account with avatar, then `RepositoryList` or Runner group) moved here unchanged, and `RepositoryList` and its toggle moved out of the route verbatim.
- `src/features/platform-connections/gitlab.tsx`: `pool: { scopes: [GITLAB], … }`.
  - List cell: GitLab icon, then the one connection's name, or `N connections` with every name joined by `, ` in the title, or `GitLab` when there are none or the field is absent.
  - Detail: Scope "GitLab", then one two-line chip per connection (name, then `baseUrl/scopePath`) in server order, or "No connections listed".
  - Partial data: a nameless connection shows its address once, and an empty part adds no stray `/`.
- `src/features/operations/operations.functions.ts`: `RunnerPool.gitlabConnections?: GitlabPoolConnection[]` (a type-only import).
- `src/routes/_app/runner-pools.index.tsx`: `PoolRow` renders `platformForPool(pool).pool.destination(pool)` (leading, then text, with title). It no longer uses `Avatar`/`githubAvatar` or compares `scope`.
- `src/routes/_app/runner-pools.$poolId.index.tsx`: the Destination group renders `<PoolDestination pool={pool} />` from `platformForPool(pool).pool.Destination`. Removed `RepositoryList`, `useState`, `ChevronDown`, `cn`, `Avatar` and `githubAvatar`.
- `crates/gridops-api/src/resources/gitlab.rs`:
  - New `POOL_CONNECTIONS` (SELECT + JOIN) and `POOL_CONNECTION_ORDER` constants, shared by the detail's `pool_gitlab_connections` (same output) and the new `pools_gitlab_connections(state, &[String]) -> HashMap<String, Vec<Value>>`.
  - The new function is one `IN (SELECT value FROM json_each(?))` query, ordered `membership.pool_id, membership.created_at, connection.id`, with each row mapped through `pool_connection_json`.
- `crates/gridops-api/src/resources.rs`: `runner_pools` calls `pools_gitlab_connections` once, after the page query, and adds `"gitlabConnections": map.remove(id).unwrap_or_default()` to each item (`[]` for a pool with none).

## Design decisions
- **One optional `pool` key, not three fields (A warn 1).** A platform that owns a scope must supply both renderers, and the type enforces it. Bitbucket omits the key, so its pools resolve to GitHub, their scope's owner. `platformForPool` returns `PlatformDescriptor & { pool: PoolBindings }` through a type-guarded `find`, so the routes call the hooks unconditionally with no `?.` and no route-side fallback.
- **The fallback lives in the registry (A warn 2).** It is a module-level constant, so it is stable and outside `platforms`. None of its hooks reads `name`, so `azure-devops` is never echoed. Its placeholder is a `span`, neither an `img` nor an icon, as the wireframe's "neutral placeholder square" requires. `platformForDeliveryId`'s GitHub fallback is untouched.
- **The detail hook is a component the route renders as JSX** (`const PoolDestination = platformForPool(pool).pool.Destination; <PoolDestination pool={pool} />`), because GitHub's rows hold toggle state. Each descriptor's component identity is stable, so state is kept across re-renders of the same pool.
- **GitLab Scope text is the literal "GitLab", with no `capitalize` span.** Today's raw value renders as "Gitlab" (wireframe D1). GitHub keeps `<span className="capitalize">{pool.scope}</span>` exactly.
- **GitLab connection chip title.** The whole chip's `title` is the name and address on two lines, so the truncated address is still readable (wireframe D note 2). Tests read `textContent`, so the title does not affect the "address appears exactly once" pin.
- **Rust: a shared SELECT and ORDER constant (A warn 3).** The detail query now also selects `membership.pool_id`. `pool_connection_json` never reads it (see evidence), so the detail output is unchanged. An empty page returns early with no query.
- **Scope cap.** This change touches 9 production files on two surfaces (API and UI). I did not split it: the brief, A and T planned it as one unit, T's RED spans both layers, the Rust part is about 40 lines, and this is a single coherent defect, not a polish batch. A split would have left half of T's suite RED with no owner.

## Reuse / extend-vs-new
Extended, as the brief and A recommended:
- the `PlatformDescriptor` contract (a new optional key in the style of `connections` / `platformOptions`);
- the registry (`platformForPool` beside `getPlatform` / `platformForDeliveryId`, reusing `unknownPlatform` for the fallback's base);
- `resources/gitlab.rs` (the batch helper beside `pool_gitlab_connections`, reusing `pool_connection_json` and the `json_each(?)` batch pattern of `queued_job_pools` / `run_platforms`).

`RepositoryList` moved into `github.tsx` (the refactor-to-extend the brief names). No parallel path was created, and `readOnlySummary` is unchanged.

## Architecture notes for A
- **Contract change:** `PlatformDescriptor` gains the optional `pool?: PoolBindings`, and `types.ts` exports `PoolDestinationView` and `PoolBindings`. `types.ts` now also imports the `RunnerPool` type from `operations.functions` (the same direction as its existing `Runner`/`SettingsPage` imports).
- **Registry exports:** `platformForPool` and the `PoolPlatform` type. `registry.tsx` now imports `PropertyRow` from `~/components/page`. `components/page.tsx` imports nothing from features, so there is no cycle.
- **Descriptor imports:** `github.tsx` and `gitlab.tsx` import `PropertyRow` (and `github.tsx` imports `githubAvatar`), as A's plan review anticipated.
- **Wire field:** `operations.functions.ts` takes a type-only import of `GitlabPoolConnection` from `runner-pools.functions` (no cycle; the file is already on the GitLab allowlist).
- **API:** the `GET /api/v1/runner-pools` items gain `gitlabConnections`. No other endpoint, migration or OpenAPI change: that 200 schema is a bare `object`, per A.
- **Out of scope:** the runner-row and dashboard siblings (A warn 5) are untouched.

## Deviations from spec / wireframe
None.
- The wireframe's GitLab icon was a "GL" stand-in; the real lucide `Gitlab` icon from the descriptor is used at 16px (`size-4`), matching the avatar slot.
- The unknown-scope placeholder is a 16px dashed rounded square.

## Tier 1 self-check (incl. tests now GREEN)
- `npx vitest run tests/pool-destination.test.ts tests/platform-neutral-source.test.ts`: **Test Files 2 passed (2), Tests 45 passed (45)**. All of T's 26 RED tests are now GREEN.
- `npx vitest run` (full): **Test Files 52 passed (52), Tests 582 passed (582)**. The RED baseline was 26 failed / 556 passed.
- `npm run lint` (`eslint src tests *.config.ts eslint.config.js --max-warnings=0`): clean, no output.
- `npm run build` (`vite build && tsc --noEmit`): built in 1.22s, and typecheck exited 0. A bare `npm run typecheck` in a fresh worktree fails only because the gitignored `src/routeTree.gen.ts` has not been generated yet; the build generates it.
- `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api the_pool_list_carries`: `test resources::gitlab_tests::the_pool_list_carries_each_pools_gitlab_connections_in_detail_order_and_none_for_a_github_pool ... ok`, 1 passed.
- `cargo test -p gridops-api` (full): unit tests **123 passed**, `control_plane_contracts` **36 passed**, and `fleet_http` **143 passed, 1 failed**. The failure is `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`:
  - it depends on the environment and predates #159: the fixture agent reports the compiling OS (`native.rs:201`), so on this arm64 Mac the "Linux" machine counts as a macOS ARM64 machine and the preflight prints "All checks passed";
  - `scripts/release-preflight.sh` never calls `/runner-pools`.

  See evidence.md.
- `cargo fmt --check -p gridops-api`: exit 0.
- `cargo clippy -p gridops-api --all-targets -- -D warnings`:
  - Only the pre-existing `unused_async_trait_impl` findings appear (host-agent `helper.rs`/`native.rs`, api `fleet/boundary.rs`, `fleet/enrollment.rs`, `resources/route_security.rs`).
  - With that lint allowed, only the pre-existing `assert!`-emptiness findings remain (`overview_queue.rs:403-404`, `tests/fleet_http/{builtin_host_budget,recovery,start_retry}.rs`). These are the same set T recorded.
  - There is no finding in `resources.rs` or `resources/gitlab.rs`.

## Evidence appendix
docs/handoffs/159/evidence.md

## Tests that look wrong (for T)
None.

## Known issues
- The brief's headless UI walkthrough (`/runner-pools` and a GitHub and a GitLab pool's detail) is Tier 2/3 and was not run here; it belongs to T and U.
- The local `release_preflight` failure and the clippy findings above predate #159 and are outside its scope.

## Files changed
- crates/gridops-api/src/resources.rs
- crates/gridops-api/src/resources/gitlab.rs
- src/features/operations/operations.functions.ts
- src/features/platform-connections/github.tsx
- src/features/platform-connections/gitlab.tsx
- src/features/platform-connections/registry.tsx
- src/features/platform-connections/types.ts
- src/routes/_app/runner-pools.$poolId.index.tsx
- src/routes/_app/runner-pools.index.tsx
