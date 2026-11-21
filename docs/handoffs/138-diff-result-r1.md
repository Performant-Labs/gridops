## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** crates/gridops-api/src/resources/gitlab_tests.rs:1849 — the test `deleting_a_gitlab_pool_completes_while_gitlab_fails` assumes the pool-delete path routes each runner row through `delete_runner_resources` (the GitLab arm); the brief's D17 section says pool deletes "each GitLab runner goes through the same arm," but no excerpt in this prompt shows the pool-delete code calling it — the evidence would be the pool-delete function in `resources.rs` (its call to `delete_runner_resources` / the cascade path).
- **[NV-2]** crates/gridops-api/src/resources/gitlab.rs:139 — `state.runtime_secret(key)` and `state.vault.seal/open` signatures (returns `Option<String>`, sync seal) are assumed by `verify_gitlab_connection`, `remove_gitlab_runner` and the tests; no excerpt of `state.rs`'s `runtime_secret` or `Vault` is shown. `crates/gridops-api/src/state.rs` (runtime_secret) and `gridops-core` `Vault` would settle it.
- **[NV-3]** crates/gridops-core/src/gitlab/fake.rs — the tests assert `FakeGitLab` "never sees the token" and that `with_scope`/`fail_next`/`calls`/`runners` exist with those signatures; only `call` and `delete_runner` are excerpted. An excerpt of the remaining `FakeGitLab` methods would settle whether the "right credentials" reasoning (base-URL keying only) holds.
- **[NV-4]** crates/gridops-api/src/resources/gitlab.rs:604 — `is_unique_violation` assumes the insert error surfaces as `ApiError::Internal` carrying the `sqlx::Error` (`?` on the sqlx error inside the `async` block converts via `#[from] anyhow::Error`); this depends on the `From<sqlx::Error> for ApiError` chain shown nowhere in this prompt. The error.rs excerpt stops before any such impl.
- **[NV-5]** docs/fleet-api.md:189 — the new docs rows claim the GitLab GET route refuses a bearer token with `403 token_not_accepted`; this is enforced by `token_routes()` not containing the route, which the diff does not show (only `session_routes()` lines are in the diff). The `token_routes()` function in `main.rs` would settle it (and the token_access matrix test only checks documented rows).

### WARN findings
- **[W-1]** crates/gridops-api/src/resources/gitlab.rs:574 — `stored_connection` maps any `scope_kind` other than `"group"` to `Scope::Project`, so a corrupted/unknown value is silently reinterpreted instead of refused; the explicit-arm discipline applied elsewhere (e.g. `ensure_target_access`, `delete_runner_resources`) is not applied here. Recommend an explicit match with an error arm.
- **[W-2]** crates/gridops-api/src/resources/gitlab.rs — the D17 "missing connection row" case (non-NULL `gitlab_connection_id` whose row is gone) is handled in code but has no dedicated in-crate test (the tests cover NULL id, missing runner id, unreachable, NotFound and pool-delete); the brief's test list names "a missing connection that still deletes the row without calling GitLab," which only the NULL-connection case partially covers. Recommend adding the stale-id case.
- **[W-3]** crates/gridops-api/src/resources/gitlab.rs:70 — `gitlab_connections` (GET) returns `canManage` computed but performs no `require_*` check beyond `AuthUser`; the spec description says any signed-in user may list — consistent, but confirm this matches Bitbucket's GET (not shown) to avoid spec drift in disclosure between the two connection types.
- **[W-4]** crates/gridops-api/src/resources.rs:1770 — for a non-GitLab pool, `input.run_untagged` is silently ignored on update (and on create stored as `false`); the client gets 200 with the field discarded rather than a 400. Defensible per the handoff's design note, but it is a silent data-loss path worth a WARN.

### NIT findings
- **[NIT-1]** crates/gridops-api/src/resources/gitlab.rs:232 — the duplicate-check comment says checking before `verify_access` "names the connection to edit instead," but the check runs after `resolve_scope`, so a bad token on a duplicate still costs one GitLab call before the 409; the ordering rationale in the comment only holds for the token-valid case.
- **[NIT-2]** crates/gridops-api/src/resources.rs:1867 — link-row `created_at` is faked with `now + position` to preserve order; a comment (present in tests, absent here) would help future readers since it is a synthetic timestamp.
- **[NIT-3]** crates/gridops-api/src/resources/gitlab.rs:60 — `CreateGitlabConnection.base_url` is `Option<String>` while every other field is defaulted `String`; the asymmetry is only justified by the `DEFAULT_BASE_URL` fallback — fine, but the `#[serde(default)]` on an `Option` is redundant.

### Verdict
PASS — no BLOCK findings; testing may proceed.