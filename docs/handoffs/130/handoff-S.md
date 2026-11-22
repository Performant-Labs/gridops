# Handoff-S: Phase 8 - Cap the time a fleet placement spends in `registering` (#130)

**Date:** 2026-11-22
**Branch:** claude/fix-130-registering-cap
**Issue:** #130
**Brief:** docs/handoffs/130-brief.md
**Handoff-A reviewed:** docs/handoffs/130/handoff-A.md (plan, PASS) and docs/handoffs/130/handoff-A-dup.md (anti-duplication, PASS)
**Handoff-T reviewed:** docs/handoffs/130/handoff-T-red.md and docs/handoffs/130/handoff-T-green.md (PASS, zero blocking issues)
**Handoff-F reviewed:** docs/handoffs/130/handoff-F.md
**Operator-facing report:** N/A (no visual surface; see below)

## A precondition
Confirmed: A returned PASS on the plan (0 block, 6 warn, settled by D1-D10) and PASS on the anti-duplication gate (0 block).

## T precondition
Confirmed: T-red validated RED, T-green reported GREEN with zero blocking issues.

## Visual-diff-tool precondition
N/A. The change adds no UI surface. The only front-end edits are one entry in a closed list (`placementFailureReasons`) and one label in an existing `Record<Reason, string>`. There is nothing to render or diff, so the browser and pixel-diff preconditions do not apply, and no HTML report is produced.

## Preview / spec sanity check
No defect in the source of truth. The brief delegates the mechanism (time cap vs attempt count, age source) to the architect and asks for the whole class to be swept, which A did. The new failure code is a wire-contract change, and A caught and bound it (D4, D5). No ADVISORY-HOLD.

