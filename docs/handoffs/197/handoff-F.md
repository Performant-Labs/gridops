# Handoff-F: Phase 6 - Fleet health shared contract (#197), round 2

**Date:** 2026-11-22
**Branch:** claude/fleet-197-contract (head c3f2ebc; production edits uncommitted, for the driver to commit)
**Issue:** #197

Round 1 is kept unchanged below, under "Round 1". This round implements what T's round-2 RED (`handoff-T-red.md`, "The contract F implements against (this round)") asks for, in answer to diff gate round 1 (`docs/handoffs/197-diff-result-r1.md`).

## What was done
- `src/features/fleet-health/page.tsx`: `HealthSections` puts each section in its own `Slot`, a `<div data-health-section="<slot>">`. There are six slots: `live-strip`, `areas-of-concern`, `fleet-score`, `category-legend`, `machine-table` and `machine-detail`. The stale warning stays outside the slots. `HealthSlot` is a union of the six names, so a typo fails `tsc`.
- `docs/design/fleet-health.md`, "The page": lists the six slots as part of the composition. Wave 1 fills a slot's content and may move the slot, but keeps the attribute. No slot renders while loading, on a 403 or on a failure. Adds warn 28's sentence: #197's page test asserts the composition slots, not the placeholder text.
- `docs/design/fleet-health.md`, "Testing" (diff gate W-3): the existing "Limits:" sentence now says that catching these cases is a non-goal. A wave-1 rollup can't rely on the check to flag a lock that `BEGIN IMMEDIATE` holds before its first write, or a slow write to a `WITHOUT ROWID` table.

## Design decisions
- **A wrapper element, not an attribute on each section's root.** The sections are wave-1 files (#195, #202, #203). If the attribute sat on their roots, every wave-1 rewrite would have to remember to keep it. In `page.tsx` the wrapper survives a section rewrite, and moving a slot means moving one `<Slot>` line. This is the shape T's contract names ("wraps each section in its own element").
- **`min-w-0` on the wrapper.** Two slots are now grid items (in the `lg:grid-cols-2` row). Without `min-w-0`, a wide table or chart that wave 1 adds would widen the grid track instead of shrinking or scrolling. Today the wrapper is otherwise layout-neutral: it is a block `div`, and the section inside fills it as before.
- **No exported slot constant.** The test pins the names on purpose, and the design doc lists them. A shared export would let a rename pass both sides silently.

## Reuse / extend-vs-new
Extends the existing `HealthSections` in `page.tsx`. Nothing new outside it, and `Slot` is private to the file.

## Architecture notes for A
None. No layer, dependency, schema or public-interface change. A DOM attribute is added for tests, and the design doc gains two paragraphs.

## Deviations from spec / wireframe
None. No wireframe.

