## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** `FakeGitLab` exists at `gridops_core::gitlab::fake` behind feature `test-support` with the methods the tests need (create/delete runner, inspectable state for orphan/rollback assertions) — an excerpt of `crates/gridops-core/src/gitlab/fake.rs`.
- **[NV-2]** `GitlabRunnerMaterial`, `placement::store_bootstrap`, and the wire shape `{"kind":"gitlab_runner","token","url","wait_timeout_seconds"}` exist as described (D1) in `crates/gridops-core/src/fleet/protocol/agent.rs` and `crates/gridops-reconciler/src/fleet.rs`.
- **[NV-3]** `runners_gitlab_connection_idx` and `runners_gitlab_runner_id_idx` are actually present in `migrations/0047_gitlab.sql` (D7), and `runners` has `gitlab_connection_id` / `gitlab_runner_id` / `ci_platform` columns — the migration text.
- **[NV-4]** `ensure_target` "already has the GitLab arms" and `ensure_target_identity_for_pool` / `link_pool_target` are GitHub-only today (D6/D11) — excerpts of `crates/gridops-core/src/fleet/targets.rs` and `fleet/service.rs`.
- **[NV-5]** `GITLAB_EXIT_REASONS` and its three values exist in `crates/gridops-core/src/provisioning.rs` (D10).
- **[NV-6]** `next_bitbucket_connection` exists with the ordering semantics the brief claims to mirror ("`created_at, connection_id` order", per-connection cap) — the function source in `crates/gridops-reconciler/src/main.rs`.
- **[NV-7]** D5's outcome envelopes (IdleExpired inside `Failed { ResourceUnavailable, retryable: false }` on Inspect; RegistrationRefused inside `Failed { Configuration, retryable: false }` on start) match what #137 shipped — the corresponding protocol/fleet source lines.
- **[NV-8]** `GITLAB_DEFAULT_WAIT_TIMEOUT_SECS = 600` and the absence of a pool timeout column (D12) — `provisioning.rs` and the `runner_pools` schema.

### WARN findings
- **[W-1]** Done-when omits an explicit `IdleExpired` test even though the Outcomes section specifies its behavior (removed without error, replaced if below `min_count`); "cleanup on each exit path" arguably covers it but the acceptance list names only `RegistrationRefused`. Add `IdleExpired → removed + replacement` to the Done-when list explicitly.
- **[W-2]** The placeholder-client regime (every method returns `Upstream`) is specified only for the start path. Cleanup, `delete_runner` on exit paths, and the orphan sweep will also fail while the placeholder is live. Specify expected behavior: cleanup failures are logged/retried by the normal reconciler loop without wedging the pool, and the sweep never deletes anything when the client errors (fail-closed on the "never touch runners GridOps did not create" guarantee).
- **[W-3]** Orphan-sweep safety is described prose-wise but has no testable acceptance criterion beyond "tests cover the sweep". Make it explicit: a test asserting the sweep leaves untouched any GitLab runner whose description does not parse to a GridOps UUID and any runner whose UUID has a live `runners` row.
- **[W-4]** Rollback ordering is stated twice with slightly different trigger sets ("if `create_runner` succeeds but the start fails" in Scope vs "a failed insert also rolls the GitLab runner back" in Where-the-code-goes). These are compatible (insert failure is one way a start fails) but the reconciler should have a single rollback path covering: insert failure, fleet/manager start failure, and start-then-immediate-exit. Worth one clarifying sentence to avoid two divergent rollback implementations.

### NIT findings
- **[NIT-1]** The description format is given as "e.g. `gridops runner <uuid>`" — pick the exact format now so the sweep's parser and the writer are pinned by one constant, not two implementations of an example.
- **[NIT-2]** `run_untagged` is passed to `create_runner` but its source column and its effect on the orphan-sweep/rollback paths are untested in the Done-when; include it in the register test's assertions.

### Verdict
PASS — no BLOCK findings; implementation may proceed.