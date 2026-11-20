# Handoff-F: Phase 5 - GitLab shared contract (#137)

**Date:** 2026-11-20
**Branch:** claude/gitlab-137-contract-fa3bbe
**Issue:** #137

## What was done

- `migrations/0046_gitlab.sql` (new). This is the first table rebuild in `migrations/`.
  - It starts with `-- no-transaction`, turns foreign keys off, and runs its own `BEGIN`/`COMMIT`.
  - In the brief's order, it creates `gitlab_connections`, `runner_pool_gitlab_connections`, `runner_pools.run_untagged` and `gitlab_jobs`, then rebuilds `fleet_ci_targets` and `runners`.
  - The `_gitlab_0046_fk_guard` CHECK guard runs before `COMMIT`, and foreign keys are turned back on at the end.
- `crates/gridops-core/src/gitlab/mod.rs` (new):
  - `GitLabCredentials` and `CreatedRunner`, each with a hand-written redacting `Debug`;
  - `ScopeKind` and `Scope` (with `kind`, `id`, `path` and `target`);
  - `Job`, and `JobStatus` (with `as_str`, `is_terminal` and `is_active`);
  - `GITLAB_ACTIVE_JOB_STATUSES`, `GitLabError`, and `#[async_trait] GitLabApi` (7 methods, credentials first).
- `crates/gridops-core/src/gitlab/fake.rs` (new): `FakeGitLab`, `FakeCall`, `FakeMethod` and `FakeRunner`, behind `cfg(any(test, feature = "test-support"))`. It mounts T's `fake_tests.rs`.
- `crates/gridops-core/src/lib.rs`: adds `pub mod gitlab;`.
- `crates/gridops-core/Cargo.toml`: adds `async-trait`, and `[features] test-support = []`.
- `Cargo.lock`: `async-trait` is added to `gridops-core`'s dependencies.
- `crates/gridops-core/src/fleet/domain.rs`:
  - adds `GitlabConnectionId`, the same shape as `BitbucketConnectionId`;
  - adds `CiTarget::GitlabProject` and `CiTarget::GitlabGroup`, with their canonical keys;
  - adds `is_gitlab()`, and `is_bitbucket()` now names the Bitbucket variants explicitly;
  - adds `CiPlatform::Gitlab` and `CiPlatform::as_str()`.
- `crates/gridops-core/src/fleet/mod.rs`: re-exports `GitlabConnectionId`.
- `crates/gridops-core/src/fleet/capabilities.rs`: adds `TargetKind::GitlabProject`, `TargetKind::GitlabGroup`, their `From<&CiTarget>` arms and `is_gitlab()`.
- `crates/gridops-core/src/fleet/targets.rs`: `ensure_target` stores the two GitLab variants, idempotently. Bitbucket is still refused with the same error.
- `crates/gridops-core/src/fleet/admission.rs`: `target_from_db` parses the `gitlab_project` and `gitlab_group` rows.
- `crates/gridops-core/src/fleet/protocol/agent.rs`: adds `GitlabRunnerMaterial`, a single-variant internally tagged enum with a hand-written redacting `Debug`. `PrepareEnvironment` and `ExecutionConfiguration` are untouched.
- `crates/gridops-core/src/fleet/protocol/outcomes.rs`: adds `DiagnosticCode::IdleExpired` and `DiagnosticCode::RegistrationRefused`.
- `crates/gridops-core/src/provisioning.rs`:
  - `for_selected_provider` now takes `platform: CiPlatform` (D9);
  - adds `GITLAB_DEFAULT_WAIT_TIMEOUT_SECS`, `GITLAB_RUNNER_TOKEN_PREFIX`, the three `GITLAB_EXIT_*` codes and `GITLAB_EXIT_REASONS`.
