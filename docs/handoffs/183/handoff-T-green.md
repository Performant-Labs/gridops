# Handoff-T-green: Phase 7 - Retire a GitLab connection's fleet targets on removal (#183)

**Date:** 2026-11-22
**Branch:** claude/gitlab-183-retire-target
**Issue:** #183
**Handoff-F reviewed:** docs/handoffs/183/handoff-F.md
**Handoff-T-red:** docs/handoffs/183/handoff-T-red.md

## GREEN confirmation
Run (`CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`): `cargo test -p gridops-core -p gridops-api -p gridops-reconciler --no-fail-fast`, plus a re-run of `-p gridops-api --bin gridops-api` after the test repairs below.

- gridops-core: lib 311 pass; `fleet_schema` 17 pass; `gitlab_pool_fleet` 8 pass; every other binary passes.
- gridops-api: `control_plane_contracts` 39/39 (all four RED HTTP tests GREEN); unit binary 130/130 after the repairs; `fleet_http` 149 pass, 1 fail (`release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`, pre-existing, macOS-host-only, recorded in handoff-T-red, not touched by this diff).
- gridops-reconciler: 125/125.

**Spot-check (tests pin behaviour):** I temporarily removed the retired filter from `list_targets` and from `target_id_for_key` in `targets.rs`. `a_retired_gitlab_target_is_never_revived_offered_or_relinked` failed (7 pass, 1 fail). Production file restored byte-for-byte; `git status` shows only the two test files below as modified.

## Test repairs made in this phase (test-only; no production change)
1. `crates/gridops-api/src/resources/gitlab_tests.rs`: **deleted** `a_connection_a_fleet_target_references_is_refused_with_409_not_500` (the stale test F flagged). It pinned the refusal #183 removes. It is superseded by the extended `gitlab_connection_list_shows_seeded_connections_and_delete_answers_404_and_409` (never-used target, now 204) and test (a), which asserts the retired row. A rewrite would duplicate them. Removed the now-unused imports `CiTarget`, `GitlabConnectionId`.
2. `crates/gridops-api/src/fleet/openapi.rs` (`#[cfg(test)]` module): `the_gitlab_operations_are_specified_and_all_are_served` asserted the old 409 description text ("while a pool or a fleet target references it"). F's handoff did not flag this one; it failed in my full run. It now pins the new contract: the DELETE 409 description names a pool using the connection and live fleet work on its fleet targets.

## Tier 1 results
| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo +1.98.1 clippy -p gridops-core -p gridops-api -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` | PASS (no warnings, after my test edits) |
| Build of the three crates (via the test run) | PASS |
| Existing tests still pass | PASS, except the one pre-existing macOS-only failure above |
| API smoke | Covered by `control_plane_contracts` and `fleet_http`, which drive the real router and validate DELETE responses against the OpenAPI schema |
| Frontend (F reported build+tsc, eslint, vitest 624) | Not re-run: only a one-sentence copy string changed in `gitlab.tsx`; no behaviour. F's results accepted. |

Cross-check of F's reported numbers: core, reconciler, `control_plane_contracts` and `fleet_http` match F exactly. The unit binary differed: F reported one failure, I found two (the second is the openapi.rs test above, which F did not list). Both are now fixed.

## Tier 2 results
- **Coverage per criterion:** PASS (see below).
- **Test quality (test-quality.md §7):** PASS. Each test names one behaviour, sits at HTTP/core/schema tier with no duplicates; the superseded unit test was removed rather than rewritten. Suite is proportionate: 10 new/changed tests for one migration, one core function and one handler.
- **Type safety:** PASS. No `any`/unsafe casts introduced; the only TS change is a string.
- **Error handling:** PASS. 409 refusals (pool, runner, live work) leave all state unchanged (snapshot assertion); grant on a retired target answers 404; the trigger refuses un-retire, re-retire, and identity changes.
- **Data integrity:** PASS. Retire + revokes + events + connection delete happen in one `BEGIN IMMEDIATE`; history references keep their `target_id`; `canonical_key` stays UNIQUE and a retired key is never revived (`gitlab_connection_target` returns `None`, no insert).
- **API contract:** PASS. DELETE 204/404/409 validated against the OpenAPI schema; the spec text is updated and now pinned.
- **Security / auth:** PASS. Handler path unchanged (admin only); no new route (decision §4).
- **Migration safety:** PASS. 0048 rebuild test proves records, FKs, schema objects, STRICT, trigger name and `foreign_key_check` are preserved; the FK guard is tested. Retirement is intentionally irreversible (trigger), as the brief requires; the migration adds a nullable column, matching 0047 style.
- **Playwright:** the project has no Playwright suite in this change's scope; N/A.
- **Evidence appendix:** reviewed `docs/handoffs/183/evidence.md` (86 lines); the facts my tests rely on in unchanged code (NO ACTION FK, CHECK arms, trigger, `registry/delete.rs` revision bump) are listed there. No additions needed.

## Acceptance criteria status
- (a) Issue's case (prod shape): PASS, `removing_a_gitlab_connection_retires_its_fleet_target_and_revokes_its_grant`.
- (b) History case: PASS, `removing_a_gitlab_connection_keeps_its_fleet_history_and_revokes_its_platform_link`.
- Never-used (list-created) target deletes: PASS, `gitlab_connection_list_shows_seeded_connections_and_delete_answers_404_and_409`.
- Live placement / queued intent / unreleased allocation refused with 409 naming the pool, no change; admitted intent is history: PASS, `a_gitlab_connection_is_not_removed_while_anything_still_runs_through_it`.
- Retired targets invisible and never revived/relinked/admitted: PASS, `a_retired_gitlab_target_is_never_revived_offered_or_relinked`, `a_retired_gitlab_target_is_not_valid_for_its_pool_and_is_never_linked`.
- Schema/trigger/retire-is-final: PASS, the four `fleet_schema` tests.
- Copy (OpenAPI, confirm text, docs, CHANGELOG): not behaviour-tested by design; OpenAPI text now pinned by the repaired unit test; remaining copy left to S.

## Blocking issues
None.

## Advisory notes
- The 0048 trigger also permits retiring a non-GitLab row (F's documented hedge). Harmless today; worth a kind clause if GitHub/Bitbucket retirement is ever added.
- `release_preflight` fails on macOS hosts independent of this change.
- Not hit by a UI surface beyond a copy sentence: U is N/A; hand to S.