## What I verified myself (not only from the handoffs)
- **Age source (D2).** I grepped every `UPDATE workload_placements` writer in `crates/` and `migrations/`. The only writer that sets `registering` is `apply_to_placement` (`registry/commands.rs:408`), which sets `updated_at=now`. Every other writer either moves the placement out of `registering` (`mark_starting`, `abandon_unstarted`, `fail_placement`, `set_state`, the two `uncertain` writes, the two `running` writes) or is `release()` (`commands.rs:503`), which runs only when the placement is being released. So `updated_at` is a reliable entry time, and no migration is needed.
- **Contract surfaces (D5).** I grepped the repo for `operator_released` (a sibling reason) to enumerate every place a reason is listed: `failure.rs`, `start_health.rs`, `docs/openapi/gridops-v1.json`, `src/features/fleet/query-contracts.ts`, `src/features/fleet/placement-failures.ts`, `docs/fleet-api.md`, plus `unconfirmed.rs` (a use site) and `recovery.rs` (a test of that reason). All six list surfaces carry `registration_timed_out`. None was missed.
- **No secret reaches the recorded message.** The last error is recorded as `{error:#}`. The error sources on the new path are `FleetError::Invariant` labels, SQL errors, "GitHub API request failed ({status}): {body}" (the body is capped at 500 characters, `github.rs:319-322`) and context strings. GitLab `fleet_material` `?` errors are likewise context text and database errors. The JIT config and registration token are in `material`, which is never formatted into an error. `PlacementFailure::new` truncates the message to 240 characters.
- **Independent re-run** (`CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, shared target dir; I touched `failure.rs`, `start_health.rs` and `fleet.rs` first, because my first compile used a stale core artifact from a sibling worktree, as F reported):
  - `cargo test -p gridops-reconciler` -> 128 passed, 0 failed (includes the 3 `registration_cap_tests`);
  - `cargo test -p gridops-core --lib -- fleet::failure fleet::start_health` -> 9 passed;
  - `npx vitest run tests/placement-failures.test.ts` -> 9 passed.
  These match T-green (the core total is 311 there because T merged two duplicate tests).

## Spec compliance (brief item by item)

| Requirement | Compliant | Evidence |
|---|---|---|
| 1. Bound the time in `registering`; give up with `placement::abandon_unstarted` (refuses once a `start_runner` exists) | YES | `expire_registering` in `fleet.rs` calls `abandon_unstarted`. The give-up is a no-op for a started placement (evidence entry 1). |
| 1. Record a `PlacementFailure` naming the cause with the last error, truncated | YES | New reason `registration_timed_out`, constructor `PlacementFailure::registration_timed_out(elapsed_ms, allowed_ms, last_error)`. The message carries `{error:#}`, cut to 240 characters. The test asserts the `409` shows and the length is at most 240. |
| 1. Time cap in the style of `QUEUE_TIMEOUT_MILLIS`; justify how age is measured; confirm what `updated_at` means in `registering`; prefer no migration | YES | `REGISTRATION_TIMEOUT_MILLIS = 10 min` beside `QUEUE_TIMEOUT_MILLIS`, age from `placement.updated_at`, justified in A's D2 and re-verified above. No migration. The constant's comment states the invariant. The within-cap test pins "not `created_at`" by admitting the placement an hour ago. |
| 1. Each failure keeps logging until the cap; log once at warn on give-up | YES | The `could not start fleet runner` warn is unchanged. The give-up warn is logged only when `abandon_unstarted` returns `true`. A later pass finds the placement `failed` and not in the `registering` list. |
| 2. After the give-up, `finish` marks the runner row `failed` and the pool replaces it | YES | The RED test asserts the runner row is `failed` or `deleted` after one pass, and the capacity is `released`. |
| 2. Decide and test whether a registration failure counts as a start failure | YES | D6: `is_start_failure(RegistrationTimedOut, _) == false`. Pinned in `only_failures_the_machine_could_have_caused_count`, and the RED tests assert there is no `placement_retried` event. Reason: a GitHub problem across the org must not set healthy machines aside or loop retries. |
| 2. Upstream registration cleanup uses the existing path; verify it exists; add none | YES | Evidence entries 4-6: `finish` -> pool pass -> `delete_runner`, which removes by `github_runner_id` or by name. No new code. |
| 3. GitLab cannot leave a placement in `registering` forever, or the cap covers it | YES | A found that it can (the shared prefix, `?` errors in `fleet_material` and `fleet_rollback`). The cap lives in `tick`'s loop, so it covers every platform with no GitLab-specific code. After the give-up the circuit counts once via `finish`. |
| 4. Sweep every error path out of `register_and_start` | YES | The D9 table is in handoff-A and is copied into handoff-F. I cross-checked each exit against `register_and_start` and `start_with_material` in the worktree. 13 exits, none missed. |
| Test: a `registering` placement whose JIT keeps failing is `failed` past the cap, allocation `released`, runner row leaves `starting`; before the cap still `registering` with capacity held | YES | `a_runner_github_never_registers_gives_up_its_placement_past_the_cap` (fake GitHub 409 with an online same-named runner). |
| Test: a registration that succeeds later, within the cap, starts normally and is not given up | YES | `a_runner_registered_within_the_cap_starts_normally`. It also checks one `start_runner`, no failure event, and `running` keeps its capacity. |
| Follow existing reconciler test patterns; fakes over axum; fleet fixtures | YES | Fixtures were generalised, not copied (`Fleet::on_machine`, `Fleet::github`, configurable fake GitHub in `Setup`). |
| Out of scope respected: no change to `generate_jit_config`'s 409 handling, no deploy, no prod data | YES | `github.rs` is untouched. |
| Docs: CHANGELOG entry; short note in `docs/fleet-automation.md` or the closest fleet doc | YES | CHANGELOG under Unreleased -> Fixed, citing #130. The note is in `docs/fleet-api.md`, the doc that holds the `placement_failed` reason list and the retry paragraph. `fleet-automation.md` has no placement-failure text. |

## Quality audit
| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | One additive enum value on `PlacementFailureGroup.reason`, in the OpenAPI enum, the Rust set and the TS list. The TS test asserts the OpenAPI enum equals `placementFailureReasons` as a set, so the surfaces cannot drift apart silently. |
| Error handling | PASS | An error from `expire_registering` is logged and does not abort the pass, so `follow` and `finish` still run for other placements. `abandon_unstarted` is transactional (`BEGIN IMMEDIATE`). |
| UI/UX match to spec | N/A | One label, "Registration timed out", in an existing map. |
| Accessibility | N/A | No UI change. |
| Architecture gate | PASS | A PASS on plan and on duplication. No finding in this audit conflicts with A. |
| Code organization | PASS | Constant and helper sit beside `QUEUE_TIMEOUT_MILLIS` and `expire_queued`. No dead code, no TODOs, no new dependency. |
| Security | PASS | No new input. No secret reaches the recorded message (see above). |
| Performance | PASS | One age comparison per `registering` placement per pass. `abandon_unstarted` runs only past the cap. The `(state, updated_at)` index exists. |
| Visual regression | N/A | No visual surface. |
| Naming consistency | PASS | `registration_timed_out` is deliberately not named `*_deadline_expired`, because the `*_deadline_expired` codes count as machine faults and this one does not (A's choice, D6). It is snake_case like every other code, and its facts use the existing `elapsed_ms` and `allowed_ms` keys. |
| Test quality (`testing/test-quality.md` section 7) | PASS | See below. |

### Test-quality audit (section 7 rubric)
- **`a_runner_github_never_registers_gives_up_its_placement_past_the_cap`:** names one behaviour (give-up past the cap releases capacity and ends the runner row). It fails for the right reason (T-green's mutation that makes `expire_registering` return early turned it red). Integration tier is the cheapest that can show capacity release and the runner row, which need the real pass loop and the database. It asserts observable state, not implementation.
- **`a_runner_registered_within_the_cap_starts_normally`:** it is green on master by design, because nothing gave up a placement before. Section 7 asks whether it can fail for the right reason. T-red showed it does: it goes red under age-from-`created_at` (the D2 mistake) and under a cap that fires early. The brief requires it. Its overlap with the first test's before-cap check is the "held" assertion only. Its own signal is the later success, the single `start_runner`, and `running` keeping capacity. Keep.
- **`a_placement_whose_machine_lost_authority_gives_up_past_the_cap`:** a distinct branch (the `Ok(())` exit, no attempt made). It is the only test of D3's "whatever the attempt returned" and went red under the same mutation. Keep.
- **Core and TS:** assertions were added to two existing tests (`reason_codes_are_stable_and_round_trip`, `only_failures_the_machine_could_have_caused_count`), and the two by-code tests that duplicated them were deleted. The TS side adds 2 tests, one of which is the OpenAPI/TS drift guard. The suite is proportionate: 3 integration tests for 3 behaviours, with the rest at the cheapest tier.
- **Smells:** none. No tautology, no unreachable outcome, no snapshot padding, no mock-shaped assertion. `assert_still_held` and `assert_given_up` are shared helpers, not copies.
- **No "delete or merge" findings.**

## Scope check
Delivered exactly the brief's scope. Over-delivery is limited to what the brief's own "sweep the class" instruction asks for: the cap fires whatever the attempt returned (needed for the no-authority exit), plus the third test for that exit. The 13 changed files (8 source/docs, 5 test) exceed the role's usual ~6-file cap, but A's D5 requires the contract surfaces to land together (a server code the UI's strict zod parse rejects would break the machine page), and F recorded this. There is no under-delivery.

## Verdict

PASS: all acceptance criteria are met, the implementation is spec-compliant, and quality is acceptable. Ready for O to commit.

## Advisory notes
1. **Docs wrapping (cosmetic).** In `docs/fleet-api.md` (about line 616) the reflowed sentence "`recentPlacementFailures` (the last 24 hours, grouped by reason). Only failures after the release that added this are" is about 119 characters, while the surrounding paragraph wraps at about 80. It renders identically. Re-wrapping is optional.
2. **Restart edge case (accepted by design).** If the reconciler is down for more than the cap while a placement is `registering`, the first attempt after restart that fails transiently will give that placement up at once, with no further retry. The cost is one replaced runner. Time-based expiry like `expire_queued` has the same shape, so I do not call it a defect.
3. **Upstream cleanup has no reconciler test of its own.** The brief only asks to verify that the path exists, and evidence entries 4-6 do. After a give-up in the 409 test, the by-name `delete_runner` fallback would remove the online same-named runner the fake lists, because the name is unique to the row (A recorded this as existing behaviour and hedged it). If the operator wants it pinned, the cheapest place is the first reconciler test, asserting `github_removals` after the pool pass.
4. **No GitLab reconciler test.** By design: the cap is a no-op where the GitLab rollback already fails the placement, and the `?`-error paths go through the same platform-agnostic `tick` code the GitHub tests exercise.
5. **Pre-existing, not this story.** Five clippy findings on master in unchanged files (`autoscaling.rs:1890`, `:1976`, `autoscaling_gitlab_tests.rs:289`, `:385`, `builtin_host.rs:119`) fail `npm run check:rust` whatever this change does. T flagged them for O.
6. **Follow-up candidate (A's warn 6).** GitLab's `fail_unstarted` records every GitLab registration failure as `runner_removed_before_start`, which misnames the cause on the machine page. It is out of scope here.
