# Handoff-S: Phase 10 - GitLab HTTP client implementing GitLabApi (#146)

**Date:** 2026-11-20
**Branch:** claude/gitlab-146-client-534ea2 (head 8b0cad1)
**Issue:** #146
**Spec:** `docs/handoffs/146-brief.md` (the brief wins over the issue text where they differ), with the #137 contract in `crates/gridops-core/src/gitlab/mod.rs`
**Handoff-T reviewed:** `docs/handoffs/146/handoff-T-green.md` (and `handoff-T-red.md`)
**Handoff-A reviewed:** `docs/handoffs/146/handoff-A.md` (up-front plan review). The anti-duplication gate's own output is `docs/handoffs/146-diff-result-r1.md` (commit `8b0cad1`, "a-dup -- verdict: PASS"). There is no `handoff-A-dup.md` in this directory, so I relied on that result file.
**Handoff-F reviewed:** `docs/handoffs/146/handoff-F.md`
**Operator-facing report:** N/A (no visual surface, so no HTML report is required)

## A precondition

Confirmed. `handoff-A.md` is **PASS** (0 block, 5 warn). The anti-duplication gate result (`146-diff-result-r1.md`) is **PASS** (0 block). Its warnings are dispositioned under "Scope check" and "Advisory notes".

## T precondition

Confirmed. `handoff-T-green.md` reports "Blocking issues: None": 18/18 unit tests green, full `cargo test -p gridops-core` green, and two mutations against the real `http.rs` each turned one test red.

## Visual-diff-tool precondition

N/A. #146 is a library HTTP client with no rendered surface, no Playwright suite and no wireframe. The browser-tool and visual-diff preconditions do not apply.

## Preview / spec sanity check

I checked the brief (the source of truth) against GitLab's own current REST docs and source (fetched 2026-11-20 from `gitlab-org/gitlab` master). No defect that justifies `ADVISORY-HOLD`; one inconsistency worth knowing about:

