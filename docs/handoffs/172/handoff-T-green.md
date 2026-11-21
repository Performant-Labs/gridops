# Handoff-T-green: Phase 7 - GitLab Docker pool default image and no runner group (#172)

**Date:** 2026-11-21
**Branch:** claude/gitlab-172-pool-image-bb7d36
**Issue:** #172
**Handoff-F reviewed:** docs/handoffs/172/handoff-F.md
**Handoff-T-red:** docs/handoffs/172/handoff-T-red.md

## GREEN confirmation

Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`.

    $ npx vitest run
    Test Files  54 passed (54)   Tests  623 passed (623)
    $ cargo test -p gridops-api -p gridops-core --no-fail-fast
    api bin 130 ok (gitlab_tests 39/39), control_plane_contracts 36 ok, fleet_http 143 passed / 1 failed,
    core lib 307 ok, gitlab_pools 5 ok, every other binary ok
      the one failure: release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine
      (pre-existing and environmental: fails identically on the unmodified baseline on this Mac host, per T-red)

All Phase-4 RED tests (10 vitest, 4 API, core compile errors) now pass. F reported no wrong tests; I changed no tests.

Spot-check (mutation), reverted afterwards with `git checkout`; the tree is clean:
- Removing `&& !gitlab` from the Docker-image and runner-group rules in `models.rs`: `a_gitlab_docker_pool_is_created_with_no_image_...` and `a_gitlab_pool_is_updated_with_no_image_...` FAIL.
- Removing `!dockerImageOptional &&` from `validateProvider`: the create-schema and update-schema optional tests FAIL.

## Tier 1 results

| Check | Result |
|---|---|
| `npm run typecheck` (tsc --noEmit) | PASS, clean |
| `npm run lint` (eslint --max-warnings=0) | PASS, clean |
| `npx vitest run` | PASS 623/623 |
| `cargo fmt --check` | PASS |
| `cargo clippy --all-targets -- -D warnings` | FAIL on host rustc 1.99.1, only for new lints `unused_async_trait_impl` / `assert_is_empty` in files this change does not touch (as F reported). With those two allowed: PASS, workspace clean |
| `cargo test -p gridops-api -p gridops-core` | PASS except the one baseline environmental failure above |
| API smoke | Covered by the real-router tests in `gitlab_tests.rs` (201/200/400/404/403 statuses) |

## Tier 2 results

- Test coverage per criterion: PASS (table below).
- Test quality: PASS. Each test names one behavior and runs at the cheapest tier that exercises it (rules through the real routers, schema at unit level, forms in jsdom, neutrality at source level). Guard tests are labelled as such in T-red. No redundant tests found worth pruning; the suite is proportionate (5 Rust + 16 vitest + neutrality checks).
- Type safety: PASS. No `any` casts added; the only cast is the existing `as UpdateRunnerPoolInput & {...}` widened to carry the non-wire flag.
- Error handling: PASS. Padded image on a GitLab pool still 400; GitHub empty image and runner group 0 keep their exact 400s; invalid body to a missing pool is 404; a read-only member's invalid body is 400, a valid one 403.
- Data integrity: PASS. Empty image stored empty; runner group stored as 1 through the existing normalisation. No migration needed, and none added.
- API contract: PASS. `runnerGroupId` optional on both POST and PUT; documented in CHANGELOG (422->400 and 400->404 behaviour changes).
- Security: PASS. Validation moved after `pool_access` and before `assert_pool_admin`; no authorisation check removed or weakened. `dockerImageOptional` is never sent on the wire (tested).
- Migration safety: N/A.
- Playwright: the project has no Playwright suite run in this phase (jsdom vitest only). No e2e skips to flag; the live surface is U's walkthrough.
- Platform neutrality: PASS. `platform-neutral-source` and "GitLab code has one home (#141)" pass; the image literal lives only in `gitlab.tsx`, pinned equal to `GITLAB_RUNNER_IMAGE` by test.
- Evidence appendix: PASS. I checked all seven entries in `evidence.md` against source (`targets.rs:503-509`, `provisioning.rs:34`, `resources.rs:2674-2679`, `:1741-1746`, `:5032-5034`, `fleet-profile-section.tsx:39-41`, `github.tsx:277-282`); the excerpts are verbatim. Nothing to add.
- Cross-check of F's reported commands: all reproduced with identical results (623 vitest, gitlab_tests 39, core 307, api 130/36/143+1 baseline failure, fmt clean, clippy clean with the two lints allowed).

## Acceptance criteria status

| Criterion | Status | Backing test |
|---|---|---|
| POST gitlab+docker, empty image, no/0 runnerGroupId -> 201, stored empty, resolves to GITLAB_RUNNER_IMAGE | PASS | `a_gitlab_docker_pool_is_created_with_no_image_and_no_runner_group_...` |
| Update of that pool with empty image/no group succeeds | PASS | `a_gitlab_pool_is_updated_with_no_image_and_no_runner_group` |
| Padded image on GitLab pool still 400 | PASS | `a_gitlab_pools_docker_image_is_still_checked_when_one_is_given` |
| GitHub exact 400s on create and update; org create with no group -> 400 | PASS | `a_github_pool_keeps_the_docker_image_and_runner_group_400s_...` |
| Invalid update to missing pool -> 404 (reader still 400, valid 403) | PASS | `an_invalid_update_is_a_404_for_a_missing_pool_...` |
| createRunnerPoolSchema / parseUpdateRunnerPoolInput optional rules; GitHub message kept | PASS | `tests/gitlab-pool-schema.test.ts` (+4) |
| New-pool form: optional, not prefilled, default named, empty sent empty; GitHub required+prefilled; typed image survives | PASS | `tests/gitlab-pool-default-image.test.ts` |
| Settings: saves empty; read-only and detail show "Default (...)" | PASS | same file |
| Fleet editor starts from raw GitLab image; GitHub from pool image | PASS | same file |
| Descriptor default == `GITLAB_RUNNER_IMAGE` | PASS | same file |
| Neutral-source coverage of validateProvider, routes, fleet-profile-section | PASS | `tests/platform-neutral-source.test.ts` |
| Docs and CHANGELOG | PASS | reviewed `docs/gitlab-runners.md`, `CHANGELOG.md`; match behaviour |
| Headless UI walkthrough | Not mine | U (Phase 9) |

## Blocking issues

None.

## Advisory notes

- Workspace `clippy -D warnings` fails on this host's rustc 1.99.1 only for pre-existing lints in untouched files; O should file a separate toolchain-lint issue or confirm CI pins an older toolchain.
- `release_preflight` failure in `fleet_http` is environmental (Mac host reports a ready macOS ARM64 machine) and predates #172.
- `docs/gitlab-runners.md` has a slightly awkward line wrap ("using" alone on a line) in the Docker-socket paragraph; cosmetic only.
