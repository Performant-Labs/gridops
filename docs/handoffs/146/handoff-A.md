# Handoff-A: Phase 3 - GitLab HTTP client implementing GitLabApi (#146)  (up-front plan review)

**Date:** 2026-11-20
**Branch:** claude/gitlab-146-client-534ea2 (head cdda02e)
**Brief reviewed:** docs/handoffs/146-brief.md   **Reuse map:** N/A. No `survey.md` was provided, so the brief's own analogues are the baseline: `BitbucketClient` (`crates/gridops-core/src/bitbucket.rs`), `GitHubClient` (`crates/gridops-core/src/github.rs`), and the #137 contract and fake (`crates/gridops-core/src/gitlab/{mod.rs,fake.rs,fake_tests.rs}`).   **Wireframe:** N/A (no UI surface)
**Verdict:** PASS

## Summary

PASS. The plan extends the right object in the right place. It implements the #137 `GitLabApi` trait in `gitlab/http.rs`, next to `fake.rs`. That is the file the contract's doc comment and the epic's ownership table name. It needs no new dependency and adds only the one allowed line in `mod.rs`.

The plan departs from its neighbours in three places, and the brief justifies each one in writing:

- **Typed `GitLabError` instead of `anyhow`.** This is the #137 contract.
- **No response body in errors.** Bitbucket and GitHub echo up to 500 characters of the body. GitLab can't, because the `create_runner` body holds the `glrt-` token.
- **Per-call base URL instead of `with_base_url`.** This is D2.

None of the three is drift. The five warnings below pin choices the brief leaves open. Without them, T and F could resolve those choices differently, and one could also copy a latent Bitbucket bug.

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | "`GitLabClient::new()` builds the HTTP client only" and "plus a `pub use http::GitLabClient;` if wanted" | pattern consistency, naming; cross-cutting (production wiring) | **The constructor's signature is unpinned.** Both neighbours are `#[derive(Clone)]` structs whose `new()` returns `anyhow::Result<Self>`, and their callers construct them as `X::new()?`:<br>- `gridops-api/src/main.rs:82-83`<br>- `gridops-reconciler/src/main.rs:367-368`<br>**No wave-1 issue owns the production wiring.** Someone has to write `Arc::new(GitLabClient::new()?)` in those two `main.rs` files. R2-W3 of the #137 brief gives #138 the `AppState.gitlab` field "and its constructor argument" and gives #139 the `Reconciler` field. `FakeGitLab` is built only under `cfg(test)` or the `test-support` feature, so a production `main.rs` can't be built with it. That leaves the real client unwired, and #146 doesn't own `main.rs`. | **Pin the shape:**<br>- `#[derive(Clone)] pub struct GitLabClient { http: reqwest::Client }`<br>- `pub fn new() -> anyhow::Result<Self>`<br>- `pub use http::GitLabClient;` in `gitlab/mod.rs`. Don't add it to `lib.rs`: `lib.rs` is shared, and #137 left the gitlab types at the module path `gridops_core::gitlab::*`.<br>**Record a wiring owner on #136.** Suggested: whichever of #138 or #139 lands after #146 swaps in the real client, or else #143. |
| 2 | warn | "Error mapping (every method)" and "`delete_runner` 404 → `Ok(())`" | pattern consistency, abstraction level (the request and error seam) | **The brief defines one mapping for every method but doesn't name the seam that applies it.**<br>**Bitbucket's shape has a latent bug.** `BitbucketClient::send` bails on every non-2xx status (`bitbucket.rs:225-233`). Its `delete_runner` checks `NOT_FOUND` only after calling `send` (`:175-185`), so that check can never run. A port of Bitbucket's shape would make GitLab's "404 counts as success" rule unreachable.<br>**GitHub's shape is correct.** `try_delete` (`github.rs:183-190`) checks the status before mapping.<br>**Neither neighbour should be copied on body handling.** Both copy the response body into the error (`bitbucket.rs:228-233`, `github.rs:86-91`), which the brief forbids for GitLab. | **One private seam in `http.rs`:**<br>- `send(credentials, method, url, body) -> Result<reqwest::Response, GitLabError>`. It returns the raw response for any status and maps only transport errors to `Upstream`. It is the only place that calls `expose_secret()`, to set the auth header.<br>- `fn error_for(status, headers, body_bytes) -> GitLabError`, used by every method. It reads the 403 body for `insufficient_scope` and never puts the body into the error.<br>- `get_json<T>`.<br>- `get_all_pages<T>`, shared by `pending_jobs` and `group_projects`.<br>**`delete_runner`** checks for 404 before calling `error_for`, as GitHub's `try_delete` does.<br>**The auth header.** `PRIVATE-TOKEN` with `set_sensitive(true)` is acceptable as briefed. The neighbours use `bearer_auth`, which GitLab also accepts and which reqwest marks sensitive itself. No change is needed. |
| 3 | warn | "A request timeout is set" and "A hard page cap (e.g. 1000 pages)" | pattern consistency (module constants) | **Neither value is pinned.** T has to test against both (the brief review's W-1 and W-5 make the same point).<br>**Timeouts.** The repo precedent is a connect timeout plus a request timeout: 10s and 20s in `gridops-agent/src/provider_auth.rs:334-335`, 10s and 30s in `gridops-host-agent/src/client.rs:73-74`.<br>**Page caps.** GitHub caps its page loops at 100 pages (`github.rs:308,350`) and stops silently at the cap. The brief's choice to return an `Upstream` error at the cap is a justified divergence. | **Pin module-top `const`s, in the style of Bitbucket's `API_BASE_URL`:**<br>- `CONNECT_TIMEOUT = 10s`<br>- `REQUEST_TIMEOUT = 30s`<br>- `MAX_PAGES` (100, matching GitHub, unless the brief records a reason for 1000).<br>**Don't add a test-only constructor or field just to shrink the cap.** A looping mock server can serve `MAX_PAGES` local responses cheaply. Only if that proves slow, put the limit behind `#[cfg(test)]`. |
| 4 | warn | Tests: "In `http.rs` (`#[cfg(test)] mod tests`) or a sibling test module file under `src/gitlab/`", and the `#[ignore]` live test | naming, file structure | **The brief leaves the test location open**, and this module will hold a hand-written mock server plus about 30 cases.<br>**The local pattern is a sibling `<module>_tests.rs` file** mounted with `#[cfg(test)] #[path = "..."] mod tests;`. Examples: `gitlab/fake.rs:301-303` mounts `fake_tests.rs`, and `fleet/admission.rs:1504` mounts `admission_tests.rs`.<br>**Hand-writing the server has precedent.** `gridops-manager/src/stand_in_docker.rs` writes HTTP/1.1 responses by hand. The workspace's usual mock is axum, but axum isn't a `gridops-core` dependency, so the brief's choice is correct.<br>**Live tests have their own precedent.** `crates/gridops-agent/tests/sandbox_live.rs` uses `#[ignore = "<reason>"]` and a module doc that gives the env vars and the exact `cargo test … -- --ignored` command.<br>**New files don't collide with other issues.** "Owns `http.rs` only" doesn't conflict with new files, because no other wave-1 issue owns them. | **Unit tests and mock server.** Pin `crates/gridops-core/src/gitlab/http_tests.rs`, mounted from `http.rs` as `fake.rs` mounts its tests. Bound the mock server: one request per connection, `Connection: close`, `Content-Length` only, no chunked encoding (the brief review's W-2).<br>**The live test.** Put it at `crates/gridops-core/tests/gitlab_live.rs`, next to `gitlab_contract.rs`, or in `http_tests.rs`. Either way, use the reasoned `#[ignore = "needs GITLAB_TEST_TOKEN and network access"]` form and document the env vars and the run command in the module doc. |
| 5 | warn | "API root: `credentials.base_url` joined with `api/v4/`" (no scheme policy) | cross-cutting (where input is validated) | **Each call sends a token-bearing request to a base URL the operator supplies.** No issue states a scheme policy. #137's schema says `base_url` is "normalised by #138: scheme+host[+port][+path], no trailing slash" (137-brief:453).<br>**The in-repo precedent for a configurable token-bearing URL** is the host agent's `ClientError::InsecureUrl`: `https`, or `http` only to a loopback address (`gridops-host-agent/src/client.rs:70-71`). | **No change to #146 is needed.** The natural seam is #138's normalisation on create and update.<br>**Ask O to confirm on #136 that #138 enforces `https` (with loopback `http` allowed).**<br>**If #146 adds a defensive check anyway,** it must keep the loopback `http` exception, or every mock-server test against `http://127.0.0.1` breaks. |

