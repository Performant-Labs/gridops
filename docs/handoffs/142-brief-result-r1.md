## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** `tart exec` propagates the guest command's exit status (the whole 0/75/77 design depends on it) — upstream `tart` source or documentation (a quoted sentence from the `tart` repo showing `exec` exits with the guest command's status).

**[NV-2]** `gitlab-runner run-single` accepts `--url`, `--executor shell`, `--max-builds 1`, `--wait-timeout <seconds>`; reads its token from `CI_SERVER_TOKEN`; and its exit/output distinguishes an idle wait-timeout from a job run (passing *and* failing — see W-1) from a refused token — `gitlab-runner` source (e.g. `commands/run_single.go` and the token-reading code) or official docs, quoted in the handoff.

**[NV-3]** `POST <url>/api/v4/runners/verify` returns 403 for an invalid/revoked `glrt-` token — GitLab API documentation page or observed response, quoted.

**[NV-4]** The brief's claim that `start_runner` runs the script via `/bin/bash -lc` and that the Done-when `bash -c <script>` form is equivalent — the shown excerpt says `tart exec -i <vm> /bin/bash -lc <script>`; the equivalence of login-shell vs non-login execution for the script's builtins is plausible but not shown. A one-line note or local `bash -lc` vs `bash -c` check on the script's constructs settles it.

### WARN findings
**[W-1]** The failing-job → 0 requirement is under-specified relative to the verification it rests on. Done-when demands a fake `run-single` that "ran one failing job" still yields exit 0, but if `run-single` exits nonzero after a failed job, the script must distinguish "nonzero because a job failed" from "nonzero for any other failure" — and the brief elsewhere says "any other failure keeps its own non-table status". These two rules collide unless the script has a reliable job-ran signal. NV-2 as written only covers distinguishing idle vs job vs refused; make the failing-job detection mechanism explicit (output marker, per-build status, or a documented exit code) and cite evidence for it, or the test "another fake failure → its own status" may be indistinguishable from the failing-job case.

**[W-2]** Possible vacuous test requirement: Done-when says the `bash` + fakes test "runs at least one such quoted value [containing `'`] through `bash`", but the brief offers the implementer the option to refuse `'` in URLs instead of fixing `shell_single_quote`, and every other interpolated value is constrained (`valid_guest_path` disallows `'`; the token never enters the script text). If "refuse" is chosen, no script-interpolated value can contain `'` and the criterion is satisfiable only as a standalone unit test of the helper. State that the criterion may be met by a direct `shell_single_quote` output-through-`bash` test, so it cannot be silently dropped.

**[W-3]** Missing error path: the brief covers the agent-crash window after `tart exec` returns, but not `tart exec` itself failing to spawn or being killed (non-zero/nonsignal-status, or no exit status at all, e.g. signal termination). Specify what is recorded in those cases (no code → no `exitReason`), and add a fake-`tart` test case for a signal-terminated child if the implementation uses `status.code()`.

### NIT findings
**[NIT-1]** The shown `validate` excerpt's native-runtime refusal message is Bitbucket-worded ("Native macOS runners support GitHub only; Bitbucket runners require a VM.") and runs before the platform match, so a native GitLab request will be refused with a message that never mentions GitLab. Cosmetic, but worth either accepting explicitly in the handoff or routing GitLab to its own message.

**[NIT-2]** The Done-when "hostile URL/path values are single-quoted" is slightly redundant with the (stronger) explicit `bash` execution test; harmless, but one of the two should name the other's coverage to avoid divergent assertions.

### Verdict
PASS — no BLOCK findings; implementation may proceed.