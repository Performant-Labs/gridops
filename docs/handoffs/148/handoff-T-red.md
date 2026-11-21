# Handoff-T-red: Phase 4 - GitLab demand matching, scale-down and a pending-jobs poll (#148)  (round 2: D18)

**Date:** 2026-11-20
**Branch:** claude/gitlab-148-scaling-71c514 (head 4a1fbbd)
**Brief / wireframe reviewed:** docs/handoffs/148-brief.md (all 271 lines; this round adds the "Amendments, round 3: the shared `gitlab_jobs` write (D18, agreed with #140)" section and its Constraints line, read with `git diff 1a4c512 572f828`). Also read: #140's D18 text, `gitlab-140-webhooks-a9fae5/docs/handoffs/140-brief.md:280-372` at `ba0a441`. Wireframe N/A (no UI surface).

## A precondition

Confirmed: A returned **PASS** on the amended plan in round 4 (`docs/handoffs/148/handoff-A.md`, commit 4a1fbbd), with 6 warnings. The brief was not amended after that, so no test pins a warning's suggested fix. Here is how each warning bears on the tests:

- **Warning 1** (the dedupe key and where it runs): the new dedupe test pins what the brief states: dedupe on `(base_url, job_id)`, demand counted once, candidate pools from either connection. It doesn't pin the display tier or the private key's shape.
- **Warning 2** (one row `running`, its duplicate `pending`): not adopted, so not pinned.
- **Warning 3** (where the D18 tests live): I put them in `gitlab/jobs_tests.rs`, as A suggested. I mounted them from `gitlab/mod.rs`, not from `jobs.rs`; the reason is under "Test wiring".
- **Warning 4** (status guard on the re-check mark): not adopted, so not pinned.
- **Warning 5** (`runner_events`): the test pins the brief's fields. It doesn't pin the message, and it doesn't pin the duplicate-row case.
- **Warning 6** (a failed `group_projects`; the interval): not adopted. I check the interval by reading the code in Phase 6.

A's "Notes for O" asked T for three items: the D18 unit tests, the dedupe test and the `runner_events` row. All three are below.

## Tests authored

The first T-red (`1a4c512`) authored 26 tests. This round adds 10 tests and changes 1, for 36 in total. Every earlier test was re-read against D18. They are consistent with it: both fixtures put each connection on its own base URL, so D18's `base_url` busy match and the dedupe leave their expectations unchanged.

There are three tiers, and each is the cheapest one where the rule actually runs:
- **The D18 writer** is unit-tested against a real migrated SQLite database through its public API (`record_gitlab_job`, `set_gitlab_runner_busy`, `status_tier`, `status_write_applies`). The rules are SQL, so a database is the cheapest sufficient tier.
- **Demand matching** is tested through the public `autoscaling` API on SQLite.
- **The poll** is tested through `gitlab_poll::poll_once` on SQLite with `FakeGitLab`.

### `crates/gridops-core/src/gitlab/jobs_tests.rs` (new, 9 tests): D18's Done-when list