- `crates/gridops-api/src/resources.rs` and `crates/gridops-reconciler/src/main.rs`: mechanical caller change from `bool` to `CiPlatform::{Bitbucket, Github}`.
- `crates/gridops-manager/src/main.rs`: `ProvisionRunner` gains `gitlab: Option<GitlabBootstrap>` and `gitlab_runner_token: Option<String>`. `GitlabBootstrap` is new. `validate` gains the `("gitlab", _)` arm with the exact #147/#142 message.
- `crates/gridops-tart-agent/src/main.rs`: the same two fields and `GitlabBootstrap`, the `("gitlab", _)` arm naming #142, and private copies of the exit codes and `GITLAB_EXIT_REASONS`.
- `crates/gridops-agent/src/session.rs`: adds `"glrt-"` to `SECRET_PREFIXES` (length 11).
- `docs/openapi/gridops-v1.json`:
  - adds the 7 W-7 operations, all with "Not served yet: implemented by #138" (or #140);
  - adds the `ConnectionId` path parameter and the `platform-connections` and `webhooks` tags;
  - adds the `ApiErrorBody`, `GitlabConnection`, `GitlabConnectionList`, `CreateGitlabConnectionRequest`, `GitlabConnectionCreated`, `UpdateGitlabConnectionRequest` and `GitlabWebhookSecret` schemas.

## Design decisions

- **Rebuild column definitions are copied from the live `sqlite_master` text** (0001 to 0045 applied to a scratch database). Only the `ci_platform` CHECK changes, and the two GitLab columns are appended after `runtime`. Each existing column keeps its declared type, NOT NULL and default text (for example `busy integer DEFAULT false`), as T's `pragma_table_info` comparison requires. The `fleet_ci_targets` rebuild works the same way, and its trigger is recreated word for word.
- **`ensure_target`**: the GitLab arms return early through a private `ensure_gitlab_target`. I chose this over widening the GitHub tuple to six optional columns, because that would have touched every GitHub bind. The shared `SELECT id ... WHERE canonical_key=?` moved into a private `target_id_for_key`. The GitHub insert and the Bitbucket refusal, with its exact string, are unchanged.
- **Manager `skip_serializing_if = "Option::is_none"`** on both new fields. The manager forwards Tart starts by re-serializing `ProvisionRunner` (`evidence.md`), so without it every forwarded GitHub or Bitbucket body would gain `"gitlab":null,"gitlabRunnerToken":null`. The Tart agent only deserializes, so it needs only `#[serde(default)]`.
- **`GitlabBootstrap` keeps `deny_unknown_fields`**, as the brief pins, although `BitbucketBootstrap` lacks it (A round-3 warn 3). The Tart copy derives `Debug, Deserialize` like its sibling, and the manager copy derives `Deserialize, Serialize`.
- **Tart agent `#[allow(dead_code, reason = ...)]`** on the fields, `GitlabBootstrap` and the exit-code constants. Only #142 reads them. I used `allow` rather than `expect`, because the test build does read some of them, which would leave an `expect` unfulfilled.
- **`for_selected_provider`** is rewritten as one `match platform`, with a `(mode, ephemeral, runtime)` tuple. GitHub and Bitbucket give today's values, which T's 12-row table confirms. GitLab is always `ephemeral`/`true`/`vm`.
- **`FakeGitLab` choices the tests leave open:**
  - `job` matches project id and job id together, as GitLab's `GET /projects/:id/jobs/:job_id` does.
  - `verify_access` is `Ok` for a seeded scope of the same kind and id, and `NotFound` otherwise.
  - Unseeded `pending_jobs` and `group_projects` return an empty list.
  - The failure check runs before any effect, so a failed `create_runner` doesn't use up an id.
  - The mutex is recovered from poisoning (`PoisonError::into_inner`), because the workspace denies `unwrap` and `expect`.
- **OpenAPI errors:**
  - Each non-2xx response is inlined with `ApiErrorBody`, and none references the fleet `ErrorResponse`, as T's test requires.
  - The `403 token_not_accepted` bearer refusal is stated in the tag description, not as a response. That body isn't `ApiErrorBody`, and the test checks every error body with content.
  - Hand insertion keeps the hand-formatted file's diff to added lines only.

## Reuse / extend-vs-new

Extended, per the brief: `CiTarget`, `CiPlatform`, `TargetKind`, `ensure_target`, `target_from_db`, `DiagnosticCode`, `ProvisioningPlan::for_selected_provider`, both private `ProvisionRunner` structs and their `validate`, `SECRET_PREFIXES`, and the existing OpenAPI document.

New, because the brief places them there and no GitLab code existed:
- `gitlab/mod.rs` and `gitlab/fake.rs`, beside `bitbucket.rs`;
- `GitlabConnectionId`, beside `BitbucketConnectionId` (same shape);
- `GitlabRunnerMaterial`, a public type the brief pins in `protocol/agent.rs`;
- `migrations/0046_gitlab.sql`.

No parallel path was created.

## Architecture notes for A

- **New module:** `gridops_core::gitlab`, exposing the public trait `GitLabApi` (async-trait). Its fake sits behind the new `test-support` feature. No crate enables that feature yet: wave-1 issues add `features = ["test-support"]` to their `gridops-core` dev-dependency.
- **New core dependency:** `async-trait` (a workspace dep, already used by `gridops-agent` and `gridops-reconciler`).
- **Changed public interfaces:**
  - `ProvisioningPlan::for_selected_provider` takes a `CiPlatform` (D9);
  - new enum variants on `CiTarget`, `CiPlatform`, `TargetKind` and `DiagnosticCode`;
  - `CiTarget::is_bitbucket` semantics are now explicit. Behaviour is unchanged for GitHub and Bitbucket.
- **Schema:**
  - 3 new tables and 1 new column.
  - `fleet_ci_targets` and `runners` are rebuilt, keeping every row, index, trigger and FK, and the `pragma_foreign_key_list` of every referrer.
  - It is the first `-- no-transaction` migration.
- **Wire:** new `GET /v1/runners`-path request fields `gitlab` and `gitlabRunnerToken`. They are optional and skipped when absent, so older peers see no change.
- No handler, reconciler loop or HTTP client was added. `AppState` and `Reconciler` are untouched (R2-W3).

## Deviations from spec / wireframe

- No wireframe applies (no UI).
- The 7 new OpenAPI operations carry no `operationId`, matching the 56 existing operations, none of which has one. So `redocly lint` reports 7 more `operation-operationId` warnings (67, against 60 on `HEAD`), and lint still passes.
- Otherwise none. The brief's deviations D1 to D13 are implemented as written.

## Tier 1 self-check (incl. tests now GREEN)

    cargo fmt --all -- --check                                   -> clean
    cargo build --workspace --locked                             -> Finished
    cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl
                                                                 -> Finished, no warnings
    cargo +1.98.1 clippy -p gridops-core --all-targets --features test-support --locked -- -D warnings ...
                                                                 -> Finished, no warnings
    npx -y @redocly/cli@2.72.1 lint docs/openapi/gridops-v1.json -> "Your API description is valid." (67 warnings: 63 operation-operationId, 4 no-invalid-media-type-examples; HEAD has 60)

T's authored tests, now GREEN:

    gridops-core --test fleet_schema          14 passed; 0 failed   (6 new/extended GitLab tests)
    gridops-core --test gitlab_contract       14 passed; 0 failed
    gridops-core --test fleet_agent_outcomes   8 passed; 0 failed   (gitlab_runner_endings_round_trip_as_failure_diagnostics)
    gridops-core --test fleet_agent_commands   9 passed; 0 failed   (both guards)
    gridops-core --lib                       258 passed; 1 failed   (the 1 is db::tests::fresh_database_applies_every_migration, see "Tests that look wrong")
      incl. gitlab::fake::tests::* (6), admission_tests::gitlab_targets_are_stored_once_and_load_back_unchanged,
            provisioning::tests::{each_platform_gets_its_plan_on_both_providers, gitlab_runner_constants_are_pinned,
            gitlab_exit_reasons_map_each_exit_status_once, + the 2 updated}
    gridops-agent                             41 passed; 0 failed   (events_mask_credentials)
    gridops-api --bin gridops-api             60 passed; 0 failed   (both GitLab spec tests)
    gridops-tart-agent                        all passed            (the 3 GitLab tests)
    gridops-manager --bin                     94 passed; 11 failed  (the 3 GitLab tests pass; the 11 are environmental, see Known issues)

Whole workspace, `cargo test --workspace --no-fail-fast` (run to completion): 13 failures, every one accounted for:
- 1 is `db::tests::fresh_database_applies_every_migration`, a test count for T.
- 11 are the `gridops-manager` `SUN_LEN` tests.
- 1 is `gridops-api --test fleet_http` `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`. This workstation is a macOS ARM64 machine, so the preflight passes there. T recorded the same failure in Phase 4, before any change.

`gridops-reconciler` and `gridops-host-agent` are all green. An earlier summing pass, piped through `awk`, counted 14 failures. The complete run's list has 13, and the extra one wasn't reproduced or identified.

0046 on a scratch database (0001 to 0045, `sqlite3` 3.54.0): applied, `PRAGMA integrity_check` = `ok`, `PRAGMA foreign_key_check` empty, and all 10 `runners` indexes present.

## Evidence appendix

`docs/handoffs/137/evidence.md`. It covers:
- foreign keys on every connection;
- the three inbound `runners` FK actions;
- sqlx's `-- no-transaction` detection;
- the unchanged `is_bitbucket` callers in `PrepareEnvironment` and `static_compatibility`;
- the manager's re-serializing Tart forward.

## Tests that look wrong (for T)

- `crates/gridops-core/src/db.rs` `db::tests::fresh_database_applies_every_migration` (line 336, existing, not T-authored). It asserts `assert_eq!(tables, 70)`, and a fresh database now has **73** tables, because 0046 adds `gitlab_connections`, `runner_pool_gitlab_connections` and `gitlab_jobs`. The temp guard table is dropped before `COMMIT`. The expected count should become 73, and optionally `table_exists` assertions for the three new tables. This is test-only: no production change is needed.

## Known issues

- **11 `gridops-manager` tests fail on this macOS workstation, unrelated to #137:** the `provision_lock_tests`, `route_timeout_tests` and `tart_outage_tests` modules.
  - Each fails with `path must be shorter than SUN_LEN`. `stand_in_docker::start` binds a Unix socket at `temp_dir()/gridops-stand-in-docker-<uuid>.sock`, about 114 characters under the macOS `/var/folders/.../T/`, while the macOS limit is 104.
  - None of those modules, nor `stand_in_docker.rs`, is in this diff, and CI runs on Linux. T saw the same class of host-specific failure (`release_preflight`) in Phase 4.
- **`release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on this workstation**, unrelated to #137. The test host is itself a macOS ARM64 machine, so the preflight reports PASS where the test expects a failure. T recorded it in Phase 4, before any change.
- **The 0046 guard checks the whole database.** `count(*) FROM pragma_foreign_key_check`, as the brief pins, covers every table, not only the rebuilt ones. A production database that already holds a dangling reference anywhere (possible only if it was ever written with foreign keys off) would refuse to migrate and roll back rather than commit. That is the safe failure, but the operator would need to clean the row first. I left the statement as the brief and T's guard test pin it.

## Files changed

- `Cargo.lock`
- `migrations/0046_gitlab.sql` (new)
- `crates/gridops-core/Cargo.toml`
- `crates/gridops-core/src/lib.rs`
- `crates/gridops-core/src/gitlab/mod.rs` (new)
- `crates/gridops-core/src/gitlab/fake.rs` (new)
- `crates/gridops-core/src/fleet/domain.rs`
- `crates/gridops-core/src/fleet/mod.rs`
- `crates/gridops-core/src/fleet/capabilities.rs`
- `crates/gridops-core/src/fleet/targets.rs`
- `crates/gridops-core/src/fleet/admission.rs`
- `crates/gridops-core/src/fleet/protocol/agent.rs`
- `crates/gridops-core/src/fleet/protocol/outcomes.rs`
- `crates/gridops-core/src/provisioning.rs`
- `crates/gridops-api/src/resources.rs`
- `crates/gridops-reconciler/src/main.rs`
- `crates/gridops-manager/src/main.rs`
- `crates/gridops-tart-agent/src/main.rs`
- `crates/gridops-agent/src/session.rs`
- `docs/openapi/gridops-v1.json`
- Handoff artifacts: `docs/handoffs/137/handoff-F.md`, `docs/handoffs/137/evidence.md` (new), `docs/handoffs/137/decisions.md` (appended)
