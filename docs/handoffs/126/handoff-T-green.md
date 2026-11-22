# Handoff-T-green: Phase 7 - Container jobs on the linux pool: runner externals and tool cache at host-valid paths (#126)

**Date:** 2026-11-22
**Branch:** claude/fix-126-externals
**Issue:** #126
**Handoff-F reviewed:** docs/handoffs/126/handoff-F.md
**Handoff-T-red:** docs/handoffs/126/handoff-T-red.md

## GREEN confirmation
All runs with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, as a non-root user (uid 501) on a Mac with no `/var/lib/gridops-agent`, so the 0555 and "agent host" stand-in tests ran in full rather than skipping.
```
cargo test -p gridops-core --lib          -> 313 passed; 0 failed
cargo test -p gridops-host-agent          -> lib 27, dispatch 8, docker_gitlab 10, docker_runtime 2 (+1 +1 filtered), version 1: all passed
```
Same results as F reported (no discrepancy).

**One test repaired (mine):** `a_symlinked_externals_is_followed_or_refused_never_cached_dangling` is now `a_symlinked_externals_is_followed_and_the_cache_never_holds_a_dangling_link`. F documents "followed" on `fill_externals_cache`, so a refusal now fails the test (the `Err` branch was removed). It passes against F's code. No other test changed; no production code touched.

**Spot-check that tests fail when behavior is removed** (production edits made temporarily, then reverted; `git status` shows only the test file modified):
- Dropped `,readonly` from the cache `--mount`: `a_shared_runner_gets_its_folders_at_host_paths_and_the_cache_read_only` FAILED.
- Removed the `.env` rewrite from the runner script: `the_runner_resolves_its_tool_cache_to_the_work_folder_whatever_the_image_sets` FAILED.

## Tier 1 results
| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS (clean, after fmt of my test edit) |
| `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` | PASS, no warnings |
| `cargo test -p gridops-core --lib` / `-p gridops-host-agent` | PASS (above) |
| `cargo test --workspace --locked` | all binaries pass except 1, see below |
| Server start / API smoke | N/A: the change is host-agent only; no server, wire or schema change |

**Workspace failure, unrelated to this change:** `gridops-api` `fleet_http` `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails (149 passed, 1 failed). The preflight prints `PASS macos-machine: macOS ARM64 machine ready ... fleet-host` for a fixture that is meant to hold only a Linux machine, because this host is itself Darwin arm64 and the fixture machine evidently takes the real host's OS and architecture. This diff touches nothing under `crates/gridops-api` or the preflight script, so it is a pre-existing, machine-dependent test (the same class as #186). Not a #126 blocker. Advisory only.

## Tier 2 results
- **Coverage per criterion:** every brief item and A warn has a test, or is listed under "Not covered" in handoff-T-red (below). PASS.
- **Test quality (§7):** each of the 18 + 2 + 3 new/changed tests names one behavior; the two "guard" tests (non-socket runner unchanged, GitLab unchanged) are intentionally green-by-design and cheap; pure functions are unit-tested, the fill and cleanup go through the stand-in `docker` (no daemon). Narrowing the symlink test removed the only either/or test. No redundant tests found. PASS.
- **Type safety:** no `any` equivalent; the cache name is built by a pure function returning `Option<String>` that accepts only lowercase hex; the runner root is built from the typed `PreparedEnvironmentId`. PASS.
- **Error handling:** every failure path of requirement 3 / decision 5 has an exact `(FailureCode, retryable)` test with the reason text asserted (no working dir, no externals, 0555 root without move-aside, create/cp/rm/inspect failures, bad id, Start with cache missing in JIT and persistent). PASS.
- **Data integrity:** atomic fill through `<hex>.partial` and rename; second fill does no new `create`/`cp`; crash-leftover container does not block; sweep keeps a cache when Docker does not answer "missing". PASS.
- **Security / wiring (read, not tested):** `start` resolves the image id on every Start, checks the cache before creating any folder, and creates the container from the id, not the tag (`shared_runner` and `start` in `docker.rs`); a failed `docker create` or `feed_secret` removes both folders. Cleanup removes work folder and runner root, never the cache. PASS by reading.
- **Migration safety:** no schema or wire changes. Hosts need the new agent build; CHANGELOG `### Upgrading` and `docs/fleet-agent.md` say so. PASS.
- **Playwright:** N/A (no UI).
- **Evidence appendix:** `evidence.md` (F's, 7 entries, verbatim excerpts) covers the unchanged-code facts the tests rely on (`missing()`, the log-forwarding path, `feed_secret` 500 ms, retried Start, GitLab early return). I found nothing further to add.
- **Cross-check of F's commands:** reproduced; results match.

## Acceptance criteria status
- Item 1 (externals at host-valid path, cache once per image, runner root outside work, name function, id re-resolved per Start, symlink choice, cleanup of both roots): PASS. Tests: `the_cache_name_is_...`, `the_cache_is_filled_once_per_image_...`, `a_symlinked_externals_is_followed_...`, `a_shared_jit_runner_...`, `a_shared_persistent_runner_...`, `a_runner_root_is_a_direct_child_...`, `cleanup_removes_...`.
- Item 2 (tool cache per runner): PASS. `the_runner_resolves_its_tool_cache_...`, `a_shared_runner_gets_its_folders_...`.
- Item 3 (fail loudly): PASS. `an_image_without_a_working_directory_or_externals_...`, `an_unwritable_cache_root_...`, `a_docker_error_during_the_fill_...`, `a_github_prepare_sharing_the_socket_fails_...`, `a_shared_start_without_a_cache_...`, `the_start_check_finds_the_cache_...`.
- Item 4 (others unchanged): PASS. `a_runner_that_does_not_share_the_socket_...`, `only_a_github_runner_that_shares_the_socket_...`, `tests/docker_runtime.rs`, `tests/docker_gitlab.rs`.
- Item 5 (tests incl. stale-cache sweep, parse never returns the cache, roots disjoint): PASS. `a_fill_sweeps_caches_whose_image_is_gone_...`, `finds_the_work_folder_and_the_runner_root_but_never_the_externals_cache`, `the_runner_root_and_the_externals_cache_are_never_under_a_work_folder`.
- Item 6 (docs, CHANGELOG, copy-cost figures): present and measured from the runner 2.338.2 tarball, not the pool image (F says why).

## Blocking issues
None.

## Advisory notes
- Not exercised by any test and needing a real engine or fleet host (U/S/deploy verification): Start's success path (needs a writable `/var/lib/gridops-agent`), `docker cp -L` and `--mount ...,readonly` on a real daemon, and a host bind source resolved through the runner root's `externals` link (the premise of the fix). The brief forbids live fleet verification from the Mac.
- Per-start copy and fill sizes are a proxy from the release tarball; the pool image may add files.
- The `release_preflight` failure above is environmental and pre-existing.
- No UI surface: U is N/A.
