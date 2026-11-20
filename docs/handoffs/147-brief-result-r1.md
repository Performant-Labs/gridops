## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** The claim that `gitlab-runner run-single` accepts/reads the `glrt-` token from stdin (the way the GitHub runner consumes `--jitconfig` on stdin) — evidence: gitlab-runner source (`commands/run_single.go` and the config/token-loading path) or official docs showing stdin token ingestion for `run-single`; if `run-single` only accepts `--token` or a config file, the "never in args" requirement conflicts with the chosen command.

**[NV-2]** The claim that `run-single --max-builds 1` exits with code 0 after exactly one job (the "one job then exit → completed" path) — evidence: gitlab-runner source for `run-single`'s max-builds/exit handling.

**[NV-3]** The claim that `--wait-timeout` expiration exits with a distinguishable code the script can map to 75 (`IdleExpired`) and token refusal to 77 — evidence: gitlab-runner source for wait-timeout and authentication-refused exit behavior (what exit codes/stderr the runner actually produces for each).

**[NV-4]** The claim that the agent's existing `requires_docker_socket` access is sufficient for the Docker *executor* running inside a `gitlab/gitlab-runner` container (i.e. socket mount actually lets the inner executor create sibling containers) — evidence: the current container-job Docker access implementation in `crates/gridops-host-agent/src/docker.rs` plus a run of the runner image with that mount.

**[NV-5]** The claim that the pinned exit-code table `[(0,"completed"),(75,"idle_expired"),(77,"registration_refused")]` matches the contract constants `GITLAB_EXIT_*` already merged in `b22c451` — evidence: excerpt of `gridops-core/src/provisioning.rs` from the merged commit showing the constants' values.

### WARN findings
**[W-1]** Exit code 0 is mapped unconditionally to `exitReason: "completed"` on the manager path. Any clean non-job exit (e.g. runner shut down by signal handling, or a future behavior change) would be reported as "completed". Recommendation: accept for now but note the ambiguity in the docs, or have the script emit a distinct code on the job-completed path if the contract permits.

**[W-2]** Done-when covers the token-hiding test and exit mapping, but does not list tests for the manager-path deliverables in D10/D15 (validation-arm behavior, `exitReason` absent/present on `GET /v1/runners`, the `ManagedRunner` re-parse tests D15 explicitly requires). Recommendation: add these to Done-when so the manager arm can't land untested.

**[W-3]** Two different exit-detection mechanisms are specified: the fleet path branches on `diagnostic.code` (D5) while the manager path branches on container exit status (D10). Since both consume the same script with the same exit codes, the brief should state how the host agent derives `IdleExpired`/`RegistrationRefused` diagnostics from the container (presumably the same exit codes) to avoid divergent implementations of the same contract.

**[W-4]** Error path under-specified: "refuse the GitLab start with a clear configuration failure" when the socket is missing — no required diagnostic code / outcome shape is named for that refusal. Recommendation: pin which `DiagnosticCode`/failure outcome it maps to.

### NIT findings
**[NIT-1]** The default image is "pinned to a version" but no pin or version-selection rule is given; name the pin (or delegate it explicitly) so it's testable.

**[NIT-2]** The default job image for `--docker-image` is referenced ("default job image") but never specified or given an override knob; state what it is.

### Verdict
PASS — no BLOCK findings; implementation may proceed.