# Handoff-T-green: Phase 7 - GitLab platform-connection and pool-target API (#138)

**Date:** 2026-11-20
**Branch:** claude/gitlab-138-api-44d3a8 (on F's 282f386)
**Issue:** #138
**Handoff-F reviewed:** docs/handoffs/138/handoff-F.md
**Handoff-T-red:** docs/handoffs/138/handoff-T-red.md

## GREEN confirmation

Re-ran every command F reported; the results match F's.

```
cargo fmt --all -- --check                   -> clean
cargo test -p gridops-api --no-fail-fast
  bin (src/main.rs)             ok. 87 passed (now 91 with T's 4 added tests)
  control_plane_contracts       ok. 36 passed
  fleet_http                    143 passed; 1 failed
cargo test -p gridops-core --no-fail-fast --tests   -> all ok (lib 263; gitlab_pools 5)
cargo test -p gridops-reconciler                    -> ok. 68 passed
```

The one failure is `fleet_http::release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`. It is environmental: this machine is a ready macOS ARM64 machine, which that test asserts it is not. T did not touch that file and it failed identically at RED (before F's code). It is unrelated to #138. No discrepancy with F.

**Spot-check that tests fail when behavior is removed (production code mutated temporarily, then restored byte-for-byte; `git status` shows only the test file changed).** Dropping the `url.query().is_some()` refusal and widening the name limit to 81 made exactly the two new tests that pin them fail (28 passed, 2 failed). Restored, 30/30 pass.

## Test changes in this phase (T owns tests)

F flagged no wrong tests, but listed behaviors with no test. Added 4 tests to `crates/gridops-api/src/resources/gitlab_tests.rs`, each pinning one behavior in F's code:

- `a_base_url_with_credentials_a_query_or_a_fragment_is_refused_and_a_default_port_dropped`: userinfo, query, fragment are 400s that store nothing, never reach GitLab and don't echo the password; `:443` is dropped.
- `a_connection_name_longer_than_80_characters_is_refused_but_80_is_allowed`: boundary at 80.
- `an_empty_patch_returns_the_connection_unchanged_with_no_call_and_no_audit_event`.
- `a_gitlab_runner_delete_records_whether_the_gitlab_cleanup_completed_or_was_deferred`: `gitlabCleanup` is `complete` / `deferred` in the runner event.

These pass against F's code and fail under mutation (above for the first two). They were not RED-first because they pin behavior F already implemented that the RED suite did not cover. `cargo fmt` applied to the test file. No production code was changed.

## Tier 1 results

| Check | Result |
|---|---|
| fmt | PASS |
| clippy (`-p gridops-api -p gridops-core --all-targets`) | PASS, no warnings in any changed file |
| Build / test targets compile | PASS |
| Existing tests still pass | PASS (except the pre-existing environmental release_preflight) |
| API smoke | PASS: the binary-level `control_plane_contracts` tests exercise all six routes (401/403/200/404/409/204, CORS PATCH) against the spec |

## Tier 2 results

- **Coverage per criterion:** PASS, see below.
- **Test quality:** PASS. Each test names a behavior and sits at the cheapest tier (model unit tests in core, in-crate HTTP with FakeGitLab, 4 binary-level for route mounting and CORS). The 40-test RED suite is large but each maps to a distinct brief item; no merges needed.
- **Type safety:** PASS. No `any`-style casts; typed request structs; no `unsafe`.
- **Error handling:** PASS. Every `GitLabError` variant maps to a 400 naming the reason; `Upstream` detail is logged, not returned.
- **Data integrity:** PASS. Duplicate (409, incl. unique-index race), delete-while-targeted by pool / fleet target / live runner (409, in-transaction), SET NULL runner case.
- **API contract:** PASS. `check_response` validates bodies strictly against the OpenAPI spec for every route.
- **Security:** PASS. Admin-only; cookie-only routes (bearer refused, in `access.rs` and `token_access` tests); secrets shown once; no secret in other responses, audit events or plain-text rows.
- **Migration safety:** N/A, no migration (0047 already has the columns).
- **Playwright:** N/A, no UI surface.
- **Evidence appendix:** T's tests rely only on facts inside the diff (the normaliser and name rule are in changed code); F's evidence.md already covers the unchanged migration/helper facts. Nothing to add.

## Acceptance criteria status

- Create / duplicate / bad token / missing scope / missing role: PASS (`creating_a_connection_...`, `a_duplicate_...`, `gitlab_refusals_on_create_...`, `only_a_system_admin_...`)
- Update, verify, rotate: PASS (`patch_*`, `verify_answers_...`, `rotating_...`)
- Delete-while-targeted: PASS (pool, fleet target, live runner tests)
- Secret non-disclosure: PASS (`no_response_but_create_and_rotate_...`)
- `control_plane_contracts.rs` covers the new routes: PASS (4 tests)
- Responses validate against the spec: PASS (`check_response`)
- Pool create/update with GitLab targets and `run_untagged`, no mixing: PASS (`gitlab_pools.rs`, pool tests)
- Fleet authorization treats GitLab like Bitbucket: PASS (`service_gitlab_tests.rs`)
- `Arc<dyn GitLabApi>` injected via state: PASS (`with_gitlab` used by every in-crate test)

## Blocking issues

None.

## Advisory notes

- No test captures logs to prove no secret is logged (known issue in F's handoff). Reviewed by inspection: only `GitLabError::Upstream` detail and the D17 reason are logged; `GitLabCredentials` has a redacting Debug.
- `fleet_http::release_preflight` should be run on a non-macOS-ARM64 or CI machine to confirm it is green there.
