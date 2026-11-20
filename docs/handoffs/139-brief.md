# [GitLab: reconciler runner lifecycle (register, roll back, clean up)](https://github.com/Performant-Labs/gridops/issues/139)

Part of #136. **Wave 1.** Depends only on #137. Tests use `FakeGitLab`.

## Need

When a GitLab pool needs a runner, the reconciler must register it with GitLab, hand the agent the registration, and remove it from GitLab when it leaves service. This holds for any provider (Docker or Tart). The agent side lives in #147 (Linux) and #142 (macOS).

## Scope

In `crates/gridops-reconciler/src/` (put the new code in a `gitlab_lifecycle.rs` module and call it from `main.rs`):

- **Starting a runner:** call `create_runner` with the pool's labels as tags, the pool's `run_untagged`, and a description that names the GridOps runner ID. Store `ci_platform='gitlab'`, `gitlab_connection_id` and `gitlab_runner_id` in the same insert. Send `PrepareEnvironment` with the GitLab `CiTarget`, the base URL in `configuration`, and the token as the stdin registration material. Pick connections the way `next_bitbucket_connection` does.
- **Rollback:** if `create_runner` succeeds but the start fails, delete the GitLab runner. A failed registration must never wedge the pool or leak a GitLab runner (compare 08c32b3 and #130).
- **Cleanup:** call `delete_runner` when a GitLab runner row leaves service, whether by ephemeral exit, `IdleExpired`, drain or pool delete. Also sweep GitLab runners whose description names a GridOps runner that no longer exists.
- **Outcomes:** `IdleExpired` → removed without error, so the reconciler replaces it if the pool's `min_count` needs one. `RegistrationRefused` → a failure with a readable diagnostic.

**Owns:** `crates/gridops-reconciler/src/gitlab_lifecycle.rs` and its tests, plus the call sites in `main.rs`. #148 adds its own module next to this one. Neither issue edits the other's module.

## Done when

- Tests in the `start_tests.rs` style cover register, rollback after a failed start, cleanup on each exit path, the orphan sweep and `RegistrationRefused`.

## Clarifications (orchestrator, before the brief gate)

Everything above this heading is the issue text and is unchanged. This section records the contract as #137 actually shipped it (master `b22c451`, PR #150). The epic #136's "Contract deviations shipped by #137" table (its latest comment) replaces the issue text where they differ; the full rationale is `docs/handoffs/137-brief.md`. This section removes no scope.

### Contract deviations that apply here

- **D1.** No `ExecutionConfiguration` change. On the **fleet path** (a fleet host agent, #147) the base URL travels as `url` in the sealed `GitlabRunnerMaterial` (`crates/gridops-core/src/fleet/protocol/agent.rs`), wire `{"kind":"gitlab_runner","token","url","wait_timeout_seconds"}`, stored with `placement::store_bootstrap` exactly where `crates/gridops-reconciler/src/fleet.rs` stores today's `github_registration` material. "The base URL in `configuration`" in the issue text is replaced by this.
- **D2.** Every `GitLabApi` method takes `&GitLabCredentials { base_url, access_token }` first. One `Arc<dyn GitLabApi>` serves every connection; build the credentials per call from the connection's `base_url` and its access token, read from the vault by `gitlab_connections.access_token_key` (as Bitbucket's `runtime_secret(app, &connection.access_token_key)` does).
- **D5.** Fleet path outcomes: `DiagnosticCode::IdleExpired` arrives inside `Failed { code: ResourceUnavailable, retryable: false, diagnostic }` on Inspect; `DiagnosticCode::RegistrationRefused` inside `Failed { code: Configuration, retryable: false, diagnostic }` on the start command. Branch on `diagnostic.code`, never on the `FailureCode` alone.
- **D6.** Pool-target sync for GitLab is #139's: the GitLab counterpart of `sync_github_targets` / `pool_targets` in `crates/gridops-core/src/fleet/targets.rs`, so a pool with `runner_pool_gitlab_connections` rows gets `gitlab_project` / `gitlab_group` fleet targets (`ensure_target` already has the GitLab arms).
- **D7.** The `runners` indexes are `runners_gitlab_connection_idx` and `runners_gitlab_runner_id_idx` (already in 0047).
- **D9.** `ProvisioningPlan::for_selected_provider(provider, platform: CiPlatform, …)`. Pass `CiPlatform::Gitlab` for a GitLab start: mode `ephemeral`, `ephemeral: true`, runtime `vm`.
- **D10.** On the **runner-manager path** (built-in Docker host and Tart, both via `POST /v1/runners` from `start_admitted_runner` in `main.rs`), a GitLab request carries `"platform":"gitlab"`, `"mode":"ephemeral"`, `"runtime":"vm"`, `gitlab: {"url": <base_url>, "waitTimeoutSeconds": <secs>}` and the secret `gitlabRunnerToken` (the `glrt-` token), and no `jitConfig`, `registrationToken`, `bitbucket` or `bitbucketOauthClientSecret`. The listing (`GET /v1/runners`) reports `exitReason` (`completed` / `idle_expired` / `registration_refused`, `GITLAB_EXIT_REASONS` in `crates/gridops-core/src/provisioning.rs`) for an exited GitLab runner; #139 reads and maps it: `idle_expired` and `completed` → removed without error; `registration_refused` → a failure with a readable diagnostic. The manager/Tart side that produces `exitReason` is #147 / #142 and is not on master; test the reconciler against a stubbed manager response, as the existing `start_tests.rs` does.
- **D11.** Admission is #139's: the GitLab arms of `ensure_target_identity_for_pool` (`crates/gridops-core/src/fleet/service.rs`, today `_ => false`) and of `link_pool_target` (`crates/gridops-core/src/fleet/targets.rs`, today GitHub-repository only). A `gitlab_project` / `gitlab_group` target is valid for a pool when that pool has a `runner_pool_gitlab_connections` row for the target's `gitlab_connection_id`. User-scoped GitLab target visibility is deferred (admins only).
- **D12.** The idle timeout is `wait_timeout_seconds` / `waitTimeoutSeconds`, `GITLAB_DEFAULT_WAIT_TIMEOUT_SECS = 600`. Pools have no timeout column, so send the default.
- **D14.** The API never starts GitLab runners (#138 makes `starts_belong_to_reconciler` true for GitLab pools). GitLab starts live only in the reconciler; the GitLab registration lifecycle lives only in `gitlab_lifecycle.rs`. Choosing `CiPlatform::Gitlab` in the reconciler's `for_selected_provider` caller is #139's; the API's caller is not touched.
- **D15.** The manager relays a Tart agent's `exitReason` (#147 adds that field). #139 only reads `exitReason` from the manager listing.
- **D16.** The migration is `migrations/0047_gitlab.sql`. Add no migration; if a schema gap is found, stop and report it.

### The reconciler's `gitlab` client (R2-W3 of `137-brief.md`)

- `Reconciler` (`crates/gridops-reconciler/src/main.rs`) gains `gitlab: Arc<dyn GitLabApi>`, next to `bitbucket`. **#139 owns the field.** #148 reads `app.gitlab`; if #148 lands first it adds the same field with the same name and type, and whichever lands second rebases onto it.
- The real HTTP client is #146's, in parallel, not on master. Until it lands, production wires a small placeholder implementation that returns `GitLabError::Upstream("GitLab client not available yet")` from every method, so the reconciler builds and runs; #146 replaces that one line. Tests inject `gridops_core::gitlab::fake::FakeGitLab` (feature `test-support`, as a dev-dependency) as `Arc<dyn GitLabApi>`.
- Production must not start a GitLab runner it cannot register: with the placeholder, `create_runner` fails, the start fails cleanly (no row left `starting`, capacity released, no GitLab runner leaked), and the pool is not wedged.

### Where the code goes

- New behaviour in `crates/gridops-reconciler/src/gitlab_lifecycle.rs` (connection selection, tags/description, register, rollback, cleanup on every exit path, orphan sweep, `exitReason` / diagnostic mapping) with its tests; `main.rs` gets only the call sites (and the `gitlab` field). Do not edit #148's `gitlab_poll.rs`.
- The description names the GridOps runner ID in a form the orphan sweep can parse back exactly (e.g. `gridops runner <uuid>`); the sweep deletes only GitLab runners whose description parses to a GridOps runner ID that has no live `runners` row, and never touches runners GridOps did not create.
- Tags: the pool's labels (`runner_pools.labels` JSON); `run_untagged` from `runner_pools.run_untagged`.
- The insert stores `ci_platform='gitlab'`, `gitlab_connection_id` and `gitlab_runner_id` in the same statement as the row (so `create_runner` runs before the insert, and a failed insert also rolls the GitLab runner back).
- Connection choice mirrors `next_bitbucket_connection`: a `runner_pool_gitlab_connections` membership in `created_at, connection_id` order. GitLab runners are ephemeral, so unlike Bitbucket the selection must not cap at one live runner per connection; it must still pick deterministically and allow `min_count` / demand above 1.
- The GitLab arms in `crates/gridops-core/src/fleet/service.rs` and `targets.rs` (D6, D11) are #139's; touch nothing else in those files.

### Don't touch

Shared contract types (`gridops-core/src/gitlab/mod.rs`, `gitlab/fake.rs`, `fleet/domain.rs`, the agent protocol types, `provisioning.rs`, migrations), the API (`gridops-api`, #138/#140), the manager and agents (#147/#142), the UI (#141), and #148's `gitlab_poll.rs`. If the contract truly needs a change, stop and report it rather than changing it. Mars production is deployed separately; deploy nothing.
