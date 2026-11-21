# Handoff-A: Phase 3 - GitLab: run gitlab-runner in Linux Docker runners (#147)  (up-front plan review, round 3)

**Date:** 2026-11-20
**Branch:** claude/gitlab-147-hostagent-e4a01d (head 7db0f88)
**Brief reviewed:** docs/handoffs/147-brief.md, including "Amendments (architecture review R1)", "Amendments (architecture review R2)" and "Amendments (brief review R1)"   **Reuse map:** N/A. No `survey.md` was provided. The baseline is the analogues the brief names: the GitHub branch of `docker.rs`, the manager's `runner_command` and `provision_runner_inner`, `instrumented_runner_script`, and the #137 contract (`b22c451`).   **Wireframe:** N/A (no UI surface)
**Round 1:** BLOCK at `5339e22`. **Round 2:** BLOCK at `2df6790`. This file replaces both, following the `docs/handoffs/137` precedent.
**Verdict:** PASS

## Summary

PASS, with 0 block findings and 2 warnings. The R2 amendments resolve the round-2 block (the Start readiness wait now fits inside the start-command deadline) and all four round-2 warnings. I checked each one against the code (see "Round-2 findings" below). The brief-review amendments add no drift: the fleet URL check sits where the material is parsed today, the token handling matches `docker.rs:128`, and the consolidated ownership list matches every compile-forced site.

The two warnings are clarifications for T and F:

- every Start failure after `docker create` must remove the container, including a refusal that lands inside `feed_secret`;
- the shared GitLab script's inputs (the env names and the http(s) URL rule) belong with the script in core.

## Round-2 findings (all resolved)

- **Finding 1 (block), the Start deadline. Resolved.**
  - `GITLAB_START_READY_TIMEOUT_SECS = 15` and `GITLAB_VERIFY_MAX_TIME_SECS = 10` are pinned in `provisioning.rs`, with a doc comment that cites `START_COMMAND_DEADLINE_MILLIS` (30 s, `gridops-reconciler/src/fleet.rs:48`) and `DELIVERY_MARGIN_MILLIS` (5 s, `registry/commands.rs:32`), and a test. `START_COMMAND_DEADLINE_MILLIS` is unchanged.
  - A verify that times out or gets no 200/403 exits with a distinct `GITLAB_EXIT_VERIFY_UNAVAILABLE`, outside `GITLAB_EXIT_REASONS`, and Start reports it as today's retryable runtime failure.
  - **The arithmetic holds, and the constants should not be shrunk.** The brief review's round-2 W-1 adds verify (10 s) to the wait (15 s) and gets 25 s. That double-counts: the script reads the token during `feed_secret` (`docker.rs:493-527`), so the verify runs **inside** the readiness wait, not before it. The budget is Docker create plus `feed_secret` plus at most 15 s:
    - Typical case: create under 1 s, `feed_secret` about 1 s (the Running poll plus its 0.5 s detach), verify 1-2 s. That is a few seconds, like today's GitHub Start.
    - Slow-GitLab case: `curl --max-time 10` ends the verify before the agent's 15 s wait runs out, so the script's exit, not the agent's timeout, ends the Start. `feed_secret`'s worst case is about 5.5 s plus inspect latency. Total: about 17-21 s, under 30 s minus 5 s.
- **W-2, compile-forced fallout. Resolved.** `grep -rn "HelperResponse::Failed" crates/` at `7db0f88` finds 15 sites in 8 files: `helper.rs` (3), `dispatch.rs` (1), `docker.rs` (1), `native.rs` (1), `runtime.rs` (1), `tests/dispatch.rs` (3), `tests/docker_runtime.rs` (1) and `gridops-api/tests/fleet_http/placement.rs` (4). All 8 files are in R2 W-2 and NIT-2.
- **W-3, non-secret values as env vars. Resolved in direction.** The script text is constant and the values arrive as fixed-name env vars, as `GRIDOPS_REGISTRATION_URL` does today. Where the names are defined is left open; see warning 2.
- **W-4, deviation D147-2. Resolved.** Recorded in the PR and on #136. #147 edits only the text of the `DiagnosticCode::RegistrationRefused` doc comment (`outcomes.rs:41-43`), with no type, variant or wire change. That edit is named in the brief and in NIT-2. No wave-1 issue owns `outcomes.rs`: epic #136's table gives it to #137, which is merged. So this is a spec'd edit, not a drive-by.
- **W-5, Docker executor leftovers. Resolved.** `--docker-disable-cache` removes per-runner cache volumes, and NV-1 covers confirming that flag. The orphaned job containers after a mid-job kill are recorded as a known gap for #143, in the PR and in `docs/fleet-agent.md`.

## Brief-review amendments (checked; no drift)

- **NIT-1, the token in a non-exported `S`.** This matches the GitHub branch (`docker.rs:128`, `IFS= read -r S`). With `--init`, PID 1 is Docker's init, so the main-process check in Done when (`docker inspect`, `/proc/1/environ`) is the same property the GitHub path keeps today.
- **W-2, the fleet URL check.** It belongs at Start, after the `GitlabRunnerMaterial` parse and before `docker create`: the material arrives only on Start, and that is where a material that won't parse is already refused (`docker.rs:117-119`). Right layer.
- **NIT-2, consolidated ownership.** It is complete:
  - The GitLab advertisement is the `supported_runners` JSON in `docker_backend` (`docker.rs:475-480`), so no other host-agent file is needed.
  - The manager test that asserts the placeholder message (`main.rs:3270`) is in `main.rs`, which #147 owns.
  - The `HelperResponse::Failed` sites are listed under W-2 above.