| Test | What it pins |
|---|---|
| `status_tier_ranks_every_status_into_its_d18_tier` | Tier 0 is `created`, `waiting_for_resource`, `preparing`, `manual` and `scheduled`. Tier 1 is `pending`, 2 is `running`, and 3 is the four terminal statuses. `unknown` has no tier (`None`). |
| `a_status_write_applies_only_upward_or_at_the_same_tier_when_newer_and_never_to_a_terminal_row` | **Each tier transition.** All 11 × 11 stored/incoming status pairs, each with the incoming time older, equal and newer (363 cases). It checks both `status_write_applies` and `record_gitlab_job`. When the write applies, the outcome is `StatusApplied` with `previous_status` = the stored status. Only `status`, `status_changed_at`, `pipeline_id` and `updated_at = now_ms` change, and `pool_id` (seeded `p1`) is never written. Otherwise the outcome is `Ignored` and the row is unchanged in every column. This also pins the same-tier rule: equal times don't apply. |
| `a_new_row_gets_a_uuid_and_the_given_values_or_the_defaults` | An `Inserted` row has a UUID v4 `id`, the given tags (contract encoding), retries and runner, `pool_id` NULL and `updated_at = now_ms`. The outcome has `previous_status` None. With no optional values the row gets `[]`, 0 and NULL. `Some([])` is stored as `[]`. |
| `unknown_writes_nothing` | `unknown` gives `Ignored`. It creates no row, and on a row with room for every fill it fills nothing. *Control:* the same observation with `Pending` is `Inserted`. |
| `a_terminal_row_never_changes_in_any_column` | Each of the 4 terminal statuses gets a newer `pending`, `running`, `success` or `failed`, with tags, a runner, retries and a pipeline. Each is `Ignored`, and every column is unchanged. *Control:* the same observation onto a `running` row is `StatusApplied`, and it fills the runner and the tags. |
| `the_runner_id_is_filled_once_and_never_cleared_or_overwritten` | A rejected (older) write fills `gitlab_runner_id`. The outcome is `FilledOnly` with `previous_status`/`status` = `running`, and only the runner and `updated_at` change. An applied write with another id keeps the first id, and an applied write with `None` doesn't clear it. An applied write fills an empty id. A rejected write onto a row that already has a runner is `Ignored`. |
| `tags_are_replaced_only_by_non_empty_tags_and_filled_on_a_rejected_write` | On applied writes, `Some([])` and `None` keep the stored tags, and non-empty tags replace them. A rejected write fills empty stored tags (`FilledOnly`, and only the tags and `updated_at` change). A rejected write never replaces stored tags and never "fills" with `[]` (`Ignored`). |
| `retries_count_and_pipeline_move_only_with_an_applied_status_write` | `None` keeps the stored retries, `Some` moves them, and `pipeline_id` moves with an applied status. On a rejected write, neither moves (`Ignored`). |
| `set_gitlab_runner_busy_matches_the_runner_id_on_any_connection_of_the_same_instance` | **The `base_url` match.** The job's connection is `conn-a`. The runner that is matched was registered through `conn-a-group`, a sibling connection on the same base URL. These runners are not matched, and none of them changes: runner 55 on another instance, a deleted 55, runner 56, and a `github` runner carrying 55. Setting busy writes `busy=1`, `status='busy'`, `last_heartbeat_at` and `updated_at`; clearing it writes `0` and `'online'`. No other column of any runner changes. The call returns `("sibling", Some("p1"))`, or `None` for an unknown id. |

### `crates/gridops-core/src/autoscaling_gitlab_tests.rs` (+1 test, 11 in total)

| Test | What it pins |
|---|---|
| `one_gitlab_job_recorded_under_a_project_and_its_group_connection_is_one_job` | **The D18 dedupe.** `conn-a` (project) and `conn-a-group` (group, same base URL) each record job 501. With one pool linked to both connections, demand is **1**, not 2. With one pool per connection, the pools of either connection are candidates and the job is owned once: the older pool (`group-only`) gets 1 and the other gets 0. A per-row scan gives (1, 1). A dedupe that keeps only the first row's connection gives (0, 1). The case where the same id is on *different* instances stays at 3, in the existing collision test. |

The other 10 tests are unchanged (see the round-1 table in git history at `1a4c512`). They cover: tag matching; connection linkage; untagged jobs; A-B4 matching on configured labels only; the A2-W5 platform gate; the A-B3 tiers and D8 statuses; the display tier taking room; A-W4's one ordered list; the A-B2 rule that ids never collide; and the saturated-pool handoff.

### `crates/gridops-reconciler/src/gitlab_poll_tests.rs` (1 test changed, 16 in total)

| Test | Change |
|---|---|
| `concluding_a_running_row_clears_its_runners_busy_flag_and_records_one_runner_event` (was `concluding_a_running_row_clears_its_runners_busy_flag`) | The test now also checks:<br>- the freed runner's `status` is `online` (D18);<br>- **exactly one** `runner_events` row matches `event LIKE 'GitLab job %'`: `runner_id` `ran-it`, `pool_id` `gl`, `level` `info`, `event` `GitLab job failed`, and metadata `jobId` 841, `projectId` 11, `pipelineId` 1, `connectionId` `conn-a`. Each value may be a JSON number or a string;<br>- a second `running` row (842) with **no** `gitlab_runner_id` is concluded (`success`) and frees no runner and writes no event.<br>The runner on another instance (`conn-b`, `BASE_B`) stays busy, as D18's `base_url` match requires. |

The other 15 poll tests are unchanged. They cover: recording a job; the group poll; concluding through each `job` outcome; the rate-limit backoff; the A2-B1 limit, age and order; the re-check stamp; no backwards move; no resurrection; manual→pending; the A2-W6 failed-project rule; `gitlab_credentials`; `UnavailableGitLab`; `maybe_scale_down`; and the constants. They already conclude only through `GitLabApi::job` (they count `FakeMethod::Job` calls). D18 doesn't change any of them.

