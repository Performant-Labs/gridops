# Handoff-A-dup: Phase 7 - Fleet health shared contract (#197)  (anti-duplication gate, round 3 re-check)

**Date:** 2026-11-22
**Branch:** claude/fleet-197-contract
**Diff base:** fbacbc2 (merge-base with master); this pass re-checks caa663b..f8b8b37 (the S-rework round)
**Reuse map:** none (no `survey.md`); analogues are those named in `handoff-A.md` and the brief's Reuse section
**Verdict:** PASS

## Summary
The S-rework round extends existing objects and builds no parallel path. The previous PASS (caa663b) still stands for everything outside this delta. The delta is one prop on the existing `NavLinkItem`, one dead helper deleted from the guard, and three new tests placed in the files that already hold the contract tests or the nav tests.

## Reuse checked in the delta
- `src/components/app-shell.tsx`: `activeOptions={{ exact: !active }}` is added to the existing `Link` in `NavLinkItem`. It is driven by the app's existing `isActivePath` result, so the router follows the app's one decision and there is no second active-state rule.
- `speed_contract_tests.rs` and `health_live_contract_tests.rs`: the #193 and #201 realistic-volume tests sit in the existing contract files. They use `testdata::build_week`, `testdata::insert_jobs`, `JobSeed`, `speed_check::check`, `speed::speed_indexes` and `collect_live`, all of which already exist. No new fixture builder or timing harness.
- `scripts/fleet-health-guard.mjs`: deletes an unused helper; nothing added.
- `tests/app-shell-nav.test.ts`: a new file, flat in `tests/` like its neighbours. It repeats the jsdom/`createRoot`/`matchMedia` setup that `tests/tart-host-card.test.ts` and `tests/connections-routes.test.ts` each carry. The project has no shared render helper, so the per-file setup is the local pattern, not drift.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `tests/app-shell-nav.test.ts:36-47` | The `matchMedia` stub is the third copy of the same jsdom shim (also in `connections-routes.test.ts` and `tart-host-card.test.ts`). | If a fourth copy appears, extract a shared test helper. Not worth doing in this PR. |
| 2 | warn | `src/components/chart-theme.ts` (carried from the earlier pass, `chart-theme.ts:11-12`) | `GOOD_SCORE`/`FAIR_SCORE` still restate `successTone`'s thresholds. | Unchanged; a follow-up or wave-1 change should tie them together. |

No duplication; extension is clean. No unplanned drive-by edits of shared code: `app-shell.tsx` was confirmed unowned by wave-1 in S's round, and the guard script and tests are #197's own.

## Notes for F
None (PASS).
