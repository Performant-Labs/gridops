# Handoff-A: Phase 3 - GitLab shared contract (#137)  (up-front plan review, round 2)

**Date:** 2026-11-20
**Branch:** claude/gitlab-137-contract-fa3bbe (head e0e42e1)
**Brief reviewed:** docs/handoffs/137-brief.md   **Reuse map:** N/A. No `survey.md` was provided, so the brief's own analogues (the Bitbucket connection, client and runner columns; the GitHub fleet path) are the baseline.   **Wireframe:** N/A (no UI surface)
**Verdict:** BLOCK

## Summary

BLOCK, on one new finding. The amendments resolve all three round-1 blocks and all eight round-1 warnings, and they fit the code. The new block is that the contract pins the agent protocol only for the fleet host-agent path, and macOS/Tart runners (#142) never use that path:

- The host agent can't run Tart VMs. `HostRuntime::call` answers `Unsupported` for `TartVm`, and `backends()` advertises Docker and native only.
- Tart runners start through the runner-manager path. The reconciler's `start` sends a `POST /v1/runners` to `gridops-manager`, which forwards it to `gridops-tart-agent`. The body is camelCase `ProvisionRunner` JSON whose platform comes from `ProvisioningPlan::for_selected_provider`. Exits come back through the `GET /v1/runners` listing.
- The brief pins none of this for GitLab.
- The `GitlabRunnerMaterial` doc comment and W-4 name #142 as a consumer of sealed bootstrap material and of `CommandOutcome` diagnostics, but the Tart agent receives neither and doesn't depend on `gridops-core`.

My round-1 review assumed (in `decisions.md`) that #142 went through fleet placement. That assumption was wrong, and this finding corrects it.

### Resolved findings

- **[R-1]** `CiTarget` GitLab variants had no storage in `fleet_ci_targets`. **Resolved** by A-1 option (a), which covers:
  - the rebuild, with the kinds, columns, shape CHECK, trigger and the seven inbound FKs;
  - `TargetKind::Gitlab*` and the canonical keys;
  - the `ensure_target` and `target_from_db` arms;
  - the `fleet_ready.sql` upgrade test.
- **[R-2]** The base URL was planned for `ExecutionConfiguration`. **Resolved** for the fleet path by A-2:
  - `GitlabRunnerMaterial` `{"kind":"gitlab_runner","token","url"}` lives in `fleet/protocol/agent.rs`;
  - `ExecutionConfiguration` is left untouched;
  - a byte-identity test covers a GitHub `PrepareEnvironment`.
- **[R-3]** `GitLabApi` had no per-connection credentials and `Scope` was undefined. **Resolved** by A-3:
  - every method takes `&GitLabCredentials` first;
  - `Scope` and `ScopeKind` are defined;
  - one `Arc<dyn GitLabApi>` serves every connection;
  - `FakeGitLab` is keyed by `base_url`.
- **Round-1 warnings 4 to 11.** All resolved:
  - W-4: `DiagnosticCode`, so `reported_gone` is unchanged.
  - W-5/W-6: the constraints are pinned, the 0043-style open-jobs index is used, and `runners` gets two indexes.
  - W-7: the OpenAPI auth, envelope and `ApiErrorBody` are pinned.
  - W-8: the rebuild order and FK guard are pinned. I checked that sqlx 0.9.0 honours the directive only when the file begins with it (`sqlx-core-0.9.0/src/migrate/source.rs:234`, `sql.starts_with("-- no-transaction")`). So `-- no-transaction` must be the first line of 0046, ahead of any header comment.
  - Types: `ExternalId`, `Option<Duration>`, `Vec<String>` and `JobStatus` are pinned.
  - `group_projects` is added.
  - The canonical keys and `TargetKind` are pinned.

These parts were also checked and fit the codebase:
- No schema CHECK constrains diagnostic codes.
- No trigger or view outside its own trigger references `fleet_ci_targets`.
- `POST /api/v1/webhooks/gitlab` mirrors the live `/api/v1/webhooks/github` (`gridops-api/src/main.rs:199`).
- The `GitlabConnection` JSON matches `bitbucket_connection_json`: `provider`, camelCase fields, and the `{canManage, items}` list envelope.

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | block | A-2 doc comment "opened by the host agents (#147 Linux Docker, #142 Tart/macOS)"; W-4 "pinned for #147, #142 and #139"; no runner-manager entries anywhere in the contract | pattern consistency / layering (the wire contract between parallel wave-1 issues) | **The host agent never handles Tart.** `crates/gridops-host-agent/src/runtime.rs:30-87` dispatches only `Docker` and `NativeProcess`, and `TartVm` gets `Failed { code: Unsupported }`. No fleet placement on a Mac reaches a host agent, so sealed bootstrap material is never opened there.<br>**The Tart path is the runner manager.**<br>- `gridops-reconciler/src/main.rs:2385-2660` builds the `ProvisionRunner` JSON. Its common fields are `runnerId`, `platform`, `mode`, `provider` and `runtime`. The per-platform secrets are `jitConfig`; `registrationToken` with `registrationUrl`; and `bitbucket {accountUuid, runnerUuid, oauthClientId}` with `bitbucketOauthClientSecret`. It sends the request to `manager_url()`.<br>- `gridops-manager/src/main.rs:323-480` has a private `ProvisionRunner` that matches on `(platform, mode)` and answers anything else with "Runner platform is invalid." (`:471`). It forwards Tart starts to `gridops-tart-agent`.<br>- `gridops-tart-agent/src/main.rs:250-370` has its own private struct with the same reject arm. Its `Cargo.toml` has no `gridops-core` dependency, so it can't use `GitlabRunnerMaterial` or `DiagnosticCode`.<br>**Exits.** They are reported as `state`/`status` strings in `GET /v1/runners` (`gridops-tart-agent/src/main.rs:660-718`), not as `CommandOutcome`s, so W-4's pin can't reach #142.<br>**Platform selection.** `ProvisioningPlan::for_selected_provider(provider, bitbucket_connection_selected: bool, ...)` (`gridops-core/src/provisioning.rs:17-59`) returns only `"github"` or `"bitbucket"`. Its module says it "owns the decisions that must not drift" between the API and the reconciler, and both call it (`gridops-api/src/resources.rs:4259` and `gridops-reconciler/src/main.rs:2404`). GitLab changes that signature.<br>**Effect.** #139 (producer), the manager (owned by no wave-1 issue) and #142 (consumer) would each invent the GitLab request fields and exit strings in parallel. As briefed, #142 would wait for a `PrepareEnvironment` that never arrives. This is the same class as round-1 finding 2, on the other start path. | Either (a) pin the runner-manager GitLab contract in #137, or (b) descope GitLab on Tart and on the built-in host in writing. Steps are in "Notes for O". |
| 2 | warn | A-1 "so a GitLab `CiTarget` can be stored, granted, admitted ..." | completeness (behaviour ownership) | `ensure_target_identity_for_pool` (`fleet/service.rs:736-807`) runs on every intent check (`:392`, `:420`, `:733`) and ends `_ => false`, so a GitLab intent gets `Forbidden`. As planned, GitLab targets are stored and loadable but never admitted. Three other places have no GitLab arm either:<br>- `link_pool_target` (`fleet/targets.rs`) is GitHub-only.<br>- The user-scoped SQL has none: `readable_targets_cte` (`registry/queries.rs:825-850`) and the admission user-authorization predicate in `admission.rs`.<br>- So only system admins and reconciler principals can see or admit GitLab targets.<br>That default is safe, but no issue is named for any of these arms. | Name #139, next to the pool-target sync, as owner of the GitLab arm of `ensure_target_identity_for_pool`. The arm joins `runner_pool_gitlab_connections` and `gitlab_connections` on `scope_kind`/`scope_id` against the target's ids. Record that user-scoped GitLab target visibility stays admin-only, with no `gitlab_user_grants`, until #138 (which owns "authorization") or a follow-up adds it. Reword "admitted" in A-1. |
| 3 | warn | A-1 rebuild of `fleet_ci_targets`; "Done-when, extended for A-1" | pattern consistency (rebuild completeness) | 0022 declares three outbound FKs on the table: `installation_id REFERENCES installations(id)`, `repository_id REFERENCES repositories(id)` and `bitbucket_connection_id REFERENCES bitbucket_connections(id)`. A-1 lists the inbound FKs and the new `gitlab_connection_id` FK, but not these three. The assertions for the exempted table check kinds, columns, the trigger and the CHECK, but not `pragma_foreign_key_list('fleet_ci_targets')`. A rebuild that drops one of them would still pass. For `runners`, the brief does list and assert its outbound FKs. | List the three FKs as "recreated unchanged". Assert that `pragma_foreign_key_list('fleet_ci_targets')` equals them plus `gitlab_connection_id -> gitlab_connections(id)`. |
| 4 | warn | A-3 "#138 injects it into `AppState`, and #139/#148 into the reconciler" | cross-cutting (shared seams, add-only merges) | `AppState` (`gridops-api/src/state.rs:15-40`) and `Reconciler` (`gridops-reconciler/src/main.rs:41-57`) each get one `gitlab` field and one constructor change. #140 also needs `state.gitlab`, because it calls `GitLabApi::job` for `tag_list`, but #138 owns `state.rs`. Two issues adding the same field don't make an add-only merge, and #148 versus #139 is the same situation for the reconciler. | Name one owner per struct: #138 for `AppState`, #139 for `Reconciler`. State that #140 and #148 rebase onto that field and don't add their own. |
| 5 | warn | W-7 `/api/v1/platform-connections/gitlab/{connectionId}` | naming | Every templated path in `docs/openapi/gridops-v1.json` uses a snake_case parameter (`host_id`, `pool_id`, `target_id`, `token_id` and so on). The live Bitbucket route is `/api/v1/platform-connections/bitbucket/{connection_id}` (`gridops-api/src/main.rs:365`). | Use `{connection_id}`. |

## Notes for O

To resolve finding 1, amend `docs/handoffs/137-brief.md` with one of these two options, and say which one it is.

**(a) Pin the runner-manager GitLab contract in #137.** This is the recommended option. The epic keeps #142 in wave 1 and says it "Depends only on #137 (the agent protocol)".

1. **`ProvisioningPlan::for_selected_provider`.** Replace `bitbucket_connection_selected: bool` with a selected-platform argument, either `fleet::CiPlatform` or a small enum in `provisioning.rs`. Pin the GitLab plan as `platform "gitlab"`, `mode "ephemeral"`, `ephemeral true`, and `runtime "vm"` on Tart. #137 makes this signature change and updates the two callers mechanically (`gridops-api/src/resources.rs:4259` and `gridops-reconciler/src/main.rs:2404`). The GitHub and Bitbucket results stay identical and the existing `provisioning.rs` tests stay green. Nothing selects GitLab yet, so no behaviour changes.
2. **The GitLab request body.** Pin the exact `POST /v1/runners` JSON for `platform: "gitlab"` that both `gridops-manager` and `gridops-tart-agent` accept, the way A-2 pins the fleet material. Mirror the Bitbucket precedent: a non-secret `gitlab: { url }` object (plus the idle wait, if the runner needs it, for example `waitTimeoutSeconds`) and a top-level secret, for example `gitlabRunnerToken` (the `glrt-` token), with `mode: "ephemeral"`.
3. **Exit reporting.** Pin how the Tart agent reports the two GitLab exits in `GET /v1/runners`, for example `state: "exited"` with `status: "idle_expired"` or `"registration_refused"`. State that #139 maps them to the same handling as the fleet `DiagnosticCode::{IdleExpired, RegistrationRefused}`.
4. **Manager ownership.** Name the owner of the `gridops-manager` GitLab arm, which accepts `platform: "gitlab"` for `provider: "tart"` and forwards it. No wave-1 issue owns that crate. Also state whether GitLab on the built-in Docker host (`provider: "docker"`, no fleet profile) is supported in wave 1, or refused with a readable error.
5. **Corrections.** Fix the `GitlabRunnerMaterial` doc comment and W-4's consumer list. Sealed material and `DiagnosticCode` outcomes belong to the fleet host-agent path (#147, and #139's fleet branch). #142 receives the runner-manager request from step 2. Add a deviation row, because this changes #142's "on a GitLab `PrepareEnvironment`" and #139's "Send `PrepareEnvironment` ... for any provider (Docker or Tart)". Repeat that row on #136.

