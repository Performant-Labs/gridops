# Decision journal: #130 cap the time a fleet placement spends in `registering`

## Phase 3 Architecture (A, up-front plan review) - 2026-11-22T10:33:00-08:00
- **Decided:** PASS, with 0 block and 6 warn findings (`docs/handoffs/130/handoff-A.md`). The brief leaves the mechanism to the architect. The binding decisions are D1-D10 in the handoff:
  - **D1:** a time cap, `REGISTRATION_TIMEOUT_MILLIS = 10 min`, defined in reconciler `fleet.rs` next to `QUEUE_TIMEOUT_MILLIS`. No attempt count: that would need a migration, and the pass interval is configurable.
  - **D2:** the age is `now - Placement.updated_at`. It is reliable: the only write that sets `registering` is the prepare-success branch of `apply_to_placement`, every other writer leaves `registering` or never runs for it, and later prepare results are stored `stale`. The `(state,updated_at)` index exists for this. No migration. `created_at` would wrongly count the prepare time.
  - **D3:** the check runs in `tick`'s `registering` loop, after every attempt, whatever it returned. It calls `abandon_unstarted`, whose own guards make it a no-op for a placement that started or already failed. It logs at warn once, when `abandon_unstarted` returns `true`.
  - **D4:** a new reason, `registration_timed_out`, with a named constructor next to `runner_removed_before_start`. The facts are `elapsed_ms` and `allowed_ms`, plus the attempt's error, truncated.
  - **D5:** the code lands in six places in the same PR: `failure.rs`, `start_health.rs`, the OpenAPI enum, the TS `placementFailureReasons`, the TS label, and `docs/fleet-api.md`.
  - **D6:** `is_start_failure` is `false`, so the start is not retried and the machine's start gate does not count it. Replacement goes through the pool's normal provisioning, at most one wasted prepare per slot per cap. No GitHub provision-circuit counting.
  - **D7:** the upstream cleanup exists: `delete_runner` removes by id or by name. Nothing new.
  - **D8:** GitLab is covered by D3, with no GitLab-specific code, and counts once through `finish`.
  - **D9:** the sweep table of every exit.
  - **D10:** the doc note goes in `docs/fleet-api.md`, not `fleet-automation.md`. The CHANGELOG entry goes under Unreleased → Fixed.
- **Warns:**
  1. Gating the cap on `Err` alone misses the exits that return `Ok` without progress, chiefly a host without authority. After a host recovery that one is permanent, and it also blocks retiring the host. Resolved by D3.
  2. The new reason is a change to the closed, wire-served set, and the UI's zod parse rejects codes it doesn't know. Resolved by D4 and D5.
  3. GitLab can stay in `registering` through `?` errors and the shared prefix. Resolved by D3 and D8.
  4. The doc home is wrong. Resolved by D10.
  5. The tests risk copying the existing fleet fixture.
  6. GitLab's `fail_unstarted` misnames its cause as `runner_removed_before_start`. Follow-up only.
- **Assumed:**
  - With no `survey.md`, the analogues the brief names serve as the Reuse map.
  - The API and the reconciler use the same wall clock, so `updated_at` (written by the API) compares with the reconciler's `now_millis()`. They run on the same host in prod.
  - Admission never places on a host without authority, so a give-up because of an offline host is replaced elsewhere.
- **Hedged:**
  - D3 widens the brief's trigger from "fails" to "past the cap, whatever the attempt returned". It is a superset, both required tests still hold, and it follows the `expire_queued` analogue the brief names. I did not block, because the brief leaves the mechanism to the architect and asks for the class to be swept. If the operator wants the narrower trigger, D9's no-authority rows become "not ended" and nothing else changes.
  - The code name `registration_timed_out` is chosen over `registration_deadline_expired` on purpose. The `*_deadline_expired` codes count as machine faults, and this one does not.
  - `delete_runner`'s by-name fallback would remove an *online, idle* same-named GitHub runner of a failed row. That is existing behaviour, and the name is unique to the row, so I raised no finding.
- **Evidence:**
  - **Files read:**
    - the brief;
    - reconciler `fleet.rs` (all of it), `gitlab_lifecycle.rs:480-800`, `main.rs` (the pool pass, `delete_runner`, `retry_github_runner_cleanup`, the provision circuit), `jit_conflict_tests.rs`, `start_tests.rs` (`Setup`, `fake_github`) and `gitlab_lifecycle_tests.rs` (the `Fleet` fixture);
    - core `fleet/placement.rs`, `failure.rs`, `retry.rs` and `start_health.rs`;
    - in `registry/`: `commands.rs` (record_result, apply_to_placement, release, settle_unreceived, cancel_undeliverable), `unconfirmed.rs`, `retire.rs` and `mod.rs` (the recovery epoch bump);
    - `queries.rs` (`placement_failures`), `github.rs` (`remove_runner`, `generate_jit_config`);
    - `migrations/0022` (`workload_placements` and its triggers) and `0028` (the state/updated_at index);
    - `src/features/fleet/query-contracts.ts`, `placement-failures.ts`, `tests/placement-failures.test.ts`, the OpenAPI enum, `docs/fleet-api.md`, `docs/fleet-automation.md` and CHANGELOG.
  - **Checks run:**
    - every `UPDATE workload_placements` writer, found with grep;
    - every place a reason code is listed (`operator_released` / `runner_removed_before_start`);
    - `git diff origin/master -- crates src migrations` is empty;
    - `git worktree list`, plus each sibling worktree's status and its brief, for overlap.
