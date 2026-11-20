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
