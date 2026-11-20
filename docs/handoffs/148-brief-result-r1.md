## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** A2-W4 requires writing to a live-runner row matched by `gitlab_runner_id` and `gitlab_connection_id` of a `gitlab_jobs` row — the shown migration excerpt (`migrations/0047_gitlab.sql:62-65`) stops at `job_id` and shows no `gitlab_runner_id`, `gitlab_connection_id`, `status_changed_at`, or `updated_at` columns on `gitlab_jobs`, and the runners table's GitLab columns are not shown at all. Evidence that would settle it: the remainder of `0047_gitlab.sql` (columns after line 65) and the runner table's `gitlab_runner_id`/`busy`/`gitlab_connection_id` columns; if those columns don't exist, A2-W4 and A2-B1's `updated_at` mechanism are unimplementable without a contract change the brief forbids.
- **[NV-2]** A-W1/A2-W7 assume `Reconciler` test-constructor sites exist at "`main.rs` near line 4825 and `start_tests.rs` near line 425" — no excerpt shows them. Evidence: those two locations in the working tree.
- **[NV-3]** A-B1's fallback interval: whether `reconcileIntervalSeconds` is cheaply readable from a spawned worker (the brief itself allows a constant instead, so this is only documentation hygiene). Evidence: the `run_passes`/interval-reading code in `crates/gridops-reconciler/src/main.rs`.
- **[NV-4]** A-W4 assumes `assigned_queued_jobs` and `queued_job_candidates` in `autoscaling.rs` can be extended with a GitLab arm using "the same `room` subquery and `PoolStanding::of`" — the outline was read by the brief author but no excerpt is attached here. Evidence: `crates/gridops-core/src/autoscaling.rs:210-600` as excerpt, so the diff gate can check the UNION/merge matches the existing query shape.
- **[NV-5]** A2-W6's invariant "must not conclude rows of a project whose `pending_jobs` failed" combined with the group-poll path: a group connection whose `group_projects` call itself fails has no project list at all; the brief doesn't say whether *all* of that connection's projects count as "failed" (blocking conclusion) or as unknown (allowing conclusion of every stale row). Evidence to settle: a one-line clarification in the brief; as written it is ambiguous, but both readings are safe-ish, so it is not a contradiction.

### WARN findings
- **[W-1]** Untested binding rules: A2-W5 (platform `EXISTS` check for GitLab pools) and A-W4 (cross-platform candidate ordering: GitHub before GitLab, then pool order; a GitLab job consuming room ahead of a later GitHub job in a shared pool) have no corresponding Done-when tests. Add a test each, or the two rules most likely to regress silently ship unpinned.
- **[W-2]** A-B3 says `manual`, `scheduled`, `unknown` and terminal statuses are "never read", while A2-W2/W3 and the A2 Done-when explicitly have the poll read and rewrite `manual`/`scheduled` rows (a `manual` row reported `pending` becomes `pending`). Read charitably A-B3 means "never read as demand", but the wording conflicts; reword A-B3 to "never demand" so the two rounds can't be read as disagreeing.
- **[W-3]** A2-B1's starvation bound relies on *every* writer setting `updated_at`, including #140's webhook (not yet on master and owned by another session). If the webhook ever writes a row without bumping `updated_at`, that row becomes immediately re-checkable every poll. Add an explicit note-to-#140 in the shared notes (as done for the credentials loader) that webhook writes must set `updated_at`.
- **[W-4]** The Done-when "no collision between a GitHub job and a GitLab job with the same numeric id" tests A-B2, but no test pins the *cross-connection* collision case (two GitLab connections reporting the same `job_id`), which the key `Gitlab { connection_id, job_id }` exists to prevent and the fake's base-URL keying makes cheap to test. Recommend adding it.
- **[W-5]** NV-5 above: specify the `group_projects`-failure behavior explicitly (recommend: skip the whole connection's conclusion step for that poll), so the implementer doesn't have to guess.

### NIT findings
- **[NIT-1]** A-B1 names the fallback constant `GITLAB_POLL_INTERVAL` 30 s but leaves the choice to implementer judgment ("if that is not cheaply readable"); just pick one now to avoid a bikeshed in review.
- **[NIT-2]** The D12 note repeats that `wait_timeout_seconds` belongs to #139/#147 — fine, but the Constraints section's "do not touch `gitlab_lifecycle.rs`" already covers it; harmless redundancy.

### Verdict
PASS — no BLOCK findings; implementation may proceed.