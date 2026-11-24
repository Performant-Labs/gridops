## Brief Review (Round 1)

### BLOCK findings
None.

### Resolved findings
**[R-1]** "Item 4's acceptance test is stated as 'the speed check on the synthetic week stays well under budget' — this is not a testable criterion. No numeric threshold is given anywhere in the brief..." — the brief now names the exact budget (`speed_check::check` fails above 100 ms per statement / 50 ms write-lock hold), defines the synthetic week (`testdata::build_week`: five `FLEET_HOSTS`, ~50,000 jobs, a 30 s sample per machine for 7 days) and names the test pattern to follow (`the_preview_stays_under_the_slow_statement_limit_on_a_week_of_jobs`).

### Needs-verification findings
**[NV-1]** Item 4's claimed baseline measurement ("about 110 ms on one synthetic week") and "Mars has months of history" — the measurement script/output and Mars's actual placement count would settle it. Neither is shown in this prompt; both only motivate the fix, not its acceptance.
**[NV-2]** The synthetic-week volumes (~50,000 jobs, a sample every 30 s per machine for 7 days, five `FLEET_HOSTS`) are asserted from `fleet_health/testdata.rs` and `tests/fleet_health_speed_check.rs` but the constants' text is not quoted in this prompt — the actual constants at `testdata.rs` would settle whether the stated test fixture matches.
**[NV-3]** The claim that `retention.rs` prunes "only the eight `RetentionTable` tables" and that the only `DELETE FROM workload_placements` is in pool deletion (`targets.rs` ~:1037) — quoted excerpts of the `RetentionTable` enum and the placement-delete site would settle it. The brief asserts this in prose; the code text is not shown.
**[NV-4]** Item 7's description of `parse_rules`/`parse_rule` ("check keys, category, field and an empty pattern, and nothing on the list's length or a pattern's length", `job_categories.rs` :100-180) — the shown source of those functions would settle it. Note the brief *does* quote the 422 detail "the pattern is empty" (:132-134), which corroborates the empty-pattern check, but the absence of length checks is asserted, not shown.
**[NV-5]** Item 8's claim that all five listed save handlers call `audit` and that `save_rules` (`job_categories.rs` :221-246) does not — the shown source of those call sites would settle it. The event names are quoted, which corroborates the handlers' behavior, but `save_rules`'s lack of an audit call is asserted.
**[NV-6]** Item 9's claim that `add()` sets `pattern: ""` (:118) and that `latestPreview` (:68-80) drops but does not abort older responses — the `.catch` and "Preview paused" text is quoted, but these two specifics are not.

### WARN findings
**[W-1]** Item 3 still offers two alternative fixes ("Compute it once per request cycle (or cache it for 30 s)") without picking one. The two give observably different behavior (a 30 s live cache changes response freshness), so the "test that fails before the fix" is not yet determinate. Pick one.
**[W-2]** Item 6's fix ("count busy GitLab runners too") is under-specified: it doesn't say whether busy runners count as running when `current_job_id` is NULL, how `status='busy'` interacts with the `wj` join, or what the runner's job is reported as. The row is joined to `workflow_jobs wj` at :82-93 (quoted); a busy GitLab runner has no `wj` row, so the counting predicate must be defined. State the intended SQL condition.
**[W-3]** Item 5's fix ("match the Overview, and skip offline machines") delegates the spec to `disk_alerts`'s code. Acceptable, but the brief should state whether the health page must reproduce the Overview's threshold levels (warning/critical) and heartbeat window (120 s) verbatim, or only the source of `total_bytes`; the "test that fails before the fix" depends on that.

### NIT findings
**[NIT-1]** Item 2's 30 s failure backoff and 2 s timeout have no explicit "Done when" coverage beyond "each item has a test that fails before the fix"; the OpenAPI criterion only mentions the 503/stale semantics. Consider naming the 30 s backoff window in the OpenAPI/staleness documentation decision too.
**[NIT-2]** Item 1's requirement that "a cache hit on any window must not wait on the live computation either" is stated only as a rider to item 3; the locking topology should live in item 1's fix text so the per-window refactor is reviewed against both conditions in one place.

### Verdict
PASS — no BLOCK findings; implementation may proceed. (W-1 and W-2 should be resolved during implementation, but they are under-specifications the implementer can settle, not brief-level contradictions.)