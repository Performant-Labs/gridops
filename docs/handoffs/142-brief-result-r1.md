## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** `tart exec` propagates the guest command's exit status (the whole 0/75/77 design rests on it; `tart exec --help` at 2.41.1 says nothing) — the `tart` source's exit-status handling for `exec` (file:line in the tart repo, or a maintainer-documented statement) would settle it.
- **[NV-2]** `gitlab-runner run-single` accepts `--url`, `--executor shell`, `--max-builds 1`, `--wait-timeout <seconds>`, reads its token from `CI_SERVER_TOKEN`, and its idle-timeout / job-run / refused-token exits are distinguishable as assumed — `gitlab-runner` source (e.g. `commands/run_single.go` and the wait-timeout handling) or official docs, with quoted lines.
- **[NV-3]** `POST <url>/api/v4/runners/verify` answers 403 for an invalid/revoked `glrt-` token (only if the script uses verify) — GitLab API docs page or `gitlab-runner` source citing that endpoint and its 403 behaviour.
- **[NV-4]** The brief asserts the shell executor ignores a job's `image:` (docs section, item 4) — a GitLab documentation page on the shell executor stating this, cited in the handoff.
- **[NV-5]** `tart exec -i` forwards the host-side stdin bytes to the guest command's stdin (the token-delivery path; the Bitbucket arm uses the same mechanism but no source for it is shown in this prompt) — tart source line handling `exec` stdin piping.

### WARN findings
- **[W-1]** Exit-status persistence: if the `tart exec` child dies from a signal, `status.code()` is `None`; the brief's table ("an exit status not in the table yields no `exitReason`") implicitly covers it, but the code comment required for the crash window should also name the signal case so `list_runners` handles a `None` code explicitly rather than accidentally.
- **[W-2]** The exit-capture tests need a `FAKE_TART` variant that exits with a scripted status; the brief notes this but does not list it as a Done-when test item by name. Add it to the Done-when test list (record the argv/stdin-recording fake is strengthened, or a second fake added) so the diff gate can check it exists.
- **[W-3]** The script-exit-code Done-when test asserts the token reaches `gitlab-runner` "only through `CI_SERVER_TOKEN`" — but the script also writes the token to curl's stdin in the verify mechanism. The test must additionally assert the fake `curl` receives the token on stdin and in no argv (the brief says this under the script spec but the Done-when test bullet only mentions `gitlab-runner`); make the Done-when bullet cover both fakes explicitly.

### NIT findings
- **[NIT-1]** Scope item 3 opens with "Add it to whatever shell-lint coverage the repo already runs" and then, mid-paragraph, concludes there is none and none is added. State the conclusion first to avoid a momentary contradiction with the instruction to add coverage.
- **[NIT-2]** The `shell_single_quote` fix-vs-refuse decision is left as a handoff choice; prefer fixing the helper (it is in this crate and other platforms' scripts share the defect) — but either resolution satisfies the brief, so this is a recommendation only.

### Verdict
PASS — no BLOCK findings; implementation may proceed.