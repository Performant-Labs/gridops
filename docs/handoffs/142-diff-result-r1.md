## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** scripts/prepare-tart-gitlab-runner-image.sh:10-12 — the pinned SHA-256 values for gitlab-runner 19.5.3 darwin/arm64 and darwin/amd64 are asserted but not verifiable from this prompt; a wrong checksum makes every image build fail (or, if both were wrong in a matching way, would need the release.sha256 file to confirm). — Evidence that would settle it: the published `https://gitlab-runner-downloads.s3.amazonaws.com/v19.5.3/release.sha256` contents compared against the two literals.
- **[NV-2]** main.rs (waiter in `start_runner`) — the claim that `tart exec` returns the guest's exit status is supported by the appendix's Tart `Exec.swift:195-196` / `Root.swift:164-168` excerpts, but the handoff itself notes the open sub-question of what `tart exec` reports when the VM is stopped externally (NV-1's residual). — Evidence: Tart source for the exec session's behaviour on VM stop, or a manual run on a real Mac (#143).
- **[NV-3]** main.rs:1897 (`shell_single_quote` fix) — the claim that `'\''` quoting is correct for `/bin/bash` and for the guest's bash 3.2 is exercised by the hostile-URL test in this environment, but the guest VM's actual shell version is not shown. — Evidence: the guest base image's `bash --version`, or a run of the hostile-value test against the prepared image (#143).

### WARN findings
- **[W-1]** crates/gridops-tart-agent/src/gitlab.rs:130-140 — `record_exit_code` in main.rs records an exit code for every VM runner (GitHub and Bitbucket included), not just GitLab; the `platform` gate in `list_runners` keeps it invisible, but the record file for non-GitLab runners now carries an `exitCode` field no consumer reads. — Recommendation: fine as-is; note it in the handoff so #147/#139 do not mistake it for a contract field.
- **[W-2]** crates/gridops-tart-agent/src/gitlab.rs:161-170 — the script remaps `run-single`'s own 75/77 exits to 70 and a silent exit-0 to 70; this is defensible (avoids a crashed runner listing as `idle_expired`) but 70 is a new undocumented-to-the-manager status. The docs table covers it; ensure the manager side (#147) does not need to know it. — Recommendation: state in the PR body that 69/70 never produce an `exitReason`.
- **[W-3]** crates/gridops-tart-agent/src/gitlab.rs:96-100 — refusing `"` and `\` in the token is a validation tightening beyond the brief's "single line, `glrt-` prefix" rule, motivated by the unescaped JSON body in the verify call. If GitLab ever issues tokens containing those bytes, valid tokens would be refused. — Recommendation: acceptable given real `glrt-` tokens are alphanumeric, but the constraint is a hypothesis about GitLab's token alphabet; keep it stated in the handoff.

### NIT findings
- **[NIT-1]** crates/gridops-tart-agent/src/gitlab_tests.rs — `a_gitlab_runner_gets_its_token_on_stdin_and_lists_its_exit_reason` parses `records-at-stop` as a single `TartRecord`; the fake appends every record file, so the test would break if another record existed in that fake home. Harmless today.
- **[NIT-2]** scripts/prepare-tart-gitlab-runner-image.sh:69 — `git --version >&2` prints the guest git version but the readiness loop's final `tart exec ... /usr/bin/true` after the loop can never distinguish "ready" from "90 tries exhausted" (both fall through); the Bitbucket script has the same shape, so this is inherited, not new.

### Verdict
PASS — no BLOCK findings; testing may proceed.