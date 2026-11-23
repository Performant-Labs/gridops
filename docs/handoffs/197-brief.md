# [Fleet health: shared contract (design doc, migration, types, fake source, OpenAPI)](https://github.com/Performant-Labs/gridops/issues/197)

Wave 0 of epic #198. The issue body is the spec: read it in full (`gh issue view 197 --repo Performant-Labs/gridops`), and read the epic (#198) and every wave-1 issue (#190, #191, #192, #193, #194, #195, #199, #200, #201, #202, #203, #204) for the "Done when" lists you must turn into contract tests and the "Owns" lists that go in `docs/design/fleet-health-owners.json`.

## Why this story is shaped the way it is

Eleven-plus wave-1 issues will be done in parallel by weaker agents, each in its own worktree. They only succeed if this PR lands **every** shared type, migration, hook and wiring line, so that no two wave-1 PRs touch the same file. When in doubt, wire it here. A wave-1 issue must never need to edit `main.rs`, any `mod.rs`/`lib.rs`, the API router, `src/lib/navigation.ts` or `page.tsx`'s composition.

## Corrections to the issue text

- **Migration number:** the issue says `0049`, but `0049_fleet_inventory_snapshot_retention.sql` already exists on master. Use the next free number (`0050` today; re-check with `ls migrations | sort | tail -1` before writing). Verified at `83149f5` on this branch: the last three are `0047_gitlab.sql`, `0048_fleet_target_retire.sql`, `0049_fleet_inventory_snapshot_retention.sql`.
- If any wave-1 issue's "Owns" list conflicts with the table in #197 (two issues owning one file, or a file missing from the stub table), resolve it in the plan, say which way and why, and make the owners file reflect it. Do not edit the GitHub issues.

## Build and machine constraints (the Mac)

- Rust builds share one target dir via the untracked `~/Projects/gridops/.cargo/config.toml`. Never set `CARGO_TARGET_DIR`, never commit `.cargo/`. "Blocking waiting for file lock" is normal.
- `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2` are set in `.claude/settings.local.json`. Prefer `-p <crate>` runs while iterating; run the full suite once at the end.
- Disk is tight (about 29 GB free). Don't leave large scratch files around.

## The Mars database timing check

