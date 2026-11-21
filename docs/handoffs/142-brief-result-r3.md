## Brief Review (Round 1)

### BLOCK findings
None.

### Resolved findings
**[R-1]** Earlier [B-1] (round 2): "`gitlab_runner_token: Option<SecretString>` … but the brief's own verified excerpt of `main.rs` shows `gitlab_runner_token: Option<String>`" — the amended contract section now states "**It is a plain `Option<String>`, not a `SecretString`** (corrected in brief review round 2, B-1 …)", restates the secret-handling obligations in terms of the plain field (no `Debug`/`Display`/log/error/BadRequest leak, `.take()` + `SecretString::from(...)` before `start_runner`), and adds an unconditional Done-when test ("Token Debug leak (review round 2, B-1)") asserting the token appears in no `BadRequest` message or error for each refused shape.
**[R-2]** Earlier [W-1] (round 2, `registrationToken` silently ignored) — the contract section now has an explicit "Enforcement gap for `registrationToken` (review round 1, W-1)" paragraph stating it is silently ignored, and requiring the handoff and PR body to record it as a contract deviation for the epic table.
**[R-3]** Earlier [W-2] (round 2, default binary path unspecified) — the scope now has "Default binary path (review round 1, W-2)": one named constant (`DEFAULT_GITLAB_RUNNER_BINARY`), a test that it passes `valid_guest_path`, agreement between agent, image script and docs, and the handoff naming the path.
**[R-4]** Earlier [W-1/W-2/W-3] (round 1: TOCTOU, crash window, "exactly 0/75/77" overstating) — all previously resolved and still addressed in the amended text.

### Needs-verification findings
**[NV-1]** `tart exec` propagates the guest command's exit status — the exit-code capture design depends on it; the brief confirms `tart exec --help` is silent. Evidence: tart source (exec command implementation) or upstream documentation of exit-status semantics, cited in the handoff.
**[NV-2]** `gitlab-runner run-single` accepts `--url`, `--executor shell`, `--max-builds 1`, `--wait-timeout <seconds>`, reads its token from `CI_SERVER_TOKEN`, and distinguishes idle-timeout / job-run / refused-token detectably. Evidence: `gitlab-runner` source (e.g. `commands/run_single.go`) or documentation.
**[NV-3]** `POST <url>/api/v4/runners/verify` answers 403 for an invalid/revoked `glrt-` token (only if the script uses verify). Evidence: GitLab runners/verify API documentation or rails source.

### WARN findings
**[W-1]** The brief leaves the 77-detection mechanism open ("pick a mechanism whose behaviour you can show") and NV-2/NV-3 are load-bearing assumptions the design rests on — if both verify-403 and run-single's distinguishable exits fail verification, the whole 77 contract fails. The brief already says "stop and report", which is the right posture; the handoff must cite concrete evidence for whichever mechanism ships.

### NIT findings
**[NIT-1]** Done-when's "Token delivery" test says "a test using a fake `tart` that records its argv and stdin is the strongest form" — that phrasing flirts with the round-1 "if feasible" problem; the unconditional "Script exit codes" test's fakes already record argv and environment, so the two criteria overlap. Minor redundancy, not a hedge worth blocking on.
**[NIT-2]** The exit-mapping criterion requires testing "the values the implementation actually writes for GitHub and Bitbucket runners" but the brief does not constrain what those platform-field values are; fine as worded (self-consistent), just noting the diff gate should check the tested values match the writer.

### Verdict
PASS — no BLOCK findings; implementation may proceed.