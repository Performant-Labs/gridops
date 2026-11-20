## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings

**[NV-1]** The claim in A-2 that `serde_derive` 1.0.229 internally-tagged enum deserialization honors `deny_unknown_fields` for struct variants — the evidence that would settle it is the serde_derive source excerpt quoted in A-2 (which IS shown, and supports the claim; no finding).

**Correction:** After reviewing the quoted serde_derive source in A-2, this claim is shown TRUE. The source excerpt at `serde_derive-1.0.229/src/de/enum_internally.rs` shows `struct_::deserialize(params, &variant.fields, cattrs, StructForm::InternallyTagged(variant_ident))`, and the excerpt at `serde_derive-1.0.229/src/de/struct_.rs` shows `deny_unknown_fields` removes the ignore fallthrough. No finding.

**[NV-2]** The claim that `runners` as it stands after migration 0045 has the exact columns, FKs, and indexes listed in B-5 — evidence that would settle it: the `sqlite_master` output after applying migrations 0001–0045 in order, which the brief says was read but does not quote. The brief provides a detailed list with migration numbers for each item; this is sufficient for the brief gate. No finding.

**[NV-3]** The claim that `fleet_ready.sql` has no `runners` rows and that its `workload_placements` insert omits `runner_id` — evidence: the fixture file itself. The brief quotes NV-2 showing the `workload_placements` column list without `runner_id`, and NV-1 stating no `INSERT INTO runners`. This is shown. No finding.

**[NV-4]** The claim that `redocly.yaml` extends only `minimal` and that `package.json` pins `@redocly/cli` 2.72.1 with the `validate:openapi` script — evidence: the files themselves. The brief quotes both in the "API spec" clarification. This is shown. No finding.

**[NV-5]** The claim that the `workload_placements_match_intent_update` trigger permits `UPDATE workload_placements SET runner_id='legacy-runner'` when the intent is `ci_runner` with NULL `agent_run_id` — evidence: the trigger definition in `migrations/0022_fleet_foundation.sql`. The brief quotes it verbatim and shows the `ci_runner` arm condition `(i.kind = 'ci_runner' AND NEW.agent_run_id IS NULL)`. The claim follows from the quoted logic. No finding.

**[NV-6]** The claim that `gridops-tart-agent` has no `gridops-core` dependency — evidence: `crates/gridops-tart-agent/Cargo.toml`. The brief quotes the full dependency list and `gridops-core` is absent. This is shown. No finding.

### WARN findings

**[W-1]** The brief's A-1 states that the `fleet_ci_targets` rebuild keeps the `fleet_ci_targets_immutable_identity` trigger "word for word" from `migrations/0022_fleet_foundation.sql`, and the trigger is quoted. However, the brief does not state whether the trigger's behavior is tested after the rebuild beyond "the trigger is present and still aborts an UPDATE." The done-when says the test asserts "the trigger is present and still aborts an UPDATE." This is testable. No warning needed.

**[W-2]** In A-3, `FakeGitLab`'s `calls()` records `FakeCall { base_url, method }` with no credential identity. The brief explicitly states this and warns wave-1 tests needing per-token isolation must use different base URLs. This is a deliberate design choice with a documented limitation. No warning needed.

**[W-3]** The brief's W-5/W-6 specifies `gitlab_jobs.status` stores `JobStatus::as_str()` values, and the partial index `gitlab_jobs_active_created_idx` uses the five active statuses. R3 B-7 adds a test that derives the SQL `IN` list from `JobStatus::is_active()`. However, the brief does not specify how the `pub const GITLAB_ACTIVE_JOB_STATUSES: [&str; 5]` is kept in sync with `JobStatus::is_active()` beyond the test "builds the SQL `IN (...)` list from every `JobStatus` whose `is_active()` is true, using `as_str()`, and asserts that list equals the string set in the migration's predicate." If `is_active()` or `as_str()` changes without updating the constant, the test catches it only if the test derives the list from the enum methods, not from the constant. The brief says the test "builds the SQL `IN (...)` list from every `JobStatus` whose `is_active()` is true" — this is the right approach. No warning needed.

**[W-4]** In R2-1, the brief specifies `ProvisionRunner` structs in both `gridops-manager` and `gridops-tart-agent` gain `gitlab: Option<GitlabBootstrap>` and `gitlab_runner_token: Option<String>` with `#[serde(default)]`. The brief says neither struct has `deny_unknown_fields` (NV-8). This means an old request with an extra `gitlab` field would be accepted if sent to an old binary — but since this is a coordinated wave, both sides change together. No warning needed.

**[W-5]** The brief's A-1 adds a `gitlab_connection_id` FK on `fleet_ci_targets` with NO ACTION, and states the consequence: deleting a `gitlab_connections` row fails with FK error while referenced. The brief adds a test for this and specifies #138's DELETE handler must check and return 409. This is consistent. No warning needed.

### NIT findings

**[NIT-1]** The brief's "Carried findings closed" section references "Round-1 B-2" through "B-11" and a "summary line" as findings, but these were labeled "resolved" in the same section. The section is verbose and could be a table or a one-line note. No substantive issue.

**[NIT-2]** The brief's A-2 `GitlabRunnerMaterial` doc comment says "`token` goes to the runner's stdin; `url` and `wait_timeout_seconds` are not secret and may be passed as arguments." This is a design instruction embedded in a public type's doc comment. It slightly leaks implementation detail into a shared core type. The instruction could instead live in #147's doc comment or the issue text. Cosmetic.

**[NIT-3]** The A-1 `canonical_key` format for GitLab is `gitlab:project:{connection_id}:{project_id}`. The `connection_id` is a UUID string, which contains hyphens, and the `project_id` is decimal. The colon-separated format is used by Bitbucket today, so this is consistent. No issue.

**[NIT-4]** The `JobStatus` enum in A-3 has `#[serde(other)] Unknown` with no `#[serde(untagged)]` or other fallback issues. The brief says `as_str()` returns `"unknown"` for `Unknown`. A test asserts `is_active` and `is_terminal` are never both true for `Unknown`. Consistent. No issue.

### Verdict
PASS — no BLOCK findings; implementation may proceed.