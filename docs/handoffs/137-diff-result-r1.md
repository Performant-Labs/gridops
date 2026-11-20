## Implementation Review (Round 1)

### BLOCK findings

None.

### Needs-verification findings

**[NV-1]** `migrations/0046_gitlab.sql:1` — The claim that sqlx detects `-- no-transaction` only when the file's text *starts* with the directive — The diff shows 0046's first line is `-- no-transaction`. The evidence appendix cites `sqlx-core-0.9.0/src/migrate/source.rs:234` (`sql.starts_with("-- no-transaction")`) as settling this. That source excerpt is not shown in this prompt; the SOURCE EXCERPTS section does not include it. Settling evidence: the sqlx-core source at that file:line, or a documentation page for sqlx migrations describing the `-- no-transaction` behavior.

**[NV-2]** `migrations/0046_gitlab.sql:112` — The claim that `PRAGMA foreign_keys` is a no-op inside a transaction, so the migration correctly places `PRAGMA foreign_keys=OFF` before `BEGIN` and `PRAGMA foreign_keys=ON` after `COMMIT` — The SQLite documentation or a test against the pinned SQLite version demonstrating that `PRAGMA foreign_keys` cannot be changed while a transaction is open would settle this. Not shown in this prompt.

**[NV-3]** `migrations/0046_gitlab.sql:113` — The claim that `PRAGMA foreign_key_check` reports violations even when `foreign_keys` is OFF — This is asserted in the brief and tested in the new `the_0046_foreign_key_guard_fails_on_a_dangling_reference` test, but the behavior itself is a SQLite library fact. Settling evidence: the SQLite documentation for `PRAGMA foreign_key_check`, or an excerpt from the local `sqlite3` 3.54.0 showing the check returns violations with `foreign_keys=OFF`.

**[NV-4]** `crates/gridops-core/src/gitlab/mod.rs:242` — The claim that `JobStatus::Unknown` is stored as `"unknown"` because the original GitLab string isn't kept, and `as_str()` returns `"unknown"` — Settling evidence: the `JobStatus::as_str` match arm in the diff at `gitlab/mod.rs:224-236` (shown) confirms `Unknown => "unknown"`, but whether any consumer needs the original string is a design question not testable here.

**[NV-5]** `crates/gridops-manager/src/main.rs:1711` — The claim that the manager forwards Tart starts by re-serializing `ProvisionRunner`, so the `skip_serializing_if = "Option::is_none"` on `gitlab` and `gitlab_runner_token` keeps absent fields out of forwarded GitHub/Bitbucket bodies — The SOURCE EXCERPTS show `start_runner` at `main.rs:1711` does re-serialize (`serde_json::to_value(&*input)`). The claim that `skip_serializing_if` works to omit the fields is standard serde behavior and matches the diff. This is TRUE, not a finding.

**[NV-6]** `crates/gridops-tart-agent/src/main.rs:287-300` — The claim that the Tart agent's private copies of `GITLAB_EXIT_*` and `GITLAB_EXIT_REASONS` are exactly equal to the core constants — The diff shows both sets of values (`0`, `75`, `77`; `"completed"`, `"idle_expired"`, `"registration_refused"`). The values in the diff are consistent. This is TRUE.

**[NV-7]** `migrations/0046_gitlab.sql:60-68` — The claim that the `fleet_ci_targets` rebuild keeps the outbound FKs to `installations`, `repositories`, and `bitbucket_connections` with NO ACTION, and adds `gitlab_connections` with NO ACTION — The diff's `fleet_ci_targets_new` table definition shows all four FKs. The test `gitlab_upgrade_from_0045_keeps_the_fleet_graph_and_every_reference` asserts the four rows via `pragma_foreign_key_list`. The behavior is SQLite-standard for a column-level `REFERENCES` without an explicit action. Settling evidence would be `pragma_foreign_key_list('fleet_ci_targets')` output on a migrated database, which the test claims to check.

**[NV-8]** `crates/gridops-core/src/gitlab/fake.rs:183-197` — The claim that `job` matches `project_id` and `job_id` together, `verify_access` returns `Ok` for a seeded scope of the same kind and id, and unseeded `pending_jobs`/`group_projects` return empty lists — These are implementation choices the handoff says the tests leave open. The diff shows exactly these behaviors. This is TRUE.

**[NV-9]** `crates/gridops-core/src/fleet/targets.rs:69-92` — The claim that the Bitbucket refusal error is kept word-for-word as `"only GitHub targets are synced"` — The diff shows the string in the test `gitlab_targets_are_stored_once_and_load_back_unchanged`, but the `ensure_target` function itself in the diff does not show the Bitbucket arm's error; it shows the GitLab arms return early, and the original GitHub arms follow. The handoff claims the original Bitbucket refusal (the `_ =>` arm) is unchanged. Settling evidence: the full `ensure_target` function in `crates/gridops-core/src/fleet/targets.rs`, specifically the wildcard arm and its `FleetError::InvalidRequest` payload.

