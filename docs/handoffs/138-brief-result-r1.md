## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** D16's description of `migrations/0047_gitlab.sql` (tables, columns, unique on `(base_url, scope_kind, scope_id)`, `run_untagged` column) — the migration file itself; if the unique constraint or a column the handlers rely on (e.g. `webhook_secret_key`) is absent, the brief's "add no migration" instruction blocks implementation.
**[NV-2]** `GitLabError` has exactly the variants the error-mapping table assumes (`Unauthorized`, `MissingScope(s)`, `MissingRole`, `NotFound`, `RateLimited`, `Upstream(String)`) — excerpt of `gridops-core/src/gitlab/mod.rs` (the "Don't touch" section forbids changing it, so the mapping must match it exactly).
**[NV-3]** `GitLabApi` method signatures match the D2/D3 claims (`&GitLabCredentials` first arg; `resolve_scope(credentials, ScopeKind, path)`), and `verify_access` can be called against a stored scope without re-resolving — excerpt of the trait in `gridops-core/src/gitlab/mod.rs`.
**[NV-4]** `FakeGitLab` exists behind feature `test-support` in `gridops-core` — excerpt of `gridops-core/src/gitlab/fake.rs` / the crate's `Cargo.toml` feature list.
**[NV-5]** `starts_belong_to_reconciler` in `crates/gridops-api/src/resources.rs` is today `providers.iter().any(|p| p == "tart")` and its only callers are in `resources.rs` (D14 asserts this; it bounds the blast radius of the predicate change) — excerpt of `resources.rs`.
**[NV-6]** Bitbucket's "rule" for delete-while-targeted (409) and duplicate-connection 409 behavior, which the brief says to mirror — excerpt of the Bitbucket handlers in `resources.rs`.
**[NV-7]** The Bitbucket webhook-URL construction "from config" that the GitLab webhook URL must mirror — excerpt of the Bitbucket connection handler and the config type it reads.
**[NV-8]** The four GitLab path descriptions in `docs/openapi/gridops-v1.json` actually contain the "Not served yet: implemented by #138." sentences to remove, and that the routes/status codes quoted (PATCH update, 204 verify, `/webhook-secret/rotate`) match the file — excerpt of the spec.

### WARN findings
**[W-1]** "RateLimited / Upstream → 502 (or the repo's existing upstream-error status for Bitbucket)" leaves the status code ambiguous — pick one status in the brief so the acceptance tests are deterministic.
**[W-2]** The Done-when test list omits the list endpoint (`GET …/gitlab`): no test covers `GitlabConnectionList { canManage, items }` or that list responses omit secrets (secret non-disclosure is only specified for "other responses" generally). Add an explicit list/secret-non-disclosure-on-list criterion.
**[W-3]** Duplicate detection (409 on `(baseUrl, scopeKind, scopeId)`) requires create to run `resolve_scope` first to obtain `scope_id`; the brief doesn't state the ordering (resolve-before-duplicate-check vs. catch a DB unique-violation). Specify it so the duplicate-path test expectation is unambiguous.
**[W-4]** `AppState::new` change mechanism is left open ("for example a builder `with_gitlab(...)`") while `main.rs` is shared with #140 — a coordination hazard; name the exact mechanism so #140 doesn't conflict.

### NIT findings
**[NIT-1]** The issue text's rotate route (`…/rotate-webhook-secret`) and update verb (`PUT` for Bitbucket) are superseded by the spec; consider one-line strikethroughs to avoid future readers re-implementing the wrong route.
**[NIT-2]** `baseUrl` normalization ("scheme, host, optional port and path, no trailing slash") doesn't say whether comparison for duplicate detection uses the normalized form (it should — state it).

### Verdict
PASS — no BLOCK findings; implementation may proceed.