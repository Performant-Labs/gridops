# Handoff-S: Phase 8 - GitLab shared contract (#137)

**Date:** 2026-11-20
**Branch:** claude/gitlab-137-contract-fa3bbe
**Issue:** #137
**Handoff-T reviewed:** docs/handoffs/137/handoff-T-green.md (and handoff-T-red.md)
**Handoff-A reviewed:** docs/handoffs/137/handoff-A.md (plan, round 3: PASS) and docs/handoffs/137/handoff-A-dup.md (anti-duplication: PASS)
**Handoff-F reviewed:** docs/handoffs/137/handoff-F.md
**Spec:** docs/handoffs/137-brief.md (issue text, plus the amendments and deviations D1-D13; where they conflict, the amendments win, as the brief states)
**Operator-facing report:** not applicable (no visual surface)

## A precondition
Confirmed. A returned PASS on the plan (round 3, four non-blocking warnings) and PASS on the anti-duplication gate (three warns, none blocking).

## T precondition
Confirmed. T-green reports "Blocking issues: None". Its 12 remaining workspace failures are workstation-specific (11 `gridops-manager` `SUN_LEN` socket-path tests and 1 macOS-host `release_preflight` test), in files outside the #137 diff.

## Visual-diff-tool precondition
Not applicable. #137 is a Rust and SQL contract change (types, schema, trait and fake, wire shapes, OpenAPI JSON). It has no rendered surface, so the browser, Playwright and ImageMagick preconditions and the HTML report do not apply.

## Preview / spec sanity check
Done on the spec (brief) before auditing. No source-of-truth defect that justifies ADVISORY-HOLD:
- Schema: CHECKs, FK actions and the NO ACTION consequence of `fleet_ci_targets.gitlab_connection_id` are internally consistent and documented (A-1).
- API: status codes follow standard semantics (201 create with the one-time secret, 204 delete/verify, 409 conflicts, 202 webhook accept, 401 secret mismatch, PATCH for a partial update). Error body is the real `ApiError` shape, not the fleet `ErrorResponse`.
- Two documentation staleness items exist in the brief's deviation table. They are not defects in what F built, so they are advisory notes for O (see below), not a hold.

## Visual diff results
Not applicable.

## Independent verification I ran (not only T's reports)

