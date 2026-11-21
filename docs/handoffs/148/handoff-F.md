# Handoff-F: Phase 6 - GitLab demand matching, scale-down and a pending-jobs poll (#148)

**Date:** 2026-11-20
**Branch:** claude/gitlab-148-scaling-71c514 (implemented on head 0d1e27b)
**Issue:** #148
**Brief:** docs/handoffs/148-brief.md (all three amendment rounds). **RED:** docs/handoffs/148/handoff-T-red.md (round 2, D18). **A:** docs/handoffs/148/handoff-A.md (round 4, PASS with 6 warnings).

## What was done

- `crates/gridops-core/src/gitlab/jobs.rs` (new): #148's local copy of D18, the shared `gitlab_jobs` writer. It has the brief's exact API: `JobObservation`, `JobWrite`, `JobWriteOutcome`, `status_tier`, `status_write_applies`, `record_gitlab_job` and `set_gitlab_runner_busy`. Every D18 rule is in the one guarded `INSERT ... ON CONFLICT(connection_id, job_id) DO UPDATE ... WHERE ... RETURNING` statement.
- `crates/gridops-core/src/gitlab/mod.rs`: `pub mod jobs;`, the one agreed line. It is not behind `test-support`.
- `crates/gridops-core/src/autoscaling.rs`: the GitLab arm of queue ownership.
  - A private `QueuedJobKey { Github(i64), Gitlab { base_url, job_id } }`.
  - A private `KeyedJobOwner`.
  - `gitlab_job_candidates`, merged with the GitHub candidates into one ordered list.
  - A `pool_room!()` macro that both arms share.
  - `assigned_queued_jobs` counts the jobs of both platforms. Every other public output is filtered to GitHub jobs.
- `crates/gridops-reconciler/src/gitlab_poll.rs` (new): the poll and the placeholder.
  - `worker`, `poll_once(app, &mut backoff, now)`, the record and conclusion steps, and per-request timeouts.
  - The constants `GITLAB_JOB_CHECK_AGE_MS`, `GITLAB_JOB_CHECK_LIMIT`, `GITLAB_REQUEST_TIMEOUT` and `GITLAB_RATE_LIMIT_BACKOFF`.
  - The `pub(crate) UnavailableGitLab` placeholder.
- `crates/gridops-reconciler/src/main.rs`: only the edits A-W1 and A2-W7 allow.
  - `mod gitlab_poll;`.
  - The `gitlab: Arc<dyn GitLabApi>` field on `Reconciler`, with the placeholder as its value in `main()`.
  - `tokio::spawn(gitlab_poll::worker(..))`, next to `github_sync_worker`.
  - `gitlab_credentials` next to `runtime_secret`.
  - The `gitlab` field in the test constructor `reconciler_with_github`.
- `crates/gridops-reconciler/src/start_tests.rs`: one line, the `gitlab` field in its `Reconciler { .. }` literal. A-W1 lists this edit by name ("`start_tests.rs` near line 425"). The compiler requires it, and it changes no test behaviour.

## Design decisions

1. **D18's outcome needs the previous status, which one upsert can't return.** SQLite's `RETURNING` sees only the new row. So `record_gitlab_job` works in two steps:
   - It reads the stored row to work out the outcome.
   - It then runs the one guarded upsert. The upsert holds every D18 rule itself, so it is correct on its own, and its `WHERE` also requires that the status and `status_changed_at` are still the ones it read.

   If another writer got in between, the upsert writes nothing, and the function reads again (at most 5 times, then `sqlx::Error::Protocol`). Without that check, the outcome could report a stale `previous_status`. The poll would then miss a `running` → terminal transition and leave a runner busy.

   I rejected a `BEGIN`/savepoint around the read and the write, because D18 says callers own transactions, and a deferred transaction's upgrade fails with `SQLITE_BUSY_SNAPSHOT` instead of waiting.
2. **The demand scan dedupes inside the GitLab arm**, not with a count afterwards. The arm's CTE groups the D8-active rows by `(gitlab_connections.base_url, job_id)`:
   - **Queued time:** the job's is `MIN(created_at)`.
   - **Tags:** taken from any row that has some.
   - **Candidate pools:** those linked to any of its connections, through `json_group_array(connection_id)` and an `EXISTS`, so each pool is listed once per job.
   - **Status:** the furthest along of the job's rows, with `running` > `pending` > waiting. So when one row is `running` and its duplicate is still `pending`, the job is not demand.

   This adopts A's round-4 warnings 1 and 2 (dedupe in the arm, so every tier sees one job; the highest tier decides). The brief only states the dedupe, so the highest-tier rule goes beyond its text. It changes no pinned behaviour.
