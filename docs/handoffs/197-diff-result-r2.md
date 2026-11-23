## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** crates/gridops-reconciler/src/main.rs:~2172 — claim: `category_for_insert` at the poll's `upsert_workflow_job` call sites is never invoked on a connection that already holds an open write transaction (W-15). — Evidence: the call sites of `upsert_workflow_job` in the poll path (`crates/gridops-reconciler/src/main.rs`, the functions that acquire `connection`), shown as file:line excerpts.

**[NV-2]** crates/gridops-core/src/fleet_health/speed_check.rs:~70-160 — claim: SQLite's update/commit/rollback hooks fire on the connection's worker thread and `tracing::dispatcher::set_default` set from a hook routes sqlx's slow-statement WARN to the check's subscriber, so slow statements are actually caught (the design itself lists what is not caught). — Evidence: a passing run of `cargo test -p gridops-core --test fleet_health_speed_check` on CI, and/or sqlx/tracing source showing the log site's target (`sqlx::query`) and thread.

**[NV-3]** .github/workflows/ci.yml:~86 — claim: `runner.environment == 'github-hosted'` is a valid step-level condition expression on the runner types this job selects, so `actions/setup-node` runs on hosted runners only. — Evidence: GitHub Actions docs for the `runner` context (`runner.environment`, `runner.name`) and a green run of the `Fleet health guard` job on both runner kinds.

**[NV-4]** whole PR — claim: every wave-1 contract test fails against the stubs once its marker is removed (acceptance: "Prove it by removing all markers on a scratch branch"), and the speed-check set stays under 60 s in CI. — Evidence: the marker-removal run's log (T-red round 2, referenced as `handoff-T-red.md` but not shown here) and the CI timings.

**[NV-5]** crates/gridops-core/src/fleet/registry/queries.rs:~2325-2385 — claim: `Snapshot { user_id: "", system_admin: true }` makes the private helpers (`counts`, `latest_measurement`) return the same values `list_hosts` gives an admin (D-3(f): "no parallel queries"). — Evidence: the bodies of `counts` and `latest_measurement` in `queries.rs`, and the passing `host_live_reads_match_what_list_hosts_gives_an_admin` test run.

### WARN findings
**[W-1]** docs/design/fleet-health-owners.json:195-entry — the brief's ownership table gives #195 "any `src/features/fleet-health/frame-*.tsx`"; the owners file omits that glob with no recorded resolution (the design doc's page section likewise drops the "frame" concept in favour of slots, but neither handoff says so). — Recommendation: add the glob (harmless while empty) or record the decision to drop it in the design doc's Ownership section, per the brief's "resolve it in the plan, say which way and why".

**[W-2]** PR-level — two acceptance items of #197 are explicitly deferred and not yet done: the Mars-copy migration timing (recorded wall time in the PR description) and the two live guard test PRs (closed afterwards, head SHAs recorded). The handoff carries them for O; they must land before merge or the "Done when" list is unmet. — Recommendation: track them as merge-blocking checklist items for O; do not merge without them.

**[W-3]** crates/gridops-api/src/fleet/health.rs:~85 — `live(now)` is fetched after `windowed` on every request with no cache and both halves must succeed; a live-half failure 503s the whole page even when the windowed half is fine. This is documented as #194's scope, but the failure coupling is worth stating in the design doc. — Recommendation: note in `docs/design/fleet-health.md` that the live half is uncached and fails the request until #194.

### NIT findings
**[NIT-1]** scripts/fleet-health-guard.mjs:~150 — `const escape = …` is defined and never used.

**[NIT-2]** crates/gridops-api/src/fleet/health_test_support.rs:~60 — `TestClock::set` carries a `dead_code` allow pending #194; fine, but a comment with the issue marker already exists — the allow can be dropped when #194 lands (already noted in T's list).

### Verdict
PASS — no BLOCK findings; testing may proceed.