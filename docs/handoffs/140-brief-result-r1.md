## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** The retry arm "re-runs the GitLab processing from the stored payload" — evidence: the `webhook_deliveries` schema and the `retry` handler's payload access in `crates/gridops-api/src/webhooks.rs` (the excerpt shows `:113-185` only in summary; a line showing the raw payload column being read back would settle that the payload is stored and retrievable for GitLab rows).
- **[NV-2]** `Idempotency-Key` is "constant across GitLab's retries" and is present on Job events — evidence: the GitLab webhook header table and the Job events docs page itself (the excerpt cites the docs page, but the Job-events payload/header availability is asserted, not shown).
- **[NV-3]** The `subtle` crate is available for constant-time comparison in `gridops-api` (used by the amendment) — evidence: `crates/gridops-api/Cargo.toml` dependency list; the source list says Cargo.toml was read but no excerpt shows it.
- **[NV-4]** `FakeGitLab` supports `fail_next` and exposes a call log sufficient for "no `GitLabApi::job` call" — evidence: `crates/gridops-core/src/gitlab/fake.rs` (read but not excerpted here).
- **[NV-5]** `runners` table has `gitlab_runner_id`, `gitlab_connection_id`, `ci_platform`, `busy`, `status`, `last_heartbeat_at`, `current_job_id`, `github_runner_id`, `deleted` as the busy-tracking amendment assumes — evidence: the `runners` DDL and `migrations` (0047 excerpt stops at fleet_ci_targets).

### WARN findings
- **[W-1]** Lifecycle ranking for the tie rule lists `created < waiting_for_resource < preparing < pending < running < concluded`, but `JobStatus` also has `manual`, `scheduled` and `unknown` (NV-4 shows these exist and unknown values deserialize as `Unknown`). The brief never ranks them, so a `manual`/`scheduled`/`unknown` event at an equal `status_changed_at` has undefined behavior, and storing `unknown` as a status interacts unclearly with the active-status index. Define their rank (and whether `manual`/`scheduled`/`unknown` events should be processed at all) and add a test if any are in scope.
- **[W-2]** The OpenAPI contract (NV-3) declares a `400` "The payload is not a JSON object" response, but the brief's scope and "Done when" never mention validating/handling a non-object or malformed payload, nor a test for it. Add the malformed-payload path and a test, or note explicitly that the delivery-id body hash requires reading the raw body before parse failure anyway.
- **[W-3]** The busy clear sets `runners.status = 'online'` when a job concludes, but no excerpt shows the set of valid `runners.status` values or that `online` is one; writing a runner that was never `busy` (e.g. a concluded event for a job whose `running` event was missed) would flip it to `online` unconditionally. The excerpts do not settle the runners.status enum — narrow the update (e.g. only when currently `busy`) or verify the enum.

### NIT findings
- **[NIT-1]** The amendment's "Duplicates" section still says "only when its status is `processed`" while the orchestrator decision later overrides it to "any existing id"; readers of the amendments section alone will get the wrong rule. Strike or annotate the superseded sentence.
- **[NIT-2]** `action` = the payload's `build_status` is specified for build events but not for non-build events (presumably NULL); say so explicitly.

### Verdict
PASS — no BLOCK findings; implementation may proceed.