# Handoff-A-dup: Phase 7 - Fleet health shared contract (#197)  (anti-duplication gate)

**Date:** 2026-11-22
**Branch:** claude/fleet-197-contract
**Diff base:** fbacbc2 (merge-base with master)   **Diff head:** caa663b
**Reuse map:** none (no `survey.md`); analogues are those named in `handoff-A.md` (#137 GitLab contract, `/fleet/workload`, #152 retention) and the brief's Reuse section
**Verdict:** PASS

## Summary
F extended the analogous objects and built no parallel path. The new modules (`fleet_health`, the api health and job-category handlers, the reconciler passes, the charts, the guard) are the ones the brief creates. Everything that overlaps existing code reuses it. Round 2's only production change is the slot wrappers in `HealthSections` in `page.tsx`, which extends that component in place.

## Reuse checked in the diff
- `fleet_queue_breakdown` (`overview_queue.rs`): `pools_with_platforms`, `waiting_job` and `split_waiting_jobs` were taken out of `queue_breakdown`, which now calls them. No second queue split.
- `host_live_reads` (`registry/queries.rs`): built from `counts`, `root_budget`, `allocation_read`, `latest_measurement`, `placement_failures` and `start_gate_read`. No parallel host projection.
- GitLab writer (`gitlab/jobs.rs`): D18's single UPSERT is extended in place (name, path, started/completed, category on insert).
- `FleetHealthState` uses `RegistryClock` and `system_clock()`. `AppState` gets one field plus two builder methods in the existing style.
- `ApiError` gets one variant (`Invalid`, 422); `require_admin` is made `pub(super)` and reused. The `TOKEN_SCOPED_ROUTES` row follows `/fleet/workload`.
- Frontend: `useFleetHealth` follows `FleetWorkloadSection`'s fetch pattern (component state, no query client); a private `request` follows the fleet `admin-api.ts` shape (A warn 27); `~/lib/api.ts` `ApiError` is extended, not replaced; the nav list gets one item and `isActivePath` is fixed.
- The db.rs speed-check hook instruments the existing pool builder under the `test-support` gate, on top of the #179 slow-statement log.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `src/components/charts/chart-theme.ts:11-12` | `GOOD_SCORE = 90` and `FAIR_SCORE = 70` restate the thresholds of `successTone` (`src/features/dashboard/metric-tiles.ts:69-71`) as a second copy; the doc comment says they are equal, but nothing ties them. A later change to either drifts silently. | In a wave-1 or follow-up change, have `successTone` use `scoreBand`/`bandTone`, or add a test that pins the two together. Not a parallel path: a shared constant pair, so no block. |
| 2 | warn | `crates/gridops-api/src/resources/overview_queue.rs:~372` and `resources.rs:90-98` | `fleet_queue_breakdown` still returns the legacy `ApiResult` (A warn 24, noted open in round 4), and `fleet_queued_jobs!` repeats the body of `visible_queued_jobs!` minus the user join instead of composing it. The design doc (`fleet-health.md:281`) names the `crate::resources` call but does not call it out as a new `fleet → resources` edge. Neither is a duplicate query path (the pool and job-owner logic is shared). | #201 maps the error where it calls it; if the two clauses are ever edited, edit both. Consider adding one sentence to the design doc naming the edge. |
| 3 | warn | `src/features/dashboard/metric-tiles.ts:42-47` | `metricTiles` takes `_health` and does nothing with it until #203. It is a deliberate hook (the brief's wiring-here rule), so not drift. | None; #203 fills it. |

No duplication; extension is clean. No unplanned drive-by edits of shared code were found: the shared-file edits (`resources.rs`, `overview_queue.rs`, `registry/queries.rs`, `jobs.rs`, `api.ts`, `navigation.ts`) are each named in the brief.

## Notes for F
None (PASS).
