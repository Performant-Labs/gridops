# [GitLab: job webhook ingestion and busy tracking](https://github.com/Performant-Labs/gridops/issues/140)

Part of #136. **Wave 1.** Depends only on #137. Tests use `FakeGitLab`.

## Need

GitLab tells GridOps about job state changes through **Job events** webhooks. GridOps has to record them, so that #148 can turn them into demand and the runners list can show which GitLab runners are busy.

## Scope

In `crates/gridops-api/src/webhooks.rs` (new code in a `webhooks/gitlab.rs` module):

- `POST /api/v1/webhooks/gitlab` finds the connection from the payload's project or group and compares `X-Gitlab-Token` with that connection's webhook secret in constant time. Reject the delivery with 401 if no connection matches. Record deliveries in the same table as GitHub deliveries.
- Handle `object_kind: "build"`, writing to `gitlab_jobs` (from #137):
  - `pending`: fetch the job's `tag_list` with `GitLabApi::job`, because the payload carries only the runner's tags. Then upsert the job.
  - `running` with `runner.id`: record `gitlab_runner_id` on the job and mark the GridOps runner with that `gitlab_runner_id` as busy.
  - `success`, `failed`, `canceled` or `skipped`: conclude the job and clear busy.
- Order events by `build_status_changed_at`, because GitLab can deliver them out of order. Ignore an event older than the stored status. A retried job (`retries_count` > 0, new `build_id`) is a new row.

**Owns:** `crates/gridops-api/src/webhooks/gitlab.rs` and its tests. Route registration in `main.rs` is shared with #138. Keep it to the added lines.

## Done when

- Tests cover a bad token, an unknown project, a pending job looked up for tags, out-of-order events, a retried job, a duplicate delivery, and busy set and cleared on the right runner.

## Clarifications (orchestrator, before the brief gate)

Everything above this heading is the issue text and is unchanged. This section records the contract as #137 shipped it (master `b22c451`). The "Contract deviations shipped by #137" table on epic #136 replaces the issue text wherever they differ.

### Contract deviations that apply here

- **D2:** every `GitLabApi` method takes `&GitLabCredentials { base_url, access_token }` first. One `Arc<dyn GitLabApi>` serves every connection. The handler builds the credentials from the matched connection's `base_url` and the access token stored under its `access_token_key`.
- **D7:** `runners` has `runners_gitlab_runner_id_idx`. Busy tracking finds the GridOps runner by `runners.gitlab_runner_id` (and the connection), using that index.
- **D8 / D13:** `gitlab_jobs` has the partial index `gitlab_jobs_active_created_idx` over the statuses `created`, `waiting_for_resource`, `preparing`, `pending`, `running` (equal to `gitlab::GITLAB_ACTIVE_JOB_STATUSES` / `JobStatus::is_active()`). #148 scans that index for demand, so `gitlab_jobs.status` must hold GitLab's own status strings exactly as the contract's `JobStatus` serializes them. A concluded job (`success`, `failed`, `canceled`, `skipped`) must leave the index.
- **D16:** the migration is `migrations/0047_gitlab.sql`, not 0046.
- D12 to D15 do not touch this issue's code.

### Schema written (from 0047, unchanged)

`gitlab_jobs` (`id`, `connection_id`, `project_id`, `job_id`, `pipeline_id`, `status`, `tag_list` JSON text, `status_changed_at`, `retries_count`, `gitlab_runner_id`, `pool_id`, `created_at`, `updated_at`), unique on `(connection_id, job_id)`. Write exactly these columns and contract statuses; #148 reads them. A retried job carries a new `build_id`, so it lands as a new row by the unique key. `pool_id` may stay NULL here unless the pool is unambiguous from `runner_pool_gitlab_connections`; #148 owns pool assignment for demand.

`gitlab_connections` holds `base_url`, `scope_kind` (`project` | `group`), `scope_id`, `access_token_key` and `webhook_secret_key`. A project-scoped connection matches on the payload's `project_id`; a group-scoped connection matches when the project belongs to the group (the payload's `project` path namespace, or the stored group). Secrets are read through the existing runtime-secret store by key.

### Shared files and coordination

- **`AppState.gitlab`:** #138 adds `gitlab: Arc<dyn GitLabApi>` to `crates/gridops-api/src/state.rs`, set through a builder method so `AppState::new` keeps its signature, with a placeholder client in `main.rs` until #146 lands. #138 has not landed yet. This issue needs the same field. If #138 is not on master when this branch is built, add exactly that field and builder (same name and type) and the same placeholder wiring; whichever of #138 and #140 lands second reconciles to one copy. Do not add any other `state.rs` change.
- `main.rs`: added route line(s) for `POST /api/v1/webhooks/gitlab` only (plus the `AppState.gitlab` wiring above if #138 is not on master).
- `webhooks.rs`: only the `mod gitlab;` declaration (and a re-export if the router needs it).
- Do not change shared contract types (`gridops-core` gitlab module, domain, protocol, the migration, the OpenAPI spec). If the contract truly needs a change, stop and report it.
- The handler must match the request and response shapes of `POST /api/v1/webhooks/gitlab` in `docs/openapi/gridops-v1.json`.
