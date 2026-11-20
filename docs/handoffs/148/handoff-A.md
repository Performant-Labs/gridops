# Handoff-A: Phase 3 - GitLab demand matching, scale-down and a pending-jobs poll (#148)  (up-front plan review, round 3)

**Date:** 2026-11-20
**Branch:** claude/gitlab-148-scaling-71c514 (head c48f951, on master b22c451)
**Brief reviewed:** docs/handoffs/148-brief.md (amended in c48f951, section "Amendments, round 2": A2-B1, A2-W2/W3, A2-W4 to A2-W7 and the A2 Done-when additions)   **Reuse map:** N/A. No `survey.md` was provided, so the baseline is the code analogues used in rounds 1 and 2: GitHub queue ownership in `gridops-core/src/autoscaling.rs`; the GitHub sync worker and job check in `gridops-reconciler/src/main.rs` (`github_sync_worker`, `retry_when_busy`, `unconfirmed_jobs`, `confirm_jobs`); and the shared write helpers in `gridops-core/src/db.rs`.   **Wireframe:** N/A (no UI surface)
**Verdict:** PASS

## Summary

PASS, with no block findings and 4 warnings.

**The round-2 block is resolved.** A2-B1 now has all three parts of the analogue's bound:

- **Selection by "no report and no check within the age".** Rows are selected on `gitlab_jobs.updated_at`, which every writer sets. The poll stamps it before each `job` request, the way `confirm_jobs` writes `workflow_job_sync.checked_at`.
- **Waiting rows first, running rows last.** Each group is taken oldest first, as `unconfirmed_jobs` orders them.
- **`GITLAB_JOB_CHECK_LIMIT`.** The cap is 10 rows per connection per poll.

The two new Done-when tests pin this.

**Round-2 warnings 2 to 7 are all taken up:**

- the rank order includes `manual` and `scheduled`, and the poll never writes `unknown`;
- the rank decides, not timestamps;
- the busy flag is cleared on conclusion;
- the platform `EXISTS` is kept;
- time limits are per request;
- the helpers are placed in the right modules.

I checked every column the amendments rely on against `migrations/0047_gitlab.sql`: `gitlab_jobs.updated_at`, `gitlab_jobs.gitlab_runner_id`, `runners.gitlab_runner_id` and `runners.gitlab_connection_id`.

**A2-W4's busy clear is safe.** GitLab runners are ephemeral (`run-single`, D9: `mode: "ephemeral"`), so the runner a concluded job names can't have taken another job since. That is the hazard `confirm_jobs` guards against for closed GitHub jobs.

