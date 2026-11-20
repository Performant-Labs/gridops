# Handoff-T-red: Phase 4 - GitLab HTTP client implementing GitLabApi (#146)

**Date:** 2026-11-20
**Branch:** claude/gitlab-146-client-534ea2 (head 09d3cf6)
**Brief / wireframe reviewed:** docs/handoffs/146-brief.md, docs/handoffs/146-brief-result-r1.md, docs/handoffs/146/handoff-A.md; wireframe N/A (no UI surface)

## A precondition

Confirmed: A returned **PASS** on the plan (`docs/handoffs/146/handoff-A.md`), with five warnings and no blocks. The suite adopts the three warnings that change what a test can observe:

- **W1 (constructor shape).** Tests call `GitLabClient::new()` through `.context(..)`, so `anyhow::Result<Self>` works and so would any std error. They need `GitLabClient: Debug`, which the brief requires. `Clone` is not pinned. The live test imports `gridops_core::gitlab::GitLabClient`, so **the `pub use http::GitLabClient;` re-export in `gitlab/mod.rs` is required**.
- **W3 (constants).** No test names `MAX_PAGES` or the timeouts. The page-cap test accepts any cap from 2 to 1000 pages. A draft run with a cap of 1000 took 0.36s, so no test-only cap hook is needed.
- **W4 (file layout).** The unit tests live in `gitlab/http_tests.rs`. The live test is `tests/gitlab_live.rs` and uses the reasoned `#[ignore = …]` form, with a module doc that gives the run command.

W2 (the `send`/`error_for` seam) is F's design. The tests pin only its effects: a 404 is forgiven on delete and nowhere else, and no body is echoed. W5 (scheme policy) is not in #146's scope, and no test pins it. If F adds a scheme check anyway, it **must allow `http` to loopback**, because every test points at `http://127.0.0.1:<port>`.

## Tests authored

### F must add, in `crates/gridops-core/src/gitlab/http.rs`

    #[cfg(test)]
    #[path = "http_tests.rs"]
    mod tests;

and, in `gitlab/mod.rs`, `pub mod http;` plus `pub use http::GitLabClient;`. These are the brief's allowed mod.rs lines.

### `crates/gridops-core/src/gitlab/http_tests.rs` (new; unit, with an in-process mock server)

This is the cheapest tier that can observe the behaviour. The client's whole contract is what goes over HTTP, so every test drives `GitLabClient` against `Mock`. `Mock` is a tokio `TcpListener` server with no new dependency. It handles one request per connection, reads a `Content-Length` body (it refuses chunked bodies), and replies with `Connection: close`, as A-W4 and brief-review W-2 asked. It records each request's method, raw target, headers and body.

Query parameters are compared as **decoded, sorted pairs**, so `scope[]=pending` and `scope%5B%5D=pending` compare equal and parameter order is free. Paths are compared **raw**, so the `%2F` encoding is observable. Each request is checked with `assert_authenticated`: `PRIVATE-TOKEN` equals the token, `User-Agent` is `GridOps/0.1`, and the token is not in the URL.