| Check | Result |
|---|---|
| `cargo test -p gridops-core --locked` | every binary ok: lib 259, `fleet_schema` 14, `gitlab_contract` 14, `fleet_agent_outcomes` 8, `fleet_agent_commands` 9, others ok |
| `cargo test -p gridops-api -p gridops-reconciler -p gridops-host-agent -p gridops-agent -p gridops-tart-agent --no-fail-fast` | all pass except `fleet_http release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` (host is macOS ARM64; file untouched by the diff) |
| `gridops-manager` GitLab tests (filtered) | 3 pass. I did not re-run the 11 `SUN_LEN` tests; they are in untouched modules and CI is Linux |
| `cargo fmt --all -- --check` | clean |
| `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (the exact CI line, `.github/workflows/ci.yml:111`) | clean |
| `cargo +1.98.1 clippy -p gridops-core --features test-support` (lib build with the fake compiled outside `cfg(test)`) | clean |
| `npx @redocly/cli@2.72.1 lint docs/openapi/gridops-v1.json` | valid, exit 0, 67 warnings (63 `operation-operationId`, 4 existing example warnings; 60 on `HEAD`, the +7 are the new operations, which follow the 56 existing ones in having no `operationId`) |
| sqlite3 3.54.0: migrations 0001-0045, snapshot DDL, apply `0046_gitlab.sql`, diff DDL | `runners` differs only by the widened `ci_platform` CHECK, the two appended nullable columns and the two new indexes; `fleet_ci_targets` differs only by the two kinds, three nullable columns and the extended shape CHECK. All other indexes, FKs and the immutability trigger are identical. `PRAGMA foreign_key_check` empty, `integrity_check` ok |

## Spec compliance

| Spec requirement | Compliant | Notes |
|---|---|---|
| `CiPlatform::Gitlab`, `as_str()` (R2-1) | YES | wire `"gitlab"`; `CiPlatform` has no match sites |
| `GitlabConnectionId` alongside `BitbucketConnectionId` | YES | same shape; re-exported from `fleet/mod.rs` |
| `CiTarget::GitlabProject` / `GitlabGroup`, canonical keys `gitlab:project:{conn}:{id}` / `gitlab:group:{conn}:{id}` | YES | `canonical_key` (infallible) gained both arms |
| `is_bitbucket()` explicit; `is_gitlab()` added | YES | no longer `!is_github()`; GitLab is never classified as Bitbucket |
| Exhaustive `match` sites compile; `target_from_db` GitLab arms | YES | `From<&CiTarget> for TargetKind`, `target_from_db` extended in place |
| Other sites that must not misclassify GitLab | YES | PrepareEnvironment Bitbucket-only rules and `static_compatibility` use explicit Bitbucket tests; no GitLab platform rule added (left to #147/#142 as the brief says); user-scoped readable-targets and `ensure_target_identity_for_pool` stay admin-only / `_ => false` (D11, #139) |
| `TargetKind::GitlabProject/GitlabGroup` + `is_gitlab`, bare snake_case wire strings | YES | serde attributes unchanged; `SupportedRunner` unchanged |
| `ensure_target` stores GitLab, idempotent, Bitbucket refusal string unchanged | YES | private `ensure_gitlab_target` + shared `target_id_for_key`; error `"only GitHub targets are synced"` kept word for word |
| Migration `0046_gitlab.sql`: `-- no-transaction`, FK off, own transaction, `pragma_foreign_key_check` guard, FK on | YES | first line is the sqlx directive; statement order matches W-5/W-6 |
| `gitlab_connections` + 3 unique indexes; `runner_pool_gitlab_connections` + index; `runner_pools.run_untagged INTEGER NOT NULL DEFAULT 0`; `gitlab_jobs` with unique `(connection_id, job_id)` | YES | DDL matches the brief's pinned text |
| Active-jobs partial index `gitlab_jobs_active_created_idx` (R3 B-7, D13) | YES | five-status predicate equals `GITLAB_ACTIVE_JOB_STATUSES` (tested) |
| `fleet_ci_targets` rebuild (A-1): kinds, 3 nullable columns, shape CHECK with six arms, STRICT, UNIQUE canonical_key, trigger verbatim, 4 outbound NO ACTION FKs | YES | confirmed by my DDL diff and T's tests |
| `runners` rebuild (B-5): all columns, defaults, CHECKs, 4 outbound FKs (+ new), 10 indexes, inbound FKs and rows intact | YES | `runners_github_id_unique` not resurrected; inbound `runner_events` cascade not triggered (mutation-checked by T) |
| `GitLabApi` (7 methods, credentials first), `GitLabCredentials`, `Scope`/`ScopeKind`, `CreatedRunner`, `Job`, `JobStatus` (+`is_terminal`, `is_active`, `as_str`), `GitLabError` | YES | signatures match A-3; hand-written redacting `Debug` on credentials and `CreatedRunner`; `#[serde(other)] Unknown` |
| `FakeGitLab` behind `cfg(any(test, feature = "test-support"))`, `Clone`+`Default`, keyed by `base_url`, setup methods, `fail_next` FIFO, effects, `calls()` in invocation order, doc comment states limits | YES | `test-support` feature added; `async-trait` added to core |
| `FakeGitLab` own tests, including the whole-vector `assert_eq!` on `calls()` (5 calls, 4 methods, 2 base URLs, 2 clones, one `fail_next` failure) | YES | `fake_tests.rs`, 6 tests |
| `GitlabRunnerMaterial` (single-variant internally tagged enum, `deny_unknown_fields`, exact Debug string); `ExecutionConfiguration` and `PrepareEnvironment` untouched (D1) | YES | no definition of either struct changed in the diff; field-set test and the master-bytes `PrepareEnvironment` test are present and green |
| `DiagnosticCode::{IdleExpired, RegistrationRefused}`, `FailureCode` unchanged (D5), round-trip tests inside `CommandOutcome::Failed` | YES | both pairs, serialize/deserialize/equality/match, as the brief specifies |
| `for_selected_provider(provider, platform: CiPlatform, ...)` (D9); constants `GITLAB_DEFAULT_WAIT_TIMEOUT_SECS`, `GITLAB_RUNNER_TOKEN_PREFIX`, exit codes 0/75/77, `GITLAB_EXIT_REASONS` | YES | GitHub and Bitbucket results unchanged (12-row table test, 8 rows verified against the old `bool` code); both callers changed mechanically |
| Manager and Tart agent `ProvisionRunner` gain `gitlab` and `gitlabRunnerToken`; exact `("gitlab", _)` refusal strings; Tart exit-reason copy + pin test (D10) | YES | manager string names #147 and #142, Tart string names #142; manager skips the new fields when absent so forwarded GitHub/Bitbucket bytes are unchanged |
| Redaction: `glrt-` added (`glpat-` already present), array length 11, test extended | YES | whole token masked; short `glrt-short` left alone |
| OpenAPI: 7 operations, "Not served yet: implemented by #138/#140", `cookieAuth` only (webhook `security: []`), `ApiErrorBody`, PascalCase components, `{connection_id}`, DELETE 409 text "pool or a fleet target" | YES | spec tests `the_gitlab_operations_are_specified_before_they_are_served` and `the_gitlab_schemas_have_the_pinned_fields_and_no_secrets` green; `TOKEN_SCOPED_ROUTES` unchanged |
| No handler, reconciler loop, HTTP client, host-agent code, UI, `AppState`/`Reconciler` field (R2-W3) | YES | none added |
| Done-when: build, clippy, existing suites, fixtures survive the rebuild, `FakeGitLab` tests, `redocly lint` | YES | see verification table; the only failures are the 12 host-specific ones |

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | Spec-only. Paths, verbs, status codes and schemas match W-7; secrets appear only in the create and rotate responses |
| Error handling | PASS | `GitLabError` variants as specified; fake failure injection tested; the `fk_guard` failure path tested; manager and Tart refusal arms return errors, not panics |
| UI/UX match to spec | N/A | no UI |
| Accessibility | N/A | no UI |
| Architecture gate | PASS | A plan PASS (round 3) and A-dup PASS. No quality finding conflicts with either handoff |
| Code organization | PASS | New module beside `bitbucket.rs`; GitLab arms added in place; no dead code beyond the Tart agent's `#[allow(dead_code, reason = ...)]` items that #142 reads, each with a stated reason |
| Security | PASS | Tokens never in `Debug` (hand-written on credentials, `CreatedRunner`, `GitlabRunnerMaterial`); `glrt-` redaction; fake never records tokens; token in `GitlabRunnerMaterial.token` is a `String` by design (must serialize; brief closes round-1 B-5) |
| Performance | PASS | Active-jobs index is partial and bounded to waiting or running jobs; the rebuilds are one-time and copy `INSERT ... SELECT` |
| Visual regression | N/A | no UI |
| Naming consistency | PASS | `Gitlab*` for types, snake_case wire strings, `gitlab_*` columns, matching the Bitbucket analogues |
| Test quality (`testing/test-quality.md` section 7) | PASS | see below |

