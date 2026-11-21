
## Phase 2 Design (D) - 2026-11-21
- **Decided:** wireframe at docs/handoffs/159/wireframe.html covers list and detail destination for GitHub repo (1, many), GitHub org, Bitbucket, GitLab (1, 2, none) and unknown scope. GitHub and Bitbucket frames are the unchanged baseline.
- **Assumed:** unknown scope shows "—" in the list and Scope "Unknown" in the detail; GitLab empty state shows "GitLab" (list) and "No connections listed" (detail); partial connection data shows only present parts; all GitLab connections listed without a toggle.
- **Hedged:** these copy choices are listed as open questions in handoff-D.md; none blocks tests or code.
- **Evidence:** headless render screenshots inspected (frames A, B re-inspected after a tooltip fix; C-E once); GL icon is a stand-in, real lucide icon not rendered.

## Phase 3 Architecture review (A, up-front plan review) - 2026-11-21T00:56:00-08:00
- **Decided:** PASS, with five warns and no blocks. The plan extends the #145 descriptor contract and the registry (a `platformForPool` resolver beside `getPlatform` and `platformForDeliveryId`) and adds one batched helper beside `pool_gitlab_connections`. Moving `RepositoryList` to `github.tsx` is a refactor-to-extend that the brief names. The warns:
  1. Group the scope declaration and both renderers under one optional descriptor key, with the detail renderer as a JSX-rendered component.
  2. The registry owns the unknown-scope fallback copy and never echoes the raw scope.
  3. Use the `json_each(?)` batch pattern, share the detail's column list and order, and test in `gitlab_tests.rs` (no query counter).
  4. Extend `SHARED`/`BRANCHING` in the source-guard test, not a blanket `/github/i` (the list route's GitHub empty-state copy is out of scope).
  5. The runner-row sibling (`runner-row.tsx:46, :109, :127`) and the dashboard's raw scope label (`routes/_app/index.tsx:279`) are still untracked: add an Out of scope line and file a follow-up.
- **Assumed:** The brief's Scope section stands in for the missing `docs/handoffs/159/survey.md` Reuse map, as in #141. The wireframe's copy choices are approved as drawn: handoff-D's approval line is still unfilled.
- **Hedged:** Warn 5 has now been raised three times (#141 A round 3, #141 S, here). Recording it is O's call, not a block on this plan's architecture.
- **Evidence:** Read the brief, the brief gate result r1, handoff-D, the wireframe (as text), `types.ts`, `registry.tsx`, `github.tsx`, `gitlab.tsx`, `bitbucket.tsx`, both routes, `targets.ts:146-152`, `operations.functions.ts:1-35`, `runner-pools.functions.ts:1-80`, `tests/platform-neutral-source.test.ts`, `resources.rs:800-834, :1292-1480, :2067-2104`, `resources/gitlab.rs:370-565`, and `gitlab_tests.rs` (helper index); the OpenAPI `/api/v1/runner-pools` entry; `gh issue view 159` and the open-issue list; and the worktree-branch diffs for coexistence. `git diff origin/master HEAD -- src tests crates` is empty (no code yet).

## Phase 4 Tests RED (T) - 2026-11-21T01:10:31-08:00
- **Decided:** RED is valid. The tests:
  - `tests/pool-destination.test.ts`, 30 jsdom tests through both rendered routes with the production registry: 22 RED, plus 8 GitHub/Bitbucket invariant guards that are green before and after;
  - `tests/platform-neutral-source.test.ts`, where both routes join SHARED, BRANCHING gains `githubAvatar|repositoryCount|bitbucketConnections|gitlabConnections`, and each route has a registry-import check (4 RED);
  - one Rust integration test in `gitlab_tests.rs` (RED: `gitlabConnections` is absent from the list item).

  The only name pinned is `platformForPool`, from the brief's example and A's resolver. The views are tested by what they render, so the hook names stay F's choice.
- **Assumed:** the wireframe's copy defaults are approved, as A recorded, even though handoff-D's approval line is unfilled. That copy is "—"/"Unknown destination", "GitLab", "N connections", "No connections listed", Scope "Unknown", and the partial-data rules. Bitbucket list items are GitHub-scoped `organization` pools (`RunnerPool` carries no Bitbucket fields).
- **Hedged:** "single batched query" is pinned structurally (two GitLab pools are each grouped correctly from one page load) and not by counting queries, per A's warn 3. Satisfiability was checked with throwaway sketches: the frontend in a scratch copy outside the worktree, and the backend in the worktree, reverted with `git checkout --`. All new tests passed against the sketches except the "GitLab has one home" guard, which tripped only because the sketch put the GitLab code outside `gitlab.tsx`.
- **Evidence:** the baseline (`npx vitest run`) was 548/548. After this phase: 26 failed and 556 passed, and every failure is a new test. Rust: `left: None` for `gitlabConnections` at gitlab_tests.rs:2153. eslint on tests is clean, `tsc` shows no error under tests/, `cargo fmt --check` passes, and clippy shows no finding in gitlab_tests.rs (the other clippy findings predate this change). Advisory for O: `runner_pools` reports `totalRunners: 1` for a pool with no runners (LEFT JOIN count). This predates #159 and is not in scope.

## Phase 6 Implementation (F) - 2026-11-21T01:18:20-08:00
- **Decided:** One optional `pool?: { scopes, destination, Destination }` key on `PlatformDescriptor` (A warn 1).
  - GitHub owns `repository`/`organization`, GitLab owns `gitlab`, and Bitbucket declares none.
  - `platformForPool` returns a descriptor whose `pool` is always present. For an unknown scope that is a registry-owned fallback ("—"/"Unknown destination"/placeholder; Scope "Unknown"), which never reads `name` and so never echoes the raw scope (A warn 2).
  - Both routes call the hooks unconditionally, and `RepositoryList` moved to `github.tsx`.
  - API: `pools_gitlab_connections` is one `json_each(?)` query sharing the detail's SELECT/JOIN and ORDER constants, and `runner_pools` adds `gitlabConnections` (`[]` when a pool has none).
- **Assumed:** The scope cap does not apply. The change spans 9 files on two surfaces, but the brief, A and T planned it as one unit, T's RED spans both layers, and it is one defect, not a polish batch. The GitLab detail chip's `title` (name, then address) is decorative and the tests do not read it.
- **Hedged:** One `fleet_http` test (`release_preflight…without_a_macos_arm64_machine`) fails locally because the fixture agent reports the host's compile OS (macOS here). It predates this change and is unrelated: the preflight script never reads `/runner-pools`. The clippy findings are the pre-existing set T recorded.
- **Evidence:**
  - vitest: 52 files, 582/582 (targeted: 45/45).
  - eslint: clean.
  - `npm run build` + tsc: exit 0.
  - `cargo test -p gridops-api the_pool_list_carries`: 1 passed. Full `gridops-api` suite: 123 + 36 + 143 passed, plus the 1 environment-dependent failure above.
  - `cargo fmt --check`: exit 0.
  - clippy: no finding in the changed Rust files.

## Phase 7 Verify (T) - 2026-11-21
- **Decided:** GREEN, no blocking Tier 2 issue. No test needed repair (F flagged none).
- **Assumed:** the `release_preflight` failure is environmental (fixture reports compile OS on a Mac), not a #159 regression, because the script has no reference to `/runner-pools` and the diff does not touch preflight or fleet code.
- **Hedged:** "single batched query" is verified by reading `pools_gitlab_connections` (one `json_each(?)` query, early return on empty page), not by a query counter, as A agreed. The live UI path is left to U.
- **Evidence:** vitest 52 files / 582 passed; eslint clean; `npm run build` + tsc exit 0; cargo fmt exit 0; `cargo test -p gridops-api` 123 + 36 + 143 passed, 1 pre-existing environment failure; clippy shows no finding in the changed Rust files.

## Phase 7 Anti-duplication (A) - 2026-11-21
- **Decided:** PASS. F extended `PlatformDescriptor`, the registry and `resources/gitlab.rs`; no parallel path, and `RepositoryList` moved as the brief's named refactor-to-extend.
- **Assumed:** two similar chip renderings (GitHub `RepositoryList`, GitLab connection chip) are not duplication: they are in separate descriptors with different shapes.
- **Hedged:** the runner-row and dashboard siblings (A Phase 3 finding 5) remain open and untracked by this diff; recorded as warn for O.
- **Evidence:** `git diff e022a5d HEAD` over src, crates and tests; handoff-F.md; handoff-A.md.

## Phase 8 UI Walkthrough (U) - 2026-11-21
- **Decided:** PASS. All 8 brief contract-table cases (GitHub repo one/many, GitHub org, Bitbucket, GitLab one/two/zero connections, unknown scope) were independently reproduced live in a headless browser, in both `/runner-pools` and a pool's detail page, on SPA-nav and hard-load paths, at 1280px and 360px, with zero console errors/warnings throughout.
- **Assumed:** GridOps has no e2e seed script and login is GitHub-OAuth-only (same conclusion #141/#145's U rounds reached), so this pass built the real API against a throwaway SQLite DB it seeded directly, and minted a session cookie with the same HMAC scheme `auth.rs` uses, rather than mocking the API. `u-drive.mjs` was not used (it assumes an HTMX login form and HTMX swap timing; this is a React/TanStack SPA with no HTMX), matching the #141 U rounds' documented reasoning.
- **Hedged:** The pre-existing `RepositoryList` "Show N more" toggle (unchanged by #159, moved verbatim into `github.tsx`) was not independently re-exercised — the seed's many-repo pool has 3 repos, below the toggle's threshold, and F's handoff states the toggle logic is untouched. Noted as a minor gap, not a #159 regression. One pre-existing, out-of-scope note: the list's destination column is hidden below the `xl` (1280px) breakpoint by a Tailwind class #159 did not touch, so it is invisible on the list at 360/768/1024px (visible only on the detail page) — predates #159, not a defect of this change.
- **Evidence:** `docs/handoffs/159/handoff-U.md` (full state matrix, live DOM captures, console logs); screenshots `docs/handoffs/159/screenshots/u-list-1280.png` and `u-detail-gitlab-two-360.png`; a cross-check `curl` of `GET /api/v1/runner-pools` against the same seeded DB confirming the API's `gitlabConnections` ordering/emptiness matches what the UI rendered.
