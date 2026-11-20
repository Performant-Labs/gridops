## Brief Review (Round 1)

### BLOCK findings
**[B-1]** The "Done when" section requires "Manual check: run the container with a real `glrt-` token from a test project. It shows online in GitLab, runs one job, and exits," but the Boundaries section states this check "cannot be run in this issue: no test project or token exists yet… not run, deferred to #143." The brief therefore contains an acceptance criterion that is internally acknowledged as unsatisfiable — a vacuous criterion, and two sections of the brief disagree. Fix: strike the manual check from "Done when" (or rewrite it as "deferred to #143; the PR states this explicitly"), so Done-when contains only criteria testable within #147.

### Needs-verification findings
**[NV-1]** That `gitlab-runner run-single` (at the pinned version) accepts the flags used (`--max-builds 1`, `--wait-timeout`, `--executor docker --docker-image … --docker-disable-cache`) — evidence: the pinned gitlab-runner version's CLI source or `gitlab-runner run-single --help` output.
**[NV-2]** That `run-single` can receive the `glrt-` token through a mechanism that keeps it out of the container main process's argv and `/proc/1/environ` (the brief itself flags env-on-child as unacceptable if the child is PID 1) — evidence: gitlab-runner source showing how `run-single` reads a registration token (flag/env/config-file/stdin), cited in the handoff.
**[NV-3]** That `run-single` exposes a distinguishable signal (log line or exit status) that a job was received, so exit 0 vs 75 can be mapped — evidence: gitlab-runner source location pinned in a constant and cited per the brief.
**[NV-4]** That GitLab's `POST {url}/api/v4/runners/verify` returns 200 for a valid `glrt-` token and 403 for a refused one (and accepts form-encoded `token`) — evidence: GitLab runner API documentation (`/api/v4/runners/verify`).
**[NV-5]** That a completed GitHub ephemeral runner's exited container today inspects as `Failed { ResourceUnavailable, retryable: false }`, which B-2's "always Failed{ResourceUnavailable} for exited GitLab containers, diagnostic None for exit 0" relies on to be "as the GitHub runner reports today" — evidence: `crates/gridops-host-agent/src/docker.rs` / `dispatch.rs` inspect branch for exited runners.
**[NV-6]** That the pinned `gitlab/gitlab-runner` image tag is Ubuntu-based and ships `bash` and `curl` (the script depends on both) — evidence: the image's Dockerfile/manifest at the pinned tag.

### WARN findings
**[W-1]** D1 (the merged contract, which "wins over the issue text") says `url` and `wait_timeout_seconds` "may be arguments," while R2 amendment W-3 mandates they travel as fixed-name env vars on both paths. Since the amendments win internally, D1's permissive wording is not a contract violation, but a reader of #137's contract alone could implement args. Recommendation: note the env-var choice against D1 in the PR/deviation table so #139/#142 implementors don't diverge.
**[W-2]** URL validation is specified only on the manager path (D10: "URL is an http(s) URL"). On the fleet path, `GitlabRunnerMaterial` is parsed with serde, which will accept any string; nothing in the brief rejects a non-http(s) or malformed `url` at Prepare/Start, and the verify `curl` would then fail as "retryable runtime failure" instead of a configuration error. Recommendation: specify refusal of non-http(s) URLs on the fleet path as `Failed { Configuration, retryable: false }`.
**[W-3]** `GITLAB_RUNNER_IMAGE` and `GITLAB_DEFAULT_JOB_IMAGE` are left as placeholders ("<pinned ubuntu-based tag>", "<pinned image>") — the choice is deferred to implementation with a headless registry check and a test that neither ends in `:latest`. That is testable, but the brief does not pin the values or the check's recorded evidence location. Recommendation: require the tag values and registry-check evidence in the PR description so review can confirm them.

### NIT findings
**[NIT-1]** B-2's verify snippet uses `"$S"` without saying where `S` is read (presumably `read -s` from stdin); spell it out so the "token never in env" property is auditable.
**[NIT-2]** "Boundaries" lists owned files that B-3/W-2 then extend (helper.rs, dispatch.rs, native.rs, runtime.rs, three test files, one API-crate test); a consolidated ownership list would prevent the boundary section from being read as exhaustive.

### Verdict
BLOCK — 1 blocking finding; must resolve before implementation starts.