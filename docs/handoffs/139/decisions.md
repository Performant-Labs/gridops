# Decision journal: #139 GitLab reconciler runner lifecycle

## A (Phase 3, up-front plan review): 2026-11-20T15:19:09-08:00
- **Decided:** BLOCK, with 4 block findings and 5 warn findings (see `handoff-A.md`).
  - (1) The orphan sweep has no seam. `GitLabApi`, which the brief puts off limits, has no way to list runners, and #146 doesn't add one. Without the sweep, there is also no recovery for a failed GitLab delete (no cleanup table, no migration allowed).
  - (2) Pool delete, and the runner delete and rebuild actions, run in the API (`delete_runner_resources`, which sends GitLab runners to `cleanup_github_runner`). The `runners` rows then cascade away, so the reconciler can't clean them up.
  - (3) "Same insert" contradicts D1 on the fleet path. The fleet path registers in `register_and_start`, after the row and placement exist. On the manager path it departs from the analogue, which inserts, registers in `start_admitted_runner`, updates the ID, and rolls back through `failure_cleanup`.
  - (4) "main.rs gets only the call sites" leaves out `fleet.rs`, core `placement.rs` (the D5 diagnostic), the `Runner` and `ManagedRunner` structs, `delete_runner`'s platform dispatch, `Cargo.toml` and the test constructors. Read literally, it leads to a parallel pipeline.
  - These parts of the plan were accepted as consistent: the child-module placement (`super::` imports, as in `fleet.rs`), the `gitlab` field, per-call credentials through `runtime_secret`, connection selection modelled on `next_bitbucket_connection`, the GitLab arms in `targets.rs` and `service.rs`, and D9's `CiPlatform::Gitlab`.
- **Assumed:** #138's "GitLab sections of `resources.rs`" doesn't cover runner deletion; its issue text lists only the connection and pool-target handlers. A pool can't mix GitLab targets with others (#138), so a pool with `runner_pool_gitlab_connections` rows is a GitLab pool.
- **Hedged:**
  - Finding 1 offers two options, (a) a contract extension or (b) descoping plus a database-only retry. A gives no exact retry design, because none is established for a platform without a cleanup table.
  - Finding 3 accepts "create before insert" on the manager path, if the brief limits it to that path in writing and keeps rollback in the existing failure branch.
  - No Reuse map (`survey.md`) was provided, so the brief's named analogues were the baseline.
- **Evidence:**
  - Read: the brief, `139-brief-result-r1.md`, issues #136, #138, #146 and #148 (via `gh issue view`), and #137's `decisions.md` and `handoff-A.md`.
  - Source: `crates/gridops-reconciler/src/{main.rs,fleet.rs,start_tests.rs (head),Cargo.toml}`, `crates/gridops-core/src/gitlab/{mod.rs,fake.rs}`, `crates/gridops-core/src/fleet/{placement.rs,targets.rs,service.rs,protocol/agent.rs,protocol/outcomes.rs}`, `crates/gridops-core/src/provisioning.rs` (`failure_cleanup`), `crates/gridops-api/src/resources.rs` (`delete_runner_pool`, `delete_runner_resources`), `migrations/0047_gitlab.sql`, and `git show 08c32b3`.

## A (Phase 3, up-front plan review, round 2 on the amended brief): 2026-11-20T15:34:37-08:00
- **Decided:** BLOCK, with 4 block findings and 5 warn findings (see `handoff-A.md`).
  - Round-1 findings 1–3 are resolved: D17 option (a), #138 owns the API delete arm, and each path has one ordering. Finding 4 is resolved except for `retry.rs`.
  - New blocks:
    - (1) Counting `RegistrationRefused` through `record_provision_failure` doesn't work. On the manager path, `start_admitted_runner` resets the count at POST success (`main.rs:2766`), before the refusal arrives, so the count cycles 0→1 and the create/refuse/delete loop goes on. The fleet path has no reset, so a pool would stay `blocked`.
    - (2) D147-1's "single substitution point" in `gitlab_lifecycle.rs` can't serve the fleet path. Core `pool_docker_image` fixes the image when the API saves the profile, and core can't call into the reconciler.
    - (3) The sweep's connection set ("referenced by a pool or a `runners` row") misses the connection whose only pool was deleted, because both tables cascade. That is the pool-delete case the sweep covers.
    - (4) Core `fleet/retry.rs` copies `ci_platform` but not `gitlab_connection_id` into the retried runner row, and the file isn't in the seam or Owns.
  - Warnings:
    - `RunnerSummary.id` is `u64` while the rest of the contract uses `ExternalId`.
    - The sweep cadence analogue is wrong (`cleanup_retention` runs every pass; use the `maybe_sync_github` pattern).
    - The marker has no instance discriminator, so two deployments would sweep each other's runners.
    - A failed fleet `create_runner` leaves the placement `registering` (name `abandon_unstarted`).
    - The `builtin_host.rs` test fixture needs the new `Runner` fields.
- **Assumed:**
  - #147's agent runs the image from `PrepareEnvironment` and doesn't pick the gitlab-runner image itself (per the brief's D147-1 note).
  - A GitLab connection's access token stays readable after its last pool is deleted, until the connection row is deleted.
- **Hedged:**
  - Finding 1's fix names "proof of registration" (a `completed` or `idle_expired` exit, or busy) as the reset point. Another reset point is acceptable if the brief states one for both paths.
  - Finding 2 offers two homes: core `targets.rs`, or the profile-save owner (#138/#141).
  - Warning 7 is a warn, not a block, because it needs two deployments on one GitLab scope. It is flagged for O to decide while amending D17.
- **Evidence:**
  - Read: the amended brief (`git diff`), `139-brief-result-r1.md`, #136's comments (D1–D16 and D17), and issues #138, #140, #146, #147 and #148.
  - Source: `crates/gridops-reconciler/src/{main.rs (Reconciler, Runner, ManagerView, reconcile, reconcile_pool, provision, provision_on_host, start_admitted_runner, delete_runner, retry_github_runner_cleanup, maybe_sync_github, cleanup_retention, record_provision_failure, pool_state), fleet.rs (whole file), builtin_host.rs (tests), Cargo.toml}`, `crates/gridops-core/src/gitlab/{mod.rs,fake.rs}`, `crates/gridops-core/src/fleet/{domain.rs (ExternalId), targets.rs (pool_docker_image, save_pool_fleet, link_pool_target, pool_targets, delete_pool), service.rs (ensure_target_identity_for_pool), retry.rs (retry_failed_start), start_health.rs, placement.rs (reported_gone), protocol/agent.rs, protocol/outcomes.rs}`, `crates/gridops-core/src/provisioning.rs` (`for_selected_provider`, `failure_cleanup`) and `migrations/0047_gitlab.sql`.
