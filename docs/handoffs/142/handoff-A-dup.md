# Handoff-A-dup: Phase 7 - GitLab macOS runners on Tart pools (#142)  (anti-duplication gate)

**Date:** 2026-11-20
**Branch:** claude/gitlab-142-tart-13d6d4
**Diff base:** 229081c   **Diff head:** 3c215ee
**Reuse map:** none (no survey.md); baseline is the analogues named in docs/handoffs/142-brief.md
**Verdict:** PASS

## Summary
F extended the existing objects. The only new objects are the ones the brief requires in writing (gitlab.rs, the image script). No parallel path exists.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | info | `crates/gridops-tart-agent/src/gitlab.rs` | New module is mandated by brief Scope 1; holds only GitLab-specific validation, script, exit lookup. Reuses `shell_single_quote`, `GITLAB_EXIT_REASONS`, `GitlabBootstrap`, `ProvisionRunner` from main.rs; no second quoting helper. | none |
| 2 | info | `main.rs` `validate`, `provision_runner`, `start_runner`, `list_runners`, `TartRecord`, config | Extended in place: new tuple arm, new platform arm using the existing `.take()` + SecretString + stdin channel, waiter extended with update-in-place `record_exit_code`, serde-default fields like `root`, env config copied from the Bitbucket pattern. | none |
| 3 | info | `shell_single_quote` | Fixed in place (brief-sanctioned), not duplicated. | none |
| 4 | info | `scripts/prepare-tart-gitlab-runner-image.sh` | Modelled on the Bitbucket image script as the brief specifies; differences (checksum, pinned version) are GitLab-specific. | none |
| 5 | warn | `gitlab.rs` `valid_url` / `valid_token` | Hand-rolled validators; no shared URL validator exists in the crate (only `valid_name/id/guest_path`), so nothing to reuse. Not a duplicate. | none |

No duplication; extension is clean. No edits to manager, host-agent, core or shared contract types, so no drive-by drift.

## Notes for F
None.
