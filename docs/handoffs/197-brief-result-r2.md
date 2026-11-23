## Brief Review (Round 1)

### BLOCK findings
None.

### Resolved findings
**[R-1]** "Two parts of the brief disagree about where frontend contract tests live and how many there are per issue. The acceptance-criteria section states: '…live in `*_contract_tests.rs` / `*.contract.test.ts(x)` files **next to the module they test**, **one per wave-1 issue**.' D-2 then says… a reviewer of the eventual diff cannot tell which one to enforce." — The brief now carries an explicit "**D-2 supersedes the copied acceptance text for frontend contract tests**" paragraph: the placement, `.ts` extension, and "one or more per issue" rules are stated as overriding the copied sentence, with the Rust placement rule restated (including the #190 per-crate exception, which also resolves NIT-1). The contract is now internally consistent.

### Needs-verification findings
**[NV-1]** Claim: the `INSERT INTO workflow_jobs` statements in `crates/gridops-core/src/autoscaling.rs` are all inside `#[cfg(test)]` and are not production insert sites (D-1 contradicts the copied issue table) — settle with `crates/gridops-core/src/autoscaling.rs` around the `INSERT INTO workflow_jobs` occurrences.
**[NV-2]** Claim: `crates/gridops-api/src/webhooks/gitlab.rs` contains no `INSERT` and already reads `build_name` and `project.path_with_namespace` — settle with that file's text.
**[NV-3]** Claim: the third production writer is `upsert_workflow_job` at `crates/gridops-reconciler/src/main.rs:~2133` and `record_gitlab_job`/`UPSERT` at `crates/gridops-core/src/gitlab/jobs.rs:~155/:202`, and that `gitlab::Job`/`JobObservation` currently carry no job name or project path — settle with those file excerpts.
**[NV-4]** Claim: `vitest.config.ts`'s include is exactly `["tests/**/*.test.ts"]` (the basis of D-2's `.ts`-not-`.tsx` rule) — settle with `vitest.config.ts`.
**[NV-5]** Claim: `require_admin` exists at `crates/gridops-api/src/fleet/profiles.rs:330-349` in the `fleet/workload` pattern, and `RegistryService::list_hosts`'s `open_snapshot` refuses a principal with no user id — settle with `fleet/profiles.rs` and `fleet/registry/queries.rs` excerpts.
**[NV-6]** Claim: `queue_breakdown` in `resources/overview_queue.rs` is `pub(super)` and user-scoped — settle with the file.
**[NV-7]** Claim: `migrations/0047_gitlab.sql` gives `gitlab_jobs` only `status_changed_at, created_at, updated_at` and the table is not STRICT — settle with the `0047_gitlab.sql` table definition.
**[NV-8]** Claim: axum matches the specific `/api/v1/fleet/...` route before the `{*path}` catch-all regardless of registration order — settle with axum routing documentation/source.
**[NV-9]** Claim (new, D-3(f)): the helpers behind `host_projection` (`physicalBudget`, `physicalAllocation`, `latestRootMeasurement`, placement-failure counts) are private in `crates/gridops-core/src/fleet/registry/queries.rs` and can be composed into a principal-free fleet-wide read — settle with `registry/queries.rs`.
**[NV-10]** Claim (new, D-4): the health route accepting admin `fleet:read` tokens "as workload does" — settle with the `TOKEN_SCOPED_ROUTES` table and `fleet/workload` handler excerpts in `crates/gridops-api/src/`.

### WARN findings
**[W-1]** The guard-check test-PR criterion ("fails on a test PR that edits a file outside an issue's list") still does not specify how the guard maps a PR/branch to a wave-1 issue (branch-name convention, PR body, issue reference). Without that mapping the two throwaway PRs are not deterministic and "outside an issue's list" is unenforceable. — Specify the mapping in `docs/design/fleet-health.md` and cover it in the guard's own tests.
**[W-2]** #190's "nothing is recompiled when nothing changed" criterion remains an implementation-side effect with no defined observable proxy. — Define the proxy (cache hit / rebuild-on-mtime-only) in the design doc before the contract test is written.
**[W-3]** The Mars recipe is still un-dry-run against a DB ~2× the size of the last measurement, and the "under a minute" number is taken on a loaded Mac with only `uptime` recorded (the on-failure deletion path is now specified, which resolves the earlier NIT). — If the run exceeds a minute under load, re-measure rather than treating the first number as final.
**[W-4]** Nothing in the prompt confirms that #190–#195, #199–#203 is the complete wave-1 set (e.g. whether #196 exists) — the brief asserts the issue list was read but the mapping is unverifiable here; if an issue is missed, the owners file and contract-test coverage will be incomplete. — Confirm against the epic before freezing the owners file.

### NIT findings
**[NIT-1]** D-3(b)'s superseded original text ("or a signature #201 fills in") is still present above "(b), closed"; the closed version is clearly later and marked, but a top-down reader could still implement the stub. Annotate the superseded sentence inline.
**[NIT-2]** The one-line deletion note now covers failure of the `VACUUM INTO`, `scp` and migration run, but not the case where the guard-check test PRs or scratch branches leave the 2.9 GB copy on disk between steps — a one-line "delete immediately after the migration run, not at the end of the story" would tighten it.

### Verdict
PASS — no BLOCK findings; implementation may proceed.