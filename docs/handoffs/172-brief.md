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
  - A GitLab pool needs no runner group: `runnerGroupId` may be absent, 0 or any value. Make `runner_group_id` `#[serde(default)]` on both input structs and store a normalised value for GitLab pools (the column's existing default/`1`, whatever the schema requires; read the migration) so nothing downstream sees a nonsense id. GitHub pools (scope `repository`/`organization`) keep "Runner group ID must be positive." exactly, including when the field is absent (absent → 0 → the same 400; confirm the current behaviour for an absent field and keep GitHub's outcome identical, or document the difference if today it is a 422 from serde).
  - **Update** (`UpdateRunnerPool::validate` has no scope): the update handler (`crates/gridops-api/src/resources.rs`, the `input.validate()` call ~line 1603) must know whether the stored pool is a GitLab pool (its stored `scope`, or `fleet::targets::is_gitlab_pool`) and apply the same two relaxations. Prefer passing that fact into validation over duplicating the rules; one function owns each rule.
  - The pool's Tart image rule is unchanged (out of scope; #173 owns the Tart side).
- GitHub and Bitbucket pools validate exactly as before: every existing error string, status code and stored value unchanged.

### Frontend: the registry decides

- No platform comparisons (`=== "gitlab"`, `scope === "gitlab"` for this purpose, `GITLAB` constant) in `schemas.ts`'s image/runner-group rules, in `runner-pools.new.tsx` or in `runner-pools.$poolId.settings.tsx`. The platform descriptor in `src/features/platform-connections/` declares it, e.g. an optional `PlatformDescriptor`/`PoolBindings` key saying the platform has a default Docker runner image (and its display text, `gitlab/gitlab-runner:ubuntu-v19.6.0`) and does not use a runner group. GitLab's descriptor sets it in `gitlab.tsx`; GitHub and Bitbucket leave it unset. Reuse `platformForPool` / the selection-driven resolution the form already has rather than adding a new lookup. Choose one key; keep it small.
- **New pool form:** while the selection targets a platform with a default Docker image (GitLab), the Docker image field is optional, is not prefilled with the GitHub default, says in its help/placeholder that an empty value uses `gitlab/gitlab-runner:ubuntu-v19.6.0`, and an empty value is sent as empty. Switching the selection back to GitHub/Bitbucket restores today's behaviour (required, GitHub default prefilled) without clobbering an image the user typed.
- **Settings form:** a GitLab pool's Docker image field is optional with the same help; an empty value is saved as empty. The read-only settings view and pool detail show the effective image text (e.g. "Default (gitlab/gitlab-runner:ubuntu-v19.6.0)") instead of a blank, via the descriptor.
- **Schema (`schemas.ts`):** the submit-time schema accepts an empty Docker image and any/absent `runnerGroupId` for a GitLab pool and keeps the GitHub rules. On create the schema can know the pool is GitLab from its `scope` field (it already branches on it in `validateGitlabTargets`); on update pass the platform fact in (e.g. a schema factory or a parse option fed from the descriptor), rather than comparing ids in the form. The server stays the authority.
- The GitLab descriptor's `toCreatePayload` stops sending `runnerGroupId: 1` (or keep it only if removal breaks something; say which in the handoff).

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
- Vitest: `createRunnerPoolSchema` / `updateRunnerPoolSchema` accept a GitLab pool with an empty Docker image and no runner group, and reject a GitHub pool with either, with today's messages.
- Vitest (jsdom) for the new-pool form: selecting a GitLab target makes the Docker image optional, not prefilled with the GitHub default, with the default named in the help; submitting empty sends an empty image and no runner group; selecting a GitHub target keeps it required and prefilled. Settings form: a GitLab pool saves with an empty image.
- `tests/platform-neutral-source.test.ts` covers `schemas.ts`'s provider/runner-group rules, `runner-pools.new.tsx` and `runner-pools.$poolId.settings.tsx`: no platform id comparisons.
- The headless UI walkthrough creates a GitLab Docker pool with the image left empty (with a stubbed or seeded GitLab connection), confirms it saves and shows the default image text, and confirms a GitHub pool's form is unchanged.
- `npm run lint`, `npm run typecheck`, `npm test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test -p gridops-api -p gridops-core` pass.
