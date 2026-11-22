## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** Claim: `list_targets` creates a target for every GitLab connection via `sync_gitlab_targets` — supported by the excerpt (`targets.rs:210-212` shows `list_targets` calling `sync_gitlab_targets(pool)`), but the claim that sync *creates* rows for every connection needs the body of `sync_gitlab_targets` to settle it (crates/gridops-core/src/fleet/targets.rs, the `sync_gitlab_targets` function body).
**[NV-2]** Claim: the delete handler refuses with 409 while a `fleet_ci_targets.gitlab_connection_id` matches — the excerpt does not include `crates/gridops-api/src/resources/gitlab.rs`; the handler source (~lines 365-450) would settle it.
**[NV-3]** Claims about migration `0047_gitlab.sql`: FK default NO ACTION, CHECK requiring non-null connection for gitlab kinds, UNIQUE `canonical_key`, and a trigger aborting every UPDATE — none of `0047_gitlab.sql` is excerpted; the migration text would settle all four.
**[NV-4]** Claims about referencing tables in `0022_fleet_foundation.sql` (`host_target_grants.revoked_at`, `profile_ci_targets.enabled/revoked_at`, etc.) — the migration text would settle them.
**[NV-5]** Claim that "a new connection gets a new id, so a new `canonical_key`" — needs the `canonical_key` construction (targets.rs key-building code or the sync functions) to confirm the key embeds the connection id.
**[NV-6]** Claim that the existing audit event `gitlab.connection_deleted` and the `BEGIN IMMEDIATE` transaction shape exist "as today" — the current delete handler source would settle it.

### WARN findings
**[W-1]** Two acceptance-relevant decisions are explicitly deferred ("the architect decides"): whether to emit a `fleet_events` row per retired target, and the FK/CHECK mechanism (SET NULL vs. nulling on retire). These leave the test list under-specified for those behaviors — no test covers the chosen FK option's FK-integrity outcome, and the fleet_events assertion is conditional. Recommend the plan that follows this brief commits to both decisions and adds a test for the chosen option.
**[W-2]** The "sweep the class" step (#3) has its deliverable (a verdict list) in the handoff, but no acceptance criterion in the Tests section that the sweep actually happened (e.g., a test or checklist gate). Recommend tying the sweep deliverable to reviewable output in the diff gate.
**[W-3]** Edge case not covered by tests: retiring a target whose `profile_ci_targets` are currently `enabled=1` and in use mid-session — the tests check `revoked_at`/`enabled=0` after connection delete, but not the behavior of an in-flight host handshake session grant against a just-retired target. Consider a test or explicit statement that in-flight sessions are terminated/refused.

### NIT findings
**[NIT-1]** Test list says "and/or api `fleet_http` / gitlab resource tests" — ambiguous which crate owns which assertion; pin it so the diff gate can check test placement.

### Verdict
PASS — no BLOCK findings; implementation may proceed.