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
- Delete: 204, 404, and 409 while a pool or a fleet target references the connection (Bitbucket's rule).
- Remove the "Not served yet: implemented by #138." sentences from these four paths' descriptions when the handlers land. Leave `/api/v1/webhooks/gitlab` (#140's) alone.
- The webhook URL returned is the public URL of `POST /api/v1/webhooks/gitlab` (#140 serves it), built the way Bitbucket's webhook URL is built from config.

### Error mapping from `GitLabError`

`Unauthorized` → 400 "GitLab rejected the access token"; `MissingScope(s)` → 400 naming `s`; `MissingRole` → 400; `NotFound` (from `resolve_scope`) → 400 naming the path; `RateLimited` / `Upstream` → 502 (or the repo's existing upstream-error status for Bitbucket). No response or log line contains the token or webhook secret.

### Injection and production wiring

- `AppState` gains `gitlab: Arc<dyn GitLabApi>`. Keep `AppState::new`'s existing callers compiling with as small a change as possible (for example a builder `with_gitlab(...)`), because `main.rs` is shared with #140.
- The real HTTP client is #146's, in parallel, and is not on master. Until it lands, production wires a small placeholder implementation that returns `GitLabError::Upstream("GitLab client not available yet")` from every method, so the server builds and runs; #146 replaces that one line in `main.rs`. Tests inject `gridops_core::gitlab::fake::FakeGitLab` (feature `test-support`) as `Arc<dyn GitLabApi>`.

### Don't touch

Shared contract types (`gridops-core/src/gitlab/mod.rs`, `fake.rs`, `fleet/domain.rs`, the agent protocol, migrations). Other wave-1 issues own the reconciler (#139), webhooks (#140), scaling (#148), the agents (#147, #142) and the UI (#141). If the contract truly needs a change, stop and report it rather than changing it.