- **W-1, env vars against D1.** D1 says the values "may be arguments", so env vars are within it. Recording the choice next to D147-1 and D147-2 is enough.

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | B-2: "An exit with 77 during Start → `Failed { Configuration, retryable: false, diagnostic: Some(RegistrationRefused) }`. Any other exit or the timeout during Start → today's `Failed { Runtime, retryable: true }` behaviour." | cross-cutting (cleanup of a failed start); pattern consistency (reusing `feed_secret`) | **Only Start can remove a container that Start leaves behind.**<br>- Every later command needs the journaled `local_identity`, which only a successful Start binds (`dispatch.rs:256` and `:269`).<br>- The reconciler issues Cleanup only with `RunnerStarted` evidence (`gridops-reconciler/src/fleet.rs:505` and `:593`).<br>- So today's Start runs `docker rm -f` before it reports a failure after create (`docker.rs:247-249`).<br>**The brief specifies the 77 result but not the removal.** A refused Start that returns without `rm -f` leaves an exited `gridops-{environment}` container that nothing will ever clean up.<br>**The common refusal lands inside `feed_secret`, not in the readiness wait.** A 403 from a nearby GitLab can arrive within `feed_secret`'s Running poll plus its 0.5 s detach. `feed_secret` then fails its `running == "true" \|\| exit == "0"` check (`docker.rs:523-526`), and today's caller turns any `feed_secret` error into `rm -f` plus `Runtime, retryable: true`. That is the wrong outcome for a refusal, and a retryable one. | Make it explicit to T and F. In the GitLab Start, after any failure following `docker create` (exit 77, `GITLAB_EXIT_VERIFY_UNAVAILABLE`, any other exit, or the readiness-wait timeout):<br>1. read `State.ExitCode` first, including when `feed_secret` returned an error;<br>2. then run `docker rm -f`, as today;<br>3. then report the outcome: 77 gives `Configuration` with `RegistrationRefused`, and everything else gives today's `Runtime`, retryable.<br>The GitHub path's handling of a `feed_secret` error stays byte-for-byte unchanged. Tests: a runner that exits 77 immediately after reading stdin gives `Configuration` with `RegistrationRefused` and leaves no `gridops-{environment}` container; a readiness-wait timeout leaves none either. |
| 2 | warn | D10: "the same runner script as the fleet host agent (one shared script in `gridops-core/src/provisioning.rs`)"; R2 W-3: "fixed-name environment variables (for example `GRIDOPS_GITLAB_URL`, ...)"; brief review W-2: the fleet path checks `url` "the way the manager path does" | abstraction level (shared seam); pattern consistency | **For GitHub, each caller owns its own script and env names.**<br>- `docker.rs:145-154` and `main.rs:2676-2699` each hold a script and the env literals it reads.<br>- That works because no script is shared.<br>**#147 moves the GitLab script into core, so its inputs become a contract across crates.**<br>- The env names would be spelled in three places: the core script, `docker.rs` and `main.rs`. A typo in one caller leaves the variable empty, and that shows up only at runtime (`run-single --url ""`).<br>- The http(s) URL rule would be implemented twice: the manager's `validate` and `docker.rs` Start.<br>R2 W-3 doesn't say who owns the names. There is no precedent for a shared script's inputs, so this is a warn, not a block. | Keep the script's inputs with the script in `provisioning.rs`:<br>- Either a `gitlab_runner_env(url, wait_timeout_seconds) -> Vec<String>` that both callers pass as container env (it fills the job image from `GITLAB_DEFAULT_JOB_IMAGE` itself), or `pub const` names that both callers use.<br>- One core check for "parses as a URL with scheme `http` or `https`", used by the manager's `validate` and by the fleet Start. Both crates already depend on `url`.<br>Test that the script references every name the env builder sets. |

## Notes for O

None required (PASS). Both warnings are for T and F to act on in this run:

- **Finding 1:** T writes the two tests named there, and F routes exit 77 before `feed_secret`'s generic error path.
- **Finding 2:** F puts the env builder (or the name constants) and the URL check in `provisioning.rs`.

Neither needs a brief amendment or a follow-up issue.

## Patterns referenced

- `crates/gridops-host-agent/src/docker.rs`: Start (`:102-251`, including the material parse at `:117-119`, the token read at `:128`, the env literals at `:145-154` and the failed-start `rm -f` at `:247-249`), `feed_secret` (`:493-527`), and the `docker_backend` advertisement (`:475-480`).
- `crates/gridops-host-agent/src/dispatch.rs:240-290` (Start needs an owned environment; Inspect, Stop and Cleanup need the journaled `local_identity`) and `:414-422` (`outcome`).
- `crates/gridops-reconciler/src/fleet.rs:36-48` (`START_COMMAND_DEADLINE_MILLIS`), `:483-566` (`follow`) and `:568-610` (`finish`). Also `crates/gridops-core/src/fleet/registry/commands.rs:32` (`DELIVERY_MARGIN_MILLIS`).
- `crates/gridops-manager/src/main.rs`: `ProvisionRunner::validate` (`:385-490`), `runner_command` (`:2655-2704`), `valid_registration_url` (`:2776`) and the placeholder test (`:3270`).
- `crates/gridops-core/src/provisioning.rs` (the `GITLAB_*` constants, `instrumented_runner_script`), `fleet/protocol/outcomes.rs:36-43` and `fleet/protocol/agent.rs:125-151` (`GitlabRunnerMaterial`). Also epic #136's ownership table (`gh issue view 136`).
