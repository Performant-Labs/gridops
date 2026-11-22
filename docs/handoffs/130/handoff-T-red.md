# Handoff-T-red: Phase 4 - Cap the time a fleet placement spends in `registering` (#130)

**Date:** 2026-11-22
**Branch:** claude/fix-130-registering-cap
**Brief / wireframe reviewed:** `docs/handoffs/130-brief.md`, `docs/handoffs/130/handoff-A.md` (D1-D10). There is no wireframe, because no UI surface is added; the one closed-set label is pinned below.

## A precondition
Confirmed: A returned **PASS** on the plan (Phase 3), with 0 block and 6 warn findings, settled by D1-D10.

## Tests authored

Every file I touched is test code. `main.rs` only gains a `#[cfg(test)] mod` line. No production code changed (`git diff -- crates/gridops-reconciler/src/fleet.rs` is empty, and the `failure.rs` and `start_health.rs` hunks sit inside `#[cfg(test)] mod tests`).

### Reconciler integration tests: `crates/gridops-reconciler/src/registration_cap_tests.rs` (new, wired from `main.rs`)

These are at the integration tier because the behaviour is the pass loop: `tick`, then `finish`, then the pool pass. They run against the real database and registry and against `Setup`'s fake GitHub over axum. No lower tier can show capacity being released and the runner row leaving `starting`.

| Test | Pins | Notes |
|---|---|---|
| `a_runner_github_never_registers_gives_up_its_placement_past_the_cap` | The brief's required test, D3 and D4. GitHub answers every JIT with 409 while an online runner holds the name. **Before the cap**, after 2 refused passes (`asked >= 2`), the placement is still `registering`, its allocations are not released, the runner row is still `starting`, and no `placement_failed` event exists. **Past the cap** (`updated_at` backdated by cap + 1 min), one pass leaves the placement `failed`, every allocation `released`, the runner row `failed` or `deleted`, and exactly one `placement_failed` event with `reason=registration_timed_out`, `allowed_ms=600000` and `elapsed_ms >= 600000`. The message contains `409` (the last error) and is at most 240 characters. There is **no `placement_retried` event** (D6). | RED |
| `a_runner_registered_within_the_cap_starts_normally` | The brief's second required test, plus D2. The placement is admitted an hour ago (`created_at` backdated) but has been `registering` for 9 minutes (`updated_at`). It is refused once more at 9 minutes and is still held. Then GitHub accepts, and the pass issues exactly one `start_runner`, so the placement is `starting` with no failure event. Once the agent reports it started, it is `running` and its capacity is still held. | **Guard, green on master by design** (see below) |
| `a_placement_whose_machine_lost_authority_gives_up_past_the_cap` | D3 and A's warn 1: the cap does not depend on the attempt failing. The machine's authority lapses, and passes run without renewing it. Before the cap, GitHub is never asked (`asked == 0`) and the placement is held. Past the cap, it is given up with the same assertions as the first test. | RED |

**Why the guard is green before the feature, and why it is still a valid part of the contract.** On master nothing gives up a `registering` placement, so "not given up within the cap" holds trivially. The test exists to reject wrong implementations. I applied a throwaway sketch of D3/D4/D6, later reverted, and mutated it twice:
- **Age measured from `created_at`** (which D2 rejects): this guard **fails** at its refused 9-minute pass (`left: "failed", right: "registering"`), and both RED tests fail too.
- **`is_start_failure(RegistrationTimedOut) == true`** (which D6 rejects): both RED tests **fail** on the `placement_retried` assertion. The run showed the retry event, `"Started the runner again as pool-gh-… (attempt 2 of 3)"`.

A cap that fires too early also fails it, for example one that counts attempts or fires on the first error.

### Core unit tests (cheapest tier for the closed set and the start-gate rule)

