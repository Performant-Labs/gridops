# Handoff-A: Phase 3 - GitLab reconciler runner lifecycle (#139)  (up-front plan review, round 5)

**Date:** 2026-11-20
**Branch:** claude/gitlab-139-lifecycle-53c8ce (head a9e1f38 plus the uncommitted round-4 amendment; master b22c451)
**Brief reviewed:** docs/handoffs/139-brief.md (blob a67191d, which adds "Round-4 amendments (architecture gate R4)")   **Reuse map:** N/A. No `survey.md` was provided, so the brief's named analogues are the baseline: `next_bitbucket_connection`, the Bitbucket and GitHub arms of `provision_on_host` / `start_admitted_runner` / `delete_runner`, the GitHub fleet path in `fleet.rs`, `record_provision_failure` and `start_tests.rs`.   **Wireframe:** N/A (no UI surface)
**Verdict:** PASS

## Summary

PASS, with 0 block findings and 2 warnings.

The round-4 amendment resolves the round-4 block and all three round-4 warnings, and each fix uses an existing seam:
- `load_pool` shares the reconciler's only `Pool` query.
- The reload after the status loop follows the existing re-read at `main.rs:1028-1041`.
- The probe limit sits in the shared start loop, which serves both paths.
- The double delete is removed the way the Bitbucket arm avoids one.

Both warnings concern how far the round-4 fixes reach. The probe limit applies per pass, not per cool-down. The "load the pool at the moment of the count" rule needs a stated scope.

## Status of round-4 findings

