
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
