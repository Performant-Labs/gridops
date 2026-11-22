# Handoff-A-dup: Phase 7 - Cap the time a fleet placement spends in `registering` (#130)  (anti-duplication gate)

**Date:** 2026-11-22
**Branch:** claude/fix-130-registering-cap
**Diff base:** 9e357e0   **Diff head:** cca9ed7
**Reuse map:** none (`survey.md` absent); the analogues named in the brief and in handoff-A D1-D10 were used.
**Verdict:** PASS

## Summary
F extended the analogous objects and built no parallel path. The give-up calls the existing `placement::abandon_unstarted`; the cause is a new variant in the existing closed `PlacementFailureReason` set; the cap is a constant beside `QUEUE_TIMEOUT_MILLIS` with a helper beside `expire_queued`. Replacement and upstream cleanup go through the existing `finish` -> pool pass -> `delete_runner` path; nothing was added there. No migration, no new module, no new core function, no GitLab-specific code.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | none | `crates/gridops-reconciler/src/fleet.rs` (`tick`, `expire_registering`) | D3 followed: the cap runs after every attempt, whatever it returned, in `tick`'s `registering` loop, so it covers all platforms. A give-up error is logged and does not abort the pass. | none |
| 2 | none | `failure.rs`, `start_health.rs`, `gridops-v1.json`, `query-contracts.ts`, `placement-failures.ts`, `docs/fleet-api.md` | D5: all six contract surfaces carry `registration_timed_out`. `is_start_failure` is `false` (D6). | none |
| 3 | none | `registration_cap_tests.rs`, `gitlab_lifecycle_tests.rs`, `start_tests.rs` | Fixtures were generalised, not copied: `Fleet::with_pool` split into `on_machine` plus `Fleet::github`, parameterised on pool; `Setup`'s fake GitHub gained a switchable 409 and runner-list route. Only the new tests live in the new module. | none |
| 4 | warn (informational) | `docs/fleet-api.md` | The docs note went to `fleet-api.md` rather than `fleet-automation.md`, as D10 decided. | none |

No duplication; extension is clean. No unplanned edits of shared or other-agent-owned code: the touched files are the ones D5 lists, plus `main.rs` for the two-line test module wiring.

## Notes for F
None required (PASS).
