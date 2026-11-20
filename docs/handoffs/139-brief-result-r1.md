## Brief Review (Round 1)

### BLOCK findings
**[B-1]** Missing acceptance test for D6 (GitLab pool-target sync). The brief explicitly takes ownership: "Pool-target sync for GitLab is #139's: the GitLab counterpart of `sync_github_targets` / `pool_targets` … so a pool with `runner_pool_gitlab_connections` rows gets `gitlab_project` / `gitlab_group` fleet targets", and the seam section lists "the GitLab pool-target sync (extend `pool_targets` and the `sync_github_targets` call sites, or a `sync_gitlab_targets` beside it)". But the amended Done-when tests only "admission (`ensure_target_identity_for_pool` / `link_pool_target`) for GitLab targets" — it never requires a test that a pool with GitLab connections actually receives `gitlab_project`/`gitlab_group` targets. This is a scoped deliverable with no acceptance criterion; add a Done-when item covering the sync (pool with membership rows → targets created/linked).

**[B-2]** Missing acceptance tests for the round-2 block 2 error paths. The brief specifies two concrete refusal behaviors: "`save_pool_fleet` refuses to save that profile with `FleetError::InvalidRequest`, and a manager-path start is refused before `create_runner`" when a GitLab pool has `docker_image=''` and `GITLAB_RUNNER_IMAGE` is not yet on master. The round-2/round-3 Done-when additions cover only the Tart case ("a manager-path GitLab start on a Tart-only pool … calls `create_runner` and sends `tart_image`"). Neither refusal path — the `save_pool_fleet` `InvalidRequest` refusal nor the pre-`create_runner` manager-path refusal (whose Done-when sibling, "no GitLab runner created", is also absent) — has a test. Add Done-when items for both, including that `FakeGitLab` records no `create_runner` call.

**[B-3]** Missing acceptance test for the `requires_docker_socket` refusal. The host-agent facts section makes a testable commitment: "a GitLab fleet start on a profile without `requires_docker_socket` is refused before `create_runner` with a readable reason (no GitLab runner created), and the Prepare refusal, if it happens anyway, is a counted failure." No Done-when entry covers either half of this. Add: a fleet-path start on a GitLab pool whose profile lacks `requires_docker_socket` is refused before `create_runner` (fake records no call), and a Prepare refusal counts through `record_provision_failure` once per runner.

### Needs-verification findings
**[NV-1]** All cited file:line claims about current master: the reset at `main.rs:2766`, the GitHub-branch dispatch at `main.rs:2811`, the status loop at `main.rs:807`, `finish` re-visiting terminal placements at `fleet.rs:265`, and `ensure_target_identity_for_pool`'s `_ => false` arm — settled by reading those lines in the repo.

**[NV-2]** D17's `Scope` type and `list_runners` signature: that `Scope` exists in `crates/gridops-core/src/gitlab/`, that connections carry a scope usable as `Scope::target(connection_id)`, and that `FakeGitLab` supports `fail_next`, `calls()`, `FakeMethod`, `with_scope`/`with_job` — settled by `crates/gridops-core/src/gitlab/mod.rs` and `gitlab/fake.rs`.

**[NV-3]** The claim that `delete_pool` hard-deletes `runner_pools` and that both `runner_pool_gitlab_connections` and `runners` cascade, so a deleted pool's connection retains no membership/runner rows — settled by the `delete_pool` implementation and migration FK definitions (`migrations/0047_gitlab.sql` and earlier).

**[NV-4]** That the `runners` indexes `runners_gitlab_connection_idx` / `runners_gitlab_runner_id_idx` exist in 0047, that `runner_pools.labels`/`run_untagged` and `gitlab_connections.access_token_key` columns exist, and that a `settings` key-value table exists for `lastGitlabRunnerSweepAt` — settled by the migration files.

**[NV-5]** That `retry_failed_start` in `crates/gridops-core/src/fleet/retry.rs` rebuilds the row via `INSERT…SELECT` (so the `gitlab_connection_id` copy is possible and `gitlab_runner_id` omission is coherent), and that `maybe_sync_github` / `retry_github_runner_cleanup` exist as the named call-site patterns — settled by those files.

**[NV-6]** That `record_provision_failure` has an existing threshold/circuit behavior and `set_pool_wait_reason` exists (round-3 warnings rely on both) — settled by `main.rs`.

**[NV-7]** D5's claim that Inspect `IdleExpired`/`RegistrationRefused` arrive as `Failed { ResourceUnavailable, retryable: false, diagnostic }` and Start refusals as `Failed { Configuration, … }`, and that #140's webhook sets the manager-path busy status used by the round-3 reset — these describe the not-yet-merged agent behavior (#147/#140); settled only when those PRs' diffs exist. Tests here run against fixtures, so this is a cross-PR contract risk, not a defect in this brief.

### WARN findings
**[W-1]** The brief's done-when relies on behaviors produced by three unmerged siblings (#142, #146, #147) plus #140; the tests are against stubs, so contract drift between the stub shape and the shipped agent/manager (e.g. exact `exitReason` strings, diagnostic codes) will surface only at integration. Recommend the Done-when also state that stub fixtures must mirror the published D10/D5/host-agent contract verbatim, so drift is detectable when the siblings land.
**[W-2]** "Readable diagnostic" appears as an acceptance criterion in at least four places without a definition (contains the code? non-empty? a specific prefix?). Recommend one sentence defining it (e.g. "the diagnostic's message names the GitLab connection and `DiagnosticCode`") so the criterion is testable as written.
**[W-3]** With the placeholder client and round-3's "every fleet `create_runner` failure counts once", production will accumulate provision failures toward the circuit for every GitLab start attempt until #146 lands; the brief asserts "the pool is not wedged" but the circuit opening is the designed behavior, and "not wedged" is untestable as phrased. Recommend stating the expected observable behavior with the placeholder (failures counted, circuit opens, no leaked runners, no rows left `starting`).

### NIT findings
**[NIT-1]** "token never in logs or row" is listed as one criterion; the fleet path deliberately stores the token in bootstrap material (`placement::store_bootstrap`). Say "never in the `runners` row or reconciler logs" to avoid the apparent conflict with D1.
**[NIT-2]** D7 says the indexes are "already in 0047" while D16 says "the migration is `migrations/0047_gitlab.sql`" — presumably already shipped by #137; one clarifying word ("already-shipped 0047") would remove the reading that #139 adds it.

### Verdict
BLOCK — 3 blocking finding(s); must resolve before implementation starts.