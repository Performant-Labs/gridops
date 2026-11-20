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


## Amendments (architecture review, round 1)

The Phase-3 review (`docs/handoffs/137/handoff-A.md`) blocked the plan on three contract-shape problems and raised eight warnings. This section resolves every one of them. **Where it conflicts with anything above, this section wins.** Each place it departs from the issue text is marked **[deviation]** and is listed again, with the wave-1 issue it affects, in the last subsection. The PR description and a comment on #136 repeat that list.

### A-1 (block): GitLab fleet targets get storage in 0046

Option (a) of the review: `0046_gitlab.sql` also rebuilds `fleet_ci_targets`, so a GitLab `CiTarget` can be stored, granted, admitted and loaded into `PrepareEnvironment.target`.

- New columns, all nullable: `gitlab_connection_id TEXT REFERENCES gitlab_connections(id)`, `gitlab_project_id INTEGER`, `gitlab_group_id INTEGER`. They come after `repository_uuid`, in that order.
- The `gitlab_connection_id` FK has no `ON DELETE` clause, so its action is SQLite's default, NO ACTION. That is the same as the Bitbucket column beside it, which 0022 declares as `bitbucket_connection_id TEXT REFERENCES bitbucket_connections(id)`. The other two actions are impossible here (brief review R1, B-3):
  - SET NULL would break the shape CHECK, which requires the column to be non-NULL for both GitLab kinds.
  - CASCADE would delete the target row, but every inbound FK of `fleet_ci_targets` is plain NO ACTION, so that delete is refused while a grant, intent, placement, event or observation references the target.
  - **Consequence:** while a `fleet_ci_targets` row references a GitLab connection, a `DELETE FROM gitlab_connections` for that connection fails with an FK error. This holds whether or not a pool targets it. Bitbucket behaves the same way today: `delete_bitbucket_connection` in `crates/gridops-api/src/resources.rs` checks pool memberships and live runners before deleting, but it doesn't check fleet targets.
  - #138's DELETE handler must check for a referencing `fleet_ci_targets` row (`WHERE gitlab_connection_id=?`) before it deletes, and refuse with the same 409 it returns for pools. The spec's 409 for that operation covers both cases (see W-7).
  - This contract adds no way to delete a fleet target.
- `kind` CHECK becomes `IN ('github_repository','github_organization','bitbucket_workspace','bitbucket_repository','gitlab_project','gitlab_group')`.
- The per-kind shape CHECK keeps its four arms. Each existing arm also requires `gitlab_connection_id IS NULL AND gitlab_project_id IS NULL AND gitlab_group_id IS NULL`. Two arms are added in the same style:
  - `kind='gitlab_project'`: `gitlab_connection_id` NOT NULL, `gitlab_project_id` NOT NULL and > 0, `gitlab_group_id` NULL, and every GitHub and Bitbucket column NULL.
  - `kind='gitlab_group'`: `gitlab_connection_id` NOT NULL, `gitlab_group_id` NOT NULL and > 0, `gitlab_project_id` NULL, and every GitHub and Bitbucket column NULL.
  - "Every GitHub and Bitbucket column" means exactly the six columns 0022 defines besides `id`, `kind` and `canonical_key`. The GitHub ones are `installation_id`, `repository_id` and `organization_id`; the Bitbucket ones are `bitbucket_connection_id`, `workspace_uuid` and `repository_uuid`. The existing four arms, verbatim from `migrations/0022_fleet_foundation.sql`, each of which gains the three GitLab `IS NULL` terms:

        (kind = 'github_repository' AND installation_id IS NOT NULL AND installation_id > 0 AND repository_id IS NOT NULL AND repository_id > 0 AND organization_id IS NULL AND bitbucket_connection_id IS NULL AND workspace_uuid IS NULL AND repository_uuid IS NULL)
        OR (kind = 'github_organization' AND installation_id IS NOT NULL AND installation_id > 0 AND organization_id IS NOT NULL AND organization_id > 0 AND repository_id IS NULL AND bitbucket_connection_id IS NULL AND workspace_uuid IS NULL AND repository_uuid IS NULL)
        OR (kind = 'bitbucket_workspace' AND bitbucket_connection_id IS NOT NULL AND workspace_uuid IS NOT NULL AND repository_uuid IS NULL AND installation_id IS NULL AND repository_id IS NULL AND organization_id IS NULL)
        OR (kind = 'bitbucket_repository' AND bitbucket_connection_id IS NOT NULL AND workspace_uuid IS NOT NULL AND repository_uuid IS NOT NULL AND installation_id IS NULL AND repository_id IS NULL AND organization_id IS NULL)
- The table stays `STRICT`, `canonical_key` stays `NOT NULL UNIQUE`, and the trigger `fleet_ci_targets_immutable_identity` (BEFORE UPDATE, RAISE ABORT 'CI target identity is immutable') is recreated word for word. Here it is verbatim from `migrations/0022_fleet_foundation.sql`:

      CREATE TRIGGER fleet_ci_targets_immutable_identity
      BEFORE UPDATE ON fleet_ci_targets
      BEGIN SELECT RAISE(ABORT,'CI target identity is immutable'); END;

