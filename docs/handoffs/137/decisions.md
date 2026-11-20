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

## T (Phase 4, author tests / RED): 2026-11-20T13:10:01-08:00
- **Decided:** RED is valid, and F may start. I authored 40 new or extended tests, plus 2 guards that are green by design, across 11 files.
  - Runtime RED: 6 in `fleet_schema.rs`, 1 in `session.rs` and 2 in `openapi.rs`. All of them fail on the missing feature: no `gitlab_connections`, no `0046_gitlab.sql`, no added columns, `glrt-` not masked, and no GitLab paths in the spec.
  - Compile RED: `gitlab_contract.rs`, `gitlab/fake_tests.rs`, `admission_tests.rs`, `provisioning.rs`, `fleet_agent_outcomes.rs`, the manager and the Tart agent. Every error names a missing contract item.
  - Placement:
    - The fake tests are a sibling file that F mounts from `fake.rs` (the `admission_tests.rs` precedent). As an integration test, a `cfg(feature = "test-support")` fake would be skipped silently by plain `cargo test`.
    - The `gitlab/mod.rs` unit tests the brief places in that file live in `tests/gitlab_contract.rs`, because they only touch public API.
    - The GitLab `PrepareEnvironment` test also lives in `gitlab_contract.rs`, so the two guards in `fleet_agent_commands.rs` keep compiling and running.
  - Deviation: the definition-unchanged check also exempts `runner_pools`, because the brief's own `ALTER TABLE runner_pools ADD COLUMN` rewrites its `sqlite_master` text. The brief's exemption list ("only runners and the runners_* indexes") can't hold as written. In its place, the test asserts that every column of every table is unchanged, and checks each added column (exactly six) on its own. This is stricter than the brief, not looser.
- **Assumed:**
  - Where the brief is silent, the fake's setup methods and `fail_next` take `&self`.
  - The `FakeMethod` variants are named after the trait methods (`CreateRunner` … `VerifyAccess`).
  - `GitLabError`, `FakeCall` and `FakeMethod` derive `PartialEq` and `Debug`. That is needed for the brief's own `assert_eq!` on the whole `calls()` vector.
  - A call that fails through `fail_next` has no side effect.
  - The rebuild keeps each existing `runners` column's declared type and default text.
  All of these are listed in `handoff-T-red.md` "Ready for F".
- **Hedged:**
  - Two tests are deliberately green before the feature: the GitHub `PrepareEnvironment` master-bytes test and the `ExecutionConfiguration` field-set test. They pin the "unchanged" criteria D1 and B-7, and are labelled as guards, not RED.
  - What these tests deliberately leave unpinned: the `verify_access` and `group_projects` answers for unseeded scopes and groups, `job` lookups across projects, and whether a failed `create_runner` uses up an id.
  - A's round-3 warning 2 (relaying `exitReason` through the manager's `ManagedRunner`) has no test, because the brief doesn't adopt it.
- **Evidence:**
  - Brief: `docs/handoffs/137-brief.md` (all 1010 lines) and `handoff-A.md` round 3.
  - Source: `fleet/{domain,capabilities,targets,admission,admission_tests}.rs`, `protocol/{agent,outcomes}.rs`, `ids.rs` (`UpstreamText`), `provisioning.rs`, the manager and Tart `ProvisionRunner`/`validate` and test fixtures, `gridops-agent/src/session.rs` and `gridops-api/src/fleet/openapi.rs`.
  - Migrations: 0001 to 0045 applied to a scratch `sqlite3` database, which gave the exact `pragma_table_info`, `index_list` and `foreign_key_list` of `runners` and `fleet_ci_targets`.
  - The master bytes of the GitHub fixture, re-captured and equal to the brief's literal.
  - A throwaway probe of today's `for_selected_provider` (8 GitHub and Bitbucket rows matched).
  - A throwaway draft 0046 (never staged): 14/14 passed, and the mutation without transaction control failed 3, catching the `runner_events` cascade.

## F (Phase 5, implementation): 2026-11-20T13:30:41-08:00
- **Decided:** Implemented the contract against T's RED suite. Every T-authored test is GREEN. One pre-existing test, `db::tests::fresh_database_applies_every_migration`, hard-codes 70 tables and now sees 73, because 0046 adds `gitlab_connections`, `runner_pool_gitlab_connections` and `gitlab_jobs`. It is flagged for T and not edited.
  - I did not take a scope-cap split. This touches more than 6 files across several surfaces, but it is the reviewed types-and-stubs contract with a single RED suite, so no sub-phase could go GREEN alone.
  - Extended the existing objects: `ensure_target`, `target_from_db`, `TargetKind`, `CiTarget`, both `ProvisionRunner` structs and `SECRET_PREFIXES`. The `gitlab` module is new because the brief places it there (no prior GitLab code exists).
  - `ensure_target`: the GitLab arms insert through a private `ensure_gitlab_target`. The final `SELECT id` moved into a private `target_id_for_key`, shared by both paths. The Bitbucket wildcard and its error string are unchanged.
  - The manager's new fields use `skip_serializing_if = "Option::is_none"`, so the bytes forwarded to the Tart agent for GitHub and Bitbucket starts are unchanged.
  - The Tart agent's new fields, `GitlabBootstrap` and its exit-code copy are read only by #142, so they carry `#[allow(dead_code, reason = ...)]`.
  - `FakeGitLab::job` matches the project id as well as the job id, as GitLab's `GET /projects/:id/jobs/:job_id` does. `verify_access` is `Ok` for a seeded scope of the same kind and id, and `NotFound` otherwise. A failed call never uses up a runner id.
- **Assumed:** The OpenAPI 403 `token_not_accepted` refusal of bearer tokens is stated in the tag description and not declared as a response. Its body is not `ApiErrorBody`, and T's test forbids any other error schema on these operations.
- **Hedged:** The 0046 guard checks `pragma_foreign_key_check` over the whole database, as the brief pins. So a production database that already holds a dangling reference anywhere would refuse to migrate rather than commit. This is the safe failure, and it is recorded under Known issues in `handoff-F.md`.
  - 11 `gridops-manager` tests fail on this macOS workstation. Their Unix socket paths under `temp_dir()` exceed `SUN_LEN`. Those modules are untouched, and CI is Linux.
- **Evidence:**
  - Tests: `cargo test` for `gridops-core` (lib, `fleet_schema`, `gitlab_contract`, `fleet_agent_outcomes`, `fleet_agent_commands`), `gridops-agent`, `gridops-api --bin`, `gridops-manager` and `gridops-tart-agent`.
  - Lint and build: `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (clean), the same for `gridops-core --features test-support`, `cargo fmt --all -- --check`, `cargo build --workspace --locked`, and `npx @redocly/cli@2.72.1 lint` (valid; only the operationId warnings the spec already has).
  - 0046 applied with `sqlite3` 3.54.0 to a scratch database built from 0001 to 0045: integrity `ok` and an empty FK check.
  - Source facts are in `evidence.md`.