## Test-quality audit

- **Per test:** each names one behavior and fails for a specific reason. T's mutation spot-checks (exit code 75 to 76, `run_untagged` default 0 to 1, a migration without transaction control that dropped the `runner_events` cascade) each turned named tests red. I confirmed the new tests assert behavior (wire strings, constraint refusals, row equality, FK lists, exact `Debug` output), not implementation details.
- **Tier:** serde and type shapes are plain unit/integration tests with no I/O. Schema rules run on real WAL SQLite files, the only tier that observes CHECKs, FKs, triggers and a table rebuild. No e2e or UI test is added (none applies).
- **Suite proportionality:** about 14 contract tests, 6 fake tests, 6 schema tests, plus small additions across outcomes, commands, provisioning, redaction, OpenAPI, manager and Tart agent. For a contract issue that adds 3 tables, 2 table rebuilds, a client trait and fake, and 4 wire changes, this matches what the brief specifies test by test. I found no test to delete or merge: the two "guard, green by design" tests (`a_github_prepare_environment_keeps_its_master_bytes`, `execution_configuration_has_exactly_its_four_fields`) are labelled as such and catch a different failure each (a field added with `skip_serializing_if` leaves the first green; the second catches it).
- **Smells:** none found (no assertion-free, unreachable-outcome, mock-shaped or snapshot-everything tests).
- **"Extended, not loosened":** the brief allowed the schema-unchanged assertion to exempt only `runners` and its indexes. The extended `additive_upgrade_preserves_every_legacy_row_index_and_trigger` also exempts `runner_pools`, because `ALTER TABLE ... ADD COLUMN run_untagged` necessarily rewrites that table's stored DDL. The brief's text overlooked this. The test compensates by asserting the added column is `NOT NULL DEFAULT 0` and 0 on every row, and it still compares every pre-existing column of every row exactly. I accept this as faithful to the intent.

