# Handoff-T-red: Phase 4 - Fleet health shared contract (#197)

**Date:** 2026-11-22
**Branch:** claude/fleet-197-contract (head 829367f, test files uncommitted and staged by path)
**Brief / wireframe reviewed:** `docs/handoffs/197-brief.md` (with D-1 to D-4, W-15/W-16 binding, the warns of `handoff-A.md`); no wireframe (the page shows placeholder cards). I also read the issue bodies of #190 to #204 as fetched on 2026-11-22.

## A precondition

A returned **PASS** on the plan (Phase 3, round 3, `docs/handoffs/197/handoff-A.md`). There are no BLOCK findings. Warns 22 to 27 are fixes inside #197's diff, and several of the tests below pin them (22, 23, 24, 25, 26).

## Tests authored

There are two kinds:
- **#197 tests** are live now. They pin what #197 itself delivers: migration 0050, the shared types, `category.rs`, `merge_concerns`, the fake source, the speed-check helper, the synthetic week, the wiring hooks, the guard, and the shared frontend modules.
- **Contract tests** cover each wave-1 "Done when". They are marked `#[ignore = "contract: #N"]` or `it.skip("[contract: #N] …")`. Once unmarked, each must fail against F's stubs. The stub behaviour each one relies on is listed under "Stubs".

### Rust, `gridops-core`

