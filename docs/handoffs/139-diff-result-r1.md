## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** crates/gridops-reconciler/src/start_tests.rs:247-260 — the tests filter manager start bodies by `body["poolId"]`, but the diff never shows `poolId` being added to the start request in `start_admitted_runner` — the excerpt of `main.rs`'s request `json!` (only `"runnerId"` visible in the hunk) or a passing test run would settle it.
- **[NV-2]** crates/gridops-reconciler/src/gitlab_lifecycle.rs (`maybe_sweep`) — the sweep stores `lastGitlabRunnerSweepAt` as `now.to_string()` into `settings.value`; whether the `settings` table's `value` column is text and `setting_i64` parses it (the brief says the table is a key-value store in `0001_initial.sql:165`, not shown) — a `0001_initial.sql` excerpt of the `settings` `CREATE TABLE` and `setting_i64`'s body would settle it.
- **[NV-3]** handoff "Tier 1 self-check" — the claims that the 12 workspace test failures and 17 clippy findings are pre-existing on commit 2aa6002 and untouched by this change cannot be checked from this prompt; a run of `cargo clippy --workspace --all-targets -- -D warnings` and the two named failing suites on the base commit would settle it. Note the Done-when requires clippy `-D warnings` to pass, so if the toolchain drift is real the acceptance criterion cannot be met without out-of-scope edits — flag for the PR gate.
- **[NV-4]** crates/gridops-reconciler/src/gitlab_lifecycle.rs (`left_service`) — the claim that `system_event` writes into `runner_events` with the columns the W-2 assertion queries (`event`, `message`, `pool_id`) — an excerpt of `system_event` in `main.rs` would settle it.
- **[NV-5]** crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs (`Fleet::new`) — the fleet fixture's `host_backends`/`fleet_hosts` insert shapes and `RegistryService::record_command_result` path are unchanged code not shown here; whether the fixture actually drives the real admission path (vs. failing silently into a different branch) is settled only by the reported green test run, which this prompt cannot reproduce.
- **[NV-6]** crates/gridops-reconciler/src/main.rs (`start_admitted_runner` GitLab arm) — the brief (manager path, "One ordering per path") requires `failure_cleanup(true, true, …)` to drive the rollback delete; the diff gates the deregister on `failure_cleanup(true, true, false).remove_provider_runner` — the `failure_cleanup` signature/defaults in `gridops-core` are not shown; an excerpt would confirm the flag order matches the brief's intent.

### WARN findings
- **[W-1]** crates/gridops-reconciler/src/gitlab_lifecycle.rs (`fleet_target`) — the Docker-socket refusal counts through `record_provision_failure(&app.database, pool)` with the pass's `Pool`, while the round-4 amendment says every GitLab count is given a `load_pool` copy; the handoff's round-5 warning exempts start-loop sites, and `fleet::provision` is a start-loop site, so this is likely intentional — but the brief's round-4 text and the handoff's round-5 text are in tension here; record the reconciliation in the PR description.
- **[W-2]** crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs (`a_tart_start_…`) — the Tart register test does not run `assert_no_token_stored`/log capture, though Done-when 1 says "token secrecy holds" for Docker **and** Tart; add the assertions to the Tart test.
- **[W-3]** crates/gridops-reconciler/src/gitlab_lifecycle.rs (`next_connection`) — the "live" status list includes `'stopped'`, so a stopped runner keeps its connection weighted; harmless for selection but worth a comment or alignment with `runner_live`'s rule (which uses `status!='deleted'`), since two different liveness definitions now exist in one module.
- **[W-4]** handoff "Deviations" — the busy reset (round-3 "may also reset") is not implemented; the brief marks it optional and #140-dependent, and the deviation is recorded — acceptable, but Done-when item 10's "after a later successful GitLab run the count is reset" is tested only via the exit path; ensure the PR description states this per brief-gate R2 W-1.

### NIT findings
- **[NIT-1]** crates/gridops-core/src/fleet/targets.rs (`sync_gitlab_targets`) — it lists all `gitlab_connections` on every `list_targets`/`save_pool_fleet` call; fine at current scale, but it is not scoped to a pool, unlike `sync_github_targets`; a doc comment noting this is connection-global would help.
- **[NIT-2]** crates/gridops-reconciler/src/gitlab_lifecycle.rs (`sweep_connection`) — the `ExternalId::try_from(id)` skip log says "not a valid id", but the id is a `u64` from GitLab; the message reads oddly if the only failure mode is a zero/out-of-range id.

### Verdict
PASS — no BLOCK findings; testing may proceed.