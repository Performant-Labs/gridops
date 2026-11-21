# Handoff-T-red: Phase 4 - Runner rows and dashboard take the pool platform from the registry (#169)

**Date:** 2026-11-21
**Branch:** claude/gitlab-169-runner-rows
**Brief / wireframe reviewed:** docs/handoffs/169-brief.md, docs/handoffs/169/wireframe.html (frames A-E, contract table E), docs/handoffs/169/handoff-D.md, docs/handoffs/169/handoff-A.md

## A precondition
Confirmed: A returned PASS on the plan (Phase 3), 0 block / 5 warn (docs/handoffs/169/handoff-A.md). The tests follow A's warns 1, 2, 4 and 5 (test homes, guard patterns, registry-owned absent-scope default). Warn 3 (reuse `connectionAddress`/`connectionLabel`) is implementation guidance and is not pinned; the tests pin only the resulting text.

## Tests authored

### Rust API: `crates/gridops-api/src/resources/gitlab_tests.rs` (A warn 5: the #141 harness, beside the #141/#159 list tests)
| Test | Pins | Tier / why |
|---|---|---|
| `the_runners_list_carries_each_runners_pool_scope_and_gitlab_connection_name` | Brief API bullet 1: each runners-list item carries `poolScope` (`p.scope`) and `gitlabConnectionName` (`gc.name`). GitLab runner: `"gitlab"` and its connection's name. GitHub runner on an organization pool: `"organization"` and an explicit `null`. | Integration through the real router (`as_admin GET /api/v1/runners`). The wire contract is the behaviour, and a unit test cannot see the SQL. |
| `the_pool_list_counts_no_runners_for_an_empty_pool_and_only_live_runners_otherwise` | Brief API bullet 2: `totalRunners` is 0 for a pool with no runners, 2 for two live runners, and 1 for one live plus one deleted runner. | Integration through the real router (`GET /api/v1/runner-pools`). The bug is in the SQL `LEFT JOIN` count. |

### Vitest: `tests/runner-row-platform.test.ts` (A warn 5: the existing runner-row home and its `runner()` fixture)
New `describe("runner destination comes from the pool's platform (#169)")`. Each case renders `RunnerRow` with the production registry and checks three places:
- the right-hand column's text and `title` (the one `xl:block` element);
- the line under the name (the one `md:hidden` element);
- the details sheet's "Repository" row value. The label stays "Repository", per D's open question 1.

The `it.each` cases, from contract table E:

| Case | Text | Title | Today |
|---|---|---|---|
| GitHub repository pool (`poolScope: "repository"`) | `acme/web` | `acme/web` | passes (baseline guard) |
| GitHub organization pool, no repository | `acme` | `acme` | passes (baseline guard) |
| Bitbucket runner on a GitHub (`organization`) pool | `octo-org` | `octo-org` | passes (baseline guard) |
| Older server (no `poolScope`), with a repository | `acme/web` | `acme/web` | passes (baseline guard) |
| Older server (no `poolScope`), no repository | `acme` | `acme` | passes (baseline guard) |
| GitLab pool, connection name | `Mobile app` | same | **RED** |
| GitLab pool, no name: `gitlabBaseUrl/gitlabScopePath` | `https://gitlab.example.com/acme/mobile` | same | **RED** |
| GitLab pool, base URL only (no stray `/`) | `https://gitlab.example.com` | same | **RED** |
| GitLab pool, scope path only (no stray `/`) | `acme/mobile` | same | **RED** |
| GitLab pool, no name and no address | `GitLab` | `GitLab` | **RED** |
| Unknown scope (`azure-devops`) | `—` | `Unknown destination` | **RED** |

In every GitLab case the installation login is `octo-org`, so an exact-text match also proves the row never falls back to it. The unknown case carries a repository and a login, so an exact `—` proves neither of them, nor the raw scope, leaks.

The tier is a jsdom component render, the cheapest one that sees all three places. jsdom applies no CSS, so all three render together, and the responsive class is the only handle on "the narrow line" and "the right-hand column".

One more test: `the narrow-screen line still shows the running job's name over the destination`. A GitLab runner with `currentJobName` shows `build:ios` on the narrow line and `Mobile app` in the column. It guards the `currentJobName ?? destination` order through F's refactor. It is RED only because the column assertion depends on the new destination.

### Vitest: `tests/dashboard-pool-scope.test.ts` (new)
Renders the Overview route through `Route.options.component`, with a mocked `createFileRoute` loader. This is A warn 5's pattern, so `PoolsSection` is not exported just for the test. The page is signed out with no installations, so it makes no fetches.

For each pool row, the test reads the `scope · mode` span and checks that it keeps the `capitalize` class. That class is what makes "repository" display as "Repository" today.

| Pool | Expected | Today |
|---|---|---|
| `web-ci` (repository) | `repository · ephemeral` | passes (baseline guard) |
| `org-pool` (organization) | `organization · persistent` | passes (baseline guard) |
| `gl-pool` (gitlab) | `GitLab · ephemeral` | **RED** |
| `odd-pool` (`azure-devops`) | `Unknown · ephemeral` | **RED** |

A route render is the cheapest tier that sees the dashboard row without exporting a private component.

