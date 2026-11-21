## Brief Review (Round 1)

### BLOCK findings
None.

### Resolved findings
**[R-1]** Earlier B-1: "The 'Done when' section requires 'Manual check: run the container with a real `glrt-` token from a test project…' but the Boundaries section states this check 'cannot be run in this issue… not run, deferred to #143.' … a vacuous criterion, and two sections of the brief disagree." The brief now states in Done when: "Not run in #147: deferred to #143, as Boundaries records (no test project or token exists yet). The criterion testable within #147 is that the PR description states explicitly that this check was not run and is deferred to #143; it is not faked or simulated." The two sections agree; the criterion is now satisfiable within #147.

### Needs-verification findings
**[NV-1]** `gitlab-runner run-single` at the pinned version accepts `--url`, `--executor docker`, `--docker-image`, `--docker-disable-cache`, `--max-builds 1`, `--wait-timeout` — evidence: the pinned version's CLI source or `run-single --help` output, cited in the handoff (the brief itself now carries this as an R1 NV; still unverified from the prompt).
**[NV-2]** The mechanism by which `run-single` takes the `glrt-` token, and that it keeps the token out of the container main process's argv and `/proc/1/environ` — evidence: gitlab-runner source at the pinned version.
**[NV-3]** The distinguishable signal that a job was received (log line or exit status) mapping to exit 0 vs 75 — evidence: gitlab-runner source location, pinned in a constant and cited per the brief.
**[NV-4]** `POST {url}/api/v4/runners/verify` accepts form-encoded `token`, answering 200 for valid and 403 for refused — evidence: GitLab runner API documentation for `/api/v4/runners/verify`.
**[NV-5]** (verified in this round by excerpts) The exited-runner Inspect behavior — the shown excerpts confirm it: `docker.rs:312` `Ok(_) => failed(FailureCode::ResourceUnavailable, false)`, `helper.rs:67` `Failed { code, retryable }` with no diagnostic, `dispatch.rs:417-421` mapping to `diagnostic: None`, and the `docker_runtime.rs:82-84` test pinning the exited case. No finding; B-2's "as the GitHub runner reports today" claim matches the shown source.
**[NV-6]** The pinned `GITLAB_RUNNER_IMAGE` tag is Ubuntu-based and ships `bash` and `curl` — evidence: the image's Dockerfile/manifest at the pinned tag.
**[NV-7]** `GITLAB_START_READY_TIMEOUT_SECS = 15` and `GITLAB_VERIFY_MAX_TIME_SECS = 10` together fit within `START_COMMAND_DEADLINE_MILLIS` (30 s) minus `DELIVERY_MARGIN_MILLIS` (5 s) — evidence: `crates/gridops-reconciler/src/fleet.rs` (the constant's value and how the deadline bounds create + stdin feed + wait); the brief cites the value but the excerpt is not shown here. Note that verify (≤10 s) plus ready wait (15 s) alone sums to 25 s, leaving 0 s of the 25 s budget for container create and stdin feed if the marker only appears at the last moment — the arithmetic in the doc comment should be checked against the actual constant when the source is available.

### WARN findings
**[W-1]** Timing arithmetic is tight: `GITLAB_VERIFY_MAX_TIME_SECS = 10` consumed inside the container plus `GITLAB_START_READY_TIMEOUT_SECS = 15` of agent-side wait uses the entire 25 s budget (30 s − 5 s margin) before counting container create and stdin feed. Recommendation: either shrink one constant (e.g. ready wait 10 s, or verify 7 s) or have the doc comment justify the overlap explicitly, so a slow-but-healthy runner isn't reported as a retryable runtime failure at the deadline.

**[W-2]** The value of `GITLAB_EXIT_VERIFY_UNAVAILABLE` is not pinned (only "a distinct code, not in `GITLAB_EXIT_REASONS`, not 0/75/77"). Recommendation: pin a concrete value (e.g. 76 or 78) with a test asserting it collides with none of the table codes, so the implementor can't accidentally pick a code Docker or gitlab-runner uses for another meaning.

**[W-3]** Done-when's first criterion — "Tests show the token isn't visible through `docker inspect` or the container main process's `/proc/<pid>/environ`" — is only testable end-to-end if NV-2's mechanism lands; if NV-2 turns out false the brief says "stop and report," but Done-when has no fallback criterion. Recommendation: state explicitly that this test is written against the confirmed mechanism and blocks the PR if NV-2 fails, so Done-when cannot be partially satisfied.

### NIT findings
**[NIT-1]** B-3 says the helper creates `DiagnosticReference.id` "the way `cleaned()` creates its ids" — no excerpt shows `cleaned()`; fine as a spec, but the diff gate should confirm that constructor exists in `docker.rs`.

**[NIT-2]** R2 W-3 replaces the W-8 shell-quoting requirement with env vars, but W-8's section still says "W-8: a separate GitLab script … `url` is shell-quoted safely (test with a URL containing quotes/spaces is refused or quoted)". The brief notes the replacement inline, and fleet-path URL validation (R1 W-2) covers refusal — but the duplicated requirement across amendment rounds invites divergence. Recommendation: in the PR, state once that R2 W-3 supersedes the W-8 quoting sentence and that R1 W-2's URL refusal test is the surviving criterion.

### Verdict
PASS — no BLOCK findings; implementation may proceed.