## Brief Review (Round 1)

### BLOCK findings

None.

### Resolved findings

**[R-1]** **[B-1]** The `fleet_ready.sql` acceptance criterion in "Done-when, extended for A-1" is self-contradictory: it claims `fleet_ready.sql` has "no `runners` rows" (verified in the Clarifications section), yet the same criterion says to test that "the `fleet_ready.sql`'s Bitbucket and GitHub targets survive" and that "that fixture carries the whole fleet graph that references targets." The fixture cannot simultaneously contain no `runners` rows and carry a complete fleet graph (which includes `workload_placements` requiring `runners`). This makes the acceptance test vacuous for the `runners` rebuild hazard: the test cannot exercise the cascade/set-null foreign key behavior on `runner_events` and `log_streams` if those tables are empty. The criterion must specify a fixture that actually contains `runners` rows with inbound references, or the test must be explicitly acknowledged as not covering the `runners` rebuild hazard.

### Needs-verification findings

**[NV-1]** The claim that the `UPDATE workload_placements SET runner_id='legacy-runner'` is allowed by the `workload_placements_match_intent_update` trigger — the evidence would be the trigger's definition in `migrations/0022_fleet_foundation.sql`, which is not shown in this prompt. The brief asserts it was checked on a scratch SQLite database, but that is an assertion, not shown evidence.

**[NV-2]** The claim that `PRAGMA foreign_key_check` reports violations while `foreign_keys` is OFF — the brief asserts this was checked with SQLite 3.54.0, but the SQLite documentation excerpt or a test output is not shown. The evidence would be a SQLite documentation page on `PRAGMA foreign_key_check`, or the scratch test output.

**[NV-3]** The claim that `PRAGMA foreign_keys` is a no-op inside a transaction — the evidence would be SQLite documentation on `PRAGMA foreign_keys` stating it cannot be changed within a transaction.

### WARN findings

**[W-1]** The brief places the `PRAGMA foreign_keys=OFF` and `PRAGMA foreign_keys=ON` outside the `BEGIN`/`COMMIT` block, but the `-- no-transaction` directive combined with sqlx's connection pooling means other migrations or queries could run on the same connection between the `PRAGMA foreign_keys=OFF` and the `PRAGMA foreign_keys=ON` statements. The brief should specify that the entire migration must run as a single sqlx migration statement (which it appears to, given the `-- no-transaction` directive), or that a dedicated connection is used for this migration.

**[W-2]** The brief specifies that `gitlab_jobs.open` index includes `Unknown` status as "open" (via `WHERE status NOT IN ('success','failed','canceled','skipped')`), but `Unknown` is a serde catch-all for unrecognized GitLab statuses. If GitLab introduces a new terminal status that is not yet mapped, it would be classified as `Unknown` and treated as open, which could cause the reconciler to repeatedly poll a concluded job. This is a behavior risk for #148, not a contract defect, but should be acknowledged.

### NIT findings

**[NIT-1]** The brief says `GitlabRunnerMaterial.token` is a `String` rather than a `SecretString`, and the rationale is that the existing host-agent `Material` types also carry the token as a `String`. A `SecretString` would be safer and could still serialize to the pinned JSON; the hand-written `Debug` redaction is a mitigation, not a substitute for the type-level guarantee.

**[NIT-2]** The `gitlab_connections` table has no `deleted_at` column, while other connection tables in the schema may have soft-delete support. This is not specified as a requirement, but if #138 implements soft delete, a migration would be needed.

### Verdict

PASS — no BLOCK findings; implementation may proceed.