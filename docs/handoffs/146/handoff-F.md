# Handoff-F: Phase 6 - GitLab HTTP client implementing GitLabApi (#146)

**Date:** 2026-11-20
**Branch:** claude/gitlab-146-client-534ea2 (head 4e81696)
**Issue:** #146

## What was done
- `crates/gridops-core/src/gitlab/http.rs` (new): `GitLabClient`, the real `GitLabApi` over GitLab REST v4. It mounts T's `http_tests.rs` with `#[cfg(test)] #[path = "http_tests.rs"] mod tests;`.
- `crates/gridops-core/src/gitlab/mod.rs`: adds `pub mod http;` and `pub use http::GitLabClient;`. These are the brief's two allowed lines. Nothing else in the contract changed.

## Design decisions
- **Constructor (A-W1).** `#[derive(Debug, Clone)] pub struct GitLabClient { http: reqwest::Client }` with `pub fn new() -> anyhow::Result<Self>`. This is the same shape as `BitbucketClient` and `GitHubClient`. The client holds no token, so its derived `Debug` holds none either. It is re-exported from `gitlab/mod.rs` only, not from `lib.rs`.
- **The request and error seam (A-W2).** Every method goes through the same helpers:
  - `send` returns the raw response for any status. It is the only call to `expose_secret()`, which sets `PRIVATE-TOKEN` with `set_sensitive(true)`. A transport error becomes a fixed `Upstream` description; `reqwest`'s own message names the URL, so it is not repeated.
  - `success` and `error_for` map statuses. Only a 403's body is read, to look for `insufficient_scope`. No body ever goes into an error.
  - `get_json`, `get_all_pages` and `decode` read responses. A decode error drops `serde`'s message, which can quote the body.
  - `delete_runner` checks for 404 before calling `error_for`, as GitHub's `try_delete` does. That avoids Bitbucket's unreachable-404 shape.
- **Constants (A-W3).** `CONNECT_TIMEOUT` is 10s, `REQUEST_TIMEOUT` is 30s and `MAX_PAGES` is 100, matching GitHub's page cap. A server that still has a next page after page 100 gets `Upstream("GitLab listed <what> across more than 100 pages")`. It is not truncated silently.
- **Building the API root.** The client clones `base_url`, clears its query and fragment, then `path_segments_mut().pop_if_empty().extend(["api","v4"]).extend(segments)`.
  - This keeps a path prefix, and a trailing slash does not double up.
  - Each segment is percent-encoded on its own, so `resolve_scope`'s `group/sub/project` becomes one `group%2Fsub%2Fproject` segment. The `url` crate writes `%2F` in uppercase.
- **Pagination.**
  - **`Link` first.** A `rel="next"` entry is found in any `Link` header, at any position, with `rel` matched case-insensitively and `rel` lists allowed. It is resolved against the current page URL and requested as given.
  - **Same origin only.** The `Link` URL is used only if its scheme, host and `port_or_known_default()` equal `credentials.base_url`'s. Otherwise the call is an `Upstream` error.
  - **`x-next-page` otherwise.** With no `Link` next, the client takes `x-next-page`. It must be all digits, and empty or absent means the last page. The next URL is the first-page URL plus `page=N`.
- **`Retry-After`.** The value is trimmed. It must be non-empty and all ASCII digits, then it is parsed as `u64`. This also rejects `+30`, which `u64::from_str` would accept, and an overflow gives `None`.
- **`MissingScope` scope text.** The 403 body's `scope` field is used only if it looks like a scope list: lowercase letters, `_` and spaces, 64 bytes at most. Anything else falls back to `"api"`. Without this check a hostile or buggy server could put arbitrary text, a token included, into the error.
- **`resolve_scope` for a group** sends `?with_projects=false`. GitLab's `GET /groups/:id` otherwise embeds the group's projects in the reply, which is wasted work on a large group. The tests compare the path only, so this is invisible to them. GitLab marks the parameter deprecated, but v4 still honours it.
- **`verify_access`.** The steps are:
  1. `GET /personal_access_tokens/self`. `api` satisfies both needed scopes. `create_runner` is checked before `read_api`.
  2. `GET /user`. `is_admin` defaults to false, because GitLab omits it for non-admins, and an admin passes.
  3. `GET /{projects|groups}/:id/members/all/:user_id`. A 404 there is `MissingRole`. The required level is Maintainer (40) for a project and Owner (50) for a group.
- **Response structs** derive `Deserialize` only, never `Debug`. `NewRunner` holds the runner token as a `String` just long enough to move it into a `SecretString`.
- **No logging.** `http.rs` emits no log lines, so no body can be logged.

