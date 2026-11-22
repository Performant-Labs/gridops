# Handoff-A: Phase 3 - Cap the time a fleet placement spends in `registering` (#130)  (up-front plan review)

**Date:** 2026-11-22
**Branch:** claude/fix-130-registering-cap (head 9e357e0; `git diff origin/master -- crates src migrations` is empty, so no code exists yet)
**Brief reviewed:** docs/handoffs/130-brief.md   **Reuse map:** there is no `docs/handoffs/130/survey.md`. The brief names its own analogues: `placement::abandon_unstarted`, `QUEUE_TIMEOUT_MILLIS` / `expire_queued`, `finish` → `retry::retry_failed_start`, `start_health::is_start_failure`, the failed-runner cleanup, and `jit_conflict_tests.rs`. Recommendation: EXTEND (no new module, no migration).   **Wireframe:** N/A (no UI surface; one closed-set label is added, see D5)
**Verdict:** PASS

## Summary
PASS, with 0 block and 6 warn findings. The plan extends the right objects: `abandon_unstarted` for the safe give-up, `PlacementFailure` for the cause, the existing `finish` / `delete_runner` path for the replacement and the upstream cleanup, and a reconciler constant in the style of `QUEUE_TIMEOUT_MILLIS`. Nothing in the brief is wrong. The brief leaves the mechanism to the architect, so the binding decisions D1-D10 below are the plan for T and F. The warns are places where the brief is incomplete:
- the trigger misses the exits that return `Ok` without making progress;
- a new failure reason changes the wire contract the UI parses strictly;
- GitLab can still stay in `registering`;
- the note belongs in a different doc;
- the tests could end up copying an existing fixture.

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | Build §1: "When `register_and_start` **fails** for a placement that has been in `registering` longer than a cap" | pattern consistency / sweep completeness | **Gating the cap on `Err` misses the exits that return `Ok(())` without progress.** When the host has no work authority (`fleet.rs:357-359`), or `issue_command` returns `None` because authority lapsed mid-attempt (`fleet.rs:519-528`), the placement stays `registering`, holding its allocation and its `starting` row. After a host recovery this lasts forever: the epoch bump (`registry/mod.rs:587-604`) leaves placements alone, and `host_has_authority` needs `p.host_epoch=h.epoch` (`placement.rs:124-127`). The live placement also blocks retiring that host (`retire.rs:138-139, 169-171`). The analogue the brief names, `expire_queued` (`fleet.rs:298-320`), is age-only and ignores why the intent is still queued. | D3: check the cap after every attempt, whatever it returned. This is a superset of item 1. Both required tests still hold. |
| 2 | warn | Build §1: "Record a `PlacementFailure` that names the cause" | cross-cutting (wire contract) | **The reasons are a closed set that is served to the UI** (`failure.rs:25-26`: "a new reason is a new variant and a new stable code, never free text"). `Unknown` is reserved for "no cause" (`failure.rs:7-9`), so this needs a new variant, which is a contract change. The brief names none of the surfaces. The UI parses the host read with `z.enum(placementFailureReasons)` inside a `strictObject` (`query-contracts.ts:129-148`), and `tests/placement-failures.test.ts` pins "refuses a reason outside the closed set". If the server emits the new code and the TS list doesn't have it, the machine page fails to parse for 24 h after any give-up on that host. | D4 + D5: one new variant, `registration_timed_out`, updated in every listed file in the same PR. |
| 3 | warn | Build §3: "register_and_start's GitLab branch already rolls back … Confirm that it cannot leave a placement in `registering` forever" | layering (where the cap lives) | **It can.** GitLab registration-API errors and `start_with_material` errors are handled: `fail_unstarted` abandons at once. These still loop:<br>- the shared prefix before the GitLab branch: no prepared environment, no prepare command, no runner row (`fleet.rs:360-377`);<br>- `?`-propagated errors inside `fleet_material` (`gitlab_lifecycle.rs:583-585, 602, 606-607, 624-629`, among them the `gitlab_runner_id` write after a successful registration);<br>- errors from `fleet_rollback` (`?` at `fleet.rs:398`);<br>- the no-authority exit from finding 1.<br>If F puts the cap inside the GitHub half of `register_and_start`, GitLab stays uncovered. | D3 / D8: put the cap in `tick`'s `registering` loop so it applies to every platform. No GitLab-specific code. |
| 4 | warn | Docs: "Add a short note to `docs/fleet-automation.md` (or the closest fleet doc)" | naming / file structure | `docs/fleet-automation.md` is the bot and API-token guide and has no placement-lifecycle text. The closest doc is `docs/fleet-api.md:609-628`, the `placement_failed` and retry paragraph. That paragraph has to change anyway: it lists every reason code and the codes that are not retried. | D10: put the note in `docs/fleet-api.md`, plus the CHANGELOG entry. |
| 5 | warn | Tests: "fleet fixtures as in the existing fleet tests" | abstraction level (test-code duplication) | **No reconciler test drives a GitHub fleet placement into `registering` today.** The pieces are spread out:<br>- `Fleet::with_pool` seeds the host, backend, credentials and profile, but only for GitLab `pool-gl` (`gitlab_lifecycle_tests.rs:1746-1858`);<br>- `Setup::new` builds the reconciler with a fixed fake GitHub (`start_tests.rs:405-449`). Its JIT always succeeds, and it has no runner-list route;<br>- `jit_conflict_tests.rs` has the 409-and-list fake, but it only exercises `GitHubClient`.<br>A third, hand-copied host-seeding fixture would be drift that Phase 7 flags. | T: extend what exists. Make the `Setup` fake GitHub's JIT answer configurable (or let `Setup` take a GitHub URL), and generalise `Fleet::with_pool`'s host seeding for a GitHub pool. Don't copy them. Backdate `workload_placements.updated_at` with SQL to get past the cap. Precedent: `gitlab_lifecycle_tests.rs:1609` backdates `runner_events.created_at`. |
| 6 | warn (follow-up only) | Not in the brief | cross-cutting (cause naming) | GitLab's `fail_unstarted` records every GitLab registration failure as `runner_removed_before_start("failed")` (`gitlab_lifecycle.rs:725-729`), which misnames the cause on the machine page. This is out of scope here. | No action this run. It is a candidate follow-up issue. |

