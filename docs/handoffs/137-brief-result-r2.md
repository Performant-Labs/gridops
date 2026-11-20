## Brief Review (Round 1)

### BLOCK findings

None.

### Resolved findings

**[R-1]** **[B-1]** `runners.run_untagged` is listed under **Schema** for `0046_gitlab.sql` but `runner_pools.run_untagged` is named in the schema item itself: "`runner_pools.run_untagged` (integer, default 0)." The bullet introducing the migration attributes it to `runners`, while the field specification says `runner_pools`. The prompt shows both in the same Scope section, so this is an internal contradiction. The owning table must be fixed before implementation, because a migration touching the wrong table is a schema defect.

**[R-2]** **[B-2]** The acceptance criterion "existing GitHub and Bitbucket rows survive the `runners` rebuild (tested on `tests/fixtures/fleet_ready.sql` and `fleet_legacy.sql`)" is untestable for the `runners` table as scoped. The brief states the rebuild is on `runners` (widening the `ci_platform` CHECK), but the fixture names and the parenthetical reference GitHub and Bitbucket rows surviving a `runners` rebuild. There are no shown fixture excerpts, so the prompt does not establish that either fixture even contains `runners` rows or GitHub/Bitbucket `runners` rows, nor that a migration test is wired to run them. As written, the criterion asserts a data-preservation property against files that are only named, not quoted, and no test mechanism is specified. The criterion must either cite the actual migration test that inspects pre- and post-rebuild row counts/values from those fixtures, or be reworded to name the concrete test harness that exercises it.

