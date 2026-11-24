# Handoff-A: Phase 3 - Fix scoring, rollup and speed-index defects from the wave-1 review (#239)  (up-front plan review, round 2)

**Date:** 2026-11-24
**Branch:** claude/fh-239 (head a81489f, which amends the brief after round 1's BLOCK at 0b435f8; code base 73de941; origin/master 20c22ad, one build-only commit ahead, #238)
**Brief reviewed:** docs/handoffs/239-brief.md (blob f4a2e56, amended)   **Reuse map:** none (no `docs/handoffs/239/survey.md`). Every item fixes an existing object in place: `score.rs` (#192), `speed.rs` (#193), `fleet_health_rollup.rs` (#191), `fleet_host_rollup.rs` (#200), and the design doc. The only new object is one grace constant.   **Wireframe:** N/A (no UI)
**Verdict:** PASS

## Summary
PASS, with 0 block and 8 warn findings. The amendment resolves both round-1 blocks:
- **Block 1 [completeness], resolved.** Item 1 now states the grace-hour contract with #204: live, by time, with one shared constant, plus its own test.
- **Block 2 [completeness], resolved.** The Done-when now requires the design doc to state rules (a)-(i), and the Constraints forbid editing `0050`.

The brief also adopts round 1's choices for item 5 (private to `score.rs`) and item 8 (the machine rollup stops reading the queue), and its closing-keyword rule. Every item still extends the object that owns the behaviour, so there is no parallel path.

Three new warns refine the contract that the amendment made load-bearing:
- the job rollup's listed-hours loop must honour the same grace;
- the covered-time denominator must be in the numerator's unit, whole minutes;
- item 6 has an interim double count that the design doc has to state.

Each is resolvable inside the brief's own items without a product decision, so each is a warn binding on F, not a block.

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | Item 1: "#204 detects an unwritten hour **by time** … It does not use the rollups' markers", with item 7's grace scoped to "a catch-up hour" | cross-cutting (the rollup → source → scorer contract) | **Detecting an unwritten hour by time needs an invariant the plan doesn't create: no rollup writes an hour before `hour_end + grace`.**<br>- The job rollup's listed-hours loop rolls any listed hour that has ended (`fleet_health_recompute WHERE hour<=now−1h`, `fleet_health_rollup.rs:158-165`), with no grace.<br>- #190's backfill (open PR #237, `list_hours`) lists the completion hours of every job a rules change moved in the last 7 days, and that can include the hour that just ended.<br>- So after a rules change between :00 and :15, `fleet_health_hourly` has rows for an hour #204 treats as unwritten and computes live. #204's issue says it "reads `fleet_health_hourly` … for the window, plus" the live hours, so it would count that hour twice.<br>- Late arrivals aren't lost, because the catch-up's `rollup_next` marker rolls the hour again at +15 min (:172-179). The double count is the only problem.<br>- It is rare and bounded, and the fix fits inside item 7, so it's a warn. | Binding for F:<br>- Apply the grace to the listed-hours filter too: `hour + HOUR_MILLIS + ROLLUP_GRACE_MILLIS <= now`. This makes "written ⟺ past grace" true of both rollups. (After item 8, the machine rollup has no listed loop.)<br>- Design doc rule (e): "no hour is rolled, listed or catch-up, before `hour_end + 15 min`".<br>- Rule (a): "#204 reads hourly rows only for hours past their grace, and computes the rest live".<br>- Rule (a) should also say what happens to a past-grace hour that has no rows yet (pass lag, at most `reconcileIntervalSeconds`, 30 s by default, `main.rs:508`; catch-up after downtime, one hour per pass). It is covered time with nothing in it. That is today's behaviour, and it was the brief gate's W-2.<br>- #197's contract tests roll `H = NOW − 2h` (`fleet_health_rollup_contract_tests.rs:39`), so none of them change. |
| 2 | warn | Item 1: "use the covered time, `now − ceil_hour(now − W)`, for every time denominator"; item 3: 0 online minutes scores 0 | contract shape (the numerator's unit); refines round 1's warn 5 | **`online_minutes` counts whole minutes, but the brief gives the denominator in ms.**<br>- The numerator is the count of distinct minutes with at least one sample (`fleet_host_rollup.rs:101-104,153`), and #204's live hour will count the same way.<br>- Divided by fractional covered minutes, a perfect machine on 1h, with the current minute still unsampled, reads low. The synthetic week samples every 30 s (`fleet-health.md:402`).<br>- At H+5 min 29 s it reads 5 ÷ 5.48 = 91%, which scores 56. At H+1 min 29 s it reads 67%, which scores 0 and raises a critical concern. That is item 1's own symptom, smaller but still there.<br>- The brief's tests (10:30, H+10 min) fall on whole minutes, so they can't show it.<br>- Separately, item 3 removes the host path's only zero guard, `(online_minutes > 0)` at `score.rs:653`. When covered time is 0 minutes, 0/0 gives NaN, `raw.round() as u8` turns it into 0, and a critical concern fires. | Binding for F:<br>- Add one private helper in `score.rs` for the covered span in ms, `now − ceil_hour(now − W)`. Use it at :481, :603, :618 and for `capacity_misfit`'s first hour (:508).<br>- Availability (fleet and machine) divides by whole covered minutes, `covered_ms / 60_000` as an integer, the way `window.millis() / 60_000` does today.<br>- Apply the fleet path's existing `> 0` guard (:495) to both. 0 covered minutes is `None` ("not enough data"), which is different from item 3's 0 online minutes over covered time.<br>- A perfect machine then reads at least 100%, because the minute in progress can add one, and `higher` clamps the score to 100.<br>- Busy share keeps seconds (`covered_ms / 1000`).<br>- Rule (a) states the minute unit and the 0-minute case, so #204's oracle matches.<br>- Test cases to pin: 1h at H+5 min 29 s with the current minute unsampled scores 100; H+0 min 29 s gives `None`, not 0.<br>- For O, a follow-up: on 1h, busy share is still noisy in each hour's first minutes, because jobs count at completion over a span of seconds. Whether time shares get a minimum span, like the 20-job minimum, is a spec decision, so it isn't invented here. |
| 3 | warn | Items 1, 7 and 9: "the one shared grace constant" | dependency direction | **The constant is now a cross-crate contract, but the brief doesn't say where it lives.**<br>- #204 (`gridops-api`) needs it.<br>- `gridops-api` depends only on `gridops-core` (`crates/gridops-api/Cargo.toml:17`). The reconciler's constants are private, and `fleet_host_rollup.rs` already defines its own `HOUR` and `MINUTE` (:26-27). | - Add `pub const ROLLUP_GRACE_MILLIS: i64 = 15 * 60_000` in `gridops_core::fleet_health`, beside `HOUR_MILLIS` (`mod.rs:52`).<br>- Both rollups' conditions use it, finding 1's listed filter included. There is no second copy.<br>- Leave the two settings keys alone, because their stored values are live. |
| 4 | warn | Item 6, and Done-when (d) | cross-cutting (the oracle's view of final attempts) | **Item 6 fixes the permanent double count, but an interim one is left for #204's oracle to trip on.**<br>- The rollup's final-attempt rule is global (`MAX(later.run_attempt)` over the run, `fleet_health_rollup.rs:60-64`), so re-rolling the earlier hour does drop attempt 1.<br>- Until then, attempt 1 still counts in its rolled hour, and #204's live hour counts attempt 2. That lasts until attempt 2's hour is rolled at its end + 15 min, plus one pass for the listed hour.<br>- The oracle computes final attempts from raw data, so an end-to-end seed whose re-run crosses into an unrolled hour would disagree. | Rule (d) also says: "until the later attempt's hour is rolled, the earlier attempt still counts in its own hour". The oracle can then model this, or #204's seed can avoid it. |
| 5 | warn | Item 6: "find its earlier attempts' `completed_at` hours … List those … inside the same write transaction" (round 1, warn 7; brief gate W-1) | layering (reads outside the write transaction, `fleet-health.md:126`) | **Unchanged from round 1.** Only the insert can go inside the transaction. If the lookup goes there too, it breaks the read/write split. If the insert runs after the hour's own `DELETE`, or lists H itself, H stays listed and is rolled again on every pass. | - Do the lookup in the read phase, next to `hour_facts`, through `workflow_jobs_run_status_idx`.<br>- Use `INSERT … ON CONFLICT(hour) DO NOTHING` in the last write batch, before `DELETE FROM fleet_health_recompute WHERE hour=?` (:290-295).<br>- Leave H itself out, and keep batches at 500 rows or fewer. |
| 6 | warn | Item 11: "Use the newest-row subqueries #191 uses" (round 1, warn 8) | pattern consistency | **Unchanged from round 1.** `speed.rs:221` also joins every `workload_placements` row, and `workload_placements_runner_idx` isn't unique. | - Copy both of #191's subqueries inline: the runner (`fleet_health_rollup.rs:51-53`) and the placement (:54-57).<br>- Add no shared SQL fragment, because each statement writes these inline.<br>- `speed_check`'s 100 ms check must stay green. |
| 7 | warn | Tests for items 1-11 (round 1, warn 9) | file structure (`fleet-health.md` Ownership) | **Unchanged from round 1.** | - Put each item's tests in the owning module's `#[cfg(test)] mod tests`, adding one to `fleet_health_rollup.rs`, and seed through `gridops_core::fleet_health::testdata`.<br>- In #197's contract files, change only item 8's `a_pass_rolls_up_a_listed_hour_and_takes_it_off_the_list` (`fleet_host_rollup_contract_tests.rs:318`).<br>- If item 4 changes the MacBookPro wording, change `fake.rs`, `src/features/fleet-health/fixture.json` and the `OpenAPI` example together.<br>- Prove the 50 ms and 500-row rule with the existing `speed_check` week tests. |
| 8 | warn | Work running at the same time | cross-cutting (coexistence) | #240 (open, worktree `fh-240`) also edits `docs/design/fleet-health.md`. PR #237 (#190) is open on the backfill. Its diff touches neither rollup, so finding 1's listed-loop change doesn't conflict with it. | - Whichever of #239 and #240 lands second rebases the design doc.<br>- The closing-keyword rule is now in Constraints. |

## Notes for O
Not required (PASS). Two points for follow-up work:
- Finding 2's busy-share noise at the top of each hour on the 1h view, if O wants a minimum span for time shares.
- Making #204's issue body match the contract the design doc will state: rows only for hours past grace, everything else live.

## Placement and layering, confirmed (recorded for T and F; supersedes round 1's section)
- **`score.rs` (items 1-5).** Pure changes inside #192's module, and `the_scoring_engine_does_no_io` still holds.
  - Item 1 follows finding 2.
  - Item 3 uses `rows.is_empty()` for "no data".
  - Item 4 is handled in `score_fleet`'s per-host map: `cpu_budget_millis == 0` gives every component `None`, with `score: None` and `busy_share: None`. `concerns.rs` needs no change (:69, :147).
  - Item 5 stays private, as the brief decided.
- **`speed.rs` (items 10 and 11).**
  - Item 10 filters `ratio > 1.0` before the sort and truncate (:180-190).
  - Item 11 follows finding 6.
  - Rule (i) only documents current behaviour: `fleet[key]` holds every machine's runs, the machine's own included (:131-138).
- **Reconciler rollups (items 6-9).** These extend the existing `run_pass`, `rollup_hour` and `write_*` functions. There is no new module and no new settings key.
  - Job rollup, listed loop: `hour + HOUR_MILLIS + ROLLUP_GRACE_MILLIS <= now` (finding 1).
  - Job rollup, catch-up (:176): `next + HOUR_MILLIS + ROLLUP_GRACE_MILLIS <= now`.
  - Machine rollup: delete the listed loop and the recompute `DELETE` (:232-235, :267-273). Its catch-up (:287) becomes the same grace shape.
  - `fleet_health_passes` (`main.rs:629-643`) keeps its order and its log-and-continue handling.
- **No parallel path.** Phase 7 would block any of these:
  - a second scoring entry point;
  - a separate speed query;
  - a new rollup module or recompute table;
  - a second grace constant.

## Patterns referenced
- `docs/design/fleet-health.md`: the hourly rollups (:118-128), the speed index (:175-191), and scores (:193-222).
- `crates/gridops-core/src/fleet_health/score.rs`: the denominators (:481, :603, :618, :653), the zero guard (:495), `higher` (:23-29) and `component` (:78-110).
- `crates/gridops-reconciler/src/fleet_health_rollup.rs` and `fleet_host_rollup.rs`: the listed loop and catch-up, the final-attempt rule, and the online-minutes bitmask.
- `crates/gridops-core/src/fleet_health/speed.rs`: the fleet median and `grouped_durations`.
- `crates/gridops-reconciler/src/fleet_health_rollup_contract_tests.rs`, PR #237's `list_hours`, and #204's issue body: the fixed and adjacent contracts.
