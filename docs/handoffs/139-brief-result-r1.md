## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** D7/D16 assert the schema already exists: `migrations/0047_gitlab.sql` on master contains the `runners.gitlab_connection_id` / `runners.gitlab_runner_id` columns and the indexes `runners_gitlab_connection_idx` and `runners_gitlab_runner_id_idx`, plus the `runner_pool_gitlab_connections` table — evidence: the text of `migrations/0047_gitlab.sql` on master `b22c451`. If 0047 is not yet merged, D16's "add no migration" makes the brief unimplementable as written.
- **[NV-2]** "exactly as the Bitbucket arm does" / "exactly where `fleet.rs` stores today's `github_registration` material" / "~`main.rs:2766`" / "~`main.rs:2811`" presuppose current reconciler code shape (a Bitbucket registration arm in `start_admitted_runner`, the failure-reset after POST, the GitHub default branch in `delete_runner`'s dispatch). Evidence: the relevant excerpts of `crates/gridops-reconciler/src/main.rs` and `fleet.rs` at the merge base.
- **[NV-3]** The sweep's premise that "a connection whose only pool was deleted has no membership or runner rows left, because `delete_pool` hard-deletes and both tables cascade" — evidence: the `delete_pool` code path and the FK definitions in `migrations/0047_gitlab.sql`. If the cascade assumption is wrong, the sweep-per-connection design silently over/under-deletes.
- **[NV-4]** D17's trait signature and `GitlabRunnerMaterial` wire shape (`{"kind":"gitlab_runner","token","url","wait_timeout_seconds"}` in `crates/gridops-core/src/fleet/protocol/agent.rs`) match what #137 shipped and what #146 is implementing — evidence: those file excerpts on master, plus issue #136 comment 7053069685.
- **[NV-5]** `placement::store_bootstrap` and `placement::abandon_unstarted` exist with the semantics the brief assumes (bootstrap persists per placement; abandon releases "registering" placements) — evidence: `crates/gridops-core/src/fleet/placement.rs`.
- **[NV-6]** `settings` is a generic key-value table that tolerates new keys (`lastGitlabRunnerSweepAt`, `gitlabRunnerSweepIntervalSeconds`) without migration, as `maybe_sync_github`'s precedent implies — evidence: the `settings` schema and `maybe_sync_github`.

### WARN findings
- **[W-1]** Done-when does not cover round-2 Block 2 (image substitution): no test for the readable "GitLab pool has no runner image" refusal on `save_pool_fleet` and before a manager-path `create_runner`, nor for the non-empty-image pass-through. Blocks 1 and 4 explicitly add done-when items; Block 2 does not. Add them to the amended Done-when.
- **[W-2]** "Token never in logs or row" (amended Done-when) is ambiguous: the fleet path deliberately stores the token in the bootstrap material via `placement::store_bootstrap`, which persists somewhere (likely a row). State explicitly "never in the `runners` row, never logged; the sealed bootstrap material is the sanctioned store" so the criterion is testable rather than self-contradictory on first reading.
- **[W-3]** With the placeholder `GitLabApi` in production, the sweep runs every 600s over every `gitlab_connections` row and errors each time ("GitLab client not available yet"), producing recurring warn noise for every connection until #146 lands. Recommend documenting that the sweep skips/warns-once when the client is the placeholder, or accepting the noise explicitly.
- **[W-4]** The Docker-socket refusal ("refused before `create_runner` with a readable reason") and the Inspect-time late `RegistrationRefused` (failed, counted) have no explicit done-when entries. Add them, since they are behavior #139 owns.
- **[W-5]** Block 1's reset condition ("reported busy, or exits completed/idle_expired") needs a defined observation point on the manager path for "busy" (manager listing status? poll?). As written, an implementer could plausibly never reset if the runner goes idle-expired before any busy report is observed — the exit-based reset covers that, but the busy-based one needs a named source.

### NIT findings
- **[NIT-1]** Name collision risk: the reconciler already has a `delete_runner` function in `main.rs` and `GitLabApi` also has `delete_runner`; the seam description says "delete_runner's dispatch" for both. Naming the GitLab helper `deregister_gitlab_runner` (as the brief itself does for Bitbucket) would avoid confusion.
- **[NIT-2]** Connection selection ("fewest live GitLab runners, ties by created_at, connection_id") requires a live-runner count query per pass that the Bitbucket counterpart doesn't need; the done-when's "register" tests don't ask for a deterministic-selection test. Consider one.

### Verdict
PASS — no BLOCK findings; implementation may proceed.