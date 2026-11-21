# [GitLab: a GitLab Docker pool can't use the default gitlab-runner image](https://github.com/Performant-Labs/gridops/issues/172)

Part of epic #136. Found by #143's live end-to-end run against gitlab.com. Two other sessions are fixing #171/#174 (GitLab connection verification and connection form text) and #173 (Tart image script and agent) in parallel: do not touch `src/features/platform-connections/gitlab.tsx`'s connection form fields/text, the GitLab connection verify code, `scripts/prepare-tart-gitlab-runner-image.sh` or the Tart agent.

## Need

`docs/gitlab-runners.md` (~line 161) and the fleet code (`docker_runner_image` in `crates/gridops-core/src/fleet/targets.rs` ~line 503: `"" if gitlab => GITLAB_RUNNER_IMAGE`) say a GitLab Docker pool with no image runs `gitlab/gitlab-runner:ubuntu-v19.6.0` (`GITLAB_RUNNER_IMAGE` in `crates/gridops-core/src/provisioning.rs`). Pool validation never lets that happen:

1. **Image, API.** `validate_pool_configuration` in `crates/gridops-core/src/models.rs` (~line 443-457) returns "Docker image (dockerImage) is required for a pool that uses Linux · Docker." whenever the Docker provider is selected and the selected Docker image is empty, so `POST /api/v1/runner-pools` with `scope: "gitlab"`, `provider: "docker"` and an empty `image`/`dockerImage` is a 400.
2. **Image, form.** `validateProvider` in `src/features/runner-pools/schemas.ts` (~line 90) has the same rule, and the Docker image `<Input>` is `required` in `src/routes/_app/runner-pools.new.tsx` (~line 231) and `src/routes/_app/runner-pools.$poolId.settings.tsx` (~line 172). Worse, the new-pool form prefills `options.defaults.dockerImage` (the control plane's GitHub runner image, `GRIDOPS_RUNNER_IMAGE`), which cannot run GitLab jobs, so a GitLab pool created with the defaults gets a GitHub image.
3. **Runner group.** `validate_pool_configuration` ends with `if runner_group_id <= 0 { "Runner group ID must be positive." }`, and `runner_group_id` is a required, non-defaulted field of `CreateRunnerPool` / `UpdateRunnerPool`. The form's `configurationShape.runnerGroupId` is `z.number().int().positive().default(1)`. A runner group is a GitHub setting that GitLab pools do not use. (The GitLab descriptor's `toCreatePayload` sends `runnerGroupId: 1` today only to get past this.)

## Scope

### API (Rust)

- In `models.rs`, make the Docker-image rule and the runner-group rule depend on whether the pool is a GitLab pool:
  - **Create** (`CreateRunnerPool::validate`): the pool is GitLab exactly when `scope == "gitlab"` (already enforced against `gitlab_connection_ids` by `validate_gitlab_targets`). A GitLab pool may send an empty `image`/`dockerImage` for Docker; it is stored empty and `docker_runner_image(.., gitlab = true)` resolves it to `GITLAB_RUNNER_IMAGE` at provisioning, which is what the docs say. A non-empty image is still checked with `valid_image`.
  - **Seam (decided):** add a `gitlab: bool` parameter to `validate_pool_configuration`, following the `macos_runtime` precedent and `docker_runner_image(.., gitlab)`. `CreateRunnerPool::validate()` derives it from `scope == "gitlab"`; `UpdateRunnerPool::validate(gitlab: bool)` takes it as an argument. Update every caller (`resources.rs` update handler, `crates/gridops-core/tests/gitlab_pools.rs:69`, any others).
  - A GitLab pool needs no runner group: `runnerGroupId` may be absent, 0 or any value. Make `runner_group_id` `#[serde(default)]` on both input structs. **Reuse the existing normalisation** in both handlers (`resources.rs` ~1737-1741 on update, ~2670-2674 on create: `repository|gitlab` store 1); add no new normalisation and no migration. GitHub `organization` pools keep "Runner group ID must be positive." exactly. With `#[serde(default)]`, a GitHub body with no `runnerGroupId` changes from serde's 422 to that 400 message; this change is accepted, and T pins it.
  - **Update:** the update handler (`crates/gridops-api/src/resources.rs`, `input.validate()` ~line 1603) moves validation after the pool loads (`pool_access`) and passes `pool.scope == "gitlab"`, which is how the handler already checks elsewhere (~1643, 1664, 1737); not `fleet::targets::is_gitlab_pool`. Accepted precedence change: an invalid body sent to a missing or inaccessible pool now returns 404 instead of 400; T pins it.
  - The pool's Tart image rule is unchanged (out of scope; #173 owns the Tart side).
