# Handoff-A-dup: Phase 7 - Fix scoring, rollup and speed-index defects from the wave-1 review (#239)  (anti-duplication gate)

**Date:** 2026-11-24
**Branch:** claude/fh-239
**Diff base:** 73de941   **Diff head:** 5368f69
**Reuse map:** none (no survey.md); the plan-review handoff-A.md (round 2) names the owning object per item.
**Verdict:** PASS

## Summary
PASS. F extended the object that owns each behaviour, and no parallel path exists. There is no new module, table, settings key, scoring entry point or speed query. The only new public object is one constant, `ROLLUP_GRACE_MILLIS`, which A's plan review (warn 3) required.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `crates/gridops-core/src/fleet_health/score.rs` (`measure_*`, `combine_measured`) | The public component functions are now thin wrappers over private `measure_*` functions, and `combine` over `combine_measured`. This is the private-to-`score.rs` shape the brief decided, with one scoring rule per component. Public signatures and wire format are unchanged. No action needed. | None. |
| 2 | warn | `crates/gridops-reconciler/src/fleet_health_rollup.rs` `EARLIER_ATTEMPT_HOURS`, `crates/gridops-core/src/fleet_health/speed.rs` `grouped_durations` | The newest runner and placement subqueries are copied inline into speed.rs, as A warn 6 required, because each statement in this codebase writes them inline. This is deliberate and not a parallel path. | None. |

Checks made:
- **Grace rule:** `ROLLUP_GRACE_MILLIS` is defined once, in `gridops_core::fleet_health` beside `HOUR_MILLIS`. Both rollups import it, with no second copy. The `15 * MINUTE` constants that remain are in `#[cfg(test)]` code, as independent expectations. `START_GATE_WINDOW_MILLIS` is an unrelated constant.
- **Scoring:** `score_fleet` is still the only entry point. The covered-time helpers (`first_hour`, `covered_millis`, `covered_minutes`) are private and shared by every denominator, including `capacity_misfit`.
- **Rollups:** the changes are in `run_pass`, `rollup_hour`, `write_hour` and the catch-up of #191/#200. The machine rollup lost its queue read, so it no longer shares the queue's consumer role. This is a removal.
- **Scope:** the diff touches only the files the plan named, plus the contract test change the brief placed in scope (item 8) and `docs/design/fleet-health.md`. It does not touch `0050_fleet_health.sql`, shared files or another agent's files.
- **Layering:** the new earlier-attempt read runs in the read phase outside the write transactions. `the_scoring_engine_does_no_io` still holds. There is no new crate dependency.

No duplication; the extension is clean.
