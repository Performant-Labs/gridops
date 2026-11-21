# Handoff-T-red: Phase 4 - Pool list and pool detail take the destination from the platform registry (#159)

**Date:** 2026-11-21
**Branch:** claude/gitlab-159-pool-destination (head cfac099; nothing under `src/` or under `crates/*/src` outside the test file changed)
**Brief / wireframe reviewed:** docs/handoffs/159-brief.md, docs/handoffs/159-brief-result-r1.md, docs/handoffs/159/wireframe.html (frames A-E), docs/handoffs/159/handoff-D.md, docs/handoffs/159/handoff-A.md

## A precondition
Confirmed: A returned **PASS** on the plan (`docs/handoffs/159/handoff-A.md`, commit cfac099), with five warns. Three of them shape the tests:
- **Warn 2:** the registry owns the unknown-scope fallback and its copy, and the raw scope is never shown. The tests pin "—" / "Unknown destination" (list) and Scope "Unknown" (detail), and check that `azure-devops` does not appear.
- **Warn 3:** the Rust test lives in `crates/gridops-api/src/resources/gitlab_tests.rs` and uses its `harness()` and `seed_*` helpers. There is no query counter.
- **Warn 4:** both routes join `SHARED`, and `BRANCHING` gains `githubAvatar`, `repositoryCount`, `bitbucketConnections` and `gitlabConnections`. Each route also gets a registry-import assertion. There is no blanket `/github/i` check, so the list route's GitHub empty-state copy stays.

## Baseline
On cfac099, after `npm ci`: `npx vitest run` gave **51 files, 548 tests, all passed**.

## Tests authored

### `tests/pool-destination.test.ts` (new; jsdom, the rendered routes with the production registry; 30 tests)
**Why jsdom and the routes.** The brief's claim is about what each view shows. Rendering the two route components pins the cell and the Destination group as the operator sees them, whatever F names the descriptor hooks. It also catches a correct hook that the route forgets to call.
- The only name pinned is `platformForPool`, which is the brief's own example and A's named resolver.
- The router is mocked as in `webhook-delivery-platform.test.ts`, and `fetch` serves empty runners and events lists.

| Group | Tests | Pins |
|---|---|---|
| Registry | 10 | <ul><li>`platformForPool` is exported from `registry.tsx`.</li><li>It resolves each of these to GitHub's descriptor (`toBe(getPlatform("github"))`): a repository pool with one repository, one with several, an organization pool, and a Bitbucket pool (as a list item and as a detail with workspaces). Bitbucket owns no scope.</li><li>It resolves a GitLab pool to GitLab's descriptor with one connection, two connections, and none.</li><li>An unknown scope (`azure-devops`) does not throw and gives a fallback that is not one of `platforms`.</li></ul> |
| List cell, GitHub and Bitbucket (**invariant guards, green before and after**) | 4 | <ul><li>The text and `title` are today's: `acme/web`, `5 repositories`, `acme` (organization), and `octo-org` (Bitbucket pool).</li><li>The only image is `https://github.com/<login>.png?size=40`, 16x16, placed before the text.</li></ul> |
| List cell, GitLab | 5 | <ul><li>One connection shows `Mobile app` with title `Mobile app`.</li><li>Two connections show `2 connections` with title `Mobile app, Docs site`.</li><li>`[]` and an absent field (older server) both show `GitLab` with title `GitLab`.</li><li>In every case there is no image, the GitLab descriptor's icon comes before the text, and the installation login `octo-org` appears nowhere in the row.</li><li>Partial data (wireframe B4): a nameless connection shows `baseUrl/scopePath`.</li></ul> |
| List cell, unknown scope | 1 | The cell shows `—` with title `Unknown destination`. It has no image and no GitLab icon, and the row shows neither the account login nor the raw scope. |
| Detail, GitHub and Bitbucket (**invariant guards**) | 4 | <ul><li>A repository pool shows the rows `Scope: Repository` and `Account: acme`, the GitHub avatar, and the chips `acme/web`, `acme/api` with no toggle.</li><li>A pool with nine repositories shows six chips and `Show 3 more` (`aria-expanded=false`). Clicking it shows nine chips and `Show fewer` (`aria-expanded=true`).</li><li>An organization pool shows `Runner group: #7`. A Bitbucket pool with workspaces shows `Runner group: #1`.</li><li>In both, Scope shows `Organization`, and Account shows the avatar plus `acme`.</li></ul> |
| Detail, GitLab | 5 | <ul><li>The only property row is `Scope: GitLab`.</li><li>One connection shows its name and `https://gitlab.com/acme/platform/mobile-app`.</li><li>Two connections keep the **server's** order (Platform group, then Mobile app), with each name before its own address and no toggle.</li><li>`[]` and an absent field both show `No connections listed`.</li><li>In every case there is no image, no `octo-org` and no `Runner group`.</li><li>Partial data (D5): an empty `scopePath` shows `https://gitlab.com` with no trailing `/`, and a nameless connection's address appears exactly once.</li></ul> |
| Detail, unknown scope | 1 | The only property row is `Scope: Unknown`. There is no image, no account, no runner group and no `azure-devops`. |