### Source guard: `tests/platform-neutral-source.test.ts` (A warn 2: join `SHARED`, receiver-qualified patterns)
- `SHARED` adds `src/features/runners/runner-row.tsx` and `src/routes/_app/index.tsx`.
- `BRANCHING` adds two patterns:
  - `/["'](repository|organization)["']/`, a quoted GitHub scope id. A warn 1: the absent-scope default lives in the registry, not as a literal in the row.
  - `/\b(runner|pool)\.(repository|accountLogin)\b/`, GitHub's destination fields outside the descriptors.
- New line rule: a line containing `poolScope` must also contain `platformForPool(`.
- New `it.each` over both files: each imports `platformForPool` from `~/features/platform-connections/registry` and calls `platformForPool(`.
- The existing `/\bpool\.scope\b/` pattern now also covers `index.tsx`.
- All four #159 `SHARED` files still pass the new patterns. I checked this by running them, not by assuming it.

## RED confirmation

`npx vitest run` (full suite): `Test Files 3 failed | 50 passed (53)`, `Tests 13 failed | 589 passed (602)`. Exactly the 13 intended tests fail. No other test regressed, and nothing fails on an import, setup or type error. Exact failing output:

- Runner row, GitLab and unknown cases: `AssertionError: right-hand column: expected { text: 'octo-org', title: 'octo-org' } to deeply equal { text: 'Mobile app', … }`. The same shape appears for each GitLab case (received `octo-org`, the installation login, which is the bug). The unknown case gives `expected { text: 'octo-org/app', … } to deeply equal { text: '—', … }`.
- Narrow line with a job: `AssertionError: expected 'octo-org' to be 'Mobile app'` (the column assertion).
- Dashboard: `expected { text: 'gitlab · ephemeral', capitalized: true } to deeply equal { text: 'GitLab · ephemeral', capitalized: true }`, and `received "azure-devops · ephemeral"` for `Unknown · ephemeral`.
- Source guard for `runner-row.tsx`: flags lines 46, 109 and 127 (`runner.repository ?? runner.accountLogin`).
- Source guard for `index.tsx`: flags line 279 (`{pool.scope} · {pool.mode}`).
- `platformForPool` import and call: `expected 'import { Link } from "@tanstack/react…' to match /import \{[^}]*\bplatformForPool…/`, for both files.

`CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api --bins -- the_runners_list_carries_each_runners_pool_scope_and_gitlab_connection_name the_pool_list_counts_no_runners_for_an_empty_pool`, which reports `0 passed; 2 failed`:
- `gitlab_tests.rs:2223`: `assertion left == right failed`, with `left: None` and `right: Some(String("gitlab"))`. `poolScope` is absent from the item; the item is dumped, and it has no `poolScope` or `gitlabConnectionName`.
- `gitlab_tests.rs:2291`: `assertion left == right failed: a pool with no runners`, with `left: Number(1)` and `right: Number(0)`. This is the NULL-row count bug. The 2 and 1 assertions after it already hold today and guard the fix.

Hygiene, on the test files as authored:
- `npx eslint` on the three TS test files: clean.
- `npx tsc --noEmit`: clean, after `npx vite build` generated the gitignored `src/routeTree.gen.ts`. Without it, every route file reports unrelated errors.
- `cargo fmt --check`: clean, after `cargo fmt` reflowed two `assert_eq!` calls in my test.
- `cargo clippy -p gridops-api --all-targets -- -D warnings`: no finding in `gitlab_tests.rs`. The crate does not pass clippy as-is on this machine's toolchain (rustc 1.99.1), because of two lints that already fail in files this run does not touch:
  - `clippy::unused-async-trait-impl`: `fleet/boundary.rs:78`, `fleet/enrollment.rs:46`, `resources/route_security.rs:16`, and `gridops-host-agent/src/helper.rs:188` and `native.rs:200`;
  - `clippy::assert-is-empty`: `resources/overview_queue.rs:403-404`, and `tests/fleet_http/*`.

  With those two lints allowed, clippy finishes clean.

`node_modules` was absent from the worktree, so I ran `npm ci` (lockfile-pinned, identical to master's).

## Notes for F
- The Done-when "the registry decides" contract is pinned at the source level. Keep the call on one line: `platformForPool({ scope: runner.poolScope })`. A `poolScope` on a line without `platformForPool(` fails the guard.
- In `index.tsx`, pass the pool itself: `platformForPool(pool).pool.<scopeLabel>(pool)`. `platformForPool({ scope: pool.scope })` trips #159's existing `\bpool\.scope\b` pattern.
- Quoted `"repository"` / `"organization"` literals are not allowed in either file, so the older-server default belongs in `registry.tsx` (A warn 1).
- The hook names (`runnerDestination`, `scopeLabel`) are not pinned. The tests assert rendered output only.
- The dashboard row must keep its `capitalize` class, which is how GitHub's raw scope keeps reading "Repository" and "Organization".

## Ready for F
Confirmed: RED is valid. 13 Vitest and 2 Rust tests fail on the feature assertion, and the baseline cases (GitHub, Bitbucket, older server, repository and organization scope text) pass and guard against regressions. F may implement against these tests.
