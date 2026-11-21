# Handoff-T-red: Phase 4 - GitLab macOS runners on Tart pools, Tart agent side (#142), second run

**Date:** 2026-11-20
**Branch:** claude/gitlab-142-tart-13d6d4 (head 85150ff)
**Brief / wireframe reviewed:** docs/handoffs/142-brief.md (in full, unchanged since 12421a7); docs/handoffs/142/handoff-A.md (second pass); handoff-S.md; handoff-F.md; handoff-T-green.md (including the S test-only rework); wireframe N/A (no UI surface)
**Previous run's RED handoff:** this file at commit 48c5c0e. It holds the original RED proof against the base `b22c451`: the compile RED, the runtime RED on a stub scaffold, and mutations m1 to m15 on a draft.

## A precondition

Confirmed: A returned **PASS** on the plan in its second pass (`handoff-A.md`, commit 85150ff), with 8 warnings and no blocks.

**This run starts with the code already in place.** A's finding 8 says that HEAD already holds the previous run's complete implementation and my suite for it, which passed A-dup and then had S's test-only rework (6d5e2e1). So I did not write a second suite. Instead I:

- audited the existing 51 tests against every Done-when item and S's findings;
- extended them in place where I found a gap;
- proved RED by **removal**: I mutated the production code in a scratch copy and showed that each new or changed test fails for the right reason, and that the existing contract tests fail too.

On this branch the whole suite is GREEN before F does anything. That is expected and does not make the RED invalid. A classic "fails before the code exists" RED is impossible here, because the code exists. The evidence that the tests are a real contract is the mutation table below.

## Audit of the existing suite against the brief

Every Done-when bullet already had a test, and S's blocking finding 1 was closed by 6d5e2e1. Four gaps remained. Each is a branch of the shipped behaviour that I could delete with all 51 tests still green:

1. **A sibling of S's finding 1** in `hostile_urls_reach_gitlab_runner_verbatim_and_run_nothing`. Its `'` case still adapts to the implementation: it accepts either "validation refuses `'`" or "accepted and quoted", whichever the code does. So a change that refuses `'` in GitLab validation, together with reverting `shell_single_quote`, stays green. But it breaks Bitbucket, which accepts `'` and quotes with the same helper. I removed the conditional, so the expectation is now fixed.
2. **Script: a job received with no build-limit line.** The fake's job behaviours always print both lines. So deleting the script's `Checking for jobs... received` match stays green, and a runner that took a job and then stopped early would list 70 instead of `completed`. The brief says "0: the runner took a job (whatever the job's own result)".
3. **Waiter: a `tart exec` ended by a signal.** It has no exit status. Recording it as 0 (`code().unwrap_or(0)`) would list a killed runner as `completed`, and no test covered that path. The listing matrix only seeds `exit_code: None` directly.
4. **Waiter: the exit-code write fails.** A's finding 1 says to "log only the id and the error, and still stop the VM". A waiter that panics or returns on that error would leave the VM running (a leaked VM slot), and no test covered it.

Also, validation refuses tokens containing `"` or `\`. That refusal is the precondition for the guest script's unescaped JSON verify body (`gitlab.rs` `valid_token` doc comment), and it was unpinned. I added the two refusal rows.

**S's optional item 7 (trimming the refusal table) is decided as "keep".** The rows S grouped together share a predicate only in this implementation. As behaviour they are distinct inputs the brief names ("single line", "no whitespace or control characters"). An implementation that checked only `\n`, or only `' '`, would pass the trimmed table. So they are not redundant at the behaviour level.

## Tests authored (this run: changes in place, no new files)

| Test | Change | Criterion / behaviour pinned | Tier, and why |
|---|---|---|---|
| `gitlab::tests::each_invalid_gitlab_request_is_refused_without_its_token` | +2 rows, from 22 to 24: a token with `"`, and a token with `\` | Launch: invalid shapes refused with `BadRequest` and no token (B-1). Also the guest script's precondition that a token never breaks out of the verify call's JSON string | unit: `validate` is pure |
| `gitlab::tests::the_guest_script_exits_with_the_status_each_ending_maps_to` | +1 row, "a job taken, then stopped early" → exactly 0. `FAKE_GITLAB_RUNNER` gains a `job-taken` behaviour: the received line, then exit 0 | Script exit codes: "run-single behaves as having run one job → exactly 0" when no build-limit line follows | process: only bash running the real script shows it |
| `gitlab::tests::hostile_urls_reach_gitlab_runner_verbatim_and_run_nothing` | The conditional `'` block (refuse **or** accept) is replaced by a fixed hostile URL in the table, `https://gitlab.example.com/it';touch${IFS}pwned-quote;'s`, which must reach `--url` verbatim, exit 0 and create no `pwned*` file | Launch: "hostile URL/path values are single-quoted"; the brief's `'` edge case, under bash. A broken helper either fails to parse or runs the `touch` | process |
| `tests::a_gitlab_runner_gets_its_token_on_stdin_and_lists_its_exit_reason` | +1 row, `killed`: the fake `tart exec` ends with `kill -KILL $$`. The record holds **no** exit code (at stop, in memory, and after a reload), and the listing shows `exited` with no `exitReason`. The loop now carries the recorded code separately from the fake's ending | Exit mapping: "a GitLab runner with no recorded/unknown code lists no `exitReason`", reached through the real waiter, not seeded | integration: needs the waiter |
| `tests::a_gitlab_runner_whose_exit_code_cannot_be_written_is_still_stopped` | **new** | A finding 1: if the exit-code write fails, the VM is still stopped. A guard assertion (the record on disk has no exit code) proves the write really failed, so the test cannot pass vacuously | integration: needs the waiter and the record store |

**Fixture change (test code only).** `FAKE_TART`'s `exec -i` branch now waits while `exec-hold` exists, and given `exec-status` of `killed` it ends by `SIGKILL`. Otherwise it behaves as before. Every `main.rs` hunk is inside `mod tests` (line 2387 onward). No production code, script or doc was touched.

## RED confirmation

**On HEAD, with these tests:** `cargo test -p gridops-tart-agent` gives **52 passed, 0 failed** on 4 consecutive runs. `cargo fmt --all -- --check` is clean. `cargo +1.98.1 clippy -p gridops-tart-agent --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (the CI line) is clean.

