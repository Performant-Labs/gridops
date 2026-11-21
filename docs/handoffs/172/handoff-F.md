# Handoff-F: Phase 6 - GitLab Docker pool default image and no runner group (#172)

**Date:** 2026-11-21
**Branch:** claude/gitlab-172-pool-image-bb7d36
**Issue:** #172

## What was done

- `crates/gridops-core/src/models.rs`: `validate_pool_configuration` takes a new last parameter, `gitlab: bool`, following the `macos_runtime` precedent. When it is true, an empty selected Docker image passes, but a non-empty one is still checked with `valid_image`, and the runner-group rule is skipped. `CreateRunnerPool::validate()` passes `self.scope == "gitlab"`. `UpdateRunnerPool::validate(&self, gitlab: bool)` passes its argument through. `runner_group_id` is `#[serde(default)]` on both input structs.
- `crates/gridops-api/src/resources.rs`: `update_runner_pool` now calls `input.validate(pool.scope == "gitlab")` right after `pool_access` and before `assert_pool_admin`, as A's warning 2 asked.
- `src/features/platform-connections/types.ts`: new optional `PlatformDescriptor.defaultDockerImage?: string` key, top-level, following the `labelRules` precedent.
- `src/features/platform-connections/gitlab.tsx`: only the descriptor's pool-level key changed. It sets `defaultDockerImage: "gitlab/gitlab-runner:ubuntu-v19.6.0"`. Connection fields, connection text and `toCreatePayload` are untouched.
- `src/features/runner-pools/targets.ts`: three pure helpers that take the descriptors as an argument and import no registry:
  - `defaultDockerImage(descriptors, selection)`, in the same shape as `labelRules`;
  - `dockerImageHelp(defaultImage)`, the row's help copy;
  - `shownDockerImage(stored, defaultImage)`, which composes "Default (…)".
- `src/features/runner-pools/schemas.ts`:
  - `validateProvider` takes a neutral `dockerImageOptional = false` parameter.
  - The create schema passes `value.scope === "gitlab"`.
  - The update schema is built by `updateRunnerPoolSchemaFor(dockerImageOptional)`. The exported `updateRunnerPoolSchema` is `updateRunnerPoolSchemaFor(false)`, so it keeps the GitHub rules.
  - `parseUpdateRunnerPoolInput(input, { dockerImageOptional })`: leaving the option out means `false`.
