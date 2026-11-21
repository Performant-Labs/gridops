# [GitLab: HTTP client implementing GitLabApi](https://github.com/Performant-Labs/gridops/issues/146)

Part of #136. **Wave 1.** Depends only on #137 (merged as `b22c451`).

## Need

The real HTTP implementation of the `GitLabApi` trait that #137 defines. Every other wave-1 issue tests against `FakeGitLab`, so this one runs alongside them.

## Scope (issue text)

Implement `GitLabApi` in `crates/gridops-core/src/gitlab/http.rs` as `GitLabClient`, with a configurable base URL like `BitbucketClient::with_base_url`:

- `create_runner` → `POST /user/runners`, with `runner_type` set to `project_type` or `group_type`. GitLab returns the token only once.
- `delete_runner` → `DELETE /runners/:id`. A 404 counts as success.
- `job` → `GET /projects/:id/jobs/:job_id`, including `tag_list`.
- `pending_jobs` → `GET /projects/:id/jobs?scope[]=pending`, following `Link`/`x-next-page` pagination.
- `resolve_scope` → `GET /projects/:path` or `GET /groups/:path` (URL-encoded).
- `verify_access`: report a token without `create_runner`/`read_api` as `MissingScope`, and a caller below Owner as `MissingRole`.

Map 401, 403, 404 and 429 (with `Retry-After`) to `GitLabError`. Never log a request or response body that contains a token.

**Owns:** `crates/gridops-core/src/gitlab/http.rs` only.

## Done when (issue text)

- Tests against a mock HTTP server cover every method, each error mapping, pagination, and a check that the token never appears in any `Debug` output or error.
- One ignored test (`#[ignore]`, run by hand with `GITLAB_TEST_TOKEN`) exercises create, then delete, against a real gitlab.com project.

## Contract as shipped (these win over the issue text)

The epic's "Contract deviations shipped by #137" table (latest comment on #136) replaces the issue text where they differ. The contract is `crates/gridops-core/src/gitlab/mod.rs` on master; rationale in `docs/handoffs/137-brief.md`.

- **D2.** Every method takes `credentials: &GitLabCredentials { base_url, access_token }` first. One `Arc<dyn GitLabApi>` serves every connection, so the base URL comes from the credentials on each call, not from the client. `GitLabClient::new()` builds the HTTP client only. A `with_base_url`-style constructor is therefore not needed for configurability; the base URL is already per call. (Tests point `credentials.base_url` at the mock server.)
- **D3.** `resolve_scope(credentials, kind: ScopeKind, path)`: `ScopeKind::Project` → `GET /projects/:path`, `ScopeKind::Group` → `GET /groups/:path`, the path URL-encoded as one segment (`group/sub/project` → `group%2Fsub%2Fproject`). Returns `Scope::Project { id, path }` / `Scope::Group { id, path }` with `path` = GitLab's `path_with_namespace` (project) or `full_path` (group).
- **D4.** `group_projects(credentials, group_id)` → `GET /groups/:id/projects?include_subgroups=true`, paginated, returning every project id.
- **Retry-After rule (137-brief, B-6).** A header value that is a non-negative decimal integer `n` (surrounding whitespace trimmed) gives `Some(Duration::from_secs(n))`; an absent header gives `None`; any other value (HTTP-date included, `-1` included) gives `None`. A unit test covers `"30"`, `" 30 "`, absent, `"Tue, 05 Jan 2021 11:00:00 GMT"`, `"-1"`.

