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