- These inbound foreign keys must keep every row and reference unchanged: `host_target_grants`, `host_session_target_grants`, `profile_ci_targets`, `workload_intents`, `workload_placements`, `fleet_events` (nullable `target_id`) and `external_runner_observations`. All of them are plain `REFERENCES fleet_ci_targets(id)` from 0022. No migration after 0022 touches the table (`grep fleet_ci_targets migrations/*.sql` matches only 0022). The seven clauses, at lines 101, 142, 175, 219, 246, 395 and 407 of 0022, are `target_id TEXT NOT NULL REFERENCES fleet_ci_targets(id),`, except that the `fleet_events` one at line 395 is `target_id TEXT REFERENCES fleet_ci_targets(id),`.
- Canonical keys, persisted UNIQUE and immutable, and mirroring Bitbucket's format: `gitlab:project:{connection_id}:{project_id}` and `gitlab:group:{connection_id}:{group_id}`, with the decimal id. `CiTarget::canonical_key` returns these. It is infallible, so the "return an error" rule doesn't apply.
- `TargetKind` (`fleet/capabilities.rs`) gains `GitlabProject` and `GitlabGroup` (wire `gitlab_project`, `gitlab_group`) and a `const fn is_gitlab(self) -> bool`. `From<&CiTarget> for TargetKind` maps the two new variants to them. This is what the host agents of #147 and #142 advertise in `SupportedRunner`.
  - Wire format (brief review R1, B-6). `TargetKind` is a plain unit-variant enum, and its serde attributes stay exactly as they are today. It is not tagged. Each variant serializes as a bare snake_case string, so the two new variants are `"gitlab_project"` and `"gitlab_group"`, the same strings as the `CiTarget` `platform` tag values and the `fleet_ci_targets.kind` values. The definition today, verbatim:

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum TargetKind {
            GithubRepository,
            GithubOrganization,
            BitbucketWorkspace,
            BitbucketRepository,
        }

  - `SupportedRunner` (`#[serde(deny_unknown_fields)] struct { target_kind: TargetKind, mode: RunnerMode }`) is unchanged, so a GitLab advertisement is `{"target_kind":"gitlab_project","mode":...}`.
  - `TargetKind::is_bitbucket` is already explicit (`matches!(self, Self::BitbucketWorkspace | Self::BitbucketRepository)`), so it needs no change.
  - A serde test pins both new wire strings.
- Storage glue, so the round trip works and is tested in this issue:
  - `fleet::targets::ensure_target` accepts the two GitLab variants and inserts the matching row. It still refuses Bitbucket with the existing error.
    - Signature (brief review R1, B-9), unchanged from today: `pub async fn ensure_target(pool: &SqlitePool, target: &CiTarget) -> Result<String, FleetError>`. The `Ok` value is the `fleet_ci_targets.id` of the row for `target.canonical_key()`. That id is a UUID v4 string made with `Uuid::new_v4().to_string()` on first insert.
    - It is idempotent. The insert is `... ON CONFLICT(canonical_key) DO NOTHING`, followed by `SELECT id FROM fleet_ci_targets WHERE canonical_key=?`, so a second call for the same target returns the first id.
    - For a GitLab variant it binds `kind` (`gitlab_project` or `gitlab_group`), `canonical_key`, `gitlab_connection_id` (`connection_id.as_str()`) and either `gitlab_project_id` or `gitlab_group_id`. All six GitHub and Bitbucket columns stay NULL.
    - A Bitbucket variant (`BitbucketWorkspace` or `BitbucketRepository`) still returns exactly today's error, `Err(FleetError::InvalidRequest("only GitHub targets are synced"))`, and writes nothing. Its `&'static str` is kept word for word.
    - An id that doesn't fit `i64` returns `FleetError::InvalidRequest` with a field name (`"project"` or `"group"`), the way today's code uses `"installation"`, `"repository"` and `"organization"`. `ExternalId` caps ids at `i64::MAX`, so this can't happen in practice.
    - `FleetError` is the enum in `crates/gridops-core/src/fleet/service.rs`, and this contract doesn't change it.
  - `fleet::admission::target_from_db` parses `gitlab_project` and `gitlab_group` rows back into the `CiTarget` variants.
  - Syncing a pool's GitLab connections into fleet targets (the GitLab counterpart of the GitHub pool-target sync at the top of `fleet/targets.rs`) is behaviour, and it belongs to **#139**. **[deviation: #139 gains this ownership]**

### A-2 (block): the base URL travels in the bootstrap material, not in `ExecutionConfiguration`

**[deviation]** The issue says "with the base URL in `configuration`". `ExecutionConfiguration` is **not** changed. It is profile-derived runtime configuration and `deny_unknown_fields`, so a new field would break older host agents on every `PrepareEnvironment`. The registration URL already travels next to the token in the sealed bootstrap material (`github_registration { token, url, ... }`, produced in `gridops-reconciler/src/fleet.rs` and parsed by the private `Material` enums in the host agents).

The GitLab material is pinned as a public type in `crates/gridops-core/src/fleet/protocol/agent.rs`:

    /// Bootstrap material for a GitLab runner, sealed by the reconciler (#139)
    /// with `placement::store_bootstrap` and opened by the host agents
    /// (#147 Linux Docker, #142 Tart/macOS). `token` goes to the runner's stdin;
    /// `url` is not secret and may be passed as an argument.
    #[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    pub enum GitlabRunnerMaterial {
        GitlabRunner {
            token: String, // the `glrt-` runner authentication token
            url: String,   // the connection's base_url, e.g. https://gitlab.com
        },
    }

- The JSON is exactly `{"kind":"gitlab_runner","token":"glrt-…","url":"https://gitlab.example.com"}`. There are no other fields.
- Why it is an enum with one variant and not a struct (brief review R1, B-4): `#[serde(tag = "kind", rename = "gitlab_runner", deny_unknown_fields)]` on a struct serializes the literal above, but it **can't deserialize its own output**. With `deny_unknown_fields`, the struct's deserializer rejects `kind` as an unknown field. Without `deny_unknown_fields`, it ignores `kind`, so a payload with another `kind` would be accepted. The internally tagged single-variant enum above was checked against the workspace's locked `serde` 1.0.229 and `serde_json` 1.0.151 in a scratch crate:
  - It serializes as the literal above.
  - It accepts the literal, with the fields in any order.
  - It rejects a payload with no `kind` ("missing field `kind`").
  - It rejects another `kind` ("unknown variant `github_registration`, expected `gitlab_runner`").
  - It rejects an extra field ("unknown field `extra`").
  - Consumers read the fields with `let GitlabRunnerMaterial::GitlabRunner { token, url } = material;`. That pattern is irrefutable while the enum has one variant.
