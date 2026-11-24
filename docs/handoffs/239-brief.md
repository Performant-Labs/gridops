# [#239](https://github.com/Performant-Labs/gridops/issues/239)

Part of #198. Found by the Opus review of the wave-1 PRs (#229, #230, #236, #235). The epic called for this review; it ran after they merged. **Must land before #204**: #204's independent Python oracle computes from the design doc and would disagree with the current code.

All references are to master at 73de941.

## Scoring (`crates/gridops-core/src/fleet_health/score.rs`, from #192 / PR #236)

1. **Denominators assume a full window.** `fleet_online_share` (~:481), the machine availability (~:649) and busy share (~:653) divide by `window.millis()`, but the rows cover only whole hours from `ceil_hour(now − W)` to `now` (plus #204's in-progress hour).
   - At 10:30, a perfect machine on 24h reads 97.9% online. On **1h** it reads 50%: availability scores 0, and a critical `fleet-component:availability` concern fires on every load that isn't exactly on the hour.
   - Fix: use the covered time, `now − ceil_hour(now − W)`, for every time denominator.
   - Test with a `now` that isn't on the hour. Every test today uses an on-the-hour `FIXTURE_NOW`.
   - **The grace-hour contract with #204.** #204's source computes live, bounded, every hour of `[ceil_hour(now − W), now)` the rollups haven't written: the in-progress hour, and the hour that just ended until `hour_end + 15 min`. #204 detects an unwritten hour **by time**, using the one shared grace constant from items 7 and 9. It does not use the rollups' markers. The scorer's covered time is `now − ceil_hour(now − W)` on that basis.
   - Test: a perfect machine on 24h at `now = H + 10 min`, with the ended hour supplied as the source would supply it, scores availability 100.
2. **Infra-failure share divides by successes.** ~:581/:590 divide by `completed_count`, which counts successes only. The design defines the denominator as every completed final-attempt job (`job_count`).
   - In an outage where nothing succeeds, the component goes to "not enough data" and its concern can't fire.
   - Fix: use `job_count` and update the pinned test. This reverses PR-Agent's "fix" e71dcf6.
3. **A machine offline all window is hidden.** ~:653: `(online_minutes > 0).then(...)` turns 0 online minutes into "not enough data".
   - #200 writes rows with `online_minutes = 0` on purpose. Only *no rows at all* is missing data, and 0 must score 0.
4. **A machine with a CPU budget of 0 gets a score.** The design says it has none. It must get no machine score and raise no `host-score` concerns. That covers MacBookPro. Adjust #197's fixture wording if needed.
5. **Component scores are rounded before combining.** :103 and :435 feed rounded `u8`s into `combine`. The design rounds only the final score. Keep `f64` until the end.
   - **Decided:** keep the unrounded value private to `score.rs`, for example a private `measure()` returning `(ScoreComponent, Option<f64>)` and a raw combine that `combine()` calls.
   - `ScoreComponent`, the wire format and the signatures of `combine(&[ScoreComponent])` and the public component functions don't change.
   - The busy-share gate and the concern bands keep reading the rounded component score.

## Job rollup (`crates/gridops-reconciler/src/fleet_health_rollup.rs`, from #191 / PR #229)

6. **A re-run counts twice when its attempts fall in different hours.** Attempt 1 completes in hour 10. The re-run (`run_attempt` 2) completes in hour 11. Hour 10 is never re-rolled, so both count.
   - Fix: while rolling hour H, for each run in it with `run_attempt > 1`, find its earlier attempts' `completed_at` hours through `workflow_jobs_run_status_idx`. List those in `fleet_health_recompute` inside the same write transaction.
   - Test: attempts in different hours.
7. **Late arrivals are lost.** `completed_at` is the platform's own timestamp. A job finishing at 10:59:40 whose webhook was missed is recorded by the poll at about 11:00:40, after hour 10 was rolled at about 11:00:10.
   - Fix: roll a catch-up hour only once `hour_end + 15 min <= now`, so each hour is still rolled once.
   - Test: a row inserted after the hour ends but inside the grace period is counted.

## Machine rollup (`crates/gridops-reconciler/src/fleet_host_rollup.rs`, from #200 / PR #230)

8. **It consumes the recompute queue meant for the job rollup.** It deletes every listed hour (~:267-273), so if the job rollup's pass errored first (log-and-continue), the hours #190 listed are lost. It also rolls and clears listed hours that haven't ended yet.
   - **Decided:** the machine rollup stops reading and deleting `fleet_health_recompute`, because its rows don't depend on categories. There's no consumer column and no new migration. It still skips hours that haven't ended.
   - This changes #197's contract test `a_pass_rolls_up_a_listed_hour_and_takes_it_off_the_list`. That change is in scope here.
9. **Late samples are lost.** Same as item 7: the catch-up at ~:287 rolls an hour as soon as it ends. Apply the same 15-minute grace.

## Speed index (`crates/gridops-core/src/fleet_health/speed.rs`, from #193 / PR #235)

10. **`topKeys` can list keys with a ratio of 1.0 or less.** ~:180-190. These are shown as "pushing it up". Filter `ratio > 1.0`.
11. **Duplicate runner rows can double-count a job.** ~:220-221 joins every `runners` row with the same `github_runner_id` (the unique index was dropped in 0013). Use the newest-row subqueries #191 uses.

## Done when

- [ ] Each item has a test that fails before the fix.
- [ ] A mutation check is shown in the handoff for items 1, 2, 3 and 6.
- [ ] `docs/design/fleet-health.md` states every rule this issue changes, because #204's oracle is written from it:
  - (a) the covered-time denominators for availability (fleet and machine) and busy share, plus item 1's grace-hour contract;
  - (b) infra failures `÷ job_count`;
  - (c) only *no rows* is missing machine data, 0 online minutes scores 0, and a CPU budget of 0 gives no score and no `host-score` concern;
  - (d) the job rollup lists an earlier attempt's hours in `fleet_health_recompute`;
  - (e) the 15-minute grace before a catch-up hour is rolled;
  - (f) only the job rollup reads `fleet_health_recompute`;
  - (g) `topKeys` hold only keys with a ratio above 1.0;
  - (h) the busy-share gate and the concern bands read the rounded component score;
  - (i) the fleet median includes the machine's own runs, so a machine running most of a key's jobs drifts toward 1.0.
- [ ] SQLite discipline holds: no write transaction over 50 ms, batches of at most 500 rows.

## Constraints

- Keep `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`. Use the shared cargo target from the untracked `.cargo/config.toml`, and never set `CARGO_TARGET_DIR`. Prefer `-p` crate test runs.
- Don't edit `migrations/0050_fleet_health.sql`: its checksum is applied.
- In the PR description, put no closing keyword before a wave-1 issue number, or the Fleet health guard fails the PR.
- Work headless: no visible browsers and no screenshots on the desktop.
- Open the PR against master on Performant-Labs/gridops, never upstream iamngoni/gridops.
