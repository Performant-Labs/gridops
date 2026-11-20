# Decision journal: #137 GitLab shared contract

## A (Phase 3, up-front plan review): 2026-11-20T11:21:08-08:00
- **Decided:** BLOCK, with 3 block findings and 8 warn findings (see `handoff-A.md`).
  - (1) The brief adds `CiTarget` GitLab variants without widening `fleet_ci_targets`, which is the only source of `PrepareEnvironment.target` (through `target_from_db`).
  - (2) The base URL is planned for `ExecutionConfiguration`, but the existing pattern puts it in the bootstrap material next to the token (`github_registration { token, url }`). Because the struct uses `deny_unknown_fields`, the new field also risks older host agents rejecting every command.
  - (3) `GitLabApi` methods take no connection credentials and `Scope` is undefined. That doesn't fit the single shared-client pattern (`BitbucketClient` in `AppState`) or #138's single `Arc<dyn GitLabApi>`.
  - These parts of the plan were accepted as consistent: module placement in gridops-core, the `async_trait` seam, the `test-support` feature, a separate `gitlab_jobs` table, `runner_pool_gitlab_connections`, the type naming, and the deliberate `runners` rebuild.
- **Assumed:** Wave-1 GitLab runners on fleet machines (#147, #142) go through fleet admission and placement. This rests on #147 owning `gridops-host-agent/src/docker.rs` and on `builtin_host.rs` describing the built-in host as the runner-manager path, not the host agent. Host agents can run an older binary than the server. This rests on the `results_flushed` "Older agents leave it out" precedent and the `AgentUpgraded` operation.
- **Hedged:** Finding 1 offers O two resolutions: fold the `fleet_ci_targets` rebuild into 0046, or name the owning issue in writing. Finding 2 accepts keeping `ExecutionConfiguration` if the brief justifies it in writing and adds `skip_serializing_if` and a byte-identity test. Either way, the epic's own text (#139) says "base URL in `configuration`", so O must record the decision where #139 reads it. No Reuse map (`survey.md`) was provided, so the brief's named analogues were the baseline.
- **Evidence:**
  - Read: the brief, the brief-gate results r1 and r2, and epic #136 plus issues #138, #139, #140, #145, #147 and #148 (via `gh issue view`).
  - Source: `crates/gridops-core/src/fleet/{domain.rs,admission.rs,targets.rs,capabilities.rs,placement.rs,protocol/agent.rs,protocol/outcomes.rs,protocol/inventory.rs}`, `crates/gridops-core/src/{bitbucket.rs,db.rs,lib.rs,builtin_host.rs}`, `crates/gridops-host-agent/src/{docker.rs,client.rs}`, `crates/gridops-reconciler/src/fleet.rs`, `crates/gridops-api/src/{main.rs,state.rs,error.rs,resources.rs (Bitbucket sections),fleet/openapi.rs}` and `crates/gridops-agent/src/session.rs`.
  - Migrations: 0018, 0019, 0020, 0022, 0028, 0043 and 0045.
  - Spec: a summary of the `docs/openapi/gridops-v1.json` paths, security and components.

## A (Phase 3, up-front plan review, round 2): 2026-11-20T12:03:51-08:00
- **Decided:** BLOCK, with 1 block finding and 4 warn findings (see `handoff-A.md`).
  - All three round-1 blocks are resolved:
    - (R-1) A-1 rebuilds `fleet_ci_targets`.
    - (R-2) A-2 puts the base URL in `GitlabRunnerMaterial` and leaves `ExecutionConfiguration` untouched.
    - (R-3) A-3 adds per-call `GitLabCredentials` and defines `Scope`.
  - All eight round-1 warnings are resolved.
  - The new block: the contract pins the agent protocol only for the fleet host-agent path. Tart/macOS runners (#142) go through the runner-manager path instead: the reconciler's `start`, then `gridops-manager` `POST /v1/runners`, then `gridops-tart-agent`, using the private camelCase `ProvisionRunner` structs. Platform selection is `ProvisioningPlan::for_selected_provider`, and exits come back through the `GET /v1/runners` status listing. None of these is pinned for GitLab. The `GitlabRunnerMaterial` doc comment and W-4 also name #142 as a consumer of channels it never receives.
  - The warnings:
    - `ensure_target_identity_for_pool` refuses GitLab kinds, and no owner is named for that arm.
    - The outbound FKs of `fleet_ci_targets` are neither listed nor asserted.
    - No single owner is named for the `AppState` and `Reconciler` `gitlab` field.
    - `{connectionId}` should be `{connection_id}`.
- **Assumed:** None new. The round-1 assumption that #142 runs through fleet admission and placement was **refuted**:
  - `HostRuntime::call` answers `Unsupported` for `TartVm`;
  - `backends()` lists Docker and native only;
  - `gridops-tart-agent` has no `gridops-core` dependency.
- **Hedged:** Finding 1 offers O two resolutions. Either (a) pin the runner-manager GitLab contract in #137 (the `ProvisioningPlan` signature, the `/v1/runners` GitLab JSON, the exit status strings, and an owner for the manager arm), or (b) descope GitLab on Tart and the built-in host in writing. I classed it as correctness, not completeness, because the brief's pinned doc comment and W-4 contradict the code about how #142 receives its material and reports its outcomes. No Reuse map (`survey.md`) was provided.
- **Evidence:**
  - Read: the brief (all 645 lines), the round-1 `handoff-A.md`, epic #136, and issues #138, #139, #140, #142 and #147 (via `gh issue view`).
  - Host agent and runner-manager path: `crates/gridops-host-agent/src/{runtime.rs,docker.rs}`, `crates/gridops-tart-agent/{Cargo.toml,src/main.rs}` (`ProvisionRunner`, `validate`, `list_runners`), `crates/gridops-manager/src/main.rs` (`ProvisionRunner`, `validate`, `provision_runner`), `crates/gridops-reconciler/src/{main.rs,fleet.rs}` and `crates/gridops-core/src/provisioning.rs`.
  - Fleet core: `crates/gridops-core/src/fleet/{targets.rs,admission.rs,service.rs,registry/queries.rs,protocol/outcomes.rs}`, `crates/gridops-core/src/{bitbucket.rs,lib.rs}` and `crates/gridops-core/Cargo.toml`.
  - API: `crates/gridops-api/src/{main.rs,state.rs,resources.rs (Bitbucket JSON)}` and the path parameters of `docs/openapi/gridops-v1.json`.
  - Migrations: 0018, 0022 and 0043.
  - Dependencies: `sqlx-core-0.9.0/src/migrate/source.rs:234` and `sqlx-sqlite-0.9.0/src/migrate.rs` (`no_tx` handling).