### Test wiring (not production code)

- `crates/gridops-core/src/gitlab/mod.rs` gains `#[cfg(test)] #[path = "jobs_tests.rs"] mod jobs_tests;`, after `pub mod fake;`. This is a test-only line. A suggested mounting from `jobs.rs` instead (the `fake.rs` pattern). I mounted from `mod.rs` so the tests stay independent of whichever copy of `jobs.rs` is used. If #140's `jobs.rs` replaces #148's at the second merge, these tests keep running against it. That gives the "union of both test sets" A asked for, without anyone re-mounting them. The tests use only `crate::gitlab::jobs::` public items.
- The round-1 wiring is unchanged and already committed: the reconciler's `gridops-core` dev-dependency with `test-support`, `mod gitlab_tests` in `autoscaling.rs`, and `mod gitlab_poll_tests;` in `main.rs`.

**Coverage holes, by design:**
- The timeout path (round 1) still can't be driven.
- The poll's use of D18 isn't directly observable. That the poll calls `record_gitlab_job`/`set_gitlab_runner_busy` and has no SQL merge of its own is checked by reading the code in Phase 6.
- The re-check mark being a one-column `UPDATE` is also checked by reading the code in Phase 6.

## RED confirmation

### Compile RED (the real state of the branch)

`cargo test -p gridops-core --lib --no-run`:

    error[E0432]: unresolved import `crate::gitlab::jobs`
      --> crates/gridops-core/src/gitlab/jobs_tests.rs:33:20
    error: could not compile `gridops-core` (lib test) due to 1 previous error

`cargo test -p gridops-reconciler --no-run` gives the same 8 errors as round 1. Each names a missing #148 item:
- E0432 `crate::gitlab_poll`;
- E0560 `Reconciler.gitlab`;
- E0425 `gitlab_credentials` ×3;
- E0277/E0599, knock-on effects of the missing `gitlab_credentials`.

**Consequence F should expect:** the core test binary won't compile until `gitlab/jobs.rs` exists. Until then, none of core's tests can run, including the 24 existing autoscaling tests. Add `jobs.rs` (even a skeleton) first.

### Runtime RED behind the compile RED (throwaway stubs, never staged)

Each stub came from the session scratchpad, was applied for one run, and was then removed (`git checkout` / `rm`). `git status` now shows only the four test paths.

**1. A no-behaviour D18 stub** (`jobs.rs` with the exact signatures: `status_tier` → None, `status_write_applies` → false, `record_gitlab_job` → `Ignored`, `set_gitlab_runner_busy` → None), plus `pub mod jobs;`.

`cargo test -p gridops-core --lib -- gitlab::jobs_tests autoscaling` gave **20 failed / 24 passed** (the 24 are the existing autoscaling tests, all still green):

    gitlab::jobs_tests::status_tier_ranks_every_status_...              left: None       right: Some(0)
    gitlab::jobs_tests::a_status_write_applies_only_upward_...          left: false      right: true
    gitlab::jobs_tests::a_new_row_gets_a_uuid_...                        left: "Ignored"  right: "Inserted"
    gitlab::jobs_tests::unknown_writes_nothing                           left: "Ignored"  right: "Inserted"       (the control)
    gitlab::jobs_tests::a_terminal_row_never_changes_in_any_column       left: "Ignored"  right: "StatusApplied"  (the control)
    gitlab::jobs_tests::the_runner_id_is_filled_once_...                 left: "Ignored"  right: "FilledOnly"
    gitlab::jobs_tests::tags_are_replaced_only_by_non_empty_tags_...     left: "Ignored"  right: "StatusApplied"
    gitlab::jobs_tests::retries_count_and_pipeline_move_only_...         left: "Ignored"  right: "StatusApplied"
    gitlab::jobs_tests::set_gitlab_runner_busy_matches_...               left: None       right: Some(("sibling", Some("p1")))
    autoscaling::gitlab_tests::one_gitlab_job_recorded_under_a_project_and_its_group_connection_is_one_job   left: 0  right: 1
    + the 10 round-1 autoscaling tests, with the same failures as round 1 (e.g. collision left: 1 right: 3)

At first, `unknown_writes_nothing` and `a_terminal_row_never_changes_in_any_column` **passed** against this stub, because a writer that does nothing meets an invariant that says "nothing changes". That is not a valid RED. I added a live-writer control to each (above). Both now fail on that control.

