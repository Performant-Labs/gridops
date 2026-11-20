# [GitLab: shared contract (types, schema, client trait + fake, agent protocol, API spec)](https://github.com/Performant-Labs/gridops/issues/137)

Part of #136. **Wave 0.** Every other GitLab issue builds on this one. Keep it to types, schema and stubs, so it can merge in a day.

## Need

The GitLab work splits into eight issues that run in parallel. They can only do that if the shared shapes exist first: the platform type, the tables, the client interface with a fake, the agent protocol and the API spec. This issue lands those and nothing else. **No behaviour.** Every function body outside the fake is `todo!()`, or the code isn't called yet.

## Scope

**Domain** (`crates/gridops-core/src/fleet/domain.rs`):
- `CiPlatform::Gitlab`.
- `GitlabConnectionId`, alongside `BitbucketConnectionId`.
- Two new `CiTarget` variants: `GitlabProject { connection_id, project_id }` and `GitlabGroup { connection_id, group_id }`.
- Make every exhaustive `match` compile. Where behaviour is needed, use an explicit `unimplemented` error that names the issue that owns it.

**Schema** (one migration, `0046_gitlab.sql`):
- `gitlab_connections` (`id`, `name`, `base_url`, `scope_kind` in `project`/`group`, `scope_id`, `scope_path`, `access_token_key`, `webhook_secret_key`, `created_by`, timestamps), with a unique index on `(base_url, scope_kind, scope_id)`.
- `runner_pool_gitlab_connections`, shaped like `runner_pool_bitbucket_connections`.
- `runner_pools.run_untagged` (integer, default 0).
- `runners.gitlab_connection_id` and `runners.gitlab_runner_id`, plus an index, and the `ci_platform` CHECK widened to include `'gitlab'`. SQLite can't alter a CHECK, so this is a table rebuild that keeps every index and FK.
- `gitlab_jobs` (`connection_id`, `project_id`, `job_id`, `pipeline_id`, `status`, `tag_list` JSON, `status_changed_at`, `retries_count`, `gitlab_runner_id`, `pool_id`, timestamps), with a unique index on `(connection_id, job_id)` and an index for unconcluded jobs like 0045's.

**Client interface** (`crates/gridops-core/src/gitlab/mod.rs`):
- An `#[async_trait] pub trait GitLabApi` with these methods:
  - `create_runner(scope, tags, run_untagged, description) -> CreatedRunner { id, token: SecretString }`
  - `delete_runner(id)`
  - `job(project_id, job_id) -> Job { id, project_id, pipeline_id, status, tag_list }`
  - `pending_jobs(project_id) -> Vec<Job>`
  - `resolve_scope(path) -> Scope`
  - `verify_access(scope)`
- A `GitLabError` enum with these variants: `Unauthorized`, `MissingScope(String)`, `MissingRole`, `NotFound`, `RateLimited { retry_after }`, `Upstream(String)`.
- `gitlab::fake::FakeGitLab`: a cloneable in-memory implementation that records calls and can be told to fail. It's behind `cfg(any(test, feature = "test-support"))` so other crates' tests can use it.

**Agent protocol** (`crates/gridops-core/src/fleet/protocol/agent.rs`):
- GitLab targets travel in `PrepareEnvironment.target`.
- The registration material on stdin is the `glrt-` token, with the base URL in `configuration`.
- New outcome codes: `IdleExpired` (the runner waited its timeout without a job) and `RegistrationRefused`.
- Add serde round-trip tests for the new variants.

**API spec** (`docs/openapi/`): the request and response schemas for `/api/v1/platform-connections/gitlab` (CRUD, verify, rotate webhook secret) and `POST /api/v1/webhooks/gitlab`, with no handlers yet. #138, #141 and #140 implement against this spec.

**Redaction:** add `glrt-` and `glpat-` patterns to secret redaction (`gridops-agent/src/session.rs`).

## Done when

