# [GitLab: runner rows and dashboard show a GitLab pool as GitHub](https://github.com/Performant-Labs/gridops/issues/169)

Part of epic #136. Follows #159 (merged as `ea3ec47`), which added `platformForPool(pool: { scope })` in `src/features/platform-connections/registry.tsx` and the optional `pool?: PoolBindings` descriptor key (`scopes`, `destination(pool: RunnerPool)`, `Destination` component) in `types.ts`. The reviews of #141 and #159 raised these leftovers three times.

## Need

Two more views assume every pool is a GitHub pool, and one pool-list count is off by one.

1. **Runner rows** (`src/features/runners/runner-row.tsx`): the destination is `runner.repository ?? runner.accountLogin` in three places, the details sheet's "Repository" row (~line 46), the small line under the name below `md` (~line 109) and the right-hand column with its title (~line 127). A runner of a `scope: "gitlab"` pool has no repository, so it shows the GitHub installation's login. `RunnerRow` is used by `src/routes/_app/runners.tsx` and the pool detail page.
2. **Dashboard** (`src/routes/_app/index.tsx`, `PoolsSection`, ~line 279): `{pool.scope} · {pool.mode}` with CSS `capitalize`, so a GitLab pool shows "Gitlab · ephemeral".
3. **`totalRunners`** in the pool list API (`runner_pools` in `crates/gridops-api/src/resources.rs`, ~line 1329): `COUNT(CASE WHEN r.deleted_at IS NULL THEN 1 END)` over `LEFT JOIN runners r` counts the NULL row of a pool with no runners, so an empty pool reports 1.

## Scope

### API

- The runners list (`runners` handler, `resources.rs` ~lines 1925-2010) adds two fields to each item: `poolScope` (`p.scope`) and `gitlabConnectionName` (`gc.name` from the existing `LEFT JOIN gitlab_connections gc`; null for non-GitLab runners or a gone connection). Add both to the `Runner` type in `src/features/operations/operations.functions.ts`, optional (absent from an older server).
- Fix `total_runners` so a pool with no runners counts 0 (e.g. `COUNT(r.id)` guarded by `r.deleted_at IS NULL`, or `CASE WHEN r.id IS NOT NULL AND r.deleted_at IS NULL`). Sweep the other `LEFT JOIN runners` counts in `resources.rs` for the same shape and fix any that count a NULL row. (`online`/`busy` and the dashboard's counts compare non-null columns and are already right; line ~342 counts `DISTINCT r.id` and is right. Confirm, don't assume.)
- No migration, no OpenAPI change unless the spec documents these lists (it did not for #159).

### Frontend: the registry decides

- Extend `PoolBindings` (one key, as #159's architecture review asked) with:
  - a **runner destination** hook: what a runner of this platform's pool shows as its destination, `(runner: Runner) => { text: string; title: string }`. GitHub's returns exactly today's `runner.repository ?? runner.accountLogin` for both. GitLab's returns the runner's connection name, falling back to `gitlabBaseUrl/gitlabScopePath` (no stray `/` when one part is missing), then "GitLab"; never the installation login.
  - a **scope label** hook for the dashboard: `(pool: { scope: string }) => string`. GitHub's returns today's text (the raw scope, which the dashboard's `capitalize` turns into "Repository" / "Organization"); GitLab's returns "GitLab".
- The registry's unknown-scope fallback supplies both: destination "—" with title "Unknown destination", scope label "Unknown". It never echoes the raw scope.
- `runner-row.tsx` resolves the pool's platform with `platformForPool({ scope: runner.poolScope })` and uses the hook in all three places. A runner from an older server without `poolScope` must look exactly as today (resolve as GitHub's `repository` scope, or keep today's text, chosen so GitHub/Bitbucket output is unchanged; no platform comparison in the component).
- `index.tsx` uses `platformForPool(pool).pool.scopeLabel(pool)` (or the chosen name) in place of `pool.scope`.
- Keep the existing `getPlatform(runner.platform)` uses (CI platform badge, runner link, actions menu): they name the runner's CI platform and are correct.
- A Bitbucket runner sits on a GitHub-scoped pool, so it resolves to GitHub and looks exactly as today.
- GitLab-specific code stays in `gitlab.tsx`, GitHub-specific in `github.tsx`; the "GitLab code has one home (#141)" test keeps passing.

## Constraints

- No deploy. Nothing GitLab ships to Mars until epic #136 is done.
- Rust builds share this Mac: keep `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2` (`.claude/settings.local.json`).
- Do not change the pool list/detail behaviour #159 shipped, the pool forms or the Targets picker.

## Done when

- `tests/platform-neutral-source.test.ts` covers `src/features/runners/runner-row.tsx` and `src/routes/_app/index.tsx`: neither compares a platform or kind id, reads `.scope` for display or branching other than passing it to `platformForPool`, or names `accountLogin`/`repository` as the destination outside the descriptors; both import from `~/features/platform-connections/registry`. Keep the patterns exact and greppable.
- Vitest (jsdom, as `tests/pool-destination.test.ts` does) shows, for runners of a GitHub repository pool, a GitHub organization pool (no repository), a Bitbucket runner on a GitHub pool, a GitLab pool runner with a connection name, with only an address, with neither, an unknown scope, and an older-server runner with no `poolScope`: the details sheet's destination row, the narrow-screen line and the right-hand column (text and title). GitHub, Bitbucket and older-server output is exactly today's.
- Vitest shows the dashboard pool row's scope text for repository, organization, gitlab and unknown scopes ("repository"/"organization" unchanged, "GitLab", "Unknown").
- Rust API tests: the runners list returns `poolScope` and `gitlabConnectionName` (null for a GitHub runner); the pool list returns `totalRunners: 0` for a pool with no runners and the right count for a pool with runners, including one with a deleted runner.
- The headless UI walkthrough opens `/runners`, a GitLab pool's detail page (runner rows) and the dashboard `/`, for a GitHub and a GitLab pool, and confirms the above.
- `npm run lint`, `npm run typecheck`, `npm test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test -p gridops-api` pass.
