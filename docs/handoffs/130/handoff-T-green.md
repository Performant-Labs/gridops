# Handoff-T-green: Phase 7 - Cap the time a fleet placement spends in `registering` (#130)

**Date:** 2026-11-22
**Branch:** claude/fix-130-registering-cap
**Issue:** #130
**Handoff-F reviewed:** docs/handoffs/130/handoff-F.md
**Handoff-T-red:** docs/handoffs/130/handoff-T-red.md

## GREEN confirmation
Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`.
```
cargo test -p gridops-reconciler   -> 128 passed; 0 failed
  registration_cap_tests::a_runner_github_never_registers_gives_up_its_placement_past_the_cap ... ok
  registration_cap_tests::a_placement_whose_machine_lost_authority_gives_up_past_the_cap ... ok
  registration_cap_tests::a_runner_registered_within_the_cap_starts_normally ... ok
cargo test -p gridops-core --lib   -> 311 passed; 0 failed
cargo test -p gridops-api --test control_plane_contracts -> 39 passed (after touching the test file, because the shared target dir can serve another worktree's binary, as F noted)
npx vitest run                     -> 54 files, 626 tests passed (both new TS tests pass)
```
**Spot-check (the tests fail when the behaviour is removed).** I made `expire_registering` return early unconditionally (`|| true`). Result: both RED tests (`..._gives_up_its_placement_past_the_cap`, `..._lost_authority_gives_up_past_the_cap`) FAILED, and the within-cap guard stayed green. I then reverted with `git checkout`, and re-ran: 3 passed. T's Phase-4 mutations (age from `created_at`, `is_start_failure == true`) were already recorded in handoff-T-red.

**Test change in this phase (tests only).** In `failure.rs` and `start_health.rs` I added `RegistrationTimedOut` to the two existing tests listed in handoff-T-red (`reason_codes_are_stable_and_round_trip`, with a pinned `"registration_timed_out"` literal, and `only_failures_the_machine_could_have_caused_count`). I deleted `a_registration_timeout_has_its_own_stable_code` and `a_registration_timeout_is_not_the_machines_fault`, which then duplicated them.

## Tier 1 results
| Check | Result |
|---|---|
| `cargo fmt --check` | PASS |
| `cargo clippy -p gridops-core -p gridops-reconciler -- -D warnings` | PASS |
| `cargo clippy ... --all-targets -- -D warnings` | 5 findings, all pre-existing on master in unchanged files (`autoscaling.rs:1890,:1976`, `autoscaling_gitlab_tests.rs:289,:385`, `builtin_host.rs:119`). None in a changed file. Same as F and T-red reported. |
| `npx eslint src/features/fleet tests/placement-failures.test.ts --max-warnings=0` | PASS |
| `npx tsc --noEmit` (fleet files) | Only 2 `routeTree.gen` errors in `fleet.index.tsx` and `fleet.machines.$hostId.tsx`, from the generated route tree absent in this worktree (unchanged by this diff). The RED type error in `placement-failures.test.ts` is gone. |
| `docs/openapi/gridops-v1.json` parses | PASS |
| API smoke | `control_plane_contracts` 39 passed. The change adds no endpoint. |

F's results reproduce, with one expected difference: the core count is 311, not 313, because of the duplicates I removed.

## Tier 2 results
- **Coverage:** PASS. Each criterion has a test (see below).
- **Test quality:** PASS. Each test names a behaviour. The three reconciler tests sit at the integration tier because they need the real pass loop, the database and capacity rows. The core and TS tests are unit/contract level. After my merge there are no duplicate tests. The suite is proportionate: 3 integration, 1 TS contract pair (one test is also the OpenAPI/TS drift guard), and assertions added to 2 existing unit tests.
- **Type safety:** PASS. No `any` or casts. The TS label map is `Record<Reason, string>`, so the compiler enforces the new label.
- **Error handling:** PASS. `expire_registering` errors are logged and do not abort the pass, so other placements still get `follow` and `finish`. The 409 and no-authority paths are tested.
- **Data integrity:** PASS. `abandon_unstarted` refuses once a `start_runner` exists (evidence entry 1), so a started runner is never given up. It releases allocations in one transaction. The second-pass behaviour (no repeat warn, no duplicate failure event) is covered by the exactly-one `placement_failed` assertion.
- **API contract:** PASS. The new code is in the Rust enum, `from_code`, the OpenAPI enum, the TS list, the TS label and `docs/fleet-api.md`. The TS test asserts that the OpenAPI enum equals `placementFailureReasons` as a set.
- **Security:** PASS. There are no new inputs. The error text is truncated to 240 characters by `PlacementFailure::new`.
- **Migration safety:** N/A. There is no schema change.
- **Playwright:** N/A. The change adds no UI surface, only one label in an existing list.
- **Evidence appendix:** `evidence.md` has 6 entries that match source. I checked the cited facts against the tests' use of them, and I add none.

## Acceptance criteria status
| Criterion | Result | Test |
|---|---|---|
| A `registering` placement whose JIT keeps failing is `failed` past the cap, allocations released, runner row leaves `starting`; before the cap, still `registering` with capacity held | PASS | `a_runner_github_never_registers_gives_up_its_placement_past_the_cap` |
| A placement whose registration succeeds within the cap starts normally and is not given up | PASS | `a_runner_registered_within_the_cap_starts_normally` |
| The cap does not depend on the attempt failing (no authority) | PASS | `a_placement_whose_machine_lost_authority_gives_up_past_the_cap` |
| Registration failure is not a start failure (no retry, no start-gate count) | PASS | the `RegistrationTimedOut` assertion in `only_failures_the_machine_could_have_caused_count`, and the `placement_retried` absence check in the first reconciler test |
| Failure code is stable and known on every contract surface | PASS | `reason_codes_are_stable_and_round_trip`, and the 2 TS tests in `placement-failures.test.ts` |
| Upstream registration cleanup uses the existing path | PASS (no new code) | evidence entries 4-6 |
| GitLab: cannot loop forever | PASS (by analysis) | F's D9 table: the rollback already fails the placement, so the cap is a no-op there |
| Sweep of every `register_and_start` exit | PASS | F's D9 table |
| CHANGELOG, docs | PASS | `CHANGELOG.md`, `docs/fleet-api.md` |

## Blocking issues
None.

## Advisory notes
- Five clippy findings on master in unchanged files fail `npm run check:rust` whatever this change does. They are not for F to fix in this story.
- The brief names `docs/fleet-automation.md` for the docs note. F put it in `docs/fleet-api.md`, the closest doc that describes `registering` and the failure reasons. Acceptable.
- No GitLab-specific reconciler test exists. That is by design (the cap is a no-op there).
- A's warn 6 (GitLab `fail_unstarted` records `runner_removed_before_start`) remains a follow-up.