- GitHub and Bitbucket pools validate exactly as before: every existing error string, status code and stored value unchanged.

### Frontend: the registry decides

- No platform comparisons (`=== "gitlab"`, the `GITLAB` constant) in `validateProvider` in `schemas.ts`, in `runner-pools.new.tsx`, in `runner-pools.$poolId.settings.tsx` or in `fleet-profile-section.tsx`.
- **Descriptor key (decided):** a top-level optional `PlatformDescriptor` key holding the platform's default Docker runner image reference (e.g. `defaultDockerImage?: string`), following the `labelRules` precedent, with a pure helper in `src/features/runner-pools/targets.ts` (or beside the existing selection helpers) that resolves it from the selection (new pool) or from `platformForPool(pool)` (settings/detail). Not in `PoolBindings`. GitLab's descriptor sets it to `gitlab/gitlab-runner:ubuntu-v19.6.0`; GitHub and Bitbucket leave it unset. One helper composes the display label "Default (gitlab/gitlab-runner:ubuntu-v19.6.0)". In `gitlab.tsx` edit only the descriptor's pool-level keys; #171/#174 own its connection fields and text.
- A vitest reads `crates/gridops-core/src/provisioning.rs` with `readFileSync` and asserts the descriptor's value equals `GITLAB_RUNNER_IMAGE`, so the frontend's copy cannot drift.
- **New pool form:** while the selection resolves a default Docker image (GitLab), the Docker image field is optional, is not prefilled with the GitHub default (`options.defaults.dockerImage`), says in its help/placeholder that an empty value uses the default, and an empty value is sent as empty. Switching the selection back to GitHub/Bitbucket restores today's behaviour (required, GitHub default prefilled) without clobbering an image the user typed.
- **Settings form:** a GitLab pool's Docker image field is optional with the same help; an empty value is saved as empty. The read-only settings view and pool detail show the composed "Default (…)" label instead of a blank.
- **Fleet execution editor:** when the pool's Docker image is empty, the settings route passes `FleetExecutionSection` the descriptor's **raw** default image reference (`gitlab/gitlab-runner:ubuntu-v19.6.0`, never the "Default (…)" label) as `dockerImage`, so a new Docker fleet platform of a GitLab pool starts from the GitLab runner image, not `ghcr.io/actions/actions-runner:latest`. `fleet-profile-section.tsx` stays free of platform-specific code (its `defaultImages` fallback is unchanged). GitHub/Bitbucket pools pass exactly what they pass today.
- **Schema (`schemas.ts`):** `schemas.ts` must not import the registry or any descriptor (`gitlab.tsx` and `bitbucket.tsx` import from it; that would be a cycle). `validateProvider` takes a neutral parameter (e.g. `dockerImageOptional: boolean`). Create derives it from `scope` inside `createRunnerPoolSchema`'s superRefine (the schema already branches on scope in `validateGitlabTargets`). Update receives it through `parseUpdateRunnerPoolInput(values, { dockerImageOptional })`, which the settings route feeds from the descriptor helper. The exported `updateRunnerPoolSchema` keeps the GitHub rules. The server stays the authority.
- **Runner group on the wire (decided, option A):** the form's request body keeps carrying `runnerGroupId: 1` (GitHub's partial and the schema's `.default(1)` put it there for every pool; the server already stores 1 for `repository|gitlab`). No form change for the runner group; GitLab's `toCreatePayload` may keep or drop its `runnerGroupId: 1` (dropping changes nothing on the wire). "A GitLab pool needs no runner group ID" is met and tested at the API: the Rust tests send no `runnerGroupId` and `0`. Existing pins at `tests/gitlab-descriptor.test.ts:315` and `tests/gitlab-pool-forms.test.ts:124` stay valid; update them only if the partial changes.

