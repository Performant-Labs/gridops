## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** Migration numbering: that `0047_gitlab.sql`, `0048_fleet_target_retire.sql`, `0049_fleet_inventory_snapshot_retention.sql` are the last three and no `0050` exists — an `ls migrations` listing or repo tree at `83149f5`.
**[NV-2]** D-1's insert-site claims: `process_workflow_job` at `webhooks.rs:~645`, `upsert_workflow_job` at `reconciler/main.rs:~2133`, `UPSERT`/`record_gitlab_job` at `gitlab/jobs.rs:~155/:202`, that the `autoscaling.rs` INSERTs are all `#[cfg(test)]`, and that `webhooks/gitlab.rs` has no INSERT — the cited file:line excerpts.
**[NV-3]** `gitlab_jobs` in `0047` lacks `started_at`/`completed_at` and is not STRICT — the text of `migrations/0047_gitlab.sql`.
**[NV-4]** `vitest.config.ts`'s include is verbatim `["tests/**/*.test.ts"]` and would not collect `.test.tsx` or `src/` files — the file's `test` block.
**[NV-5]** D-4: `require_admin` exists at `fleet/profiles.rs:330-349` and the workload route uses `FleetUser<Read>` + it — that excerpt.
**[NV-6]** D-3(e): `gitlab/mod.rs:22-26` shows the mod-declaration precedent — that excerpt.
**[NV-7]** D-3(f): `RegistryService::list_hosts`' `open_snapshot` refuses principals without a user id, and the named helpers are private in `registry/queries.rs` — those excerpts.
**[NV-8]** Registration-point claims: `fleet/mod.rs` router ends in a `/api/v1/fleet/{*path}` catch-all; `reconcile()` sequences `fleet::tick`/`cleanup_retention`; `db.rs` has `SLOW_STATEMENT = 100 ms` with `.log_slow_statements(...)`; `navigation.ts` is a `NavGroup[]` array — the cited files.
**[NV-9]** `webhooks/gitlab.rs` "already reads" `build_name` and `project.path_with_namespace`, and the GitLab job API exposes `name` and project path to the poll — the corresponding source excerpts.
**[NV-10]** `axum` behavior: "specific route matched first regardless of order" vs the trailing catch-all — axum routing documentation/source.
**[NV-11]** The Mars recipe (`VACUUM INTO` via throwaway nginx container, ~93 s at 1.5 GB, last run 2026-11-15) — the project memory entry itself; the brief itself flags it as not dry-run.

### WARN findings
**[W-1]** Single-story scope is very large (migration + schema changes to live GitLab ingestion path, guard script, ~13 contract-test files, source-trait refactor, principal-free registry read, two throwaway PRs, a 2.9 GB DB timing run). Consider explicitly sequencing the riskiest verifications (migration timing, D-3(f)) early in the plan so failure doesn't strand the rest.
**[W-2]** "The migration applies in under a minute" is measured on a borrowed Mac under unknown load; the criterion records load but sets no pass condition for it (e.g., retry under low load vs. accept). Define what load conditions make the number valid.
**[W-3]** "Prove it by removing all markers on a scratch branch" — specify the proof artifact (CI run link, command output in PR description), otherwise it's unverifiable after the fact.
**[W-4]** Guard test-PR procedure requires re-opening both PRs after any post-HEAD change to the guard/owners/contract files (including rebases); this is easy to miss late. Recommend the plan carry an explicit checklist item keyed on merge-to-branch time.
**[W-5]** "Whole speed-check set stays under 60 s in CI" — no stated measurement method (CI job timing? local timed run?). Name where the number is read from.

### NIT findings
**[NIT-1]** "Migration number … re-check with `ls migrations | sort | tail -1`" sorts lexically, which breaks past 0099; harmless today, but note it.
**[NIT-2]** The brief says the owners file "starts from this table" but D-1/D-2/W-5 already amend it in three places; the plan should print the final table once rather than leave the delta implicit.

### Verdict
PASS — no BLOCK findings; implementation may proceed.