**The remaining warnings are about where the two-writer rule lives, plus three small points to pin down.**

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | A-W3, A2-W2/W3 and A2-W4: the guarded `gitlab_jobs` write and the busy clear, "posted to #136 for #140"; Constraints: "Keep the diff to the files above plus tests" | pattern consistency (shared write rules for webhook and reconciler); dependency direction | **The two writers of `gitlab_jobs` get the rule as prose, not shared code.**<br>- For GitHub, the webhook (`gridops-api/src/webhooks.rs:558-626`) and the reconciler's sync and job check (`main.rs:1934-1974,2180-2209`) share their job and runner writes through `gridops-core` helpers: `associate_runner_with_job`, `record_job_report` and `finish_earlier_attempts` (`db.rs:195-300`, re-exported at `lib.rs:36-37`). There is one implementation of each rule, and both crates call it.<br>- The brief puts #148's copy of the GitLab rule in `gitlab_poll.rs`. That rule is the rank order, the same-rank timestamp rule, terminal rows never changing, `updated_at` stamping, `ON CONFLICT DO NOTHING` then a guarded update, and the busy clear.<br>- `gridops-api` can't import a reconciler module. So #140 has to write a second copy from the text posted to #136, and the two copies can drift. One already did once: issue #140's text orders events by timestamp alone.<br>**Why only a warning.**<br>- #148 alone writes the rule once.<br>- The brief records the coordination decision in writing (#136).<br>- Rounds 1 and 2 accepted a prose rule.<br>- No GitLab helper exists on master to extend. | Put the guarded write in `gridops-core`, beside `associate_runner_with_job`, as one or two functions both writers call. For example:<br>- `record_gitlab_job(executor, connection_id, &Job, observed_at)`, the rank-guarded upsert;<br>- `conclude_gitlab_job(executor, row_id, status, observed_at)`, which also clears the runner's `busy` flag.<br>The first of #140 and #148 to land adds them, as A-W2 and A2-W7 already arrange for `gitlab_credentials`. Add `crates/gridops-core/src/db.rs` (or a non-contract file next to it) and its `lib.rs` re-export to the allowed files, and leave a note for #140 in the shared notes folder. If O keeps two copies, say so in the brief, with the reason: parallel sessions, and the crate boundary. |
| 2 | warn | A2-W2/W3: "a same-rank write applies only when its `status_changed_at` is newer"; Scope: "refresh the status of recorded jobs that GitLab still reports" | cross-cutting (what `status_changed_at` means for both writers) | **An unchanged status is a same-rank write with a newer time.**<br>- The poll's observation time is always later than the previous poll's.<br>- So, as written, the poll rewrites `status_changed_at` on every recorded `pending` row on every poll. The column then means "last seen" rather than "when the status changed".<br>- It costs one write per pending row every 30 s.<br>- Any later reader of "pending since", such as the GitLab queue views left to the UI and API issues, would show the time of the last poll. | Add one line to A2-W2/W3: an identical status leaves `status_changed_at` (and `status`) alone. The timestamp only orders different statuses at the same rank. The poll may still set `updated_at`, which A2-B1 owns. |
| 3 | warn | A2-W6: "it must not conclude rows of a project whose `pending_jobs` failed or timed out this poll" | cross-cutting (the conclusion step's input) | **`group_projects` failing isn't covered.**<br>- If `group_projects` fails or times out, no project's `pending_jobs` was called. Read literally, no project "failed", so every stale row of the group becomes a conclusion candidate while there is no pending answer at all.<br>- A `job` answer is authoritative, so this isn't a wrong write. But genuinely pending rows take that poll's `GITLAB_JOB_CHECK_LIMIT` slots ahead of stale ones.<br>- The brief-gate result on this brief raises the same point (its NV-5/W-5). | Say that a failed or timed-out `group_projects` skips that connection's conclusion step for the poll. A `RateLimited` there backs off the whole connection, as the existing rate-limit bullet says. |
| 4 | warn | A-B1: "the same `reconcileIntervalSeconds` setting `run_passes` uses; if that is not cheaply readable from the worker, a named constant `GITLAB_POLL_INTERVAL`"; A2-W4: "the row's `gitlab_connection_id`" | naming; pattern consistency | **Two wording points that F would otherwise have to settle.**<br>- **The interval can be read cheaply.** `run_passes` reads it with `setting_i64(&database, "reconcileIntervalSeconds", 30).clamp(5, 3_600)` (`main.rs:405-407`), so the fallback constant has no reason to exist. Leaving both open lets two implementations pass review (brief-gate W-1/NIT-1).<br>- **`gitlab_jobs` has `connection_id`, not `gitlab_connection_id`.** `gitlab_connection_id` is the `runners` column (`0047_gitlab.sql:64,144`). | Pin the interval to `setting_i64(.., "reconcileIntervalSeconds", 30).clamp(5, 3_600)`, as `run_passes` does, and drop the fallback. In A2-W4, write the match as `runners.gitlab_runner_id = gitlab_jobs.gitlab_runner_id AND runners.gitlab_connection_id = gitlab_jobs.connection_id AND runners.deleted_at IS NULL`. |

## Notes for O

No block, so the plan may proceed to T. The warnings are cheap to fold into the brief before T starts, or to record as accepted:

1. **Warning 1.** Decide where the two-writer rule lives:
   - either one `gridops-core` helper, with the first of #140 and #148 to land adding it, and `db.rs`/`lib.rs` added to the allowed files;
   - or two copies, with the reason written into the brief.
2. **Warnings 2 to 4.** One line each:
   - an identical status leaves `status_changed_at` alone;
   - a failed `group_projects` skips the connection's conclusion step;
   - pin the interval to `setting_i64`, and fix the A2-W4 column names.

## Patterns referenced

- `crates/gridops-reconciler/src/main.rs:370-520,1396-1438,2030-2220,3692-3725`:
  - `main()` and its spawned workers, `run_passes` reading `reconcileIntervalSeconds` with `setting_i64`, `github_sync_worker` and `retry_when_busy`;
  - `maybe_scale_down`;
  - `JOB_CHECK_AGE_MS` and `JOB_CHECK_LIMIT`; `unconfirmed_jobs` (no report and no check within the age, waiting before running); `confirm_jobs` (stamps `checked_at` before the request, and leaves the runner alone for closed jobs);
  - `installation_token` and `runtime_secret`.
- `crates/gridops-core/src/db.rs:195-300` and `crates/gridops-core/src/lib.rs:36-37`: `associate_runner_with_job`, `record_job_report` and `finish_earlier_attempts`. These are shared write rules, called from both `crates/gridops-api/src/webhooks.rs:558-626` and the reconciler.
- `crates/gridops-core/src/autoscaling.rs:40-80,213-300,436-520,595-640`: the `pool_platforms!()` macro (correlated on `candidate`), `assigned_queued_jobs`, `queued_job_owners`, `queued_job_candidates` and `provider_capacities`.
- `migrations/0047_gitlab.sql`, in full: the `gitlab_jobs` columns including `updated_at` and `gitlab_runner_id`; `runners.gitlab_connection_id` and `gitlab_runner_id`; and the active partial index. Also `crates/gridops-core/src/gitlab/mod.rs:116-200`: `Job`, which has no runner id, and `JobStatus`, `is_active` and `is_terminal`.
- `docs/handoffs/137-brief.md:246,678,783` (D9: GitLab runners are ephemeral, `run-single --wait-timeout`), and issues #139 and #140 via `gh issue view`.
