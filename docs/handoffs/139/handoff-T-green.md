# Handoff-T-green: Phase 7 - #139 GitLab reconciler runner lifecycle

**Date:** 2026-11-20
**Branch:** claude/gitlab-139-lifecycle-53c8ce
**Issue:** #139
**Handoff-F reviewed:** docs/handoffs/139/handoff-F.md
**Handoff-T-red:** docs/handoffs/139/handoff-T-red.md

## GREEN confirmation

F edited no test file (`git diff 2aa6002 --name-only` lists no `*_tests.rs` or `tests/` path), and F flagged no test as wrong. I changed no test either.

- `cargo test -p gridops-reconciler`: 94 passed, 0 failed (68 existing, 26 new `gitlab_lifecycle_tests`).
- `cargo test -p gridops-core`: lib 264 passed; `gitlab_pool_fleet` 3 passed; every other integration binary ok.
- Mutation spot-check: I replaced the `delete_runner` call in `gitlab_lifecycle::deregister` with `Ok(())`. `a_failed_manager_start_deregisters_the_runner_and_the_next_pass_starts_again` then failed at `gitlab_lifecycle_tests.rs:584`. The file was restored with `git checkout` (worktree clean).

## Tier 1 results

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS (clean) |
| `cargo clippy --workspace --all-targets --keep-going -- -D warnings`, filtered to files F touched | PASS: no finding in any changed file. The 17 findings F reported are in untouched files (toolchain drift). |
| Existing reconciler and core tests | PASS |
| F's reported 12 workspace failures | Reproduced on a sample: `gridops-api` `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails (Darwin host reports macOS) and `gridops-manager` `provision_lock_tests` fail with `path must be shorter than SUN_LEN`. Both are environmental, in code this change does not touch, and F showed them failing at 2aa6002. |
| No `unwrap`/`expect`/`panic!` in `gitlab_lifecycle.rs` | PASS |

## Tier 2 results

- **Coverage per criterion:** PASS, see below.
- **Test quality:** PASS. Each test names one behavior and fails for a feature assertion (RED showed that). Manager and fleet paths are separate environments, not duplicates. 26 reconciler tests for items 1-16 over two paths is proportionate; I found nothing to prune or merge.
- **Type safety:** PASS. No `any`-equivalents; clippy clean on changed files.
- **Error handling:** PASS. Refusals, registration failure, delete failure, listing failure and the unwired client each have a test; `deregister_runner` logs and never fails the pass.
- **Data integrity:** PASS. Counts/resets are behind guarded `UPDATE ... rows_affected()==1` (tests for old-completed-runner not resetting a later count, both paths); retry copies `gitlab_connection_id` and never `gitlab_runner_id`.
- **Security:** PASS. Token secrecy asserted across `runners` columns, events, waits, audit and logs (`glrt-` absent). The marker is parsed strictly (UUID only); the sweep ignores other-instance and malformed markers.
- **Migration safety:** N/A, no migration (D16).
- **API contract:** PASS. D10 request shape and sealed `GitlabRunnerMaterial` are pinned by tests.
- **Playwright:** N/A, no UI surface.
- **Evidence appendix:** `docs/handoffs/139/evidence.md` has 11 entries; I spot-checked two against source (`retry.rs:269-270`, `retry.rs:113`) and the excerpts are verbatim. My tests rely on nothing outside it that I found.

## Acceptance criteria status

Items 1-16 of the brief's Done-when list are each backed by a Phase-4 test (see handoff-T-red tables) that now passes. Exceptions noted by F, both accepted:
- The optional manager-path busy reset is not implemented (needs #140); no test pins it. Not a criterion.
- `ManagedRunner` does not gain `exitReason`; the read lives on `ManagerRunners` via a listing shape with the same wire behaviour (the field would break an existing test literal). Behavior is covered by the exit tests.

## Blocking issues

None.

## Advisory notes

- Clippy `-D warnings` workspace-wide cannot pass on this toolchain because of 17 pre-existing findings outside this change. Note it in the PR.
- U is N/A: no UI surface. S next.
- PR note (F): record master 50ed9b4 local / c575130 origin, and `GITLAB_RUNNER_IMAGE` absent from both, so the refusal form is asserted (Done-when item 4).
- `ensure_target_access` for a GitLab target reads `bitbucket_connection_id` (NULL); passes for `Principal::Reconciler`; user-scoped GitLab visibility deferred per D11.