No block findings. The plan is consistent with existing patterns once D1-D10 are followed.

## Architect decisions (binding for T and F; the brief delegates these)

**D1. The cap is a time cap.** Add `const REGISTRATION_TIMEOUT_MILLIS: i64 = 10 * 60_000;` in `crates/gridops-reconciler/src/fleet.rs`, next to `QUEUE_TIMEOUT_MILLIS` (`:42`). The reconciler's policy constants live there. Reasons for a time cap rather than an attempt count:
- A count needs a persisted counter, which means a migration.
- The pass interval is configurable (`reconcileIntervalSeconds`, default 30 s, `main.rs:487`), so a count is not a stable duration.

Ten minutes is about 20 attempts at the default interval, long enough to ride out a transient GitHub 5xx or rate limit.

**D2. Age is `now - placement.updated_at`.** `Placement.updated_at` is already loaded by `placement::placements` (`placement.rs:41, 48`). I confirmed it is a reliable entry time:
- The only write that sets `registering` is `apply_to_placement` when the prepare succeeds. It sets `updated_at=now` (`registry/commands.rs:348, 408-412`).
- Any later result for that prepare is terminal, so it is stored as `stale` and never applied (`commands.rs:249-262`).
- Every other writer of `workload_placements.updated_at` either moves the placement out of `registering` or never runs for it:
  - `mark_starting` (`placement.rs:364`);
  - `abandon_unstarted` and `fail_placement` (`placement.rs:407`, `failure.rs:199`);
  - `release()` (`commands.rs:503`), which is always followed by `fail_placement`;
  - `keep_uncertain_while_unsettled` and `cancel_undeliverable` (`:546`, `:792`);
  - `set_state` (`placement.rs:440`), which is called only from `starting`;
  - `unconfirmed.rs:172` and `commands.rs:638`, which only act on `uncertain`.
- No trigger touches `updated_at` (0022:438-500).
- The index `workload_placements_state_idx(state,updated_at)` (`0028_fleet_bootstrap.sql:18`) exists for exactly this kind of read.

