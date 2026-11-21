# Handoff-T-green: Phase 7 - GitLab job webhook ingestion and busy tracking (#140)

**Date:** 2026-11-20
**Branch:** claude/gitlab-140-webhooks-a9fae5 (head `9c73436`)
**Issue:** #140
**Handoff-F reviewed:** `docs/handoffs/140/handoff-F.md`
**Handoff-T-red:** `docs/handoffs/140/handoff-T-red.md`

## GREEN confirmation

All commands ran with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`.

- `cargo test -p gridops-api --bin gridops-api` -> `121 passed; 0 failed` (was 90 passed / 31 failed at RED). This includes the 30 webhook tests, the renamed OpenAPI test and the access-matrix test.
- `cargo test -p gridops-core --lib` -> `291 passed; 0 failed`, including the 10 D18 `jobs_tests`.

**Mutation spot-check:** in `webhooks/gitlab.rs:367` the clear arm was disabled (`if status.is_terminal()` -> `if false`). Four tests failed (`running_marks_the_runner_busy_and_a_conclusion_clears_it_on_that_runner_only`, `after_a_conclusion_no_later_event_...`, `a_stale_conclusion_...`, `a_failure_inside_the_write_rolls_it_all_back_...`). The file was restored with `git checkout`; `git status` is clean and the 60 `gitlab_tests` pass again. The tests pin behaviour, not implementation.

## Tier 1 results

| Check | Result |
|---|---|
| `cargo build` (via test builds) | PASS |
| `cargo test -p gridops-api --test control_plane_contracts` | PASS, 36 passed |
| `cargo test -p gridops-api --test fleet_http` | 143 passed, 1 failed: `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`. Matches F's report. Host-OS dependent (the fixture's "Linux" machine is the local host agent, which reports `cfg!(target_os)`, so on this Mac it has a macOS ARM64 machine). The diff touches no machine, host-agent or preflight code (`git diff --stat` over `crates`: only webhooks, main.rs, resources, openapi, gitlab/jobs, gitlab/mod). Not caused by #140; expected to pass on Linux CI. Not blocking. |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy -p gridops-api -p gridops-core --all-targets` | PASS for touched files: warnings exist only in `autoscaling.rs`, `overview_queue.rs`, `fleet/boundary.rs`, `fleet/enrollment.rs`, `route_security.rs`, `fleet_http/*`, `gridops-host-agent`; none in `webhooks*`, `gitlab/*`, `openapi.rs`, `main.rs`. |
| API smoke | Covered by the in-crate HTTP tests through `public_routes()` (202/400/401 checked against `docs/openapi/gridops-v1.json`). No server start needed. |

## Tier 2 results

- **Coverage per criterion:** PASS (table below).
- **Test quality:** PASS. Each of the 30 tests names a behaviour and goes through the router, so none binds F's internals. Tests 21, 23, 24, 25 are distinct state cases of the round-4 busy rules, not duplicates. The D18 suite is byte-identical to #148's and is not duplicated. Proportionate to a 1,661-line test file for one handler plus a retry arm: acceptable, no deletions requested.
- **Type safety:** PASS. Rust; no `unsafe`, no new `unwrap` on request paths in `webhooks/gitlab.rs` (clippy clean on the file).
- **Error handling:** PASS. 400 (non-object body), 401 (bad, missing, unknown-project token), duplicate 202, a failed write leaves a `failed` delivery and rolls back (test 30).
- **Data integrity:** PASS. Writes run in one `BEGIN IMMEDIATE` transaction, with rollback tested via an abort trigger; retry applies it. Duplicate delivery ids are acknowledged without reprocessing.
- **API contract:** PASS. The 202 has no body, as the spec declares; responses validated by `check_response`. The spec sentence "Not served yet" is removed; `docs/fleet-api.md` row added.
- **Security:** PASS on tests; the constant-time compare, the delivery-id header-order comment and the no-token-in-logs fact are not testable here and are left to S, as F and T-red already noted. Token compare reviewed in `matching_connection` (compares every candidate, no early exit).
- **Migration safety:** N/A, no schema change.
- **Playwright:** N/A, no UI surface and the project has no Playwright run for this change.
- **Evidence appendix:** `docs/handoffs/140/evidence.md` already lists the unchanged-source facts the tests rely on (retry refusal for unverified rows, NULL installation requires admin, credentials helper, `runtime_secret`, `ApiError::Unauthorized`). No additions needed.
- **F's results cross-checked:** every number F reported reproduced exactly.

## Acceptance criteria status

| Criterion | Status | Backing test |
|---|---|---|
| Pending job looked up for tags and recorded | PASS | 3, 17, 18 |
| Bad token / unknown project | PASS | 1, 2, 7, 9 |
| Duplicate delivery | PASS | 4, 5 |
| Retried job is a new row | PASS | 10 |
| Busy set and cleared on the right runner | PASS | 19-27 |
| Status ordering (tier, time, terminal final) | PASS | 11-15, 26, 27 |
| Non-build / malformed events | PASS | 6, 15, 16 |
| Retry through the GitLab arm; partial failure recoverable | PASS | 28-30 |
| OpenAPI no longer "Not served yet" | PASS | 31 |
| D18 writer rules | PASS | 32-41 |

## Tests changed in this phase

None. F reported no wrong tests and none were found.

## Blocking issues

None.

## Advisory notes

- `fleet_http::release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on a macOS ARM64 dev box regardless of #140; confirm on Linux CI.
- Master has moved (#160, #161). F reports `jobs.rs`, `jobs_tests.rs` and `gitlab/mod.rs` are byte-identical to master, so the merge should be clean.
- No UI surface: U is N/A, ready for S.