## Tier 1 self-check (incl. tests now GREEN)
- `npx vitest run tests/fleet-health-page.test.ts`: **4 passed (4)**. Before this change it was 3 failed and 1 passed (T's RED).
- `npx vitest run`: Test Files 56 passed, 4 skipped (60). **Tests 642 passed, 18 skipped (660).** The 18 skipped are the marked contract tests.
- `npm run lint` (eslint, `--max-warnings=0`): clean.
- `npx tsc --noEmit`: clean.
- `npm run build`: `✓ built in 939ms`, and the typecheck is clean.
- `node --test scripts/fleet-health-guard.test.mjs`: 13 of 13 pass.
- No Rust file changed this round, so cargo was not re-run. T-red round 2 ran fmt and CI's clippy on this head, and both were clean.

## Evidence appendix
None: this round relies on no behaviour outside the diff. The 403 and failure states that the page test checks are in `page.tsx`, the file shown in the diff.

## Tests that look wrong (for T)
None.

## Known issues
- Diff gate **B-1** is met by T's marker-removal run (`handoff-T-red.md`, round 2, item 3). There is no production change for it.
- The following are not F's and are carried for O. None of them blocks this round's tests.
  - **W-1:** the guard's two sample PRs. They must be cut from a head after T's round-2 contract-test edit.
  - **W-2:** the Mars-copy timing.
  - **W-5:** the #204 reminder to move `FakeFleetHealthSource` behind `test-support`.
- **W-4** is T's doc comment in `health_wiring_tests.rs`.
- The NITs are left as they are:
  - NIT-1: the escaped pipes are correct Markdown;
  - NIT-2 and NIT-3 belong to #201 and #195/#203, which are the issues that touch those lines.

## Files changed
- `src/features/fleet-health/page.tsx`
- `docs/design/fleet-health.md`

---

# Round 1

# Handoff-F: Phase 5 - Fleet health, the groundwork (#197)

**Date:** 2026-11-22
**Branch:** the #197 story worktree (`.claude/worktrees/gridops-197`)
**Issue:** #197

## What was done

Core (`gridops-core`):
- `migrations/0050_fleet_health.sql`: the fleet health tables (`job_category_rules`, `fleet_health_hourly`, the host rollup, backfill and snapshot tables), the `category` column on `workflow_jobs`, the GitLab job name/path/started/completed columns, and the `gitlab_jobs_completed_idx` and `gitlab_jobs_unclassified_idx` indexes.
- `src/fleet_health/mod.rs`: the shared types (`FleetHealth`, `HealthWindow`, `HostHealth`, `FleetLiveStats` (now `Default`), `HostLiveStats`, `Concern` and the rest), the `FleetHealthSource` trait, `assemble`, and the module mounts.
- `src/fleet_health/category.rs`: `JobCategory`, `NewJob`, `JobOrigin` and `category_for_insert`, the classifier used at insert time.
- `src/fleet_health/classifier_cache.rs`: a stub for #199's rules cache.
- `src/fleet_health/score.rs`, `speed.rs`, `concerns.rs` (with `merge_concerns`): the scoring, speed and concern building blocks the waves fill.
- `src/fleet_health/fake.rs`: the fixture-backed `FleetHealthSource` the API serves until #204.
- `src/fleet_health/speed_check.rs`: the database speed check (slow statements over 100 ms, write transactions over 50 ms), for test builds only.
- `src/fleet_health/testdata.rs`: `seed_fleet` and `build_week`, the synthetic week of jobs and samples.
- `src/db.rs`: instruments test-build pools for the speed check. `src/lib.rs`: the module mount. `Cargo.toml`: the `test-support` feature and its dependencies.
- `src/fleet/registry/{mod.rs,queries.rs}`: `host_live_reads`, the fleet-wide host read (D-3f).
- `src/gitlab/{mod.rs,jobs.rs,http.rs}`: the job's `name`, path, `started_at` and `completed_at`, read and stored.

API (`gridops-api`):
- `src/fleet/health.rs` (new): `FleetHealthState` (source plus clock) and the `GET /api/v1/fleet/health` handler (fleet admin, `fleet:read` token accepted).
- `src/fleet/health_live.rs` (new): the `collect_live` stub (#195/#202/#203 fill it).
- `src/fleet/health_source.rs` (new): the `DbFleetHealthSource` stub (#204).
- `src/fleet/job_categories.rs` (new): the three settings handlers. They check for a system admin, then return 501 "arrive with #199".
- `src/fleet/mod.rs`: mounts the new modules, adds the health route and its `TOKEN_SCOPED_ROUTES` row. `src/fleet/profiles.rs`: `require_admin` is now `pub(super)`.
- `src/state.rs`: the `fleet_health` field, plus `with_fleet_health_source` and `with_fleet_health_clock`.
- `src/main.rs`: the `/api/v1/settings/job-categories` routes (GET, PUT) and the `/preview` route (POST), in `session_routes()`.
- `src/error.rs`: `ApiError::Invalid { code, message, details }`, a 422 with the body `{error, code, details}`.
- `src/resources.rs` and `src/resources/overview_queue.rs`: the `fleet_queued_jobs!` clause, shared helpers taken out of `queue_breakdown`, and `fleet_queue_breakdown` (all mapped pools, counted by category).
- `src/webhooks.rs`: the GitHub `workflow_job` insert binds `category`. `src/webhooks/gitlab.rs`: the observation carries the job name (`build_name`) and the project path.

Reconciler:
- `src/fleet_health_{rollup,backfill,snapshot}.rs` and `src/fleet_host_rollup.rs` (new): stubs with `run_pass`, and `rollup_hour` for the two rollups.
- `src/main.rs`: `fleet_health_passes` runs the four passes after the fleet tick and logs a failure without stopping. `cleanup_retention` keeps 90 days. `upsert_workflow_job` binds `category`.
- `src/gitlab_poll.rs`: the polled observation carries the job name (no project path: the webhook fills it).

Frontend:
- `src/lib/api.ts`: `ApiError` carries `code` and `details`.
- `src/features/fleet-health/`:
  - `types.ts`: the zod schemas.
  - `api.ts`: `fetchFleetHealth`, `useFleetHealth`, `isHealthForbidden`.
  - `fixture.json`: the shared fixture.
  - `chart-theme.ts`: the category colours and labels.
  - `page.tsx`: `FleetHealthPage`.
  - `placeholder-card.tsx`, and the wave stubs `areas-of-concern`, `fleet-score-card`, `machine-score-table`, `machine-detail`, `category-legend` and `live-strip`.
- `src/components/charts/{chart-theme.ts,score-gauge.tsx}`: the shared score bands and the gauge.
- `src/routes/_app/fleet.health.tsx`: the route. `src/lib/navigation.ts`: the Health item, plus the fix for which item is active. `src/features/dashboard/metric-tiles.ts` and `src/routes/_app/index.tsx`: the health hook for the Overview tile.
- `src/features/settings/job-categories-section.tsx` and `src/routes/settings.general.tsx`: the placeholder section. `src/styles/app.css`: the `--category-*` tokens.

Guard, CI and docs:
- `scripts/fleet-health-guard.mjs`: the PR file-ownership guard. `docs/design/fleet-health-owners.json`: the owners map.
- `.github/workflows/ci.yml`: the guard unit tests in the web job, and a new `fleet-health-guard` job (PRs only).
- `docs/design/fleet-health.md` (the design doc), `docs/fleet-api.md`, `docs/openapi/gridops-v1.json` (the health path and the job-category paths and schemas), and `CHANGELOG.md`.

## Design decisions

- **`useFleetHealth` uses component state, not react-query.** `tests/dashboard-pool-scope.test.ts` renders the Overview without a `QueryClientProvider`, so a react-query hook broke it. I followed `FleetWorkloadSection`: `useState` and `useEffect`, a 30 s `setTimeout`, and `refreshSignal`. It stops polling on 401 or 403.
- **The 422 body has its own schema, `ApiInvalidBody`.** `ApiErrorBody`'s fields are pinned by the openapi test, so I didn't add `details` to it.
- **The health handler always reads `HealthWindow::Day`.** Window selection is a wave feature. The OpenAPI document gives the `window` query enum (1h, 24h, 7d), defaulting to 24h.
- **The job-category handlers are stubs that check for a system admin first.** A non-admin gets 403 today, as the contract requires. An admin gets 501 until #199.
- **`fleet_queue_breakdown` reuses `queue_breakdown`'s helpers.** I moved `pools_with_platforms`, `waiting_job` and `split_waiting_jobs` out of `queue_breakdown` instead of copying them; the per-user Overview breakdown behaves as before. `fleet_queued_jobs!` is `visible_queued_jobs!` without the user join (evidence: `resources.rs:80-88`).
- **The category is computed before the INSERT, on a connection taken from the pool, in both GitHub writers (webhook and reconciler).** `ON CONFLICT` doesn't touch `category`, so a job is classified once, when it is first seen.
- **Synthetic week batches: 75 jobs and 500 samples per transaction (were 150 and 1,000).** In the full parallel workspace run (load about 10), four batches held the write lock for 53 to 84 ms, over the 50 ms limit. With the halved sizes, two back-to-back runs passed in 8.7 s and 8.6 s while the core lib tests ran alongside. Per-transaction times measured in isolation are mostly 3 to 13 ms, with a peak of 25 ms.
- **The navigation fix in `isActivePath`.** With `/fleet` and `/fleet/health` both in the nav, both items were active at once. A parent item is now inactive when a more specific item matches.

## Reuse / extend-vs-new

Each item below extends the existing object, as the brief's Reuse map calls for:
- `FleetUser<Read>` and `require_admin`, the fleet auth.
- `require_system_admin` and `session_routes()`, the settings auth (evidence: `auth.rs:185-191`, `main.rs:416`).
- `queue_breakdown` (refactored to share its helpers) and `visible_queued_jobs!`.
- The `AppState` builder methods.
- `system_clock()` (`registry/mod.rs:78-80`).
- The #179 slow-statement log (`db.rs:20/40`).
- The `FleetWorkloadSection` fetch pattern.
- `PageHeader`, `EmptyState`, `Callout` and `SettingsSection`.
- The existing nav list.

The new modules are the ones the brief creates: `fleet_health`, the API health and job-category modules, the reconciler passes, the charts, and the guard.

## Architecture notes for A

- **New core module:** `fleet_health`. `testdata` and `speed_check` are built for test builds and the `test-support` feature only.
- **New public interfaces:**
  - `GET /api/v1/fleet/health`
  - `GET` and `PUT /api/v1/settings/job-categories`, `POST /api/v1/settings/job-categories/preview`
  - `ApiError::Invalid`, rendered as a 422
- **Schema:** migration 0050.
- **Shared code changed:**
  - `ApiError` (TS) gains `code` and `details`.
  - `isActivePath`.
  - `require_admin` visibility.
  - `queue_breakdown`'s helpers, a refactor that keeps behaviour.
  - Both GitHub job INSERTs gain a 16th column.
- **Dependency direction:** api → core and reconciler → core. No new edge.
- **CI:** a new job, `fleet-health-guard`, with read permission on pull requests.

## Deviations from spec / wireframe

None in behaviour. The brief's #197 surface is placeholders only, so there is no wireframe to match. All waves land as stubs with "Coming in #N" cards.

## Tier 1 self-check (incl. tests now GREEN)

Every run used `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`. The runs were made with temporary local copies of the test-side fixes listed under "Tests that look wrong". Those copies are reverted now and none were staged.

- `cargo fmt --all --check`: clean.
- `cargo +1.98.1 clippy --workspace --locked -- -D warnings -A clippy::unused_async_trait_impl` (production targets): `Finished`, no warnings.
  - The CI command also uses `--all-targets`, which adds lint errors in T's test files (listed below).
- `cargo test --workspace --locked --no-fail-fast` (one full run).
  - Every #197 test passes except the failures below:
    - `the_synthetic_week_has_the_fixture_machines_at_realistic_volume`: write lock held 84 ms under load. Fixed since by halving the batches.
    - `webhooks::gitlab_tests::a_first_seen_running_event_marks_the_runner_busy_and_a_later_pending_is_ignored`: T's.
    - `db::tests::fresh_database_applies_every_migration`: T's.
    - The `gridops-manager` tests `tart_outage_tests::*` and `route_timeout_tests::*` fail with "path must be shorter than SUN_LEN". That is the macOS temp-dir socket path length, in a crate this change doesn't touch.
  - Per target:
    - api bin: 135 passed, 1 failed (the GitLab test above).
    - api `control_plane_contracts`: 39 passed.
    - api `fleet_http`: 151 passed.
    - reconciler: 133 passed, 11 ignored (the contract markers).
    - core lib: 320 passed, 1 failed (the migration count, T's).
- `cargo test -p gridops-core --test fleet_health_speed_check`, after halving the batches, run twice under load:
  - `test result: ok. 4 passed; 0 failed; ... finished in 8.72s`
  - `test result: ok. 4 passed; 0 failed; ... finished in 8.55s`
- `cargo test -p gridops-core --lib`: `320 passed; 1 failed (db::tests::fresh_database_applies_every_migration); 17 ignored`.
- `npx vitest run`: 638 passed, 18 skipped (the contract markers). `eslint`: clean. `npx vite build`: ok. `tsc --noEmit`: clean.
- `node --test scripts/fleet-health-guard.test.mjs`: 13 of 13 pass.
- `npm run validate:openapi`: valid, with 71 warnings against 67 at HEAD. The 4 new warnings are missing `operationId`s, the same as the existing operations.

## Evidence appendix

`docs/handoffs/197/evidence.md` has 9 entries:
- `auth.rs:185-191`
- `main.rs:416`
- `registry/mod.rs:78-80`
- `resources.rs:80-88`
- `fleet/error.rs:29-35`
- `db.rs:20/40`
- `migrations/0014:19-24`
- `tests/dashboard-pool-scope.test.ts:89`
- `webhooks/gitlab_tests.rs:163-170`

## Tests that look wrong (for T)

I edited none of these. Each is test-only.

1. **Missing `name` field on GitLab `Job` literals.** `gitlab::Job` gained `name` (#190 W-16), and these literals don't set it, so the test targets don't compile:
   - `crates/gridops-core/src/gitlab/http_tests.rs` (about line 332): `name: Some("build".to_owned())`, to match the HTTP payload.
   - `crates/gridops-core/src/gitlab/fake_tests.rs` (about line 48): `name: None`.
   - `crates/gridops-api/src/webhooks/gitlab_tests.rs` (about line 356): `name: None`.
   - `crates/gridops-reconciler/src/gitlab_poll_tests.rs` (about line 177): `name: None`.
2. **`crates/gridops-core/tests/fleet_health_contract.rs:36`**: `Concern.link` is `Option<String>`, so the literal should be `link: Some("/fleet/health".to_owned())`.
3. **`crates/gridops-core/src/fleet/registry/query_tests.rs` (about line 2779)**: `format!("{}-root", &id[..30])` gives an id that isn't a UUID and repeats an existing one. A distinct UUID such as `id.replacen("10000000", "11000000", 1)` fixes it.
4. **`crates/gridops-core/tests/fleet_schema.rs`**, `gitlab_tables_enforce_the_pinned_constraints` (about line 1116): the pinned index list must add `gitlab_jobs_completed_idx` and `gitlab_jobs_unclassified_idx`, which come from 0050 (brief D-3c).
5. **`crates/gridops-core/src/db.rs:347`**, `fresh_database_applies_every_migration`: the expected table count must be 79 (74 plus the 5 tables 0050 adds). Migration PRs bump it, as #150 did.
6. **`webhooks/gitlab_tests.rs`**, `a_first_seen_running_event_marks_the_runner_busy_and_a_later_pending_is_ignored` (about line 1314):
   - The later `pending` event carries `build_name: "rspec"` (evidence: `gitlab_tests.rs:163-170`).
   - Per W-16, that fills `job_name` on a row first written without one, which bumps `updated_at`, so the whole-row equality fails.
   - The assertion should exclude `job_name` and `updated_at`, or the test should expect the name fill.
7. **CI clippy (`--all-targets`, 1.98.1) errors in T's test files:**
   - `health_test_support.rs:44`: `TestClock::set` is never used.
   - `fleet_health_speed_check.rs:124`: an operation with no effect.
   - `fleet_health_rollup_contract_tests.rs:126`: `doc_markdown` (backticks).
   - `fleet_health_schema.rs:456`: `type_complexity`.
   - `score_contract_tests.rs:339`: `float_cmp`.
   - `score_contract_tests.rs:591`: `manual_is_multiple_of`.
   - `speed_contract_tests.rs:81`: `needless_question_mark`.
   - `speed_contract_tests.rs:302` and `:304`: `cast_possible_wrap` (`usize` to `i64`).

Items 1 and 2 stop the core, api and reconciler test targets from compiling until T applies them. My GREEN runs above used local copies of those fixes, which are now reverted.

## Known issues

- **Scope cap: no split.** The scope is well over 6 files and several surfaces, but the brief requires #197 to land as one PR containing every shared hook, so that wave 1 can work in parallel. A passed that plan at Phase 3, and T wrote the suite for the whole scope.
- **The guard's live test PRs are O's.** The brief's sample PRs against `Performant-Labs/gridops`, one passing and one that the guard should refuse, need a push and a PR, which F doesn't do. The guard's logic is covered by its 13 unit tests.
- **The Mars database speed measurement is O's.** The brief's timing on a copy of the production database needs an ssh session to Mars. I didn't run it. Locally, the synthetic week passes the speed check.
- **`gridops-manager` tests fail locally with "SUN_LEN".** This is environmental (macOS socket path length), and the crate is untouched.
- **Local stable clippy (1.99.1) fires new lints in code that was already there.** CI runs 1.98.1, which is clean on the production targets.

## Files changed

.github/workflows/ci.yml
CHANGELOG.md
Cargo.lock
Cargo.toml
migrations/0050_fleet_health.sql
crates/gridops-core/Cargo.toml
crates/gridops-core/src/lib.rs
crates/gridops-core/src/db.rs
crates/gridops-core/src/fleet/registry/mod.rs
crates/gridops-core/src/fleet/registry/queries.rs
crates/gridops-core/src/gitlab/mod.rs
crates/gridops-core/src/gitlab/jobs.rs
crates/gridops-core/src/gitlab/http.rs
crates/gridops-core/src/fleet_health/mod.rs
crates/gridops-core/src/fleet_health/category.rs
crates/gridops-core/src/fleet_health/classifier_cache.rs
crates/gridops-core/src/fleet_health/concerns.rs
crates/gridops-core/src/fleet_health/fake.rs
crates/gridops-core/src/fleet_health/score.rs
crates/gridops-core/src/fleet_health/speed.rs
crates/gridops-core/src/fleet_health/speed_check.rs
crates/gridops-core/src/fleet_health/testdata.rs
crates/gridops-api/src/error.rs
crates/gridops-api/src/main.rs
crates/gridops-api/src/state.rs
crates/gridops-api/src/resources.rs
crates/gridops-api/src/resources/overview_queue.rs
crates/gridops-api/src/webhooks.rs
crates/gridops-api/src/webhooks/gitlab.rs
crates/gridops-api/src/fleet/mod.rs
crates/gridops-api/src/fleet/profiles.rs
crates/gridops-api/src/fleet/health.rs
crates/gridops-api/src/fleet/health_live.rs
crates/gridops-api/src/fleet/health_source.rs
crates/gridops-api/src/fleet/job_categories.rs
crates/gridops-reconciler/src/main.rs
crates/gridops-reconciler/src/gitlab_poll.rs
crates/gridops-reconciler/src/fleet_health_rollup.rs
crates/gridops-reconciler/src/fleet_host_rollup.rs
crates/gridops-reconciler/src/fleet_health_backfill.rs
crates/gridops-reconciler/src/fleet_health_snapshot.rs
docs/fleet-api.md
docs/openapi/gridops-v1.json
docs/design/fleet-health.md
docs/design/fleet-health-owners.json
docs/handoffs/197/evidence.md
docs/handoffs/197/handoff-F.md
docs/handoffs/197/decisions.md
scripts/fleet-health-guard.mjs
src/lib/api.ts
src/lib/navigation.ts
src/styles/app.css
src/components/charts/chart-theme.ts
src/components/charts/score-gauge.tsx
src/features/fleet-health/api.ts
src/features/fleet-health/types.ts
src/features/fleet-health/fixture.json
src/features/fleet-health/chart-theme.ts
src/features/fleet-health/page.tsx
src/features/fleet-health/placeholder-card.tsx
src/features/fleet-health/areas-of-concern.tsx
src/features/fleet-health/fleet-score-card.tsx
src/features/fleet-health/machine-score-table.tsx
src/features/fleet-health/machine-detail.tsx
src/features/fleet-health/category-legend.tsx
src/features/fleet-health/live-strip.tsx
src/features/dashboard/metric-tiles.ts
src/features/settings/job-categories-section.tsx
src/routes/_app/index.tsx
src/routes/_app/fleet.health.tsx
src/routes/settings.general.tsx
