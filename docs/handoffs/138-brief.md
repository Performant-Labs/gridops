# [GitLab: platform-connection and pool-target API](https://github.com/Performant-Labs/gridops/issues/138)

Part of #136. **Wave 1.** Depends only on #137. Tests use `FakeGitLab`, not the HTTP client.

## Need

Operators need to connect a GitLab project or group and point a pool at it, the way `/api/v1/platform-connections/bitbucket` works today.

## Scope

Implement the handlers for the spec that #137 adds:

- `GET/POST/PATCH/DELETE /api/v1/platform-connections/gitlab`:
  - Creating a connection resolves the path with `resolve_scope`, runs `verify_access`, stores the token and a generated webhook secret by key, and returns the webhook URL and secret **once**. No other response includes either secret.
  - A duplicate `(base_url, scope)` gets a 409, as the Bitbucket duplicate-workspace fix does (3629667).
  - Deleting a connection that a pool still targets follows Bitbucket's rule.
  - Add `POST …/:id/verify` and `POST …/:id/rotate-webhook-secret`.
- Runner-pool create and update accept GitLab targets and `run_untagged`. A pool can't mix GitLab targets with GitHub or Bitbucket targets.
- Fleet authorization and bot-token scopes (`gridops-core/src/fleet/authorization.rs`) treat GitLab connections the same as Bitbucket ones.
- Inject `Arc<dyn GitLabApi>` through `gridops-api/src/state.rs`, so tests swap in the fake.

**Owns:** the GitLab sections of `crates/gridops-api/src/resources.rs` and `state.rs`, the authorization changes, and new tests under `crates/gridops-api/tests/`. Route registration in `main.rs` is shared with #140. Keep it to the added lines.

## Done when

- HTTP tests cover create, duplicate, bad token, missing scope, missing role, update, verify, rotate, delete-while-targeted and secret non-disclosure.
- `control_plane_contracts.rs` covers the new routes.
- Responses validate against the spec.

## Clarifications (orchestrator, before the brief gate)

Everything above this heading is the issue text and is unchanged. This section records the contract as #137 actually shipped it (master `b22c451`, PR #150). The epic #136's "Contract deviations shipped by #137" table replaces the issue text where they differ. It removes no scope.

### Contract deviations that apply here

- **D2.** Every `GitLabApi` method takes `&GitLabCredentials { base_url, access_token }` as its first argument. One `Arc<dyn GitLabApi>` serves every connection: the handler builds the credentials from the connection's stored base URL and decrypted token for each call.
- **D3.** The call is `resolve_scope(credentials, kind: ScopeKind, path)`. The request's `scopeKind` picks the `ScopeKind`.
- **D14.** `starts_belong_to_reconciler` in `crates/gridops-api/src/resources.rs` (today `providers.iter().any(|p| p == "tart")`) must also return true for a pool that has `runner_pool_gitlab_connections` rows. So the API's in-request start (`provision`) never starts a GitLab pool's runners and only ever passes `Github` or `Bitbucket`; GitLab registration belongs to #139's reconciler module. #138 owns this predicate change. Its signature may change to take what it needs (for example the pool id or a `has_gitlab_targets` flag); update its callers in `resources.rs`.
- **D16.** The migration is `migrations/0047_gitlab.sql` (not 0046). It already creates `gitlab_connections` (`id`, `name`, `base_url`, `scope_kind`, `scope_id`, `scope_path`, `access_token_key`, `webhook_secret_key`, `created_by`, timestamps, unique on `(base_url, scope_kind, scope_id)`), `runner_pool_gitlab_connections`, and `runner_pools.run_untagged`. Add no migration unless a real gap is found; if one is, stop and report it, since the schema is shared contract.
- D11: admitting GitLab targets into fleet (`ensure_target_identity_for_pool`, `link_pool_target`) is #139's. This issue stores the pool-to-connection link rows only.

### The OpenAPI spec is the contract (`docs/openapi/gridops-v1.json`)

Where the issue text and the spec differ, the spec wins:

- The rotate route is `POST /api/v1/platform-connections/gitlab/{connection_id}/webhook-secret/rotate` (not `…/rotate-webhook-secret`). It returns 200 with `GitlabWebhookSecret { webhookUrl, webhookSecret }`.
- `POST …/{connection_id}/verify` returns **204** on success and 400 naming the refused token, missing scope or missing role.
- Update is **`PATCH`** (Bitbucket's is `PUT`), with `UpdateGitlabConnectionRequest { name?, accessToken? }`, a partial update. A new access token is verified (`resolve_scope` is not needed again; `verify_access` against the stored scope is) before it is stored.
- Create: `CreateGitlabConnectionRequest { name, baseUrl (default "https://gitlab.com"), scopeKind: project|group, scopePath, accessToken }` → **201** `GitlabConnectionCreated { connection, webhookUrl, webhookSecret }`. 400 for invalid input or a token missing a scope or role, 403 when the caller may not manage connections, 409 for a duplicate `(baseUrl, scopeKind, scopeId)`.
- `GitlabConnection` is `{ id, provider: "gitlab", name, baseUrl, scopeKind, scopeId, scopePath, createdAt, updatedAt, canManage }` and never carries the token, the webhook secret or the webhook URL. `GitlabConnectionList` is `{ canManage, items }`.
- `baseUrl` is normalised: scheme, host, optional port and path, no trailing slash. Reject anything that is not http(s) with a host.
- Delete: 204, 404, and 409 while a pool, a fleet target or a live runner references the connection. This is stricter than Bitbucket, which ignores fleet targets: in 0047 pool memberships cascade on delete, `fleet_ci_targets` is NO ACTION (so an unchecked delete surfaces as a 500) and `runners` is SET NULL. Check all three first and answer 409. A successful delete also removes both stored secrets (`access_token_key`, `webhook_secret_key`).
- Remove the "Not served yet: implemented by #138." sentences from these four paths' descriptions when the handlers land. Leave `/api/v1/webhooks/gitlab` (#140's) alone.
- The webhook URL returned is the public URL of `POST /api/v1/webhooks/gitlab` (#140 serves it). Bitbucket has no webhook URL; the analogue is GitHub's `effective_webhook_url` (`resources.rs:5214`). Generalise that helper to take the path rather than adding a third URL builder. The GitLab URL is `config base_url` joined with `/api/v1/webhooks/gitlab` and has no override setting.

### Error mapping from `GitLabError`

Every `GitLabError` maps to **400** (`ApiError::BadRequest`), as Bitbucket's upstream errors do; the spec declares no 502 on these operations. Messages: `Unauthorized` → "GitLab rejected the access token"; `MissingScope(s)` → names `s`; `MissingRole` → says the token's user lacks the Maintainer (or Owner) role; `NotFound` (from `resolve_scope`) → names the path; `RateLimited` → asks to retry later; `Upstream(m)` → "GitLab request failed". No response, log line or audit event contains the token or the webhook secret.

### Secrets

- The access token and the webhook secret are sealed with `vault.seal` into `runtime_secrets`, under the connection row's `access_token_key` and `webhook_secret_key`, the way Bitbucket stores its token. #140 reads the webhook secret by that key.
- The webhook secret is generated with `auth::random_token`. Rotation overwrites the value under the same `webhook_secret_key`, so the previous secret stops working at once. A PATCH with a new token overwrites under the same `access_token_key`.
- Create, update, rotate and delete each emit an audit event like Bitbucket's, with no secret in its metadata.

### Code layout and test seam (architecture gate, round 1)

- The handlers go in a new submodule `crates/gridops-api/src/resources/gitlab.rs`, following `resources/tart_host.rs` (`resources.rs` is already 6,203 lines). `resources.rs` keeps only the `mod` line, the D14 predicate and the pool create/update plumbing.
- `crates/gridops-api` has no library target, and every test under `crates/gridops-api/tests/` runs the binary as a child process (`CARGO_BIN_EXE_gridops-api`), which wires the placeholder client. So:
  - **The GitLab handler tests are in-crate `#[cfg(test)]` modules** (for example `resources/gitlab/tests.rs` or a `mod tests` in `resources/gitlab.rs`). The fixture is a temporary SQLite database with migrations applied, `Config::from_env().with_base_url(..)` as in `oauth.rs:836-850`, `Vault::new`, `AppState::new(..).with_gitlab(Arc::new(fake))`, and `crate::session_routes()` driven with `tower::ServiceExt::oneshot` as in `access.rs:375`. These cover create, duplicate, bad token, missing scope, missing role, update (rename and token), verify, rotate, delete-while-targeted (pool, fleet target, live runner), secret non-disclosure across every response, pool create/update with GitLab targets and `runUntagged`, the mixing refusal and the D14 predicate.
  - `crates/gridops-api/Cargo.toml` `[dev-dependencies]` adds `gridops-core` with feature `test-support`, so `FakeGitLab` is reachable.
  - `crates/gridops-api/tests/control_plane_contracts.rs` covers only binary-level cases that need no answer from GitLab: 401 when signed out, 403 for a non-admin, bearer-token refusal, 400 on invalid input, the list over seeded rows, delete 404/409, and the placeholder's mapped 400 on create. Every response body it checks validates against the spec's schema.
- Done-when "HTTP tests cover …" is met by the in-crate tests; "`control_plane_contracts.rs` covers the new routes" by the binary-level cases above.

### Authorization (architecture gate, round 1; replaces the issue's `authorization.rs` item)

`gridops-core/src/fleet/authorization.rs` has no platform-specific logic. Bitbucket's rules live in `fleet/service.rs` (`ensure_pool_access` around :814, `ensure_target_access` around :872) and `fleet/registry/queries.rs` (`readable_targets_cte` around :824), and all depend on `bitbucket_user_grants`. GitLab has no grants table, D11 defers user-scoped GitLab visibility, and no migration may be added. So:

- **GitLab access is system-admin-only until grants exist.** The connection routes require `require_system_admin` (as Bitbucket's do) and are cookie-only (`session_routes()`, `SameOrigin`), so a bearer token is refused.
- `ensure_target_access` in `fleet/service.rs` today treats every non-GitHub target as Bitbucket and reads `row.get::<String>("bitbucket_connection_id")` (:931), which panics on a GitLab target (NULL) once #139 admits them, including for the Reconciler (submit :252, recheck :370). Replace the catch-all `else` with explicit arms: the Bitbucket kinds keep today's rule; a GitLab kind (`gitlab_project`, `gitlab_group`) allows service principals and system admins and refuses other users.
- `ensure_pool_access` requires a system admin for a pool with `runner_pool_gitlab_connections` rows, apart from service principals.
- `readable_targets_cte`, `authorization.rs` and the API token scopes do not change: token scopes are platform-agnostic, so "bot-token scopes treat GitLab the same as Bitbucket" holds without an edit. Record this as a deviation from the issue text.
- Record a deviation: #138 edits `fleet/service.rs` (the two functions above), next to #139's `ensure_target_identity_for_pool` arm. Keep the edit to those two functions.

### Runner pools with GitLab targets (pins the wire names #141 builds against)

- `CreateRunnerPool` and `UpdateRunnerPool` in `crates/gridops-core/src/models.rs` gain `gitlab_connection_ids` (wire `gitlabConnectionIds`; `Vec<String>` with `#[serde(default)]` on create, `Option<Vec<String>>` on update, as Bitbucket's) and `run_untagged` (wire `runUntagged`; `bool`, default false on create, `Option<bool>` on update). Existing struct literals (`control_plane_contracts.rs:699`, the models.rs tests) get the new fields.
- Validation mirrors `validate_bitbucket_connections`: up to 100 unique UUID-shaped ids, and no more connections than `maxCount`. GitLab runners run on Linux and on the vm runtime (D9), so do not copy Bitbucket's Tart-only rule.
- **No mixing.** A pool with GitLab targets has `scope: "gitlab"`, no `repositoryId`/`repositoryIds`, and no `bitbucketConnectionIds`; any other combination is a 400. `scope: "gitlab"` without GitLab connections is a 400. The scope check at `models.rs:182` accepts `"gitlab"`. `installationId` stays required by the existing type and keeps anchoring the pool's ownership and access as today; it does not make a GitHub target. An update may not move a pool between GitLab and non-GitLab scope.
- The pool response (and the pool options / form data the UI loads) gain `runUntagged` and `gitlabConnections` (`[{ id, name, baseUrl, scopeKind, scopePath }]`), shaped like the Bitbucket connections the pool response already carries.
- The handler writes `runner_pools.run_untagged` and the `runner_pool_gitlab_connections` rows (replaced as a set on update, as Bitbucket's are). Admission into fleet targets is #139's (D11).
- Hand-off to #139 (record it in the PR body): the reconciler's pool-target sync must not derive a GitHub target for a pool whose scope is `"gitlab"`; `resources.rs:4362` and `gridops-reconciler/src/main.rs:2669` only act on `"organization"`, so they are already inert for it.

### Runner delete: the GitLab arm (contract change D17, epic #136 comment 7053069685)

- `delete_runner_resources` in `crates/gridops-api/src/resources.rs` (around :4666) sends every non-Bitbucket runner to `cleanup_github_runner`. A `ci_platform = 'gitlab'` row must not reach the GitHub cleanup (nor be queued in `github_runner_cleanup`).
- Add a GitLab arm: when the row has `gitlab_connection_id` and `gitlab_runner_id`, load the connection, build `GitLabCredentials` from its base URL and decrypted token, and call `GitLabApi::delete_runner`. `GitLabError::NotFound` counts as success.
- If the connection is gone (row missing, or `gitlab_connection_id` NULL after SET NULL), the runner id is missing, the token can't be read, or the call fails for any other reason: log a warning with no secret, still complete the GridOps delete, and leave the GitLab runner for #139's orphan sweep (which D17 builds on the new `list_runners`). Never fail a pool or runner delete because GitLab is unreachable. This differs on purpose from Bitbucket's arm, which refuses the delete with 503.
- The same holds for pool deletes that remove or cascade runner rows: each GitLab runner goes through the same arm, and the pool delete completes.
- `list_runners` and `RunnerSummary` are #139's to add to the trait; #138 does not touch the trait and does not call `list_runners`.
- In-crate tests with `FakeGitLab` cover: a successful delete (the fake records `delete_runner` with the right id and credentials), a 404 (`NotFound`) that still deletes the row, GitLab unreachable (`Upstream`) that still deletes the row, and a missing connection that still deletes the row without calling GitLab; and a pool delete with a GitLab runner that completes while GitLab fails.
- Owns: the GitLab arm in `delete_runner_resources` and whatever pool-delete path calls it in `resources.rs`.

### PATCH plumbing

PATCH is new to this API, so the work also covers: the `routing::patch` import and the route lines in `main.rs`; the `access.rs` route-matrix parsers, which skip PATCH today (:213, :247); PATCH in the CORS allowed methods; the "Who may call what" rows in `docs/fleet-api.md`; and the `openapi.rs` test that asserts the "Not served yet" prefix the description edits remove.

### Owns (amended)

The GitLab sections of `resources.rs`, new `resources/gitlab.rs` (+ its tests), `state.rs`, `fleet/service.rs` (the two functions above), the GitLab fields and validation in `gridops-core/src/models.rs`, `crates/gridops-api/Cargo.toml` (dev-dependency), `access.rs` (PATCH parsing), `openapi.rs` (its test), `docs/openapi/gridops-v1.json` (the four paths' descriptions only), `docs/fleet-api.md` (rows), `control_plane_contracts.rs`, and added lines in `main.rs`.

### Injection and production wiring (pinned identically with #140)

#140 needs the same field and writes the same code; whichever PR lands second keeps one copy. So this shape is exact:

- In `crates/gridops-api/src/state.rs`: `pub gitlab: Arc<dyn GitLabApi>` on `AppState`.
- `AppState::new` keeps its signature and initialises the field with a **private `UnconfiguredGitLab` defined in `state.rs`**, whose every method returns `GitLabError::Upstream("GitLab client not available yet".into())`.
- `pub fn with_gitlab(mut self, gitlab: Arc<dyn GitLabApi>) -> Self` sets it.
- `main.rs` gets **no** placeholder or wiring line; #146 adds `.with_gitlab(real)` when its client lands. `main.rs` edits are only the route lines and the `routing::patch` import.
- `crates/gridops-api/Cargo.toml` `[dev-dependencies]` gets `gridops-core = { path = "../gridops-core", features = ["test-support"] }`. Tests inject `gridops_core::gitlab::fake::FakeGitLab` through `with_gitlab`.

### Don't touch

Shared contract types (`gridops-core/src/gitlab/mod.rs`, `fake.rs`, `fleet/domain.rs`, the agent protocol, migrations). Other wave-1 issues own the reconciler (#139), webhooks (#140), scaling (#148), the agents (#147, #142) and the UI (#141). If the contract truly needs a change, stop and report it rather than changing it.