**[NV-10]** `crates/gridops-core/src/gitlab/mod.rs:43-49` — The claim that `GitLabCredentials`'s hand-written `Debug` prints `access_token: "[redacted]"` with the fixed placeholder, not secrecy's own placeholder — The diff shows `field("access_token", &"[redacted]")`. This is TRUE.

**[NV-11]** `docs/openapi/gridops-v1.json` — The claim that `redocly lint` passes with the new operations — The handoff reports 67 warnings (63 operation-operationId, 4 no-invalid-media-type-examples) and a successful lint. Settling evidence: running `npm run validate:openapi` with the pinned `@redocly/cli` 2.72.1. The diff's OpenAPI changes are shown, and the structural test `the_gitlab_operations_are_specified_before_they_are_served` passes per the handoff. The lint result itself is a tool-run fact not shown here.

**[NV-12]** `migrations/0046_gitlab.sql:171-175` — The claim that the FK guard checks the whole database and would refuse to migrate on any pre-existing dangling reference anywhere — The diff shows the guard does `count(*) FROM pragma_foreign_key_check` without filtering by table. This is TRUE from the shown SQL.

### WARN findings

**[W-1]** `crates/gridops-core/tests/fleet_schema.rs:520-533` — The `gitlab_tables_enforce_the_pinned_constraints` test deletes connection `'gl-b'` after inserting a runner referencing it, to verify the `SET NULL` behavior of `runners.gitlab_connection_id`. However, the test's assertion relies on `gitlab_jobs` and `runner_pool_gitlab_connections` rows being cascade-deleted by `gitlab_connections.id`'s `ON DELETE cascade`. The test deletes `'gl-b'` after inserting a job via `job` with `connection_id='gl-b'`; that job was inserted successfully earlier. If the FK actions were mis-declared in the migration, the test would fail. That's good. No issue.

**[W-2]** `crates/gridops-core/src/gitlab/fake.rs:52-63` — The fake's `Instance` struct uses `HashMap<FakeMethod, VecDeque<GitLabError>>` for failures, but `FakeMethod` derives `Hash`, so queued failures per method work. However, `fail_next` pushes one error and returns immediately; there's no way to queue a failure for a specific method and a specific base_url *and* inspect the queue length. Tests use it correctly. No issue.

**[W-3]** `crates/gridops-core/src/provisioning.rs:40-47` — The `for_selected_provider` signature change from `bitbucket_connection_selected: bool` to `platform: CiPlatform` is a breaking public API change. The two call sites in the diff (`gridops-api/src/resources.rs:4258` and `gridops-reconciler/src/main.rs:2403`) are updated. Any third call site outside the diff would fail to compile. The handoff says only these two exist. Settling evidence: a grep of `for_selected_provider` in the repository. Not shown, but the diff shows both callers updated; no other callers appear in the diff.

**[W-4]** `migrations/0046_gitlab.sql:60-68` — The `fleet_ci_targets` rebuild uses `STRICT` mode. The original `fleet_ci_targets` from 0022 was also `STRICT` per the brief. The new table preserves this. No issue.

### NIT findings

**[NIT-1]** `crates/gridops-core/src/gitlab/mod.rs:265` — The `GitLabApi` trait has no default methods; every method is required. This is intentional, but a real client (#146) will implement all seven. Fine.

**[NIT-2]** `crates/gridops-core/src/gitlab/fake.rs:183` — The `job` method silently ignores unseeded jobs and returns `NotFound`. That's correct behavior for a fake, but the doc comment on `FakeGitLab` says "job returns NotFound for an unknown job" without noting that an unseeded lookup is "unknown". Minor.

**[NIT-3]** `crates/gridops-core/tests/fleet_schema.rs:695-701` — The `gitlab_upgrade_from_0045_keeps_the_fleet_graph_and_every_reference` test builds the database by calling `Database::legacy()`, then `MIGRATOR.run_to(45, ...)`, then loading `fleet_ready.sql`. The handoff notes this. The test then updates `workload_placements` to reference `'legacy-runner'`. The SOURCE EXCERPTS show `workload_placements` has `runner_id TEXT REFERENCES runners(id)` at 0022. The update is valid because `fleet_legacy.sql` has a runner `'legacy-runner'`. No issue.

**[NIT-4]** `crates/gridops-core/src/gitlab/mod.rs:200-206` — `JobStatus::Unknown` deserializes via `#[serde(other)]`. If GitLab adds a status spelled `"unknown"` (unlikely but possible), the round-trip would deserialize to `Unknown` and serialize back to `"unknown"`, which is consistent. No issue.

### Verdict

PASS — no BLOCK findings; testing may proceed.