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

## A (Phase 3, up-front plan review, round 3): 2026-11-20T12:39:52-08:00
- **Decided:** PASS, with 0 block findings and 4 warn findings (see `handoff-A.md`).
  - The round-2 block R2-1 is resolved by option (a), which pins:
    - `for_selected_provider(provider, platform: CiPlatform, ...)`;
    - the `/v1/runners` `gitlab {url, waitTimeoutSeconds}` and `gitlabRunnerToken` fields in both private `ProvisionRunner` structs;
    - the `("gitlab", _)` validation arms;
    - `exitReason` with the exit codes 0, 75 and 77 and `GITLAB_EXIT_REASONS`;
    - ownership in D9 to D12.
  - R2-W1 to R2-W4 are all resolved.
  - The new warnings:
    - (1) #139's row puts GitLab registration into the API's in-request `provision` (`resources.rs:4169`), a second lifecycle in a file whose GitLab sections #138 owns, with no deviation recorded. The recommended fix routes GitLab pool starts to the reconciler, following #119's `starts_belong_to_reconciler`.
    - (2) The manager re-parses the Tart listing into its own `ManagedRunner`, which has no `exit_reason`, so a Tart `exitReason` would be dropped before the reconciler sees it.
    - (3) The pinned `GitlabBootstrap` lacks `Debug`, which the Tart agent's `ProvisionRunner` derive needs, and uses `deny_unknown_fields`, which neither crate's `BitbucketBootstrap` does.
    - (4) Text still names #142 as a fleet-path consumer: brief lines 203, 435 and 582, and rows D1, D5 and D6.
- **Assumed:** #137's own implementation doesn't depend on warns 1 or 4. They change only the ownership rows and the deviation table. Warns 2 and 3 touch code #137 writes, so they are cheap to fold in before T. The task text named no earlier BLOCK findings to mark, so R2-1 is reported under "Resolved findings".
- **Hedged:**
  - Warn 1 isn't a block, because the code already duplicates Bitbucket registration in the API and the reconciler. The departure is from the epic's disjoint-ownership rule, not from a code pattern.
  - Warn 2 isn't a block. The name and values are pinned, and the natural way #147 adds the field to the shared `ManagedRunner` would carry Tart entries through anyway. The risk is merge order and a non-obvious implementation.
  - No Reuse map (`survey.md`) was provided.
- **Evidence:**
  - Read: the brief (all 1010 lines), `git diff f51bfcc..HEAD -- docs/handoffs/137-brief.md`, the round-2 `handoff-A.md` and `decisions.md`, and epic #136 plus issues #139, #142 and #147 (via `gh issue view`).
  - Core: `crates/gridops-core/src/{provisioning.rs,lib.rs}`, and `fleet/domain.rs` (`CiPlatform`).
  - Manager: `crates/gridops-manager/src/main.rs` (`ProvisionRunner`, `BitbucketBootstrap`, `validate`, `ManagedRunner`, `list_runners`, `tart_runners`, `start_runner` forwarding, `take_bootstrap_secret`, `runner_command`).
  - Tart agent: `crates/gridops-tart-agent/src/main.rs` (`ProvisionRunner`, `BitbucketBootstrap`, `validate`, `list_runners`, `start_vm`, `start_runner`, `bitbucket_runner_script`).
  - Reconciler: `crates/gridops-reconciler/src/main.rs` (`MANAGER_RUNNER_LIST`, `ManagerRunners`, `ManagedRunner`, the state mapping in `reconcile_pool`, and the `json!` request builder).
  - API: `crates/gridops-api/src/resources.rs` (`starts_belong_to_reconciler`, `provision` and its Bitbucket branch).
  - Branch state: `git diff master --stat -- crates`. The only crate commit on the branch is dd525d5 (#135), which is upstream and not in local `master`. No #137 code exists.
