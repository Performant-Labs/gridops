# [#240](https://github.com/Performant-Labs/gridops/issues/240)

Part of #198. Found by the Opus review of the wave-1 PRs (#228, #234, #233). The epic called for this review; it ran after they merged. **Must land before #204**: #204 owns the source selection in `health.rs` and must meet p95 < 300 ms cold. References are to master at 73de941.

## Endpoint cache (`crates/gridops-api/src/fleet/health.rs`, from #194 / PR #228)

1. **One mutex for every window, held across the computation.** ~:53-56 and :147-154. A cold 7d computation (with the speed index after #204) blocks fresh cache hits for 1h, 24h and the Overview tile.
   - Fix: a lock per window. A cache hit must never wait on a computation.
   - Verified at 73de941: `HealthCache` is `entries: Mutex<HashMap<HealthWindow, CachedHealth>>` (one `tokio::sync::Mutex`, :53-56), and `health()` takes `health.cache.entries.lock().await` at :147, checks the entry at :149-153, then awaits `compute(health, window, now)` at :154 with the guard still held, so a hit on any window waits behind a miss on any other.
   - With item 3, the same rule covers the shared live part: a cache hit on any window must not wait on the live computation either, so sharing the live part across windows must not bring back one lock that every window's hit waits on.
2. **No timeout, and failures aren't remembered.** If the source hangs, as SQLite does in a #179-style lock storm, the lock is held forever and every request hangs. During an outage every request calls the failing source again, one at a time.
   - Fix: wrap the compute in `tokio::time::timeout` (2 s) and treat a timeout as a failure. After a failure, wait 30 s before trying again, serving stale or 503 meanwhile.
   - Test both with the fake source: one that hangs, and one that fails.
   - "Stale or 503" is the existing rule, unchanged (`health.rs` :38-41 and :166-176, `docs/design/fleet-health.md` "Cache (#194)"): the last good response for that window is served with `stale: true` while it is under 10 minutes old (`STALE_MAX_AGE_MILLIS`, measured from when it was computed), and `503 dependency_unavailable` otherwise, including when there is no earlier response. A timeout is handled the same way as any other failure.
   - What "hangs" means here: the pool's `busy_timeout` is 5 s per statement (`crates/gridops-core/src/db.rs` :35), so under a #179-style lock storm (CHANGELOG, #179: writers held the lock, "other writers timed out and the API answered agents 503") each statement of a compute can wait up to 5 s, and a compute of many statements far longer, all with the cache lock held. The fake source's "hangs" case is a future that never resolves.
3. **The live part is computed once per window.** Compute it once per request cycle (or cache it for 30 s) and share it across windows.
   - Verified: `compute()` (`health.rs` :182-190) awaits `health.source.windowed(window, now)` and then `health.source.live(now)` for every window's miss, so each of the three windows reads the live part separately.

## Live stats (`crates/gridops-api/src/fleet/health_live.rs`, from #201 / PR #234)

4. **The live read slows down as history grows.** `host_live_reads` → `counts()` (`crates/gridops-core/src/fleet/registry/queries.rs` ~:1243) filters only on `host_id`, so it reads every placement the machine has ever had. Retention never deletes placements.
   - Verified: `host_live_reads` (queries.rs :2340-2389) calls `counts(&mut tx, &snapshot, host_id, None, None, CountScope::Host)`; with no backend, no target filter and `CountScope::Host`, the statement built at :1263-1271 ends `FROM workload_placements p JOIN workload_intents i ON i.id=p.intent_id LEFT JOIN runners r ON r.id=p.runner_id WHERE p.host_id=?`, with no state condition. `crates/gridops-core/src/retention.rs` prunes only the eight `RetentionTable` tables (webhook deliveries, audit events, capacity samples, host samples, reconciler passes and the three fleet health tables); the only `DELETE FROM workload_placements` is in pool deletion (`crates/gridops-core/src/fleet/targets.rs` ~:1037), not retention.
   - The index exists: `CREATE INDEX workload_placements_host_idx ON workload_placements(host_id,state);` (`migrations/0022_fleet_foundation.sql` :267). Placement states are `reserved`, `preparing`, `registering`, `starting`, `running`, `draining`, `stopping`, `stopped`, `cleaned`, `retryable`, `failed` and `uncertain` (the same file, :251).
   - It measured about 110 ms on one synthetic week. Mars has months of history.
   - Fix: a live variant limited to non-terminal states through `workload_placements_host_idx(host_id,state)`. Use it from the live path.
   - Test: the live path (`host_live_reads`, and `collect_live` which calls it) passes the speed check on the synthetic week. That is the budget: `gridops_core::fleet_health::speed_check::check(&pool, work)` fails when any statement on the database takes over 100 ms (the #179 slow-statement limit, `db.rs` `SLOW_STATEMENT`) or any write transaction holds the write lock over 50 ms (`WRITE_LOCK_LIMIT`). The synthetic week is `fleet_health::testdata::build_week(&pool, end)`: the five fixture machines (`FLEET_HOSTS`), about 50,000 jobs and a sample every 30 s per machine for 7 days (`crates/gridops-core/tests/fleet_health_speed_check.rs` checks those volumes). The pattern to follow is `the_preview_stays_under_the_slow_statement_limit_on_a_week_of_jobs` in `crates/gridops-api/src/fleet/job_categories.rs`.
5. **The disk alarm uses the admin-set budget, not the disk.** ~:174-178 uses `host_resource_domains.disk_bytes`. The Overview's existing alert (`resources.rs` ~:5918-5950) uses the observed root capacity, and only for machines with a recent heartbeat.
   - Example: a 2 TB root with a 500 GiB budget and 60 GiB free. The Overview says critical, the health page says nothing.
   - Fix: match the Overview, and skip offline machines.
   - Verified: `health_live.rs` :174 is `let disk_total = read.physical_budget.disk_bytes.get();`, and `physical_budget` comes from `root_budget` (queries.rs :1309-1318), `SELECT cpu_millis,memory_mib,disk_bytes FROM host_resource_domains WHERE host_id=? AND parent_domain_id IS NULL`. The Overview's `disk_alerts` (`crates/gridops-api/src/resources.rs` :5918-5955) takes `total_bytes` from `fleet_observed_domains d ON d.host_id=fh.id AND d.local_key=<root key>` and free space from the newest `host_samples.disk_free_bytes`, and only for hosts `WHERE fh.enrollment_state='approved' AND fh.lifecycle_state NOT IN ('retired') AND fh.last_heartbeat_at>=?` bound to `now - 120_000`.
6. **GitLab runners never count as running.** ~:82-93 keys on `current_job_id`, which `set_gitlab_runner_busy` (`gitlab/jobs.rs` ~:363) never sets.
   - Fix: count busy GitLab runners too.
   - Verified: the query at `health_live.rs` :82-93 has `LEFT JOIN workflow_jobs wj ON wj.id=r.current_job_id` and `AND r.current_job_id IS NOT NULL`. `set_gitlab_runner_busy` (`crates/gridops-core/src/gitlab/jobs.rs` :355-379) runs `UPDATE runners SET busy=?1,status=CASE WHEN ?1 THEN 'busy' ELSE 'online' END,last_heartbeat_at=?2,updated_at=?2` and its doc comment says it "Writes only `busy`, `status` (`busy` or `online`), `last_heartbeat_at` and `updated_at`". The only production writer of `current_job_id` is `associate_runner_with_job` (`crates/gridops-core/src/db.rs` :206-245), which matches by runner name for `workflow_jobs` (GitHub) jobs.

## Job category rules (`crates/gridops-api/src/fleet/job_categories.rs` and `src/features/settings/job-categories-section.tsx`, from #199 / PR #233)

The UI file is at `src/features/settings/job-categories-section.tsx` (the repository has no `web/` directory).

7. **No limit on rule count or pattern length.** The body limit is 25 MB.
   - Verified: the job category routes are in `session_routes()` (`crates/gridops-api/src/main.rs` :373-380), and the app router carries `.layer(DefaultBodyLimit::max(25 * 1024 * 1024))` (main.rs :133); the fleet router's smaller `DEFAULT_JSON_BYTES` limit (`fleet/mod.rs` :313) does not cover them. `parse_rules` / `parse_rule` (`job_categories.rs` :100-180) check keys, category, field and an empty pattern, and nothing on the list's length or a pattern's length.
   - Thousands of rules compile on an async worker, save in one long write transaction, then get compiled again by every classifier.
   - Fix: `parse_rules` returns 422 above 200 rules or 1 KiB per pattern, with messages per field. Add the same check to the UI.
8. **Saves aren't audited.** Every other settings save writes an audit event.
   - Verified: `save_settings` (`resources.rs` :3612, `"settings.updated"`), `save_builtin_runner_host` (:3708, `"settings.builtin_runner_host_changed"`), `save_builtin_runner_host_budget` (:3833, `"settings.builtin_runner_host_budget_changed"`), `save_agent_settings` (`agent.rs` :543, `"fix_agent.settings_updated"`) and the AI connection writes (`agent.rs` :417 `"ai_provider.connected"`, :495 `"ai_provider.disconnected"`) all call `audit`; `save_rules` (`job_categories.rs` :221-246) does not.
   - Fix: `audit(..., "job_categories.updated", ...)` with the lists before and after, in the same transaction.
   - Note for the implementer: the existing `audit` (`crates/gridops-api/src/auth.rs` :435-448) takes `&AppState` and inserts into `audit_events` on `state.database` (the pool), and every handler above calls it after `transaction.commit()`. Writing it in the same transaction therefore needs the same insert run on the save's transaction (for example a variant of `audit` that takes the connection), with the same `audit_actor` label and metadata.
9. **One message for every preview failure.** The UI says "a rule does not compile" for a blank new row, a 5xx and network errors alike.
   - Verified: the preview's `.catch(() => { ... setPreview(null); setPreviewPaused(true); })` (`job-categories-section.tsx` :82-89) ignores the error, and `previewPaused` renders "Preview paused: a rule does not compile." (:204). A newer request only drops an older response (`latestPreview`, :68-80); nothing is aborted. A blank new row (`add()` makes `pattern: ""`, :118) is refused by the server as a 422 whose detail is "the pattern is empty" (`parse_rule`, `job_categories.rs` :132-134), not as a compile failure.
   - Fix: show it only for a 422, and attach each error to its row.
   - Also cancel an in-flight preview with `AbortController`. `api()` (`src/lib/api.ts` :12, :33-42) passes its `RequestInit` options, `signal` included, through to `fetch`.

## Done when

- [ ] Each item has a test that fails before the fix.
- [ ] The speed check passes.
- [ ] The OpenAPI spec is updated for any new 422 details.
- [ ] The OpenAPI spec's `GET /api/v1/fleet/health` responses still describe when a 503 is served and that a stale response is a 200 with `stale: true`; the 503 is documented today as "The health source failed and there is no response under 10 minutes old to serve" (`docs/openapi/gridops-v1.json`), and its description is updated if item 2 changes when a 503 is served.

## Constraints

- Keep `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`. Use the shared cargo target from the untracked `.cargo/config.toml`, and never set `CARGO_TARGET_DIR`. Prefer `-p` crate test runs.
- Work headless: no visible browsers and no screenshots on the desktop.
- Open the PR against master on Performant-Labs/gridops, never upstream iamngoni/gridops.

## Source material read

- `crates/gridops-api/src/fleet/health.rs` (in full)
- `crates/gridops-api/src/fleet/health_live.rs` (in full)
- `crates/gridops-core/src/fleet/registry/queries.rs` (`counts`, `root_budget`, `HostLiveRead`, `host_live_reads`: :1243-1318, :2315-2389)
- `crates/gridops-core/src/fleet_health/speed_check.rs` (in full)
- `crates/gridops-core/src/fleet_health/testdata.rs` (module header and constants, :1-60)
- `crates/gridops-core/tests/fleet_health_speed_check.rs` (in full)
- `crates/gridops-core/src/retention.rs` (in full)
- `crates/gridops-core/src/fleet/targets.rs` (the placement delete, ~:1000-1050)
- `crates/gridops-core/src/db.rs` (pool options :20-50, `associate_runner_with_job` :206-245)
- `migrations/0022_fleet_foundation.sql` (`workload_placements` and its indexes, :236-269)
- `crates/gridops-api/src/resources.rs` (`disk_alerts` :5877-5955; the settings save handlers :3612-3930)
- `crates/gridops-core/src/gitlab/jobs.rs` (`set_gitlab_runner_busy`, :340-379)
- `crates/gridops-api/src/fleet/job_categories.rs` (in full)
- `crates/gridops-api/src/main.rs` (router and body limit :100-150, `session_routes` :270-400)
- `crates/gridops-api/src/auth.rs` (`audit`, `audit_actor`, :435-465)
- `crates/gridops-api/src/agent.rs` (`save_api_key` :163-199 and the `audit` call sites)
- `src/features/settings/job-categories-section.tsx` (in full)
- `src/lib/api.ts` (:1-50)
- `docs/openapi/gridops-v1.json` (`/api/v1/fleet/health` responses, the job category 422s)
- `docs/design/fleet-health.md` (the source, cache and speed check sections)
- `CHANGELOG.md` (the #179 entries)
- `web/src/features/settings/job-categories-section.tsx`: not found (the file is at `src/features/settings/job-categories-section.tsx`)