| Test | File | Pins |
|---|---|---|
| `a_registration_timeout_has_its_own_stable_code` | `crates/gridops-core/src/fleet/failure.rs` (tests mod) | D4/D5: `from_code("registration_timed_out")` is not `Unknown`, `as_str()` round-trips, and serde serialises to the same code. |
| `a_registration_timeout_is_not_the_machines_fault` | `crates/gridops-core/src/fleet/start_health.rs` (tests mod) | D6: `is_start_failure(reason, None) == false`. This covers the start gate as well as retry. It is guarded by an assertion that the code is known, because `from_code` maps unknown codes to `Unknown`, which is already `false`. Without the guard the test would pass vacuously. |

The tests name the variant by its **code**, not as `PlacementFailureReason::RegistrationTimedOut`. Naming a variant that doesn't exist yet would make RED a compile error, which is the wrong reason. In Phase 6 I'll add the variant to `reason_codes_are_stable_and_round_trip` and `only_failures_the_machine_could_have_caused_count`, as D5 lists, and drop whatever then duplicates. **F does not edit tests.**

### TS contract tests: `tests/placement-failures.test.ts` (vitest)

| Test | Pins |
|---|---|
| `reads a machine whose runners could not register in time (#130)` | D5 and A's warn 2. A host read carrying `registration_timed_out` parses through `hostQueryResultSchema`, which uses a strict zod enum, and the label is **"Registration timed out"**. That wording is A's example in D5, and it is now pinned. |
| `accepts exactly the reasons the published API serves` | D5. The OpenAPI `PlacementFailureGroup.reason` enum contains `registration_timed_out`, and as a set it equals `placementFailureReasons`. That parity also stops future drift between the published contract and the UI's parser. |

### Fixture changes (extended, not copied: A's finding 5)

- `start_tests.rs`: `fake_github()` now also returns `Arc<GitHubRegistrations>`, which `Setup` exposes as `setup.github_registrations`. It has three parts:
  - `conflict`: an `AtomicBool`. While it is set, every JIT answers `409`, and `GET /orgs/{org}/actions/runners` lists the refused name as an **online, idle** runner, which #155 never removes.
  - `asked`: an `AtomicU64` that counts JIT requests.
  - While `conflict` is off, the list route answers 404, which is exactly what the fake answered before the route existed, so other tests' cleanup paths are unchanged.
- `gitlab_lifecycle_tests.rs`: `Fleet` is now `pub(crate)` and gains a `pool` field.
  - Machine seeding moved out of `with_pool` into a shared `Fleet::on_machine(setup, pool, target, target_kinds, requires_docker_socket)`.
  - `with_pool` (GitLab) and the new `Fleet::github()` both call it. `Fleet::github()` is the GitHub org pool `pool-gh`: Docker, Linux x64, `placement='machines'`, keeping 1 runner.
  - `newest`, `row` and `placed` read `self.pool` instead of the literal `'pool-gl'`.
  - `pass`, `newest`, `commands`, `prepared`, `started` and `finish` are `pub(crate)`.
  - All 126 pre-existing reconciler tests still pass, including every GitLab `Fleet` test.

## RED confirmation

**Rust, reconciler** (`cargo test -p gridops-reconciler registration_cap_tests`; the full run gives `126 passed; 2 failed`, and the 2 are the new tests):
```
test registration_cap_tests::a_runner_github_never_registers_gives_up_its_placement_past_the_cap ... FAILED
test registration_cap_tests::a_placement_whose_machine_lost_authority_gives_up_past_the_cap ... FAILED
test registration_cap_tests::a_runner_registered_within_the_cap_starts_normally ... ok
panicked at crates/gridops-reconciler/src/registration_cap_tests.rs:102:5:
assertion `left == right` failed
  left: "registering"
 right: "failed"
(same for both)
```
Line 102 is the first assertion in `assert_given_up`: the placement is still `registering` past the cap. Both tests' before-cap assertions passed first: refused passes held the capacity (`asked >= 2`, or `asked == 0` for the authority case). So each fails for the right reason: there is no cap.

