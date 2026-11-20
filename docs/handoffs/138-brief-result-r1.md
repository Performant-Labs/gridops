## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** The migration `migrations/0047_gitlab.sql` "already creates" `gitlab_connections` with the stated columns and unique constraint, plus `runner_pool_gitlab_connections` and `runner_pools.run_untagged` — settle by showing the actual file contents of `migrations/0047_gitlab.sql`.
- **[NV-2]** `resources.rs:5214` is GitHub's `effective_webhook_url` and can be generalised to take a path — settle by showing that function's source.
- **[NV-3]** `fleet/service.rs` `ensure_target_access` (:931) reads `bitbucket_connection_id` in a catch-all `else` arm that would panic on a NULL for GitLab targets, and `ensure_pool_access` is around :814 — settle by showing those functions' source.
- **[NV-4]** `access.rs:213` and `:247` skip PATCH in the route-matrix parsers — settle by showing those lines.
- **[NV-5]** `resources.rs:4362` and `gridops-reconciler/src/main.rs:2669` only act on scope `"organization"`, so a `"gitlab"`-scoped pool is inert to them — settle by showing those code regions.
- **[NV-6]** `models.rs:182` is a scope check that accepts adding `"gitlab"`; `CreateRunnerPool`/`UpdateRunnerPool` field shapes match "as Bitbucket's" — settle by showing `models.rs` around those lines.
- **[NV-7]** `crates/gridops-api/tests/` runs the binary via `CARGO_BIN_EXE_gridops-api` and the crate has no library target, so in-crate tests must drive `session_routes()` with `tower::ServiceExt::oneshot` (as `access.rs:375` does) — settle by showing `access.rs:375` and the crate's `Cargo.toml` targets.
- **[NV-8]** `FakeGitLab` is exposed under feature `test-support` in `gridops-core` and `Vault::new`/`AppState::new(..)` support the described fixture (`oauth.rs:836-850`) — settle by showing `gridops-core` feature declarations and `oauth.rs:836-850`.
- **[NV-9]** `delete_runner_resources` (`resources.rs` ~:4666) currently sends every non-Bitbucket runner to `cleanup_github_runner` — settle by showing that function.
- **[NV-10]** `openapi.rs` contains a test asserting the "Not served yet" description prefix that must be updated — settle by showing that test.
- **[NV-11]** `auth::random_token` exists and `vault.seal` stores into `runtime_secrets` keyed by `access_token_key`/`webhook_secret_key` as Bitbucket does — settle by showing the Bitbucket handler's seal path and `auth::random_token`.

### WARN findings
- **[W-1]** Spec wording "400 naming the refused token, missing scope or missing role" collides superficially with the later hard rule "No response … contains the token." Almost certainly the intent is naming the *reason*, but as written it is ambiguous and an implementer could echo token material. Rewrite as "naming the rejected reason (bad token, missing scope, or missing role) without disclosing the token."
- **[W-2]** Done-when test list omits baseUrl validation: normalization (scheme/host/port/path, no trailing slash) and rejection of non-http(s)/hostless URLs is in scope ("Reject anything that is not http(s) with a host") but appears in no test list. Add it as an explicit acceptance item.
- **[W-3]** The no-mixing rule says a GitLab pool has "no `repositoryId`/`repositoryIds`" while "installationId stays required by the existing type." The brief does not show whether `repositoryId` and `installationId` are the same field; if `installationId` *is* the GitHub repository anchor, the two sentences conflict. State explicitly which field `installationId` is and why requiring it does not violate "no GitHub targets."
- **[W-4]** Delete-while-targeted tests require seeding `fleet_ci_targets` and runner rows referencing a GitLab connection, but per D11 no code path in #138 creates GitLab fleet targets — only manual DB seeding can produce that state. Note in the test plan that these fixtures are seeded directly, so the tests don't imply #138 writes fleet-target rows.
- **[W-5]** The in-crate vs binary test split means the same route set is exercised twice with different harnesses. Recommend the in-crate tests assert the response bodies validate against `docs/openapi/gridops-v1.json` schemas too, not only the binary-level cases, since the in-crate tests are the ones covering the success paths.

### NIT findings
- **[NIT-1]** The issue text's route `…/rotate-webhook-secret` and the spec's `…/webhook-secret/rotate` differ; the spec-wins clause resolves it, but a one-line note in the PR body avoiding the issue-text spelling would prevent copy-paste drift.
- **[NIT-2]** "`control_plane_contracts.rs:699` … get the new fields" cites a struct literal that will move as tests change; consider citing the struct definitions instead.

### Verdict
PASS — no BLOCK findings; implementation may proceed.