3. **The candidate lists are merged in Rust, not with a `UNION ALL`.** The GitLab rows are appended and the list is stable-sorted by `(queued_at, QueuedJobKey)`. `QueuedJobKey` derives `Ord` with `Github` declared first, so GitHub goes before GitLab, as A-W4 requires. The sort is stable, so each job's pools keep the SQL pool order and stay contiguous, which `assign_queued_jobs` needs. The GitHub query is unchanged apart from `wj.created_at AS queued_at` and the shared `pool_room!()`.
4. **Matching follows A-B4.**
   - Every job tag must be among the pool's configured `labels`, compared with `lower()` on both sides. No system or catch-all label is implied, so exclusive pools are matched like any other.
   - An untagged job (`json_array_length(tag_list)=0`) matches only pools with `run_untagged=1`.
   - The bare `EXISTS (SELECT 1 FROM (pool_platforms!()) platform)` stays, per A2-W5.
5. **The poll's conclusion step.**
   - **Which rows:** the connection's D8-active rows that are not in this poll's pending answer and not in a project whose `pending_jobs` failed. Their `updated_at` must be older than `GITLAB_JOB_CHECK_AGE_MS`.
   - **Order:** `pending`, then the other waiting statuses, then `running`; within each, `updated_at`, `created_at`, `id` (least recently checked first). At most `GITLAB_JOB_CHECK_LIMIT`.
   - **The check mark** is the poll's own one-column `UPDATE`, run before each `job` request.
   - **Writing the answer:** through D18 as GitLab gives it, at the check time. `NotFound` is written as `canceled`, with the row's own pipeline and no tags. A `job` answer that isn't terminal is written too, for example `pending` → `running`, and D18 decides whether it applies.
   - **Freeing a runner:** when the outcome is `StatusApplied` from `running` to a terminal status with a known `gitlab_runner_id`, the poll calls `set_gitlab_runner_busy(.., false, ..)`. If that returns a runner, it writes one `runner_events` row. All of this happens in one transaction inside `retry_when_busy`.
6. **Stopping on errors.**
   - A `RateLimited` answer from any call stops that connection for this poll, with no conclusion step, and puts it in the worker's `HashMap<String, Instant>` until `now + retry_after`. The default is `GITLAB_RATE_LIMIT_BACKOFF` (60 s), capped at one hour.
   - Any other error from `pending_jobs` marks that project unanswered. A timeout counts as such an error.
   - A failed `group_projects` skips the connection's conclusion step, because without a project list there is no pending answer at all. This adopts A's warning 6a.
   - Errors are logged per connection and never fail `poll_once`.
