## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** D8/D13 claim that the partial index `gitlab_jobs_active_created_idx` exists on `gitlab_jobs` with exactly `WHERE status IN ('created','waiting_for_resource','preparing','pending','running')`, and that `GITLAB_ACTIVE_JOB_STATUSES` / `JobStatus::is_active()` match that list — the migration excerpt stops at line 65 and no index definition or constant is shown. Evidence: the index-creation statement in `migrations/0047_gitlab.sql` (below line 65) and `GITLAB_ACTIVE_JOB_STATUSES` / `is_active()` in `crates/gridops-core/src/gitlab/mod.rs`.

**[NV-2]** NV-4 claims about `FakeGitLab`: `with_job` (fake.rs:98), `with_group_project` (fake.rs:108), `fail_next` (fake.rs:120), `pending_jobs` filtering to `JobStatus::Pending` of that project (fake.rs:245), `job` returning `NotFound` (fake.rs:226-233), one-shot failure semantics, and state keyed by `base_url` alone (fake.rs:67-76) — none of `fake.rs` appears in the attached excerpts. Evidence: the corresponding lines of `crates/gridops-core/src/gitlab/fake.rs`.

**[NV-3]** A-B1/W1 assumptions that a `retry_when_busy` helper exists in the reconciler, that `github_sync_worker` is spawned from `main()` as claimed, and that the reconcile interval (`reconcileIntervalSeconds`) is (or is not) readable from a spawned worker. Evidence: `crates/gridops-reconciler/src/main.rs` around the sync worker spawn and the write paths it wraps.

**[NV-4]** A-W2 claims the token can be unsealed from `access_token_key` "the same way Bitbucket connections' tokens are" via `runtime_secret`. Evidence: the Bitbucket credentials loader in the reconciler and the `runtime_secret` module signature.

**[NV-5]** A-B1 claims `JOB_CHECK_AGE_MS` / `JOB_CHECK_LIMIT` exist in `confirm_jobs` and are mirrored by the new constants. Evidence: `confirm_jobs` in `crates/gridops-reconciler/src/main.rs`.

**[NV-6]** A-B4 relies on `runner_pools.labels` holding the pool's *configured* labels and on migration 0046 `runner_pool_exclusive_labels` behaving as described. Evidence: the `runner_pools.labels` read path in `autoscaling.rs` (outside the shown lines 210-600 if the label source differs) and `migrations/0046_runner_pool_exclusive_labels.sql`.

### WARN findings
**[W-1]** A-B1 leaves the poll interval undetermined ("the same `reconcileIntervalSeconds` setting …; if that is not cheaply readable … `GITLAB_POLL_INTERVAL` of 30 s") — the brief permits two different runtime behaviors. Recommend fixing one (the named constant) as the deliverable, so the worker's cadence is testable and doesn't depend on a judgment call during implementation.

**[W-2]** The no-older-over-newer test seeds `status_changed_at` "after the pass's start", while A-W3 defines observation time as taken "before the `pending_jobs` request". These can differ; a seed after the pass's start but before the observation timestamp would pass the test's wording while violating A-W3's rule. Recommend the criterion say "later than the observation timestamp the pass records for that connection".

**[W-3]** The NotFound conclusion is specified as "`canceled` (or another documented terminal status … asserted by the test)" in three places — the acceptance criterion permits an implementation-chosen outcome. Recommend fixing it to `canceled` so the test is pinned regardless of implementation.

**[W-4]** The brief says a GitLab pool "must scale down when idle with no owned GitLab jobs. Prove it with a test; do not edit that guard unless a test shows it is needed" — the second sentence is a diff-gate instruction, not a brief criterion, and the excerpt (main.rs:1396-1411) already shows GitLab pools fall through to the busy/queued checks. Keep the test requirement; drop the contingency instruction from the brief.

### NIT findings
**[NIT-1]** The "Job identity hazard" paragraph in Scope is superseded by A-B2's concrete `QueuedJobKey` enum; the earlier prose could be trimmed to avoid two statements of the same rule.

**[NIT-2]** A-W1 lists test-constructor sites "near line 4825" / "near line 425" — line numbers will drift with six parallel wave-1 sessions; naming the constructors instead would be more robust.

### Verdict
PASS — no BLOCK findings; implementation may proceed.