## Scope check
F delivered exactly the brief's scope, including the amendments (A-1 storage glue, A-2 material, A-3 credentials, R2-1 runner-manager shapes, R3 B-7 active index). No handler, loop, HTTP client, host-agent logic or UI. Over-delivery: none. Under-delivery: none against the brief. Items deliberately left to wave 1 are recorded as such (pool-target sync, admission arms, `exitReason` in the listings, `AppState`/`Reconciler` fields, GitLab platform rules).

The branch diff against local `master` also shows `dd525d5` (#135, `webhooks.rs`, `control_plane_contracts.rs`) and `.aftersight/project.json`. `dd525d5` is already on `origin/master` (local `master` is behind), so it is not part of the PR. `.aftersight/project.json` (`f3ad67c`, "gridops does not require git hooks") is not on `origin/master`: see advisory note 3.

## Verdict

PASS — all acceptance criteria met, spec-compliant, quality acceptable. Ready for O to commit.

## Advisory notes
Non-blocking. Items 1 and 2 are for O before the deviation table is pasted into the PR body and the #136 comment.

1. **A's round-3 text warnings 1 and 4 were never applied to the brief.** Still stale in `137-brief.md`: line 203 ("host agents of #147 and #142 advertise in `SupportedRunner`"), line 435 ("#147, #142 and #139 must branch on `diagnostic.code`"), line 582 ("Platform rules for GitLab are #147's and #142's"), and rows D1, D5, D6 (they list #142; D1 shows the material with three fields and omits `wait_timeout_seconds`). The code is correct (A-2 pins four fields and wins). Fix the table before it is repeated on #136 and in the PR, because #142 and #147 implementers read it there.
2. **D14 is unwritten (A round-3 warning 1).** `gridops-api/src/resources.rs::provision` (the in-request Docker start path, gated by `starts_belong_to_reconciler`) calls `for_selected_provider` and is where a GitLab start would be filled in. The brief gives "choosing `CiPlatform::Gitlab` in both callers" to #139 but #138 owns the GitLab sections of `resources.rs`. A's recommended resolution is to make GitLab starts belong to the reconciler (extend `starts_belong_to_reconciler`, owner #138), so the lifecycle lives only in #139's module. Record the decision as D14 in the PR and on #136 before #138 and #139 start.
3. **`.aftersight/project.json` is in this branch's diff against `origin/master`** (`gitHooks.required = false`, commit `f3ad67c`). It is pipeline configuration, not #137 code (T flagged it too). Confirm it is meant to ship in this PR; if not, drop it. Use `origin/master` as the PR base, not the stale local `master`.
4. **A round-3 warning 2 not adopted.** The manager's private `ManagedRunner` has no `exit_reason`, so a Tart agent's `exitReason` would be dropped on the relay hop unless #147 adds the field (with `skip_serializing_if`). Name #147 as owner of that field and its test in the #136 comment.
5. **Deploy note for the migration.** The 0046 guard runs `pragma_foreign_key_check` over the whole database (F's known issue, the brief pins it). A production database that already holds any dangling reference would roll back and refuse to start. Run `PRAGMA foreign_key_check` on the production database before deploying and clean any rows it reports.
6. **Tart agent `ProvisionRunner` derives `Debug`** and now holds `gitlab_runner_token: Option<String>`, the same exposure as the existing `jit_config`. Nothing Debug-prints the struct today (I checked), but #142 should not start to, or should wrap the token.
7. **Host-specific failures.** The 12 macOS-workstation failures (11 `SUN_LEN`, 1 `release_preflight`) need a Linux CI run to confirm. I reproduced only the `release_preflight` one; the 11 manager tests are in untouched modules.
8. **Deviations to repeat in the PR and on #136:** D1-D13 in the brief's two deviation tables (after fixing item 1).
