## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** crates/gridops-core/src/fleet/targets.rs (`pool_targets`) — the claim that `pool_targets` filters retired targets via `target_id_for_key` / sync — show `pool_targets`'s SQL (the sweep table asserts "Filters retired" but no excerpt in this prompt shows its query).
- **[NV-2]** crates/gridops-api/src/fleet/openapi.rs:348-356 — the claim that the updated assertion passes against the updated `docs/openapi/gridops-v1.json` depends on what the `conflicts(...)` helper reads (409 description vs the operation description). The 409 description in the diff does contain both substrings ("a pool uses it", "fleet work on its fleet targets"), but the helper's source is not shown — show `conflicts()` to settle it.
- **[NV-3]** crates/gridops-api/src/resources/gitlab.rs (delete handler, unchanged tail) — the claim that the connection's secrets are still deleted "as today" inside the new `BEGIN IMMEDIATE` transaction — show the post-audit-event portion of `delete_gitlab_connection`.
- **[NV-4]** crates/gridops-core/src/fleet/admission.rs (admit CTE, grant counts) — the claim that revoked grants/links alone (without an inline `retired_at` predicate) prevent admission on a retired target — show the admit CTE SQL joining `host_target_grants`/`profile_ci_targets` with `revoked_at IS NULL`.
- **[NV-5]** crates/gridops-reconciler/src/fleet.rs (fleet pass) — the claim that the reconciler's GitHub arm and fleet pass cannot touch a retired target — the excerpt only shows the GitLab lifecycle path; show the reconciler's fleet-pass target acquisition.
- **[NV-6]** migrations/0048_fleet_target_retire.sql — the claim that `-- no-transaction` plus `PRAGMA foreign_keys=OFF` before `BEGIN` actually applies under the runner's migration executor (the comment itself notes OFF inside a transaction is a no-op) — show how `MIGRATOR`/sqlx handles a `no-transaction` migration's pragmas; the passing `fleet_schema` tests are asserted, not shown running.

### WARN findings
- **[W-1]** migrations/0048_fleet_target_retire.sql:52-64 — the trigger permits a retire transition that *keeps* `gitlab_connection_id` (`NEW.gitlab_connection_id IS OLD.gitlab_connection_id OR NEW.gitlab_connection_id IS NULL`), producing a retired target that still FK-blocks its connection's delete — with no un-retire path, that connection becomes permanently undeletable (the exact symptom of #183). Production code always nulls both columns, so this is reachable only by ad-hoc SQL, but consider tightening the trigger to require the connection to become NULL when retiring a GitLab row.
- **[W-2]** docs/handoffs (Handoff-F, "Tests that look wrong") — the handoff states "I did not edit it" regarding `a_connection_a_fleet_target_references_is_refused_with_409_not_500`, but the diff deletes that test in `gitlab_tests.rs`. The deletion itself is correct (the test pinned the behaviour #183 removes), but the handoff prose contradicts the diff; update the handoff so T isn't told to fix a deleted test.
- **[W-3]** crates/gridops-core/src/fleet/targets.rs:947 — `retire_gitlab_connection_targets` maps any of its DB errors to `ApiError::Internal` in the handler; if a concurrent writer created a queued intent between the check and the retire inside the same `BEGIN IMMEDIATE` transaction this is safe, but a failure mid-loop (e.g. after some targets retired) rolls back via the `?` — confirm the handler's error return actually rolls back the transaction rather than committing (standard axum/sqlx behavior, but it's the whole-or-nothing guarantee for this feature).

### NIT findings
- **[NIT-1]** migrations/0048_fleet_target_retire.sql:17 — `organization_id INTEGER` has no FK in the rebuild; this mirrors 0047's shape (per the unchanged-schema test), so presumably correct, but the excerpt list doesn't show 0047 to confirm line-by-line parity of the non-GitLab arms.
- **[NIT-2]** crates/gridops-api/src/resources/gitlab.rs:423 — the live-work 409 message "Stop it or let it finish first." uses a period while the pool/runner refusals' punctuation style isn't shown; trivially cosmetic.

### Verdict
PASS — no BLOCK findings; testing may proceed.