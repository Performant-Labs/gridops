## Brief Review (Round 1)

### BLOCK findings

**[B-1]** `runners.run_untagged` is listed under **Schema** for `0046_gitlab.sql` but `runner_pools.run_untagged` is named in the schema item itself: "`runner_pools.run_untagged` (integer, default 0)." The bullet introducing the migration attributes it to `runners`, while the field specification says `runner_pools`. The prompt shows both in the same Scope section, so this is an internal contradiction. The owning table must be fixed before implementation, because a migration touching the wrong table is a schema defect.

**[B-2]** The acceptance criterion "existing GitHub and Bitbucket rows survive the `runners` rebuild (tested on `tests/fixtures/fleet_ready.sql` and `fleet_legacy.sql`)" is untestable for the `runners` table as scoped. The brief states the rebuild is on `runners` (widening the `ci_platform` CHECK), but the fixture names and the parenthetical reference GitHub and Bitbucket rows surviving a `runners` rebuild. There are no shown fixture excerpts, so the prompt does not establish that either fixture even contains `runners` rows or GitHub/Bitbucket `runners` rows, nor that a migration test is wired to run them. As written, the criterion asserts a data-preservation property against files that are only named, not quoted, and no test mechanism is specified. The criterion must either cite the actual migration test that inspects pre- and post-rebuild row counts/values from those fixtures, or be reworded to name the concrete test harness that exercises it.