| Round 4 | Status |
|---|---|
| Block 1 (mid-pass circuit writes on the manager path) | **Resolved.**<br>- **`load_pool`.** It goes next to `load_pools` (`main.rs:771`), the only `query_as::<_, Pool>` in the reconciler, and shares its SELECT. `fleet.rs` can import it, `record_provision_failure` and `clear_provision_failures` through `super::`, as it already imports `Pool` (`fleet.rs:36`).<br>- **Manager path.** `reconcile_pool` takes `pool: &Pool` (`:788`), so the reload after the status loop (`:794-826`) can rebind it. The fresh copy then drives `retry_deferred` and `provisioning_blocked` (`:867-870`), the start loop (`:947`) and `PassOutcome::for_pool` (`:1026`). A counted refusal therefore blocks starts, rotation, scale-up and rebalancing for the rest of that pass, and the pool ends `backoff` (or `blocked` at the threshold). A reset ends without `blocked`.<br>- **Fleet path.** I confirmed it needs no reload. All of its counts run inside `fleet::tick`: `register_and_start` (`fleet.rs:251-255`), `follow` (`:256-262`) and `finish` (`:264-268`). `reconcile()` runs `fleet::tick` at `main.rs:526`, before `load_pools` at `:566`. The fleet call in the start loop (`fleet::provision`, `fleet.rs:152-243`) only inserts the runner row and admits it; `create_runner` runs in a later tick. |
| Warning 2 (the probe should be one attempt) | **Resolved in form.** Every start, on both paths, goes through the one start loop at `:947`: `provision` hands fleet starts to `fleet::provision` (`:2320-2336`). One stop there covers both paths, as `HeldBack` does (`:969-972`). What is still open is how many starts there are per cool-down rather than per pass: warning 1 below. |
| Warning 3 (double deregistration on the fleet path) | **Resolved.** After a successful inline delete, the fleet rollback sets `gitlab_runner_id` to NULL, so `delete_runner`'s GitLab arm skips that runner. If the inline delete fails, the id stays and `delete_runner` tries again. On the manager path the GitLab arm writes `gitlab_runner_id` only after the POST, as `bitbucket_runner_uuid` is written (`:2687-2716`), so there was never a second delete there. `GitLabError::NotFound` exists (`gitlab/mod.rs:212-213`), and treating it as done matches `remove_manager_container`'s handling of a 404 (`:2950`). |
| Warning 4 (wording of "returns without error") | **Resolved.** `StartOutcome::Failed` is still counted and returned as `Err` (`:2502-2505`). `reconcile()` logs it per pool and moves on to the next pool (`:595-608`), as it does for Bitbucket. |

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | Round-4 W2: "While a pool's circuit is open (the half-open probe pass), the start loop ends after the first GitLab start it makes, on both paths … Done-when 10 adds: a probe pass on an open-circuit GitLab pool makes exactly one start." | cross-cutting (the circuit's half-open probe); pattern consistency with `tart_lane` | **The limit is one start per pass, not one per cool-down.**<br>- **Why.** Once the cool-down passes, the circuit is still open and `provision_retry_at` is in the past. A GitLab start that the manager or fleet accepts neither resets the circuit nor sets a new `provision_retry_at`. So each later pass makes one more probe start, until the first probe's refusal is counted and the circuit closes off the window again.<br>- **How often.** Passes run every 30 s by default. They also run early when a Tart start ends or the API asks for runners (`run_passes`, `:398-411`).<br>- **Manager path.** A Docker refusal is usually listed by the next pass, so the extra start is rare.<br>- **Fleet path.** The refusal arrives several ticks after the start: admission, then Prepare, then `register_and_start` in a later tick, then the Start result. So a fleet probe can still produce up to `desired - active` refused GitLab runners per cool-down. That is the round-4 cost, only spread over several passes.<br>- **The codebase's pattern.** For a start whose result arrives later, the codebase holds back other starts until it ends: `tart_lane` returns `HeldBack` while a Tart start runs (`:2421-2429`, `:969-972`). | Choose one and record it in the brief:<br>(a) **Hold back while the probe is in flight.** While a GitLab pool's circuit is open, start nothing if a GitLab runner started since the circuit last opened has neither proved nor been refused registration (still `starting` or `online`, not busy, not exited). Count that pass as held back. The cost: a recovered pool refills only after that probe runner goes busy or exits, which can take up to the 600 s idle timeout.<br>(b) **Accept the per-pass limit.** Record "a GitLab probe is one start per pass until its outcome is counted" as deliberate, and keep Done-when 10 as written. |
| 2 | warn | Round-4 block fix: "Every GitLab count (`record_provision_failure`) and reset (`clear_provision_failures`) is given a `Pool` from `load_pool` at that moment." | abstraction level (no platform branch in a shared call site); error boundary | **The rule has no stated scope.**<br>- **Shared call sites.** Read literally, it also covers the two existing counts in the start loop: the Tart lane's `Finished::Failed` (`:942`) and `StartOutcome::Failed` (`:2503`). That would add a GitLab-only `load_pool` branch to shared code. Neither site needs one. Each gets a fresh pool already: either the pool loaded at `:566` after `fleet::tick`, or the copy reloaded after the status loop. Each also ends the pass with `Err` right after counting.<br>- **The reset.** `clear_provision_failures(app, pool_id)` takes an id (round 3), so the "given a `Pool`" wording applies only to counts.<br>- **No pool.** What `load_pool` returning `None` means is not stated: the pool was deleted, or it is `deleting` if the shared SELECT keeps `load_pools`'s filter.<br>- **Side effect of the reload.** The reloaded `pool.state` is the state `record_provision_failure` wrote (`:3618`). At `:855`, if a refusal is counted while the built-in host is off, the "Cannot run: built-in runner host is off" event is written a second time in that pass. | Add one bullet to the round-4 block fix:<br>- `load_pool` is called only for counts and resets made outside the start loop: the manager status loop and `fleet::tick` (`register_and_start`, `follow`, `finish`).<br>- The start-loop sites `:942` and `:2503` keep passing the pass's `pool`, for every platform.<br>- If `load_pool` returns `None`, the count or reset is skipped. After the status loop, `None` ends that pool's pass with `Ok(())`.<br>- The `:855` check compares against the state from before the reload, or the one repeated event is accepted. |

## Notes for O

There is no block, so no amendment is required before T. The two warnings need a decision, each a one-line brief edit or a deliberate acceptance:
- **Warning 1.** Choose (a) hold back while the probe is in flight, or (b) a recorded per-pass probe.
- **Warning 2.** Add the scope bullet for `load_pool`.

Either can be folded into the brief before T writes the Done-when 10 tests, which is the cheapest point to do it.

## Patterns referenced

- `crates/gridops-reconciler/src/main.rs`:
  - `run_passes` (`:398-413`);
  - `reconcile` (`:522-609`: `fleet::tick` at `:526`, before `load_pools` at `:566`; the per-pool `Err` boundary at `:595-608`);
  - `load_pools` (`:771-784`, the only `Pool` query);
  - `reconcile_pool` (`:786-1057`: the status loop, `retry_deferred`/`provisioning_blocked`, the start loop and `HeldBack`, the re-read at `:1028-1041`);
  - `PassOutcome::for_pool`, `pool_state`, `finish_pool_pass` (`:1082-1143`);
  - `provision` (`:2286-2341`);
  - `provision_on_host`'s `tart_lane` claim and `StartOutcome::Failed` count (`:2421-2535`);
  - `start_admitted_runner`'s Bitbucket rollback and reset (`:2680-2781`);
  - `delete_runner` (`:2788-2897`);
  - `remove_manager_container` (`:2938-2958`);
  - `record_provision_failure`, `provision_failure_policy` (`:3608-3655`).
- `crates/gridops-reconciler/src/fleet.rs`: imports (`:15-36`), `provision` (`:152-243`), `tick` (`:246-274`), `register_and_start` (`:335-479`), `follow` (`:483-560`), `finish` (`:568-616`).
- `crates/gridops-core/src/gitlab/mod.rs` (`GitLabError`, `:203-219`).
- `crates/gridops-reconciler/src/start_tests.rs` (`a_successful_probe_clears_the_blocked_state_in_the_same_pass`, `:575`).
