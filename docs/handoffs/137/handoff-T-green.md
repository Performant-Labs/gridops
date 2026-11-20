# Handoff-T-green: Phase 7 - GitLab shared contract (#137)

**Date:** 2026-11-20
**Branch:** claude/gitlab-137-contract-fa3bbe
**Issue:** #137
**Handoff-F reviewed:** docs/handoffs/137/handoff-F.md
**Handoff-T-red:** docs/handoffs/137/handoff-T-red.md

## GREEN confirmation

Test repaired (test-only): `crates/gridops-core/src/db.rs` `db::tests::fresh_database_applies_every_migration` asserted 70 tables. 0046 adds `gitlab_connections`, `runner_pool_gitlab_connections` and `gitlab_jobs`, so the count is now 73. I changed the expectation and added `table_exists` assertions for the three new tables. No production code was changed by me.

`cargo test --workspace --no-fail-fast --locked`: 843 passed, 12 failed. All 12 are host-specific to this macOS ARM64 workstation and sit in files outside the #137 diff:
- 11 `gridops-manager` tests (`provision_lock_tests`, `route_timeout_tests`, `tart_outage_tests`): `Error: path must be shorter than SUN_LEN` (re-checked on `two_tart_starts_still_take_turns`).
- 1 `gridops-api --test fleet_http` `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`: the host is itself macOS ARM64. Recorded in handoff-T-red before any change.

`cargo test -p gridops-core --locked`: every binary ok (lib 259 passed, incl. the repaired db test; `fleet_schema` 14; `gitlab_contract` 14; `fleet_agent_outcomes` 8; `fleet_agent_commands` 9).

Spot-checks (behavior removed, test fails; each reverted afterwards, `git status` shows only `db.rs` modified):
- `GITLAB_EXIT_IDLE_EXPIRED` 75 to 76: `gitlab_runner_constants_are_pinned` and `gitlab_exit_reasons_map_each_exit_status_once` fail.
- `runner_pools.run_untagged` default 0 to 1 in 0046: `fleet_schema` fails 3 tests (`additive_upgrade_preserves_every_legacy_row_index_and_trigger`, `gitlab_upgrade_from_0045_keeps_the_fleet_graph_and_every_reference`, `gitlab_tables_enforce_the_pinned_constraints`).

## Tier 1 results

- `cargo fmt --all -- --check`: clean. PASS.
- `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl`: Finished, no warnings. PASS.
- `cargo +1.98.1 clippy -p gridops-core --all-targets --locked -- -D warnings`: Finished. PASS.
- `npx @redocly/cli@2.72.1 lint docs/openapi/gridops-v1.json`: valid, 67 warnings (matches F). PASS.
- Build and test compile succeed across the workspace. PASS.
- Server start / HTTP smoke: no handler added (R2-W3); the OpenAPI spec tests in `gridops-api` (60 passed) cover the contract. N/A beyond that.

F's reported results reproduce. One discrepancy: F saw a failure count of 14 on an earlier pass, 13 on the complete run. I count 12 now, which is F's 13 minus the repaired db test. No unexplained failure.

## Tier 2 results

- Coverage per acceptance criterion: PASS (see below; suites authored in Phase 4).
- Test quality: PASS. Tests name behaviors, mutation spot-checks fail for the right reason. No redundant tests identified; suite size is proportionate to a 3-table, 2-rebuild migration plus trait, fake and wire contract.
- Type safety: no `any` equivalents; secrets use hand-written redacting `Debug` (covered by tests). PASS.
- Error handling: fake failure injection, `GitLabError`, validate arms for `gitlab` (manager, tart-agent), and the `fk_guard` rollback are all tested. PASS.
- Data integrity: 0046 rebuild preserves rows, indexes, triggers, FK actions (the `runner_events` cascade mutation is caught). PASS.
- API contract: OpenAPI tests pin the 7 operations and `ApiErrorBody`. PASS.
- Security: `glrt-` added to `SECRET_PREFIXES`, tokens redacted. PASS.
- Migration safety: forward-only like the rest of `migrations/`; data preserved. No down-migration convention exists. PASS.
- Playwright: not applicable (no UI, no Playwright in this change).
- Evidence appendix: `evidence.md` already lists each unchanged-source fact with verbatim excerpts; I verified there is nothing further the tests rely on. PASS.

## Acceptance criteria status

All criteria in the brief are backed by passing Phase-4 tests: schema/migration (`fleet_schema`), trait/fake/credentials (`gitlab_contract`, `gitlab::fake::tests`), `CiTarget`/`ensure_target`/`target_from_db` (`admission_tests`), provisioning plan and exit codes (`provisioning::tests`), wire guards (`fleet_agent_commands`, `fleet_agent_outcomes`), secret masking (`gridops-agent`), OpenAPI (`gridops-api`), manager/tart validate arms. PASS.

## Blocking issues

None.

## Advisory notes

- `.aftersight/project.json` is modified in the branch diff against master; this is pipeline configuration, not #137 production code. The reviewer should confirm it is intended.
- The 0046 guard checks the whole database via `pragma_foreign_key_check` (F's known issue); safe failure mode, accepted.
- 12 host-specific failures on macOS persist; CI on Linux is the authoritative run for them.
