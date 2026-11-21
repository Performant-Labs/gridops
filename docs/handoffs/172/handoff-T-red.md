# Handoff-T-red: Phase 4 - GitLab Docker pool default image and no runner group (#172)

**Date:** 2026-11-21
**Branch:** claude/gitlab-172-pool-image-bb7d36 (head 98a3858)
**Brief / wireframe reviewed:** docs/handoffs/172-brief.md (amended, 7afb7c5), docs/handoffs/172/wireframe.html (revised, 2c1abdf), docs/handoffs/172/handoff-A.md (round 2), handoff-D.md, decisions.md

## A precondition

Confirmed: A returned **PASS** in round 2 (`handoff-A.md`, commit 98a3858), with three warnings. The tests pin each warning's suggested fix, because A asked T to:

- **Warning 1** (helper placement): `defaultDockerImage(descriptors, selection)` is exported from `src/features/runner-pools/targets.ts`, and it is tested with a fixture registry. A source check also asserts that `targets.ts` never imports the registry. The settings and detail routes are tested only through what they render, so whether they read `platformForPool(pool).defaultDockerImage` is up to F.
- **Warning 2** (order of the checks): validation runs after `pool_access` (a missing pool is a 404) and before `assert_pool_admin`. A reader who is not an admin still gets a 400 for an invalid body, and a 403 for a valid one (the control).
- **Warning 3** (action contract): `parseUpdateRunnerPoolInput(values, { dockerImageOptional })` is tested directly. The settings form test asserts that the PUT body never carries `dockerImageOptional`.

## Contract F implements against (the names the tests pin)

- Rust: `UpdateRunnerPool::validate(&self, gitlab: bool)`. `CreateRunnerPool::validate(&self)` keeps its signature. `runner_group_id` is `#[serde(default)]` on both input structs.
- TS descriptor: `PlatformDescriptor.defaultDockerImage?: string`. GitLab's is `"gitlab/gitlab-runner:ubuntu-v19.6.0"`.
- TS helper: `defaultDockerImage(descriptors, selection)`, exported from `src/features/runner-pools/targets.ts`. It returns the image, or a falsy value when no targeted platform declares one.
- TS schema: `parseUpdateRunnerPoolInput(input, { dockerImageOptional?: boolean })`. Leaving the option out means `false`. `updateRunnerPoolSchema` keeps the GitHub rule.
- Copy, from the wireframe: help `Leave empty to use gitlab/gitlab-runner:ubuntu-v19.6.0.`, placeholder `gitlab/gitlab-runner:ubuntu-v19.6.0`, display `Default (gitlab/gitlab-runner:ubuntu-v19.6.0)`. The GitHub help stays `OCI image for each Linux runner.`
- **An existing pin F must keep:** `tests/pool-target-payloads.test.ts:394,402` deep-equals the data the settings form passes to `updateRunnerPoolAction` for GitHub pools, using `toEqual`, so a key whose value is `undefined` does not count. For a pool with no default image, the data must therefore carry no `dockerImageOptional`, or carry it as `undefined`, never `false`. My throwaway reference first passed `false` and these two tests failed. With `platformImage ? true : undefined`, they pass.

## Tests authored

The tiers: the API rules are tested through the real routers (`gitlab_tests.rs`), because the handler decides the scope, the stored values and the status precedence. The schema rules are tested at unit level. The form behaviour is tested in jsdom through the rendered routes with the production registry, which is the cheapest tier where the descriptor and the route wiring actually run. Platform neutrality is checked at source level, following the existing pattern.

### `crates/gridops-api/src/resources/gitlab_tests.rs` (5 new tests, in a new #172 section at the end)

