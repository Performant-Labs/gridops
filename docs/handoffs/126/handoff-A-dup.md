# Handoff-A-dup: Phase 7 - Container jobs on the linux pool: runner externals and tool cache (#126)  (anti-duplication gate)

**Date:** 2026-11-22
**Branch:** claude/fix-126-externals
**Diff base:** 9e357e0 (merge-base with master)   **Diff head:** af5f052
**Reuse map:** none (no survey.md); the brief names the objects to extend and the analogue (`NativeRuntime`'s staged release fill, `native.rs`)
**Verdict:** PASS

## Summary
F extended the objects the brief named; no parallel path was built. Prepare, the GitHub branch of Start, the shared-folder create/parse/remove functions and `gridops_core::fleet::work_folder` were all edited in place, and the new fill follows `NativeRuntime`'s Mutex + staging + rename shape.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `crates/gridops-host-agent/src/docker.rs` (`remove_tree_blocking`) | A chmod-and-retry tree removal helper, with no analogue in the host agent (`native.rs` uses plain `tokio::fs::remove_dir_all`). Justified: `docker cp` can leave read-only folders. Not a duplicate. | None; keep private to `docker.rs`. |

Checked and consistent:
- **Start:** the two inline GitHub runner scripts became `github_runner_script(mode, shared)`, which still calls core's `instrumented_runner_script`. The old inline strings are gone, so there is one script path. The socket branch of `start` calls `shared_runner`, which reuses `create_shared_folder` (the renamed `create_shared_work_folder`).
- **Cleanup:** `parse_shared_work_leftover` / `remove_shared_work_folder` were generalised to `parse_shared_folders_leftover` / `remove_shared_folder(s)`, not duplicated for the runner root.
- **Core:** `SHARED_RUNNERS_ROOT`, `EXTERNALS_CACHE_ROOT` and `shared_runner_root` sit beside `SHARED_WORK_ROOT` / `shared_work_folder` and are re-exported the same way. The paths are not duplicated in the agent.
- **Stale check:** the sweep uses the existing `missing()`; the live-log forwarder reuses `GRIDOPS_RUNNER_ROOT`.
- **Fill:** `NativeRuntime`'s `release_lock` / `.partial` / rename pattern is followed, with `tokio::fs` and `spawn_blocking`, as round-3 warn 1 asked.
- **Scope:** GitLab, Tart, native, the server and the reconciler are untouched; `tests/docker_gitlab.rs` and `tests/docker_runtime.rs` gained only assertions.
- **Layering and dependency direction:** no new crate or cross-layer import; `gridops-host-agent` already depended on `gridops-core::fleet`.

No duplication; extension is clean. This does not assess runtime correctness (NV-1 to NV-5 need a daemon).

## Notes for F
None (PASS).