Notes on how the tests read the DOM:
- **What "shown" means.** The `capitalize` CSS class is applied, so today's `<span class="capitalize">repository</span>` reads `Repository`. GitLab's must read `GitLab`; today it reads `Gitlab`.
- **The destination cell** is the row's one titled element that is not the capacity meter and not an `img` or `svg`.
- **The GitLab icon** is identified by the `lucide-*` class of `getPlatform("gitlab").icon` when rendered. The class name is not hard-coded.
- **Property rows** are the group's `dt`s, read as `label: shown value`. The group is the `PropertyGroup` titled `Destination`, which stays in the route (wireframe C4).

### `tests/platform-neutral-source.test.ts` (extended; source guard)
- `SHARED` gains `src/routes/_app/runner-pools.index.tsx` and `src/routes/_app/runner-pools.$poolId.index.tsx`.
- `BRANCHING` gains `/\b(githubAvatar|repositoryCount|bitbucketConnections|gitlabConnections)\b/`. The two form routes still pass with it.
- New test: each of the two routes imports from `~/features/platform-connections/registry`.
- The "GitLab code has one home (#141)" tests are unchanged, so `gitlab.tsx` stays GitLab's only home and the routes must not name GitLab.

Why a source guard: the brief's Done-when names this file and these properties.

### `crates/gridops-api/src/resources/gitlab_tests.rs` (one new integration test)
`the_pool_list_carries_each_pools_gitlab_connections_in_detail_order_and_none_for_a_github_pool`
- **Setup.**
  - Two seeded connections, on different hosts and of different kinds.
  - A GitLab pool whose memberships were added in reverse id order (created_at 20 and 10). A second GitLab pool with one connection. An organization pool.
- **Assertions.**
  - `GET /api/v1/runner-pools` gives the first pool `gitlabConnections == [CONNECTION_2, CONNECTION_1]`, each as `{id, name, baseUrl, scopeKind, scopePath}`. This is membership order, not id order, and it equals the detail endpoint's `gitlabConnections`.
  - The second pool gets only its own `[CONNECTION_1]`, so a page-wide batch must group per pool.
  - The organization pool gets `[]` (the key is present).
- "Single batched query" is structural, as A asked: one load over the page's ids. The test proves that the per-pool grouping and the order are correct. It does not count queries.

Why integration: the field comes from SQL in a real handler.

## RED confirmation

**Vitest, full suite** (`npx vitest run`):
```
 Test Files  2 failed | 50 passed (52)
      Tests  26 failed | 556 passed (582)
```
Every existing test still passes. The 8 extra passes are the GitHub and Bitbucket invariant guards, which are meant to stay green. The 26 failures are all new:

| File | Failing | Representative failure (the feature's own assertion) |
|---|---|---|
| pool-destination | 10 registry | `exports platformForPool…: expected 'undefined' to be 'function'`; `resolves … to its owner: expected undefined to be { id: 'github', … }` / `{ id: 'gitlab', … }` |
| pool-destination | 6 list | `expected 'octo-org' to be 'Mobile app'` / `'2 connections'` / `'GitLab'` / `'https://gitlab.com/acme/platform/mobi…'`: today a GitLab row shows the installation's GitHub login. `expected 'octo-org/app' to be '—'`: an unknown scope shows the repository. |
| pool-destination | 6 detail | `expected [ 'Scope: Gitlab', …(2) ] to deeply equal [ 'Scope: GitLab' ]`: today it shows "Gitlab", Account and Runner group. `expected [ 'Scope: Azure-Devops', …(2) ] to deeply equal [ 'Scope: Unknown' ]`. `expected 'DestinationScopegitlabAccountocto-org…' to contain 'Mobile app'`. |
| platform-neutral-source | 4 | List route: `10: import { Avatar, githubAvatar }…`, `81: const destination = pool.scope === "repository" && pool.repositoryCount > 1 …`, `98: <Avatar … src={githubAvatar(pool.accountLogin)} />`. Detail route: lines 11, 128 (`{pool.scope}`), 129 (`githubAvatar`), 130 (`pool.scope === "repository"`). Both: `expected 'import { BUILTIN_HOST_OFF_LABEL, …' to match /from "~\/features\/platform-…/`. |

No test fails on an import, a typo or setup:
- the routes render, and the GitHub rows pass against them;
- `platformForPool` is read through a namespace import, so its absence is an assertion failure, not a crash.

**Rust** (`CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api the_pool_list_carries`):
```
panicked at crates/gridops-api/src/resources/gitlab_tests.rs:2153:5:
assertion `left == right` failed: {"accountCount":1,"accountLogin":"fixture",…,"id":"c1000000-0000-4000-8000-000000000001",…,"scope":"gitlab",…}
  left: None
 right: Some(Array [Object {…"id": String("a1000000-0000-4000-8000-000000000002")…}, Object {…"id": String("a1000000-0000-4000-8000-000000000001")…}])
test result: FAILED. 0 passed; 1 failed
```
The harness, the seeding and the request all work, and the GitLab pool is listed. The failure is the missing `gitlabConnections` key.

**Satisfiability (sketch, then thrown away).**
- **Frontend.**
  - In a scratch copy of `src/` (outside the worktree), I wrote a throwaway implementation:
    - `platformForPool`;
    - per-platform `{ scopes, destination, Destination }` hooks;
    - a registry fallback;
    - both routes calling the hooks with no branching.
  - Result: all 30 pool-destination tests and the 4 new source-guard tests passed.
  - The only failures were the two "GitLab has one home" tests. They failed because the sketch put the GitLab hooks in a scratch module instead of `gitlab.tsx`, which is the guard working as intended.
- **Backend.**
  - In the worktree, I added a throwaway `pools_gitlab_connections` with a `json_each(?)` batch ordered by `pool_id, created_at, connection.id`, and the `runner_pools` insert.
  - Result: the Rust test passed (`1 passed`).
  - I then reverted both source files with `git checkout --`. `git status` shows only the three test files.

**Static checks on the test files:**
- `npx eslint tests --max-warnings=0`: clean.
- `npx tsc --noEmit`: no error under `tests/`. The fixtures cast through `RunnerPool` / `RunnerPoolDetail`, so they compile both before and after F adds `RunnerPool.gitlabConnections`.
- `cargo fmt --check -p gridops-api`: exits 0.
- `cargo clippy -p gridops-api --all-targets -- -D warnings -A clippy::unused_async_trait_impl`: no finding in `gitlab_tests.rs`.
  - The local clippy reports `unused_async_trait_impl` (host-agent, api `fleet/boundary.rs`).
  - It also reports findings in `overview_queue.rs` and `tests/fleet_http/{builtin_host_budget,recovery,start_retry}.rs`.
  - All of these predate this change and lie outside #159, as #141's T also recorded.

## Contract notes for F
**Pinned:**
- `registry.tsx` exports `platformForPool(pool)`:
  - it accepts both a `RunnerPool` and a `RunnerPoolDetail`;
  - it returns the **same object** as `getPlatform("github")` / `getPlatform("gitlab")` for an owned scope;
  - for an unknown scope it returns a fallback that is not in `platforms` and never throws.
- Both routes import from `~/features/platform-connections/registry`. Neither contains `pool.scope`, a `scope ===` comparison, a quoted platform or kind id, `githubAvatar`, `repositoryCount`, `bitbucketConnections` or `gitlabConnections`.
- Wire field: list items carry `gitlabConnections`, the detail's projection and order, with `[]` for none.

**List cell DOM:**
- The cell stays the row's one titled element besides the capacity meter.
- It holds the leading mark (avatar `img` / icon `svg`) **before** the text.
- GitHub's avatar stays `<Avatar size={16} square src={githubAvatar(login)}>`.
- GitLab's mark is the descriptor's `icon` (any `className` is fine).
- The unknown-scope placeholder must be neither an `img` nor the GitLab icon.

**Detail DOM:**
- The rows stay inside the route's `PropertyGroup title="Destination"`.
- Scope and the other rows are `PropertyRow`s (`dt`/`dd`).
- GitHub's repository chips stay `div[title=fullName]`, and the toggle stays the group's only `button`.
- GitLab's connection items must **not** be `PropertyRow`s, because only `Scope` may be a `dt`. They also add no `button` and no `img`.
- Copy: `GitLab`, `No connections listed`, `Unknown`, and in the list `—` / `Unknown destination` / `GitLab` / `N connections`, with names joined by `, ` in the title.

**Left open:**
- the descriptor key's name and shape (A's warn 1 suggests one optional `pool` key);
- where `RepositoryList` lives inside `github.tsx`;
- the Rust helper's name;
- the shared column constant;
- the exact markup of a GitLab connection item, beyond its order and its "name before address" text.

## Ready for F
**RED is valid.** F may implement against these tests.

Run them with:
- `npx vitest run tests/pool-destination.test.ts tests/platform-neutral-source.test.ts` (and the full `npx vitest run`)
- `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api the_pool_list_carries`

The three test files are staged by explicit path.

**Advisory, for O:** not part of #159 and not caused by it.
- In this test's response, a seeded pool with **no runners** lists `"totalRunners":1`. The `runner_pools` query `LEFT JOIN`s `runners`, so `COUNT(CASE WHEN r.deleted_at IS NULL THEN 1 END)` counts the null-extended row.
- `totalRunners` does not appear to drive the list UI (the row uses online/busy/desired), so this is not a blocker. It may deserve its own issue.