| Test | What it pins (brief clause) |
|---|---|
| `create_runner_registers_a_project_runner_and_returns_its_one_time_token` | `POST /api/v4/user/runners` with a JSON body: `runner_type: project_type`, `project_id: 42`, no `group_id`, `run_untagged`, `description`, and `tag_list` as an array or a comma-joined string. The 201 `{id, token}` becomes `CreatedRunner { id, token }`. |
| `create_runner_registers_a_group_runner_with_the_group_id` | `group_type` + `group_id: 7`, no `project_id`, `run_untagged: true`, no tags sent. |
| `delete_runner_deletes_by_id_and_counts_an_already_gone_runner_as_deleted` | `DELETE /api/v4/runners/:id`: 204 is `Ok`, 404 is `Ok`, and 403 is still `MissingRole`, so only 404 is forgiven. This catches the Bitbucket unreachable-404 shape that A-W2 warned about. |
| `job_reads_one_job_with_its_pipeline_tags_and_the_callers_project` | `GET /api/v4/projects/42/jobs/1234`. The `Job` gets `pipeline_id` from `pipeline.id`, `tag_list`, and the **caller's** `project_id` (the fixture JSON has no top-level `project_id`). |
| `pending_jobs_follows_the_link_header_to_every_page` | The first request is `scope[]=pending` + `per_page=100`. `rel="next"` is found even when it is not the first link. The `next` URL is requested **as given**: it carries a `cursor=link-only` parameter that only the Link URL has. Pages are concatenated in order. |
| `pending_jobs_follows_x_next_page_when_there_is_no_link` | With no `Link`, `x-next-page: 2` produces `page=2` with the same `scope[]` and `per_page`, and an empty `x-next-page` ends the loop. An unknown status decodes as `JobStatus::Unknown`. |
| `group_projects_lists_every_project_of_the_group_and_its_subgroups` | D4: `GET /api/v4/groups/7/projects?include_subgroups=true&per_page=100` across 2 pages, returning every id. An absent `x-next-page` ends the list. |
| `a_later_page_failing_fails_the_whole_list` | A 500 on page 2 is `Upstream`, not a partial list. The test asserts that page 1 succeeded and page 2 was requested. |
| `a_server_paginating_past_the_page_cap_is_cut_off_with_an_error` | The mock paginates up to page 1001. The client must return `Upstream` after 2 to 1000 requests. A client with no cap gets a 1001-item list and fails the assertion. It doesn't hang. |
| `resolve_scope_looks_up_the_url_encoded_path_and_keeps_gitlabs_spelling` | D3: `GET /api/v4/projects/Acme%2FSub%2FAPI` becomes `Scope::Project { 42, "acme/sub/api" }` (the `path_with_namespace`, not the input), and `GET /api/v4/groups/Acme%2FSub` becomes `Scope::Group { 8, "acme/sub" }` (the `full_path`). An unknown path is `NotFound`. |
| `the_api_root_keeps_a_base_url_path_prefix_with_or_without_a_trailing_slash` | `http://host/gitlab` and `http://host/gitlab/` both produce `/gitlab/api/v4/runners/5`. |
| `error_statuses_map_to_gitlab_errors` | On `job`: 401 → `Unauthorized`; 403 `insufficient_scope` with `scope: read_api` → `MissingScope("read_api")`; 403 `insufficient_scope` with no scope → `MissingScope("api")`; any other 403 (JSON or plain text) → `MissingRole`; 404 → `NotFound`; 400, 500 and 502 (HTML body) → `Upstream` whose message contains the status code. |
| `retry_after_is_honoured_only_as_whole_seconds` | B-6 on 429: `"30"` → 30s, `" 30 "` → 30s, `"0"` → 0s, absent → `None`, the HTTP-date → `None`, `"-1"` → `None`. |
| `every_method_maps_unauthorized_and_rate_limited_responses` | The shared seam: all 7 methods, called through `&dyn GitLabApi`, map 401 to `Unauthorized` and 429 with `Retry-After: 7` to `RateLimited { Some(7s) }`. |
| `verify_access_reports_the_first_scope_the_token_lacks` | `GET /personal_access_tokens/self`. `api` satisfies both scopes. A token missing `create_runner` gets `MissingScope("create_runner")` first, and one missing only `read_api` gets `MissingScope("read_api")`. There are 8 scope sets. |
| `verify_access_requires_maintainer_on_a_project_and_owner_on_a_group` | `GET /user`, then `GET /{projects\|groups}/:id/members/all/:user_id`. A project Maintainer (40) passes and a Developer (30) fails. A group Owner (50) passes and a group Maintainer (40) fails. A 404 non-member gets `MissingRole`. An admin passes, member or not. A non-admin's `/user` **omits** `is_admin`, as GitLab does. Each non-admin case asserts the three GETs were made. |
| `the_token_is_never_sent_to_another_origin` | A `Link` next on another port (a live trap server), on another host (`localhost`, same port) or on another scheme (`https`) is `Upstream` after exactly one request. A 302 to the trap is `Upstream` containing "302". The trap receives **no** request. |
| `neither_token_appears_in_any_error_or_debug_output` | This test checks that no error leaks the access token or the runner token. It covers four inputs:<br>- Error bodies that echo both tokens: Display and Debug of every method's error, for 400, 401, 403, 404, 409, 429, 500 and 503, are secret-free.<br>- Malformed 2xx bodies: an object with the `glrt-` token as `id`, a JSON array of the tokens, and non-JSON text. serde quotes back a string it can't decode, so a naive decode error would leak. Every non-delete method must give a secret-free `Upstream`.<br>- A transport error (nothing listening on the port): every method gives a secret-free `Upstream`.<br>- After a real create, the Debug output of `CreatedRunner`, `GitLabCredentials` and `GitLabClient` is secret-free. |

