## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** Claim that the scoring denominators divide by `window.millis()` while rows cover only whole hours (`score.rs` ~:481, ~:649, ~:653), and that all current tests use an on-the-hour `FIXTURE_NOW` — the exact lines of `score.rs` and the test fixtures would settle it.

**[NV-2]** Claim that infra-failure share divides by `completed_count` (successes only) at ~:581/:590, and that the design doc defines the denominator as `job_count` — the shown text of `score.rs` at those lines plus the design doc's denominator definition would settle it.

**[NV-3]** Claim that `(online_minutes > 0).then(...)` at ~:653 maps a machine offline all window to "not enough data" rather than 0, and that #200 writes `online_minutes = 0` rows deliberately — the `score.rs` line plus the #200 rollup writer's row-construction code would settle it.

**[NV-4]** Claim that the speed index can list `ratio <= 1.0` keys in `topKeys` (~:180-190) and that duplicate `runners` rows on `github_runner_id` double-count jobs (~:220-221, unique index dropped in migration 0013) — the speed index query text and migration 0013's contents would settle it.

**[NV-5]** Claim that a re-run with attempts in different hours is counted twice because hour H is never re-rolled, and that `workflow_jobs_run_status_idx` supports finding earlier attempts' `completed_at` hours — the rollup loop in `fleet_health_rollup.rs` and the index definition would settle it.

**[NV-6]** Claim that the machine rollup consumes the shared `fleet_health_recompute` queue by deleting all listed hours (~:267-273), including hours that have not yet ended — the shown code of `fleet_host_rollup.rs` at those lines would settle it.

**[NV-7]** Claim that #204's Python oracle would disagree with the current code, which is the stated reason this "must land before #204" — the oracle's spec/design doc and the current scoring code side by side would settle it; this sequencing dependency is asserted, not shown.

### WARN findings
**[W-1]** Items 4 and 5 have no mutation-check requirement in "Done when" (only 1, 2, 3, and 6 do). If the mutation-check discipline is meant to be uniform for scoring changes, items 4 and 5 (both in `score.rs`) are an unexplained gap — either add them or state why they're excluded.

**[W-2]** Item 1's fix ("use the covered time for every time denominator") interacts with #204's in-progress hour, which the brief mentions only parenthetically. The acceptance criteria don't pin down whether the in-progress hour is included in the denominator; a test with off-the-hour `now` could pass under either interpretation. State the intended covered-range formula explicitly so the diff gate can check it.

**[W-3]** Item 8 offers two alternative fixes (consumer column vs. not reading the queue) without choosing. The brief gate should ideally select one, since the two options produce different test shapes (the #197 contract test rewrite differs). Not blocking, but resolve before implementation.

**[W-4]** The "no write transaction over 50 ms" constraint is listed under Constraints but not in "Done when," so nothing requires demonstrating it. Either add it as an acceptance item (e.g., a measured timing in the handoff) or acknowledge it is a constraint verified only by convention.

**[W-5]** Item 7's fix says "roll a catch-up hour only once `hour_end + 15 min <= now`," but item 7's test description covers only the late-insert case. There is no stated test that an hour is still rolled exactly once (the "each hour is still rolled once" claim), nor for the boundary condition at exactly `hour_end + 15 min`. Add those edge cases.

### NIT findings
**[NIT-1]** "Adjust #197's fixture wording if needed" (item 4) is vague; specify what the fixture currently says or defer the wording change to the diff gate.

**[NIT-2]** Item 2 says "update the pinned test. This reverses PR-Agent's 'fix' e71dcf6" — the brief doesn't show what that pinned test asserts, so the test update is unbounded; name the test in the brief.

### Verdict
PASS — no BLOCK findings; implementation may proceed.