| Brief claim | Check | Result |
|---|---|---|
| `POST /user/runners` body fields `runner_type`, `project_id`/`group_id`, `tag_list`, `run_untagged`, `description` | `doc/api/users.md` | Matches. The docs table types `tag_list` as "string (comma-separated)", but `lib/api/user_runners.rb:51` declares `Array[String]` with `CommaSeparatedToArray`, which accepts a JSON array or a comma string. The array F sends is valid. |
| Job JSON carries `pipeline.id` and no top-level `project_id` | `doc/api/jobs.md` | Matches (example has `"pipeline": {"id": ...}`). Settles diff-gate NV-1. |
| `scope[]=pending` | `doc/api/jobs.md` | Matches. |
| `with_projects=false` on `GET /groups/:id` | `doc/api/groups.md:44` | Documented and honoured in v4 (deprecated, "scheduled for removal in API v5"). Settles diff-gate NV-3. |
| Group runner needs Owner | `users.md`, `runners_scope.md` | Matches. |
| **Project runner needs only Maintainer** (the brief's refinement of "below Owner") | `runners_scope.md:450` says "Maintainer role for the project"; but `users.md` "Create a runner linked to a user" says "Owner role for the target namespace or project" | **GitLab's own pages disagree.** The brief follows the UI page. Not a defect in the brief and not worth a hold: if the API page is right, a project Maintainer passes `verify_access` and `create_runner` then returns 403, which maps to `MissingRole`. The failure is late, never silent. See Advisory note 1. |

## Spec compliance

Checked by reading `http.rs` (574 lines), `http_tests.rs` (1304 lines), `tests/gitlab_live.rs` (66 lines) and the 3-line `mod.rs` diff against every clause of the brief.

| Requirement (brief) | Where | Compliant |
|---|---|---|
| Implement `GitLabApi` as `GitLabClient` in `gitlab/http.rs` | `http.rs:137` `impl GitLabApi for GitLabClient` | YES |
| D2: every method takes `credentials` first; base URL per call; `new()` builds the HTTP client only | `http.rs:56-66`; `api_url(&credentials.base_url, ...)` in every method | YES |
| A-W1: `#[derive(Clone)]` struct, `new() -> anyhow::Result<Self>`, `pub use http::GitLabClient;` in `gitlab/mod.rs` only | `http.rs:51-66`; `mod.rs:20-22` | YES |
| `create_runner` → `POST /user/runners`, `runner_type` project_type/group_type + the matching id, `tag_list`, `run_untagged`, `description`; response → `CreatedRunner { id, token: SecretString }` | `http.rs:139-166` | YES |
| `delete_runner` → `DELETE /runners/:id`, 404 = `Ok(())`, 404 checked before error mapping (A-W2) | `http.rs:168-184` | YES (403 still `MissingRole`; only 404 forgiven) |
| `job` → `GET /projects/:id/jobs/:job_id`, `pipeline_id` from `pipeline.id`, caller's `project_id`, `tag_list`; unknown status → `JobStatus::Unknown` | `http.rs:186-203`, `JobResponse::into_job` | YES |
| `pending_jobs` → `GET /projects/:id/jobs?scope[]=pending`, `per_page=100`, all pages | `http.rs:205-222` | YES |
| D4 `group_projects` → `GET /groups/:id/projects?include_subgroups=true`, paginated, every id | `http.rs:224-238` | YES |
| D3 `resolve_scope`: `GET /projects/:path` / `/groups/:path`, path as one segment (`%2F`), `path_with_namespace` / `full_path` | `http.rs:240-267`, `api_url` (url crate `path_segments_mut().extend`) | YES (group lookup adds `with_projects=false`; see Scope check) |
| API root = `base_url` + `api/v4/`, path prefix kept, no doubled slash | `http.rs:405-415` | YES |
| Auth: `PRIVATE-TOKEN`, `set_sensitive(true)`, token never in a URL | `http.rs:78-85` (the only `expose_secret()`) | YES |
| `user_agent("GridOps/0.1")`, `redirect(Policy::none())`, request timeout | `http.rs:58-64`; `CONNECT_TIMEOUT` 10s, `REQUEST_TIMEOUT` 30s (A-W3) | YES |
| Pagination: `Link rel="next"` first, else `x-next-page` (empty/absent = last) | `http.rs:505-536`, `link_next` | YES |
| `Link` followed only on the same scheme, host and port as `base_url`, else `Upstream` | `http.rs:566-570`, used at `:515` | YES |
| Hard page cap → `Upstream` | `MAX_PAGES = 100` (`http.rs:39`, loop `:118`, error `:131`) | YES (100, not the brief's "e.g. 1000"; A-W3 pinned 100; see Scope check) |
| 401 → `Unauthorized` | `http.rs:443` | YES |
| 403 `insufficient_scope` → `MissingScope(scope or "api")`; other 403 → `MissingRole` | `http.rs:444-447`, `missing_scope` | YES (the scope text is sanitised to `[a-z_ ]{1,64}`, else `api`; disclosed by F) |
| 404 → `NotFound` (delete: `Ok`) | `http.rs:448`, `:180` | YES |
| 429 → `RateLimited { retry_after }` by the B-6 rule | `http.rs:449-451`, `retry_after` (`:482`) | YES (trim, all-ASCII-digits, `u64`; `-1`, `+30`, HTTP-date, absent → `None`) |
| Other non-2xx, transport and decode errors → `Upstream`, fixed text, no body | `http.rs:452`, `transport_error`, `decode` | YES |
| `verify_access`: scopes (`create_runner` then `read_api`; `api` satisfies both), then `/user` (admin passes), then membership; Maintainer (40) project / Owner (50) group; 404 → `MissingRole` | `http.rs:269-320` | YES (documented deviation from the issue's "below Owner") |
| No error string, log line or `Debug` holds a response body or the access token | no logging in `http.rs` (grep: no `tracing`/`log`/`println`/`dbg`/`unwrap`/`expect`); response structs derive `Deserialize` only | YES |
| Tests: mock server on tokio `TcpListener`, records method/path+query/headers/body | `http_tests.rs` `Mock` | YES |
| Tests cover every method, each error mapping, pagination (both forms), cross-origin `Link`, path prefix, Retry-After cases, verify_access matrix, token-never-leaks | 18 tests; see Test quality | YES |
| One `#[ignore]` live test: `GITLAB_TEST_TOKEN`, `GITLAB_TEST_PROJECT`, optional `GITLAB_TEST_URL` (default gitlab.com); resolve, create, delete | `tests/gitlab_live.rs:25-66` | YES (exists and compiles; **not run**, no token) |
| Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p gridops-core` | see Quality audit | YES under CI's pinned toolchain (see below) |
| No contract-type, trait, `GitLabError` or `fake.rs` change; no `Cargo.toml` change; only `pub mod http;` (+ `pub use`) in `mod.rs` | `git diff cdda02e HEAD --stat`: 4 code files (`http.rs`, `http_tests.rs`, `gitlab_live.rs` new; `mod.rs` +3) | YES |

**Clippy gate, resolved.** T and F both reported `cargo clippy --all-targets -D warnings` failing on `autoscaling.rs:1615` and `:1701` and left it for O. I ran it under the toolchain CI pins (`.github/workflows/ci.yml:88-101`: 1.98.1, with the comment that a newer clippy "brings new lints that fail `-D warnings`"):

    $ cargo +1.98.1 clippy -p gridops-core --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl
    Finished `dev` profile ... (no warnings, no errors)
    $ cargo +1.98.1 fmt --all -- --check
    (clean)

The two failures come from the local default toolchain (1.99.1) being newer than CI's. They are not a repo defect and not a #146 defect. The gate is green as CI runs it.

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | Each method uses the endpoint, query and body the brief pins. The doc checks above confirm the endpoints, the `scope[]` filter, the nested `pipeline.id` and the array `tag_list`. |
| Error handling | PASS | One seam (`send` raw / `success` / `error_for` / `decode`). A failing later page fails the whole list. The page cap is an error, not a truncation. Delete is idempotent on 404 only. |
| UI/UX match to spec | N/A | No UI. |
| Accessibility | N/A | No UI. |
| Architecture gate | PASS | A: PASS on the plan; the anti-duplication gate: PASS. No quality finding here conflicts with either. |
| Code organization | PASS | Constants at the top, one private seam, response structs separate from the contract types, no dead code, no TODO, no commented-out code. Tests are in a sibling `http_tests.rs`, as `fake.rs` does. |
| Security | PASS | `expose_secret()` is called once, in `send`, and the header is `set_sensitive(true)`. Redirects are off, a cross-origin `Link` is refused, no error carries a body, a `reqwest` message or a `serde` message, and the 403 `scope` text is sanitised before it is repeated. Both secrets (access token, `glrt-` runner token) are covered by the leak test. Residual notes are in the Advisory notes. |
| Performance | PASS | `per_page=100`, one request per page, 10s connect / 30s request timeouts, 100-page cap. `with_projects=false` avoids embedding projects in the group lookup. |
| Visual regression | N/A | No UI, no Playwright suite. |
| Naming consistency | PASS | `GitLabClient`, `GitLabApi`, `GitLabError`, `ScopeKind` and `Scope` as the contract names them; GitLab's own field names in the response structs. |
| Test quality (`testing/test-quality.md` §7) | PASS | See below. No "delete or merge" finding that blocks. |

### Test-quality audit (rubric §7 over the 18 unit tests and the live test)

- **Per test.** Each of the 18 names one behaviour and asserts the observable contract (what goes over the wire and what the trait returns), not private structure. No test names `MAX_PAGES`, the timeouts, `send` or `error_for`. T's mutation runs (17 on a draft, 2 on the real `http.rs`) show that each pinned behaviour turns a named test red and that none hangs. Tier is correct: the client's whole contract is HTTP, so an in-process mock is the cheapest tier that can observe it, with no new dependency.
- **Per suite.** 18 tests for a new seven-method client plus its security properties is proportionate to a feature story, and no test duplicates another per-method contract test. `every_method_maps_unauthorized_and_rate_limited_responses` fans 401/429 over all seven methods; I keep it because the brief's error-mapping clause says "every method" and `delete_runner` takes a different path (`error_for` directly, not `success`), which is exactly where Bitbucket's neighbour had its bug.
- **Smells.** No assertion-free, tautological, mock-shaped, snapshot or coverage-padding test. Three small items, none blocking:
  1. The `" 30 "` row of `retry_after_is_honoured_only_as_whole_seconds` cannot see the `.trim()`: hyper strips the whitespace before the client reads the header (T says the same). The row stays as the brief's required case; the trim is correct by reading (`http.rs:483`). A direct test of the private helper would pin behaviour no wire can produce, which is the §2 smell, so I am not asking for one.
  2. In `neither_token_appears_in_any_error_or_debug_output` the statuses 400, 409, 500 and 503 all take the same `_ =>` arm of `error_for`. It is a loop inside one test that runs in milliseconds, it guards a security invariant, and trimming it costs more than it saves. I am not asking for it.
  3. The closed-port transport case binds a listener and drops it; another parallel test's mock could in theory take the port before the connect. The window is a few microseconds across roughly 28k ephemeral ports. I treat it as negligible.

## Scope check

F delivered the brief's scope. Over- and under-delivery, item by item:

- **Under-delivery: none.** Every clause in the Spec compliance table is met. The one thing not exercised is the live test, which needs a real token.
- **`MAX_PAGES = 100` against the brief's "e.g. 1000".** The brief marks 1000 as an example; A's plan review (A-W3) pinned 100 "matching GitHub, unless the brief records a reason for 1000", and the brief records none. It is a ceiling of 10,000 items per list that errors rather than truncates. This resolves diff-gate W-1: not a deviation, but listed in the Advisory notes.
- **`tests/gitlab_live.rs` outside `http.rs`.** Diff-gate W-2 flagged it. A's plan review (A-W4) named exactly this file, next to `gitlab_contract.rs`, and noted that a new file collides with no wave-1 issue. Not a deviation. `origin/master` has not moved under `crates/gridops-core/src/gitlab/` or `crates/gridops-core/tests/`, so no merge collision either.
- **`verify_access` 403 from the membership endpoint → `MissingRole`.** Diff-gate W-4. Step 1 already proved the token's scopes, and `error_for` checks the body for `insufficient_scope` first, so a scope problem would still surface as `MissingScope`. Acceptable.
- **Unrequested additions (small, disclosed by F):** `with_projects=false` on the group lookup (documented in v4, path unchanged), and the `MissingScope` text sanitiser (extends the brief's "no body in any error" rule to that field). The live test also calls `verify_access` and deletes twice, which is within the spirit of "create, then delete".
- **Contract and ownership:** no contract type, trait, `GitLabError`, `fake.rs` or `Cargo.toml` change; `mod.rs` carries exactly `pub mod http;` and `pub use http::GitLabClient;`.

## Verdict

PASS. All acceptance criteria that can be checked offline are met, the implementation is spec-compliant (including the brief's documented `verify_access` role refinement), the tests are good and proportionate, and the clippy gate is green under CI's pinned toolchain. Ready for O to commit.

## Advisory notes

None of these blocks the PR. They are recorded so O can decide where to file them.

1. **Project-runner role: GitLab's docs disagree.** `doc/ci/runners/runners_scope.md` says a project runner needs Maintainer; `doc/api/users.md` ("Create a runner linked to a user") says Owner "for the target namespace or project". The brief follows the first. If the second is right, a project Maintainer passes `verify_access` and gets `MissingRole` from `create_runner` instead; that fails safe. The by-hand `#[ignore]` run settles it: run it once with a **Maintainer-but-not-Owner** token on a project, and record the answer on #136.
2. **The live test has never run.** No `GITLAB_TEST_TOKEN` was available to F or T, or to me. Until someone runs it, the wire behaviour against a real GitLab (including the array `tag_list`, confirmed only from GitLab's source) is verified by docs and the mock server, not by a live call.
3. **Granular-scope tokens (GitLab 19.2+).** `GET /personal_access_tokens/self` can return `granular_scopes`; the docs example shows `scopes` still present, but the note says granular tokens are "instead of" the standard attribute. `TokenResponse.scopes` is required, so if a granular token omits it the result is a generic `Upstream`, not `MissingScope`. It fails closed (never a false pass). Worth a follow-up if granular tokens are expected.
4. **`MissingScope` text excludes digits.** A real scope such as `k8s_proxy` would be reported as `api`. GridOps only needs `api`, `read_api` and `create_runner`, so it has no effect today.
5. **No response-size cap.** `decode` and the 403 read take the whole body, bounded only by the 30s request timeout. `base_url` is operator-supplied, so the exposure is low; a cap would be cheap hardening if the base URL ever becomes less trusted.
6. **`MAX_PAGES = 100`** caps a list at 10,000 items (see Scope check). Raise it if a group with more than 10,000 projects matters.
7. **Epic-level items, unchanged from A's plan review.** Nothing constructs `GitLabClient` in production yet (`gridops-api` and `gridops-reconciler` `main.rs`), and no `base_url` scheme policy exists in `http.rs` (the tests need loopback `http`). O should record the wiring owner (#138, #139 or #143) and the scheme-policy owner (#138's `base_url` normalisation) on #136.
8. **`handoff-A-dup.md` is missing** from `docs/handoffs/146/`; the anti-duplication gate's result is only in `docs/handoffs/146-diff-result-r1.md`. The verdict is clear (PASS), but the file is not where the phase table says it should be.
