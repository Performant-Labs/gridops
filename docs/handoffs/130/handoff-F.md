# Handoff-F: Phase 5 - Cap the time a fleet placement spends in `registering` (#130)

**Date:** 2026-11-22
**Branch:** claude/fix-130-registering-cap
**Issue:** #130

## What was done
- `crates/gridops-reconciler/src/fleet.rs`: added `REGISTRATION_TIMEOUT_MILLIS` (10 min) next to `QUEUE_TIMEOUT_MILLIS`, with a comment stating the `updated_at` invariant. Added `expire_registering`, next to `expire_queued`. `tick`'s `registering` loop now calls it after every `register_and_start` attempt, whatever the attempt returned (D3). The existing `could not start fleet runner` warn on `Err` is unchanged.
- `crates/gridops-core/src/fleet/failure.rs`: added the variant `PlacementFailureReason::RegistrationTimedOut` (code `registration_timed_out`) to the enum, `as_str` and the `from_code` list. Added the named constructor `PlacementFailure::registration_timed_out(elapsed_ms, allowed_ms, last_error)` (D4).
- `crates/gridops-core/src/fleet/start_health.rs`: `is_start_failure(RegistrationTimedOut, _)` is `false`, grouped with `RunnerRemovedBeforeStart` (D6).
- `docs/openapi/gridops-v1.json`: `registration_timed_out` added to the `PlacementFailureGroup.reason` enum (D5).
- `src/features/fleet/query-contracts.ts`: `registration_timed_out` added to `placementFailureReasons` (D5).
- `src/features/fleet/placement-failures.ts`: added the label "Registration timed out" (D5).
- `docs/fleet-api.md`: the reason list, a new paragraph on the 10-minute registration cap, and the not-retried list (D10).
- `CHANGELOG.md`: an entry under Unreleased → Fixed, citing #130 (D10).

## Design decisions
- **`expire_registering` takes the attempt's error.** `register_and_start`'s result is kept as `attempt`. Its error is passed by reference, so the existing warn still logs it with `?error`. The helper formats the error with `{error:#}`, so any `.context(...)` chain is kept in the recorded message (for example, "GitHub App credentials are unavailable" or "GitHub API request failed (409 Conflict): …"). `PlacementFailure::new` then truncates the message to 240 characters.
- **The age is taken from the `Placement` the loop already loaded** (`placement.updated_at`, D2). It is not re-read after the attempt. If this pass's attempt started the runner, `abandon_unstarted` refuses because a `start_runner` row now exists, so reading the age before the attempt is safe.
- **A give-up error is logged, not propagated.** An error from `expire_registering` is logged at warn (`could not give up fleet runner`) and the loop continues. This matches how the surrounding per-placement loops treat errors. The alternative was `?`, as `expire_queued` does. I rejected it because a DB error on one placement would then abort the whole pass, including `follow` and `finish` for every other placement.
- **The message wording.** With an error: "The runner could not be registered and started within 10 minutes: {error}". With no attempt (no authority): "…within 10 minutes; no start could be issued." No separate `last_error` fact is added: the message already carries the error, truncated, and `with` would not truncate a fact. The facts are `elapsed_ms` and `allowed_ms`, the same keys as `with_timing`.
- **The give-up warn is logged once.** It is logged only when `abandon_unstarted` returns `true`, which needs at least one allocation released, so later passes stay silent.

## Reuse / extend-vs-new
EXTEND, per the brief and D1-D10:
- the give-up uses the existing `placement::abandon_unstarted`;
- the cause is a new variant in the existing closed set, with a constructor beside `runner_removed_before_start` and `operator_released`;
- the cap is a reconciler policy constant beside `QUEUE_TIMEOUT_MILLIS`, and the helper sits beside `expire_queued`;
- replacement and upstream cleanup use the existing `finish` → pool pass → `delete_runner` path (D7).

There is no new module, no new core function, no SQL sweep, no migration, no GitLab-specific code (D8), and no new cleanup path.

## Architecture notes for A
- **Layers:** the reconciler (`fleet.rs` policy and loop) and core (`failure.rs` closed set, `start_health.rs`). No new dependencies.
- **Contract change:** a new failure reason code, `registration_timed_out`, on the wire. It has landed on all of D5's surfaces in this change:
  - Rust enum, `as_str` and `from_code`;
  - the `start_health` arm;
  - the OpenAPI enum;
  - the TS `placementFailureReasons`;
  - the TS label;
  - `docs/fleet-api.md`.
- An older server reading the new code maps it to `Unknown` (the existing `from_code` fallback).

### D9 sweep: every exit from `register_and_start`, with the cap (copied from handoff-A, now implemented)