| Test | Criterion pinned | RED? |
|---|---|---|
| `a_gitlab_docker_pool_is_created_with_no_image_and_no_runner_group_and_runs_the_gitlab_runner_image` | A create with `scope: "gitlab"`, `provider: "docker"` and an empty `image`/`dockerImage` gives 201, with `runnerGroupId` 0, left out or -4. `image` and `docker_image` are stored empty, `runner_group_id` is stored as 1, and `docker_runner_image(stored, true) == GITLAB_RUNNER_IMAGE`. | RED |
| `a_gitlab_pool_is_updated_with_no_image_and_no_runner_group` | A PUT to a GitLab pool with an empty image and runner group 0 or left out gives 200. The image is saved empty and the group stays 1. | RED |
| `a_gitlab_pools_docker_image_is_still_checked_when_one_is_given` | A padded image on a GitLab pool gives the 400 `Docker image must contain 1-300 non-padding characters.`, on both create and update, and stores nothing. | Guard (passes today; it fails if F skips `valid_image` for GitLab) |
| `a_github_pool_keeps_the_docker_image_and_runner_group_400s_on_create_and_update` | For `repository` and `organization` pools, on create and on update, an empty image gives exactly the Docker-image 400 and `runnerGroupId: 0` gives exactly the runner-group 400, and nothing is stored. **Also:** an organization create with no `runnerGroupId` gives the 400 `Runner group ID must be positive.` (the accepted change from serde's 422). | RED on the 422→400 case. The rest is a guard. |
| `an_invalid_update_is_a_404_for_a_missing_pool_and_still_a_400_for_a_pool_the_caller_only_reads` | An invalid body sent to a missing pool gives 404 (the accepted precedence change). Per A's warning 2, a read-only member's invalid body still gives 400, and the valid-body control gives 403. | RED on the 404 case |

The test also adds the helpers `with_changes`, `pool_images`, `error_message` and `github_pool_body`, plus the const `REPOSITORY_POOL`/`MISSING_POOL`. I updated one stale comment in `gitlab_pool_body`: it now says any runner group value is accepted for a GitLab pool.

### Callers updated for the decided `UpdateRunnerPool::validate(gitlab: bool)` seam (test code only)

- `crates/gridops-core/tests/gitlab_pools.rs`: `validate_update` passes `true`, because every body there is a GitLab pool update. It has a doc comment.
- `crates/gridops-core/src/models.rs`: the **`#[cfg(test)] mod tests` block only**. I changed its 13 `UpdateRunnerPool` calls to `.validate(false)`, because they are all GitHub repository pool updates. No production line in this file changed: `git diff` shows only `.validate()`→`.validate(false)` inside the test module. F writes no tests, so I updated these callers here and F does not need to touch them.

### `tests/gitlab-pool-schema.test.ts` (+4 tests, describe "a GitLab pool's Docker image is optional (#172)")

| Test | Pins | RED? |
|---|---|---|
| create accepts a gitlab-scoped pool with an empty Docker image and keeps it empty | `createRunnerPoolSchema` derives the rule from `scope` | RED |
| create still rejects a repository / an organization pool's empty Docker image with today's message (2 cases) | the GitHub rule, path `dockerImage` | Guard |
| update accepts an empty Docker image only when the caller says the image is optional | `parseUpdateRunnerPoolInput(.., { dockerImageOptional: true })` accepts it. With `false`, or with the option left out, it throws today's message. | RED |
| the exported update schema keeps the GitHub rule | `updateRunnerPoolSchema` is unchanged | Guard |

### `tests/gitlab-pool-default-image.test.ts` (new, jsdom, 12 tests)

| Test | Pins (wireframe frame) | RED? |
|---|---|---|
| descriptor: is the server's GITLAB_RUNNER_IMAGE, so the frontend's copy cannot drift | reads `provisioning.rs` with `readFileSync`, and `getPlatform("gitlab").defaultDockerImage` equals the constant | RED |
| descriptor: is resolved from a new pool's selection by a pure helper over the descriptors it is given | `defaultDockerImage(platforms, …)`: falsy for none or GitHub, the image for a GitLab project, and it honours a **fixture** descriptor list (A warning 1) | RED (missing export) |
| new pool: required and prefilled with the GitHub default before any target is chosen | 1a | Guard |
| new pool: optional, empty and names the GitLab default for a GitLab target, and an empty image is sent empty | 1b: `required` false, value `""`, the placeholder, the help, and a POST with `image: ""` and `dockerImage: ""` | RED |
| new pool: switching back to GitHub restores today's row, and a typed image survives switching either way | 1a/1c/1d | RED (its first GitLab assertion; the rest guards against a naive reset-on-switch) |
| settings: optional with the default named, saves empty, and never sends the optional flag in the body | 2a, plus A warning 3 | RED |
| settings: a GitHub pool's row is unchanged | 2c, and its PUT carries no `dockerImageOptional` | Guard |
| settings: the read-only view shows the GitLab default for an empty image, and any stored image verbatim | 3a/3b | RED |
| detail: shows the GitLab default for a GitLab pool with an empty image, as text and title | 3c | RED |
| detail: shows a stored image verbatim | 3d | Guard |
| fleet: is the raw GitLab runner image for a GitLab pool with an empty image, with no hint | 2d: "Add platform" starts from the raw image, never the label or the GitHub image | RED |
| fleet: is the pool's own image for a GitHub pool | 2e | Guard |

(I dropped a draft test, "GitHub/Bitbucket declare no default", because the helper test and the GitHub form guards already cover it.)

### `tests/platform-neutral-source.test.ts` (extended)

- `src/features/fleet/fleet-profile-section.tsx` joins `SHARED`, the list of files that may compare against no platform, kind or scope id.
- New describe "the default Docker image is the descriptor's (#172)":
  - the body of `validateProvider` names no platform and no `scope`;
  - `schemas.ts` imports nothing from `platform-connections/`, which would create an import cycle;
  - `targets.ts` never imports the registry.

These are all guards and pass today. The existing "GitLab code has one home (#141)" test also guards the literal: the image string may live only in `gitlab.tsx`.

## RED confirmation

**Rust API** (`CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api --bin gridops-api -- resources::gitlab_tests`): **35 passed, 4 failed**. The 4 failures are the new RED tests, and each fails on its feature assertion:

    a_github_pool_keeps_…_on_create_and_update  (gitlab_tests.rs:2502)
      create an organization pool without runnerGroupId: "…missing field `runnerGroupId`…"
      left: (422, "")   right: (400, "Runner group ID must be positive.")
    a_gitlab_docker_pool_is_created_with_no_image_…  (:2381)
      gitlab-group-zero: {"error":"Docker image (dockerImage) is required for a pool that uses Linux · Docker."}
      left: 400   right: 201
    a_gitlab_pool_is_updated_with_no_image_and_no_runner_group  (:2413)
      runner group 0: {"error":"Docker image (dockerImage) is required …"}
      left: 400   right: 200
    an_invalid_update_is_a_404_for_a_missing_pool_…  (:2555)
      {"error":"Use 2-48 lowercase letters, numbers, and hyphens for the pool name."}
      left: 400   right: 404

I put the `runnerGroupId: 0` case first in both GitLab loops on purpose, so the RED shows the image rule failing. The serde-default rule fails separately, in the GitHub test, and on the left-out case behind it.

**Rust core (compile RED, the decided seam):** `cargo test -p gridops-core --lib --no-run` gives 13 × `error[E0061]: this method takes 0 arguments but 1 argument was supplied` (models.rs:780–942, the test module, against `models.rs:302`). `cargo test -p gridops-core --test gitlab_pools --no-run` gives the same error at `gitlab_pools.rs:71`. **What F should expect:** core's lib tests and `gitlab_pools` do not compile until `UpdateRunnerPool::validate` takes `gitlab: bool`.

**Frontend** (`npx vitest run`): **10 failed / 613 passed (54 files)**. Only the new RED tests fail:

    descriptor equals GITLAB_RUNNER_IMAGE        expected undefined to be 'gitlab/gitlab-runner:ubuntu-v19.6.0'
    pure helper                                   TypeError: defaultDockerImage is not a function   (the export does not exist yet)
    new pool: GitLab target optional …            optional: expected true to be false
    new pool: switching …                         expected 'runner:default' to be ''
    settings: optional … saves empty              optional: expected true to be false
    settings: read-only shows the default         expected '' to be 'Default (gitlab/gitlab-runner:ubuntu-v19.6.0)'
    detail: shows the default                     expected { text: '', title: '' } to deeply equal { text/title: 'Default (…)' }
    fleet: raw GitLab runner image                expected 'ghcr.io/actions/actions-runner:latest' to be 'gitlab/gitlab-runner:ubuntu-v19.6.0'
    schema: create accepts gitlab empty image     [{"path":["dockerImage"],"message":"Docker image (dockerImage) is required …"}]: expected false to be true
    schema: update with dockerImageOptional       Error: Docker image (dockerImage) is required for a pool that uses Linux · Docker.

`npx tsc --noEmit` fails only on the new API surface: TS2305, no export `defaultDockerImage`; TS2339, `defaultDockerImage` is not on `PlatformDescriptor`; TS2554 ×2, `parseUpdateRunnerPoolInput` takes 1 argument. `npx eslint` on the three changed test files is clean. Typecheck needs the gitignored `src/routeTree.gen.ts`. I generated it in this worktree with `@tanstack/router-generator`. It is not tracked.

### The tests can be passed: throwaway reference implementations, never staged

To prove the tests are not contradictory, I wrote a minimal reference implementation of the brief, ran the tests against it, and then reverted it. Both patches are kept outside the repo, in the session scratchpad.

- **Frontend reference:** descriptor key, `targets.ts` helper and label composer, `validateProvider(…, dockerImageOptional)`, the update-schema factory, the action destructuring `dockerImageOptional`, and the three routes. Results:
  - `npx vitest run`: **623 / 623 passed**, including `platform-neutral-source` and "GitLab code has one home";
  - `tsc --noEmit`: clean;
  - eslint on the tests: clean.
- **Rust reference:** `#[serde(default)]`, `gitlab: bool` on `validate_pool_configuration`, `UpdateRunnerPool::validate(gitlab)`, and the update handler calling `validate(pool.scope == "gitlab")` after `pool_access` and before `assert_pool_admin`. Results:
  - `cargo test -p gridops-core --lib --test gitlab_pools`: 307 + 5 passed;
  - `cargo test -p gridops-api --no-fail-fast`: 130 + 36 passed, and fleet_http 143/144.
  - The one fleet_http failure, `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`, **fails the same way on the unmodified baseline**. The preflight reports "macOS ARM64 machine ready … All checks passed" on this Mac host, so the failure comes from the environment and has nothing to do with #172.

Afterwards I reverted `src/**`, restored `models.rs` from my test-only copy, and ran `git checkout` on `resources.rs`. `git status` now shows only the 6 test paths below.

### Hygiene

- I ran `rustfmt --edition 2024 --check` on both Rust test files: clean.
- I staged the following by explicit path:
  - `crates/gridops-api/src/resources/gitlab_tests.rs`
  - `crates/gridops-core/src/models.rs` (test module only)
  - `crates/gridops-core/tests/gitlab_pools.rs`
  - `tests/gitlab-pool-default-image.test.ts`
  - `tests/gitlab-pool-schema.test.ts`
  - `tests/platform-neutral-source.test.ts`

### Coverage holes, by design

- The API proves that the stored empty image resolves through `docker_runner_image(.., true)`. The reconciler's own call (`gridops-reconciler/src/main.rs:2559-2561`) is unchanged and already pinned in `gitlab_lifecycle_tests.rs:561`.
- The live walkthrough is U's job: create a GitLab Docker pool with the image left empty, check that it saves and shows the default text, and check that a GitHub pool's form is unchanged. Docs and the CHANGELOG are checked in Phase 6.

## Ready for F

RED is valid. Every new RED test fails on the assertion the feature will satisfy. The only exceptions are the missing-API compile errors for the decided signatures and the missing `defaultDockerImage` export, which is the intended contract. Each guard test is labelled as one and protects an unchanged behaviour that the brief requires. F may implement against these tests.