"The migration applies to a copy of the Mars database in under a minute" needs a real copy. Get it read-only: on Mars (`ssh deploy@5.189.148.42`), use `VACUUM INTO` into a file under `/tmp` on the host via a throwaway container (see the project memory: `docker run --rm -v gridops_gridops-data:/data nginx:alpine` + `apk add sqlite`; `VACUUM INTO` took about 93 s last time, on 2026-11-15, when the DB was about 1.5 GB; #197 gives 2.9 GB now, so expect longer. The recipe is quoted from the project memory and has not been dry-run for this story), `scp` it to the Mac scratch area (the session's scratchpad directory, not the worktree, so it can't be committed), delete the remote copy, apply all migrations up to and including the new one with the project's normal migration path, and record the wall time (and the Mac's `uptime` load at the time) in the pipeline result and in the PR description's Done-when list, next to that item. Delete the local copy afterwards, and delete both the remote `/tmp` copy and the local copy even when a step fails (the `VACUUM INTO`, the `scp` or the migration run), before reporting the failure. Never write to the live DB, never restart a Mars service. If Mars is unreachable, say so in the result rather than skipping silently.

## Guard check test PRs

The guard maps a PR to a wave-1 issue from its body, as #197 §6 says: a `Closes #N` (also `Fixes`/`Resolves`, case-insensitive) naming a wave-1 issue in the owners file. A PR naming none of them is out of the guard's scope and passes. A PR naming two or more wave-1 issues fails. The guard's own tests cover all three cases.


The "Done when" asks for two throwaway PRs proving `Fleet health guard` fails (one editing a file outside an issue's list, one editing a contract test). Open them against `Performant-Labs/gridops` only (always pass `--repo Performant-Labs/gridops`, never upstream `iamngoni/gridops`), from branches off this story's branch, then close them and delete their branches. Cut them from the story branch's head after the last change to `scripts/fleet-health-guard.mjs`, `docs/design/fleet-health-owners.json`, any contract test file or the `Fleet health guard` job in `.github/workflows/ci.yml`; if any of those changes afterwards (including by a rebase or update of the story branch), open the two test PRs again from the new head and record both runs' head SHAs in the PR description.

## Acceptance criteria, copied from the issues (fetched 2026-11-22)

The issues stay the spec; these are copies so the brief can be judged on its own. If an issue changes after 2026-11-22, the issue wins and the plan says so.

### #197's own "Done when" (this story)

- The migration applies to a copy of the Mars database (2.9 GB) in under a minute.
- A test deserializes the UI fixture into the Rust types, so the doc, types, OpenAPI and fixture can't drift apart.
- `GET /api/v1/fleet/health` returns the fixture, and `/fleet/health` renders it with every placeholder card.
- The full workspace checks pass: `npm run check:rust`, `npm run test:rust`, `npm run lint`, `npm run typecheck`, `npm test` and `npm run validate:openapi`.
- Every wave-1 "Done when" test exists here, ignored or skipped with its issue's marker, and fails against the stubs once its marker is removed. Prove it by removing all markers on a scratch branch.
- `Fleet health guard` passes on this PR, fails on a test PR that edits a file outside an issue's list, and fails on one that edits a contract test. Both test PRs are closed afterwards.
- The speed-check helper fails a deliberately slow query in its own test.
- **Grep check:** for each wave-1 issue, every file in its "Owns" list exists, and no two wave-1 issues own the same file.

Also from #197's body: `merge_concerns` and `category.rs` are implemented and tested here; the guard script has its own tests; the whole speed-check set stays under 60 s in CI; contract tests are marked `#[ignore = "contract: #<issue>"]` (Rust) or `it.skip("[contract: #<issue>] …")` (Vitest), live in `*_contract_tests.rs` / `*.contract.test.ts(x)` files next to the module they test, one per wave-1 issue. (Placement and count are refined below: D-2 governs frontend contract tests, and #190 gets one Rust contract-test file per crate; see the note under D-2.) Make the grep check automatic: a test of `scripts/fleet-health-guard.mjs` reads `docs/design/fleet-health-owners.json` and fails when an owned file (after glob expansion) is missing or when two wave-1 issues own the same file.

### Wave-1 "Done when" lists to write as contract tests

Items marked *(not a test)* need real data or a human and stay as Done-when items in the wave-1 PRs, per #197 §5.

**#190 (job categories):**
- Cache test: a rule change is picked up within a minute, and nothing is recompiled when nothing changed.
- Ingestion tests: a GitHub webhook, a poll insert and a GitLab job event each store the expected category.
- Backfill test on 10,000 seeded rows: all classified, no write transaction over 50 ms, re-running changes nothing.
- Reclassify test: changing a rule reclassifies recent rows and marks their hours for recompute.

**#191 (hourly job rollup):**
- Tests with seeded jobs and placements give the expected rows, including an unmanaged runner, a job completing just after the hour boundary, a re-run (only the final attempt counts), a GitLab job, and a category of NULL (counted as `other`).
- Re-running an hour gives identical rows.
- A test seeds 7 days of jobs (50,000) and shows no write transaction over 50 ms.

**#192 (scores and windowed concerns):**
- Table tests for every component at 100, at 0, at the midpoint and beyond both ends.
- The not-enough-data reweighting test: the weights still sum to 100.
- Histogram percentile tests: a single bucket, uniform, and everything in the 40+ bucket.
- On #197's fixture: the slow host scores lowest on relative speed, the host with start failures lowest on reliability, and the fixture's expected concerns come out.
- Each concern rule fires at its threshold and not just below it.
- Property test (proptest, if it's already in `Cargo.lock`; otherwise 1,000 seeded random inputs): no score outside 0–100.

**#193 (speed index):**
- Tests: a machine running every key at 1.5× gets 1.5 (±0.01); keys under the 5-run minimum are ignored; fewer than 3 keys gives "not enough data"; the same machine at two runner sizes gets two indexes; each shard suffix form normalizes.
- *(not a test)* Run against a copy of the Mars DB (ask in the issue for one if you don't have access); quote the 7-day indexes for `io` and `Jupiter` in the PR.

**#194 (endpoint):**
- Handler tests against the fake source: the response equals the OpenAPI example (compare parsed JSON), for each window.
- A bad window returns 400; an unauthenticated request is refused the same way `/fleet/hosts` refuses it.
- Cache tests: a second call within 30 s doesn't call the source; 10 concurrent calls on an empty cache call the source once; a failing source serves the stale response, then a 503 after 10 minutes.

**#195 (page frame, Areas of concern, fleet score card):**
- Vitest: the concern list order, the 5-row limit and Show all, the empty state, rows keeping their place by `key` across a refresh, the highlight on a new concern; the score band colors; a not-enough-data component greyed out; the window remembered.
- *(not a test)* Headless Playwright on the fixture at 360 px and 1280 px, light and dark: no console errors, no horizontal scroll. Screenshots attached to the PR.

**#199 (category rules API and Settings section):**
- API tests: list, save (order kept), 422 on a bad regex, preview counts and the `other` list on seeded jobs, and non-admins refused.
- Vitest: reorder, add, delete, the 422 message shown on the right rule, and the preview refresh being debounced.
- *(not a test)* Headless Playwright screenshot of the section at 1280 px in light and dark mode, attached to the PR.

**#200 (hourly machine rollup):**
- Tests: a full hour of samples, a machine offline for half the hour (30 minutes), an hour with no samples (0, row present), percentiles on a known series, a failure that isn't the machine's fault (not counted), and re-running an hour giving the same row.
- A test with 7 days of samples for 5 machines (one every 30 s) shows no write transaction over 50 ms.

**#201 (live stats and live concerns):**
- Tests with a seeded DB fire each concern rule at its threshold and not just below it.
- The per-category running counts add up to the runners running now.
- Offline: a machine whose last sample is 6 minutes old is offline with the right `since`; a machine with budget 0 never raises `host-offline`.
- *(not a test)* Timing: on a copy of the Mars DB the function takes under 50 ms (quote the number).

**#202 (machine score table and detail):**
- Vitest: worst-first sorting, the speed-index text per runner size, expanding a row, the `#machine-<host>` deep link expanding the right row, and the per-category table matching the fixture.
- *(not a test)* Headless Playwright on the fixture at 360 px and 1280 px, light and dark, with one row expanded: no console errors, no horizontal scroll. Screenshots attached to the PR.

**#203 (live strip, category legend, Overview tile):**
- Vitest: the category totals add up to the counts shown; the oldest-wait formatting (seconds, minutes, hours); the tile's tone per band and its hint with 0, 1 and 2 critical concerns.
- The existing `metric-tiles` tests still pass unchanged (an existing test, not a new contract test).
- *(not a test)* Headless Playwright on the fixture at 360 px and 1280 px, light and dark: no console errors, no horizontal scroll. Screenshots attached to the PR.

#204 is wave 2 (Opus), not wave 1: its "Done when" is not written as contract tests here. #197 only creates its stubs (`health_source.rs`, `fleet_health_snapshot.rs`).

The rules, weights, thresholds, buckets and concern keys those tests need are in the issue bodies (#192's two score tables and concern table, #201's live-concern table, #197 §2's buckets and seed rules); copy them into `docs/design/fleet-health.md` and take the tests' expected values from there.

## Ownership, copied from the issues

The owners file starts from this table. Paths are as the issues give them; crate paths are under `crates/`.

| Issue | Owns (from its "Owns" section) | Shared files where it may add lines |
|---|---|---|
| #190 | `gridops-reconciler/src/fleet_health_backfill.rs`, `gridops-core/src/fleet_health/classifier_cache.rs` | "the `category` lines at the insert sites": `process_workflow_job` in `gridops-api/src/webhooks.rs`, the three `INSERT INTO workflow_jobs` in `gridops-core/src/autoscaling.rs`, and `gridops-api/src/webhooks/gitlab.rs` |
| #191 | `gridops-reconciler/src/fleet_health_rollup.rs` and its tests | |
| #192 | `gridops-core/src/fleet_health/score.rs`, `concerns.rs` and their tests | |
| #193 | `gridops-core/src/fleet_health/speed.rs` and its tests | |
| #194 | `gridops-api/src/fleet/health.rs` and its tests | |
| #195 | `src/features/fleet-health/page.tsx`, `areas-of-concern.tsx`, `fleet-score-card.tsx`, any `src/features/fleet-health/frame-*.tsx`, and their tests | |
| #199 | `gridops-api/src/fleet/job_categories.rs` and its tests, `src/features/settings/job-categories-section.tsx` and its test | |
| #200 | `gridops-reconciler/src/fleet_host_rollup.rs` and its tests | |
| #201 | `gridops-api/src/fleet/health_live.rs` and its tests | |
| #202 | `src/features/fleet-health/machine-score-table.tsx`, `machine-detail.tsx`, any `src/features/fleet-health/machine-*.tsx`, their tests | the one "Health" link line in `src/routes/_app/fleet.machines.$hostId.tsx` |
| #203 | `src/features/fleet-health/live-strip.tsx`, `category-legend.tsx`, any `src/features/fleet-health/live-*.tsx`, their tests | the Fleet health tile in `src/features/dashboard/metric-tiles.ts` |
| #204 (wave 2) | `gridops-api/src/fleet/health_source.rs`, `gridops-reconciler/src/fleet_health_snapshot.rs`, "the source selection in `crates/gridops-api/src/fleet/health.rs`", the end-to-end test | |
| #197 | everything else it creates, including every contract test file | |

### Overlaps and gaps found while copying (resolve each in the plan, per "Corrections to the issue text")

1. **Contract test files inside wave-1 globs.** #197 §5 puts contract tests next to the module they test and keeps them #197's, but `machine-*.tsx` (#202), `live-*.tsx` (#203) and `frame-*.tsx` (#195) match a file such as `machine-score-table.contract.test.tsx`, and "and their tests" can be read to include them. The owners file and the guard must make contract test files #197's whatever glob they match, and the guard's own tests must cover that case.
2. **`gridops-api/src/fleet/health.rs` has two owners:** #194 (wave 1) and #204 (wave 2, "the source selection"). Not a wave-1 duplicate, but the owners file must say how #204 is treated (the guard runs for wave-1 issues only, per #197 §6).
3. **#190's insert-site edits may not be add-only.** Setting `category` at an existing `INSERT INTO workflow_jobs` changes the column list and values of that statement; the guard's shared-file rule only allows added lines. The plan decides how the owners file expresses #190's entries for `webhooks.rs`, `autoscaling.rs` and `webhooks/gitlab.rs`, and the guard's tests cover it.
4. **`gitlab_jobs` has no `started_at` or `completed_at` column** (`migrations/0047_gitlab.sql`: `status_changed_at`, `created_at`, `updated_at` only), so "the `gitlab_jobs` equivalent" of the `workflow_jobs(completed_at)` index, and #191's GitLab queue wait and duration, have no column to use. Also, `gitlab_jobs` in 0047 is not `STRICT` (only `fleet_ci_targets_new` is). The plan states what the migration adds for GitLab jobs and why, and the design doc records it.
5. **Job-category routes:** #197's table says the three routes go in `gridops-api/src/fleet/mod.rs`, but their paths are `/api/v1/settings/job-categories…` and every other `/api/v1/settings…` route is registered in `gridops-api/src/main.rs`. Either file works (`fleet::router()` is merged into the app in `main.rs`); the plan picks one and no wave-1 issue edits it.

## Decisions after the first architect review (2026-11-22, orchestrating session)

The Phase 3 review of the first plan (`docs/handoffs/197/handoff-A.md`) returned four BLOCK findings. Each is decided here; these decisions override the ownership table and the overlap list above wherever they disagree.

### D-1. Job classification is wired at the three real writers, through one seam #190 owns

- The production writers are `process_workflow_job` in `crates/gridops-api/src/webhooks.rs` (~`:645`), `upsert_workflow_job` in `crates/gridops-reconciler/src/main.rs` (~`:2133`) and the `UPSERT` / `record_gitlab_job` in `crates/gridops-core/src/gitlab/jobs.rs` (~`:155`, `:202`). The `INSERT INTO workflow_jobs` statements in `crates/gridops-core/src/autoscaling.rs` are all in `#[cfg(test)]` and are not insert sites. `crates/gridops-api/src/webhooks/gitlab.rs` has no INSERT.
- #197 adds the `category` column to each of the three writers' statements and binds a value from one function in `crates/gridops-core/src/fleet_health/classifier_cache.rs` (for example `category_for_insert(db, job_name, workflow_name, repository, labels) -> Option<JobCategory>`, one per source if the shapes differ). The stub returns `None`. #190 fills in the body. **#190 owns no shared-file lines**: its owners entry is `classifier_cache.rs`, `fleet_health_backfill.rs` and its contract test files only (this settles overlap 3).
- **GitLab names and timing.** Today `gridops-core::gitlab::Job` and `JobObservation` carry no job name or project path, and `gitlab_jobs` has no start or finish time. Migration 0050 adds nullable `gitlab_jobs.job_name`, `gitlab_jobs.project_path`, `gitlab_jobs.started_at` and `gitlab_jobs.completed_at` (epoch ms), plus the `completed_at` index. #197 extends `gitlab::Job` (`name`, serde default), `JobObservation` (`job_name`, `project_path`, both `Option`, `None` keeps the stored value), and the writer. The webhook passes `build_name` and `project.path_with_namespace`, which `webhooks/gitlab.rs` already reads. The poll passes the job API's `name` and the project path where it has them. The writer sets `started_at` the first time the status becomes `running`, and `completed_at` the first time it becomes terminal, both from `status_changed_at`. It never overwrites a set value. Existing rows stay NULL; #190's backfill classifies them from whatever is there (tags at least). Test all of this here: it is #197's code, not a stub. Record it in the design doc.
- After this story's plan passes, the orchestrating session posts a comment on #190 that corrects its insert-site list. That is not this pipeline's job.

### D-2. Frontend contract tests live in `tests/`, where Vitest collects them

`vitest.config.ts` only includes `tests/**/*.test.ts` (verbatim, its `test` block: `environment: "node",` / `include: ["tests/**/*.test.ts"],`), and the existing tests use `createElement`, `createRoot` and `act` in jsdom with no testing-library. Put frontend contract tests at `tests/fleet-health-<issue>-*.contract.test.ts` (one or more per frontend wave-1 issue), in the existing style, and give each frontend wave-1 issue its own `tests/fleet-health-<issue>-*.test.ts` glob in the owners file for the tests it adds. Contract test files stay #197's whatever glob they match, and the guard checks that. Don't change the Vitest config.

**D-2 supersedes the copied acceptance text for frontend contract tests.** The sentence copied from #197's body under "Acceptance criteria" ("live in `*_contract_tests.rs` / `*.contract.test.ts(x)` files next to the module they test, one per wave-1 issue") is overridden by D-2 for every frontend (Vitest) contract test: they live in `tests/` as `tests/fleet-health-<issue>-*.contract.test.ts` (`.ts`, not `.tsx`, because the include glob only collects `*.test.ts`), not next to the module, and there may be one or more files per frontend wave-1 issue. The marker form is unchanged: `it.skip("[contract: #<issue>] …")`. A `src/features/fleet-health/machine-*.contract.test.tsx`-style file (overlap 1) therefore isn't created; the guard's "contract test files are #197's whatever glob they match" rule and its tests still cover such a path in case one appears. The Rust placement rule is unchanged: Rust contract tests are `#[ignore = "contract: #<issue>"]`, live in `*_contract_tests.rs` files next to the module they test, declared from a #197-owned parent (D-3(e)), one file per Rust wave-1 issue, except #190, which gets one contract-test file per crate it touches (W guidance below). The "fails once its marker is removed" check applies to both sets as placed here.

### D-3. #197 wires every hook a wave-1 issue would otherwise need in a shared file

- (a) An `AppState` field in `crates/gridops-api/src/state.rs` for the health source and #194's cache, following the `gitlab` / `with_gitlab` precedent. `FakeFleetHealthSource` gets the seams #194's tests need (a call count, fail-next, held calls, a settable clock), like `FakeGitLab`.
- (b) A fleet-wide queue breakdown #201 can call from `collect_live`: see "(b), closed" below for the one shape to implement.
- (c) The Overview tile. #197 calls `useFleetHealth("24h")` in `src/routes/_app/index.tsx`, adds the optional second parameter to `metricTiles`, and widens the `MetricTile.to` union. #203's change to `metric-tiles.ts` is then add-only, and the guard enforces it. Until #203 lands, the tile renders nothing.
- (d) A 422 error variant in `ApiError` (and the fleet `ErrorCode` if the category routes use the fleet envelope; see W-5 below), for #199's invalid-regex response.
- (e) Contract-test `mod` declarations sit in #197-owned parent files, as `gitlab/mod.rs:22-26` does, so a wave-1 agent can't unhook its own tests.

- (f) **A principal-free, fleet-wide host read for #201** (decided after the second architect review). #201's issue says to reuse the functions `/fleet/hosts` uses (counts, `physicalBudget`, `physicalAllocation`, `latestRootMeasurement`, placement failures). Those are private helpers in `crates/gridops-core/src/fleet/registry/queries.rs`, reachable only through `RegistryService::list_hosts(principal, …)`, whose `open_snapshot` refuses a principal with no user id. #197 adds a public, principal-free, fleet-wide read in `gridops_core::fleet::registry`, for example `RegistryService::host_live_reads(now)` or a free function, built from the **same** helpers `host_projection` uses (no parallel queries), bounded in the #179 style, and tested here against a seeded fleet: it must give the same per-host counts, budget, allocation and latest measurement that `list_hosts` gives an admin. `collect_live` takes it (or the pool it needs) without a principal. #201 then calls it and edits nothing in `registry/`.
- (b), closed. Per W-17: add a fleet-wide `pub(crate)` variant in `resources/overview_queue.rs` (`split_queue` + `waiting_job_pools`, with the job query not filtered by user), re-exported from `resources.rs`, and tested here. Don't add a stub signature for #201 to fill in.
- (a), refined. Per W-18: the cache's clock lives on the health state (the `RegistryClock` / `with_clock` shape), not on the fake. The state's type and its default source are built in `fleet/health.rs`, so #194 and #204 never edit `state.rs` or `main.rs`.
- (d), refined. Per W-20: the 422 body carries per-rule errors, for example `{error, code, details: [{position, message}]}`. Extend `ApiError` to render an optional `details` field without changing any existing error's body, and test both.

### D-4. The health endpoint is admin-only

`GET /api/v1/fleet/health` returns fleet-wide data cached once per window, so it uses the `GET /api/v1/fleet/workload` pattern: `FleetUser<Read>` plus `require_admin` (`fleet/profiles.rs:330-349`). Make `require_admin` `pub(super)` (or move it into a shared fleet module). A non-admin gets 403; add that case to #194's contract tests. The page and the Overview tile hide on 403, as the workload UI does. **Tokens (decided, per W-19):** the health route accepts `fleet:read` tokens of an admin, as workload does. Add the `TOKEN_SCOPED_ROUTES` row, both `docs/fleet-api.md` tables and the OpenAPI security entry. The job-category routes are session-only and admin-only (they are under `refuse_bearer`, per W-5). Record D-4 in `docs/design/fleet-health.md`.

**Issue corrections:** once the plan passes, the orchestrating session comments on #190 (insert sites, D-1), #194 (admin-only rather than "copy `query::hosts`", D-4) and #201 (the host read of D-3(f) and the queue variant of D-3(b)). This pipeline doesn't post them.

**Writer transactions (W-15, binding):** the classifier seam takes the caller's `&mut SqliteConnection` (or is pure over a cached `Arc<Classifier>` obtained before the transaction opens), never a `&SqlitePool` inside a write transaction. Bind `category` in `VALUES` only; `ON CONFLICT DO UPDATE` keeps the stored value. **W-16** is binding as well: extend the `jobs.rs` module-doc rules and `gitlab/jobs_tests.rs` in place, put the serde default on `http.rs`'s `JobResponse`, let the `UPSERT`'s `WHERE` guard admit name and path fills, and update every `Job` literal.

### The review's WARN findings are adopted as guidance

W-5: register the category routes in `session_routes()` in `crates/gridops-api/src/main.rs` (under `refuse_bearer`), not in `fleet::router()`. This replaces overlap 5. W-6 to W-13 are adopted as written in `handoff-A.md`: an `async_trait` source behind `Arc<dyn …>`; `testdata.rs` and the speed-check helper behind `cfg(any(test, feature = "test-support"))`; `run_pass(&SqlitePool, now: i64)` called log-and-continue next to `fleet::tick` and before the manager-unavailable early return; the speed-check helper doesn't rely on a thread-local subscriber and its slow self-test runs in its own test binary; add `regex`, and expose per-rule compile errors from `category.rs`; no `proptest`, so #192 uses 1,000 seeded inputs; STRICT primary keys get non-NULL sentinels for an unmanaged runner's host and pool, with no FK on `host_id`; zod schemas and the fleet `request` transport in `types.ts`; chart theme on the existing `var(--…)` tokens; a unique nav shortcut and correct active state for Machines vs Health; the guard's tests run with `node --test`; one contract-test file per crate for #190.

## Registration points that exist today (verified at `83149f5`)

So that every hook can be wired here, these are where they go:

- `crates/gridops-core/src/lib.rs`: a flat `pub mod` list; `pub mod fleet_health;` goes there.
- `crates/gridops-api/src/fleet/mod.rs`: private `mod` lines at the top and `pub(crate) fn router()` (a chain of `.route(...)` ending in the `/api/v1/fleet/{*path}` not-found catch-all; axum matches the specific route first regardless of order). `crates/gridops-api/src/main.rs` merges `fleet::router()` and registers the `/api/v1/settings…` routes. `crates/gridops-api/src/lib.rs` does not exist (the api crate is a binary).
- `crates/gridops-reconciler/src/main.rs`: `reconcile()` runs the per-pass steps one after another (`fleet::tick`, `cleanup_retention`, …); each new `run_pass` is one more call there. `cleanup_retention` is a sequence of `DELETE … WHERE <time> < ?` statements, where the three new 90-day deletes go.
- `src/lib/navigation.ts`: a `navigation: NavGroup[]` data array; "Machines" (`/fleet`) is an item in the "Operate" group. The "Health" entry is one more item there.
- `src/routes/`: TanStack file routes (`createFileRoute`); no generated route tree is committed. `src/routes/settings.general.tsx` composes `SettingsSection`s, where `job-categories-section.tsx` is rendered.
- `crates/gridops-core/src/db.rs`: `SLOW_STATEMENT = 100 ms` and `.log_slow_statements(log::LevelFilter::Warn, SLOW_STATEMENT)`, the #179 hook the speed-check helper builds on.
- Build constraints above: `.claude/settings.local.json` in this worktree sets `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`; `~/Projects/gridops/.cargo/config.toml` sets `target-dir = "/Users/andreangelantoni/.cache/cargo-target/gridops"`.

## Out of scope

Filling in any stub beyond what #197 says is implemented here (`category.rs`, `merge_concerns`, `api.ts`, `chart-theme.ts`, `score-gauge.tsx`, the fake source, the guard, the speed-check helper and the synthetic dataset). Deploying. Changing prod data. Adding the required status check (done by the orchestrating session after merge).

## Docs

CHANGELOG entry. The design doc `docs/design/fleet-health.md` is the main doc deliverable.

## Source material read

Read in full: GitHub issues #197, #198, #190, #191, #192, #193, #194, #195, #199, #200, #201, #202, #203, #204 (`gh issue view`, 2026-11-22); `src/lib/navigation.ts`; `crates/gridops-core/src/lib.rs`; `.claude/settings.local.json`; `~/Projects/gridops/.cargo/config.toml`; the project memory entries on Mars DB access and deploys; `vitest.config.ts` (brief amendment, round 1); the `tests/` directory listing (brief amendment, round 1: existing frontend tests are flat `tests/*.test.ts` files).

Read in part (the sections relevant to the findings): `migrations/0047_gitlab.sql` (the `gitlab_jobs` table and indexes), `migrations/0001_initial.sql` (`workflow_jobs` columns and indexes), `crates/gridops-reconciler/src/main.rs` (`run_pass`, `reconcile`, `cleanup_retention`), `crates/gridops-api/src/fleet/mod.rs` (module lines and `router()`), `crates/gridops-api/src/main.rs` (router merge and settings routes), `crates/gridops-core/src/db.rs` (slow-statement logging), `src/routes/settings.general.tsx` (imports and sections), `.github/workflows/ci.yml` (job names), the `migrations/` listing.

Checked to exist, not read: `migrations/0022_fleet_foundation.sql`, `crates/gridops-core/src/disk.rs`, `crates/gridops-core/src/fleet/start_health.rs`, `crates/gridops-core/src/fleet/failure.rs`, `crates/gridops-core/src/autoscaling.rs`, `crates/gridops-api/src/fleet/query.rs`, `crates/gridops-api/src/webhooks.rs`, `crates/gridops-api/src/webhooks/gitlab.rs`, `crates/gridops-api/src/resources.rs`, `src/routes/_app/fleet.index.tsx`, `src/routes/_app/fleet.machines.$hostId.tsx`, `src/features/dashboard/metric-tiles.ts`, `src/features/fleet/fleet-workload-section.tsx`, `src/lib/refresh-interval.ts`, `src/lib/api.ts`, `docs/openapi/gridops-v1.json`, `CHANGELOG.md`, `docs/design/`.

Not found: `crates/gridops-api/src/lib.rs`, `docs/design/fleet-health.md` and `docs/design/fleet-health-owners.json` (both created by this story).
