# Handoff-T-green: Phase 7 - GitLab HTTP client implementing GitLabApi (#146)

**Date:** 2026-11-20
**Branch:** claude/gitlab-146-client-534ea2 (head eb7bae8)
**Issue:** #146
**Handoff-F reviewed:** docs/handoffs/146/handoff-F.md
**Handoff-T-red:** docs/handoffs/146/handoff-T-red.md

## GREEN confirmation

- `cargo test -p gridops-core --lib gitlab::http`: 18 passed, 0 failed.
- `cargo test -p gridops-core` (full): lib 279 passed; every integration binary ok (gitlab_contract 14 passed; gitlab_live 0 passed, 1 ignored; fleet_schema 14 passed). No failures.
- F's "Tests that look wrong" section: None. I changed no test.
- Spot-checks on the real `http.rs` (each applied alone, then restored byte for byte; `git status` clean afterwards, no production change left):
  - same-origin check disabled -> `the_token_is_never_sent_to_another_origin` FAILED (17 passed, 1 failed).
  - 404 no longer forgiven on delete -> `delete_runner_deletes_by_id_and_counts_an_already_gone_runner_as_deleted` FAILED.
  The tests pin behavior, not structure. The 17 further mutations in handoff-T-red.md were run against the draft and are not repeated.

## Tier 1 results

| Check | Command | Result |
|---|---|---|
| Unit suite | `cargo test -p gridops-core --lib gitlab::http` | PASS (18/18) |
| Full crate suite | `cargo test -p gridops-core` | PASS |
| Format | `cargo fmt --all --check` | PASS |
| Lint, lib | `cargo clippy -p gridops-core --lib -- -D warnings` | PASS |
| Lint, all targets | `cargo clippy -p gridops-core --all-targets -- -D warnings` | FAIL, pre-existing only: `autoscaling.rs:1615` and `:1701` (`assert!` on emptiness). Same two errors T reported in RED before F's change. Nothing in `gitlab/http.rs`, `http_tests.rs`, `gitlab_live.rs` or `mod.rs`. Outside #146's ownership; reported to O, not a blocker for F. |
| Workspace build | F reported `cargo check --workspace --all-targets` clean | Not re-run; the full crate test build compiled and the change adds only a `pub mod` and a `pub use` |
| Server/API smoke | N/A: library client, no server or endpoint added; the mock-server tests are the API smoke |  |
| Live test | `creates_then_deletes_a_project_runner_on_a_real_gitlab` | Not run: no `GITLAB_TEST_TOKEN` (same as F). Listed as ignored with a reason; compiles. |

## Tier 2 results

- **Coverage per criterion:** every brief clause has a test (see below). PASS.
- **Test quality:** 18 unit tests, each named for a behavior, at the cheapest tier that can observe wire behavior (in-process mock, no new dependency); one `#[ignore]` live test. No duplicates found; the one test that spans all methods (`every_method_maps_unauthorized_and_rate_limited_responses`) covers the shared seam without repeating per-method contract tests. Proportionate. PASS.
- **Type safety:** no `any`-equivalents, no `unwrap` or `expect` in `http.rs` outside tests; ids go through `ExternalId`. PASS.
- **Error handling:** 401, 403 (scope vs role), 404, 429 with `Retry-After`, 400/500/502, malformed 2xx, transport failure and a failing later page are all tested. PASS.
- **Data integrity:** a failed later page fails the whole list; the page cap is an error, not a silent truncation; delete is idempotent on 404 only. PASS.
- **API contract:** request shapes (path, `%2F` encoding, query, JSON body, `PRIVATE-TOKEN`, `User-Agent`) asserted on the wire. PASS.
- **Security (read of `http.rs`, for the points the tests cannot observe):**
  - `expose_secret()` is called once, in `send`, and the header is `set_sensitive(true)`. PASS.
  - No logging in `http.rs`. PASS.
  - No error carries a body, a `reqwest` message or a `serde` message; the 403 `scope` is sanitised to `[a-z_ ]{1,64}`. PASS.
  - Redirects are off (`Policy::none()`), and a `Link` to another origin is refused. PASS.
  - The 30s request timeout is read, not tested (a test would need a 30s stall). PASS.
  - `Retry-After`: the whitespace trim branch is unobservable over the wire because hyper strips it first. It is correct by reading (`.trim()` then all-digits check).
- **Migration safety:** no schema or dependency changes (`Cargo.toml` untouched). N/A.
- **Playwright:** not applicable (no UI surface, no Playwright in this crate).
- **Evidence appendix:** `evidence.md` covers the unchanged-code facts the tests rely on (`ExternalId` serde, `JobStatus::Unknown`, redacting `Debug`, `Upstream` Display). Nothing further needed.
- **F's results cross-checked:** F reported 18/18, 279 lib passed, fmt clean, lib clippy clean, all-targets clippy failing on the two pre-existing lines. My results match exactly.

## Acceptance criteria status

| Criterion | Test | Status |
|---|---|---|
| create_runner (project and group) | `create_runner_registers_a_project_runner_...`, `..._a_group_runner_...` | PASS |
| delete_runner idempotent on 404 only | `delete_runner_deletes_by_id_and_counts_...` | PASS |
| job and pending_jobs with caller's project | `job_reads_one_job_...`, `pending_jobs_follows_...` (x2) | PASS |
| group_projects with subgroups | `group_projects_lists_every_project_...` | PASS |
| pagination, failure and cap | `a_later_page_failing_...`, `a_server_paginating_past_the_page_cap_...` | PASS |
| resolve_scope with `%2F` and GitLab spelling | `resolve_scope_looks_up_the_url_encoded_path_...` | PASS |
| base URL path prefix | `the_api_root_keeps_a_base_url_path_prefix_...` | PASS |
| error mapping and Retry-After | `error_statuses_map_to_gitlab_errors`, `retry_after_...`, `every_method_maps_...` | PASS |
| verify_access scopes and roles | `verify_access_reports_...`, `verify_access_requires_...` | PASS |
| token never leaves origin | `the_token_is_never_sent_to_another_origin` | PASS |
| no secret in errors or Debug | `neither_token_appears_in_any_error_or_debug_output` | PASS |
| live create/delete on real GitLab | `creates_then_deletes_a_project_runner_on_a_real_gitlab` | NOT RUN (needs token; `#[ignore]`) |

## Blocking issues

None.

## Advisory notes

- `cargo clippy --all-targets -D warnings` fails on `autoscaling.rs:1615` and `:1701`, already failing before #146. O should rule on whether a separate issue fixes them.
- The live test is unrun; a maintainer with a token should run it once before the epic ships.
- `http.rs` is not wired into production (A-W1, epic-level; #138, #139 or #143).
- U is N/A (no UI surface); next is S.