**[R-3]** **[B-3]** `GitLabError` requires a `RateLimited { retry_after }` variant, but no unit or type is given for `retry_after`. Every downstream consumer of this enum (including the fake's failure injection and any future mapper from HTTP responses) needs the type to pattern-match and construct it. The brief provides no type, and the absence makes the shared contract under-specified for an issue whose entire purpose is to lock shapes before eight parallel issues build on it. The field type must be stated (e.g. `std::time::Duration`, `u64` seconds, an `http::HeaderValue`, or an optional timestamp).

**[R-4]** **[B-4]** The `FakeGitLab` requirement says it "can be told to fail," but the brief does not define the failure surface: which `GitLabError` variants can be injected, whether failure is per-method, per-scope, or global, or how call recording is exposed to assertions. For a shared contract issue whose acceptance criterion is only "`FakeGitLab` has its own tests," this is circular: the tests would test an unspecified API. The fake's control surface (e.g. `fail_next_with(...)`, `fail_create_runner_with(...)`, `calls() -> Vec<Call>`) must be specified, or the acceptance criterion must require the fake's public API be documented in the brief before tests can be written against it.

**[R-5]** **[B-5]** The `ci_platform` CHECK widening on `runners` is specified as "a table rebuild that keeps every index and FK," but the brief does not enumerate which indexes and FKs exist on `runners` at the time `0046` runs. The prompt shows two fixture files are read, but no excerpt from either. A rebuild migration that "keeps every index and FK" is not demonstrably complete without a list of the constraints to recreate, and the acceptance criterion only checks row survival, not index or FK survival. The brief must list the exact indexes and foreign keys on `runners` that `0046` must recreate, or the criterion must be extended to assert their presence post-rebuild.

### Needs-verification findings

**[NV-1]** The claim that `GitlabConnectionId` has "the same shape" as `BitbucketConnectionId` (newtype over `UpstreamText`, `#[serde(try_from = "String", into = "String")]`, `new`, `as_str`, `TryFrom<String>`, `From<GitlabConnectionId> for String`) rests on the clarification's assertion, but no excerpt of `domain.rs` showing either type definition is quoted in this prompt. Evidence that would settle it: the `BitbucketConnectionId` definition in `crates/gridops-core/src/fleet/domain.rs`, and the `UpstreamText` definition wherever it lives.

**[NV-2]** The claim that `GitlabApi` must use `created_by` in `gitlab_connections` but the scope section lists `created_by` as a column without a type or FK target. Evidence that would settle it: the `bitbucket_connections` table definition in `migrations/0019_runner_pool_bitbucket_targets.sql` or wherever `created_by` is defined, to confirm the type and whether it references a users table.

**[NV-3]** The claim that `access_token_key` and `webhook_secret_key` in `gitlab_connections` reference secret storage keys is not checkable from this prompt. Evidence that would settle it: the columns in `bitbucket_connections` (or another connection table) that hold analogous secret references, showing whether these are text columns holding key names, FK references to a secrets table, or something else.

**[NV-4]** The claim that "the GitLab operations must not declare a scoped `bearerAuth` requirement unless the same change adds matching `TOKEN_SCOPED_ROUTES` entries" is correctly derived from the quoted `openapi.rs` test description, but the actual `TOKEN_SCOPED_ROUTES` table is not shown. Evidence that would settle it: the `TOKEN_SCOPED_ROUTES` definition in `crates/gridops-api/src/fleet/openapi.rs`.

**[NV-5]** The claim that the `FakeGitLab` failure injection surface must be visible to "other crates' tests" via `cfg(any(test, feature = "test-support"))` implies cross-crate visibility. Evidence that would settle it: the existing `Bitbucket` fake (or another fake behind the same feature flag) showing the established pattern for exporting fakes and their control surfaces.

### WARN findings

**[W-1]** The brief still does not specify the type for `RateLimited { retry_after }` in `GitLabError`. The clarification section resolves which table `run_untagged` lives on and the `runners` rebuild details, but the original B-3 about `retry_after`'s type is not addressed in the clarifications. The brief says "This section adds facts verified in the repository and makes the scope and the done-when concrete. It removes nothing and narrows nothing," but it neither narrows nor resolves the `retry_after` type. This remains under-specified for a shared contract issue.

**[W-2]** The brief states "`FakeGitLab` has its own tests" as a done-when criterion, but the fake's failure surface and call-recording API remain unspecified after the clarifications. While the clarification section claims it "removes nothing and narrows nothing," it does not define the fake's control surface. Tests written against an unspecified API are circular.

**[W-3]** The agent protocol section says the registration material is "the `glrt-` token, with the base URL in `configuration`." The clarification confirms `ExecutionConfiguration` is `deny_unknown_fields` and that adding the base URL means adding a field to it. However, the brief does not specify the field name or type (e.g. `gitlab_base_url: String` or `Option<String>`), nor whether it is required only for GitLab targets or optional for all. Since `PrepareEnvironment` validation for Bitbucket currently rejects persistent-mode and Windows-ARM64 for Bitbucket targets, and GitLab targets must not silently misclassify, the field's presence and shape on `ExecutionConfiguration` should be explicit.

**[W-4]** The `IdleExpired` and `RegistrationRefused` outcome codes are added to `FailureCode`, `UncertainCode`, or `DiagnosticCode` (the brief says "outcome codes" and the clarification shows three enums in `outcomes.rs`). The brief does not state which enum gets which variant. A payload carrying `idle_expired` or `registration_refused` could be a `FailureCode`, `UncertainCode`, or `DiagnosticCode`, and peers built before this change reject any new variant in any of these enums. The owning enum for each new variant must be explicit.

**[W-5]** The `gitlab_jobs` table includes `pool_id` in the column list, but the unique index is on `(connection_id, job_id)`. If a single GitLab job can be processed by multiple pools (e.g. a job is retried across pools), the unique index would reject a second row for the same job from a different pool. The brief does not state whether `(connection_id, job_id)` should be unique regardless of pool, or whether the unique index should include `pool_id`. This matters for the reconciler's query behavior.

### NIT findings

**[NIT-1]** The brief says "Every function body outside the fake is `todo!()`, or the code isn't called yet," and the clarification says a `match` arm on a reachable path returns an error naming the owning issue. The two mechanisms (`todo!()` vs. explicit error) are now both specified for different situations, which resolves NIT-1 from the earlier round, but the brief could be clearer that `todo!()` is for unreachable stubs and the error is for reachable `match` arms.

**[NIT-2]** The brief does not specify whether `CreatedRunner { id, token: SecretString }` has `id` as a `String`, a newtype, or a GitLab numeric ID. The scope says `create_runner(...) -> CreatedRunner { id, token: SecretString }` but the type of `id` is unstated. Recommend stating the type of `CreatedRunner.id`.

**[NIT-3]** The brief lists `job(project_id, job_id) -> Job { id, project_id, pipeline_id, status, tag_list }` and `pending_jobs(project_id) -> Vec<Job>`, but `Job` is not defined beyond the inline field list. If `Job` is a shared struct, its definition should be explicit; if it's a type alias, the underlying type should be stated.

### Verdict

PASS — no BLOCK findings; implementation may proceed.