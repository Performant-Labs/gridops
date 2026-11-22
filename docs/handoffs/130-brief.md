# [fleet: a runner whose JIT registration was not recorded retries forever on 409 and holds its capacity](https://github.com/Performant-Labs/gridops/issues/130)

## What is already fixed (do not redo)

PR #155 (c575130) made `GitHubClient::generate_jit_config` (`crates/gridops-core/src/github.rs`) handle the 409: it looks the name up, removes an offline, idle same-named registration and asks again once. It never removes an online or busy runner. That works whether or not `runners.github_runner_id` was recorded, because the lookup is by name. Tests: `crates/gridops-reconciler/src/jit_conflict_tests.rs` (`an_offline_leftover_registration_is_replaced`, `a_live_runner_under_the_name_is_never_removed`). On prod (Mars), there were 0 409 warnings after deploy, and 0 placements are in `registering` today (checked 2026-11-22).

## What is still missing (this story)

The issue also requires: "a placement that fails the same way N times should release its capacity, not hold it indefinitely." #155 left this out on purpose ("Not included: a time cap on `registering`").

On master, `tick` in `crates/gridops-reconciler/src/fleet.rs` (~line 272) calls `register_and_start` for every `registering` placement. On an error it only logs `could not start fleet runner` and tries again next pass, about every 30 s, forever. Meanwhile the placement's `capacity_allocations` row stays unreleased and the runner row stays `starting`, which counts toward pool capacity. Errors that still loop this way include:
- a 409 whose same-named runner is online or busy;
- any other GitHub error from `generate_jit_config` or `generate_registration_token`;
- GitHub App credentials unavailable;
- the prepared environment or prepare command missing;
- a failure in `start_with_material`.

## What to build

1. **Bound the time in `registering`.** When `register_and_start` fails for a placement that has been in `registering` longer than a cap, end the placement and release its capacity.
   - Use the existing `placement::abandon_unstarted` (`crates/gridops-core/src/fleet/placement.rs`). It refuses when a `start_runner` command was issued, which is the right safety. Record a `PlacementFailure` that names the cause and carries the last error message, truncated.
   - The architect picks a time cap or an attempt count, and must justify how the age is measured. Confirm what `workload_placements.updated_at` means while a placement sits in `registering`: does anything bump it during retries? If it isn't a reliable entry time, use something that is. Prefer no migration. A time cap in the style of `QUEUE_TIMEOUT_MILLIS` (10 min) is the expected shape.
   - Each failure keeps logging as today until the cap. Log once, at warn, when the placement is given up.
2. **After it is given up, the normal path takes over.**
   - The existing `finish` pass marks the runner row `failed`, so the pool can replace it, and may retry the start on another machine (`retry::retry_failed_start`).
   - Decide and test whether a registration failure should count as a start failure for retry and start-health purposes (`start_health::is_start_failure`). Avoid a tight replace loop against a GitHub problem affecting the whole org.
   - Any upstream GitHub registration the runner row did record (`github_runner_id`) must still be cleaned up by the existing failed-runner cleanup. Verify that this path exists, and do not add a new one if it does.
3. **GitLab.** `register_and_start`'s GitLab branch already rolls back and counts failures through `gitlab_lifecycle::fleet_rollback`. Confirm that it cannot leave a placement in `registering` forever. If it can, cover it with the same cap. If it can't, say so in the plan with the code path.
4. **Sweep the class.** List every error path out of `register_and_start` in the plan. For each one, say whether the cap now ends it.

## Tests (required)

- The issue's "Done when" is already met at the client layer by #155. Add one reconciler-level test: a `registering` placement whose JIT request keeps failing (for example, a fake GitHub that always answers 409 with an online same-named runner) is given up once past the cap. Assert:
  - the placement ends `failed`;
  - its capacity allocation is `released`;
  - the runner row leaves `starting`.

  Before the cap, it is still `registering` with its capacity held.
- Add a test that a placement whose registration succeeds on a later pass, still within the cap, starts normally and is not given up.
- Follow the existing reconciler test patterns (`*_tests.rs` modules wired from `main.rs`, fake GitHub over axum as in `jit_conflict_tests.rs`, fleet fixtures as in the existing fleet tests).

## Out of scope

Changing the 409 handling in `generate_jit_config`. Deploying. Prod data.

## Docs

Add a CHANGELOG entry. Add a short note to `docs/fleet-automation.md` (or the closest fleet doc), saying a runner that cannot register within the cap gives up its placement and is replaced.