## Reuse / extend-vs-new
This extends the #137 contract: it implements the existing `GitLabApi` trait in `gitlab/http.rs`, the file the contract's doc and the epic's ownership table name. No reuse map (`survey.md`) was provided. The brief's analogues, `BitbucketClient` and `GitHubClient`, were followed for:
- the struct and constructor shape;
- `user_agent("GridOps/0.1")`;
- `Policy::none()`.

The client departs from them only where the brief justifies it in writing:
- typed `GitLabError`;
- no body in any error;
- the base URL passed on each call (D2).

No parallel path was created, and no shared code was refactored.

## Architecture notes for A
- **New public type:** `gridops_core::gitlab::GitLabClient`, re-exported from `gitlab/mod.rs`. No contract type, trait or `fake.rs` changed.
- **No new dependencies.** It uses `reqwest`, `secrecy`, `serde`, `serde_json`, `url`, `async-trait` and `anyhow`, all already `gridops-core` dependencies. `Cargo.toml` is untouched.
- **No schema changes.**
- **Wiring is still open (A-W1, epic-level).** Nothing constructs `GitLabClient` in production yet. Wiring `Arc::new(GitLabClient::new()?)` into `gridops-api`'s and `gridops-reconciler`'s `main.rs` belongs to #138, #139 or #143, not to #146.
- **Scheme policy (A-W5).** `http.rs` adds no scheme check. That stays with #138's `base_url` normalisation, and the tests need loopback `http`.

## Deviations from spec / wireframe
- **verify_access role level** (brief-mandated): Maintainer is enough on a project and Owner is required on a group. The issue text says "below Owner" for both; the brief explains why.
- **`MissingScope` sanitisation:** a `scope` value that doesn't look like a scope list is reported as `api`. The brief says "the body's scope field, else api", and this check extends its "no body in any error" rule to that field.
- **`with_projects=false`** on the group lookup. It is an efficiency addition and doesn't change the path the brief pins.
- **Wireframe:** N/A.

## Tier 1 self-check (incl. tests now GREEN)

    $ cargo test -p gridops-core --lib gitlab::http
    running 18 tests
    ... (all 18 ok)
    test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 261 filtered out; finished in 0.06s

    $ cargo test -p gridops-core
    lib: test result: ok. 279 passed; 0 failed
    every integration binary: ok (gitlab_contract 14 passed; gitlab_live 0 passed, 1 ignored; fleet_schema 14 passed; ...)

    $ cargo test -p gridops-core --test gitlab_live -- --list --ignored
    creates_then_deletes_a_project_runner_on_a_real_gitlab: test

    $ cargo fmt --all --check
    (clean)

    $ cargo clippy -p gridops-core --lib -- -D warnings
    Finished (clean)

    $ cargo clippy -p gridops-core --all-targets -- -D warnings
    error: used `assert!` to check that a value is empty --> crates/gridops-core/src/autoscaling.rs:1615:9
    error: used `assert!` to check that a value is empty --> crates/gridops-core/src/autoscaling.rs:1701:9

The two clippy errors under `--all-targets` were already failing before #146 (T's RED handoff reports them). They are in `autoscaling.rs`, which #146 does not own, so I did not fix them. Clippy reports every lint in the crate before it fails, and none is in `gitlab/http.rs`, `http_tests.rs` or `mod.rs`.

    $ cargo check --workspace --all-targets
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 24s

The other crates still build with the new `pub mod`.

I did not run the live test: there is no `GITLAB_TEST_TOKEN`.

## Evidence appendix
`docs/handoffs/146/evidence.md` covers four facts:
- `ExternalId`'s serde `try_from` and its range check;
- `JobStatus`'s `#[serde(other)] Unknown`;
- the redacting `Debug` of `GitLabCredentials` and `CreatedRunner`;
- the `Upstream` Display format.

## Tests that look wrong (for T)
None.

## Known issues
- The `cargo clippy --all-targets -D warnings` gate fails on `autoscaling.rs:1615` and `:1701`. These failures were already there and are outside #146's ownership; O needs to rule on them.
- The live test has not been run, because it needs a real token.
- Nothing wires `GitLabClient` into production yet. That is epic-level (A-W1).

## Files changed
- `crates/gridops-core/src/gitlab/http.rs` (new)
- `crates/gridops-core/src/gitlab/mod.rs` (+3 lines: `pub mod http;`, blank, `pub use http::GitLabClient;`)
