# Decision journal: #197 fleet health shared contract

## Phase 3 Architecture (A, up-front plan review) - 2026-11-22T13:44:49-08:00
- **Decided:** BLOCK, with 4 block and 9 warn findings (`docs/handoffs/197/handoff-A.md`).
  - **Block 1 [correctness]: the #190 insert sites are wrong.**
    - Every `INSERT INTO workflow_jobs` in `autoscaling.rs` is inside `#[cfg(test)]`.
    - The real poll writer is reconciler `main.rs:2133` (`upsert_workflow_job`), a file wave 1 must not edit.
    - The GitLab writer is core `gitlab/jobs.rs` (`UPSERT`), not api `webhooks/gitlab.rs`.
    - GitLab rows carry no job name to classify on.
    - Fix: #197 wires `category` at all three real writers through a #190-owned seam, and the brief decides the GitLab name and timing columns.
  - **Block 2 [correctness]: Vitest contract tests next to their components would never run.** `vitest.config.ts` includes only `tests/**/*.test.ts`. Fix: put them at `tests/fleet-health-<issue>-*.contract.test.ts`, in the createElement style.
  - **Block 3 [completeness]: shared-file hooks are missing from the brief.**
    - the `AppState` field (`state.rs`) and the fake's seams for #194's cache tests;
    - a fleet-wide `queue_breakdown` (it is `pub(super)` and user-scoped today);
    - the Overview tile wiring (`index.tsx:122`, the `MetricTile.to` union);
    - a 422 error variant;
    - contract-test `mod` lines placed in #197-owned parent files.
  - **Block 4 [correctness]: the health endpoint's auth.** "Match `query::hosts`" (principal-scoped) over a fleet-wide, per-window shared cache exposes data to callers who shouldn't see it. Fix: admin-only, like `/api/v1/fleet/workload` (`require_admin`).
  - **The warns:**
    - (5) the job-category routes go in `session_routes()`, not `fleet::router()`;
    - (6) `#[async_trait]` trait plus `Arc<dyn>`; `testdata.rs` and the speed-check helper behind `test-support`;
    - (7) `run_pass(db, now)`, called next to `fleet::tick`, before the manager early return;
    - (8) the speed-check helper's isolation, and timing transactions from outside;
    - (9) add the `regex` crate and a per-rule error API; `proptest` is absent, so #192 uses seeded inputs;
    - (10) the STRICT primary key needs a non-NULL unmanaged sentinel;
    - (11) the fleet zod transport and CSS-token chart theme;
    - (12) a unique nav shortcut and the double-active highlight;
    - (13) a forecast that #204 will need the windowed reader in core.
- **Assumed:**
  - With no `survey.md`, the analogues are the #137 GitLab contract, `/api/v1/fleet/workload` end to end, and #152's retention.
  - The issue bodies fetched on 2026-11-22 are current.
  - The runner manager's early return at `main.rs:641` is the only path in `reconcile()` that skips later steps.
- **Hedged:**
  - Block 4 is a product-visible decision (who may see the page). I chose admin-only because the codebase's only fleet-wide aggregate (`workload`) is admin-only and its UI says "Only administrators can see machines". If the operator wants non-admin access, the cache has to be keyed by visibility instead.
  - Warn 13 deliberately keeps the issue paths for wave 1 and defers relocation to #204, because changing paths now would contradict the wave-1 issue texts the GLM agents read.
  - The four blocks stop the automated run for a human amendment. I chose block over warn because each one either puts wrong files into the guard's owners file, makes contract tests silently not run, or forces a wave-1 edit to an unowned file. A Phase 7 review can't undo any of these once wave 1 starts.