### `crates/gridops-core/tests/gitlab_live.rs` (new; integration, `#[ignore]`)

| Test | What it pins |
|---|---|
| `creates_then_deletes_a_project_runner_on_a_real_gitlab` | The issue's live check, through the public re-export and `Arc<dyn GitLabApi>`. It reads `GITLAB_TEST_TOKEN`, `GITLAB_TEST_PROJECT` and an optional `GITLAB_TEST_URL` (default `https://gitlab.com`). Then it runs `resolve_scope(Project)` and `verify_access`, creates a runner tagged `gridops-live-test`, deletes it, and deletes it again (a real 404 is success). It deletes **before** asserting that the token starts with `glrt-`, so a failure leaves no runner behind. The run command is in the module doc. |

### Deliberately not tested (Tier 2 read in Phase 6)

- **The request timeout.** A test would need a 30s stall.
- **"Never log a body".** `gridops-core` has no `tracing-subscriber` dev-dependency to capture logs, and the brief allows no new dependencies.
- **`set_sensitive(true)`.** This is not observable over the wire.

I'll check all three by reading `http.rs` in Phase 6. The `" 30 "` Retry-After case runs on the wire, but hyper strips the surrounding whitespace before the client sees the value, so the trim branch itself can't be observed here.

## RED confirmation

### On the real tree (no `http.rs` yet): compile RED

`cargo test -p gridops-core --no-run`:

    error[E0432]: unresolved import `gridops_core::gitlab::GitLabClient`
      --> crates/gridops-core/tests/gitlab_live.rs:21:39
    error: could not compile `gridops-core` (test "gitlab_live") due to 1 previous error

