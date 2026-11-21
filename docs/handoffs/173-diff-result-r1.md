## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** scripts/prepare-tart-gitlab-runner-image.sh:~30 — the brief requires exit 69 for a missing `tart` binary; the diff's context does not show the `command -v`/binary-existence check that produces 69 (only the 64 path is visible), and the driver's "a missing tart exits 69" case depends on it — running `scripts/test-prepare-tart-runner-images.sh` locally (or reading the unchanged validation block of both scripts) settles it.
- **[NV-2]** crates/gridops-core/tests/gitlab_runner_script.rs:291 — the new test calls `is_reported(status)`; the helper's definition is not shown in this prompt — an excerpt of its definition in that test file would settle that the test compiles and means what it claims.
- **[NV-3]** handoff "Known issues" — the claim that the five `clippy::assert_is_empty` failures and `clippy::unused_async_trait_impl` predate this change cannot be checked from the prompt (the excerpts of autoscaling.rs/gitlab_tests/builtin_host.rs show nearby code but not the offending lines at the cited line numbers 1810/1896/288/384/119) — `git stash && cargo clippy …` on the base commit, or excerpts of those exact lines, would settle it.
- **[NV-4]** scripts/prepare-tart-gitlab-runner-image.sh:121-124 — the claim that Tart flushes the guest disk on `sync`-then-`tart stop`-then-`wait` is a runtime hypothesis supported only by #143's live rerun narrative; a real-VM run (post-merge, outside tests) is the evidence that would settle it, as the handoff itself acknowledges.
- **[NV-5]** .github/workflows/tart-installer.yml:104-114 — the claim that the macOS runner's `sh` is bash and the driver's `/bin/sleep 0.1`, `mktemp -d`, `ls` behaviour match the fake's assumptions is a CI-environment hypothesis; the first CI run of the workflow settles it.

### WARN findings
- **[W-1]** handoff Tier 1 — `cargo clippy --all-targets -- -D warnings` does not pass as-is (it needs `-A clippy::assert_is_empty -A clippy::unused_async_trait_impl`); the brief's Done-when requires clippy with `-D warnings` to pass. Even if the lints pre-exist on this branch, the gate as written is not met — route the lint fixes (or an explicit baseline exception) before merge.
- **[W-2]** scripts/test-prepare-tart-runner-images.sh:~250 — the interrupt cases send SIGTERM, never SIGINT; the scripts' `trap 'exit 130' INT` branch (and the handoff's hand-verified 130 path) is exercised only manually, not by the driver. Consider having `interrupt` send SIGINT in at least one case, or note the gap in the handoff permanently.
- **[W-3]** scripts/prepare-tart-gitlab-runner-image.sh:132 / bitbucket:113 — on the failed-check path the script calls `cleanup` explicitly and then `exit 1` fires the EXIT trap again; the second run is a no-op only because `cleanup` clears the state variables. This is correct but fragile under future edits — consider a `cleaned` guard flag.

### NIT findings
- **[NIT-1]** crates/gridops-core/src/provisioning.rs:~30 — the `GITLAB_EXIT_REASONS` doc comment still says only "a copy with the same names and values"; the brief asked to update docs that say the table is "exactly 0/75/77" — none is left, so this is fine, but the doc could now name the four reasons.
- **[NIT-2]** scripts/test-prepare-tart-runner-images.sh:~160 — the fake `tart run` holds a VM for up to 60 s of wall clock if never stopped; harmless with the stub `sleep`, worth a comment.

### Verdict
PASS — no BLOCK findings; testing may proceed.