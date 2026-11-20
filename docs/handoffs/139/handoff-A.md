# Handoff-A: Phase 3 - GitLab reconciler runner lifecycle (#139)  (up-front plan review, round 2)

**Date:** 2026-11-20
**Branch:** claude/gitlab-139-lifecycle-53c8ce (head 349b958; master b22c451)
**Brief reviewed:** docs/handoffs/139-brief.md (as amended after round 1: D17, findings 2–4, Host-agent facts)   **Reuse map:** N/A. No `survey.md` was provided, so the brief's own analogues are the baseline: `next_bitbucket_connection`, the Bitbucket and GitHub arms of `provision_on_host` / `start_admitted_runner` / `delete_runner`, the GitHub fleet path in `fleet.rs`, and `start_tests.rs`.   **Wireframe:** N/A (no UI surface)
**Verdict:** BLOCK

## Summary

BLOCK, with 4 block findings and 5 warnings.

The amendment fixes most of round 1. D17 gives the sweep a seam, #138 owns the API delete arm, each path now has one ordering that matches its analogue, and the seam keeps `gitlab_lifecycle.rs` as helpers instead of a second pipeline.

Four parts of the amended plan are still wrong or incomplete when checked against the code:
1. **Counting `RegistrationRefused`.** It can't stop the create/refuse/delete loop on the manager path, and it would leave fleet pools stuck `blocked`.
2. **The single image substitution point.** It is placed in the reconciler, but core chooses the fleet image when the profile is saved.
3. **The sweep.** It skips the connection whose only pool was deleted, which is the pool-delete case it is meant to cover.
4. **`fleet/retry.rs`.** It rebuilds GitLab runner rows without their connection, and it isn't in the seam.

## Status of round-1 findings

