## Brief Review (Round 1)

### BLOCK findings
**[B-1]** The Done-when criterion "exercised by running the generated script under `bash` against a fake `gitlab-runner`/`curl` on `PATH` **if feasible without a VM**" is not a testable acceptance criterion — the "if feasible" hedge lets the implementer skip the only test that exercises the script's exit-code behaviour (77/75/0) end-to-end, while the manual real-VM check is explicitly deferred to #143. Nothing in the brief then guarantees the core contract (which exit status the script emits in each situation). Fix: make the bash + fakes test unconditional (the brief's own test design — generated script, fakes on PATH, no `tart` — is feasible by construction), or specify exactly what makes it infeasible and what replaces it.

### Needs-verification findings
**[NV-1]** "`tart exec` propagates the guest command's exit status" — the whole exit-code capture design depends on this runtime behavior; no source excerpt is shown. Evidence: a `tart` source excerpt or documentation of `tart exec` exit-status semantics (or an existing test in `gridops-tart-agent` that demonstrates it).
**[NV-2]** `gitlab-runner run-single` accepts `--max-builds 1`, `--wait-timeout <seconds>`, reads its token from `CI_SERVER_TOKEN`, and exits distinguishably on "registration refused" vs "idle timeout" — the brief asks the implementer to show this, but the brief itself asserts flag names and env var. Evidence: `gitlab-runner` source (e.g. `commands/run_single.go`, token/env handling) or docs, cited in the handoff.
**[NV-3]** `POST <url>/api/v4/runners/verify` returns 403 for an invalid `glrt-` token — the proposed 77-detection mechanism. Evidence: GitLab `runners/verify` API documentation or rails source.
**[NV-4]** The pre-existing items the brief claims are "already on master" (`GitlabBootstrap`, `GITLAB_EXIT_*` tables, placeholder validate arm and its test, `GRIDOPS_TART_BITBUCKET_RUNNER_ROOT`, `shell_single_quote`, `valid_guest_path`, `start_runner` writing the bootstrap secret to `tart exec -i` stdin, exit-status-78-for-missing-binary convention "as the other scripts do") — no source excerpts are in this prompt. Evidence: `crates/gridops-tart-agent/src/main.rs` excerpts at the named items, and `scripts/prepare-tart-bitbucket-runner-image.sh`.
**[NV-5]** "Add it to whatever shell-lint coverage the repo already runs for the other `scripts/prepare-tart-*.sh` (check CI and `scripts/test-*.sh`)" — whether such coverage exists is unchecked. Evidence: the CI workflow file(s) and `scripts/test-*.sh`.

### WARN findings
**[W-1]** The verify-then-run-single design has a TOCTOU gap: a token valid at `verify` time can be revoked before/during `run-single`, and the brief permits the 77-detection to rest on either mechanism alone. Recommend requiring that a `run-single` failure that looks like refusal also maps to 77 (or explicitly document the gap in the handoff).
**[W-2]** "It must be written before the VM is stopped so a listing never sees `exited` without the recorded code when one exists" — if the agent crashes between `tart exec` returning and the record write, the invariant is violated on restart. Recommend stating the crash-window behavior explicitly (e.g. absent `exitReason` is acceptable for a crashed write) rather than an absolute invariant.
**[W-3]** The brief says the script "exits with exactly 0, 75 or 77" while also allowing "any other failure keeps its own non-table status" — the first sentence overstates. Recommend rewording to "exits with 0/75/77 in the three named situations" to avoid an implementer forcing unrelated failures into the table.

### NIT findings
**[NIT-N]** **[NIT-1]** The Done-when test list says "extra GitHub/Bitbucket secrets" while validation lists `jitConfig`, `bitbucket`, `bitbucketOauthClientSecret`; naming them consistently avoids a missed case.
**[NIT-2]** Exit status 78 (missing binary) is asserted "as the other scripts do" but is not added to the exit-reason table — fine, but the docs section should say a 78 exit lists with no `exitReason`.

### Verdict
BLOCK — 1 blocking finding ([B-1]); resolve the "if feasible" test criterion before implementation starts.