- `src/features/runner-pools/runner-pools.functions.ts`: `updateRunnerPoolAction` destructures `dockerImageOptional` next to `poolId` and passes it to the parser. It is never put in the PUT body (A's warning 3).
- `src/routes/_app/runner-pools.new.tsx`: the Docker image state now holds only what the user typed (`null` until then). The displayed value is the typed text, or else `""` when the selection has a platform default, or else `options.defaults.dockerImage`. While a default applies, the row:
  - is not `required`;
  - shows the default as its placeholder;
  - shows `dockerImageHelp(...)` as its help;
  - carries the "optional" tag from the wireframe.
- `src/routes/_app/runner-pools.$poolId.settings.tsx`: the editor reads `platformForPool(pool).defaultDockerImage`. With a default:
  - the row is optional and shows the same help and placeholder;
  - the action receives `dockerImageOptional: true`; for a pool without a default it receives `undefined`, never `false`, as `tests/pool-target-payloads.test.ts:394,402` requires;
  - `FleetExecutionSection` receives `pool.dockerImage || legacy image || raw default || ""`.

  The read-only view shows `shownDockerImage(...)`.
- `src/routes/_app/runner-pools.$poolId.index.tsx`: the Image property row shows `shownDockerImage(pool.dockerImage, platformForPool(pool).defaultDockerImage)`, as text and as title.
- `docs/gitlab-runners.md`: two new paragraphs in "Pools, labels and GitLab tags":
  - the Docker image may be left empty and then runs `gitlab/gitlab-runner:ubuntu-v19.6.0`, shown as "Default (…)";
  - a GitLab pool has no runner group.

  The Docker-socket section now says the default applies "when the pool's Docker image is empty".
- `CHANGELOG.md` (Unreleased): one sentence plus `#172` added to the GitLab bullet, and a new "Changed" bullet for the two accepted API behaviour changes: 422→400 for a GitHub body with no `runnerGroupId`, and 400→404 for an invalid update to a missing or inaccessible pool.

## Design decisions

- **New-pool image state holds only typed text.** This follows D's decision. A form that always held a value, and reset it when the targets changed, would clobber typed text and fail T's "typed image survives switching" test. `null` means "untouched". A typed `""` counts as typed, so clearing the GitHub default does not refill it while the user is editing.
- **One schema factory instead of a second exported schema.** `updateRunnerPoolSchemaFor(bool)` keeps one copy of the update refinements. The default (`false`) instance is the existing export, so every importer of `updateRunnerPoolSchema` is unchanged. `parseUpdateRunnerPoolInput` builds the relaxed instance only when it is asked for.
- **Display and help copy as pure helpers in `targets.ts`.** The routes are not allowed to name a platform, and both forms need the same help text. Putting the copy beside `defaultDockerImage` keeps it in one place. The brief also asked for "one helper" to compose the "Default (…)" label. I folded the "stored or default" choice into that helper (`shownDockerImage`), so the settings view and the detail route share the rule.
- **The fleet fallback is `|| (platformDockerImage ?? "")`, appended to today's expression.** GitHub and Bitbucket have no default, so they pass exactly the string they pass today.
- **The "optional" tag is a muted inline span inside the label**, following the wireframe's `<span class="tag">optional</span>`. No shared "optional" component exists in the codebase, and I did not add one.
- **`toCreatePayload` keeps `runnerGroupId: 1`.** The brief allows keeping it or dropping it, and dropping it changes nothing on the wire. Keeping it leaves the existing pins (`tests/gitlab-descriptor.test.ts:315`, `tests/gitlab-pool-forms.test.ts:124`) untouched, and it means one fewer edit in `gitlab.tsx`, which #171/#174 are also editing.

## Reuse / extend-vs-new

- Extended `validate_pool_configuration` with a context parameter, as `macos_runtime` already does. I added no parallel validator.
- Reused the existing runner-group normalisation in both handlers (`resources.rs:1741-1746`, `:2674-2679`). There is no new normalisation and no migration.
- Extended `PlatformDescriptor` with an optional key (the `labelRules` precedent). `targets.ts` gained helpers in the existing `labelRules(descriptors, selection)` shape.
- Settings, read-only settings and detail read `platformForPool(pool)`, as the detail route already does for `Destination` (#169).
- `FleetExecutionSection` and its `defaultImages` are reused unchanged.
- No new object, file or module was created.

## Architecture notes for A

- **Public interface change (Rust):** `UpdateRunnerPool::validate` now takes `gitlab: bool`. It is re-exported from `gridops-core` (`lib.rs:44`). Its only production caller is the API update handler. T updated the test callers in Phase 5.
- **Wire contract:** `runnerGroupId` is now optional in the request bodies of both `POST` and `PUT /api/v1/runner-pools`. Error precedence in `PUT`: 404 (missing or inaccessible pool), then 400 (invalid body), then the admin check. Both changes are accepted in the brief and listed in the CHANGELOG.
- **TS:** a new optional `PlatformDescriptor` key. `updateRunnerPoolAction`'s `data` type gains an optional, non-wire `dockerImageOptional`. The neutral parameter on `parseUpdateRunnerPoolInput` is optional, so existing callers are unchanged.
- **Dependency direction is unchanged.** `schemas.ts` imports nothing from `platform-connections/`, and `targets.ts` still imports types only, with no registry import. Only the routes import `platformForPool`, which they already import or which sits beside `platforms`.
- I am reporting `archChanged: true`, because a public Rust method signature and the request contract changed, even though both were decided in the brief.

## Deviations from spec / wireframe

None. The wireframe's notes say "`toCreatePayload` drops `runnerGroupId: 1`", but the amended brief makes that optional (option A), and keeping the field is the no-op choice (see Design decisions).

## Tier 1 self-check (incl. tests now GREEN)

Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`.

    $ cargo test -p gridops-api --bin gridops-api -- resources::gitlab_tests
    test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 91 filtered out

    $ cargo test -p gridops-core --lib --test gitlab_pools
    test result: ok. 307 passed; 0 failed ...
    test result: ok. 5 passed; 0 failed ...

    $ cargo test -p gridops-api -p gridops-core --no-fail-fast
    gridops-api bin: 130 passed; control_plane_contracts: 36 passed;
    fleet_http: 143 passed; 1 failed
      release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine
      (pre-existing and environmental: T recorded the same failure on the unmodified baseline on this
       Mac host, whose preflight reports a ready macOS ARM64 machine)
    gridops-core: lib 307 passed, every integration test binary ok (0 failed)

    $ cargo fmt --check
    (clean)

    $ cargo clippy --all-targets -- -D warnings
    fails only on lints new in rustc 1.99.1, in files this change does not touch:
      clippy::unused_async_trait_impl  gridops-host-agent/src/{helper.rs:188,native.rs:200},
                                       gridops-manager/src/main.rs:233,
                                       gridops-api/src/{fleet/boundary.rs:78,fleet/enrollment.rs:46,resources/route_security.rs:16}
      clippy::assert_is_empty          gridops-core/src/autoscaling.rs:1810,1896,
                                       gridops-core/src/autoscaling_gitlab_tests.rs:288,384
    $ cargo clippy --all-targets -- -D warnings -A clippy::unused_async_trait_impl -A clippy::assert_is_empty
    Finished `dev` profile ... (clean across the workspace)

    $ npx vitest run
    Test Files  54 passed (54)
         Tests  623 passed (623)
    $ npm run typecheck   (tsc --noEmit: clean)
    $ npm run lint        (eslint --max-warnings=0: clean)

T's RED was 10 failing vitest tests, 4 failing API tests, and compile errors in core. All of them now pass.

## Evidence appendix

`docs/handoffs/172/evidence.md`, with seven facts:

- `docker_runner_image` resolves an empty image to `GITLAB_RUNNER_IMAGE`;
- the `GITLAB_RUNNER_IMAGE` constant;
- the runner-group normalisation in both handlers;
- `pool_access` returns a 404;
- `FleetExecutionSection` starts a new Docker platform from its `dockerImage` prop;
- GitHub's partial always sends `runnerGroupId: 1`.

## Tests that look wrong (for T)

None.

## Known issues

- Workspace `cargo clippy --all-targets -- -D warnings` (a Done-when item) does not pass on this host's rustc 1.99.1, because of the two lints listed above. They are all in files #172 does not touch, so they are not regressions from this change. Fixing them would be a drive-by edit to shared code, so I left them alone. O should decide whether to file a separate toolchain-lint issue or confirm that CI pins an older toolchain.
- The headless UI walkthrough in the Done-when is U's job (Phase 9), and I did not run it.

## Files changed

- crates/gridops-core/src/models.rs
- crates/gridops-api/src/resources.rs
- src/features/platform-connections/types.ts
- src/features/platform-connections/gitlab.tsx
- src/features/runner-pools/targets.ts
- src/features/runner-pools/schemas.ts
- src/features/runner-pools/runner-pools.functions.ts
- src/routes/_app/runner-pools.new.tsx
- src/routes/_app/runner-pools.$poolId.settings.tsx
- src/routes/_app/runner-pools.$poolId.index.tsx
- docs/gitlab-runners.md
- CHANGELOG.md
