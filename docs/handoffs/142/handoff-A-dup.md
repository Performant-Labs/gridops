# Handoff-A-dup: Phase 7 - GitLab macOS runners on Tart pools (#142)  (anti-duplication gate, second run)

**Date:** 2026-11-20
**Branch:** claude/gitlab-142-tart-13d6d4
**Diff base:** 3c215ee (last A-dup PASS)   **Diff head:** 939eb85
**Reuse map:** none (no survey.md); baseline is the analogues named in docs/handoffs/142-brief.md
**Verdict:** PASS

## Summary
Since the last A-dup pass, the diff touches only tests (`gitlab_tests.rs`, the `mod tests` block of `main.rs`) and one docs paragraph. No production file changed, and no new object was created. The extension of the analogous objects stands; no parallel path exists.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | info | `crates/gridops-tart-agent/src/main.rs` (FAKE_TART, ~2920) | The shared fake `tart` gained `exec-hold` and a `killed` status in place, rather than a second fake being added. | none |
| 2 | info | `crates/gridops-tart-agent/src/gitlab_tests.rs` | New fake behaviours (`job-taken`, `exit-75`, `exit-77`, `silent`) extend the existing `FAKE_GITLAB_RUNNER`. The cases table became unconditional, and the hostile-URL case moved into the existing list. | none |
| 3 | info | `docs/macos-runners.md` | One sentence added to the existing GitLab section. | none |

No duplication; extension is clean. No edits to manager, host-agent, core or shared contract types, so no drive-by drift.

## Notes for F
None.