7. **The interval:** the worker sleeps `setting_i64(.., "reconcileIntervalSeconds", 30).clamp(5, 3_600)`, the pass's own read. No fallback constant is needed (A's warning 6b).
8. **The `runner_events` row:** `level` `info`, `event` `GitLab job <status>`, `message` `Job · pipeline <id>` (#140's fallback text, as A suggested), and metadata `{jobId, projectId, pipelineId, connectionId}` as JSON numbers plus the connection id string. It is a direct `INSERT` on the transaction, because the existing `pool_event` helper takes the pool rather than a transaction and has no `runner_id`.

## Reuse / extend-vs-new

- **Queue ownership extends the existing object.** `queued_job_candidates`, `assign_queued_jobs`, `queued_job_owners` and `waiting_job_pools` gained a GitLab arm. They reuse `PoolStanding::of`, the room rule (now `pool_room!()`, shared by both arms) and `pool_platforms!()`. There is no parallel ownership path.
- **The poll is new, as the brief's Scope and A-B1 require.** It follows the GitHub analogue:
  - its own worker, like `github_sync_worker`;
  - writes wrapped in `retry_when_busy`;
  - bounded checks like `unconfirmed_jobs`/`confirm_jobs` (same 5-minute age and limit of 10);
  - a check mark written before each request, like `workflow_job_sync.checked_at`.
- **`gitlab/jobs.rs` is new by agreement (D18).** It is #140's module, and #148 carries a local copy until #140 merges.
- **`gitlab_credentials` and `UnavailableGitLab`** are placed where A2-W7 says.

## Architecture notes for A

- **Layers.**
  - `gridops-core`: a new module, `gitlab::jobs` (SQL in a domain module, as in `autoscaling.rs`), plus private changes in `autoscaling`.
  - `gridops-reconciler`: a new module, `gitlab_poll`, a child of `main`. It uses `super::` helpers, so the dependency points from the poll to main. `main` refers to the poll only for the spawn and the placeholder.
- **Public API.**
  - The new public items are D18's API, exactly as the brief specifies.
  - The `autoscaling` public signatures are unchanged. `assigned_queued_jobs` now also counts GitLab jobs, as A-B2 requires.
  - `queued_job_ownership`, `waiting_job_pools`, `waiting_job_ownership`, `queued_job_assignments`, `repository_capacities`, `next_runner_provider` and `next_runner_repository` keep their types and list GitHub jobs only. GitLab jobs still take room ahead of later GitHub jobs.
- **`Reconciler`** gains `gitlab: Arc<dyn GitLabApi>`. This is #139's field. #139 is not on master, so I added it as A-W1 directs.
- **No schema change and no new dependency.** `async-trait`, `secrecy` and `uuid` were already reconciler dependencies.
- **Second-merge note:** #140 has no `jobs.rs` yet (its worktree is at the A-BLOCK commit `ebab59d`), so #148's copy is likely to land first.

## Deviations from spec / wireframe

- **The re-check mark carries a status guard.** It is `UPDATE gitlab_jobs SET updated_at=? WHERE id=? AND status IN (<D8 list>)`, not the brief's bare `WHERE id=?`. It is still one column. The guard keeps D18's "a terminal row never changes in any column" when a webhook concludes the row between the poll's selection and its mark. This adopts A's round-4 warning 4.
- **A duplicated job's status is its highest-tier row** (design decision 2). The brief states the dedupe but not the status rule.
- **A failed `group_projects` skips the connection's conclusion step** (design decision 6). The brief is silent; A's warning 6a suggested this.
- Wireframe: N/A (no UI).

## Tier 1 self-check (incl. tests now GREEN)

    $ cargo test -p gridops-core --lib -- gitlab::jobs_tests
    test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 272 filtered out

    $ cargo test -p gridops-core --lib -- autoscaling gitlab
    test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 228 filtered out
    (the 24 existing autoscaling tests, the 11 GitLab demand tests, the 9 D18 tests, and the contract's fake/http tests)

    $ cargo test -p gridops-reconciler gitlab_poll_tests
    test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 68 filtered out

    $ cargo fmt --all -- --check
    (exit 0)

    $ cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl
    (exit 0; CI's toolchain and flags)

    $ cargo test --workspace --locked --no-fail-fast
    882 passed, 12 failed, across every target. Each failure was also run on the untouched master base
    b22c451 (a throwaway worktree, since removed), where it fails identically. They are host-environment
    failures of crates this change does not touch:
      gridops-manager (bin): 11 failed. provision_lock_tests x2, route_timeout_tests x1, tart_outage_tests x8,
        each "Error: path must be shorter than SUN_LEN" (macOS's unix-socket path limit under this host's
        long temp directory). Base b22c451: "test result: FAILED. 94 passed; 11 failed", the same 11 SUN_LEN errors.
      gridops-api fleet_http: 1 failed. release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine.
        The fake fleet host reports this macOS ARM64 host's own platform ("PASS  macos-machine: macOS ARM64
        machine ready ..."), so the expected FAIL never happens. Base b22c451: the same panic at release_preflight.rs:234.
    Every other target passed, including gridops-core lib (281), gridops-reconciler (84) and the gridops-api suites.

## Evidence appendix

`docs/handoffs/148/evidence.md` (the "F (Phase 6)" section) has 10 entries:
- the Bitbucket scale-down guard;
- `retry_when_busy`;
- the interval read;
- `runtime_secret`'s unseal;
- the active-jobs partial index;
- the `gitlab_jobs` columns and identity index;
- the `gitlab_connections` `base_url` index;
- the `runner_events` columns;
- the fake's pending filter;
- `pool_platforms!()`.

## Tests that look wrong (for T)

None.

## Known issues

- **A duplicate `runner_events` row is possible** when one job is recorded under both a project connection and its group's, and both rows are `running` with the runner id. Concluding the second row calls `set_gitlab_runner_busy` again. That call returns the already-freed runner (D18 writes `busy=0` whether or not it was set), so the poll writes a second event row. The runner's state is right. This is A's round-4 warning 5, part 4, which the brief didn't adopt. Fixing it would mean changing D18's "returns the runner it changed" semantics, which #140 owns.
- **A late conclusion can free a runner that has since taken another job.** This applies to a non-ephemeral GitLab runner whose job finished without a webhook, and which then took another job before the poll's check. The webhook's terminal event (#140) has the same exposure, and both follow the brief (A2-W4). Ephemeral runners (`runner_pools.ephemeral`, default true) take one job, so they are not exposed.
- **#146's real client is on master (`eca9f0d`) but not on this branch.** So `main()` wires the brief's placeholder, and the poll logs and skips every connection in production until the client is wired in by #139 or a follow-up. That is the brief's stated plan, not a defect of this change.

## Files changed

- crates/gridops-core/src/gitlab/jobs.rs (new)
- crates/gridops-core/src/gitlab/mod.rs
- crates/gridops-core/src/autoscaling.rs
- crates/gridops-reconciler/src/gitlab_poll.rs (new)
- crates/gridops-reconciler/src/main.rs
- crates/gridops-reconciler/src/start_tests.rs (one constructor line, which A-W1 allows; a test-support literal, not a test)
- docs/handoffs/148/evidence.md (new), docs/handoffs/148/handoff-F.md (new), docs/handoffs/148/decisions.md (appended)
