## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** The claim that `list_targets` creates a GitLab target for every GitLab connection, making every connection undeletable once the fleet targets list is opened — the excerpt confirms `sync_gitlab_targets(pool)` is called at `targets.rs:212`, but the body of `sync_gitlab_targets` is not shown. Settle with the source of `sync_gitlab_targets` (likely below `targets.rs:222`) showing it inserts a row per GitLab connection.
**[NV-2]** The claim that the DELETE handler refuses with 409 on `fleet_ci_targets` count, and the refusal order versus pool/runner checks — settle with `crates/gridops-api/src/resources/gitlab.rs` ~365-450.
**[NV-3]** The claim (decision §6) that "No production code writes `host_session_target_grants`" and that `authorization.rs` has no SQL — settle with a grep of `crates/` for `host_session_target_grants` writes and a read of `authorization.rs`.
**[NV-4]** The claim in test (a) that `delete_pool` already removes the pool's placements, intents and profile links "by design" — settle with the `delete_pool` implementation showing those deletions.
**[NV-5]** The claim in test (b) that removing the connection from a pool "leaves `profile_ci_targets` enabled" — settle with the pool-update code path that unlinks a connection from a pool.
**[NV-6]** Decision §5's cited pattern (`webhooks/gitlab.rs:358` → `gitlab::jobs::record_gitlab_job` taking `&mut SqliteConnection`) and `registry/delete.rs:165-171` (revision bump on revoke) — neither excerpt is shown; settle with those file:lines.
**[NV-7]** The `fleet.target.retired` event kind "fits the existing event conventions" (decision §3 decides yes while the build section says the architect decides) — settle with the existing `fleet_events` kind registry/conventions.
**[NV-8]** Migration 0047 schema facts (CHECK, UNIQUE `canonical_key`, NO ACTION FK, trigger aborting every UPDATE, §5 rebuild) — settle with the text of `0047_gitlab.sql` §5.

### WARN findings
**[W-1]** Internal inconsistency between build §1 (offers `ON DELETE SET NULL` as an option) and binding decision §1 (explicitly rejects it, keeps NO ACTION, nulls the column in the retire UPDATE). The decisions are declared binding, so this resolves, but the brief's build section should be reconciled to avoid the implementer following the rejected option.
**[W-2]** Decision §6 says no production code writes `host_session_target_grants`, yet build §2 and decision §5 require revoking those rows (an UPDATE writing `revoked_at`). If the "no writes" claim refers only to inserts/creation paths it should say so; otherwise the implementer may conclude no revoke code is needed for that table.
**[W-3]** Grant-on-retired-target error semantics differ in strength: build §3 says "a clear 4xx error", decision §4 pins it to 404. The acceptance test ("refuses a grant write") should pin the expected status so it is testable, not implementer's choice.
**[W-4]** No acceptance test covers the OpenAPI description update (build §5) — build §5 lists four artifacts (OpenAPI copy, UI copy, docs, CHANGELOG) but the test section only exercises behavior. Add a check that the contract schema/description matches, or state explicitly that copy changes are review-verified only.
**[W-5]** The liveness refusal ("may still be on a machine": non-terminal placement, unreleased capacity allocation, or queued intent) has no test for the *unreleased capacity allocation* branch — the test list covers live placement and queued intent only. Add a case or justify its absence.

### NIT findings
**[NIT-1]** Build §2 says the `fleet_events` emission "if that event kind fits… The architect decides", but decision §3 has already decided. Trailing "architect decides" phrasing should be struck to keep one source of truth.
**[NIT-2]** Build §1's example column name `retired_at INTEGER` vs decision §1's "retire transition" trigger carve-out: the trigger exception list should explicitly name every column it permits changing (`retired_at`, `gitlab_connection_id`) to keep the trigger's "refuses every UPDATE except…" unambiguous.

### Verdict
PASS — no BLOCK findings; implementation may proceed.