| File | Tests (behavior pinned) | Tier | Why this tier |
|---|---|---|---|
| `crates/gridops-core/tests/fleet_health_schema.rs` (#197) | `the_new_tables_are_strict_with_the_contract_columns`: the five tables are STRICT, with these exact columns, types, NOT NULL flags and keys, and nullable `category` on both job tables (5 categories, no `fix-agent`); `gitlab_jobs` gets `category, job_name, project_path, started_at, completed_at` appended. `the_default_category_rules_are_seeded_in_order`: positions 0..3, the issue's patterns. `the_checks_refuse_values_outside_the_contract`. `window_reads_and_retention_deletes_go_through_an_index`: `completed_at` indexes, and EXPLAIN on the window reads and the three 90-day DELETEs. `the_upgrade_keeps_existing_jobs_and_leaves_them_unclassified`: 49 → 50 keeps rows, leaves the new columns NULL, and seeds the rules once. | integration (real migrator) | Only a migrated SQLite DB shows STRICT, CHECKs and query plans |
| `crates/gridops-core/tests/fleet_schema.rs` (#197, edited) | The upgrade tests now expect 0050's added columns (`workflow_jobs.category`; the five `gitlab_jobs` columns) and skip both job tables in the definition check | integration | Extends the existing upgrade guard rather than loosening it |
| `crates/gridops-core/tests/fleet_health_contract.rs` (#197) | Wire spellings (`JobCategory`, `HealthWindow`, `Severity` order, `ConcernSubject`, the camelCase `Concern` and `ScoreComponent`). `histogram_bucket` boundaries, lower bound inclusive. Default rules classify. Position order across all fields (labels any-match, case-insensitive). `compile` skips an invalid rule and `validate` names positions `[0,2]`. The oversized regex `(?:a{1000}){1000}` is refused. `load_rules` equals `default_rules()`. `merge_concerns` order, live-replaces-windowed, `since: None` last. `fixture.json` equals the Rust types (deny_unknown_fields), `fake::fixture(Day)` and the OpenAPI example. Five machines with their budgets, and every severity. `SLOW_HOST` and `FAILING_STARTS_HOST`. `fixture_inputs` has a backlog hour. `assemble` joins live stats. The fake counts calls, fails on demand, and holds and releases | unit/integration | Pure functions plus a fixture file |
| `crates/gridops-core/tests/fleet_health_speed_check.rs` (#197) | A slow recursive CTE fails the check with "slow statement". A 150 ms write transaction fails it with "write transaction". Fast work passes its value and its error through. `build_week`: under 20 s, the `FLEET_HOSTS` root budgets, 45k–55k jobs, `5*7*24*120` samples, and jobs on the 4 budgeted machines | integration | Its own test binary (W-12: the slow self-test runs in its own binary) |
| `crates/gridops-core/src/gitlab/jobs_tests.rs` (#197, extended in place per W-16) | `names_and_paths_are_filled_once_and_never_overwritten`, `start_and_finish_times_are_set_once_from_the_status_time`, `a_stored_category_is_never_changed_by_a_later_write`; `observe()` sets `job_name: None, project_path: None` | integration | Same harness as the existing GitLab writer rules |
| `crates/gridops-core/src/fleet/registry/query_tests.rs` (#197, D-3(f)/warn 23) | `host_live_reads_match_what_list_hosts_gives_an_admin` | integration | Compares against `list_hosts` on the same seeded fleet |
| `crates/gridops-core/src/fleet_health/score_contract_tests.rs` | 9 × `contract: #192` (component tables, sentences, reweighting, percentiles, fixture, concern thresholds, 1,000 seeded inputs). Live guard `the_scoring_engine_does_no_io` | unit | Pure scoring |
| `crates/gridops-core/src/fleet_health/speed_contract_tests.rs` | 6 × `contract: #193` | integration | Reads seeded jobs |
| `crates/gridops-core/src/fleet_health/classifier_cache_contract_tests.rs` | 2 × `contract: #190` (the cache rebuilds only on a rules change, checked at most once a minute; a GitLab event stores its category) | integration | Real rules table |

### Rust, `gridops-api`

| File | Tests | Tier |
|---|---|---|
| `crates/gridops-api/src/fleet/health_test_support.rs` (#197) | The `Harness` (real routers, admin and member sessions, `TestClock: RegistryClock`), `fixture()`, `openapi()`, `error_code()` | test support |
| `crates/gridops-api/src/fleet/health_wiring_tests.rs` (#197) | An admin GET returns the fixture; a member gets 403 `forbidden`. The `TOKEN_SCOPED_ROUTES` row matches workload's `fleet:read`, and there are no job-category token rows. OpenAPI: health security equals workload's, the window enum, statuses 200/400/401/403/503, and the 3 job-category ops are cookie-only with their statuses. `ApiError::Invalid { code, message, details }` → 422 `{error, code, details}`, and the BadRequest/Forbidden/Refused bodies are unchanged | integration (oneshot) |
| `crates/gridops-api/src/resources/fleet_queue_tests.rs` (#197, D-3(b)/warn 24) | `fleet_queue_breakdown(&pool, now)` counts all 4 open jobs with no user, gives `by_category {Test:2, Build:1, Other:1}` (NULL counts as other), and equals the Overview's `queue_breakdown` field by field for a user who sees both installations | integration |
| `crates/gridops-api/src/webhooks.rs` `mod tests` (#197, W-15) | `a_later_job_event_keeps_the_stored_category` | integration |
| `crates/gridops-api/src/fleet/health_contract_tests.rs` | 5 × `contract: #194` (the example for each window; 400/401/403 with no source call; the 30 s per-window cache; 10 concurrent calls make one computation; stale for under 10 minutes, then 503 `dependency_unavailable`, then recovery) | integration |
| `crates/gridops-api/src/fleet/job_categories_contract_tests.rs` | 5 × `contract: #199` (list; save keeps order, and empty is allowed; 422 names positions [1,3] and saves nothing; preview counts by first match over 7 days and gives the top `other` names; non-admin 403 and anonymous 401) | integration |
| `crates/gridops-api/src/fleet/health_live_contract_tests.rs` | 8 × `contract: #201` (offline at 6 min but not at 4:59, with `since`; budget 0 never offline; start gate at 3 failures and not 2; disk at 5 GiB−1 / 5 GiB / 10 GiB−1 / 10 GiB; unmatched; oldest at 5 min / 5:01 / 10 min / 10:01; uncertain; running counts add up, with NULL as other) | integration |
| `crates/gridops-api/src/webhooks/category_contract_tests.rs` | 1 × `contract: #190` (a GitHub webhook stores test/build/check/deploy/other) | integration |

### Rust, `gridops-reconciler`

| File | Tests | Tier |
|---|---|---|
| `crates/gridops-reconciler/src/fleet_health_wiring_tests.rs` (#197) | `retention_keeps_90_days_of_fleet_health_rows` (91 days old is deleted, 89 is kept, in all three tables); `a_later_poll_keeps_the_stored_category`; `every_fleet_health_pass_succeeds_on_an_empty_database` (backfill, rollup, host rollup, snapshot) | integration |
| `crates/gridops-reconciler/src/fleet_health_rollup_contract_tests.rs` | 4 × `contract: #191`: the exact rows for the seeded hour (unmanaged, the boundary in both directions, re-run attempt 2 only, GitLab, NULL as other, a failing test that isn't an infra failure, a failed placement that is); an identical re-run; a listed hour plus catch-up; 50k jobs under `speed_check` | integration |
| `crates/gridops-reconciler/src/fleet_host_rollup_contract_tests.rs` | 4 × `contract: #200`: a full hour, half an hour, no samples (a row with 0 and NULLs), p50/p95 on a known series, failures not the machine's fault left out, starts and uncertain counted, a same-row re-run, a listed hour, and 7 days × 5 machines under `speed_check` | integration |
| `crates/gridops-reconciler/src/fleet_health_backfill_contract_tests.rs` | 3 × `contract: #190`: a poll insert stores the category; 10,000 rows (1% GitLab) backfilled under `speed_check`, with a re-run that changes nothing; a rule change reclassifies the last 7 days only and lists their hours | integration |

### Frontend and guard

| File | Tests | Tier |
|---|---|---|
| `tests/navigation.test.ts` (#197, extended) | "Health" → `/fleet/health` comes right after Machines in Operate | unit |
| `tests/fleet-health-shared.test.ts` (#197) | The fixture parses with `fleetHealthSchema` (strict). The fixture has more than 5 concerns, every severity, null and non-null scores, a machine at 2 sizes, null and non-null speed indexes, categories, and live per-category counts that add up (what the contract tests need). `fetchFleetHealth(w)` does GET `?window=w` with `no-store` and same-origin, and raises `FleetRequestError` on 403 or a bad body. `scoreBand` (≥90 good, ≥70 fair, else poor, null none). `categoryColor` → `var(--category-<c>)`. `ApiError.code` and `.details` (warn 22), with the 2-argument constructor kept. `metricTiles(m)` and `metricTiles(m, undefined)` are unchanged at 5 tiles | unit |
| `tests/fleet-health-dom.ts` (#197) | Shared jsdom helpers (not a test file) | support |
| `tests/fleet-health-195-frame.contract.test.ts` | 6 × `[contract: #195]` | jsdom |
| `tests/fleet-health-199-job-categories.contract.test.ts` | 4 × `[contract: #199]` | jsdom |
| `tests/fleet-health-202-machines.contract.test.ts` | 4 × `[contract: #202]` | jsdom |
| `tests/fleet-health-203-live.contract.test.ts` | 4 × `[contract: #203]` | jsdom/unit |
| `scripts/fleet-health-guard.test.mjs` (#197) | PR→issue mapping (Closes/Fixes/Resolves in any case; none passes; two fail; #204 is out of scope). Owned files and globs pass; outside the list fails and names the file; deleted or renamed out of the list fails. A shared file accepts added lines only. Removing your own marker passes (Rust line; `it.skip("[contract: #N] x"` → `it("x"`); another issue's marker, any other edit, a renamed test or a deleted file fails; a contract test inside a wave-1 glob stays #197's. `checkOwners` names missing files and doubly owned files. The real owners file is sound and lists exactly 190–195, 199–203 (with 204 as wave 2) | `node --test` |

## RED confirmation

Every run below is saved in the session scratchpad (`red-*.log`).

**Runtime RED** (the assertion fails because migration 0050 is missing):
`cargo test -p gridops-core --test fleet_health_schema` → `test result: FAILED. 0 passed; 5 failed`
- `no such table: job_category_rules` (twice)
- `job_category_rules must be STRICT`
- `no such column: category`
- `workflow_jobs needs an index led by completed_at`

`cargo test -p gridops-core --test fleet_schema` → `14 passed; 3 failed`. The three failures are the upgrade tests: `assertion left == right failed: workflow_jobs: added columns` (twice) and `gitlab_jobs: added columns`. The other 14 still pass.

`npx vitest run tests/navigation.test.ts …`: `× lists Fleet health as Health, right after Machines in Operate (#197)`, with `AssertionError: expected '/workflow-runs' to be '/fleet/health'`. The 3 existing navigation tests still pass.

**Compile/resolve RED.** Every error names an item F delivers; none is a test typo:
- `cargo test -p gridops-core --no-run`:
  - `cannot find fleet_health in gridops_core`;
  - `couldn't read …/src/features/fleet-health/fixture.json`;
  - `cannot find function host_live_reads in module super`;
  - `struct JobObservation has no field named job_name / project_path`.
- `cargo test -p gridops-api --no-run`: 12 errors, all of them:
  - `fleet_health` missing;
  - `super::health_live` missing;
  - `super::fleet_queue_breakdown` missing;
  - `with_fleet_health_source` missing;
  - `ApiError::Invalid` missing;
  - `fixture.json` missing.
- `cargo test -p gridops-reconciler --no-run`: all errors are `cannot find module fleet_health_rollup / fleet_host_rollup / fleet_health_backfill / fleet_health_snapshot` or `gridops_core::fleet_health`.
- `npx vitest run` on the 5 new files: `Failed to resolve import "~/features/fleet-health/…"` / `~/features/settings/job-categories-section` / `fixture.json`.
- `node --test scripts/fleet-health-guard.test.mjs`: `ERR_MODULE_NOT_FOUND … scripts/fleet-health-guard.mjs`.

I fixed two wrong-reason errors in my own tests before declaring RED:
- the seed SQL in `fleet_queue_tests.rs` needed `sqlx::AssertSqlSafe`;
- 4 lines were corrupted by output redaction (a literal `[REDACTED]`); they are repaired and greps clean.

`npx eslint tests/fleet-health-*.ts tests/navigation.test.ts --max-warnings=0` is clean. `tsc` errors in the new files all come from the missing modules. `cargo fmt` has been applied.

**The limit of compile RED.** Rust stops at name resolution, so the bodies of tests over missing types are not type-checked yet. The three core contract files under `fleet_health/` are not compiled at all until F mounts them. If F hits a test-side type error that the signatures below don't explain, report it. That is a test-only rework for me; F must not edit the tests.

## The contract F implements against

Names and signatures the tests use. Anything not listed is F's choice, recorded in `docs/design/fleet-health.md`.

**Mounts F adds.** I added the api, reconciler, webhook and resources mounts already.
- In the new `crates/gridops-core/src/fleet_health/mod.rs`: `#[cfg(test)] mod score_contract_tests; mod speed_contract_tests; mod classifier_cache_contract_tests;`.
- In core `lib.rs`: `pub mod fleet_health;`.
- In api `fleet/mod.rs`: `mod health; mod health_live; mod job_categories;`.
- In the reconciler `main.rs`: `mod fleet_health_backfill; mod fleet_health_rollup; mod fleet_host_rollup; mod fleet_health_snapshot;`.
- In core `Cargo.toml` `[dev-dependencies]`: `gridops-core = { path = ".", features = ["test-support"] }`, because the core integration tests use `testdata` and `speed_check`. api and the reconciler already enable `test-support`.

**`fleet_health` (core), the types:**
- `Concern { key, severity, title, detail, subject, link: Option<String>, since: Option<i64> }`
- `Severity` is `Copy` with `Ord` Critical < Warning < Info.
- `ConcernSubject::{Fleet, Host(String), Pool(String)}`.
- `FleetHealth { window, generated_at, stale, fleet, live, hosts, concerns }`, wire camelCase, `deny_unknown_fields`:
  - `fleet { score: Option<u8>, components: Vec<ScoreComponent> }`;
  - `hosts[] { id, name, cpu_budget_millis: u32, score, components, speed: [{ cpu_millis, index: Option<f64>, … }], categories: [{ category, jobs, … }], live: Option<HostLiveStats>, … }`.
- `FleetLiveStats { queued_total, queued_by_category, running_total: u32, running_by_category: BTreeMap<JobCategory, u32>, … }`.
- `HostLiveStats { host_id, online: bool, last_seen_at: Option<i64>, running_by_category: BTreeMap<JobCategory, u32>, start_failures_last_hour, … }`.

**`fleet_health` (core), the functions and modules:**
- `category::{compile, validate, default_rules, load_rules(&mut SqliteConnection)}`.
- `merge_concerns`, `assemble`, `histogram_bucket`, `HISTOGRAM_BUCKETS`.
- `fake::{FakeFleetHealthSource (Clone; windowed_calls, live_calls, set_failing, hold, release), fixture(window), fixture_inputs, windowed_part, live_part, FIXTURE_NOW, SLOW_HOST, FAILING_STARTS_HOST}`.
- `classifier_cache::{ClassifierCache::new/current/compilations, category_for_insert}`; the stub returns `None`.
- `speed_check::check(&SqlitePool, impl Future<Output = anyhow::Result<T>>) -> anyhow::Result<T>`.
- `score::*` and `concerns::*` as `score_contract_tests.rs` uses them. `speed::{job_key, speed_indexes}`.

**`fleet_health::testdata`** (behind `test-support`):
- `FLEET_HOSTS: [(&str, u32); 5]`; `DISK_BYTES: i64 = 100 GiB` (every seeded root disk).
- `seed_fleet(&pool) -> SeededFleet { hosts: Vec<SeededHost { id, name, cpu_budget_millis, memory_budget_mib, disk_bytes, … } (Clone)>, pool_id: String, repository: String ("acme/app") }`:
  - `host(name) -> anyhow::Result<&SeededHost>`;
  - one installation, repo and pool labelled `["self-hosted","gridops"]` that runs the default `JobSeed`.
- `JobSeed` (Clone), with public fields `id, run_id (default = id), run_attempt (1), workflow ("CI"), name, labels (["self-hosted","gridops"]), host: Option<String>, cpu_millis: u32 (2000), created_at, started_at, completed_at: Option<i64>, status: String, conclusion: Option<String> ("success"), category: Option<JobCategory>, placement: PlacementEnd (Stopped), gitlab: bool`.
  - Constructors: `completed(id, name, host: Option<impl Into<String>>, created, started, completed)`, `queued(id, name, created)`, `running(id, name, host, created, started)`.
  - A running job's runner has `current_job_id` set. `host: None` gives no runner placement (unmanaged).
  - `gitlab: true` writes `gitlab_jobs` through a runner with `gitlab_runner_id`.
- `insert_jobs(&pool, &SeededFleet, &[JobSeed])`:
  - sets `workflow_runs.run_attempt` to the run's highest attempt;
  - writes `workflow_job_sync.run_attempt`.
- `PlacementEnd::{Stopped, Failed, Uncertain, Running}`.
- `SampleSeed::at(observed_at)`: `cpu_used_millis: u32 = 1000`, `memory_used_mib: u32 = 1024`, `disk_free_bytes: Option<i64> = DISK_BYTES/2`, received when observed. `insert_samples(&pool, &SeededHost, &[SampleSeed])` writes the root domain with provenance complete.
- `insert_placement_failure(&pool, &SeededHost, at, PlacementFailureReason, Option<&str>)` writes one `placement_failed` event `{reason, agent_code}`.
- `insert_start_attempt(&pool, &SeededFleet, &SeededHost, at, PlacementEnd)`: reaches `starting` at `at`, ends at `at + 1 s`, and writes no events.
- `build_week(&pool, end) -> WeekSummary { jobs: usize, samples: usize }`: `end` on a whole hour, GitHub jobs only, each its own run at attempt 1, one sample every 30 s for all 5 machines. It must stay well under 20 s, because #191, #200 and the self-test each build one (see the advisory).
- `CLASSIFIER_TEST_LOCK: tokio::sync::Mutex<()>`.

**Other core:**
- `fleet::registry::host_live_reads(&SqlitePool, now)`.
- `gitlab::jobs::JobObservation { job_name: Option<&str>, project_path: Option<&str>, … }`.

**api:**
- `AppState::with_fleet_health_source(Arc<dyn FleetHealthSource>)` and `with_fleet_health_clock(Arc<dyn RegistryClock>)`.
- `ApiError::Invalid { code: &'static str, message: String, details: Value }`.
- `resources::fleet_queue_breakdown(&SqlitePool, now) -> Result<FleetQueueBreakdown { queue: QueueBreakdown, by_category: BTreeMap<JobCategory, i64> }, _>`, re-exported from `resources.rs`.
- `fleet::health_live::collect_live(&SqlitePool, now) -> Result<(FleetLiveStats, Vec<HostLiveStats>, Vec<Concern>), _>`.
- The job-category routes: the body shapes are in the module doc of `job_categories_contract_tests.rs`.

**reconciler:**
- `fleet_health_rollup::{rollup_hour(&SqlitePool, hour, now), run_pass(&SqlitePool, now)}` and `fleet_host_rollup::{rollup_hour, run_pass}`.
- `fleet_health_backfill::run_pass`, `fleet_health_snapshot::run_pass`.
- `upsert_workflow_job(&mut SqliteConnection, &GitHubWorkflowJob, now)` keeps its signature. It reads the workflow name and repository from the run, as #190 says.
- A pass rolls up every listed hour plus at most one catch-up hour.

**Semantics I chose. F records them in the design doc:**
- `job_count` = every final-attempt job completed in the hour; `completed_count` = those with conclusion `success`.
- Unmanaged is `host_id = pool_id = 'unmanaged'` with 0 CPU-seconds.
- Percentiles may be nearest-rank or interpolated; the test series give the same answer either way.
- The persisted rules version (warn 25) is not pinned. Only "a re-run changes nothing" and "a rule change reclassifies the last 7 days and lists their hours" are pinned.

**Frontend:**
- `~/features/fleet-health/types`: `fleetHealthSchema` (strict) and `FleetHealth`.
- `api`: `fetchFleetHealth`.
- `chart-theme`: `scoreBand`, `categoryColor`.
- `page`: `loadHealthWindow`, `saveHealthWindow` (localStorage key `gridops.fleetHealth.window`).
- `areas-of-concern`, `fleet-score-card`, `machine-score-table`, `live-strip` (`LiveStrip`, `formatOldestWait`), and `~/features/settings/job-categories-section` (`JobCategoriesSection`, no props). Components take `{ data, window }` and use only `Link` from the router.
- `metricTiles(metrics, health?)`.
- `ApiError { code?, details? }`.

**Guard:** `scripts/fleet-health-guard.mjs` exports `checkPullRequest({ body, files, owners })`, `issuesInBody`, `checkOwners`, `loadOwners`. The input shapes are in the test file's header.

**Stubs, so each contract test fails once unmarked:**
- The health handler returns the 24h fixture for every window, with no 400 and no cache.
- `collect_live` and the job-category handlers return empty values or 501.
- The rollups and the backfill do nothing.
- `category_for_insert` returns `None`.
- `score` and `speed` return placeholder values.
- The frontend components render a placeholder card with none of the pinned attributes.
- `loadHealthWindow` always returns `"24h"`, and `formatOldestWait` returns `""`.

## Ready for F

The RED is valid: every new test fails because the feature is missing, never because of a test error I know of. F may implement against these tests.

**Advisories for F:**
1. `isActivePath("/fleet/health", "/fleet")` is true today, so Machines and Health would both highlight on the new page. No test pins this; U or S may flag it.
2. The speed-check set must stay under 60 s in CI. That is three `build_week` calls once wave 1 unmarks them, plus a 10k-row backfill, so `build_week` should take about 10 s or less.
3. No test covers "`/fleet/health` renders every placeholder card". That is a coverage hole for Phase 6 and U, not a pass.