**2. A throwaway reference D18 implementation** (scratchpad `jobs_ref.rs`, written from the brief's rules), run to show the tests can be passed and agree with D18: **9 passed / 0 failed**. So the D18 unit tests aren't contradictory or wrong against a faithful implementation. It was removed afterwards.

**3. The round-1 no-behaviour reconciler stub** (`gitlab_poll.rs` with the constants, `UnavailableGitLab` and a `poll_once` that returns `Ok`; the `gitlab` field and its three constructor sites; and a `gitlab_credentials` that returns `Ok(None)`).

`cargo test -p gridops-reconciler gitlab_poll_tests` gave **14 failed / 2 passed**, as in round 1. The changed test fails on its feature assertion:

    concluding_a_running_row_clears_its_runners_busy_flag_and_records_one_runner_event   left: "running"  right: "failed"

The 2 that pass are still the contract-item tests (the constants and `UnavailableGitLab`), whose RED is the compile RED.

### Hygiene

- `cargo fmt --all -- --check` is clean.
- `cargo +1.98.1 clippy -p gridops-core --all-targets -- -D warnings -A clippy::unused_async_trait_impl` (CI's toolchain and flags) exits 0. It was run with the reference `jobs.rs` in place, because the tests need the module to compile.
- `cargo +1.98.1 clippy -p gridops-reconciler --all-targets ...`, run with the stub, reports only the stub's own two `main.rs` lines and nothing in `gitlab_poll_tests.rs`.

## Ready for F

**RED is valid. F may implement against these tests.** The round-1 readings still hold (items 1-8 and 10-12 of the round-1 list, commit `1a4c512`): the `poll_once` signature, the `i64` constants, the `UnavailableGitLab` unit struct, `gitlab_credentials` in `main.rs`, the `Reconciler.gitlab` field, `NotFound` → `canceled`, group routing, the display tier using room, no edit to the Bitbucket guard, and the evidence appendix in Phase 6.

What changes or is new with D18:

1. **Create `crates/gridops-core/src/gitlab/jobs.rs` and add `pub mod jobs;` to `gitlab/mod.rs` first.** Use the brief's exact signatures. Do **not** move the `jobs_tests` mount; it stays in `mod.rs` (see "Test wiring").
2. **`JobWrite` needs no derives.** The tests name the outcome through an exhaustive `match`, so whatever derives #140's copy has, the tests still compile against it.
3. **Outcome fields:**
   - On `Inserted`, `previous_status` is `None`.
   - On `StatusApplied` and `FilledOnly`, `previous_status` is `Some(stored status)`, `status` is the stored status after the call, and `gitlab_runner_id` is the stored id after the call (as `140-brief.md` states).
   - A rejected write that fills nothing, and any write onto a terminal row, is `Ignored`, which is the only variant left. The other fields of an `Ignored` outcome are not pinned.
4. **Not pinned:** a new row's `created_at`. The round-1 poll test still requires it to fall within the poll, so `now_ms` (or the observation time) both pass.
5. **Busy clear (replaces round-1 item 9):** go through `set_gitlab_runner_busy`, matched on `base_url` among live `ci_platform='gitlab'` runners. The freed runner ends `busy=0`, `status='online'`.
6. **`runner_events`:** on a conclusion with `StatusApplied` from `running` and a known `gitlab_runner_id`, when `set_gitlab_runner_busy` returns a runner, write exactly one row with:
   - `runner_id` and `pool_id` from that return;
   - `level` `info`;
   - `event` `GitLab job <status>`;
   - metadata keys `jobId`, `projectId`, `pipelineId` and `connectionId` (numbers or strings both pass).

   The message is not pinned; A suggests `Job · pipeline {pipeline_id}`, #140's fallback. Write no row when the row had no runner id.
7. **Dedupe:** the demand scan counts a GitLab job once per `(gitlab_connections.base_url, job_id)`. Its candidate pools are the pools of every connection that recorded it, in the usual pool order. In the test, the older pool owns it.

## Evidence appendix (for Phase 6)

These tests rely on these unchanged-code facts, in addition to round 1's:
- the `runner_events` columns (`migrations/0001_initial.sql:83-94`);
- `gitlab_connections.base_url` and the unique `(base_url, scope_kind, scope_id)` index, which allow a project connection and a group connection on one instance (`migrations/0047_gitlab.sql:25-39`);
- the `gitlab_jobs` column defaults (`0047_gitlab.sql:62-77`).

I'll add them to `evidence.md` in Phase 6 with verbatim excerpts.