- **Evidence:**
  - **Read:**
    - the brief, both brief-gate results, and issues #197, #198, #190-#195 and #199-#204 (`gh issue view`);
    - `crates/gridops-core/src/{lib.rs,db.rs,autoscaling.rs (test-module bounds),gitlab/mod.rs,gitlab/jobs.rs,fleet/mod.rs}`;
    - `crates/gridops-api/src/{main.rs,state.rs,error.rs,webhooks.rs:600-720,fleet/mod.rs,fleet/boundary.rs,fleet/query.rs,fleet/profiles.rs:330-360,fleet/openapi.rs,resources/overview_queue.rs,resources/gitlab_tests.rs (header)}`;
    - `crates/gridops-reconciler/src/main.rs` (`run_pass`, `reconcile`, `upsert_workflow_job`, `cleanup_retention`) and `snapshot_retention_tests.rs`;
    - `migrations/0047_gitlab.sql`, the 0022 `fleet_hosts` / `host_samples` DDL, and the `migrations/` listing on the branch, origin/master and every worktree (0050 is free);
    - `vitest.config.ts`, `tsconfig.json`, `eslint.config.js`, `package.json`, `tests/api-tokens-section.test.ts`, `tests/navigation.test.ts`;
    - `src/lib/navigation.ts`, `src/features/dashboard/metric-tiles.ts`, `src/features/fleet/{admin-api.ts,query-api.ts,fleet-workload-section.tsx}`, `src/styles/app.css` tokens, `src/routes/_app/fleet.index.tsx`.
  - **Checked:**
    - `Cargo.lock`: no `regex`, no `proptest`.
    - `sqlite3`: a STRICT primary key rejects a NULL `pool_id`, and `window` works as a column name.

## Phase 3 Architecture (A, up-front plan review, round 2) - 2026-11-22T14:01:08-08:00
- **Decided:** BLOCK, with 1 new block and 7 new warns (`docs/handoffs/197/handoff-A.md`, rewritten). Warns 5 to 13 from round 1 are carried unchanged under their numbers, because the brief adopts them "as written in `handoff-A.md`".
  - **Resolved:** all four round-1 blocks (R-1 to R-4), by D-1 to D-4, checked against the code at 5c100ab.
  - **Block 14 [new, completeness]: #201 can't reuse the `/fleet/hosts` host reads.**
    - #201 is told to call the functions `/fleet/hosts` uses for budget, allocation, latest root measurement and counts.
    - They are private `async fn`s in core's private `fleet/registry/queries.rs` (`:1209,1243,1309,1321,1766`). The only public way in is `list_hosts(principal, …)`, and its `open_snapshot` refuses any principal without a user id.
    - `collect_live(db, now)` has no principal, and #201 owns only `health_live.rs`.
    - Fix: a D-3(f). Either #197 exposes a principal-free, fleet-wide host read in core built from the same helpers (recommended), or the brief records in writing that #201 uses its own bounded queries. Either way, #201 gets a correcting comment.
  - **The new warns:**
    - (15) the classifier seam takes the caller's `&mut SqliteConnection`, never the pool, because three of the four writer paths call it inside a write transaction (one is `BEGIN IMMEDIATE`); category is bound on insert only;
    - (16) extend D18's module-doc rules and `jobs_tests.rs`; the serde default goes on `JobResponse`; the `WHERE` guard must admit name and path fills;
    - (17) close D-3(b) to a fleet-wide `pub(crate)` variant in `overview_queue.rs`; a "signature #201 fills in" would be a parallel queue path;
    - (18) the cache's clock and the health-state type, with its default source, live in `fleet/health.rs` (`RegistryClock` / `with_clock` shape), not on the fake;
    - (19) accept `fleet:read` like `workload`, and wire the token-scope table, docs and OpenAPI;
    - (20) the 422 body carries per-rule positions;
    - (21) record D-4 in the design doc, and comment on #194 and #201 as well as #190.
- **Assumed:**
  - This is round 1 of a fresh run (the task text carried no re-review note), so completeness findings still block. I listed the round-1 blocks under "Resolved findings" anyway, so a carry-forward would match them.
  - The issue bodies fetched on 2026-11-22 are still current.
