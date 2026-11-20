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

`gitlab_connections` holds `base_url`, `scope_kind` (`project` | `group`), `scope_id`, `scope_path` (`text NOT NULL`, `migrations/0047_gitlab.sql:31`; the resolved path such as `group/sub/project`, used by the group-matching rule in the amendments below), `access_token_key` and `webhook_secret_key`. A project-scoped connection matches on the payload's `project_id`; a group-scoped connection matches when the project belongs to the group (the payload's `project` path namespace, or the stored group). Secrets are read through the existing runtime-secret store by key.

### Shared files and coordination

- **`AppState.gitlab`:** #138 adds `gitlab: Arc<dyn GitLabApi>` to `crates/gridops-api/src/state.rs`, set through a builder method so `AppState::new` keeps its signature, with a placeholder client in `main.rs` until #146 lands. #138 has not landed yet. This issue needs the same field. If #138 is not on master when this branch is built, add exactly that field and builder (same name and type) and the same placeholder wiring; whichever of #138 and #140 lands second reconciles to one copy. Do not add any other `state.rs` change.
- `main.rs`: added route line(s) for `POST /api/v1/webhooks/gitlab` only (plus the `AppState.gitlab` wiring above if #138 is not on master).
- `webhooks.rs`: only the `mod gitlab;` declaration (and a re-export if the router needs it).
- Do not change shared contract types (`gridops-core` gitlab module, domain, protocol, the migration, the OpenAPI spec). If the contract truly needs a change, stop and report it.
- The handler must match the request and response shapes of `POST /api/v1/webhooks/gitlab` in `docs/openapi/gridops-v1.json`.

## Amendments (architecture review, round 1)

These resolve the Phase 3 findings in `docs/handoffs/140/handoff-A.md`. They add to the clarifications above and win where they differ.

### Connection lookup (finding 7)

- Build the **candidate set** from the payload: every project-scoped `gitlab_connections` row with `scope_id` = the payload's `project_id`, plus every group-scoped row whose group contains the project (the group's `scope_path` is a prefix of the payload's `project.path_with_namespace` at a `/` boundary). Project ids are unique only per GitLab instance, so several candidates can exist.
- Compare `X-Gitlab-Token` with **every** candidate's webhook secret in constant time (`subtle`), and use the one that matches. No candidate, or no match, is a 401. If more than one candidate matches, prefer the project-scoped one, then the oldest `created_at`.
- Keep the processing function narrow and directly testable, like `process_workflow_job`: it takes `&SqlitePool`, `&dyn GitLabApi`, the matched connection (with its `GitLabCredentials`), and the parsed payload. Tests inject `FakeGitLab` there. The HTTP layer only reads headers and secrets, records the delivery, and calls it.

### GitLab deliveries in `webhook_deliveries` (finding 1)

- **Row.** `id` = `gitlab:` + the `Idempotency-Key` header (constant across GitLab's retries); if absent, `X-Gitlab-Event-UUID`; if both are absent, the lowercase hex SHA-256 of the raw body. Reading these optional headers does not change the contract: the OpenAPI lists the headers the handler requires (`X-Gitlab-Token`, `X-Gitlab-Event`), and neither of these is required. `event` = the `X-Gitlab-Event` header value. `action` = the payload's `build_status`. `hook_id`, `installation_id` and `repository_id` stay NULL. `signature_valid` = the token matched a candidate. A rejected delivery (no candidate, bad token) is recorded the way `receive` records an invalid signature, then answered 401.
- **Duplicates.** A delivery whose `id` already exists with status `processed` is acknowledged without reprocessing, the way `receive` handles a duplicate.
- **Retry.** `retry` in `webhooks.rs` gains **one** GitLab dispatch arm, keyed on the `gitlab:` id prefix, that re-runs the GitLab processing from the stored payload (re-resolving the connection from the payload; the token is not re-checked because `signature_valid` was recorded). A GitLab row never reaches GitHub's `process`.
- Event kinds other than `object_kind: "build"` are recorded and marked processed with no effect.

### Busy tracking (finding 2)

- **Columns.** The busy write touches only `runners.busy`, `runners.status` (`busy` while a job runs, `online` when it concludes), `last_heartbeat_at` and `updated_at`. It never writes `current_job_id`, `last_job_id` or `github_runner_id` (those join to GitHub's `workflow_jobs`).
- **Match.** Runners are matched by `gitlab_runner_id` among `ci_platform = 'gitlab'`, non-deleted runners whose `gitlab_connection_id` is a connection on the matched connection's `base_url` (runner ids are unique per GitLab instance).
- **Gate.** Busy effects run only when the guarded `gitlab_jobs` upsert changed the row, as `webhooks.rs` already does for GitHub. Clearing busy only applies to the runner recorded on that job.

### Ordering guard (finding 6)

- The out-of-order guard is one conditional upsert on `(connection_id, job_id)`: `ON CONFLICT DO UPDATE ... WHERE excluded.status_changed_at > gitlab_jobs.status_changed_at`. **Tie rule:** at an equal `status_changed_at`, the update applies only when the incoming status ranks later in the lifecycle (`created` < `waiting_for_resource` < `preparing` < `pending` < `running` < concluded); otherwise it is ignored. `status_changed_at` is stored as Unix epoch seconds (the column is an integer), parsed from `build_status_changed_at`; a missing or unparsable value falls back to the delivery's receive time.
- The `pending` tag fetch (`GitLabApi::job`) happens before the upsert. If it fails, the delivery is marked failed (retryable through the GitLab arm above) and no row is written.
- #148's poll is expected to insert only jobs it has not seen, because the contract's `Job` carries no `status_changed_at` to guard with. That is a note for #148, not a change here.

### Shared files (findings 3 to 5), replacing the "Shared files and coordination" list where they differ

- **`state.rs` (finding 3):** add exactly `pub gitlab: Arc<dyn GitLabApi>` to `AppState`; `AppState::new` keeps its signature and initialises it with a private `UnconfiguredGitLab` (in `state.rs`) whose every method returns `GitLabError::Upstream("GitLab client not available yet".into())`; add `pub fn with_gitlab(mut self, gitlab: Arc<dyn GitLabApi>) -> Self`. `main.rs` needs no wiring change for this; #146 adds `.with_gitlab(...)` with the real client. The same text goes to #138, and whichever lands second keeps one copy.
- **`crates/gridops-api/Cargo.toml` (finding 4):** `[dev-dependencies]` gains `gridops-core` with `features = ["test-support"]` (if not already present), so tests can use `FakeGitLab`.
- **`webhooks.rs` (finding 5):** `mod gitlab;` (plus a re-export for the router), the one GitLab arm in `retry`, and optionally `pub(super)` delivery bookkeeping helpers extracted from `receive` without changing its behaviour, so `gitlab.rs` reuses them instead of copying the state machine.
- `main.rs`: the route line for `POST /api/v1/webhooks/gitlab` only.

### Outside #140

The deliveries UI labels a row with neither repository nor installation "GitHub App" (`src/routes/_app/webhooks.tsx:119`, `:146`). GitLab rows will carry that label until #141 or #145 fixes it; #140 does not touch the UI.

## Brief gate round 1 additions

These add to the sections above. They do not remove or weaken anything in them.

### Done when: additional required tests (B-1, W-2)

The original "Done when" list stands. Tests must **also** cover each behaviour the amendments added:

- **Tie rule.** Two events for one `(connection_id, job_id)` with an equal `status_changed_at`: a later-lifecycle status (for example `running` after `pending`) applies; an earlier-or-equal one (for example `pending` after `running`) is ignored and leaves the row unchanged. Tested in both delivery orders.
- **Timestamp fallback.** An event with `build_status_changed_at` missing, and one with an unparsable value, are stored with `status_changed_at` = the delivery's receive time (epoch seconds).
- **Multiple candidates.** A payload matching several candidates (a project-scoped and a group-scoped connection, and two connections on different `base_url`s with the same `project_id`) uses the candidate whose secret matches the token; when more than one matches, the project-scoped one wins, then the oldest `created_at`. No match among several candidates is a 401.
- **Group prefix at a `/` boundary.** A group connection with `scope_path` `acme` matches a project at `acme/app` and `acme/sub/app`, and does **not** match `acme-other/app`.
- **Retry through the GitLab arm.** A GitLab delivery left `failed` (for example a failed `pending` tag fetch via `FakeGitLab::fail_next`) is retried through `retry`, re-runs the GitLab processing, writes the `gitlab_jobs` row and ends `processed`; it never reaches GitHub's `process`.
- **Non-`build` events.** A delivery whose `object_kind` is not `"build"` is recorded, marked `processed`, and writes nothing to `gitlab_jobs` or `runners`.
- **Stale concluded event does not corrupt busy state (W-2).** After a job's `running` event has made runner A busy and a newer event has been applied, a concluded event (`success`/`failed`/`canceled`/`skipped`) with an older `status_changed_at` is ignored by the guard and changes neither the `gitlab_jobs` row nor any runner's `busy`/`status`.

### Verified excerpts (NV-1 to NV-5)

**NV-1, `src/routes/_app/webhooks.tsx`.** Both cited lines carry the fallback label (verified with `grep -n`):

    119:      <span className="hidden max-w-52 truncate text-xs text-muted-foreground lg:block">{delivery.repository ?? delivery.accountLogin ?? "GitHub App"}</span>
    146:          <dt className="text-muted-foreground">Destination</dt><dd className="truncate">{delivery.repository ?? delivery.accountLogin ?? "GitHub App"}</dd>

**NV-2, GitLab webhook headers** (GitLab docs, `https://docs.gitlab.com/user/project/integrations/webhooks/`, fetched 2026-11-20). The header table says, of `Idempotency-Key`: "Unique ID consistent across webhook retries." Of `X-Gitlab-Event-UUID` it says it is unique for non-recursive webhooks and that recursive webhooks (triggered by earlier webhooks) share the same value. The same table also lists `webhook-id`, which the docs name as preferred over `Idempotency-Key`, and `X-Gitlab-Webhook-UUID` (unique per webhook). The brief's header order (`Idempotency-Key`, then `X-Gitlab-Event-UUID`, then the body hash) is unchanged by this note.

**NV-3, `docs/openapi/gridops-v1.json`, `POST /api/v1/webhooks/gitlab`** (extracted with `python3 -I`). The `parameters` array has exactly two entries, both `"in": "header"` and `"required": true`: `X-Gitlab-Token` ("The connection's webhook secret.") and `X-Gitlab-Event` ("The GitLab event name, for example `Job Hook`."). The description reads "Not served yet: implemented by #140. The delivery is matched to a connection by its `X-Gitlab-Token`." Responses: `202` "Accepted"; `400` "The payload is not a JSON object"; `401` "No connection's webhook secret matches" (both `ApiErrorBody`). `security` is `[]`.

**NV-4, statuses.** `crates/gridops-core/src/gitlab/mod.rs:151`:

    pub const GITLAB_ACTIVE_JOB_STATUSES: [&str; 5] = [
        "created",
        "waiting_for_resource",
        "preparing",
        "pending",
        "running",
    ];

`JobStatus::is_active()` (`mod.rs:191`) matches `Created | WaitingForResource | Preparing | Pending | Running`; `is_terminal()` matches `Success | Failed | Canceled | Skipped`; `as_str()` spells them `created`, `waiting_for_resource`, `preparing`, `pending`, `running`, `success`, `failed`, `canceled`, `skipped`, `manual`, `scheduled`, `unknown` (`#[serde(rename_all = "snake_case")]`, unknown values deserialize as `Unknown`). `migrations/0047_gitlab.sql:82`:

    WHERE `status` IN ('created','waiting_for_resource','preparing','pending','running');

**NV-5, `crates/gridops-api/src/webhooks.rs`.**
- Delivery bookkeeping is inline in `receive` (`:25-111`): insert with `status='received'` and `ON CONFLICT(id) DO NOTHING` (`:52-66`); `if result.rows_affected() == 0` answers `202 {"accepted": true, "duplicate": true}` (`:68-73`); an invalid signature does `UPDATE webhook_deliveries SET status='rejected',error=? WHERE id=?` then `Err(ApiError::Unauthorized)` (`:74-81`); then `process` and `status='processed'` or `status='failed'` with the error cut to 2,000 characters, still answering 202 (`:83-110`). `retry` (`:113-185`) repeats the processed/failed updates inline. Note: in `receive` today any existing `id` is treated as a duplicate, whatever its status.
- The GitHub conditional upsert guard is on `workflow_runs` (`:531`): `WHERE excluded.run_attempt>=workflow_runs.run_attempt`.
- The GitHub busy gate in `process_workflow_job` (`:613-625`) is a re-read, not a `rows_affected` check: "a late event of an earlier attempt is finished again, and says nothing about a runner or the queue now", followed by `if status != "completed" && SELECT status FROM workflow_jobs WHERE id=? == "completed" { return Ok(()); }` before `associate_runner_with_job`. The amendment's gate for #140 (busy effects only when the guarded `gitlab_jobs` upsert changed the row) stands as written; this is the precedent it follows in spirit.

## Source material read

- `docs/handoffs/140-brief.md`
- `docs/handoffs/140/handoff-A.md`
- `docs/handoffs/140/decisions.md`
- `crates/gridops-api/src/webhooks.rs` (in full)
- `crates/gridops-api/src/state.rs` (in full)
- `crates/gridops-api/src/main.rs` (module list, state construction, router and `public_routes`, the webhook routes)
- `crates/gridops-api/Cargo.toml`
- `crates/gridops-core/src/gitlab/mod.rs` (in full)
- `crates/gridops-core/src/gitlab/fake.rs` (in full)
- `migrations/0047_gitlab.sql` (in full)
- `docs/openapi/gridops-v1.json` (`POST /api/v1/webhooks/gitlab` path entry)
- `src/routes/_app/webhooks.tsx` (`:100-166`)
- GitLab webhooks documentation, `https://docs.gitlab.com/user/project/integrations/webhooks/` (header table)
