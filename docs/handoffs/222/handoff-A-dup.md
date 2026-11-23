# Handoff-A-dup: Phase 7 - Lock storm: DELETE FROM host_samples scans 628k rows inside the write lock (#222)  (anti-duplication gate)

**Date:** 2026-11-22
**Branch:** claude/fix-222-retention-batches
**Diff base:** 72bde79 (A's Phase 3 pass; production diff also checked against origin/master 36dbc1a)   **Diff head:** 1696975
**Reuse map:** none (no survey.md); the brief and handoff-A.md name the extension points: `gridops_core::retention`, `cleanup_retention`, `snapshot_retention`.
**Verdict:** PASS

## Summary
F extended the planned objects and built no parallel path. The loop that `snapshot_retention` owned moved into `crate::retention::delete_in_batches`, which `prune_inventory_snapshots` now calls, so there is one copy. All eight retained-table deletes in `cleanup_retention`, including #221's three fleet-health ones, now go through `RetentionTable` and `prune_retained_table`. No literal retention `DELETE` is left in the reconciler.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | info | `crates/gridops-core/src/retention.rs` | Statements are a closed set of `&'static str` consts chosen by an enum, as the brief required. `crate::retention` imports nothing from `fleet_health` (dependency direction unchanged). | None. |
| 2 | info | `crates/gridops-core/src/fleet/registry/snapshot_retention.rs` | Constants re-exported, loop delegated; its own `PRUNE_BATCH` stays because its WHERE clause differs (brief allowed this). | None. |
| 3 | warn | `crates/gridops-reconciler/src/main.rs` (`reconcile()`, `fleet_agent_logs` delete) and the `log_streams` steps in `cleanup_retention` | Still unbatched / still `?`. Brief scope excludes them and handoff-A finding 4 already requires a follow-up issue. | O opens the `fleet_agent_logs` follow-up before the PR; the PR body claims only per-table prune isolation. |

No duplication; extension is clean. No drift introduced in rework: the new reconciler code (`prune_retained_table`, one cutoff arm for the three fleet-health variants) matches the neighboring `prune_inventory_snapshots` shape (info on a non-zero count, warn and continue on error).

## Notes for F
None (PASS).