**(b) Descope.** State in writing that GitLab in wave 1 runs only on fleet machines (Linux Docker through the host agent, #147), and that GitLab on Tart/macOS and on the built-in host is deferred. Re-scope or park #142. Running it on the fleet path would first need Tart support in the host agent, which `HostRuntime` doesn't have. Remove #142 from the doc comment and from W-4, and record this as a deviation row, both in the brief and on #136.

Warn findings 2 to 5 don't block. Amending them in the same pass is cheap.

## Patterns referenced

- `crates/gridops-host-agent/src/runtime.rs:30-87` (`HostRuntime`: Docker and native only; `TartVm` is `Unsupported`)
- `crates/gridops-reconciler/src/main.rs:2385-2660`, `crates/gridops-manager/src/main.rs:323-480,1602-1640` and `crates/gridops-tart-agent/src/main.rs:250-370,660-718` plus its `Cargo.toml` (the runner-manager start path and its private `ProvisionRunner` contracts)
- `crates/gridops-core/src/provisioning.rs:1-59` (`ProvisioningPlan`, the shared platform decision)
- `crates/gridops-core/src/fleet/service.rs:736-807`, `crates/gridops-core/src/fleet/targets.rs` and `crates/gridops-core/src/fleet/registry/queries.rs:825-850` (kind-specific admission and visibility arms)
- `migrations/0022_fleet_foundation.sql:81-97`, the path parameters in `docs/openapi/gridops-v1.json`, and `crates/gridops-api/src/{main.rs:360-368,state.rs:15-40}`