**[B-3]** `GitLabError` requires a `RateLimited { retry_after }` variant, but no unit or type is given for `retry_after`. Every downstream consumer of this enum (including the fake's failure injection and any future mapper from HTTP responses) needs the type to pattern-match and construct it. The brief provides no type, and the absence makes the shared contract under-specified for an issue whose entire purpose is to lock shapes before eight parallel issues build on it. The field type must be stated (e.g. `std::time::Duration`, `u64` seconds, an `http::HeaderValue`, or an optional timestamp).

**[B-4]** The `FakeGitLab` requirement says it "can be told to fail," but the brief does not define the failure surface: which `GitLabError` variants can be injected, whether failure is per-method, per-scope, or global, or how call recording is exposed to assertions. For a shared contract issue whose acceptance criterion is only "`FakeGitLab` has its own tests," this is circular: the tests would test an unspecified API. The fake's control surface (e.g. `fail_next_with(...)`, `fail_create_runner_with(...)`, `calls() -> Vec<Call>`) must be specified, or the acceptance criterion must require the fake's public API be documented in the brief before tests can be written against it.

**[B-5]** The `ci_platform` CHECK widening on `runners` is specified as "a table rebuild that keeps every index and FK," but the brief does not enumerate which indexes and FKs exist on `runners` at the time `0046` runs. The prompt shows two fixture files are read, but no excerpt from either. A rebuild migration that "keeps every index and FK" is not demonstrably complete without a list of the constraints to recreate, and the acceptance criterion only checks row survival, not index or FK survival. The brief must list the exact indexes and foreign keys on `runners` that `0046` must recreate, or the criterion must be extended to assert their presence post-rebuild.

### Needs-verification findings

**[NV-1]** The claim that `0045_workflow_jobs_unconcluded_index.sql` provides an "index for unconcluded jobs" whose shape `gitlab_jobs` mirrors is not checkable from this prompt. The source-material list says the file was read, but no excerpt is shown. Evidence needed: the SQL text of `migrations/0045_workflow_jobs_unconcluded_index.sql` showing the unconcluded-jobs index expression and column list, so `gitlab_jobs`'s equivalent can be compared.

**[NV-2]** The claim that `runner_pool_gitlab_connections` is "shaped like `runner_pool_bitbucket_connections`" depends on the schema of `migrations/0019_runner_pool_bitbucket_targets.sql`. The brief says the file was read but quotes none of it. Evidence needed: the `CREATE TABLE` / `CREATE INDEX` text from `migrations/0019_runner_pool_bitbucket_targets.sql` to establish the column set and constraints the new table must mirror.

**[NV-3]** The claim that the redaction change is only "add `glrt-` and `glpat-` patterns to secret redaction (`gridops-agent/src/session.rs`)" assumes the existing redaction implementation is pattern-list-driven and that these two prefixes are sufficient for GitLab runner tokens and personal access tokens. The prompt quotes none of `session.rs`. Evidence needed: the relevant redaction code in `crates/gridops-agent/src/session.rs` showing how patterns are registered and matched, to confirm the new prefixes fit the existing mechanism and catch full tokens, not just the prefixes.

**[NV-4]** The claim that `docs/openapi/gridops-v1.json` exists and can be extended with GitLab schemas passing `redocly lint` is not verifiable from this prompt; the file is listed as read but no excerpt is shown. Evidence needed: the relevant existing `gridops-v1.json` sections showing the `platform-connections` and webhooks path/schema structure and any shared component naming conventions the GitLab additions must follow for lint to pass.

**[NV-5]** The brief says `CiTarget` gains `GitlabProject { connection_id, project_id }` and `GitlabGroup { connection_id, group_id }`, and that every exhaustive `match` must compile. The only `domain.rs` excerpt shown in this prompt is none; the file is listed as read. Evidence needed: the current text of `crates/gridops-core/src/fleet/domain.rs` showing all exhaustive `match` sites over `CiTarget` and `CiPlatform`, so the list of places needing `unimplemented` arms can be bounded and the "names the issue that owns it" requirement can be checked per site.

### WARN findings

**[W-1]** `runners.gitlab_connection_id` and `runners.gitlab_runner_id` are added without a stated nullability policy or FK constraint. `Bitbucket`'s analogous columns on `runners` or the pool link table are not quoted, so nothing in the prompt establishes whether these should be nullable, have FKs to `gitlab_connections`, or follow an existing pattern. Recommend the brief state nullability and FK behavior explicitly, because later issues will assume it.

**[W-2]** The `gitlab_connections` unique index is on `(base_url, scope_kind, scope_id)`, but the table also stores `name` and `scope_path`. If `scope_id` is an external GitLab identifier that can collide across self-managed instances sharing a `base_url` scheme (e.g. two configured hosts under one URL prefix), or if `name` is intended as a user-facing unique handle, the uniqueness constraint may be wrong. The brief does not define `scope_id`'s source. Recommend clarifying whether `scope_id` is the GitLab numeric ID or an internal ID, and whether `name` requires uniqueness.

**[W-3]** The brief adds `IdleExpired` and `RegistrationRefused` outcome codes to the agent protocol, but does not state whether existing agents or servers must keep accepting unknown codes, nor whether the new codes alter any retry/backoff behavior. Since this is a shared contract consumed in parallel by other issues, recommend a one-line statement that unknown outcome codes remain forward-compatible and that these codes carry no new payload beyond the variant itself.

**[W-4]** `resolve_scope(path) -> Scope` and `verify_access(scope)` in `GitLabApi` are under-specified: the relationship between a "path" and `CiTarget`'s `project_id`/`group_id` is not given, nor is the shape of `Scope` beyond appearing in `create_runner(scope, ...)`. Later issues implementing #138/#140 will need to know whether `Scope` is an enum, a struct, and whether `resolve_scope` is network-backed or local. Recommend defining `Scope` in the brief.

### NIT findings

**[NIT-1]** The scope says "Every function body outside the fake is `todo!()`, or the code isn't called yet," while the domain section says use "an explicit `unimplemented` error that names the issue that owns it." These two are different mechanisms (`todo!()` panics with a fixed message; `unimplemented` can carry a custom message). Pick one wording for consistency in review.

**[NIT-2]** `GitlabConnectionId` is introduced "alongside `BitbucketConnectionId`" but no type alias or newtype definition is shown for either, so the shared shape (u64, UUID, newtype) is unspecified. State the underlying type.

**[NIT-3]** The `gitlab_jobs.tag_list` is JSON, but `Job.tag_list` in the trait is an untyped field with no element type. The wire format of `tag_list` should state whether it is `Vec<String>` or something richer, since the schema and the trait must round-trip.

### Verdict

BLOCK — 5 blocking findings; must resolve before implementation starts.