- **Hedged:**
  - Block 14 is a gap I missed in round 1; it is the same class as round-1 block 3(b). I chose block over warn for two reasons. Nothing in #197's stub or contract tests would bring it to light, so Phase 7 would not catch it either. And if it is left, #201 has to edit unowned core files or stop and ask.
  - Warns 15 to 21 are fixes inside #197's own diff, which Phase 7 reviews before wave 1 starts, so they don't block.
- **Evidence:**
  - **Read:**
    - the amended brief (5c100ab), round 1's `handoff-A.md` and `decisions.md`, and issues #197, #190, #191, #192, #193, #194, #200, #201 and #204;
    - `crates/gridops-core/src/{db.rs,gitlab/mod.rs,gitlab/jobs.rs,gitlab/http.rs (Job/JobResponse),fleet/registry/queries.rs (list_hosts, open_snapshot, host_projection and its helpers),fleet/registry/mod.rs,fleet/authorization.rs (Principal),fleet/workload.rs}`;
    - `crates/gridops-api/src/{state.rs,error.rs,webhooks.rs:600-720,webhooks/gitlab.rs:320-400,fleet/query.rs,fleet/profiles.rs:320-365,fleet/mod.rs:30-140,resources.rs:80-100,resources/overview_queue.rs}`;
    - `crates/gridops-reconciler/src/{main.rs (sync_run_jobs, upsert_workflow_job, the job check, reconcile),gitlab_poll.rs}`;
    - `vitest.config.ts`, `tsconfig.json`, `tests/api-tokens-section.test.ts`, `src/features/dashboard/metric-tiles.ts`, `src/features/fleet/fleet-workload-section.tsx`, `src/routes/_app/index.tsx`, `src/lib/navigation.ts`.
  - **Checked:**
    - `grep`: the production `INSERT INTO workflow_jobs` sites;
    - `grep`: tokio `test-util` is only in the manager and tart-agent crates;
    - `grep`: no public, principal-free reader of `capacity_allocations` or the latest `host_samples` row exists in core.

## Phase 3 Architecture (A, up-front plan review, round 3) - 2026-11-22T14:23:09-08:00
- **Decided:** PASS, with 0 block findings and 6 new warns (`docs/handoffs/197/handoff-A.md`, rewritten). Warns 5 to 13 and 15 to 21 are carried unchanged under their numbers, because the brief cites them by number.
  - **Resolved: round-2 block 14, by D-3(f).**
    - Every `host_projection` helper takes a transaction plus a `Snapshot`.
    - An admin snapshot passes every `?=1` gate and takes `CountScope::Host`, so "equals `list_hosts` for an admin" is testable.
    - `start_health::start_gate` and `disk::alarm_for` are already public.
  - **The new warns:**
    - (22) `src/lib/api.ts`'s `api()` drops `details` and `code` from error bodies, so D-3(d)'s per-rule 422 can't reach #199 unless #197 extends the browser `ApiError`. This corrects W-20's client-side half.
    - (23) D-3(f) should be a free function (`pool`, `now`), because `RegistryService::new` needs a `Vault` and #201 fixes `collect_live(db, now)`.
    - (24) D-3(b) is the first edge from `fleet/` into `resources`. Return a plain error rather than `ApiResult`, add queued counts by category, and record the edge.
    - (25) Nothing indexes `category IS NULL`, and the applied rules version has nowhere to live. 0050 or the design doc must decide both, because #190 can't add a migration.
    - (26) D-3(a)'s first bullet still puts the clock on the fake.
    - (27) The fleet `request` transport is private in each module, so `fleet-health/api.ts` gets its own copy, and the three existing copies are left alone.
- **Assumed:**
  - This is a fresh pass on the amended brief. The task text names no earlier BLOCK findings to mark, so I list round 2's block under "Resolved findings".
  - The issue bodies are unchanged: their last update was at 12:54 PM PST (20:54 UTC), before round 1.