- `Debug` is implemented by hand and prints `token: "[redacted]"`.
- Tests: the serialized string equals the literal above; it round-trips; a payload with an extra field is rejected; a payload with another `kind` is rejected; and `format!("{:?}")` doesn't contain the token.
- A test also pins that a GitHub `PrepareEnvironment` fixture serializes byte for byte as it does on `master`, which proves `ExecutionConfiguration` is untouched.
  - The fixture is the existing `fixture()` in `crates/gridops-core/tests/fleet_agent_commands.rs`, an envelope whose operation is `prepare_environment` with target `{"platform":"github_repository","installation_id":1,"repository_id":10}` and `"configuration":{}`. The new test goes in the same file.
  - The expected bytes are a string literal in the test: the output of `serde_json::to_string(&command(fixture())?)`, captured on `master` before any change in this issue. The literal is checked in as captured and is not edited by hand.

### A-3 (block): `GitLabApi` takes credentials on every call, and `Scope` is defined

One `Arc<dyn GitLabApi>` serves every connection, the way `BitbucketClient` takes the token on each call. #138 injects it into `AppState`, and #139/#148 into the reconciler, next to `bitbucket`. Everything below lives in `crates/gridops-core/src/gitlab/mod.rs`. It uses the existing workspace deps `async-trait`, `secrecy`, `url` and `thiserror`, and `crates/gridops-core/Cargo.toml` gains `async-trait.workspace = true`.

    pub struct GitLabCredentials { pub base_url: Url, pub access_token: SecretString }
    // Debug prints base_url and "[redacted]" for the token.

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ScopeKind { Project, Group }          // as_str(): "project" | "group" (= gitlab_connections.scope_kind)

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Scope {
        Project { id: ExternalId, path: String },  // id = scope_id, path = scope_path ("group/sub/project")
        Group { id: ExternalId, path: String },
    }
    // Scope::kind() -> ScopeKind, Scope::id() -> ExternalId, Scope::path() -> &str,
    // Scope::target(connection_id: GitlabConnectionId) -> CiTarget
    //   (Project -> CiTarget::GitlabProject { project_id: id }, Group -> CiTarget::GitlabGroup { group_id: id })

    pub struct CreatedRunner { pub id: ExternalId, pub token: SecretString }  // Debug redacts token

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Job {
        pub id: ExternalId, pub project_id: ExternalId, pub pipeline_id: ExternalId,
        pub status: JobStatus, pub tag_list: Vec<String>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum JobStatus {
        Created, WaitingForResource, Preparing, Pending, Running,
        Success, Failed, Canceled, Skipped, Manual, Scheduled,
        #[serde(other)] Unknown,
    }
    // JobStatus::is_terminal(): true exactly for Success | Failed | Canceled | Skipped.
    // as_str() gives the GitLab spelling stored in gitlab_jobs.status.

    #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
    pub enum GitLabError {
        Unauthorized,
        MissingScope(String),                       // the missing token scope, e.g. "create_runner"
        MissingRole,
        NotFound,
        RateLimited { retry_after: Option<Duration> },
        Upstream(String),
    }

    #[async_trait]
    pub trait GitLabApi: Send + Sync {
        async fn create_runner(&self, credentials: &GitLabCredentials, scope: &Scope, tags: &[String],
                               run_untagged: bool, description: &str) -> Result<CreatedRunner, GitLabError>;
        async fn delete_runner(&self, credentials: &GitLabCredentials, runner_id: ExternalId) -> Result<(), GitLabError>;
        async fn job(&self, credentials: &GitLabCredentials, project_id: ExternalId, job_id: ExternalId) -> Result<Job, GitLabError>;
        async fn pending_jobs(&self, credentials: &GitLabCredentials, project_id: ExternalId) -> Result<Vec<Job>, GitLabError>;
        async fn group_projects(&self, credentials: &GitLabCredentials, group_id: ExternalId) -> Result<Vec<ExternalId>, GitLabError>;
        async fn resolve_scope(&self, credentials: &GitLabCredentials, kind: ScopeKind, path: &str) -> Result<Scope, GitLabError>;
        async fn verify_access(&self, credentials: &GitLabCredentials, scope: &Scope) -> Result<(), GitLabError>;
    }

- **[deviation]** Every method gains a leading `credentials: &GitLabCredentials` argument.
- **[deviation]** `resolve_scope` also takes `kind: ScopeKind`. The operator chooses project or group (#141), and #146 calls `/projects/:path` or `/groups/:path` from it.
- **[deviation]** There is a new method, `group_projects`. GitLab has no group-level jobs list, so #148 polls a group connection by listing its projects and then calling `pending_jobs` on each. #146 implements it as `GET /groups/:id/projects?include_subgroups=true`, paginated.
- `RateLimited.retry_after` is `Option<Duration>`, taken from `Retry-After`. Every id is `ExternalId`. `tag_list` is `Vec<String>`.
- No function body outside the fake is `todo!()`: the trait has no default methods, and the real implementation is #146's.

**`gitlab::fake::FakeGitLab`** (`crates/gridops-core/src/gitlab/fake.rs`), behind `#[cfg(any(test, feature = "test-support"))]`. `crates/gridops-core/Cargo.toml` gains `[features] test-support = []`.

- It is `Clone` (shared `Arc<Mutex<_>>` state), `Default`, and implements `GitLabApi`.
- Its state is keyed by `base_url`, so two connections on different instances stay apart.
  - What that means precisely (brief review R1, B-7 and W-8). An "instance" is one GitLab server, identified by `GitLabCredentials.base_url` and compared by `Url` equality. The fake keys **all** of its state by `base_url` alone: scopes, jobs, group projects, runners, the runner-id counter and the queued failures. `access_token` isn't part of the key.
  - Two connections with different base URLs never see each other's state.
  - Two connections on the **same** base URL with different access tokens share one instance's state, as two tokens on one real GitLab server do. A runner one creates is listed for, and can be deleted by, the other.
  - The fake never inspects the token. It doesn't model token-level authorization. To simulate a refused token, a test queues `fail_next(base_url, method, GitLabError::Unauthorized)` (or `MissingScope`/`MissingRole`), and that failure applies to the next call on that base URL whatever the token.
  - `FakeCall` holds no credential identity (see Recording below), so a test can't use `calls()` to prove which token a call used. #138, #139, #140 and #148 tests that need two connections isolated must give them different base URLs.
- Setup methods:
  - `with_scope(base_url: &Url, scope: Scope)`, used by `resolve_scope` (looked up by kind and path) and by `verify_access`.
  - `with_job(base_url: &Url, job: Job)`, used by `job` and by `pending_jobs` (status `Pending`, same project).
  - `with_group_project(base_url: &Url, group_id, project_id)`.
- Failures:
  - `fail_next(base_url: &Url, method: FakeMethod, error: GitLabError)` queues a one-shot failure for that base URL and method. Queued failures for one method are returned first in, first out.
  - `FakeMethod` has one variant per trait method.
- Effects:
  - `create_runner` returns ids 1, 2, 3, … per base URL, and tokens `glrt-fake-runner-token-{id:04}`.
  - `delete_runner` removes the runner. `runners(base_url) -> Vec<FakeRunner { id, scope, tags, run_untagged, description }>` lists the live runners.
  - `delete_runner` on an unknown id returns `NotFound`.
  - `job` returns `NotFound` for an unknown job, and so does `resolve_scope` for an unknown path.
- Recording: `calls() -> Vec<FakeCall>`. `FakeCall { base_url: Url, method: FakeMethod }` records every call, failed calls included, in order. The access token is never recorded.
- Tests in `fake.rs` cover each method, the per-base-URL isolation, `fail_next` ordering, and the recorded calls.

### W-4: the outcome codes are `DiagnosticCode` variants

`IdleExpired` and `RegistrationRefused` are added to **`DiagnosticCode`** in `fleet/protocol/outcomes.rs` (wire `idle_expired`, `registration_refused`). `FailureCode` is unchanged, so `placement::reported_gone` still recognises an exited runner. The meaning, pinned for #147, #142 and #139:

- Runner waited its timeout without a job: the Inspect result is `Failed { code: ResourceUnavailable, retryable: false, diagnostic: Some({ code: IdleExpired, .. }) }`.
- GitLab refused the `glrt-` token: the start command's result is `Failed { code: Configuration, retryable: false, diagnostic: Some({ code: RegistrationRefused, .. }) }`.

There are serde round-trip tests for both codes, inside a `CommandOutcome::Failed`.

How consumers detect these cases (brief review R2, W-2): #147, #142 and #139 must branch on `diagnostic.code` (`IdleExpired` or `RegistrationRefused`), not on the `FailureCode`. `ResourceUnavailable` and `Configuration` are also used for other failures, so the failure code alone can't tell these two cases apart. Both carry `retryable: false`, so neither should be retried.

### W-5 and W-6: schema constraints, pinned

Statement order inside 0046:

1. `gitlab_connections`.
2. `runner_pool_gitlab_connections`.
3. `runner_pools.run_untagged`.
4. `gitlab_jobs`.
5. The `fleet_ci_targets` rebuild.
6. The `runners` rebuild.

The tables:

    CREATE TABLE `gitlab_connections` (
      `id` text PRIMARY KEY NOT NULL,
      `name` text NOT NULL,
      `base_url` text NOT NULL,                   -- normalised by #138: scheme+host[+port][+path], no trailing slash
      `scope_kind` text NOT NULL CHECK (`scope_kind` IN ('project','group')),
      `scope_id` integer NOT NULL CHECK (`scope_id` > 0),
      `scope_path` text NOT NULL,
      `access_token_key` text NOT NULL,
      `webhook_secret_key` text NOT NULL,
      `created_by` text,
      `created_at` integer NOT NULL,
      `updated_at` integer NOT NULL,
      FOREIGN KEY (`created_by`) REFERENCES `users`(`id`) ON UPDATE no action ON DELETE set null
    );
    CREATE UNIQUE INDEX `gitlab_connections_scope_unique` ON `gitlab_connections` (`base_url`,`scope_kind`,`scope_id`);
    CREATE UNIQUE INDEX `gitlab_connections_access_token_key_unique` ON `gitlab_connections` (`access_token_key`);
    CREATE UNIQUE INDEX `gitlab_connections_webhook_secret_key_unique` ON `gitlab_connections` (`webhook_secret_key`);

    -- runner_pool_gitlab_connections: the 0019 Bitbucket table word for word with
    -- gitlab_connections in place of bitbucket_connections, index
    -- runner_pool_gitlab_connections_connection_idx (connection_id, pool_id).

    ALTER TABLE runner_pools ADD COLUMN run_untagged INTEGER NOT NULL DEFAULT 0;

    CREATE TABLE `gitlab_jobs` (
      `id` text PRIMARY KEY NOT NULL,
      `connection_id` text NOT NULL REFERENCES `gitlab_connections`(`id`) ON DELETE cascade,
      `project_id` integer NOT NULL,
      `job_id` integer NOT NULL,
      `pipeline_id` integer NOT NULL,
      `status` text NOT NULL,                     -- JobStatus::as_str()
      `tag_list` text NOT NULL DEFAULT '[]',      -- JSON array of strings
      `status_changed_at` integer NOT NULL,
      `retries_count` integer NOT NULL DEFAULT 0,
      `gitlab_runner_id` integer,
      `pool_id` text REFERENCES `runner_pools`(`id`) ON DELETE set null,
      `created_at` integer NOT NULL,
      `updated_at` integer NOT NULL
    );
    CREATE UNIQUE INDEX `gitlab_jobs_connection_job_unique` ON `gitlab_jobs` (`connection_id`,`job_id`);
    CREATE INDEX `gitlab_jobs_open_created_idx` ON `gitlab_jobs` (`created_at`,`id`)
      WHERE `status` NOT IN ('success','failed','canceled','skipped');

- `gitlab_jobs.id` (brief review R1, B-1) is a surrogate key.
  - It is a UUID v4 string, `uuid::Uuid::new_v4().to_string()`, generated by whoever first records the job (#140, and #148 if it records a polled job first). This is the same way `bitbucket_connections.id` (`crates/gridops-api/src/resources.rs`) and `fleet_ci_targets.id` (`ensure_target`) are made.
  - It is not the GitLab job id. The GitLab job id is `job_id`, and `Job.id` from `GitLabApi` maps to `job_id`. Likewise `Job.project_id` maps to `project_id` and `Job.pipeline_id` to `pipeline_id`.
  - A job's identity is `(connection_id, job_id)`, which the unique index enforces. A writer that sees a job already recorded updates that row, for example with `INSERT ... ON CONFLICT(connection_id, job_id) DO UPDATE SET ...` that doesn't set `id`. So `id` never changes once written.
  - No table references `gitlab_jobs.id` in this contract.
- `gitlab_jobs.tag_list` encoding (brief review R1, W-4): `serde_json::to_string(&job.tag_list)` of the `Vec<String>`. That is compact JSON, an empty list is `[]` (the column default), and the tags stay in the order the GitLab API returned them. It is read back with `serde_json::from_str::<Vec<String>>`. #140 and #148 both use this encoding.
- The open-jobs index follows 0043's `workflow_jobs_open_created_idx`, not 0045. `gitlab_jobs` has no `conclusion` column. A job is concluded exactly when `JobStatus::is_terminal()`, and #140 and #148 both use that one rule.
  - It follows from that rule that `Created`, `WaitingForResource`, `Preparing`, `Pending`, `Running`, `Manual`, `Scheduled` and `Unknown` jobs are open and stay in the open-jobs index (NIT-4). Whether #148 acts on an open `Manual` or `Scheduled` job is #148's behaviour, not part of this contract.
- `runners` gets `gitlab_connection_id text REFERENCES gitlab_connections(id) ON UPDATE no action ON DELETE set null` and `gitlab_runner_id integer`, appended after `runtime`, and **two** new indexes:
  - `runners_gitlab_connection_idx (pool_id, gitlab_connection_id, status)`, the pool lookup, after 0019's.
  - `runners_gitlab_runner_id_idx (gitlab_runner_id)`, #140's busy lookup by GitLab runner id, after `runners_github_id_idx`.

### W-8: rebuild mechanics

0046 starts with sqlx's `-- no-transaction` directive and controls the transaction itself:

    PRAGMA foreign_keys=OFF;
    BEGIN;
      -- statements 1-6 above; each rebuild = CREATE TABLE x_new (...); INSERT INTO x_new (<old columns>) SELECT <old columns> FROM x;
      -- DROP TABLE x; ALTER TABLE x_new RENAME TO x; then every index and trigger of x recreated
      CREATE TEMP TABLE _gitlab_0046_fk_guard (violations INTEGER NOT NULL CHECK (violations = 0));
      INSERT INTO _gitlab_0046_fk_guard SELECT count(*) FROM pragma_foreign_key_check;
      DROP TABLE _gitlab_0046_fk_guard;
    COMMIT;
    PRAGMA foreign_keys=ON;

- Any dangling reference makes the guard's CHECK fail, so the migration errors before `COMMIT`.
- The guard works while `foreign_keys` is OFF (brief review R1, W-7). `PRAGMA foreign_key_check` is an explicit check, not enforcement, and it reports violations whatever the `foreign_keys` setting is. This was checked with SQLite 3.54.0. With `foreign_keys` OFF, one dangling child row gave `count(*) FROM pragma_foreign_key_check` = 1, and the guard's `INSERT` failed with `CHECK constraint failed: v=0`. `PRAGMA foreign_keys` itself is a no-op inside a transaction, which is why it sits outside `BEGIN`/`COMMIT`.
- `foreign_keys` is back on for the pooled connection once the migration finishes.
- A test asserts `PRAGMA foreign_keys` is 1 on a connection taken from the pool after migrating.
- A test proves the guard itself fails (brief review R2, W-3). In `crates/gridops-core/tests/fleet_schema.rs`, on a scratch SQLite database with `foreign_keys` OFF, it inserts one child row whose FK points at a missing parent. It then runs the three guard statements above (`CREATE TEMP TABLE _gitlab_0046_fk_guard ...`, the `INSERT ... SELECT count(*) FROM pragma_foreign_key_check`, `DROP TABLE`) and asserts that the `INSERT` fails with a CHECK constraint error. With no dangling row, the same statements succeed.

### Done-when, extended for A-1

On top of the `runners` criteria above:

- `fleet_schema.rs` may also exempt `fleet_ci_targets`, its autoindex and the `fleet_ci_targets_immutable_identity` trigger from the definition-unchanged assertion. For those, it asserts:
  - the new kinds and columns exist;
  - the trigger is present and still aborts an UPDATE;
  - the shape CHECK accepts a `gitlab_project` row and a `gitlab_group` row, and refuses a GitLab row with a GitHub column set and a `gitlab_project` row without `gitlab_project_id`.
- A new test migrates to 0045, loads `fleet_ready.sql`, snapshots every table, and then applies 0046. It asserts:
  - every row of every table is equal, with `fleet_ci_targets` rows gaining three NULL GitLab columns;
  - `PRAGMA foreign_key_check` is empty, `PRAGMA integrity_check` is `ok` and `foreign_keys` is 1;
  - `pragma_foreign_key_list` of each of the seven referencing tables is unchanged.

  `fleet_ready.sql`'s Bitbucket and GitHub targets survive, and that fixture carries the whole fleet graph that references targets.
- There is a round-trip test: `ensure_target` of a `CiTarget::GitlabProject` and of a `CiTarget::GitlabGroup`, then `target_from_db` of each row, returns the same `CiTarget`. Calling `ensure_target` a second time returns the same id.

### W-7: the OpenAPI entries

New paths in `docs/openapi/gridops-v1.json`. Every operation description starts with "Not served yet: implemented by #138" (or #140), because the spec is served live.

- The connection operations declare `security: [{ "cookieAuth": [] }]` only, with no `bearerAuth`. They are session routes, like Bitbucket's, so `TOKEN_SCOPED_ROUTES` is unchanged.
  - `GET /api/v1/platform-connections/gitlab`: 200 `GitlabConnectionList { canManage: boolean, items: GitlabConnection[] }`.
  - `POST /api/v1/platform-connections/gitlab`: body `CreateGitlabConnectionRequest { name, baseUrl (default "https://gitlab.com"), scopeKind ("project"|"group"), scopePath, accessToken }`. Responses: 201 `GitlabConnectionCreated { connection: GitlabConnection, webhookUrl, webhookSecret }`, the only response that carries the secret; 409 on a duplicate `(baseUrl, scope)`.
  - `PATCH /api/v1/platform-connections/gitlab/{connectionId}`: body `UpdateGitlabConnectionRequest { name?, accessToken? }`, response 200 `GitlabConnection`. PATCH is kept as #138 says, because it is a partial update. Bitbucket's PUT is a full replace.
  - `DELETE /api/v1/platform-connections/gitlab/{connectionId}`: 204, or 409 while a pool targets it.
    - The 409 also covers a GitLab `fleet_ci_targets` row that still references the connection, whether or not any pool does. That FK is NO ACTION (see A-1), so the delete couldn't succeed anyway. The 409 description says "while a pool or a fleet target references it".
  - `POST /api/v1/platform-connections/gitlab/{connectionId}/verify`: 204, or an error that names the missing scope or role.
  - `POST /api/v1/platform-connections/gitlab/{connectionId}/webhook-secret/rotate`: 200 `GitlabWebhookSecret { webhookUrl, webhookSecret }`.
  - `GitlabConnection` is `{ id, provider: "gitlab", name, baseUrl, scopeKind, scopeId (integer), scopePath, createdAt, updatedAt (RFC 3339 strings, as Bitbucket's iso()), canManage }`. It has no secret, no token and no webhook URL.
- `POST /api/v1/webhooks/gitlab` declares `security: []`, like `/api/v1/fleet/agent/enroll`.
  - Required header parameters: `X-Gitlab-Token` and `X-Gitlab-Event`.
  - The body is a free-form JSON object.
  - Responses: 202 on an accepted delivery, 401 when no connection's secret matches.
- Errors on these paths use a new `ApiErrorBody { error: string, code?: string }` component, the shape `gridops-api/src/error.rs` returns. They do not use the Fleet `ErrorResponse`.
- Component schemas are PascalCase, and the `openapi.rs` structural tests and `npm run validate:openapi` pass.

### Remaining scope notes

- `CiTarget::GitlabProject { connection_id: GitlabConnectionId, project_id: ExternalId }` and `CiTarget::GitlabGroup { connection_id: GitlabConnectionId, group_id: ExternalId }` serialize as `{"platform":"gitlab_project",...}` and `{"platform":"gitlab_group",...}`. `CiPlatform::Gitlab` serializes as `"gitlab"`.
- `CiTarget::is_gitlab()` is added. `is_bitbucket()` tests the Bitbucket variants explicitly.
- No Bitbucket-only validation rule in `PrepareEnvironment` applies to GitLab targets. Platform rules for GitLab are #147's and #142's.
- Out of scope: any handler, reconciler loop, HTTP client, host-agent code or UI.

### Deviations from the issue text (to repeat in the PR and on #136)

| # | Issue text | Contract as shipped | Wave-1 issues affected |
|---|---|---|---|
| D1 | "the base URL in `configuration`" | No `ExecutionConfiguration` change. The base URL is `url` in the sealed `GitlabRunnerMaterial` `{"kind":"gitlab_runner","token","url"}`. | #139, #147, #142 |
| D2 | `GitLabApi` methods without credentials | Every method takes `&GitLabCredentials { base_url, access_token }` first. A single `Arc<dyn GitLabApi>` serves every connection. | #146, #138, #139, #140, #148 |
| D3 | `resolve_scope(path)` | `resolve_scope(credentials, kind: ScopeKind, path)` | #146, #138 |
| D4 | (no method) | A new `group_projects(credentials, group_id)`, so group connections can be polled | #146, #148 |
| D5 | New outcome codes (owning enum unstated) | `DiagnosticCode::{IdleExpired, RegistrationRefused}` inside `Failed { ResourceUnavailable \| Configuration }` | #147, #142, #139 |
| D6 | Schema list without `fleet_ci_targets` | 0046 also rebuilds `fleet_ci_targets` with the `gitlab_project` and `gitlab_group` kinds, adds `TargetKind::Gitlab*`, and adds `ensure_target`/`target_from_db` arms. Pool-target sync for GitLab is #139's. | #139, #147, #142 |
| D7 | "an index" on the new `runners` columns | Two indexes: `runners_gitlab_connection_idx` and `runners_gitlab_runner_id_idx` | #139, #140 |
| D8 | Unconcluded-jobs index "like 0045's" | An open-jobs partial index like 0043's, `WHERE status NOT IN ('success','failed','canceled','skipped')` | #140, #148 |

## Amendments (brief review, round 1)

The outside brief review blocked on B-1, B-3, B-4, B-6, B-7 and B-9. Each one is now resolved in place above, marked "brief review R1":

| Finding | Where it is resolved |
|---|---|
| B-1 | W-5/W-6, under the `gitlab_jobs` table: what `id` is and how it is made |
| B-3 | A-1, under the new columns: the `gitlab_connection_id` FK action and its consequence; and W-7, the DELETE 409 |
| B-4 | A-2: `GitlabRunnerMaterial` becomes a single-variant internally tagged enum, with the same JSON |
| B-6 | A-1, under `TargetKind`: the serde attributes and wire strings |
| B-7 | A-3, under `FakeGitLab`: keyed by `base_url` alone, and what that means for tokens |
| B-9 | A-1, under the storage glue: the signature of `ensure_target`, its return value and its refusal error |

These changes add detail and don't loosen anything. No acceptance criterion, scope item or earlier decision is changed.

### Evidence for the review's needs-verification items

All of this was verified in this worktree.

- **NV-1.** `BitbucketConnectionId` in `crates/gridops-core/src/fleet/domain.rs`:

      #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
      #[serde(try_from = "String", into = "String")]
      pub struct BitbucketConnectionId(UpstreamText);

  It has `new(value: impl Into<String>) -> Result<Self, FleetTextError>`, `as_str(&self) -> &str`, `impl TryFrom<String>` (calls `new`) and `impl From<BitbucketConnectionId> for String`.
- **NV-2.** `CiPlatform` is `#[serde(rename_all = "snake_case")] pub enum CiPlatform { Github, Bitbucket }` in `domain.rs`. Searching `crates/` for `CiPlatform` finds only that definition and the re-export in `crates/gridops-core/src/fleet/mod.rs`, so it has no `match` site.
- **NV-3.**
  - `redocly.yaml` is, apart from its two comment lines, exactly `extends: [minimal]`.
  - `package.json` has `"validate:openapi": "redocly lint docs/openapi/gridops-v1.json"` and `"@redocly/cli": "2.72.1"`.
- **NV-4.** `crates/gridops-core/tests/fixtures/fleet_ready.sql` contains no `runners` text at all, so it has no `runners` rows.
- **NV-5.** The header comment of `migrations/0020_native_macos_runtime.sql` says, in part, that widening the `provider` CHECK "would require rebuilding `runners`", and that because foreign keys are enabled and `runner_events.runner_id` cascades on delete, a `DROP TABLE` would take the runner event history with it.
- **NV-6 and NV-7.** Quoted verbatim in A-1 above.
- **NV-8.**
  - `target_from_db` in `crates/gridops-core/src/fleet/admission.rs` matches `kind` with the arms `"github_repository"`, `"github_organization"` and `"bitbucket_workspace" | "bitbucket_repository"`.
  - It ends with `_ => Err(FleetError::Invariant("invalid target kind")),`.
- **NV-9.**
  - `domain.rs` defines `pub const fn is_bitbucket(&self) -> bool { !self.is_github() }`.
  - Its callers are the two `prepare.target.is_bitbucket()` conditions in the `PrepareEnvironment` check in `crates/gridops-core/src/fleet/protocol/agent.rs`: `mode != RunnerMode::Persistent`, and `execution_os == ExecutionOs::Windows && architecture == Architecture::Arm64`.
  - `capabilities.rs` calls `TargetKind::is_bitbucket` (already explicit) in two places.
- **NV-12.** Every `/api/v1/...` path in `docs/openapi/gridops-v1.json` starts with `/api/v1/fleet`, `/api/v1/installations`, `/api/v1/operations`, `/api/v1/overview` or `/api/v1/runner-pools`. None starts with `/api/v1/platform-connections` or `/api/v1/webhooks`.
- **NV-13.** The `SECRET_PREFIXES` list is quoted in "Redaction" above. In `crates/gridops-agent/src/session.rs`:
  - `const SECRET_MIN_LEN: usize = 16;`
  - `fn redact` treats a char as part of a token when `c.is_ascii_alphanumeric() || c == '_' || c == '-'`. It replaces a run that starts with a listed prefix and has at least `SECRET_MIN_LEN` characters after it with the prefix followed by `[redacted]`.
  - The fake's token `glrt-fake-runner-token-0001` has 22 characters after `glrt-`, so it is masked too.
- **NV-14.** Settled by B-4 above, which tested it against the locked serde versions.

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
- docs/handoffs/137/handoff-A.md (architecture review, round 1)
- migrations/0022_fleet_foundation.sql (`fleet_ci_targets`, its trigger and the seven referencing tables)
- migrations/0018_bitbucket_connections.sql
- migrations/0043_workflow_job_attempts.sql
- crates/gridops-core/src/fleet/targets.rs (`ensure_target`)
- crates/gridops-core/src/fleet/placement.rs (`reported_gone`)
- crates/gridops-host-agent/src/docker.rs (`Material`)
- crates/gridops-reconciler/src/fleet.rs (material production)
- crates/gridops-core/src/bitbucket.rs (per-call token)
- crates/gridops-api/src/main.rs and resources.rs (Bitbucket routes and JSON)
- crates/gridops-api/src/error.rs (`ApiError` body)
- issues #146 #138 #139 #147 #140 #148 #142 #141 (names they depend on)
- docs/handoffs/137/decisions.md (brief review round 1)
- crates/gridops-core/src/fleet/service.rs (`FleetError`)
- crates/gridops-core/tests/fleet_agent_commands.rs (`fixture()`, the GitHub `PrepareEnvironment`)
- crates/gridops-api/src/resources.rs (`delete_bitbucket_connection`, and the id generation in Bitbucket connection create)
- Cargo.lock (`serde` 1.0.229, `serde_json` 1.0.151), used to build a scratch crate that checked the `GitlabRunnerMaterial` serde shape
- the local `sqlite3` 3.54.0, used to check `pragma_foreign_key_check` with `foreign_keys` OFF and `PRAGMA foreign_keys` inside a transaction
