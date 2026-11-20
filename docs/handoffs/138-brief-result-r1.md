## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** The migration `migrations/0047_gitlab.sql` already exists with exactly the described tables/constraints (`gitlab_connections` unique on `(base_url, scope_kind, scope_id)`, `runner_pool_gitlab_connections`, `runner_pools.run_untagged`; cascade / NO ACTION / SET NULL FKs) — the actual contents of `migrations/0047_gitlab.sql`.
- **[NV-2]** `ensure_target_access` at `fleet/service.rs:931` reads `bitbucket_connection_id` via a panicking `row.get::<String>` on a NULL, i.e., the catch-all `else` would panic on a GitLab target — the source at that line.
- **[NV-3]** `resources.rs:1609/1620/1639/1756/2537`, `:4189`, `:4362`, `fleet/profiles.rs:195`, `webhooks.rs:774` are inert for `scope='gitlab'` as claimed — those lines' text.
- **[NV-4]** `models.rs:182` accepts `"gitlab"` as a pool scope, and `models.rs:445` requires `runner_group_id > 0` — the model validation code.
- **[NV-5]** `resources.rs:2569` (create) and `:1676` (update) store `input.runner_group_id` for non-repository scopes and update compares it for `runtime_changed` — those lines.
- **[NV-6]** `fleet/targets.rs::pool_targets` (~:854-905) treats every non-`repository` scope as GitHub organization, and `fleet.rs:176` fails closed when no target is returned — the reconciler and targets source.
- **[NV-7]** `resources.rs:5214` (`effective_webhook_url`) and `resources.rs:4666-4756` (`delete_runner_resources`, `RunnerAccess` at :137-139) have the shapes the brief describes — those excerpts.
- **[NV-8]** The spec `docs/openapi/gridops-v1.json` actually declares the GitLab paths, statuses (201/204/400/403/409), `GitlabWebhookSecret`, and the "Not served yet" description prefixes the `openapi.rs` test asserts — the spec file and the test.
- **[NV-9]** `FakeGitLab` is exported under `gridops-core` feature `test-support` at `gridops_core::gitlab::fake::FakeGitLab`, and supports the delete/credentials-recording behavior the tests assume — `gridops-core/src/gitlab/fake.rs`.
- **[NV-10]** `AppState::new` can gain a field without breaking its signature for other callers, and `session_routes()` is reachable via `tower::ServiceExt::oneshot` from an in-binary-crate `#[cfg(test)]` module (`access.rs:375`, `oauth.rs:836-850` patterns) — `state.rs`, `access.rs`, `oauth.rs`.
- **[NV-11]** `crates/gridops-api/Cargo.toml` currently lacks a `gridops-core` dev-dependency, and gridops-core builds under `test-support` without a cycle — both `Cargo.toml` files.
- **[NV-12]** Bitbucket's duplicate-connection 409 (commit 3629667) and delete-while-targeted rule exist as described — the referenced commit's diff.

### WARN findings
- **[W-1]** Forcing `runner_group_id = 1` on create/update silently ignores client input, but the pool create/update response shape and spec section aren't listed as owned or updated to document this. Recommend stating whether the response reflects the forced value and adding it to the spec-ownership list if a schema change is needed.
- **[W-2]** "Bad token" tests (create/update with an invalid token) must distinguish `Unauthorized` from `MissingScope`/`MissingRole` messages per the error-mapping table, but the Done-when list doesn't require per-variant message assertions. Recommend an explicit criterion: each `GitLabError` variant's mapped message is asserted at least once.
- **[W-3]** The rotate route returns `webhookUrl` and `webhookSecret` on every rotation, while the top of the brief says the secret is returned "once" (at create). These are reconcilable (create returns once; rotate re-discloses), but the wording invites a misreading; recommend a sentence pinning that rotate re-discloses the new secret.

### NIT findings
- **[NIT-1]** "PATCH in the CORS `allow_methods` (around `main.rs:170`)" is stated twice in the PATCH plumbing section.
- **[NIT-2]** "delete-while-targeted" in the issue's Done-when is expanded to three variants (pool, fleet target, live runner) only in the clarifications; fine, but the Done-when wording could carry the three cases forward.

### Verdict
PASS — no BLOCK findings; implementation may proceed.