| Round 1 | Status |
|---|---|
| 1 (sweep has no seam) | **Resolved** by D17, option (a), recorded on #136. The sweep's connection set has a new gap: finding 3 below. |
| 2 (pool delete happens in the API) | **Resolved.** #138 owns the GitLab arm of `delete_runner_resources` (per the D17 comment). The sweep is the backstop, which depends on finding 3. |
| 3 ("same insert") | **Resolved.** The manager path inserts, registers in `start_admitted_runner` and then UPDATEs, like Bitbucket. The fleet path inserts in `fleet::provision` and registers in `register_and_start`, like GitHub. |
| 4 (seam / file list) | **Resolved**, except for `fleet/retry.rs` (finding 4) and one test fixture (warning 9). |
| Warnings 5, 6, 8, 9 | Resolved: typed `GitlabRunnerMaterial`, target from the selected connection, a private placeholder, tests on `Setup`/`ManagerLog`. |
| Warning 7 (count the refusal) | Adopted, but the mechanism doesn't work: finding 1. |

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | block | `RegistrationRefused` is "counted through `record_provision_failure` so a refused token does not create/start/delete every pass" (the `reconcile_pool` arm, the fleet `follow`/`finish` arms, Host-agent facts) | cross-cutting (failure accounting) | **The plan reuses the pool circuit without its reset rule, so it fails on both paths.** <br>**Manager path:** `start_admitted_runner` clears the pool's failure count as soon as the manager `POST /v1/runners` succeeds (`main.rs:2766`). The refusal comes later, as `exitReason` in the listing. The count therefore cycles 0 → 1 → 0 → 1 and never reaches the 3 that opens the circuit. At one attempt the backoff is 30 s (`provision_backoff_ms`), so a refused token still creates, starts, refuses and deletes a GitLab runner about every pass. That is the loop warning 7 was meant to stop. <br>**Fleet path:** it has never fed the pool circuit (`record_provision_failure` is called only at `main.rs:942` and `:2503`), and nothing on it resets the circuit. After three refusals, however far apart, a machines-only GitLab pool shows `blocked` (`pool_state`; "stays open … until a start succeeds", `main.rs:1101`). It stays `blocked` after the token is fixed, until an operator retries or edits the pool. | **Define the reset for GitLab.** <br>- The count resets only on proof that registration worked: a `completed` or `idle_expired` exit (the manager's `exitReason`; on the fleet path, an Inspect `ResourceUnavailable` with no diagnostic or with `IdleExpired`), or the runner going busy. <br>- `start_admitted_runner` skips its reset at POST success for `platform == "gitlab"`. <br>- Move that UPDATE into one helper, e.g. `clear_provision_failures(database, pool_id)`, called by the existing site and by both GitLab arms. <br>- On the fleet path, `fleet.rs` reaches `record_provision_failure` through `super::`, as it already reaches `installation_token`. Pass it a `Pool` freshly loaded by id, because it reads `provision_failure_count` and `provision_circuit_open` from what it is given. <br>- Done-when gains two cases: three manager-path refusals open the circuit; a fleet success after refusals clears it. |
| 2 | block | D147-1: "#139 substitutes `GITLAB_RUNNER_IMAGE` … on both paths … a single clearly named substitution point (a `gitlab_runner_image(pool_docker_image: &str)` helper in `gitlab_lifecycle.rs`)" | dependency direction / layering | **The reconciler never chooses the fleet image, and core can't call into the reconciler.** `pool_docker_image` (core `fleet/targets.rs:454`) runs only inside `save_pool_fleet` (`:501`), when the API saves a profile (`gridops-api/src/fleet/profiles.rs:115`). The result is stored in `pool_execution_profiles.image`, and admission sends that image in `PrepareEnvironment` (`admission.rs:596`). `fleet::provision` never sees it. Followed literally, the brief leaves F two choices. One is a second substitution in core, which breaks "single point". The other leaves GitLab fleet profiles on `STOCK_DOCKER_IMAGE` (`ghcr.io/actions/actions-runner:latest`), which has no `gitlab-runner`, so every GitLab fleet start fails. | Put the one substitution point in **core**, which both paths can reach. For example: `pub fn gitlab_runner_image(docker_image: &str) -> Option<String>` in `fleet/targets.rs` (already in **Owns**). <br>- `pool_docker_image` calls it for a pool with `runner_pool_gitlab_connections` rows. <br>- `provision_on_host` calls it before `for_selected_provider`. <br>- Until #147's constant lands, it returns `None` for an empty image. `save_pool_fleet` then refuses the save readably with "platform needs an image" (`targets.rs:517`), and the manager path refuses the start before `create_runner`. <br>Alternative: give the fleet image to the profile-save owner (#138/#141), next to `requires_docker_socket`, and say that the reconciler helper covers the manager path only. |
| 3 | block | Sweep: "per GitLab connection that any pool references (or that has any `runners` row)", which is also the pool-delete coverage from finding 2 | correctness of the finding-2 fix | **The sweep misses the case it exists for.** `delete_pool` hard-deletes the `runner_pools` row (`targets.rs:816`). `runner_pool_gitlab_connections` and `runners` cascade with it (`migrations/0047_gitlab.sql`, both `ON DELETE cascade` on `pool_id`). After the only pool on a connection is deleted, that connection matches neither clause and is never listed again. Any GitLab runner #138's arm "leaves for the sweep" then leaks for good. A Done-when test with a second pool on the same connection passes, while the single-pool case leaks. | Sweep **every `gitlab_connections` row**. Its access token stays readable until the connection itself is deleted. Add a Done-when case where the deleted pool was the connection's only pool. |
| 4 | block | Seam list / **Owns** (the round-1 finding-4 fix) | completeness / ownership | **`retry::retry_failed_start` (core `fleet/retry.rs:226-233`) rebuilds a failed fleet runner's row by `INSERT … SELECT`.** It copies `ci_platform` but not `gitlab_connection_id`. A GitLab start that fails because of the machine (a deadline, a lost session, an epoch change; see `is_start_failure`) is retried as a `ci_platform='gitlab'` row with no connection. `register_and_start` can't register it, and `delete_runner` can't deregister it. `retry.rs` is in neither the seam nor **Owns**, so fixing it would be an unplanned edit of shared core code. | Add `crates/gridops-core/src/fleet/retry.rs` to the seam and **Owns**: copy `gitlab_connection_id` in the `INSERT … SELECT`. Never copy `gitlab_runner_id`; the new registration sets it. Add a Done-when case: a retried GitLab fleet start registers on the same connection. |
| 5 | warn | D17 `RunnerSummary { id: u64 }`; the "test-only way to seed a runner" | pattern consistency | Every other ID in the GitLab contract is an `ExternalId`: `CreatedRunner.id`, `Job.id`, `Job.project_id` and the `Scope` IDs. `ExternalId` already deserializes from `u64` through `try_from` (`fleet/domain.rs:15-17`), so #146's client could check the value while deserializing, as it does for `Job`. With `u64`, every caller converts and decides what to skip. | Prefer `id: ExternalId`, with a one-line follow-up on the D17 comment so #146 matches. If `u64` stays, convert in exactly one place, in the sweep. Name the fake's seeding method `with_runner`, in the `with_scope` / `with_job` / `with_group_project` style. |
| 6 | warn | Sweep cadence: "reuse the existing retention/cleanup cadence" | pattern consistency | `cleanup_retention` runs on every pass (`main.rs:610`, and `:553` when the manager is down), so it offers no slow cadence to reuse. The project's slow-cadence pattern is `maybe_sync_github` (`main.rs:1555`): an interval setting plus a `last…At` row in `settings`. | Use `gitlabRunnerSweepIntervalSeconds` and `lastGitlabRunnerSweepAt` in that pattern. Call the sweep from `reconcile()` next to `retry_github_runner_cleanup`, before the manager request so it still runs while the manager is down. Wrap it in `if let Err(error) = … { tracing::warn!(…) }`. |
| 7 | warn | Description marker `gridops-runner:<runner uuid>` | cross-cutting (a destructive sweep over a shared upstream) | The marker doesn't say which GridOps instance created the runner. Two deployments connected to the same project or group (production and a dev instance, say) would each see the other's runners as marked runners with no row, and delete them, killing jobs mid-run. GitHub's cleanup deletes only IDs it stored, so this sweep is the codebase's first pattern-matched delete in a namespace other systems share. | Put a stable instance discriminator in the marker, such as the host of `GRIDOPS_BASE_URL` (`Config::base_url()`), and sweep only this instance's markers. Otherwise, record a decision to accept the risk. |
| 8 | warn | Fleet path: "the start fails cleanly" when `create_runner` fails (the placeholder client) | error handling | `fleet::tick` logs a `register_and_start` error and tries again on the next tick (`fleet.rs:251-255`). The placement stays `registering` and keeps its machine capacity, so while the placeholder is in place, every GitLab fleet start would wait there indefinitely. A retry tick after a failed bootstrap store or start command would also create a second GitLab registration while the row is live, where the sweep can't reach it. | Name the existing primitive. On a GitLab registration failure, call `placement::abandon_unstarted` with a `PlacementFailure` (as `register_and_start` already does for a removed runner, `fleet.rs:360-367`), then mark the row failed. Before creating a registration, delete any `gitlab_runner_id` already stored on the row. |
| 9 | warn | Fixture list | file structure | The `builtin_host.rs` tests build a `Runner { .. }` literal (`builtin_host.rs:63`). It breaks once `Runner` gains the GitLab columns. | Add the `builtin_host.rs` tests to the fixtures-only list, next to `start_tests.rs`. |

## Notes for O

Amend `docs/handoffs/139-brief.md`, then start a **fresh** run. Do not use `resumeFromRunId`.

1. **Finding 1.** Replace "counted through `record_provision_failure`" with a complete rule:
   - What counts: a refusal on either path.
   - What resets: only proof of registration, meaning a `completed` or `idle_expired` exit (or busy), on both paths.
   - `start_admitted_runner` does not reset at POST success for GitLab.
   - One shared reset helper.
   - The fleet path loads the pool by id before calling `record_provision_failure`.
   - Done-when: three manager-path refusals open the circuit, and a fleet success clears it.
2. **Finding 2.** Move D147-1's substitution point into core `fleet/targets.rs`, used by `pool_docker_image` (fleet, at profile save) and by `provision_on_host` (manager). Alternatively, hand the fleet image to #138/#141 with `requires_docker_socket` and limit the reconciler helper to the manager path. State which.
3. **Finding 3.** Change the sweep's set to "every `gitlab_connections` row". Add the "only pool on the connection was deleted" case to Done-when.
4. **Finding 4.** Add `crates/gridops-core/src/fleet/retry.rs` to the seam and **Owns**: the retry's `INSERT … SELECT` copies `gitlab_connection_id`, not `gitlab_runner_id`. Add a retried GitLab fleet start to Done-when.
5. While amending, decide warning 7 (an instance discriminator in the marker) and warning 5 (`RunnerSummary.id` type), since both touch the published D17 comment. Warnings 6, 8 and 9 can go into the brief as one line each.

## Patterns referenced

- `crates/gridops-reconciler/src/main.rs`:
  - `start_admitted_runner`'s circuit reset (`:2766`), `record_provision_failure` (`:3608`), `pool_state` (`:1101`);
  - `reconcile()` (`:522`), `cleanup_retention` (`:3103`), `maybe_sync_github` (`:1555`), `retry_github_runner_cleanup` (`:1440`).
- `crates/gridops-reconciler/src/fleet.rs`: `tick`, `register_and_start` (`abandon_unstarted` at `:360`), `follow`, `finish`.
- `crates/gridops-core/src/fleet/`:
  - `targets.rs`: `pool_docker_image`, `save_pool_fleet`, `delete_pool`;
  - `retry.rs`: `retry_failed_start`;
  - `admission.rs:596`, `start_health.rs`, `placement.rs`: `reported_gone`.
- `crates/gridops-core/src/gitlab/{mod.rs,fake.rs}`, `crates/gridops-core/src/fleet/domain.rs` (`ExternalId`), `crates/gridops-core/src/provisioning.rs` (`for_selected_provider`, `failure_cleanup`).
- `migrations/0047_gitlab.sql`; #136's D17 comment (issuecomment-7053069685); issues #138, #140, #146, #147 and #148.
