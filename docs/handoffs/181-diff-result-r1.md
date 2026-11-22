## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** crates/gridops-core/src/autoscaling.rs (github_queued_jobs, ~line 1000) — The helper's moved SQL is assumed to correctly join `workflow_jobs`/`workflow_runs`/pools with the `owned` id list so that `queued_job_assignments` output is byte-identical for GitHub-only pools, and that `wj.id` binds are positional-safe. — The full SQL body of `github_queued_jobs` (the unchanged `concat!` middle not shown in the diff context beyond line 937's hunk) plus an excerpt of the original `queued_job_assignments` SQL for a character-level comparison.
- **[NV-2]** crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs — `Fleet::pass()`, `Fleet::newest()`, `gitlab_setup`, `gitlab_calls`, and `ACME.id` being a valid `connection_id` for `gitlab_poll_tests::row` are assumed to exist and behave as the new tests use them. — Excerpts of gitlab_lifecycle_tests.rs (the unchanged `Fleet` methods) and gitlab_poll_tests.rs:150-200.
- **[NV-3]** crates/gridops-core/tests/gitlab_pool_fleet.rs:87-118 — `spare_runner` inserts into `runners`, `fleet_operations`, `workload_intents` with assumed column sets and a runner whose `profile_id` is derived via `workload_intents`; whether `choose_platform`'s runner query actually sees this runner as a spare with `profile_id` matching. — Excerpt of the runner-fetch SQL in `attribute_queued_jobs`/`choose_platform` (targets.rs, the rows query above line 1527) and the schema of `runners`/`workload_intents`.
- **[NV-4]** crates/gridops-core/src/autoscaling.rs:940-985 — Claim that `queued_job_owners` order for a GitHub-only pool equals the helper's `ORDER BY wj.created_at,wj.id` (Fact 2/3). The excerpts at autoscaling.rs:616-618 and 563-572 support the SQL ordering and conditional re-sort; not shown is `queued_job_owners` itself post-filter. — Excerpt of `queued_job_owners` (autoscaling.rs ~340-360) showing it preserves candidate order.
- **[NV-5]** crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs — Claimed test results (125 passed; the one gridops-api failure reproducing on master) — a CI run log or local transcript would settle it.

### WARN findings
- **[W-1]** crates/gridops-core/src/fleet/targets.rs:1394-1402 — The new doc paragraph in `attribute_queued_jobs` runs its "Otherwise it needs room: runners waiting for a machine take" sentence straight on from the GitLab parenthetical, producing one long merged sentence that mixes the GitLab-safety argument with the room rules; the original paragraph break was flattened by the diff. — Re-split the paragraphs for readability.
- **[W-2]** crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs (pending_jobs_scale_a_fleet_gitlab_pool_up_to_max_count) — The brief's "once the jobs are covered a later pass places nothing" is only implied (last total == 2 after 3 passes with 3 pending jobs), not tested against a state where coverage settles below max. — Consider adding a final assertion after the jobs are covered (e.g. delete/complete jobs, one more pass, total still 2 and no new intent).

### NIT findings
- **[NIT-1]** crates/gridops-core/src/autoscaling.rs:978 — `std::collections::HashMap` spelled with the full path inline; a `use` would match surrounding style (fmt passes either way).
- **[NIT-2]** CHANGELOG.md — Entry placed adjacent to unrelated entries (reconnect storm); confirm it sits under the Unreleased section header, which the diff context does not show.

### Verdict
PASS — no BLOCK findings; testing may proceed.