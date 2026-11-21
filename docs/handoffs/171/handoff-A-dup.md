# Handoff-A-dup: Phase 7 - GitLab token needs manage_runner; refused deletions become pool warnings (#171, #174)  (anti-duplication gate)

**Date:** 2026-11-21
**Branch:** claude/gitlab-171-scopes-cbe8b5
**Diff base:** 7164fda   **Diff head:** 3d80d4b
**Reuse map:** none (no survey.md); the analogues are named in docs/handoffs/171-brief.md and docs/handoffs/171/handoff-A.md
**Verdict:** PASS

## Summary
PASS. F extended every analogous object and built no parallel path. The scope rule is a one-line change to the loop in `verify_access`. The warning row goes through a new `runner_event` helper that `pool_event` now delegates to, so there is still one `INSERT INTO runner_events` in the reconciler. The per-runner event is written inside `deregister`, the single seam all four callers share. The help text uses the existing plain-string `help` slot. `fake.rs` is untouched.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `crates/gridops-reconciler/src/gitlab_lifecycle.rs` (`refusal_code`, `NotFound` arm) | `NotFound` has a code but cannot reach it: `deregister` and the sweep both treat `NotFound` as success. Harmless, but it suggests a path that does not exist. | Keep (the match has no wildcard, which is deliberate) and add a one-line comment, or leave. Not drift. |
| 2 | warn | `src/features/platform-connections/gitlab.tsx:342` | "GridOps checks both before saving" is ambiguous now that three scopes are listed. | Reword to "checks these". Copy only; S's call. |

Checked and consistent:
- **Scope rule:** `http.rs` extends the existing `for needed in [...]` loop. Create, token replacement and Verify keep sharing the one `verify_access` call. The API's `MissingScope` message is reused unchanged. `fake.rs` is unchanged, as A's plan asked.
- **Event write:** `runner_event` is a sibling that `pool_event` delegates to with `runner_id = None`. No second hand-written INSERT. Level is `"warning"`. The event name is a module constant.
- **Per-runner path:** the event is written in `deregister`'s `Err` arm only for a GitLab refusal. `deregister_runner`, `fleet_material`, `fleet_rollback` and the failed-start cleanup all pass `pool_id` and `runner_id`. No per-call-site copies.
- **Sweep:** `warn_linked_pools` and `warn_pool` reuse `record_refused_deletion`, so message and metadata shape exist once. The dedup state is read back from `runner_events`, as `record_provider_changes` does; no in-memory map, setting or table. No migration.
- **UI:** no new component. `PoolEventTable` is reused as it is.
- **Docs and CHANGELOG:** `docs/gitlab-runners.md` edited in place; one CHANGELOG line under Unreleased. No other scope list exists in `docs/` or the README outside handoffs.
- **Coexistence:** no #172 or #173 files touched (pool forms and validation, `scripts/`, the Tart agent, `docs/macos-runners.md`).

## Notes for F
None required (PASS).