| Exit (fleet.rs) | Before | With the cap |
|---|---|---|
| `host_has_authority` false, `Ok` | stays `registering` (forever after an epoch bump) | ended (pinned by `a_placement_whose_machine_lost_authority_gives_up_past_the_cap`) |
| `host_has_authority` / `prepared_environment` / `prepared_with` DB error, or env / prepare missing | loops | ended |
| runner-row query error, or row missing | loops | ended |
| runner not `starting` | already abandoned (`RunnerRemovedBeforeStart`) | unchanged |
| GitLab: `fleet_material` registration error, or `start_with_material` error handled by `fleet_rollback` | already abandoned by `fail_unstarted` | unchanged; the cap is a no-op |
| GitLab: `?` error from `fleet_material` or `fleet_rollback` | loops | ended |
| credentials unavailable | loops | ended |
| `generate_jit_config` error, including a 409 for a live runner | loops | ended (pinned by `a_runner_github_never_registers_gives_up_its_placement_past_the_cap`) |
| `github_runner_id` write error | loops; the upstream registration exists | ended; `delete_runner` removes it by name (D7) |
| `generate_registration_token` or repository query error | loops | ended |
| `store_bootstrap` / `issue_command` error | loops | ended |
| `issue_command` returns `None`: authority lapsed | stays `registering` | ended |
| `issue_command` returns `None`: key already exists, or `mark_starting` error after the start was issued | the command-result path resolves it | `abandon_unstarted` refuses (a start exists), which is correct |

## Deviations from spec / wireframe
None.

On the role's scope cap: this change touches 8 files across Rust, TS and docs. A's D5 binds all six contract surfaces to land in one PR, because a split would ship a server code the UI's strict zod parse rejects. Every edit outside `fleet.rs` and `failure.rs` is a one-line list entry or doc text. So I did not propose a split.

## Tier 1 self-check (incl. tests now GREEN)
Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` and the shared target dir.
```
$ cargo fmt --check                                   -> ok
$ cargo test -p gridops-core --lib
test result: ok. 313 passed; 0 failed
$ cargo test -p gridops-reconciler
test registration_cap_tests::a_placement_whose_machine_lost_authority_gives_up_past_the_cap ... ok
test registration_cap_tests::a_runner_registered_within_the_cap_starts_normally ... ok
test registration_cap_tests::a_runner_github_never_registers_gives_up_its_placement_past_the_cap ... ok
test result: ok. 128 passed; 0 failed
$ cargo test -p gridops-api --test control_plane_contracts
test result: ok. 39 passed; 0 failed
$ cargo clippy -p gridops-core -p gridops-reconciler -- -D warnings      -> Finished, no findings
$ cargo clippy -p gridops-core -p gridops-reconciler --all-targets -- -D warnings
  only the 5 pre-existing findings T flagged (autoscaling.rs:1890,:1976;
  autoscaling_gitlab_tests.rs:289,:385; builtin_host.rs:119); none in a changed file
$ npx vitest run
Test Files  54 passed (54)   Tests  626 passed (626)
$ npx eslint src/features/fleet tests/placement-failures.test.ts --max-warnings=0   -> ok
$ npx tsc --noEmit   -> no errors in src/features/fleet or tests/placement-failures.test.ts
  (T's RED type error at placement-failures.test.ts:43 is gone). The remaining errors
  all come from the missing generated src/routeTree.gen in this fresh worktree, as T noted.
$ python3 -c "import json; json.load(open('docs/openapi/gridops-v1.json'))"   -> valid
```
- **Core tests:** both of T's new tests now pass (`a_registration_timeout_has_its_own_stable_code` and `a_registration_timeout_is_not_the_machines_fault`).
- **TS tests:** both new tests now pass, and the 2 failures that were RED are gone.
- **Environment note:** the first `control_plane_contracts` run reported 2 failures in tests that exist only in the `gridops-124` worktree. The shared cargo target dir had served that worktree's binary. After touching the test file to force a rebuild from this worktree, all 39 pass. The registration-cap tests exist only in this worktree, so the reconciler run above did use this worktree's source.

## Evidence appendix
`docs/handoffs/130/evidence.md` has 6 entries:
- the `abandon_unstarted` start guard;
- the `abandon_unstarted` return value, which makes the give-up log a one-time line;
- the entry into `registering` setting `updated_at`;
- `finish` failing the runner row;
- the pool pass calling `delete_runner`;
- `delete_runner`'s removal by id or by name.

## Tests that look wrong (for T)
None.

As T's handoff says, in Phase 6 T adds `RegistrationTimedOut` to `reason_codes_are_stable_and_round_trip` (failure.rs) and to `only_failures_the_machine_could_have_caused_count` (start_health.rs). F did not touch either.

## Known issues
- Five clippy findings on master, in files this branch doesn't touch, fail `npm run check:rust` whatever this change does. T already flagged them for O.
- A's warn 6 (GitLab `fail_unstarted` records `runner_removed_before_start`) is a follow-up only and was not addressed here.

## Files changed
- crates/gridops-reconciler/src/fleet.rs
- crates/gridops-core/src/fleet/failure.rs
- crates/gridops-core/src/fleet/start_health.rs
- docs/openapi/gridops-v1.json
- src/features/fleet/query-contracts.ts
- src/features/fleet/placement-failures.ts
- docs/fleet-api.md
- CHANGELOG.md
