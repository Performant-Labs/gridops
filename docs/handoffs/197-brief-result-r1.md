## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** Claim: `axum` "matches the specific route first regardless of order" so the `/api/v1/fleet/{*path}` catch-all can sit at the end of the route chain — evidence: axum's routing/matchit documentation excerpt, or a small routing test in the repo showing the catch-all doesn't shadow specific routes.
- **[NV-2]** Claim: `gitlab_jobs` in `migrations/0047_gitlab.sql` has only `status_changed_at`, `created_at`, `updated_at` (no `started_at`/`completed_at`) and the table is not `STRICT` — evidence: the `CREATE TABLE gitlab_jobs` block from `migrations/0047_gitlab.sql`.
- **[NV-3]** Claim: `crates/gridops-core/src/db.rs` has `SLOW_STATEMENT = 100 ms` and `.log_slow_statements(Warn, SLOW_STATEMENT)` — evidence: the relevant lines of `db.rs`.
- **[NV-4]** Claim: `crates/gridops-api/src/main.rs` registers all `/api/v1/settings…` routes and merges `fleet::router()`, and `crates/gridops-api/src/lib.rs` does not exist — evidence: router-registration excerpt of `main.rs` and a directory listing of the api crate.
- **[NV-5]** Claim: `crates/gridops-reconciler/src/main.rs`'s `cleanup_retention` is a sequence of `DELETE … WHERE <time> < ?` statements — evidence: the `cleanup_retention` function body.
- **[NV-6]** Claim: `/fleet/hosts` refuses unauthenticated requests in a specific, imitable way (criterion: "an unauthenticated request is refused the same way `/fleet/hosts` refuses it") — evidence: the auth middleware / handler for `/fleet/hosts` in the api crate.
- **[NV-7]** Claim: the last migration on master is `0049_fleet_inventory_snapshot_retention.sql` (so the new one is `0050`) — evidence: `ls migrations | sort | tail -3` output at the branch head.

### WARN findings
- **[W-1]** The Mars timing criterion is ambiguous: the criterion says "the migration applies … in under a minute" (singular), but the procedure times "all migrations up to and including the new one." If the full chain is timed, the criterion depends on pre-existing migrations and machine load, and "under a minute" may fail for reasons this story doesn't own. Recommendation: define exactly what is timed (the new migration alone vs the full chain on a fresh copy), and say how load is accounted for, since `uptime` is only recorded, not constrained.
- **[W-2]** Five overlap/gap items ("resolve each in the plan") are deferred rather than resolved — particularly overlap 3, where #190's insert-site edits are structurally incompatible with the guard's stated add-only rule. A plan that resolves these wrongly invalidates the guard test PRs. Recommendation: resolve at least overlap 3 and overlap 5 (route registration file) in the brief itself, since the guard's semantics and "no wave-1 issue edits main.rs" invariant depend on them.
- **[W-3]** The guard test-PR procedure requires re-opening both test PRs after any post-head change to the guard script, owners file, contract tests, or CI job — including via rebase. This is a fragile, potentially unbounded loop if the story branch is rebased late. Recommendation: state a stop condition (e.g., final head SHAs recorded immediately before merge) or cap iterations.
- **[W-4]** Several wave-1 contract tests baked into this story's acceptance are timing-sensitive ("picked up within a minute" cache refresh in #190; "no write transaction over 50 ms" on seeded data; "10 concurrent calls call the source once" in #194) and must pass now, against stubs, once markers are removed. Tests that measure wall-clock behavior against empty stubs can be green or flaky for reasons unrelated to the wave-1 work. Recommendation: specify that the marker-removed proof run happens once on the scratch branch (not in CI), and that timing assertions are tolerant of CI variance.
- **[W-5]** #192's property-test criterion is conditional ("proptest, if it's already in `Cargo.lock`; otherwise 1,000 seeded random inputs") — the fork is decided at implementation time, and the fallback's seeding strategy is unspecified, making the criterion's strength depend on an unstated choice. Recommendation: pick the branch now, or specify the fallback's input distribution.

### NIT findings
- **[NIT-1]** "Disk is tight (about 29 GB free)" plus a 2.9 GB DB copy plus shared cargo target dir makes the Mars check the likeliest disk hazard; a sentence on minimum free space needed before starting the copy would help.
- **[NIT-2]** The brief lists #204 stubs (`health_source.rs`, `fleet_health_snapshot.rs`) as created here but they appear in the owners table under #204 (wave 2); the owners file treatment of stubs owned by future-wave issues is only implied by "the guard runs for wave-1 issues only" — say explicitly that unwritten wave-2 stubs are listed under #204 or #197.

### Verdict
PASS — no BLOCK findings; implementation may proceed.