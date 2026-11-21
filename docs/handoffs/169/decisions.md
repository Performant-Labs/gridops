
## Phase 2 Design (D) - 2026-11-21
- **Decided:** wireframe at docs/handoffs/169/wireframe.html covers runner rows (right-hand column with title, narrow line, details-sheet row) for GitHub repository, GitHub organization, Bitbucket on a GitHub pool, older server with no `poolScope`, GitLab (name, address, one address part, neither) and unknown scope; and the dashboard pool row scope text for repository, organization, gitlab and unknown. GitHub, Bitbucket and older-server frames are the unchanged baseline. `totalRunners` is API only and has no drawn UI.
- **Assumed:** the details-sheet label stays "Repository" for every platform (the hook returns only text and title, and GitHub output must not change); unknown scope shows "—" / "Unknown destination" / "Unknown"; a GitLab address with one part shows that part alone.
- **Hedged:** the label choice is open question 1 in handoff-D.md; renaming it is a copy decision that does not block tests or code.
- **Evidence:** headless chrome-headless-shell render, full page and two zoomed crops inspected; one unreadable tag in the dashboard frame was fixed and re-rendered. No survey.md exists for this run; the brief and the source files (`runner-row.tsx`, `index.tsx` PoolsSection, `registry.tsx`, `types.ts`) stood in for it.

## Phase 3 Architecture (A, up-front plan review) - 2026-11-21T02:44:00-08:00
- **Decided:** PASS, with 0 block and 5 warn findings (docs/handoffs/169/handoff-A.md). The plan extends #159's `PoolBindings` key and `platformForPool`, adds two columns to the existing `runners` handler (the source for both `RunnerRow` consumers), and fixes `total_runners` in place; no parallel path. The warns:
  - (1) the absent-`poolScope` default belongs in `registry.tsx`, mirroring `platformForDeliveryId`, and `BRANCHING` gains a quoted scope-id pattern;
  - (2) both files join `SHARED`, with receiver-qualified patterns, because `index.tsx:378/380` and `runner-row.tsx:108` legitimately say "repository";
  - (3) reuse `connectionAddress`/`connectionLabel` in `gitlab.tsx`;
  - (4) `runnerDestination`/`scopeLabel` are required members of `PoolBindings`, with a named return type;
  - (5) test homes are `resources/gitlab_tests.rs` (its harness) and `tests/runner-row-platform.test.ts`.
- **Assumed:** `docs/handoffs/169/wireframe.html` is approved as drawn (the D approval line is still empty); there is no `survey.md`, so the brief's named analogues stood in for the Reuse map.
- **Hedged:** none. The `totalRunners` sweep was checked rather than assumed: only `resources.rs:1329` counts the NULL row.
- **Evidence:** read the brief, the brief gate result, handoff-D, the #159 handoff-A, `registry.tsx`, `types.ts`, `github.tsx`, `gitlab.tsx`, `runner-row.tsx`, `routes/_app/index.tsx`, `operations.functions.ts` (`Runner`, `RunnerPool`), `resources.rs` (`runners` and `runner_pools` SQL, and every `LEFT JOIN runners` count), `migrations/0047_gitlab.sql`, `tests/platform-neutral-source.test.ts`, `tests/pool-destination.test.ts`, `tests/runner-row-platform.test.ts` and `gitlab_tests.rs:2025-2092`. Grepped the shared files for quoted scope ids (none) and checked every other branch's diff for overlap (none live). `git diff origin/master HEAD -- src tests crates` is empty.

## Phase 4 Tests RED (T) - 2026-11-21T02:55:26-08:00
- **Decided:** RED is valid. 15 tests fail on the feature assertion: 13 Vitest and 2 Rust (docs/handoffs/169/handoff-T-red.md). Every test sits in the home A named:
  - the runners-list `poolScope`/`gitlabConnectionName` test and the `totalRunners` 0 / 2 / live-only test are in `gitlab_tests.rs`;
  - the runner-row destination matrix (column text and title, narrow line, details row; 11 cases) is in `tests/runner-row-platform.test.ts`;
  - the Overview scope-label test is new, in `tests/dashboard-pool-scope.test.ts`, and renders through `Route.options.component`;
  - both files join `SHARED`, with a quoted scope-id pattern, a `runner|pool.(repository|accountLogin)` pattern, a `poolScope`-only-in-`platformForPool(` line rule and a `platformForPool` import/call assertion.
- **Assumed:** the GitLab address text keeps the scheme (`https://gitlab.example.com/acme/mobile`). The brief says `gitlabBaseUrl/gitlabScopePath` and A says to reuse `connectionAddress`, which strips only trailing slashes. The wireframe's scheme-less drawing is low-fi shorthand. The details-sheet label stays "Repository" (D open question 1).
- **Hedged:** the GitHub, Bitbucket, older-server and repository/organization cases pass today on purpose. They guard "exactly today's output" and are not part of the RED set. The narrow-line job-name test is RED only through its column assertion.
- **Evidence:**
  - full `npx vitest run`: 13 failed, 589 passed, and only the intended 13 fail;
  - `cargo test -p gridops-api --bins` filtered to the two tests: 2 failed (`poolScope` None ≠ "gitlab"; `totalRunners` 1 ≠ 0);
  - eslint, tsc (with the generated route tree) and `cargo fmt --check` are clean on the tests;
  - clippy is clean for `gitlab_tests.rs`. Two lints new in toolchain 1.99.1 (`unused-async-trait-impl`, `assert-is-empty`) already fail in untouched files.