So no migration. Don't use `created_at`: it is the admission time, so it includes the prepare (image pull or Tart clone), which has its own deadline. Put a one-line comment on the constant stating the invariant. T's backdating test pins it.

**D3. Where and when the cap fires.** In `tick`'s `registering` loop (`fleet.rs:272-276`), after each placement's `register_and_start` attempt, whatever it returned:
1. Keep the existing `warn!(… "could not start fleet runner")` on `Err`.
2. If `now - placement.updated_at >= REGISTRATION_TIMEOUT_MILLIS`, call `placement::abandon_unstarted(&app.database, placement.id, &PlacementFailure::registration_timed_out(…))`.
3. Log once at warn, only when it returns `true`.

`abandon_unstarted` is a no-op in every case where the placement must not be given up, so no new guard is needed:
- it started this pass: a `start_runner` row exists, so it refuses (`placement.rs:386-394`);
- GitLab's `fail_unstarted` already failed it: nothing is left to release, so it returns `false`;
- the runner was removed: same.

Put this in a small helper in `fleet.rs`, beside `expire_queued`. Add no new core function and no separate SQL sweep: the brief wants the last error, which only the attempt in hand has.

**D4. The cause.** Add the variant `PlacementFailureReason::RegistrationTimedOut`, code `registration_timed_out`. Add a named constructor `PlacementFailure::registration_timed_out(elapsed_ms, allowed_ms, last_error: Option<&str>)` in `failure.rs`, next to `runner_removed_before_start` and `operator_released`:
- The message says the runner could not be registered and started within the cap. It includes the attempt's error when there was one, and says that no start could be issued when there wasn't.
- `PlacementFailure::new` already truncates the message to 240 characters.
- Facts: `elapsed_ms` and `allowed_ms`, the same keys as `with_timing` (`commands.rs:652-666`) and the documented "numbers behind it".
- If you also add a `last_error` fact, truncate it before `.with(…)`, because `with` doesn't truncate. GitHub's own errors are already capped at 500 characters (`github.rs:319-322`), but other errors in the chain are not.

**D5. Every place the new code must land, in this PR.** Missing one breaks the contract:
- `failure.rs`: the variant, `as_str`, the `from_code` array, and the round-trip test list (`:235-261`). Only the `as_str` match is compiler-checked.
- `start_health.rs`: the `is_start_failure` arm, `false`, in the group with `RunnerRemovedBeforeStart`. The match is compiler-checked. Also add the case to `only_failures_the_machine_could_have_caused_count`.
- `docs/openapi/gridops-v1.json:230`: the `PlacementFailureGroup.reason` enum.
- `src/features/fleet/query-contracts.ts:129`: `placementFailureReasons`. Without it, the zod parse of the machine page fails.
- `src/features/fleet/placement-failures.ts`: the label, for example "Registration timed out". `Record<Reason,string>` makes the compiler require it once the TS list has the code.
- `docs/fleet-api.md:609-628`: the reason list, and the "not retried" list at `:626`.

**D6. A timeout is not a start failure.** `is_start_failure(RegistrationTimedOut, _) == false`.
- **Effect on retry.** `retry_failed_start` answers `NotAStartFailure` (`retry.rs:196`), so there is no immediate replacement on another machine.
- **Effect on the start gate.** It is not counted against the machine (`start_health.rs:113-159`). A GitHub problem across the whole org therefore can't take healthy machines out of placement.
- **How the runner is replaced.** `finish` fails the runner row (`fleet.rs:668-676`). The pool pass deletes it (`main.rs:953-960`) and provisions a new runner. That runner has to prepare and then spend at least the cap in `registering` before it is given up, so the loop wastes at most one prepare per pool slot about every 10 minutes. It is not a tight loop.
- **No GitHub provision-circuit count.** Do not add `record_provision_failure` counting for GitHub. GitHub fleet placement failures have never counted. GitLab already counts once through `finish` → `fleet_start_failed` (`fleet.rs:681-683`), guarded by `left`.

**D7. Upstream cleanup already exists. Add nothing.** The pool pass sends the failed row to `delete_runner` (`main.rs:2970`):
- For GitHub, it removes the registration by `github_runner_id`, or by `runner_by_name` when the id was never recorded (`main.rs:3014-3036`). This also covers a JIT that succeeded upstream when the `github_runner_id` write (`fleet.rs:441-446`) failed.
- A registration GitHub reports as running a job is not removed. A failed row is still marked deleted.
- For GitLab, it calls `gitlab_lifecycle::deregister_runner`, with the orphan sweep as backstop.

