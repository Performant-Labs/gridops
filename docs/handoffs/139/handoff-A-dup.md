# Handoff-A-dup: Phase 7 - GitLab reconciler runner lifecycle  (anti-duplication gate)

**Date:** 2026-11-20
**Branch:** claude/gitlab-139-lifecycle-53c8ce
**Diff base:** b22c451   **Diff head:** a5c9441
**Reuse map:** docs/handoffs/139-brief.md ("The seam: where the code goes")
**Verdict:** PASS

## Summary
F extended the analogous objects the brief named. `gitlab_lifecycle.rs` holds only the GitLab counterparts of the Bitbucket helpers (`next_connection`, `register`/`deregister`/`deregister_runner`, exit mapping, sweep, placeholder client). The shared pipelines (`provision_on_host`, `start_admitted_runner`, `delete_runner`, `fleet::provision`, `register_and_start`, `follow`, `finish`) gained GitLab arms. No parallel provision/start/delete pipeline exists.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | info | `crates/gridops-reconciler/src/main.rs` (`ListedManagerRunners`) | `exitReason` is read via a listing shape on `ManagerRunners` instead of a field on `ManagedRunner`, as the brief said. This is a small deviation, justified in handoff-F (a test literal of `ManagedRunner` would not compile). It reuses the same wire path and is not a parallel one. | None. Optional later collapse into `ManagedRunner`. |
| 2 | warn | `crates/gridops-reconciler/src/gitlab_lifecycle.rs` `next_connection` / `main.rs` `next_bitbucket_connection` | The two selection queries are similar but not shared. The brief deliberately differs (no one-runner cap, fewest-live ordering), so they are counterparts, not duplicates. | Track only if a third platform appears. |

Reuse checks:
- The image rule lives once, in `targets::docker_runner_image`, and `pool_docker_image` calls it.
- `load_pool` shares `load_pools`' query text through `pool_query!`.
- `clear_provision_failures` is extracted and shared, not copied.
- The shared tail `start_with_material` is a verbatim extraction, so the GitHub path is unchanged.
- `delete_runner` dispatches GitLab rows to `deregister_runner`, as the Bitbucket arm does.
- The core `placement::reported_diagnostic` sits beside `reported_gone`.
- Dependency direction is `fleet.rs` -> `gitlab_lifecycle` -> `main.rs` helpers. There is no core -> reconciler import.

`exclusive_labels` hunks that appear in a diff against local master come from master drift (#134), not from this change (`git diff b22c451 HEAD` has none).

No architectural drift was introduced during rework.

## Notes for F
None (PASS).
