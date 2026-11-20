# Handoff-T-red: Phase 4 - GitLab shared contract (#137)

**Date:** 2026-11-20
**Branch:** claude/gitlab-137-contract-fa3bbe (head 78a1266)
**Brief / wireframe reviewed:** docs/handoffs/137-brief.md (all 1010 lines, including every amendment section); wireframe N/A (no UI surface)

## A precondition

Confirmed: A returned **PASS** on the plan in round 3 (`docs/handoffs/137/handoff-A.md`, commit 78a1266), with four non-blocking warnings.

- Warnings 1 and 4 change only ownership and deviation text, so no test depends on them.
- Warning 3 (the `GitlabBootstrap` derives) is respected. No test depends on `deny_unknown_fields` on either crate's `GitlabBootstrap`. The Tart agent's copy only needs `Deserialize`, and the manager's needs `Serialize` for forwarding, which the brief pins.
- Warning 2 (relaying `exitReason` through the manager's `ManagedRunner`) is not adopted by the brief, so no test pins it. If O folds it into #137, I add that test.

## Tests authored

Every test is at the cheapest tier that can observe the behaviour. Serde and type shapes are unit or integration tests with no I/O. Schema rules run on real SQLite files, the only tier where CHECKs, FKs, triggers and the table rebuild can be seen. There are no e2e tests, because there is no UI and no handler.

### `crates/gridops-core/tests/gitlab_contract.rs` (new, integration, public API only)

| Test | What it pins |
|---|---|
| `gitlab_is_a_ci_platform_with_a_snake_case_name` | `CiPlatform::Gitlab` has wire `"gitlab"`, and `CiPlatform::as_str()` returns github/bitbucket/gitlab (R2-1 item 1). |
| `gitlab_connection_ids_are_validated_plain_strings` | `GitlabConnectionId` has the `BitbucketConnectionId` shape: `new`, `as_str`, plain-string serde, `TryFrom<String>`, `From<_> for String`, and it rejects an empty id. |
| `gitlab_targets_have_the_pinned_wire_shape_and_canonical_keys` | `CiTarget::GitlabProject`/`GitlabGroup` serialize as `{"platform":"gitlab_project",...}` and `gitlab_group`. Unknown fields, a zero id and an empty connection id are refused. The canonical keys are `gitlab:project:{conn}:{id}` and `gitlab:group:{conn}:{id}`. |
| `gitlab_targets_are_gitlab_and_never_classified_as_bitbucket` | `is_gitlab()` is added. `is_bitbucket()` is false for GitLab, which catches the `!is_github()` misclassification. GitHub and Bitbucket are unchanged. |
| `target_kinds_for_gitlab_use_the_same_bare_wire_strings` | `TargetKind::GitlabProject`/`GitlabGroup` serialize as bare strings and come `From<&CiTarget>`. `is_gitlab()` is true only for them. A `SupportedRunner` advertising `gitlab_project` parses (A-1, B-6). |
| `gitlab_runner_material_is_the_pinned_json_and_round_trips` | A-2: `to_value` equals the pinned literal by JSON-value equality, it round-trips, any key order is accepted, and the single-variant destructuring is irrefutable. |
| `gitlab_runner_material_refuses_extra_fields_and_other_kinds` | An extra field is refused, whether last or before `kind`. Also refused: another `kind`, a missing `kind`, and a missing `wait_timeout_seconds`. |
| `gitlab_runner_material_debug_never_prints_the_token` | `{:?}` doesn't contain the token, and it equals the exact pinned Debug string. |
| `client_credentials_and_created_runners_redact_their_secrets` | The `GitLabCredentials` Debug shows `base_url` and `access_token: "[redacted]"` and never the token. The `CreatedRunner` Debug never shows its token. |
| `scopes_name_their_kind_id_path_and_fleet_target` | The `ScopeKind` wire and `as_str`, plus `Scope::kind/id/path/target(conn)`, mapping to the right `CiTarget` (A-3). |
| `job_statuses_use_gitlab_spellings_and_absorb_unknown_values` | All 12 `JobStatus` spellings via serde and `as_str`. An unknown string deserializes to `Unknown`, and `Unknown.as_str()` is `"unknown"`. |
| `only_finished_jobs_are_terminal_and_only_runner_seeking_jobs_are_active` | `is_terminal` is exactly the 4 finished statuses and `is_active` exactly the 5 active ones. No status is both. `GITLAB_ACTIVE_JOB_STATUSES` equals the `is_active` set (R3 B-7). |
| `the_active_jobs_index_predicate_is_built_from_the_active_statuses` | The `gitlab_jobs_active_created_idx` statement in `migrations/0046_gitlab.sql` contains the `IN (...)` list built from `is_active` and `as_str` (whitespace-insensitive). The constant has the same order. |
| `gitlab_targets_travel_in_prepare_environment_free_of_bitbucket_rules` | A GitLab target travels unchanged in `PrepareEnvironment.target` and round-trips. Neither Bitbucket rule (persistent-only, no Windows ARM64) refuses a GitLab target. The same two shapes are refused for a Bitbucket control. |

### `crates/gridops-core/src/gitlab/fake_tests.rs` (new, unit, mounted from `fake.rs`)

`FakeGitLab` is behind `cfg(any(test, feature = "test-support"))`. An integration test only sees it with the feature on, and plain `cargo test` would then skip it silently. So these tests are a sibling module, following the `fleet/admission.rs` → `admission_tests.rs` precedent. **F must add these three lines to `crates/gridops-core/src/gitlab/fake.rs`:**

    #[cfg(test)]
    #[path = "fake_tests.rs"]
    mod tests;

| Test | What it pins |
|---|---|
| `runners_are_numbered_per_instance_with_fake_tokens_and_listed` | `create_runner` (called through `Arc<dyn GitLabApi>`) hands out ids 1, 2, … per base URL, with tokens `glrt-fake-runner-token-{id:04}`. `runners(base_url)` lists `FakeRunner { id, scope, tags, run_untagged, description }`, per instance. |
| `deleting_a_runner_removes_it_and_unknown_runners_are_not_found` | Delete removes the runner, and another token on the same base URL may delete it (state is shared per instance). An unknown id is `NotFound`. Instances are isolated. |
| `jobs_and_pending_jobs_come_from_the_seeded_instance` | `with_job` feeds `job`. An unknown job, and a job on another instance, are `NotFound`. `pending_jobs` returns only `Pending` jobs of that project on that instance. |
| `scopes_resolve_by_kind_and_path_and_groups_list_their_projects` | `with_scope` feeds `resolve_scope`, looked up by kind **and** path. A wrong kind, an unknown path or another instance gives `NotFound`. `verify_access` is `Ok` for a seeded scope. `with_group_project` feeds `group_projects` per instance. |
| `queued_failures_are_one_shot_and_first_in_first_out_per_instance_and_method` | `fail_next` queues are per (base URL, method), one-shot and FIFO. They apply whatever the token. A failed call creates nothing. Every `FakeMethod` honours its queue, and every `GitLabError` variant comes back as given. |
| `every_call_is_recorded_in_invocation_order_across_clones_without_tokens` | B-5: five calls interleaving four methods and two base URLs, through two clones (one as `dyn GitLabApi`). One call fails because of `fail_next`. Then `assert_eq!` on the whole `calls()` vector. The Debug of the log has no token, and the `FakeCall { base_url, method }` literal shows there is no other field. |

### `crates/gridops-core/tests/fleet_schema.rs` (integration, real WAL SQLite with FKs on)

| Test | What it pins |
|---|---|
| `additive_upgrade_preserves_every_legacy_row_index_and_trigger` (**extended, not loosened**) | Starting from the legacy fixture (0021): every row of every table equals the pre-upgrade row on every pre-existing column. The only added columns are `runners.gitlab_connection_id`/`gitlab_runner_id`, which must be NULL, and `runner_pools.run_untagged`, which must be `NOT NULL DEFAULT 0` and 0 on every row. Every legacy definition is byte-equal, except `runners`, its 8 indexes and `runner_pools`. `assert_runners_contract` checks exactly 10 indexes with their columns, exactly 5 outbound FKs with their actions, `ci_platform` accepting `'gitlab'`, and the `ci_platform`/`provider`/`runtime` CHECKs still refusing bad values. Also: the FK check is empty, integrity is `ok`, the database is still WAL, and `foreign_keys=1` on **every** pooled connection. A rerun replays nothing. |
| `gitlab_upgrade_from_0045_keeps_the_fleet_graph_and_every_reference` (new) | The "Done-when, extended for A-1" test. It builds 0021 + legacy, then 0045 + `fleet_ready.sql`, then runs `UPDATE workload_placements SET runner_id='legacy-runner'`, so all three inbound FKs of `runners` are populated. It snapshots, then applies 0046. Every row is exactly equal. Added columns are exactly the six listed (all NULL, or 0 for `run_untagged`). Every definition is unchanged, except the exempt `runners`, `runner_pools`, `fleet_ci_targets` and its trigger, and the trigger is still present. `pragma_foreign_key_list` is unchanged for every other table, the seven target referrers included. `fleet_ci_targets` has exactly 4 NO ACTION FKs. The runners contract holds, and the placement's `runner_id` is still `'legacy-runner'`. Both fixture targets are present. Plus the FK check, integrity, and `foreign_keys=1` on every connection. |
| `gitlab_fleet_targets_satisfy_the_shape_check_and_stay_immutable` (new) | Accepts a `gitlab_project` row and a `gitlab_group` row. Refuses 12 shapes: a GitHub or Bitbucket column on a GitLab row, a missing or zero project id, a group row naming a project, a missing connection, GitLab columns on GitHub or Bitbucket rows, an unknown kind, a reused canonical key, a non-integer id (STRICT) and a dangling connection. An UPDATE aborts with `CI target identity is immutable`, and the table is still STRICT. |
| `gitlab_tables_enforce_the_pinned_constraints` (new) | `gitlab_connections`: unique scope, CHECKs on `scope_kind` and `scope_id`, both secret keys unique, and `created_by` → `users` SET NULL. `runner_pool_gitlab_connections`: PK, FK, and the index `(connection_id,pool_id)`. `run_untagged` is 0. `gitlab_jobs`: the unique `(connection_id, job_id)`, the defaults `'[]'`/0/NULL/NULL, the partial index `(created_at,id)`, and the FKs CASCADE/SET NULL. Deleting a connection nulls `runners.gitlab_connection_id` but keeps `gitlab_runner_id`, and cascades memberships and jobs. |
| `a_gitlab_connection_a_fleet_target_references_cannot_be_deleted` (new) | B-2. A `DELETE` of a connection referenced by a `gitlab_project` or `gitlab_group` target fails with `FOREIGN KEY constraint failed`, and both tables are unchanged afterwards. An unreferenced connection deletes (the control). |
| `the_0046_foreign_key_guard_fails_on_a_dangling_reference` (new) | W-8. It extracts 0046's own guard statements (those naming `_gitlab_0046_fk_guard`, comments stripped), which must be the CREATE/INSERT/DROP triple. It runs them on a scratch database with `foreign_keys` OFF: with one dangling child, a `CHECK constraint failed` error; with none, success. |

### `crates/gridops-core/src/fleet/admission_tests.rs` (unit; needs the private `target_from_db`)

| Test | What it pins |
|---|---|
| `gitlab_targets_are_stored_once_and_load_back_unchanged` | `ensure_target` of `GitlabProject` and `GitlabGroup` returns a UUID v4, and a second call returns the same id. The row has the right `kind`, `canonical_key`, `gitlab_*` values and all six GitHub/Bitbucket columns NULL, and is stored once. `target_from_db` loads back the identical `CiTarget`. Both Bitbucket variants still return exactly `InvalidRequest("only GitHub targets are synced")` and write nothing. |

### `crates/gridops-core/src/provisioning.rs` (unit, in the existing `tests` module)

| Test | What it pins |
|---|---|
| `api_and_reconciler_inputs_produce_the_same_github_plan`, `bitbucket_is_always_persistent_and_vm_hosted` (updated) | Unchanged assertions, rewritten for the D9 signature `platform: CiPlatform`. |
| `each_platform_gets_its_plan_on_both_providers` (new) | 12 rows: 3 platforms × 2 providers × 2 pool-input sets. The 8 GitHub and Bitbucket rows were **verified against today's `bool` implementation** with a throwaway probe, and match it exactly. The GitLab rows are always `ephemeral`/`true`/`vm`, with `tart_image` on Tart and `docker_image` otherwise. |
| `gitlab_runner_constants_are_pinned` (new) | `GITLAB_DEFAULT_WAIT_TIMEOUT_SECS = 600`, `GITLAB_RUNNER_TOKEN_PREFIX = "glrt-"`, and the exit codes 0/75/77. |
| `gitlab_exit_reasons_map_each_exit_status_once` (new) | `GITLAB_EXIT_REASONS == [(0,"completed"),(75,"idle_expired"),(77,"registration_refused")]`, written as literals (B-8). |

### Other crates

| File | Test | What it pins |
|---|---|---|
| `crates/gridops-core/tests/fleet_agent_outcomes.rs` | `gitlab_runner_endings_round_trip_as_failure_diagnostics` | W-4 / B-6 exactly. For each pair, it constructs `CommandOutcome::Failed { code, retryable: false, diagnostic: Some(..) }` and serializes it, checking for `"kind":"failed"` and the wire code. It then deserializes and checks for equality, and matches out `diagnostic.code`. The pairs are (`ResourceUnavailable`, `IdleExpired`) and (`Configuration`, `RegistrationRefused`). |
| `crates/gridops-core/tests/fleet_agent_commands.rs` | `a_github_prepare_environment_keeps_its_master_bytes` | **Guard, green by design.** `serde_json::to_string(&command(fixture())?)` must byte-equal the captured `master` literal. I re-captured it on this worktree, whose core is identical to `master`, and it matched the brief's bytes. |
| same | `execution_configuration_has_exactly_its_four_fields` | **Guard, green by design.** `ExecutionConfiguration::default()` serializes to exactly the four fields, and `{"base_url":..}` or `{"url":..}` is refused as an unknown field (B-7). |
| `crates/gridops-manager/src/main.rs` (tests module) | `a_gitlab_request_carries_its_bootstrap_and_token_through_the_manager` | A GitLab `/v1/runners` body fills `gitlab {url, wait_timeout_seconds}` and `gitlab_runner_token`. Re-serializing it (the Tart forward) keeps `gitlab {url, waitTimeoutSeconds}` and `gitlabRunnerToken`. |
| same | `a_github_request_without_gitlab_fields_still_parses` | Older requests still parse, with both fields `None`. |
| same | `gitlab_runners_are_refused_by_name_until_their_owners_land` | `validate()` returns exactly `BadRequest("GitLab runners are not supported yet: Docker is #147, Tart is #142.")` for both providers. |
| same | `provision_request` fixture (updated) | Gains `gitlab: None, gitlab_runner_token: None`, so the existing tests compile against the new struct. |
| `crates/gridops-tart-agent/src/main.rs` (tests module) | `a_gitlab_request_carries_its_bootstrap_and_token`, `gitlab_runners_are_refused_by_name_until_142_lands`, `gitlab_exit_reasons_match_the_core_table` | The same parsing for the Tart struct. The exact `BadRequest("GitLab runners are not supported yet: Tart is #142.")`. The private `GITLAB_EXIT_REASONS` copy, pinned with the same literals as core. The `provision_request` fixture gains the two `None` fields. |
| `crates/gridops-agent/src/session.rs` | `events_mask_credentials` (**extended**) | `glrt-fake-runner-token-0001` and a `glpat-` token are masked whole (`glrt-[redacted]`, `glpat-[redacted]`). A short `glrt-short` is left alone. |
| `crates/gridops-api/src/fleet/openapi.rs` (tests module) | `the_gitlab_operations_are_specified_before_they_are_served` | The 7 W-7 operations exist. Each description starts "Not served yet: implemented by #138" (or #140 for the webhook). Security is exactly `[{"cookieAuth":[]}]`, or `[]` for the webhook. Success codes and body `$ref`s are as pinned. Errors use `ApiErrorBody` and never `ErrorResponse`. Both 409s exist, and DELETE's reads "while a pool or a fleet target references it". The `connection_id` path parameter is present. The webhook requires the `X-Gitlab-Token` and `X-Gitlab-Event` headers, has a 401, and takes a free-form object body. |
| same | `the_gitlab_schemas_have_the_pinned_fields_and_no_secrets` | Exact property sets for `GitlabConnection`, `GitlabConnectionList`, `CreateGitlabConnectionRequest` (`baseUrl` default `https://gitlab.com`, `scopeKind` enum), `GitlabConnectionCreated`, `UpdateGitlabConnectionRequest` (nothing required), `GitlabWebhookSecret` and `ApiErrorBody` (`error` required). `provider` is `"gitlab"` and `scopeId` is an integer. No other `Gitlab*` schema carries `webhookSecret`. |

**Not tests I author** (Tier 1 in Phase 6): `cargo build`, `cargo clippy --all-targets`, and `npm run validate:openapi` (`redocly lint`).

## RED confirmation

### Runtime RED: tests that compile against today's code and fail on the missing feature

`cargo test -p gridops-core --test fleet_schema`: **6 failed / 8 passed.** All 8 existing tests are still green.

    Error: error returned from database: (code: 1) no such table: gitlab_connections      (3 tests: shape check, constraints, FK refusal)
    Error: reading .../migrations/0046_gitlab.sql  No such file or directory (os error 2)   (guard test)
    additive_upgrade...: panicked at fleet_schema.rs:360:9   left: []  right: [("run_untagged", "INTEGER")]
    gitlab_upgrade_from_0045...: panicked at fleet_schema.rs:360:9   left: []  right: [("gitlab_connection_id", "TEXT"), ("gitlab_project_id", "INTEGER"), ("gitlab_group_id", "INTEGER")]

`cargo test -p gridops-agent`: **1 failed / 40 passed.**

    session::tests::events_mask_credentials ... FAILED
      left: "Register glrt-fake-runner-token-0001 now"
     right: "Register glrt-[redacted] now"

`cargo test -p gridops-api --bin gridops-api`: **2 failed / 58 passed.**

    the_gitlab_operations_are_specified_before_they_are_served: panicked: missing get /api/v1/platform-connections/gitlab
    the_gitlab_schemas_have_the_pinned_fields_and_no_secrets: left: []  right: ["baseUrl", "canManage", "createdAt", ...]

### Compile RED: tests over types that don't exist yet (Rust)

Each error names a missing contract item. None is a typo, a wrong import of mine or a setup problem.

- `cargo test -p gridops-core --test gitlab_contract --no-run`:
  - E0432 unresolved import `gridops_core::gitlab`.
  - E0432 unresolved `fleet::domain::GitlabConnectionId` and `protocol::agent::GitlabRunnerMaterial`.
  - E0599 no variant `CiPlatform::Gitlab`, `CiTarget::GitlabProject`/`GitlabGroup` or `TargetKind::GitlabProject`/`GitlabGroup`.
  - E0599 no method `CiPlatform::as_str`, `CiTarget::is_gitlab` or `TargetKind::is_gitlab`.
- `--test fleet_agent_outcomes --no-run`: E0599 no variant `DiagnosticCode::IdleExpired` or `DiagnosticCode::RegistrationRefused`.
- `cargo test -p gridops-core --lib --no-run`:
  - E0308 mismatched types: `CiPlatform` passed where `for_selected_provider` still takes `bool` (D9).
  - E0599 no variant `CiPlatform::Gitlab`.
  - E0425 `GITLAB_DEFAULT_WAIT_TIMEOUT_SECS`, `GITLAB_RUNNER_TOKEN_PREFIX`, `GITLAB_EXIT_COMPLETED`, `_IDLE_EXPIRED`, `_REGISTRATION_REFUSED` and `GITLAB_EXIT_REASONS` not found.
  - E0432 `fleet::domain::GitlabConnectionId`, and E0599 `CiTarget::GitlabProject`/`GitlabGroup` (in `admission_tests.rs`).
- `fake_tests.rs`, mounted for one build only, to capture this (`lib.rs` restored, diff empty): E0432/E0433 unresolved `crate::gitlab`. The `!` is not an iterator errors are knock-on effects of that unresolved import.
- `cargo test -p gridops-manager --no-run`: E0560/E0609 `ProvisionRunner` has no field `gitlab` or `gitlab_runner_token`.
- `cargo test -p gridops-tart-agent --no-run`: E0560/E0609 the same two fields, and E0425 `GITLAB_EXIT_REASONS`.

Consequence F should expect: until each contract item exists, the binaries holding these tests don't compile, so the existing tests in them can't run either. Those binaries are the `gridops-core` lib, `fleet_agent_outcomes`, `gitlab_contract`, the manager and the Tart agent. That is inherent to RED over new types in Rust.

### Guards (green by design, not RED)

`a_github_prepare_environment_keeps_its_master_bytes` and `execution_configuration_has_exactly_its_four_fields` pin the "unchanged" criteria (D1/B-7), so they must pass both before and after. Result: `cargo test -p gridops-core --test fleet_agent_commands` gives **9 passed** (7 existing + 2 guards). They aren't vacuous: a fifth `ExecutionConfiguration` field fails the value equality, a `base_url` field fails the unknown-field check, and any byte change to the GitHub command fails the literal.

### Checking the schema tests' own logic (throwaway draft, never staged)

The schema tests are the most complex and the most likely to fail F for the wrong reason, so I checked them against a **throwaway draft** of 0046. It was written in the session scratchpad, copied into `migrations/` for one run each, then deleted (`git status migrations` is clean).

- **Draft as briefed** (no-transaction, FK off, guard): `fleet_schema` gives **14/14 passed**. The assertion helpers, pragma queries and fixtures work.
- **Mutation: the same draft without the transaction control** (the naive in-transaction rebuild the brief warns about): **3 failed.**
  - `additive_upgrade...` fails on the row-equality assertion (line 127), because `runner_events` was cascaded away.
  - `gitlab_upgrade_from_0045...` and `inventory_is_invalidated...` fail with `while executing migration 46: FOREIGN KEY constraint failed`.

  So the tests catch the hazard the brief is about.

The draft is not a suggestion for F's migration. F writes 0046 from the brief.

### Hygiene

- `cargo fmt --all -- --check` is clean.
- `cargo +1.98.1 clippy -D warnings -A clippy::unused_async_trait_impl` (CI's toolchain) is clean on every target that compiles today: `-p gridops-api -p gridops-agent --all-targets`, and `gridops-core --test fleet_schema --test fleet_agent_commands`.
- The other core integration suites are green: `fleet_capabilities`, `fleet_contracts`, `fleet_inventory`, `fleet_queries`, `fleet_query_contracts`, `fleet_registry`, `api_tokens`, `builtin_host`, `retention` and `unused_provider_images`.

- **Not caused by #137, and not blocking.** `gridops-api --test fleet_http` `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on this workstation. The preflight reports `PASS macos-machine: ... fleet-host`, because the test host is itself a macOS ARM64 machine. That file has no #137 change, and CI runs on Linux.
- **Not caused by #137.** Local clippy 1.99.1 adds an `assert_is_empty` lint that flags master code in `gridops-agent` (`catalog.rs`, `changes.rs`, `tools.rs`). CI pins 1.98.1, which is clean.

## Ready for F

**RED is valid. F may implement against these tests.** Where the brief leaves a choice open, the tests take one reading. F implements to these, or tells T if one contradicts the brief:

1. **Mount the fake tests.** Add the three-line `#[cfg(test)] #[path = "fake_tests.rs"] mod tests;` to `gitlab/fake.rs`. In Phase 6 I'll check that the six fake tests actually run.
2. **Fake API shape.**
   - `with_scope`, `with_job`, `with_group_project` and `fail_next` take `&self` (shared `Arc<Mutex>` state), called as statements.
   - `runners(&Url) -> Vec<FakeRunner>`, with pub fields `id, scope, tags, run_untagged, description`.
   - `calls() -> Vec<FakeCall>`, where `FakeCall { pub base_url: Url, pub method: FakeMethod }` derives `Debug, PartialEq` and has no other field.
   - The `FakeMethod` variants are `CreateRunner, DeleteRunner, Job, PendingJobs, GroupProjects, ResolveScope, VerifyAccess` (`Debug, Clone/Copy, PartialEq`).
   - A call that fails through `fail_next` has no side effect.
3. **Item locations.**
   - `GitlabConnectionId` is in `fleet::domain` (beside the Bitbucket id).
   - `GitlabRunnerMaterial` is in `fleet::protocol::agent`.
   - `gitlab::{GitLabApi, GitLabCredentials, GitLabError, CreatedRunner, Job, JobStatus, Scope, ScopeKind, GITLAB_ACTIVE_JOB_STATUSES}` and `gitlab::fake::{FakeGitLab, FakeCall, FakeMethod, FakeRunner}`.
   - `GitLabError` derives `Debug, PartialEq`.
   - `ScopeKind::as_str()` and `CiPlatform::as_str()` exist.
4. **The Tart agent's `GITLAB_EXIT_REASONS`** must be visible to its `tests` module (`use super::*`). The manager's `GitlabBootstrap` has the fields `url` and `wait_timeout_seconds` and derives `Serialize`.
5. **0046 text the tests read.**
   - The active-index statement names `gitlab_jobs_active_created_idx` and holds `IN ('created','waiting_for_resource','preparing','pending','running')`, in that order.
   - The guard is three statements naming `_gitlab_0046_fk_guard`: CREATE, INSERT, DROP.
   - No `;` inside a `--` comment matters, because comments are stripped first.
6. **The rebuild keeps each existing `runners` column's declared type, NOT NULL and default text.** For example, `busy` keeps `DEFAULT false`, not `0`. The tests compare `pragma_table_info` before and after. Copying the current definition from `sqlite_master` and changing only the `ci_platform` CHECK and the two appended columns satisfies this.
7. **Test-design deviation from the brief, recorded in decisions.md.** The brief says the definition check "may exempt only `runners` and the `runners_*` indexes". `runner_pools` is exempt too, because the brief's own `ALTER TABLE runner_pools ADD COLUMN run_untagged` necessarily rewrites that table's `sqlite_master` text. In its place, the test asserts that every existing `runner_pools` column is unchanged and that exactly `run_untagged INTEGER NOT NULL DEFAULT 0` is appended, with 0 in every row. The row comparison likewise uses pre-upgrade columns for **every** table, with each added column checked on its own. This is stricter than the brief, not looser.
