# Decision journal: #148 GitLab demand matching, scale-down and a pending-jobs poll

## A (Phase 3, up-front plan review): 2026-11-20T15:26:47-08:00
- **Decided:** BLOCK, with 4 block findings and 4 warn findings (see `handoff-A.md`).
  - (1) The poll is planned inside the reconcile pass with unbounded `job` calls. The GitHub analogue (`github_sync_worker`, `maybe_sync_github`, `confirm_jobs`) runs in its own task, for the reason `main()` documents, and bounds its job checks with `JOB_CHECK_AGE_MS` and `JOB_CHECK_LIMIT`. Recommended fix: a `gitlab_poll::worker` spawned from `main()`.
  - (2) Public autoscaling types carry two GitHub id spaces:
    - the job id, joined to `workflow_jobs.id` by three `gridops-api` views;
    - `QueuedJob.repository_id`, matched against `runners.target_repository_id` by fleet placement.

    The brief wants GitLab jobs in all of them, with no collision and a stable API, while the diff stays in `autoscaling.rs`. Recommended fix: a private key enum, with GitLab jobs counted but not emitted by the GitHub-shaped outputs.
  - (3) The GitLab statuses aren't mapped onto the two tiers of `queued_job_candidates`. Counting `running` as demand double-counts against `runners.busy`, which #140 sets.
  - (4) The tag-matching "decide and document" choice must equal #139's registration (the pool's labels as tags). The "either way" test allows a demand/registration mismatch.
  - Accepted as consistent with the code:
    - the GitLab branch in `autoscaling.rs`;
    - `gitlab_poll.rs` as a sibling of `fleet.rs` and `tart_lane.rs`;
    - leaving the Bitbucket scale-down guard alone;
    - routing a group-polled job by `connection_id`.
- **Assumed:**
  - #139 is not on master: `origin/master` is `b22c451` and has no `gitlab_lifecycle.rs`, so #148 adds the `gitlab` field.
  - In GitLab's job state machine, `created`, `waiting_for_resource` and `preparing` come before `pending`, and no runner can take them yet. This is from GitLab's documented job lifecycle and wasn't checked against a live instance.
  - The text of issues #139 and #140, read with `gh issue view`, is what those sessions build: runner tags are the pool's labels, and #140 sets busy on `running` and orders events by `build_status_changed_at`.
- **Hedged:**
  - Finding 1 offers O two resolutions: (a) the worker, recommended, or (b) keeping the poll in the pass with a written justification and bounds. The issue text says "Every pass" and "delays scaling by one pass". Option (a) keeps that latency by looping on `reconcileIntervalSeconds`, so it doesn't change what the issue asks for.
  - Finding 2 offers (a), recommended and inside the Constraints, or (b), which widens the public types and needs sign-off from the #138 and #141 owners.
  - Finding 4 asks O to confirm GitLab's own tag case rule. I didn't verify whether GitLab compares tags case-insensitively.
  - Findings 2 and 3 go beyond the round-1 and round-2 brief-gate warnings (W-1, W-2 and W-3), which raised the open key and the open decisions only as warnings. They are blocks here because the code already determines the answer: the repository-id consumers, the busy-signal double count and the ready-only tier.
- **Evidence:**
  - Read: the brief (all 138 lines), the brief-gate results r1 and r2, `docs/handoffs/137-brief.md` (R2-W3, D8, D13, D14 and the #148 mentions) and `docs/handoffs/137/handoff-A.md`, plus issues #148, #139 and #140 via `gh issue view`.
  - Source:
    - `crates/gridops-core/src/autoscaling.rs`, lines 1-850;
    - `crates/gridops-core/src/gitlab/mod.rs`, in full;
    - `crates/gridops-core/src/fleet/targets.rs:1195-1340` and `crates/gridops-core/src/lib.rs:1-40`;
    - `crates/gridops-reconciler/src/main.rs`: `Reconciler`, `main()`, `run_pass`, `reconcile`, `github_sync_worker`, `retry_when_busy`, `maybe_scale_up`, `evict_surplus_runner`, `surplus_runner`, `maybe_scale_down`, `maybe_sync_github`, `sync_run_jobs`, `upsert_workflow_job`, `unconfirmed_jobs`, `confirm_jobs`, `next_bitbucket_connection`, `deregister_bitbucket_runner`, `runtime_secret` and the test constructor at `:4825`;
    - `crates/gridops-reconciler/src/start_tests.rs:418-440`;
    - `crates/gridops-api/src/resources.rs:720-760,2065-2100` and `crates/gridops-api/src/resources/overview_queue.rs:292-330`.
  - Migrations: `0001_initial.sql` (`runner_pools`), `0046_runner_pool_exclusive_labels.sql` and `0047_gitlab.sql`.
  - Commands: a grep for every caller of the public autoscaling API across `crates/`, and `git fetch` plus `git ls-tree origin/master crates/gridops-reconciler/src/`.
