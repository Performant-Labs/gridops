# Handoff-A-dup: Phase 7 - Tart GitLab/Bitbucket image prep verifies its image; exit 78 is a provisioning failure (#173)  (anti-duplication gate)

**Date:** 2026-11-21
**Branch:** claude/gitlab-173-tart-prep-164633
**Diff base:** 2d6e458150091c42b0e6a1c48c9ad8500eb6ed4a   **Diff head:** 42ac8e503d7e3af0ef6b5da5bfaa23ea5ceaefef
**Reuse map:** none (no survey.md); the brief names the analogues, as handoff-A.md recorded
**Verdict:** PASS

## Summary
F extended the analogous objects the brief named and built no parallel path. `GITLAB_EXIT_REASONS` and the Tart copy gained one entry each, both collision guards gained 78, `Exit` gained one variant, and `left_service` now has a single shared failure path for `RegistrationRefused` and `RunnerMissing`. Both prep scripts keep their own entry points and follow the GitHub script's sync / stop / wait-on-pid shape.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `scripts/prepare-tart-gitlab-runner-image.sh:50-85`, `scripts/prepare-tart-bitbucket-runner-image.sh:42-77` | About 40 lines (`stop_vm`, `cleanup`, traps, `wait_for_agent`) are duplicated between the two scripts. The brief allowed this (self-contained is the default; no `scripts/*.sh` sources another file), and F justified it in handoff-F. | None needed. If a third Tart prep script needs the same cleanup, open a follow-up for a shared helper. |

Checked and clean:
- Reconciler: no copied `RegistrationRefused` arm. One block writes `failure_reason`, records the event with `exit.name()` and calls `count_failure`. Counting stays in `left_service`. `from_inspect` and the fleet path are untouched. `runner_missing_reason()` sits beside `refusal_reason`, and only the permitted parts of `gitlab_lifecycle.rs` changed.
- Exit table: the 78 constant and entry are in core and in the Tart agent's own copy, with the same names and values and no new dependency. The Tart `runner_script` guard (`75|77|78`) and core's `finish 1` guard (placeholder) are extended in place.
- Manager: doc comment only; it relays the free string.
- CI: the existing `tart-installer.yml` job gained a step, and both `paths` lists were extended. No new workflow.
- Tests: the new script driver sits under `scripts/` as the brief asked, and the Rust tests are added to the existing test files.

## Notes for F
None (PASS).
