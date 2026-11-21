# Handoff-A: Phase 3 - GitLab demand matching, scale-down and a pending-jobs poll (#148)  (up-front plan review, round 4)

**Date:** 2026-11-20
**Branch:** claude/gitlab-148-scaling-71c514 (head 572f828, on master b22c451)
**Brief reviewed:** docs/handoffs/148-brief.md, amended in 2223772 and 572f828. The new section is "Amendments, round 3: the shared `gitlab_jobs` write (D18, agreed with #140)", plus the Constraints line. I reviewed it with `git diff 1a4c512 572f828`.   **Reuse map:** N/A. No `survey.md` was provided, so the baseline is the code analogues used in rounds 1 to 3: GitHub queue ownership in `gridops-core/src/autoscaling.rs`; the GitHub job check in `gridops-reconciler/src/main.rs` (`unconfirmed_jobs`, `confirm_jobs`); the shared write helpers in `gridops-core/src/db.rs`; and #140's D18 text in `gitlab-140-webhooks-a9fae5/docs/handoffs/140-brief.md:292-372` (at `ba0a441`).   **Wireframe:** N/A (no UI surface)
**Verdict:** PASS

## Summary

PASS, with no block findings and 6 warnings, one of which carries two round-3 points forward.

**D18 resolves round-3 warning 1 in the right layer.** The poll and #140's webhook now write `gitlab_jobs` through one `gridops-core` function, and the poll has no SQL merge of its own. The dependency direction is right: both `gridops-api` and `gridops-reconciler` already depend on core. The split also matches GitHub's:
- `set_gitlab_runner_busy` returns `(runner id, pool id)` the way `associate_runner_with_job` does (`db.rs:195-245`);
- the caller writes `runner_events`, as `webhooks.rs:625-650` does.

D18 actually shares more than GitHub does, because GitHub's job upsert is still two copies (`webhooks.rs:584` and `main.rs:1994`). Putting SQL in a domain module (`gitlab/jobs.rs`) is consistent with `autoscaling.rs` and `fleet/*`. Round-3 warning 2 is now decided in writing: D18's equal-tier rule applies a same-status write that carries a newer time.

**The re-check mark copies the analogue.** The poll writes it with its own one-column `UPDATE`, as `confirm_jobs` writes `workflow_job_sync.checked_at` (`main.rs:2161-2169`).