That is the missing feature: the client and its re-export. `http_tests.rs` is not compiled yet because `http.rs`, which mounts it, does not exist. Until F adds `http.rs`, the `gitlab_live` test binary does not compile, which is inherent to RED over a new type in Rust (the same as #137). The lib test binary is unaffected.

### With a throwaway stub: runtime RED (scratchpad only, removed, never staged)

To show that each test fails on its assertion and not on setup, I mounted a stub `http.rs` for one run, with `pub mod http; pub use http::GitLabClient;` added to mod.rs. In the stub, `new()` builds a client and every trait method returns `Err(Upstream("not implemented"))`. Afterwards I deleted `http.rs` and restored mod.rs byte for byte; `git status` shows only the two new test files.

`cargo test -p gridops-core --lib gitlab::http`: **0 passed; 18 failed**. Each failure is the missing behaviour:

    create_runner_* (x2), job_*, pending_jobs_* (x2), group_projects_*, resolve_scope_*,
    the_api_root_*, neither_token_*:        Error: GitLab request failed: not implemented
    delete_runner_*:                        left: Err(Upstream("not implemented"))  right: Ok(())
    error_statuses_map_to_gitlab_errors:    401: got Upstream("not implemented")
    retry_after_*:                          Retry-After Some("30")  left: Err(Upstream("not implemented"))  right: Err(RateLimited { retry_after: Some(30s) })
    every_method_maps_*:                    create_runner  left: Err(Upstream("not implemented"))  right: Err(Unauthorized)
    verify_access_reports_*:                scopes ["api"]  left: Err(Upstream("not implemented"))  right: Ok(())
    verify_access_requires_*:               Project admin=false access_level=Some(40)  left: Err(..)  right: Ok(())
    a_later_page_failing_*:                 the error comes from page 2, after page 1 succeeded  left: []  right: [None, Some("2")]
    a_server_paginating_past_the_page_cap_*: the cap stops the loop within 1000 pages, after 0
    the_token_is_never_sent_*:              only the first page is requested  left: 0  right: 1

In the first draft, `a_later_page_failing_fails_the_whole_list` **passed** against the stub, because any `Upstream` satisfied it, so it was not a valid RED. I tightened it to require that page 2 was actually reached, and it now fails as shown.

### The suite is satisfiable: throwaway draft implementation (removed, never staged)

The mock server and the matchers are new code, and errors there would be routed to F. So I checked the suite against a throwaway draft `http.rs` built to the brief and to A's pins. The draft used `send`/`error_for`/`get_json`/`get_all_pages`, the constants 10s/30s/100, `PRIVATE-TOKEN` marked sensitive, and `Policy::none()`.

- **Result:** **18 passed**. `cargo clippy -p gridops-core --all-targets -- -D warnings` reports nothing in either test file, and `rustfmt --check` passes on both. `--test gitlab_live` compiles and lists the test as ignored with its reason.
- **Mutation spot-checks** (each applied alone to the draft): every one turned at least one test red, and none hung.

| Mutation | Test(s) that failed |
|---|---|
| default redirect policy | `the_token_is_never_sent_to_another_origin` |
| same-origin check removed | `the_token_is_never_sent_to_another_origin` |
| `Link` ignored | `the_token_is_never_sent_to_another_origin`, `pending_jobs_follows_the_link_header_to_every_page` |
| `Link` not followed verbatim | `pending_jobs_follows_the_link_header_to_every_page` |
| no page cap | `a_server_paginating_past_the_page_cap_is_cut_off_with_an_error` |
| serde error echoed into `Upstream` | `neither_token_appears_in_any_error_or_debug_output` |
| response body echoed into `Upstream` | `neither_token_appears_in_any_error_or_debug_output` |
| group needs only Maintainer | `verify_access_requires_maintainer_on_a_project_and_owner_on_a_group` |
| admin not honoured | `verify_access_requires_maintainer_on_a_project_and_owner_on_a_group` |
| 404 not forgiven on delete | `delete_runner_deletes_by_id_and_counts_an_already_gone_runner_as_deleted` |
| `MissingScope` defaults to another scope | `error_statuses_map_to_gitlab_errors` |
| `per_page` dropped | the 3 list tests |
| `-1` accepted as `Retry-After` | `retry_after_is_honoured_only_as_whole_seconds` |
| path not `%2F`-encoded | `resolve_scope_looks_up_the_url_encoded_path_and_keeps_gitlabs_spelling` |
| `project_id` not taken from the caller | `job_reads_one_job_with_its_pipeline_tags_and_the_callers_project`, both `pending_jobs` tests |
| `bearer_auth` instead of `PRIVATE-TOKEN` | 7 tests |

  The first draft of the page-cap test **hung** when the cap was removed, because its mock paginated forever. I changed it so the mock stops at page 1001, and the no-cap mutation now fails in 0.36s.

### Pre-existing gate failure F will hit (not caused by #146)

`cargo clippy -p gridops-core --all-targets -- -D warnings` already fails on this branch: `used assert! to check that a value is empty` at `crates/gridops-core/src/autoscaling.rs:1615:9` and `:1701:9`. #146 does not touch that file (`git status` is clean there). F should report it rather than fix it, because the file is outside #146's ownership. Alternatively, O can rule on it.

## Ready for F

**RED is valid.** F may implement `crates/gridops-core/src/gitlab/http.rs` against these tests. That means adding the three mount lines above, `pub mod http;` and `pub use http::GitLabClient;` in `gitlab/mod.rs`, and nothing else outside `http.rs`.

Staged by explicit path:
- `crates/gridops-core/src/gitlab/http_tests.rs`
- `crates/gridops-core/tests/gitlab_live.rs`

Run with `cargo test -p gridops-core --lib gitlab::http`. The live test runs with `cargo test -p gridops-core --test gitlab_live -- --ignored`, given the environment variables in its module doc. I did not run it, because I have no GitLab token.
