# Handoff-F: Phase 5 - GitLab macOS runners on Tart pools, Tart agent side (#142), second run

**Date:** 2026-11-20
**Branch:** claude/gitlab-142-tart-13d6d4 (head 3d960f1, T's second-run RED)
**Issue:** #142
**Previous run's F handoff:** this file at commit 0f08494. It holds the full account of the implementation, with its design decisions, its real-path checks against the real gitlab-runner 19.5.3 binary and curl, and the S-rework note.

## What was done

**No production file changed in this run.** The reasons:

- A's finding 8 (second pass) says the branch already holds the complete implementation, and it must be extended, not written again.
- T's second-run RED (`handoff-T-red.md`) extended the suite in place: +2 token-refusal rows, a "job received, then stopped early" script row, a fixed expectation for the hostile `'` URL, a `killed` waiter row, and a new persist-failure test. T showed RED by removing behaviour, with mutations N1 to N5.
- I checked each pinned behaviour against the source, and the existing code already has it (below). So driving the suite GREEN needs no change.

| New or changed test | Production code that already meets it |
|---|---|
| Token `"` and `\` refused | `gitlab.rs:94-101`, `valid_token`: `is_ascii_graphic() && !matches!(byte, b'"' \| b'\\')` |
| A job taken, then stopped early → exit 0 | `gitlab.rs:167`, which matches `Checking for jobs... received` or the build-limit line, then exits 0 |
| A hostile `'` URL reaches `--url` verbatim | `main.rs:1899-1901`, `shell_single_quote`, which uses `'\''` |
| A `tart exec` killed by a signal records no code | `main.rs:1843`: the waiter calls `record_exit_code` only when there is a `code()` |
| A failed exit-code write still stops the VM | `main.rs:1861-1870`: a `persist_record` error is logged (`runner_id` and `error`), and the function returns normally, so the waiter goes on to `tart stop` |

## Design decisions

None are new in this run. The previous run's decisions stand: see this file at 0f08494.

## Reuse / extend-vs-new

Nothing new was created. Per A's finding 8, the existing objects are the ones extended. They are `gitlab.rs`, the `main.rs` wiring, `scripts/prepare-tart-gitlab-runner-image.sh` and the "GitLab" section of `docs/macos-runners.md`, and this run made no further change to any of them. The previous run's reuse map is unchanged, and A-dup passed it (`handoff-A-dup.md`).

## Architecture notes for A

None. Nothing changed in this run. The previous run's notes still apply, for O's PR body:

- the persisted `platform` and `exitCode` fields;
- `exitReason` on the wire;
- `GRIDOPS_TART_GITLAB_RUNNER_BINARY`;
- the waiter as a second writer to the record store, update-in-place only;
- a stray `registrationToken` is ignored, not refused;
- the end-to-end path can't be reached until #147 lands.

## Deviations from spec / wireframe

None. Wireframe: N/A.

## Tier 1 self-check (incl. tests now GREEN)

    $ git status --short                       -> clean (HEAD 3d960f1)
    $ cargo test -p gridops-tart-agent          (3 consecutive runs)
    test result: ok. 52 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 14.04s
    test result: ok. 52 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 13.73s
    test result: ok. 52 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 13.68s
    $ cargo fmt --all -- --check                -> clean
    $ cargo +1.98.1 clippy -p gridops-tart-agent --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl
        Finished `dev` profile [unoptimized + debuginfo] target(s)   (no warnings; CI's pinned toolchain and flags)
    $ bash -n scripts/prepare-tart-gitlab-runner-image.sh && shellcheck scripts/prepare-tart-gitlab-runner-image.sh   -> clean

None of the pre-existing `FAKE_TART` flakes reproduced in these three runs.

## Evidence appendix

None new from F in this run. No production change relies on unchanged code. The previous run's F entries and T's second-run entries in `docs/handoffs/142/evidence.md` still apply.

## Tests that look wrong (for T)

None of T's #142 tests. The two pre-existing flaky fixtures (`a_runner_waiting_for_a_slot_is_listed_as_waiting` and `every_tart_command_gets_a_path_starting_with_tarts_directory`) are not #142's. S's advisory 8 already proposes a separate issue to harden those fixtures.

## Known issues

These carry over from the previous run, and none is a production gap in #142:

- A token revoked while `run-single` is already polling surfaces as exit 77 only after gitlab-runner's 300-second not-healthy sleep. That timing is upstream behaviour.
- NV-1 is still open: it is unsettled whether `tart exec` returns the guest's status after an outside `tart stop`. This is a runtime question, and the `killed` row pins only the agent's side of it.
- The manual check with a real `glrt-` token on a real Tart VM is deferred to #143. It also can't be reached until #147 lands.

## Files changed

None in this run. The handoff files `docs/handoffs/142/handoff-F.md` and `docs/handoffs/142/decisions.md` are updated, but they are not production code.
