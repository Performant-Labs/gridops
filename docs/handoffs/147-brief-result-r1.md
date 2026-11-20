## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** The core design assumption that the pinned `gitlab-runner` version's `run-single` can receive the `glrt-` token via a mechanism that keeps it out of the container main process's argv and environ (B-2: "the implementor confirms from the pinned gitlab-runner version's source/docs how `run-single` takes its token"). If `run-single` accepts the token only as `--token <value>` or a documented env var, the entire token-hiding design fails and no test in "Done when" can pass. Settling evidence: the pinned gitlab-runner version's `run-single` flag/env source (e.g. `commands/run_single.go` and the common exec package for the pinned tag) or official documentation showing a stdin/file-based token input.

**[NV-2]** The `POST {url}/api/v4/runners/verify` endpoint exists, accepts form-encoded `token`, and returns exactly 200 for valid / 403 for refused `glrt-` tokens (B-2). Settling evidence: GitLab API documentation for the runners verify endpoint, or a source excerpt of the endpoint's handler.

**[NV-3]** `gitlab-runner run-single` produces a deterministically detectable signal (specific log line or exit-status distinction) separating "one job received and ran" from "wait timeout with no job", which the script can map to exit 0 vs 75 (B-2). Settling evidence: pinned-version source excerpt of the `run-single` command's wait/job-received paths.

**[NV-4]** `--wait-timeout` causes `run-single` to exit (rather than hang) and `--max-builds 1` causes exit after one job. Settling evidence: pinned-version source or docs for these two flags.

**[NV-5]** The GitLab Docker executor works when its daemon is the host's socket-mounted daemon while the runner itself runs in a container (see W-2). Settling evidence: GitLab docker-executor documentation on the socket strategy and builds-volume path handling, or a source excerpt showing how build-directory mounts are resolved.

**[NV-6]** Exit codes 75 and 77 are not already consumed by `gitlab-runner`/the base image's own failure exits, which would collide with the script's mapping table `GITLAB_EXIT_REASONS`. Settling evidence: pinned-version exit-status handling in `run-single` source.

### WARN findings
**[W-1]** Missing test for a stated in-scope deliverable: D6 requires the Linux Docker host agent to advertise `TargetKind::{GitlabProject, GitlabGroup}` in `supported_runners` (ephemeral mode), and the Scope says "whatever host-agent wiring is needed to advertise GitLab," but no "Done when" item or amendment names a test for that advertisement. Add an explicit test that the host agent's `supported_runners` includes both GitLab target kinds in ephemeral mode.

**[W-2]** The Docker-executor socket-aliasing hazard is unaddressed: when `gitlab-runner` runs in a container against the host's Docker socket, its Docker executor resolves build-volume and helper-image mount paths from inside the runner container while the host daemon interprets them — a classic mismatch that makes builds fail at the mount step. The brief covers *whether* the socket exists (W-5, manager refusal) but not *whether the executor functions correctly through it*. The deferred manual check (#143) is currently the only thing that would catch this; state the risk and how it will be detected before #143.

**[W-3]** The Start-time readiness wait timeout is left as "a pinned timeout such as 60 s" — no named constant, no value, no test. Pin it as a `GITLAB_*` constant with a test, like the other pinned values, otherwise the "bounded wait" criterion is untestable and reviewable only by reading the diff.

**[W-4]** Exit-code-0 ambiguity on the fleet path: an Inspect of a GitLab container that exited 0 for *any* reason (script bug, OOM-adjacent clean exit, base-image failure) yields `Failed { ResourceUnavailable, diagnostic: None }` and is treated as "completed." The contract accepts this, but the brief should state explicitly that "completed" is inferred solely from exit status 0 and not from evidence a job ran — otherwise the "one job then exit → completed" wording reads as a stronger guarantee than the mechanism provides.

**[W-5]** D10's validation list says "both fields present" without naming the two fields (presumably `url` and `gitlabRunnerToken`; `waitTimeoutSeconds` defaults per D12, and `image` handling is delegated per D147-1). Also unstated: whether the manager requires `image` on a GitLab Docker request or what happens when it is absent. Name the fields explicitly.

**[W-6]** Internal tension between "Done when" ("Manual check: run the container with a real `glrt-` token…") and Boundaries ("It is not run, deferred to #143"). Boundaries resolves it, but the Done-when section still lists an unrunnable criterion as a gate. Strike or mark it as deferred in Done when itself so the gate is not satisfiable only by reading a later section.

### NIT findings
**[NIT-1]** W-8's URL-quoting test criterion is "refused or quoted" — an either/or acceptance that any implementation trivially satisfies. Pick one: either a URL containing quotes/spaces is refused, or it is correctly quoted and verified.

**[NIT-2]** Exit codes 75/77 collide with `EX_TEMPFAIL`/`EX_NOPERM` sysexits conventions; harmless here, but a one-line comment in `provisioning.rs` noting the deliberate choice would prevent future "fix" churn.

### Verdict
PASS — no BLOCK findings; implementation may proceed, with the needs-verification items (especially NV-1, which the whole token-hiding design rests on) resolved during implementation and cited in the handoff as the brief already requires.