**The warnings concern the new cross-connection dedupe and the mechanics around D18:**
- which key the dedupe uses and where it runs;
- which status a job has when its two rows disagree;
- how the two parallel copies of `jobs.rs` merge;
- the re-check mark against "a terminal row never changes";
- the `runner_events` row the poll now writes.

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | Round 3: "The demand scan deduplicates on `(gitlab_connections.base_url, gitlab_jobs.job_id)` … candidate pools = the pools linked to either connection". A-B2: key "e.g. `Gitlab { connection_id, job_id }`". Scope: GitLab job ids are "only unique per connection" | abstraction level; pattern consistency (the candidate list's shape) | **The dedupe changes the job key, but A-B2 and the Scope weren't amended.**<br>- Deduping on `base_url` makes the instance the job's identity. That is correct: `base_url` is normalised by #138 (`0047_gitlab.sql:23-24`), and D18's `set_gitlab_runner_busy` matches runners on it. But A-B2's example key and the Scope text still say per connection, so a top-down reading can produce a connection-keyed candidate list with a separate count-dedupe on top.<br>- The text dedupes only "the demand scan". `queued_job_candidates` also feeds the display tier (`every_waiting_status`, `waiting_job_pools`). A duplicate `created` job would then take display room twice.<br>- `assign_queued_jobs` (`autoscaling.rs:564-593`) needs each job's rows contiguous and each pool listed once per job. A pool linked to both connections must not appear twice. | - In A-B2, make the key `Gitlab { base_url, job_id }`, and correct the Scope sentence: GitLab job ids are unique per instance.<br>- Dedupe inside the GitLab arm of `queued_job_candidates`, so every tier sees one job: emit one row per (job, pool).<br>- Join a pool through an `EXISTS` over the links of any of the job's active rows, the same `EXISTS … OR …` shape the GitHub arm uses for its three repository linkages (`autoscaling.rs:464-469`).<br>- Order the job by the `MIN(created_at)` of its rows. |
| 2 | warn | Round 3: "counting each GitLab job once"; A-B3: "only `pending` is scaler demand" | cross-cutting (one status rule for reads and writes) | **When the two rows of one job disagree, the plan doesn't say what status the job has.**<br>- Example: the row is `running` under one connection but still `pending` under the other. That happens when one connection is rate-limited or failed, or when one is polled before the other.<br>- If any `pending` row makes the job demand, the job counts as demand while its runner is busy. That is round-1 finding 3's double count, here only for a while.<br>- A terminal sibling row is invisible, because the scan reads only the D8 statuses.<br>**Why only a warning.** It is bounded: one delivery from #140 writes both rows (`140-brief.md:58,290`), and the poll's conclusion step catches up within the backoff plus `GITLAB_JOB_CHECK_AGE_MS`. That is the same window GitHub accepts for a missed `in_progress` webhook (`JOB_CHECK_AGE_MS`, `main.rs:2038`). The brief-gate result r1 raises the same point (W-5). | - Make the deduped job take the highest tier among its active rows, ranked with D18's `status_tier`. It is then demand only when no row is `running`.<br>- Optionally, add a `NOT EXISTS` terminal sibling check through the unique `(connection_id, job_id)` index. That doesn't scan non-active rows, so D8 still holds.<br>- Add one test: one row `running` and one `pending` for the same job give demand 0. |
| 3 | warn | Round 3 and Constraints: "#148 carries a local copy at that identical path … whichever of #140/#148 merges second takes master's file and drops its own" | parallel-agent coexistence (single owner for shared modules) | **Both sessions will add the same new file, so the second merge conflicts.**<br>- The second merger gets an add/add conflict on `gitlab/jobs.rs` and on the `pub mod jobs;` line in the contract's `gitlab/mod.rs`.<br>- #140 is still in its brief phase: its worktree, at `ba0a441`, has no `jobs.rs`. So #148 will most likely land first, and #140 will adopt #148's copy.<br>- "Drops its own" drops that copy's unit tests along with its code. The two sets were written independently from the same text, so dropping one loses coverage exactly where the two implementations could differ.<br>This is planned and written in both briefs and on #136, so it isn't a drive-by. | - Put #148's D18 unit tests in `gitlab/jobs_tests.rs`, mounted from `jobs.rs` with `#[cfg(test)] #[path = "jobs_tests.rs"] mod tests;` (the `fake.rs`/`fake_tests.rs` pattern, `fake.rs:301-303`). Keep the module itself outside `test-support`, as `140-brief.md:320` requires.<br>- The second merger keeps master's `jobs.rs` and the union of both test sets, run against it.<br>- Leave a note for #140 in the shared notes folder that #148's copy will probably land first. |
| 4 | warn | Round 3: "The re-check mark … is the poll's own one-column `UPDATE gitlab_jobs SET updated_at=? WHERE id=?`" | cross-cutting (D18's terminal-row invariant) | **The re-check mark can write to a terminal row.**<br>- A webhook can conclude the row between the poll's selection and its stamp. The stamp then changes `updated_at` on a terminal row, which breaks D18's "a terminal row never changes in any column".<br>- D18's unit tests can't catch this, because the statement is outside `jobs.rs`.<br>- GitHub's analogue has no such conflict, because `checked_at` lives in a separate table (`workflow_job_sync`). | Write the mark as `UPDATE gitlab_jobs SET updated_at=? WHERE id=? AND status IN (<the D8 list>)`. |
| 5 | warn | Round 3: on a conclusion from `running`, the poll "writes one `runner_events` row like #140 … when a runner was changed" | pattern consistency (where the reconciler writes events, and what a job check records) | **The poll's event row departs from the GitHub analogue in three ways.**<br>- The analogue, `confirm_jobs` (`main.rs:2144-2215`), records job checks with `tracing::info!` and writes no `runner_events` row. Only the webhook writes one (`webhooks.rs:636-650`). The poll doing it is a deliberate choice, and a defensible one: it keeps a runner's history complete after a missed webhook.<br>- The reconciler's event helpers (`event`, and `pool_event` at `main.rs:3740`) take no `runner_id`. As written, this becomes a free-standing `INSERT`.<br>- `message` is not specified. `Job` has no name, and #140 uses `"{build_name} · pipeline {pipeline_id}"`, falling back to `Job` (`140-brief.md:344`).<br>- With duplicate rows, concluding the second row of the same job calls `set_gitlab_runner_busy` again. Whether that returns the runner, and so writes a second event, depends on whether D18 reports a runner whose `busy` was already 0. | - Give the reconciler's event helper an optional `runner_id` (or add a sibling `runner_event` next to `pool_event`) instead of a free-standing `INSERT`.<br>- Use the message `Job · pipeline {pipeline_id}`, #140's fallback.<br>- Write the row only when the runner's `busy` actually went from 1 to 0, or record in the brief that a duplicate event per duplicate row is accepted. |
| 6 | warn | Carried from round 3, not adopted: A2-W6 (failed `group_projects`) and A-B1 (the interval fallback) | cross-cutting; naming | **Two round-3 points are still open in the brief.**<br>- **(a) A failed `group_projects`.** If `group_projects` fails or times out, no project "failed", so every stale row of the group can be concluded with no pending answer at all (round-3 W3; brief-gate NV-5/W-5).<br>- **(b) The interval fallback.** A-B1 still leaves the `GITLAB_POLL_INTERVAL` fallback open, although `run_passes` reads `setting_i64(.., "reconcileIntervalSeconds", 30).clamp(5, 3_600)` (`main.rs:407`).<br>The column-name half of round-3 W4 is superseded, because D18's `set_gitlab_runner_busy` matches on `base_url`. | - (a) A failed or timed-out `group_projects` skips that connection's conclusion step for the poll.<br>- (b) Pin the interval to the `setting_i64` read and drop the fallback constant. |

## Notes for O

There is no block, so the plan can go on to T. Two points of information first:

- **T-red (`1a4c512`) predates this amendment.**
  - Its 26 tests are consistent with D18. In both fixtures, `conn-a` and `conn-b` are on different base URLs (`autoscaling_gitlab_tests.rs:29-30`, `gitlab_poll_tests.rs:24-27`). So the collision test's count of 3 still holds under the `base_url` dedupe, and the busy-clear test's `other-connection` runner stays busy under D18's `base_url` match.
  - T's "Ready for F" item 9 (busy clear matched on `connection_id` equality) is superseded by D18's `base_url` match.
  - Three new Done-when items have no test yet: the D18 unit tests, the dedupe test and the `runner_events` row. T owns those.
- **The warnings each take one line in the brief, or a written "accepted":**
  1. The key becomes `Gitlab { base_url, job_id }`, and the dedupe goes in the GitLab arm of `queued_job_candidates` (warning 1).
  2. The highest tier decides a duplicated job's status, with a test (warning 2).
  3. The location of the D18 test file and the rule for the second merge (warning 3).
  4. A status guard on the re-check mark (warning 4).
  5. The event helper, the message text and when the event is written (warning 5).
  6. The two round-3 carry-overs (warning 6).

## Patterns referenced

- `crates/gridops-core/src/db.rs:195-320` and `crates/gridops-core/src/lib.rs:35-38`:
  - the shared GitHub write helpers (`associate_runner_with_job`, `record_job_report`, `finish_earlier_attempts`), which take `&SqlitePool` or `SqliteExecutor` and return `(runner id, pool id)`;
  - `crates/gridops-api/src/webhooks.rs:584,625-650`, the webhook's own `workflow_jobs` insert and its `runner_events` row;
  - `crates/gridops-reconciler/src/main.rs:1994`, the reconciler's own `upsert_workflow_job`.
- `crates/gridops-reconciler/src/main.rs:407,488-502,2038-2215,3740`:
  - the `reconcileIntervalSeconds` read;
  - `retry_when_busy`;
  - `JOB_CHECK_AGE_MS` and `JOB_CHECK_LIMIT`;
  - `unconfirmed_jobs` and `confirm_jobs` (the `checked_at` stamp in its own table, and only `tracing` for job checks);
  - `pool_event`.
- `crates/gridops-core/src/autoscaling.rs:217-300,436-593`: `assigned_queued_jobs`, `queued_job_owners`, `queued_job_candidates` (the `EXISTS … OR …` pool linkage, and the ordering) and `assign_queued_jobs` (each job's rows must be contiguous).
- `migrations/0047_gitlab.sql:1-45`: `gitlab_connections`, with `base_url` normalised by #138 and the unique `(base_url, scope_kind, scope_id)` index. `crates/gridops-core/src/gitlab/mod.rs:1-20,119-200`: the contract module, which has no SQL; `Job` has no name or runner id.
- `gitlab-140-webhooks-a9fae5/docs/handoffs/140-brief.md:42,58,290-372` (at `ba0a441`): the D18 API and rules, the candidate set (a project connection plus its group connections, so one delivery writes both rows), the event message and the module being outside `test-support`.