**D8. GitLab gets no separate code.** The cap in `tick` (D3) covers every GitLab path listed in finding 3. After a give-up the circuit counts exactly once, through `finish`.

**D9. Sweep of every exit from `register_and_start`.** F should copy this table into the result handoff.

| Exit (fleet.rs) | Today | With the cap (D3) |
|---|---|---|
| `host_has_authority` false (`:357-359`), `Ok` | stays `registering` (forever after an epoch bump) | ended |
| `host_has_authority` / `prepared_environment` / `prepared_with` DB error, or env / prepare missing (`:357-365`) | loops | ended |
| runner-row query error, or row missing, for example after an installation was removed (`:366-377`) | loops | ended |
| runner not `starting` (`:378-388`) | already abandoned (`RunnerRemovedBeforeStart`) | unchanged |
| GitLab: `fleet_material` registration error, or `start_with_material` error that `fleet_rollback` handles (`:392-399`) | already abandoned by `fail_unstarted` | unchanged; the cap is a no-op |
| GitLab: `?` error from `fleet_material` or `fleet_rollback` (`:392, :398`) | loops | ended |
| credentials unavailable (`:409-411`) | loops | ended |
| `generate_jit_config` error, including a 409 for a live runner (`:422-440`) | loops | ended |
| `github_runner_id` write error (`:441-446`) | loops; the upstream registration exists | ended; D7 removes it by name |
| `generate_registration_token` or repository query error (`:449-459`) | loops | ended |
| `store_bootstrap` / `issue_command` error (`:502-526`) | loops | ended |
| `issue_command` returns `None`: authority lapsed (`:519-528`) | stays `registering` | ended |
| `issue_command` returns `None`: key already exists, or `mark_starting` error after the start was issued (`:519-529`) | the command-result path resolves it (`running`, `failed` or `uncertain`) | `abandon_unstarted` refuses (a start exists), which is correct |

**D10. Docs.** Put the short note ("a runner that cannot register and start within 10 minutes gives up its placement and is replaced") in `docs/fleet-api.md`'s `placement_failed` paragraph, together with the D5 list edits. Add the CHANGELOG entry under `## Unreleased` → `### Fixed`, citing #130.

## Notes for O
None required (PASS). Warns 1-5 are settled by D1-D10, which T and F follow. Phase 7 checks:
- D3: the cap is in `tick` and does not depend on the result;
- D5: all six surfaces carry the new code;
- D7: no new cleanup path;
- finding 5: no copied fixture.

Warn 6 is a follow-up candidate only.

Coexistence: the sibling worktrees `gridops-124`, `-126` and `-152` hold only briefs. #152's brief may also add to `docs/fleet-api.md` and CHANGELOG, which would only mean an ordinary merge. No worktree has uncommitted changes to `fleet.rs`, `failure.rs`, `start_health.rs`, `placement.rs` or the fleet TS contracts.

## Patterns referenced
- `crates/gridops-reconciler/src/fleet.rs`: `:42` (`QUEUE_TIMEOUT_MILLIS`), `:267-320` (`tick` and `expire_queued`), `:356-531` (`register_and_start` and `start_with_material`), `:628-685` (`finish`).
- `crates/gridops-core/src/fleet/placement.rs`: `:41-78` (`Placement.updated_at`), `:120-133` (`host_has_authority`), `:378-430` (`abandon_unstarted`).
- `crates/gridops-core/src/fleet/failure.rs` (the closed reason set and its constructors), `start_health.rs:43-57` and `retry.rs:186-199`.
- `crates/gridops-core/src/fleet/registry/commands.rs:249-412, 503, 546, 638, 792`, `registry/mod.rs:587-604` and `registry/retire.rs:128-180`.
- `crates/gridops-reconciler/src/gitlab_lifecycle.rs:579-731`, `main.rs:953-960, 2970-3060`, `src/features/fleet/query-contracts.ts:129-149`, `docs/openapi/gridops-v1.json:230`, `docs/fleet-api.md:609-628`.