**Rust, core** (`cargo test -p gridops-core --lib`; the full run gives `311 passed; 2 failed`, and the 2 are the new tests):
```
fleet::failure::tests::a_registration_timeout_has_its_own_stable_code
  panicked at crates/gridops-core/src/fleet/failure.rs:269:9:
  assertion `left != right` failed: registration_timed_out is in the closed set
    left: Unknown
   right: Unknown
fleet::start_health::tests::a_registration_timeout_is_not_the_machines_fault
  panicked at crates/gridops-core/src/fleet/start_health.rs:253:9:
  assertion `left == right` failed: the code is known, so the check below is about it
    left: "unknown"
   right: "registration_timed_out"
```

**TS** (`npx vitest run`: `2 failed | 624 passed (626)`, and the 2 are the new tests):
```
FAIL reads a machine whose runners could not register in time (#130)
  ZodError: invalid_value at item.recentPlacementFailures[0].reason: expected one of "agent_reported"|…|"unknown"
FAIL accepts exactly the reasons the published API serves
  AssertionError: expected [ 'agent_reported', …(8) ] to include 'registration_timed_out'
```
`tsc --noEmit` reports the expected RED type error, `tests/placement-failures.test.ts(43,31)`, because `"registration_timed_out"` is not yet a `Reason`. It goes away once F adds the code to `placementFailureReasons`. The other `tsc` errors in this fresh worktree come from the missing generated `src/routeTree.gen`, not from this change. `eslint tests/placement-failures.test.ts --max-warnings=0` passes.

**Satisfiability.** I applied the throwaway sketch above in this worktree:
- the variant, `as_str`, the `from_code` entry and the constructor in `failure.rs`;
- the `false` arm in `start_health.rs`;
- in `tick`, a check after every attempt of `now - placement.updated_at >= 600000`, then `abandon_unstarted(…registration_timed_out(elapsed, 600000, last_error))`.

Under it, all 3 reconciler tests and both core tests passed. Then I reverted it: `git checkout -- crates/gridops-reconciler/src/fleet.rs`, plus a hand revert of the `failure.rs` and `start_health.rs` production hunks. `git diff` now shows only test code. The TS tests need only the D5 list, label and OpenAPI edits.

**Lint and format.** `cargo fmt --check` passes. `cargo clippy -p gridops-core -p gridops-reconciler --all-targets -- -D warnings` reports no findings in any changed file. It does report 5 findings on master in files this branch doesn't touch:
- `gridops-core/src/autoscaling.rs:1890` and `:1976`;
- `gridops-core/src/autoscaling_gitlab_tests.rs:289` and `:385`;
- `gridops-reconciler/src/builtin_host.rs:119`.

These are clippy 0.1.99 lints, mostly `assert!` used to check emptiness. They will fail `npm run check:rust` whatever F does. I'm noting them for O rather than routing them to F.

**Environment.** `node_modules` was absent, so I ran `npm ci`, which is lockfile-pinned and gitignored. Rust ran with `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2` and the shared target dir.

## What F must satisfy (from these tests; D1-D10 still govern everything else)
- `REGISTRATION_TIMEOUT_MILLIS = 600000`. The tests spell the value out as `CAP_MILLIS`, because the constant is private to `fleet`.
- Measure age from `Placement.updated_at`. Check after every attempt, whatever it returned.
- Fail with `abandon_unstarted`, recording `reason=registration_timed_out` once, with the facts `elapsed_ms` and `allowed_ms`, and a message that carries the attempt's error (`{error}`, so the `409` shows) within 240 characters.
- `is_start_failure` returns `false`, so no `placement_retried` event.
- `from_code` must know the code.
- The TS `placementFailureReasons` must include `registration_timed_out`, labelled exactly "Registration timed out".
- The OpenAPI `PlacementFailureGroup.reason` enum must match the TS list as a set.

## Ready for F
Confirmed: RED is valid. The 6 new tests fail at their feature assertions, not at imports, setup or compilation. The 1 guard test is green by design and was shown to catch the D2 and early-give-up mistakes. All pre-existing tests still pass. F may implement against these tests.