- `cargo build`, `cargo clippy --all-targets` and the existing suites pass, and existing GitHub and Bitbucket rows survive the `runners` rebuild (tested on `tests/fixtures/fleet_ready.sql` and `fleet_legacy.sql`).
- `FakeGitLab` has its own tests.
- `redocly lint` passes.

## Clarifications (brief gate, round 1)

Everything above this heading is the issue text and is unchanged. This section adds facts verified in the repository and makes the scope and the done-when concrete. It removes nothing and narrows nothing.

### Schema: which table each column lives on (B-1)

- `run_untagged` is a column of **`runner_pools`** only: `runner_pools.run_untagged` (integer, default 0). `runners` gets no `run_untagged` column.
- `runners` gets exactly two new columns, `gitlab_connection_id` and `gitlab_runner_id`, plus one new index over them, and its `ci_platform` CHECK becomes `IN ('github','bitbucket','gitlab')`.
- Both new `runners` columns must be nullable, because every existing GitHub and Bitbucket row has no GitLab identity. The Bitbucket precedent they sit beside, from `migrations/0019_runner_pool_bitbucket_targets.sql`:

      ALTER TABLE `runners` ADD COLUMN `bitbucket_connection_id` text
        REFERENCES `bitbucket_connections`(`id`) ON UPDATE no action ON DELETE set null;
      ALTER TABLE `runners` ADD COLUMN `bitbucket_runner_uuid` text;
      CREATE INDEX `runners_bitbucket_connection_idx`
        ON `runners` (`pool_id`,`bitbucket_connection_id`,`status`);

### Schema: what the `runners` rebuild must recreate (B-5)

`runners` as it stands after migration 0045, taken from `sqlite_master` after applying `migrations/0001` to `0045` in order:

- Columns, in order: `id` (text PK), `pool_id` (text NOT NULL), `github_runner_id`, `container_id`, `container_name`, `name` (NOT NULL), `os` (default `'linux'`), `architecture` (default `'x64'`), `status` (default `'starting'`), `busy` (default false), `ephemeral` (default true), `current_job_id`, `failure_reason`, `last_heartbeat_at`, `registered_at`, `deleted_at`, `created_at`, `updated_at`, `last_job_id`, `configuration_version` (default 1, NOT NULL), `target_repository_id`, `provider` (default `'docker'`, CHECK `IN ('docker','tart')`), `ci_platform` (default `'github'`, CHECK `IN ('github','bitbucket')`), `bitbucket_connection_id`, `bitbucket_runner_uuid`, `runtime` (default `'vm'`, CHECK `IN ('vm','native')`).
- Outbound foreign keys `runners` holds (all must be recreated with the same actions):
  - `pool_id` -> `runner_pools(id)` ON DELETE cascade (0001)
  - `last_job_id` -> `workflow_jobs(id)` ON DELETE set null (0006)
  - `target_repository_id` -> `repositories(id)` ON DELETE set null (0012)
  - `bitbucket_connection_id` -> `bitbucket_connections(id)` ON DELETE set null (0019)
- Indexes on `runners` (all must be recreated):
  - `runners_container_id_unique` UNIQUE (`container_id`) (0001)
  - `runners_pool_status_idx` (`pool_id`,`status`) (0001)
  - `runners_last_job_idx` (`last_job_id`) (0006)
  - `runners_pool_configuration_idx` (`pool_id`,`configuration_version`) (0007)
  - `runners_target_repository_idx` (`pool_id`,`target_repository_id`,`status`) (0012)
  - `runners_github_id_idx` (`github_runner_id`) (0013; 0013 dropped the old `runners_github_id_unique`, which must not come back)
  - `runners_bitbucket_connection_idx` (`pool_id`,`bitbucket_connection_id`,`status`) (0019)
  - `runners_name_idx` (`name`,`created_at`) (0031)
- No trigger or view references `runners`.
- Inbound foreign keys that point **at** `runners(id)`. The rebuild must leave every one of these rows and references intact:
  - `runner_events.runner_id` ON DELETE **cascade** (0001)
  - `log_streams.runner_id` ON DELETE set null (0001)
  - `workload_placements.runner_id`, no action (0022)
