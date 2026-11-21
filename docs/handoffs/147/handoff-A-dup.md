# Handoff-A-dup: Phase 7 - GitLab: run gitlab-runner in Linux Docker runners (#147)  (anti-duplication gate)

**Date:** 2026-11-20
**Branch:** claude/gitlab-147-hostagent-e4a01d
**Diff base:** 7db0f88   **Diff head:** ec7ffda
**Reuse map:** N/A (no survey.md); baseline is the analogues the brief names (GitHub branch of `docker.rs`, manager `runner_command` / `take_bootstrap_secret` / `provision_runner_inner` / `list_runners`, `instrumented_runner_script`)
**Verdict:** PASS

## Summary
F extended the analogous objects and built no parallel path. Every new object is one the brief names and justifies in writing (W-8 separate script, B-2 readiness wait, B-3 helper diagnostic). The one new seam F added unprompted, `create_args`, folds the shared create flags so both branches call one method rather than copying them.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `crates/gridops-host-agent/src/docker.rs` (`start_gitlab`) | The "retried start finds the existing container" inspect and the `docker create` / `docker()` / `rm -f` sequence repeat the shape of the GitHub `start`. It is small, and the divergent parts (token and material parse, socket binding, readiness wait) differ, so folding further would over-abstract. | None required. Note for future work: if a third platform lands, extract a shared "create then feed" skeleton. |
| 2 | warn | `crates/gridops-host-agent/src/docker.rs` (`start_gitlab_runner`) vs `feed_secret` | A second stdin-feeding routine exists beside `feed_secret`. The brief (B-2) justifies it: `feed_secret` detaches after 0.5 s, cannot see the readiness marker and cannot tell exit 77 from other exits. `feed_secret` is unchanged for GitHub. | None. Justified in writing. |

Checks made:
- **URL validation.** `valid_gitlab_url` is the only GitLab URL validator, in core, and both the manager and the fleet path call it. The manager's `valid_registration_url` is GitHub-host specific (`https://github.com/`), so it is not a duplicate.
- **Script.** `gitlab_runner_script` is its own function, as W-8 requires, shared by the manager and the fleet agent (one script, no copy). The exit-code constants and `GITLAB_EXIT_REASONS` are used directly, with no copies in the manager or the host agent.
- **Material.** The GitLab branch parses the core `GitlabRunnerMaterial` directly. `Material` in `docker.rs` has no private copy (W-4).
- **Helper protocol.** `HelperResponse::Failed` gained one optional field, and `dispatch::outcome` passes it through. No new response type.
- **Manager.** The GitLab validation arm replaces the placeholder arm in place. `runner_command` and `take_bootstrap_secret` gain platform branches. `ManagedRunner` gains `exit_reason`, and the listing uses `inspect_container` with the core table. The `stand_in_docker.rs` test seam extends the existing stand-in (additive fields, a fallback to the old `answer`).
- **Boundaries.** No edits to the tart agent, reconciler, API src, `fleet/targets.rs` or the shared contract types. `outcomes.rs` is a doc-comment change only, as the brief permits. The API-crate edit is only the test `placement.rs`, which the brief allows.
- **Layering and direction.** Core holds constants, script and validation. The host agent and manager depend on core, not the reverse. No new dependencies.
- **Behaviour change outside GitLab.** The manager no longer sets `RUNNER_ALLOW_RUNASROOT` on GitLab containers only. GitHub is unchanged.

No duplication; extension is clean.

## Notes for F
None (PASS).
