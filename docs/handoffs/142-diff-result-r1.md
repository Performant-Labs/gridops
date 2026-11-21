## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** Sources/tart/Commands/Exec.swift:195-196 — Claim: `tart exec` propagates the guest command's exit status to the `tart` process's own exit code (the entire exit-reason feature depends on it). The excerpt appears only in the author-written Evidence Appendix, not in the SOURCE EXCERPTS section, so it is an assertion, not verified text. — Evidence: a verbatim excerpt of `Sources/tart/Commands/Exec.swift` and `Sources/tart/Root.swift` at tag 2.41.1 attached as independent source context.

**[NV-2]** gitlab-runner v19.5.3 `commands/single.go` / `network/gitlab.go` — Claims: `run-single` exits 0 in both the job-taken and idle-timeout cases (so grep-based classification is required), logs the exact strings `Checking for jobs... received`, `Checking for jobs... forbidden`, `This runner has processed its build limit`, `This runner has not received a job in`, and reads the token from `CI_SERVER_TOKEN`. The classification strings in the script are matched literally against these. — Evidence: independent excerpts of `commands/single.go` and `network/gitlab.go` at tag `v19.5.3` showing the exact `logrus.Println` format strings (note the strings are variadic-concatenated; the grep must match the rendered line, which the real-binary run in the handoff partially confirms but the "received → 0" path explicitly did not).

**[NV-3]** network/gitlab.go:451-466 and network/client.go:636 — Claims: `POST <url>/api/v4/runners/verify` returns 200 for a valid and 403 for an invalid `glrt-` token, and the API base is `<url>/api/v4/`. — Evidence: independent source excerpt of gitlab-runner `network/client.go`/`gitlab.go` at `v19.5.3`.

**[NV-4]** scripts/prepare-tart-gitlab-runner-image.sh:9-11 — Claim: the two pinned SHA-256 values match GitLab's published `release.sha256` for v19.5.3 darwin-arm64/amd64. The handoff says they were checked, but no verifiable artifact is in this prompt. — Evidence: the published `https://gitlab-runner-downloads.s3.amazonaws.com/v19.5.3/release.sha256` contents.

**[NV-5]** crates/gridops-tart-agent/src/main.rs:2301-2303 — Claim (appendix): `persist_record` writes a temp file and renames, so the exit-code write is atomic. Not in the SOURCE EXCERPTS (truncated before that line). — Evidence: an excerpt of `persist_record` in the working tree.

**[NV-6]** crates/gridops-tart-agent/src/main.rs:1811-1823 — Claim (appendix): `tart exec`'s argv carries only `/bin/bash -lc <script>` and the secret goes to stdin. Excerpt truncated before those lines. — Evidence: an excerpt of `start_runner`'s command construction.

### WARN findings
**[W-1]** crates/gridops-tart-agent/src/gitlab.rs:139-146 — The script remaps a `run-single` that itself exits with 75 or 77 to exit 70. This is a reasonable anti-collision measure and is documented, but it means a genuine status-75 exit from a future/different `gitlab-runner` build would be silently converted rather than passed through. — Recommendation: keep, but note in `docs/macos-runners.md` that 70 also covers a run-single that itself exited 75/77 (the docs currently describe 70 only as "stopped without taking a job").

**[W-2]** crates/gridops-tart-agent/src/gitlab.rs:158-166 — The idle/refused/completed classification depends on literal log strings of gitlab-runner 19.5.3. A pinned-version bump could silently break the mapping (e.g. idle runs listing as 70 → `exited` with no `exitReason`). — Recommendation: add a comment in the image script tying the pinned version to the string classification, and a check on the version in the script's handoff/docs.

**[W-3]** crates/gridops-tart-agent/src/gitlab.rs:151 — `curl ... --max-time 60 --retry 3` on verify: with transient network failure the runner exits 69 and lists as `exited` with no `exitReason`, indistinguishable from other failures. Acceptable per the brief ("anything else keeps its own status"), but the manager/reconciler has no signal to retry. — Recommendation: confirm #139 treats a no-`exitReason` exit as retryable.

**[W-4]** crates/gridops-tart-agent/src/gitlab.rs:171-172 — `GRIDOPS_BOOTSTRAP_SECRET` remains an (unexported) shell variable for the whole run-single duration; it is unset only afterwards. It is not in the child env (test asserts this), so this is depth-of-exposure only. — Recommendation: consider unsetting before `run-single` and passing `CI_SERVER_TOKEN="$GRIDOPS_BOOTSTRAP_SECRET"` computed into a separate var.

### NIT findings
**[NIT-1]** Evidence appendix (T, Phase 4) — T cites `main.rs:734` for `"stopped" => "exited"`, but the SOURCE EXCERPTS show that mapping at line 752 (line 734 is in the native-runner branch). F's citation (752) is correct; T's line number is stale.

**[NIT-2]** crates/gridops-tart-agent/src/gitlab_tests.rs:170 — The fake `gitlab-runner` echoes `token T0KEN-ab` (a truncated, non-secret placeholder), matching upstream masking; good, but consider a comment making clear this mirrors upstream truncation so a future "fix" doesn't print the real token shape.

### Verdict
PASS — no BLOCK findings; testing may proceed.