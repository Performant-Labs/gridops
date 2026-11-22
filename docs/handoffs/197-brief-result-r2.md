## Brief Review (Round 1)

### BLOCK findings
None.

### Resolved findings
**[R-1]** "The brief's acceptance criteria are entirely external and not reproduced here. Every 'Done when' list, the stub table, the file-ownership table, and the spec itself live in GitHub issues… The brief as a standalone document is vacuous…" — the brief now inlines every wave-1 Done-when list, #197's own criteria, the ownership table, the five known overlaps with instructions to resolve each in the plan, the migration-number correction, and the current registration points verified at `83149f5`. The criteria are now present, testable, and internally consistent in this document.

### Needs-verification findings
**[NV-1]** Claim: migration `0049` exists and `0050` is next free — evidence: `ls migrations/` output on master at review time (the brief itself says re-check).
**[NV-2]** Claim: `gitlab_jobs` lacks `started_at`/`completed_at` (`0047_gitlab.sql` has only `status_changed_at`, `created_at`, `updated_at`) — evidence: the shown claim is the author's reading; the actual `migrations/0047_gitlab.sql` DDL excerpt would settle it (the file was read "in part" but not quoted here).
**[NV-3]** Claim: registration points exist as described (`fleet::router()` merged in `main.rs`, settings routes registered there, `navigation.ts` data array, `run_pass`/`cleanup_retention` in reconciler `main.rs`, `SLOW_STATEMENT = 100 ms` in `db.rs`, `gridops-api/src/lib.rs` absent) — evidence: the corresponding file excerpts; all are asserted as "verified at `83149f5`" but not quoted.
**[NV-4]** Claim: `VACUUM INTO` recipe via `nginx:alpine` container works read-only and migration of the 2.9 GB copy runs under a minute — evidence: the recorded wall time from the actual run (the brief itself flags the recipe as not dry-run).
**[NV-5]** Claim: `axum` matches the specific route before the `{*path}` catch-all regardless of registration order — evidence: axum routing documentation/source.

### WARN findings
**[W-1]** Five overlap/gap items (contract tests inside wave-1 globs, #190's non-add-only insert-site edits, `gitlab_jobs` columns, the settings-route file choice) are deferred to "the plan decides" without stating the candidate resolutions or their acceptance tests here. The brief is consistent, but the diff gate must check each decision lands in `fleet-health-owners.json`, the guard's tests, and `docs/design/fleet-health.md`.
**[W-2]** "If Mars is unreachable, say so in the result rather than skipping silently" leaves the under-a-minute timing criterion unsatisfiable with no defined fallback or pass condition — carry-forward from Round 1; the amendment addresses reproducibility (head SHAs) but not this. Recommend an explicit best-effort designation or a fallback dataset.
**[W-3]** "The whole speed-check set stays under 60 s in CI" has no CI-side assertion named; if it is only measured locally it is unverifiable at review time. Recommend stating where the CI timing is recorded.

### NIT findings
**[NIT-1]** #192's "Property test (proptest, if it's already in `Cargo.lock`; otherwise 1,000 seeded random inputs)" makes the criterion's shape depend on repo state; fine, but the plan should record which branch was taken so the wave-1 test is unambiguous.

### Verdict
PASS — no BLOCK findings; implementation may proceed.