- **Hedged:**
  - **Warn 22 is a warn, not a block, even though it is the same class as block 14 (a wave-1 issue would have to edit a shared file).**
    - It needs no decision in the brief: the one fix is a few lines in `src/lib/api.ts`, inside #197's own diff.
    - I put it on the Phase 7 check list, and I block it there if it is missing.
    - Round 2 used the same criterion for warns 15 to 21. Block 14 needed a choice between two designs, written into the brief.
  - Warn 24 qualifies my own round-2 W-17 rather than reversing it. The placement stays; only the return type and the category split change.
- **Evidence:**
  - **Read:**
    - the amended brief (1867382), both brief-gate results, round 2's `handoff-A.md` and `decisions.md`, and issue bodies #197, #190, #199 and #201 (`gh issue view`; update times for all 13 issues);
    - `crates/gridops-core/src/fleet/registry/{mod.rs:1-80,queries.rs (list_hosts, open_snapshot, readable_targets_cte, visible_host_ids, host_visibility, allocation_read, counts, root_budget, latest_measurement, measurement_from_row, sample_provenance, placement_failures, host_projection head)}`;
    - `crates/gridops-core/src/gitlab/{mod.rs:1-40,130-160,jobs.rs:1-60}`;
    - `crates/gridops-api/src/{state.rs:1-160,error.rs,fleet/mod.rs:1-160,fleet/profiles.rs:68-95,320-365,resources.rs:70-100,resources/overview_queue.rs:1-20,200-332,webhooks.rs:622-690}`, and reconciler `main.rs:2133-2150`;
    - `src/lib/api.ts`, `src/features/fleet/{admin-api.ts:1-60,registry-api.ts (exports),query-api.ts (exports),contracts.ts:128-141}`, `src/features/operations/operations.functions.ts:229-244`, `src/routes/settings.general.tsx` (imports, save path), `src/features/dashboard/metric-tiles.ts`, `src/routes/_app/index.tsx`, and the `src/routes` listing;
    - `.github/workflows/ci.yml` (triggers and jobs).
  - **Checked:**
    - `grep`: api `fleet/` has no `crate::resources` import today;
    - `grep`: `ApiError` from `~/lib/api` is read only for `status` (`router.tsx:15`, `job-log-viewer.tsx:126`);
    - `grep`: the `CREATE INDEX` listing across `migrations/`;
    - `workflow_jobs.id` is an integer primary key (`0001_initial.sql:220`);
    - `ls migrations`: 0049 is still the last.

## Phase 4 Tester (T, author/RED) - 2026-11-22T15:13:02-08:00

- **Decided:**
  - The suite has two parts. #197's own tests are live now: schema 0050, shared types and fixture, `category.rs`, `merge_concerns`, the fake, the speed check and `build_week`, the wiring hooks, the guard, and the shared frontend modules. Each wave-1 "Done when" is a contract test with its issue's marker.
  - Rust contract files sit next to their module and are mounted from #197-owned parents: api `fleet/mod.rs`, `webhooks.rs`, `resources.rs` and the reconciler `main.rs`, all done here. The core `fleet_health/mod.rs` is F's to create, with the mount lines in the handoff. Frontend contract tests are in `tests/fleet-health-<issue>-*.contract.test.ts` (D-2). The jsdom helpers are in `tests/fleet-health-dom.ts`.
  - The rollups get a per-hour entry point, `rollup_hour(db, hour, now)`, next to `run_pass`. Without it, the tests would depend on the catch-up order.
  - The `fleet_health_hourly` counts are pinned. `job_count` is every final-attempt job completed in the hour; `completed_count` is the ones that succeeded. Unmanaged jobs are stored as `'unmanaged'/'unmanaged'` with 0 CPU-seconds.
  - The guard takes the PR's changed files in GitHub's list-files shape (`filename`, `status`, `previous_filename`, `patch`), so it stays a pure function.