**Do not change any contract type** (`mod.rs`'s types, the trait, `GitLabError`, `fake.rs`). Six other wave-1 sessions are editing other files concurrently. The only edit allowed outside `http.rs` is the single line `pub mod http;` in `crates/gridops-core/src/gitlab/mod.rs` (plus a `pub use http::GitLabClient;` if wanted), which the module needs in order to compile. No new crate dependencies: the mock HTTP server in tests is built on `tokio::net::TcpListener` (tokio has `full`) or `axum` only if it is already a dependency of `gridops-core` (it is not; do not add it). Any needed `Cargo.toml` change is a stop-and-report.

## Clarifications (verified against GitLab's REST docs)

### Request basics
- API root: `credentials.base_url` joined with `api/v4/` (a base URL with a path prefix such as `https://host/gitlab` must keep that prefix; a trailing slash on the base must not double up).
- Auth: header `PRIVATE-TOKEN: <token>`, marked sensitive (`HeaderValue::set_sensitive(true)`). The token never goes into a URL.
- `reqwest::Client` with `user_agent("GridOps/0.1")` and `redirect(Policy::none())` (as `BitbucketClient`), so a redirect cannot carry the token to another host. A request timeout is set.
- List calls send `per_page=100`.

### Pagination
- Follow `Link: <…>; rel="next"` when present, otherwise `x-next-page` (empty or absent = last page).
- A `Link` next URL is followed **only** if it has the same scheme, host and port as `credentials.base_url`; otherwise it is an `Upstream` error (never send the token to another origin).
- A hard page cap (e.g. 1000 pages) stops a looping server with an `Upstream` error.

### Error mapping (every method)
- 401 → `Unauthorized`.
- 403 whose JSON body has `"error": "insufficient_scope"` → `MissingScope(<scope from the body's "scope" field, else "api">)`; any other 403 → `MissingRole`.
- 404 → `NotFound` (except `delete_runner`, where 404 → `Ok(())`).
- 429 → `RateLimited { retry_after }` by the rule above.
- Any other non-2xx → `Upstream("GitLab returned <status>")`. Transport and JSON decode errors → `Upstream(...)` too.
- **No error string, log line or `Debug` output contains a response body** (the `create_runner` body holds the `glrt-` token) **or the access token.** Upstream messages carry the status code and a fixed description only. Decode errors do not echo the body.

### create_runner
`POST /user/runners` with JSON: `runner_type` (`project_type` + `project_id`, or `group_type` + `group_id`, from the `Scope`), `tag_list` (comma-joined or array), `run_untagged`, `description`. Response `{ "id", "token", ... }` → `CreatedRunner { id, token: SecretString }`.

### job / pending_jobs
`GET /projects/:id/jobs/:job_id` → `Job { id, project_id, pipeline_id: pipeline.id, status, tag_list }`. The job JSON has no top-level `project_id` field; use the `project_id` the caller passed. `pending_jobs` uses `scope[]=pending`, all pages. An unknown status deserializes as `JobStatus::Unknown` (contract behaviour).

### verify_access(credentials, scope)
1. Scopes: `GET /personal_access_tokens/self` → `scopes`. `create_runner` is satisfied by `create_runner` or `api`; `read_api` by `read_api` or `api`. First missing one → `MissingScope("create_runner")` / `MissingScope("read_api")`.
2. Role: `GET /user` → `id` (and `is_admin`; an admin passes). Then `GET /projects/:id/members/all/:user_id` or `GET /groups/:id/members/all/:user_id` → `access_level`. A 404 there, or `access_level` below the required level → `MissingRole`.
   - Required level: **Owner (50) for a group scope; Maintainer (40) for a project scope.** This refines the issue's "below Owner": GitLab's `POST /user/runners` requires Owner for `group_type` but only Maintainer for `project_type`, so rejecting a project Maintainer would refuse a token that can in fact create the runner. Report this as a deviation from the issue text.

### Debug / redaction
- `GitLabClient`'s `Debug` (derive or manual) holds no token (it holds none). Tests assert that the token string appears in none of: `format!("{:?}", credentials)`, `format!("{:?}", created_runner)`, `format!("{:?}", err)` / `err.to_string()` for every error path, including a `create_runner` whose response is malformed but contains the `glrt-` token.

### Tests
- In `http.rs` (`#[cfg(test)] mod tests`) or a sibling test module file under `src/gitlab/` owned by this issue. A small in-process mock server (tokio `TcpListener`, hand-written HTTP/1.1 responses) records requests (method, path+query, headers) so tests assert the exact path, query, `PRIVATE-TOKEN` header and JSON body.
- Coverage: every trait method's success path; 401/403 (scope and role)/404/429 (with and without `Retry-After`)/500 mapping; `delete_runner` 404 = success; pagination via `Link` and via `x-next-page`; refusal of a cross-origin `Link`; base URL with a path prefix; the Retry-After parser cases; verify_access missing-scope, missing-role (project Maintainer passes, group Maintainer fails), admin passes, non-member 404 → `MissingRole`; the token-never-leaks checks.
- One `#[ignore]` test reading `GITLAB_TEST_TOKEN`, `GITLAB_TEST_PROJECT` (a path) and optional `GITLAB_TEST_URL` (default `https://gitlab.com`): resolve the project, create a project runner, delete it.

### Gates
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` (workspace lints), and `cargo test -p gridops-core` pass.