**RED by removal.** Method:

- I copied the tree (without `target`) to the session scratchpad and gave it its own `CARGO_TARGET_DIR`.
- For each mutation, I applied it, ran the full `cargo test -p gridops-tart-agent`, and then restored the file.
- Afterwards, `diff -r` of the scratch `src` against the worktree's showed them identical.
- The worktree's production files were never edited.

Mutations of the behaviour the new and changed tests pin:

| Mutation of the production code | Fails (and only these, apart from a known pre-existing flake) | Failing assertion |
|---|---|---|
| N1: `valid_token` accepts `"` and `\` | `each_invalid_..._without_its_token` | `a token with a double quote: expected BadRequest, got Ok(())` |
| N1b: `valid_token` accepts `\` only | the same | `a token with a backslash: expected BadRequest, got Ok(())` |
| N2: the script no longer matches `Checking for jobs... received` | `the_guest_script_exits_with_...` | `a job taken, then stopped early`: 70, expected 0 |
| N3a: `shell_single_quote` reverted to the pre-#142 `'\"'\"'` form | `hostile_urls_...` | `...it';touch${IFS}pwned-quote;'s`: bash `\""s: command not found`, status ≠ 0 |
| N3b: `shell_single_quote` with no escaping | `hostile_urls_...` | the same URL: the quote broke out (`s: command not found`), status ≠ 0 |
| N4: waiter records `code().unwrap_or(0)` | `a_gitlab_runner_gets_its_token_...` | `exec killed`: left `Some(0)`, right `None` |
| N5: a failed exit-code write panics the waiter (`.expect`) | `a_gitlab_runner_whose_exit_code_cannot_be_written_is_still_stopped` | `the runner was not stopped` (the waiter panicked with `Is a directory (os error 21)`) |

The existing contract tests, re-checked on **F's actual code**. The previous run's main.rs mutations were made on my throwaway draft. S's script mutations A to E were already re-proved on F's code in the rework.

| Mutation | Fails | Failing assertion |
|---|---|---|
| E1: listing without the `platform == "gitlab"` check | `only_exited_gitlab_runners_...`, `github_and_bitbucket_runners_never_...` | `github: {... "exitReason":"completed" ...}` |
| E2: listing without the `exited` check | `only_exited_gitlab_runners_...` | `gitlab-running: {... "exitReason":"completed" ...}` |
| E3: the waiter persists and `insert`s a clone of the record | `deleting_a_gitlab_runner_as_it_exits_...` | `tart.state.records.read().await.is_empty()` |
| E4: the VM is stopped before the exit code is written | `a_gitlab_runner_gets_its_token_...` | `exec 77`: records-at-stop `None`, expected `Some(77)` |

Under N2, `tests::a_runner_waiting_for_a_slot_is_listed_as_waiting` also failed once. It is the pre-existing `FAKE_TART` lost-update flake that S reported (item 8) and T-green saw. It is not #142 code, and it passed in every other run.

Every new or changed assertion fails on the behaviour it names, and none fails on setup, an import or a typo. **The RED is valid.**

## Ready for F

**RED is valid. F may implement against these tests.** On this branch the implementation already satisfies them (52/52), so F's Phase 5 has no production change to make for this suite. Per A's finding 8, F should not rewrite anything.

Files to stage, by explicit path:

- `crates/gridops-tart-agent/src/main.rs` (tests-module hunks only)
- `crates/gridops-tart-agent/src/gitlab_tests.rs`
- `docs/handoffs/142/handoff-T-red.md`
- `docs/handoffs/142/evidence.md` (two repository facts and one `std` fact appended under "T (Phase 4, RED, second run)")
- `docs/handoffs/142/decisions.md`

**Notes:**

1. **The pre-existing `FAKE_TART` flakes** (`a_runner_waiting_for_a_slot_is_listed_as_waiting`, `every_tart_command_gets_a_path_starting_with_tarts_directory`) are still there. They are not #142's and are out of scope; S's advisory 8 already proposes a separate fixture-hardening issue.
2. **The new persist-failure test works as root.** It forces the failure with a directory at `<id>.json.tmp`, not with `chmod`, so it holds on a CI runner that runs as root.
3. **Scratch files** are under the session scratchpad (`.../scratchpad/mut`, `.../scratchpad/harness`), outside the repository.
