## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** crates/gridops-core/src/fleet_health/score.rs:695 (diff) — `score_fleet` now gates the infra share on `total_jobs > 0` and divides `infra_failures / total_jobs`, but the definition of `total_jobs` (presumably `Σ job_count` over the in-window rows, pre-existing in the unchanged part of the function) is not visible in the diff or excerpts; if it is not `job_count`-based the item-2 fix is wrong. Settled by: the unchanged lines of `score_fleet` defining `enough_jobs` and `total_jobs` in score.rs (~:680-695 post-diff).

**[NV-2]** crates/gridops-core/src/fleet_health/speed.rs:179 (diff) — the claim "The index still uses every qualifying key" after `qualifying.retain(ratio > 1.0)` depends on the index being computed from `qualifying` *before* the retain mutates it in place; the index computation is not shown. Settled by: the lines of `speed_indexes` above the `top_keys` block (speed.rs ~:140-180) showing where the geometric mean reads `qualifying`.

**[NV-3] crates/gridops-core/src/fleet_health/speed.rs:236-249 (diff) — the two "newest row" subqueries are claimed to match #191's rollup join exactly; the shown excerpt (fleet_health_rollup.rs:55-61) confirms the subquery text but not that the surrounding `JOIN`/correlation semantics (correlating on `job.runner_id` vs `wj.runner_id`) yield the same rows. Settled by: running the two queries side by side on a seeded database with duplicate runners/placements (the item-11 test covers this; confirming it runs green is the evidence).

**[NV-4]** docs/handoffs/239/evidence.md (not shown) — the brief's Done-when requires a mutation check "shown in the handoff" for items 1, 2, 3 and 6; the handoff explicitly defers these to T's GREEN phase. No excerpt here can show whether they will be produced. Settled by: T's mutation-check output (revert each fix, show its test fails) attached to the handoff or the PR.

**[NV-5]** crates/gridops-core/src/fleet_health/concerns.rs (not shown beyond :56-159) — the claim that no other concern (e.g. a start-reliability or headroom concern keyed on a component score of 0) fires for a budget-0 or fully-offline machine, and that `fleet-component:availability` is the only band-driven fleet concern. Settled by: the full `concerns.rs` (band thresholds and all concern constructors).

### WARN findings
**[W-1]** crates/gridops-reconciler/src/fleet_health_rollup.rs:96-104 (diff) — `EARLIER_ATTEMPT_HOURS` finds earlier attempts via `workflow_jobs_run_status_idx` on `(run_id, status)`, but the outer filter is `sync.run_attempt > 1` on the *hour's* jobs; a job whose `run_attempt` is NULL (pre-0043 rows, counted as 1) that was re-run will never trigger the listing, leaving the old double count. The `COALESCE(earlier_sync.run_attempt,1)` guard handles the earlier side but not this side (the outer is `sync.run_attempt>1`, not `COALESCE(sync.run_attempt,1)>1`). Recommendation: mirror #191's `COALESCE(...,1)` pattern on the outer side too, or document why NULL-attempt re-runs cannot exist by the time a later attempt is recorded.

**[W-2]** crates/gridops-core/src/fleet_health/score.rs:636-641 (diff) — machine availability can exceed 1.0 when the in-progress hour's row reports 60 online minutes against fewer covered minutes (rows cover whole hours through `now`); the share is only saved by the downstream clamp in `measure`. If any future consumer reads the share rather than the score, it sees >100%. Recommendation: clamp the share to 1.0 at construction, or note the reliance on the component clamp.

**[W-3]** fleet_host_rollup.rs:255 (diff, `open_hour = floor_hour(now - ROLLUP_GRACE_MILLIS)`) — the handoff claims this equals `next + HOUR + GRACE <= now` for whole-hour `next`. That equivalence holds (checked), but it is subtle enough that a comment deriving it, as the job rollup's catch-up spells out, would prevent a future "simplification" to `floor_hour(now)`-style drift. Recommendation: add the one-line derivation to the comment.

**[W-4]** crates/gridops-reconciler/src/fleet_health_rollup.rs:341-352 (diff) — the earlier-attempt `INSERT … ON CONFLICT DO NOTHING` runs in the *last* row batch's transaction, but `requested_at` is bound to `now`; a failure of the read phase (`earlier_attempt_hours`) means the whole hour is skipped with no listing, and `reconcile()` only log-and-continues — the double count persists until the hour is listed some other way (e.g. reclassify backfill). The brief's fix says "list those hours … inside the same write transaction", which is satisfied, but the read-failure path is unhandled. Recommendation: state in the module docs that an hour whose read fails is simply retried by the catch-up only if it is still within 7 days, or list the hour itself on failure.

### NIT findings
**[NIT-1]** crates/gridops-reconciler/src/fleet_host_rollup_contract_tests.rs:344-347 (diff) — assertion message is inverted: `assert!(rows(&harness.db, NOW).await?.is_empty(), "the hour in progress was rolled")` — it asserts the hour was *not* rolled.

**[NIT-2]** docs/handoffs/239/evidence.md (Phase 7 fact) — cites `score.rs:566-568` for `first_hour`; the shown excerpt has the fn signature at :569 and body at :570. Off-by-a-few line citation.

**[NIT-3]** crates/gridops-core/src/fleet_health/score.rs (diff, test `availability_on_24h_at_ten_past...`) — comment "1390 of 1440 minutes (83)" uses "(83)" loosely; the full-window score would be round(1390/1440·100·linear-against-80-floor) — the parenthetical is not the actual component score. Cosmetic.

### Verdict
PASS — no BLOCK findings; testing may proceed.