## Notes for O

None required: the verdict is PASS. Before T starts, add one line each to the brief for:

- Finding 1: the constructor's signature and the `pub use` line.
- Finding 3: the constant values.
- Finding 4: the test file names.

Each of these is a choice T and F would otherwise make separately.

Two items belong on epic #136, not in this brief, because #146 owns neither file:

- Finding 1: who wires `GitLabClient` into the two `main.rs` files.
- Finding 5: who enforces the `base_url` scheme.

## Patterns referenced

- `crates/gridops-core/src/bitbucket.rs:64-83` (struct and constructor), `:165-234` (`delete_runner`, `request` and `send`, including the unreachable 404 branch)
- `crates/gridops-core/src/github.rs:36-95` (client builder, `send`), `:141-190` (`try_delete`, 404 before mapping), `:302-365` (page loops capped at 100)
- `crates/gridops-core/src/gitlab/mod.rs` (the contract and its trait), `gitlab/fake.rs:301-303` and `gitlab/fake_tests.rs:1-3` (sibling test-module pattern)
- `crates/gridops-api/src/main.rs:82-83` and `crates/gridops-reconciler/src/main.rs:367-368` (how clients are constructed), and `docs/handoffs/137-brief.md` R2-W3 (field ownership)
- `crates/gridops-host-agent/src/client.rs:65-78` (timeouts, `InsecureUrl`), `crates/gridops-manager/src/stand_in_docker.rs` (hand-written HTTP/1.1 mock) and `crates/gridops-agent/tests/sandbox_live.rs` (the ignored live-test convention)
