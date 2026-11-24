## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** Item 1's claim that the three denominators divide by `window.millis()` at score.rs ~:481/:649/:653 — an excerpt of those lines from `crates/gridops-core/src/fleet_health/score.rs` would settle it.
**[NV-2]** Item 1's claim that every existing test uses an on-the-hour `FIXTURE_NOW` — the test files' fixture definitions (file:line) would settle it.
**[NV-3]** Item 1's grace-hour contract ascribes behavior to #204's Python source ("computes live, bounded, every hour of `[ceil_hour(now − W), now)`", "detects an unwritten hour by time, not markers") — #204's oracle source file:line would settle it; this contract is written into a Done-when rule (a) and must be right before the oracle is trusted.
**[NV-4]** Item 2's claim that ~:581/:590 divide by `completed_count` and that `completed_count` counts successes only — the shown source at those lines plus the rollup code that computes `completed_count` vs `job_count` would settle it; likewise the claim that e71dcf6 reversed this.
**[NV-5]** Item 3's claim that ~:653 gates on `(online_minutes > 0)` and that #200 writes `online_minutes = 0` rows deliberately — the score.rs line and the #200 rollup write path (file:line) would settle it.
**[NV-6]** Item 4's claim that "the design says" a CPU-budget-0 machine has no score — the relevant passage in `docs/design/fleet-health.md` (file:line) would settle it.
**[NV-7]** Item 6's fix assumes `workflow_jobs_run_status_idx` can locate earlier attempts' `completed_at` — the index definition in `migrations/` and a query plan would settle it.
**[NV-8]** Item 8's claims that the machine rollup deletes listed hours at fleet_host_rollup.rs ~:267-273 and rolls unended hours — the shown code at those lines would settle it, as would the existence of the #197 contract test by that exact name.
**[NV-9]** Item 10's claim that topKeys include ratios ≤ 1.0 at speed.rs ~:180-190 — that excerpt would settle it.
**[NV-10]** Item 11's claim that ~:220-221 joins on `github_runner_id` and that the unique index was dropped in migration 0013 — the speed.rs excerpt and `migrations/0013_*.sql` would settle it.
**[NV-11]** The shared 15-minute grace constant referenced in item 1 as coming "from items 7 and 9" — items 7/9 *define* the fix here, so there is no pre-existing constant in this prompt; where that constant lives (a file:line after implementation, or #204's spec) must be pinned so scorer, job rollup, machine rollup and oracle agree on one value.
**[NV-12]** Done-when rule (i) — "the fleet median includes the machine's own runs" — describes existing speed-index behavior with no fix item; the speed.rs median computation excerpt would settle whether this is a documentation-only statement or an unstated behavior change.

### WARN findings
**[W-1]** Item 6's fix (list earlier attempts' hours inside the same write transaction) interacts with item 8's decision that only the job rollup reads `fleet_health_recompute` — fine — but the ordering guarantee that the job rollup's recompute listing happens with the hour-write in one ≤50 ms transaction is asserted, not designed. Recommend the brief state the transaction boundary explicitly (one recompute listing per rolled hour, batched ≤500 rows).
**[W-2]** Item 1's covered-time fix leaves unspecified what happens when an hour in the covered window is *neither rolled nor live-computable* (e.g., a hole the 15-minute grace has passed but #204 hasn't covered because it hasn't run). Recommend the design-doc rule (a) state whether holes in the covered window are scored as absent (deflating the denominator), or treated as missing data.
**[W-3]** Item 5's suggested private `measure()` returns `(ScoreComponent, Option<f64>)`; the semantics of the `Option` (no-data components?) is unspecified while the public `combine(&[ScoreComponent])` signature is frozen. Recommend pinning whether a `None` raw value still produces a `ScoreComponent` with a "not enough data" band, so the public behavior is unchanged.
**[W-4]** "SQLite discipline holds: no write transaction over 50 ms" has no stated measurement method. Recommend specifying how it is verified (e.g., log-based timing assertion in tests), otherwise the criterion is unauditable.
**[W-5]** Scope check: the brief fixes wave-1 defects across five files and rewrites design-doc rules, and it gates #204. That is bounded, but item 8 also rewrites an existing #197 contract test and item 4 touches #197 fixture wording — call out in the PR which #197 behaviors are deliberately re-baselined so the diff gate can distinguish regression from intent.

### NIT findings
**[NIT-1]** Item 2's "reverses PR-Agent's 'fix' e71dcf6" should cite the commit message or diff so the history claim is checkable.
**[NIT-2]** Item 1's example percentages (97.9%, 50%) are derivable but would be clearer with the arithmetic shown once.

### Verdict
PASS — no BLOCK findings; implementation may proceed.