- The hazard, which `migrations/0020_native_macos_runtime.sql` records in its header comment: foreign keys are enabled on every connection (`crates/gridops-core/src/db.rs` sets `.foreign_keys(true)`, and so does the test harness), and `runner_events.runner_id` cascades on delete, so a plain `DROP TABLE runners` would delete the entire runner event history (and null `log_streams.runner_id`, and be refused or orphan rows for `workload_placements`). 0020 avoided a rebuild for exactly this reason, and no migration in `migrations/` has rebuilt a table before, so 0046 is the first. The migration must be written so none of the inbound rows change. How it does that (for example a migration that runs outside sqlx's transaction so `PRAGMA foreign_keys` can be switched, followed by `PRAGMA foreign_key_check`) is the implementer's choice, but the done-when below proves the result.
- No new index or FK is dropped from the list above to satisfy the rebuild.

### Done-when, made concrete (B-2, B-5)

The criterion "existing GitHub and Bitbucket rows survive the `runners` rebuild (tested on `tests/fixtures/fleet_ready.sql` and `fleet_legacy.sql`)" is exercised by this existing harness, `crates/gridops-core/tests/fleet_schema.rs`:

- `Database::legacy()` opens a WAL SQLite file with foreign keys on, runs `MIGRATOR.run_to(21, ...)`, then loads `crates/gridops-core/tests/fixtures/fleet_legacy.sql`.
- `Database::ready()` then runs every migration and loads `crates/gridops-core/tests/fixtures/fleet_ready.sql`.
- `additive_upgrade_preserves_every_legacy_row_index_and_trigger` snapshots every table's rows and every `sqlite_master` definition on the legacy database, runs all migrations, and asserts the rows are equal, every old schema definition is unchanged, `PRAGMA foreign_key_check` is empty, `PRAGMA integrity_check` is `ok`, the journal is still WAL and foreign keys are still on, and that a rerun replays nothing.

What the fixtures hold:

- `fleet_legacy.sql` has three `runners` rows: `legacy-runner` and `legacy-native` (GitHub, `github_runner_id` 100 and 101) and `legacy-bb-runner` (`ci_platform` `'bitbucket'`, `bitbucket_connection_id` `'legacy-bb'`). It also has the inbound rows the rebuild endangers: `runner_events` row `legacy-event` and `log_streams` row `legacy-log`, both with `runner_id` `'legacy-runner'`.
- `fleet_ready.sql` has **no** `runners` rows. It inserts the fleet graph (hosts, domains, backends, `fleet_ci_targets` including a `bitbucket_workspace` target, placements, allocations, commands) after the full migration run, so it proves the post-0046 schema still accepts that graph and its constraint tests in `fleet_schema.rs` still pass.

Because 0046 must change the `runners` definition, the existing test cannot pass unchanged; it has to be extended, not loosened, so that it still proves the criterion:

- The schema-unchanged assertion may exempt only `runners` and the `runners_*` indexes listed above. For those, the test asserts instead that after migration `ci_platform` accepts `'gitlab'`, that every column listed above still exists with its default and CHECK, and that every index and every outbound and inbound FK listed above is present (via `pragma_index_list('runners')` and `pragma_foreign_key_list`).
- The row-equality assertion stays exact for every table except `runners`. For `runners`, each of the three legacy rows compares equal on every pre-0046 column, and its two new GitLab columns are NULL. `runner_events` and `log_streams` stay under the exact comparison, so the cascade and set-null hazard above is caught.
- `PRAGMA foreign_key_check` stays empty and `PRAGMA foreign_keys` is still 1 after the run.

### Schema: the shapes the new tables follow (NV-1, NV-2)

`runner_pool_bitbucket_connections`, from `migrations/0019_runner_pool_bitbucket_targets.sql`, which `runner_pool_gitlab_connections` mirrors with `gitlab_connections` in place of `bitbucket_connections`:

    CREATE TABLE `runner_pool_bitbucket_connections` (
      `pool_id` text NOT NULL,
      `connection_id` text NOT NULL,
      `created_at` integer NOT NULL,
      PRIMARY KEY (`pool_id`,`connection_id`),
      FOREIGN KEY (`pool_id`) REFERENCES `runner_pools`(`id`) ON UPDATE no action ON DELETE cascade,
      FOREIGN KEY (`connection_id`) REFERENCES `bitbucket_connections`(`id`) ON UPDATE no action ON DELETE cascade
    );
    CREATE INDEX `runner_pool_bitbucket_connections_connection_idx`
      ON `runner_pool_bitbucket_connections` (`connection_id`,`pool_id`);

The unconcluded-jobs index `gitlab_jobs` follows, from `migrations/0045_workflow_jobs_unconcluded_index.sql`: a partial index ordered oldest first, holding only jobs that finished without a conclusion, so the reconciler's check does not scan every job ever recorded:

    CREATE INDEX workflow_jobs_unconcluded_created_idx ON workflow_jobs (created_at, id)
      WHERE status='completed' AND conclusion IS NULL;

### Domain: the exhaustive matches that must compile (NV-5, NIT-1, NIT-2)

- `BitbucketConnectionId` is a newtype over `UpstreamText` (a validated string), serialised as a plain string (`#[serde(try_from = "String", into = "String")]`), with `new`, `as_str`, `TryFrom<String>` and `From<BitbucketConnectionId> for String`. `GitlabConnectionId` sits alongside it, so it has the same shape. `CiTarget` is `#[serde(tag = "platform", rename_all = "snake_case")]` with `deny_unknown_fields`, and its GitHub ids are `ExternalId` (a `NonZeroU64` at most `i64::MAX`).
- `CiPlatform` has no `match` site today; it is defined in `domain.rs` and re-exported from `crates/gridops-core/src/fleet/mod.rs`.
- Exhaustive `match` sites over `CiTarget` outside tests:
  - `CiTarget::canonical_key` in `crates/gridops-core/src/fleet/domain.rs`.
  - `impl From<&CiTarget> for TargetKind` in `crates/gridops-core/src/fleet/capabilities.rs` (`TargetKind` has only GitHub and Bitbucket variants today).
- Non-exhaustive sites that would silently misclassify GitLab rather than fail to compile, and so must be changed too:
  - `CiTarget::is_bitbucket` in `domain.rs` is written as `!self.is_github()`, so a GitLab target would report as Bitbucket. It must test the Bitbucket variants explicitly.
  - Its callers in `PrepareEnvironment` validation in `crates/gridops-core/src/fleet/protocol/agent.rs` (the persistent-mode and Windows-ARM64 rules for Bitbucket) and `TargetKind::is_bitbucket` callers in `capabilities.rs`.
  - The target-row parser in `crates/gridops-core/src/fleet/admission.rs` matches on the stored `kind` string and ends in `_ => Err(FleetError::Invariant("invalid target kind"))`, so it compiles unchanged; GitLab kinds fall into that arm until their owning issue adds them.
- The two mechanisms in the issue text apply to different places: a function body that nothing calls yet is `todo!()`; a `match` arm on a path that existing code can reach returns an error (not a panic) whose message names the GitLab issue that owns the behaviour.

### Agent protocol: verified facts (W-3 context)

- `PrepareEnvironment` and `ExecutionConfiguration` are both `#[serde(deny_unknown_fields)]`, and `ExecutionConfiguration` today holds only `network`, `session_id`, `requires_interactive` and `requires_docker_socket`. Carrying the base URL "in `configuration`" therefore means adding a field to `ExecutionConfiguration` (its doc comment requires each new field to have an explicit backend interpretation).
- The outcome code enums are not in `agent.rs`; they are `FailureCode`, `UncertainCode` and `DiagnosticCode` in `crates/gridops-core/src/fleet/protocol/outcomes.rs`, carried by `CommandOutcome`. None has a catch-all variant, so a peer built before this change rejects a payload carrying `idle_expired` or `registration_refused`.

### API spec: what exists and what lint checks (NV-4)

- `docs/openapi/gridops-v1.json` is OpenAPI `3.1.0`. It has no `/api/v1/platform-connections/...` or `/api/v1/webhooks/...` path today; the GitLab paths are new. Component schemas are PascalCase (`ErrorResponse`, `HostSummary`, ...), errors use `"default": {"$ref": "#/components/responses/ErrorResponse"}`, and the security schemes are `bearerAuth`, `cookieAuth` and `hostCredential`, with document-level security `cookieAuth`.
- `redocly lint` is `npm run validate:openapi` (`redocly lint docs/openapi/gridops-v1.json`, `@redocly/cli` 2.72.1), and `redocly.yaml` extends only `minimal`: structure, not style.
- Style is held by the tests in `crates/gridops-api/src/fleet/openapi.rs`, which are part of "the existing suites pass":
  - `the_spec_matches_the_token_scope_table_in_both_directions`: any operation whose `security` lists `bearerAuth` with a scope must appear in `TOKEN_SCOPED_ROUTES`, and the reverse. Since this issue adds no handlers, the GitLab operations must not declare a scoped `bearerAuth` requirement unless the same change adds matching `TOKEN_SCOPED_ROUTES` entries.
  - `the_spec_is_structurally_sound_openapi_31`: every operation has `responses`, every security requirement names a defined scheme, and every `$ref` resolves inside the document.

### Redaction: the existing mechanism (NV-3)

`crates/gridops-agent/src/session.rs` redacts by a fixed prefix list. `AgentEvent::new` passes `title` and `detail` through `redact`, which splits the text into runs of ASCII letters, digits, `_` and `-`, and replaces a run that starts with a listed prefix and has at least `SECRET_MIN_LEN` (16) characters after the prefix with the prefix plus `[redacted]`. The list today (one entry per line in the source, reflowed here):

    const SECRET_PREFIXES: [&str; 10] = [
        "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "sk-", "AKIA", "xoxb-", "glpat-",
    ];

`glpat-` is already present. The change is therefore to add `glrt-` (the array length becomes 11) and to extend the `events_mask_credentials` test so a `glrt-` token and a `glpat-` token are both masked. A whole token is masked, not just its prefix, because the run extends to the first character outside that set.


## Source material read
- crates/gridops-core/src/fleet/domain.rs
- migrations/0046_gitlab.sql (not found)
- migrations/0019_runner_pool_bitbucket_targets.sql
- migrations/0045_workflow_jobs_unconcluded_index.sql
- crates/gridops-core/src/gitlab/mod.rs (not found)
- crates/gridops-core/src/fleet/protocol/agent.rs
- docs/openapi/gridops-v1.json
- crates/gridops-agent/src/session.rs
- crates/gridops-core/tests/fixtures/fleet_ready.sql
- crates/gridops-core/tests/fixtures/fleet_legacy.sql
- crates/gridops-core/tests/fleet_schema.rs
- crates/gridops-core/src/db.rs
- crates/gridops-core/src/fleet/protocol/outcomes.rs
- crates/gridops-core/src/fleet/capabilities.rs
- crates/gridops-core/src/fleet/admission.rs (target-row parser)
- crates/gridops-core/src/fleet/authorization.rs (target use, tests only)
- crates/gridops-core/src/fleet/mod.rs (re-exports)
- crates/gridops-api/src/fleet/openapi.rs
- redocly.yaml
- package.json (`validate:openapi` script)
- migrations/0001_initial.sql (`runners`, `runner_events`, `log_streams`)
- migrations/0006_runner_job_history.sql
- migrations/0007_runner_pool_configuration_versions.sql
- migrations/0012_multi_repository_pools.sql
- migrations/0013_runner_identity_scope.sql
- migrations/0017_multi_provider_pools.sql
- migrations/0020_native_macos_runtime.sql
- migrations/0022_fleet_foundation.sql (`workload_placements.runner_id`)
- migrations/0031_runner_name_index.sql
- migrations/0001 to 0045 applied in order to a scratch SQLite database, to read the live `runners` definition, its indexes and the FKs pointing at it