- **Assumed:**
  - `upsert_workflow_job` keeps its signature and reads the workflow name and repository from the run (#190 step 3).
  - The fleet health components use only the router's `Link`, which the tests mock.
  - A run pass rolls up every listed hour plus at most one catch-up hour.
- **Hedged:**
  - The tests over types that don't exist yet were only checked as far as name resolution. Any test-side type error F finds is a test-only rework for me, not something F changes.
  - The persisted rules version (warn 25) is left to F. The tests pin only that a re-run changes nothing and that a rule change reclassifies 7 days.
  - Placeholder-card rendering of `/fleet/health` has no Vitest test, because a page-composition test would break when wave 1 fills the cards. It is a coverage hole for Phase 6 and U.
- **Evidence:**
  - Runtime RED:
    - `cargo test -p gridops-core --test fleet_health_schema`: 0 passed, 5 failed (no `job_category_rules`, no `category`, no `completed_at` index);
    - `--test fleet_schema`: 3 upgrade tests failed on the 0050 columns, 14 passed;
    - the vitest navigation test failed on `/fleet/health`.
  - Compile RED:
    - `cargo test --no-run` for core, api and the reconciler: every error is a missing #197 item (`fleet_health`, `host_live_reads`, the `JobObservation` fields, `fixture.json`, `health_live`, `fleet_queue_breakdown`, `with_fleet_health_source`, `ApiError::Invalid`, the reconciler modules);
    - vitest and `node --test`: missing modules.
  - Fixed before declaring RED: `AssertSqlSafe` in the seed SQL, and 4 lines corrupted by output redaction.
  - Clean: `eslint` on the new tests, and `cargo fmt`.

## Phase 5 Feature Implementor (F) - 2026-11-22T16:52:00-08:00

- **Decided:**
  - #197 lands as one pass with no scope split. The brief requires one PR with every shared hook so that wave 1 can run in parallel; A passed that plan, and T's suite covers the whole scope.
  - The shared hooks are stubs and placeholders:
    - the job-category handlers check for a system admin, then return 501;
    - `collect_live`, `DbFleetHealthSource` and the four reconciler passes return empty or `Ok`;
    - the frontend sections are "Coming in #N" cards.
  - `useFleetHealth` keeps component state rather than using react-query, because an existing Overview test renders with no `QueryClientProvider`.
  - The 422 body is a new OpenAPI schema, `ApiInvalidBody`. `ApiErrorBody`'s fields are pinned by a test.
  - `fleet_queue_breakdown` shares `queue_breakdown`'s helpers (refactored to extend, behaviour unchanged) instead of copying them.
  - The synthetic week's batches are 75 jobs and 500 samples per transaction, so that the 50 ms write-lock limit holds under a parallel workspace run.
- **Assumed:**
  - A GitHub job is classified once, at first insert. `ON CONFLICT` leaves `category` alone, and #199's 7-day reclassification is the path for a rule change.
  - The health endpoint reads the 24 h window until a wave adds window selection.
- **Hedged:**
  - Some test-side fixes are T's (`handoff-F.md`, "Tests that look wrong"): items 1 and 2 stop the core, api and reconciler test targets from compiling. I verified GREEN with local copies of those fixes, now reverted and never staged.
  - The guard's live sample PRs and the Mars database timing are left to O, because they need a push or PR, and production ssh.
- **Evidence:**
  - Tier 1 on 2026-11-22:
    - `cargo fmt --check` and the 1.98.1 clippy run on production targets are clean.
    - The full workspace test run is green apart from T's flagged tests and the manager's SUN_LEN environment failures.
    - The speed check passed twice under load after the batch change (8.7 s and 8.6 s).
    - Vitest: 638 passed. `eslint` and `tsc` are clean.
    - The guard: 13 of 13. The OpenAPI document is valid.
  - Source facts: `docs/handoffs/197/evidence.md` (9 entries).

## T (Phase 7, GREEN + Tier 2)

- **Decided:** I repaired every item under F's "Tests that look wrong" myself, test-only. The marked contract tests are left marked. The webhook test (item 6) now asserts the W-16 name fill rather than whole-row equality, because W-16 says a later event fills a missing name.
- **Assumed:** the 14 `gridops-manager` SUN_LEN failures are environmental (macOS socket path length in a long worktree path) because the crate is untouched on this branch.
- **Hedged:** two contract tests pass against the stubs (`no_score_is_ever_outside_0_to_100_on_1000_seeded_inputs`, `a_machine_with_no_cpu_budget_never_raises_host_offline`) and I kept them as guards for the real code.
- **Evidence:** the CI clippy command is clean; the workspace run was 1137 passed with only the manager failures; vitest 638 passed, 18 skipped; guard 13 of 13; the marked contract tests all fail against the stubs when run (core 16/17, api 18/19, reconciler 11/11, frontend 18/18). Handoff: `docs/handoffs/197/handoff-T-green.md`.

## Phase 3 Architecture (A, up-front plan review, round 4) - 2026-11-22T17:14:19-08:00
- **Decided:** PASS, with 0 block findings and 1 new warn (`docs/handoffs/197/handoff-A.md`, rewritten). The round 1 to 3 warns are carried word for word under their numbers, because the brief cites them by number.
  - **Why this pass ran:** the diff gate's round 1 BLOCKed (`docs/handoffs/197-diff-result-r1.md`) and F had reported `archChanged`, so the run was routed back here. The brief is unchanged since round 3 (1867382).
  - **Checked against the code at af735a4:**
    - in place: W-15, W-16, W-18/W-26, W-22, W-23, W-25 and W-27;
    - W-17/W-24 is half done: the shared helpers and the per-category counts are there, but the `ApiResult` return and the unrecorded `fleet → resources` edge are still open, and stay Phase 7 checks.
  - **Warn 28 (new): the page-render test has nothing to assert on that survives wave 1.**
    - A #197 test expecting "Coming in #N" breaks when #195, #202 or #203 fill their sections, and the guard refuses a wave-1 edit to a #197 file. That is why T left the test out and the diff gate blocked (B-2).
    - Fix, inside #197's diff: stable `data-health-section` slots in `HealthSections`, listed in the design doc as part of the composition; a #197 page test on the slots and the 403/failure states; no placeholder text asserted through the page.
- **Assumed:**
  - This is a fresh pass with no earlier BLOCK to mark: my last verdict (round 3) was PASS, and the task text names no findings.
  - F's `archChanged` covers what its handoff lists under "Architecture notes for A". Its structured result isn't kept on disk.
- **Hedged:**
  - **Warn 28 is a warn, not a block, on the same rule as warn 22.**
    - Under the ownership model, one design fits: hooks in the #197-owned composition.
    - It needs no choice written into the brief.
    - A BLOCK on an unchanged brief would stop the run for a human amendment that adds nothing this handoff doesn't already say.
    - I check it at Phase 7.
  - **The diff gate's B-1 (the marker-removal proof) is test verification, so I take no position on it.** I only noted, for O, that T-green kept two contract tests that pass against the stubs.
- **Evidence:**
  - **Read:**
    - the brief, both brief-gate results, `197-diff-result-r1.md`, `handoff-F.md`, this file, and round 3's `handoff-A.md`;
    - the routing and the Phase 4/7 driver code in the release's `coding-pipeline.workflow.mjs`;
    - core `fleet_health/{classifier_cache.rs,category.rs (exports),fake.rs (seams)}`, `gitlab/jobs.rs` (diff) and `fleet/registry/{mod.rs,queries.rs}` (diff);
    - api `webhooks.rs` (diff), `fleet/health.rs`, `state.rs` (diff), and `resources.rs` and `resources/overview_queue.rs` (diff);
    - reconciler `main.rs` (diff);
    - `src/lib/{api.ts,navigation.ts}` (diff), and `src/features/fleet-health/{api.ts,page.tsx,placeholder-card.tsx,machine-score-table.tsx,live-strip.tsx}`;
    - `docs/design/fleet-health-owners.json`, and `docs/design/fleet-health.md` (the live half, the page, the rules version).
  - **Checked:**
    - `grep`: no caller of `process_workflow_job` holds a transaction;
    - `grep`: no frontend contract test pins a section root;
    - `grep`: 0050's `CREATE INDEX` lines;
    - `grep`: the design doc has no `fleet → resources` edge note.

## Phase 4 Tester (T, author/RED, round 2) - 2026-11-22T17:32:03-08:00
- **Decided:**
  - **B-2 is met with warn 28's shape.** `tests/fleet-health-page.test.ts` (#197-owned, live) renders `FleetHealthPage` with `fetch` stubbed. It checks:
    - six `data-health-section` slots, as a set (order is #195's), each with content;
    - the `24h` request, and the picker's `7d` request with `aria-pressed`;
    - "Administrators only" on a 403;
    - a failure with Retry that recovers.

    No placeholder text is asserted.
  - **B-1:** the two contract tests that passed against the stubs now carry positive controls:
    - #192's property test must hit both clamps and combine at least once;
    - #201's no-budget test must see a budgeted machine, silent just as long, go offline in the same call.
  - **The #195 remembered-window contract test is repaired, test-only:**
    - it now uses `installMemoryStorage()` (Node 26 hides jsdom's `localStorage`);
    - it checks that the documented key is written;
    - its storage-failure leg stores a valid value first.
  - **W-4 is a doc comment in `health_wiring_tests.rs`, not a test.** Pinning today's 200 would break when #194 lands.
- **Assumed:**
  - The slot names (`live-strip`, `areas-of-concern`, `fleet-score`, `category-legend`, `machine-table`, `machine-detail`) are mine to set. A's warn 28 names the mechanism, not the values. F records them in the design doc.
  - The `gridops-manager` SUN_LEN failures are environmental, as in T-green.
- **Hedged:**
  - The 403 page test passes before F's change, because that state was already built in round 1. It is a regression guard, not RED. The failure test, which says no "Administrators only" on a 500, keeps it discriminating.
  - The Mars timing (W-2) is left to O. So are the guard's sample PRs (W-1), which must now be cut after this round's contract-test edit.
- **Evidence:**
  - **Marker-removal proof:** run on the scratch branch `scratch/197-markers-removed`, which was removed afterwards. The workspace gave 1138 passed and 61 failed. The 61 are 47 of 47 Rust contract tests (core 17, api 19, reconciler 11) and 14 manager SUN_LEN failures. Vitest: all 18 contract tests and the 3 page slot tests fail.
  - **Round 1 and T-green counts:** "57/22" from `grep -c` counted doc comments. The real counts are 47 and 18.
  - **With markers in place:**
    - vitest has 3 failures (the slot tests), 639 passed and 18 skipped;
    - `tsc`, `eslint`, `cargo fmt` and CI's clippy are clean;
    - the guard passes 13 of 13.
  - **A correct implementation can pass the new tests:** a throwaway one on the scratch branch passed the page tests and the repaired #195 test.
  - **Handoff:** `docs/handoffs/197/handoff-T-red.md` (round 2 at the top).

## Phase 6 Feature Implementor (F, round 2) - 2026-11-22T17:40:00-08:00
- **Decided:**
  - The six `data-health-section` slots go on wrapper `div`s in `page.tsx`, not on the sections' roots. The section files belong to wave 1, and a wrapper survives a rewrite.
  - Each wrapper gets `min-w-0`, because two of the slots are now grid items.
  - The slot names are typed by a private `HealthSlot` union, not exported, so the test and the design doc pin the names on their own.
  - W-3 is a non-goal sentence after the existing "Limits:" sentence in the design doc.
- **Assumed:**
  - T sets the slot names, per T-red round 2. The design doc now records them as part of the composition.
- **Hedged:**
  - W-1, W-2 and W-5 are left to O, and the NITs to the wave-1 issues that touch those lines.
  - No Rust changed this round, so cargo was not re-run.
- **Evidence:**
  - The page test passes 4 of 4 (it was 3 failing before).
  - `npx vitest run`: 642 passed, 18 skipped.
  - `npm run lint`, `tsc --noEmit` and `npm run build` are clean.
  - The guard passes 13 of 13.
  - Handoff: `docs/handoffs/197/handoff-F.md`, round 2 at the top.

## Phase 7 Architecture Reviewer (A, anti-duplication gate) - 2026-11-22
- **Decided:**
  - PASS. F extended `queue_breakdown`, `list_hosts`' helpers, D18's UPSERT, `RegistryClock`, `ApiError`, the `FleetWorkloadSection` fetch pattern and `HealthSections`. No parallel path.
- **Assumed:**
  - No survey Reuse map exists, so I used the analogues named in `handoff-A.md` and the brief.
- **Hedged:**
  - Three warns, none blocking: the 90/70 thresholds duplicated from `successTone`, `fleet_queue_breakdown` still returning `ApiResult` with the `fleet → resources` edge unnamed in the design doc, and `metricTiles`' unused `_health` hook.
- **Evidence:**
  - Read the diff for `overview_queue.rs`, `resources.rs`, `error.rs`, `state.rs`, `db.rs`, `registry/queries.rs`, `health.rs`, `health_live.rs`, `api.ts` and both `chart-theme.ts` files.
  - Handoff: `docs/handoffs/197/handoff-A-dup.md`.

## Phase 6 Feature Implementor (F, round 3, S rework) - 2026-11-22T17:55:00-08:00
- **Decided:**
  - S finding 1: `NavLinkItem` passes `activeOptions={{ exact: !active }}`, so the router's active state (and its `aria-current`/`data-status`) follows `isActivePath`.
  - Kept the app's own `aria-current` prop as the stated intent.
  - S finding 3: deleted the unused `escape` from the guard script.
- **Assumed:**
  - No two nav items share a `to` (pinned by `tests/navigation.test.ts`), so an app-inactive item is never on its own exact path.
- **Hedged:**
  - S finding 2 and the render-level nav test are T's (F writes no tests). Both are listed under "Tests missing (for T)" in `handoff-F.md`.
- **Evidence:**
  - Scratch jsdom render of the real `AppShell` (deleted after): `/fleet/health` had two current links before the fix and one (Health) after. `/fleet` and `/fleet/machines/abc` give Machines.
  - `tsc`, lint and vitest (642 passed, 18 skipped) are clean, and the guard passes 13 of 13.
  - Handoff: `docs/handoffs/197/handoff-F.md`, round 3 at the top.


## Phase 7 Tester (T, round 3, S rework) - 2026-11-22T18:15:00-08:00
- **Decided:**
  - Authored the three tests S's REWORK and F's handoff left to T: a jsdom render of the real `AppShell` (`tests/app-shell-nav.test.ts`), and the #193 and #201 realistic-volume contract tests (marked, over `build_week`, under `speed_check::check`).
  - Each Rust test has a positive assertion, so it fails against its stub once unmarked.
  - #193's index thresholds are loose (Jupiter above 1.2, the rest below 1.1) so both reasonable median definitions pass.
- **Assumed:**
  - The 14 `gridops-manager` `SUN_LEN` failures are environmental: the branch does not touch that crate.
- **Hedged:**
  - The speed-check set is now five `build_week` callers, about 45 s of the 60 s budget. A smaller week is the lever if CI is tight.
- **Evidence:**
  - Nav test fails 1 of 5 (`/fleet/health`) with F's fix reverted; both Rust tests fail against the stubs on the stub's assertion.
  - `vitest` 647 passed, 18 skipped; `tsc`, lint, build, guard 13 of 13, fmt and CI's clippy (1.98.1) clean; cargo 1138 passed, 14 manager `SUN_LEN`.
  - Handoff: `docs/handoffs/197/handoff-T-green.md`, round 3 at the top.

## A-dup (round 3, S-rework delta caa663b..f8b8b37) — PASS
Delta extends `NavLinkItem`, the existing contract-test files and the guard script in place. No parallel path. Two warns carried (shared `matchMedia` shim, chart-theme threshold copy).