### Docs

- `docs/gitlab-runners.md`: state that the Docker image field may be left empty for a GitLab pool and then uses `gitlab/gitlab-runner:ubuntu-v19.6.0`, and that a GitLab pool has no runner group. Docs and behaviour must match. CHANGELOG entry under Unreleased.

## Constraints

- No deploy. Nothing GitLab ships to Mars until epic #136 is closed.
- Rust builds share this Mac: keep `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2` (`.claude/settings.local.json`).
- No migration unless a stored GitLab runner group value truly needs one (it should not).
- Keep the "GitLab code has one home (#141)" test and `tests/platform-neutral-source.test.ts` passing; extend the latter to cover the files touched here.

## Done when

- Rust API tests (`crates/gridops-api/src/resources/gitlab_tests.rs` style): `POST /api/v1/runner-pools` with `scope: "gitlab"`, `provider: "docker"`, empty `image`/`dockerImage` and no `runnerGroupId` returns 201; the stored `docker_image` is empty and `docker_runner_image(stored, true)` is `GITLAB_RUNNER_IMAGE`. Same with `runnerGroupId: 0`. An update of that GitLab pool with an empty image and no runner group succeeds. A non-empty invalid image (padded) on a GitLab pool is still a 400.
- Rust tests that GitHub `repository`/`organization` pools still return the exact Docker-image 400 for an empty image and the exact runner-group 400 for 0, on create and update.
- Rust tests: update with an invalid body to a missing pool returns 404 (the accepted precedence change); a GitHub create body with no `runnerGroupId` returns the 400 "Runner group ID must be positive." for an `organization` pool.
- Vitest: `createRunnerPoolSchema` accepts a `gitlab`-scoped pool with an empty Docker image, and `parseUpdateRunnerPoolInput(.., { dockerImageOptional: true })` accepts one; both reject a GitHub pool's empty Docker image with today's message.
- Vitest (jsdom) for the new-pool form: selecting a GitLab target makes the Docker image optional, not prefilled with the GitHub default, with the default named in the help; submitting empty sends an empty image; selecting a GitHub target keeps it required and prefilled; a typed image survives switching. Settings form: a GitLab pool saves with an empty image, and its read-only view shows "Default (gitlab/gitlab-runner:ubuntu-v19.6.0)".
- Vitest (jsdom): on a GitLab pool with an empty image, adding a Docker platform in the Fleet execution section starts from `gitlab/gitlab-runner:ubuntu-v19.6.0`; on a GitHub pool it starts from the pool's image as today.
- Vitest: the descriptor's default image equals `GITLAB_RUNNER_IMAGE` in `provisioning.rs`.
- `tests/platform-neutral-source.test.ts` covers `validateProvider` in `schemas.ts`, `runner-pools.new.tsx`, `runner-pools.$poolId.settings.tsx` and `fleet-profile-section.tsx`: no platform id comparisons.
- The headless UI walkthrough creates a GitLab Docker pool with the image left empty (with a stubbed or seeded GitLab connection), confirms it saves and shows the default image text, and confirms a GitHub pool's form is unchanged.
- `npm run lint`, `npm run typecheck`, `npm test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test -p gridops-api -p gridops-core` pass.
