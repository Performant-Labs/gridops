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

### D17 (decided 2026-11-20, recorded on #136): `GitLabApi::list_runners` (architecture gate R1, finding 1)

The orphan sweep needs a listing the shipped trait lacked. Contract change D17 (https://github.com/Performant-Labs/gridops/issues/136#issuecomment-7053069685) gives #139 this, in `crates/gridops-core/src/gitlab/`:

    async fn list_runners(&self, credentials: &GitLabCredentials, scope: &Scope) -> Result<Vec<RunnerSummary>, GitLabError>;
    pub struct RunnerSummary { pub id: u64, pub description: String }

- Backed (in #146's HTTP client) by `GET /projects/:id/runners?type=project_type` or `GET /groups/:id/runners?type=group_type`, paginated. The `type` filter keeps shared and instance runners out.
- #139 adds the trait method, `RunnerSummary` (re-exported next to `CreatedRunner`, with `Debug, Clone, PartialEq, Eq`), and `FakeGitLab` support: `list_runners` returns the fake's runners created on that base URL for that scope, honours `fail_next`, and is recorded in `calls()` as a new `FakeMethod` variant. A test-only way to seed a runner the fake did not create (for "never touch a runner without the marker") is added to the fake. Existing fake tests stay green.
- #146 implements it on `GitLabClient` (inherent method until #139 merges; the PR merging second wires the trait impl). #139 does **not** touch #146's client; any placeholder or other `GitLabApi` impl in the tree on master at the time gets the method (returning its existing error).
- `RunnerSummary.id` stays `u64` as published in D17 (#146 implements that exact signature in parallel; changing it now would desync the two PRs). The conversion happens in exactly one place, the sweep, with `ExternalId::try_from`; an id that does not convert is logged and skipped (round 2, warning 5). The fake's seeding method is named `with_runner`, matching `with_scope` and `with_job`.
- **Description marker** (amended in round 2, warning 7). `create_runner`'s description is exactly `gridops-runner:<instance>:<runner uuid>`, where `<instance>` is the host (and `:port` if non-default) of this deployment's `config.base_url` (`GRIDOPS_BASE_URL`), and `<runner uuid>` is `runners.id`. The sweep parses only that exact form and deletes only runners whose `<instance>` equals its own, so two GridOps deployments on one GitLab scope never delete each other's runners. A description that does not parse, or names another instance, is never deleted.
- **Sweep** (amended in round 2, block 3). For **every** `gitlab_connections` row (a connection whose only pool was deleted has no membership or runner rows left, because `delete_pool` hard-deletes `runner_pools` and both `runner_pool_gitlab_connections` and `runners` cascade), list the connection's scope's runners; delete each runner whose description parses to a GridOps runner id that has **no live row** (no `runners` row, or a row with `deleted_at` set / status `deleted`). Rows still `starting` are live (do not race a start). Cadence (round 2, warning 6): follow `maybe_sync_github` — a `gitlabRunnerSweepIntervalSeconds` setting (default 600, clamped 60..=3600) and a `lastGitlabRunnerSweepAt` settings row (the `settings` key-value table; no migration). It is called from `reconcile()` next to `retry_github_runner_cleanup`, inside the usual warn-on-error wrapper; its errors (listing or delete, per connection) are logged and never fail the pass, and one connection's error does not stop the others.
- The sweep is also the retry for a failed `delete_runner` on any exit path: a delete that errors is logged and the row still leaves service; the sweep removes the GitLab runner later. A failed GitLab delete **never** fails the pool pass (the 08c32b3 wedge).

### Pool delete and API runner delete (finding 2)

`delete_runner_pool` and the runner delete/rebuild actions go through the API's `delete_runner_resources` (`gridops-api/src/resources.rs`), which the reconciler never sees. Per D17, **#138 owns** the GitLab arm there (call `delete_runner` when it can, otherwise leave the GitLab runner for the sweep). #139 does not edit `resources.rs`. The sweep is #139's coverage for these paths: a GitLab runner whose row was hard-deleted with its pool is removed by the sweep. Done-when for "pool delete" is therefore: a sweep test where the runner row (and pool) is gone and the GitLab runner is deleted.

### One ordering per path (finding 3)

Replace "in the same insert" with:

- **Manager path** (`provision_on_host` → `start_admitted_runner`): the INSERT writes `ci_platform='gitlab'` and `gitlab_connection_id` (as Bitbucket writes `bitbucket_connection_id`). `create_runner` runs inside `start_admitted_runner` at the point where the Bitbucket arm registers; `gitlab_runner_id` is set by the UPDATE that follows, as `bitbucket_runner_uuid` is. If the manager `POST /v1/runners` (or anything after `create_runner`) fails, `failure_cleanup(true, true, …)` drives `delete_runner` on GitLab, the capacity lease is released, the row is marked failed, and `StartOutcome::Failed` counts the failure through `record_provision_failure`, exactly as the Bitbucket arm does. The `glrt-` token goes in `gitlabRunnerToken`, never logged.
- **Fleet path** (`fleet::provision` → `register_and_start`): `fleet::provision` inserts the row with `ci_platform='gitlab'` and `gitlab_connection_id`; `create_runner` runs in `register_and_start` where `github_registration` is built; `gitlab_runner_id` is set by UPDATE; the material is the typed `GitlabRunnerMaterial::GitlabRunner { token, url, wait_timeout_seconds }` serialized with serde (not hand-written `json!`), stored with `placement::store_bootstrap`. If storing the bootstrap or issuing the start command fails after `create_runner`, the GitLab runner is deleted (best effort; the sweep is the backstop).
- `fleet::provision` must derive a GitLab runner's fleet target from the **selected connection** (`Scope::target(connection_id)`, or the `fleet_ci_targets` row for that connection), not `pool_targets().next()` (warning 6).

### The seam: where the code goes (finding 4)

`gitlab_lifecycle.rs` holds the GitLab-specific helpers, the counterparts of `next_bitbucket_connection`, `bitbucket_runner_labels`, `deregister_bitbucket_runner` and the Bitbucket start arm: connection selection, credentials, tags/description, register, deregister, the `exitReason` / diagnostic mapping, the orphan sweep, the placeholder client. It does **not** become a parallel provision/start/delete pipeline. The existing shared functions gain GitLab arms that call into it:

- `main.rs`: the `Reconciler` struct (`gitlab` field) and its constructors (production and tests); the `Runner` struct and the `runners()` SELECT (`gitlab_connection_id`, `gitlab_runner_id`); `ManagedRunner` / `ManagerView` (read `exitReason`, `#[serde(default)]`); `reconcile_pool`'s exited→failed mapping (GitLab `completed` / `idle_expired` → removed without error, so `min_count` replaces it; `registration_refused` → failed with a readable diagnostic, counted through `record_provision_failure` so a refused token does not create/start/delete every pass (warning 7)); `provision_on_host` and `AdmittedStart` (GitLab connection, `CiPlatform::Gitlab` in `for_selected_provider`); `start_admitted_runner` (GitLab arm); `delete_runner`'s dispatch (today any non-Bitbucket runner goes down the GitHub branch at ~`main.rs:2811`; a GitLab runner must go to the GitLab deregister instead); the sweep's call site.
- `fleet.rs`: `provision`, `register_and_start`, `follow`, `finish` gain GitLab arms (D5: branch on `diagnostic.code`).
- `crates/gridops-core/src/fleet/placement.rs`: a reader that returns the newest result's `DiagnosticCode` (next to `reported_gone`, whose signature and behaviour stay unchanged).
- `crates/gridops-core/src/fleet/service.rs` and `targets.rs`: the GitLab arms of `ensure_target_identity_for_pool` and `link_pool_target`, and the GitLab pool-target sync (extend `pool_targets` and the `sync_github_targets` call sites, or a `sync_gitlab_targets` beside it) (D6, D11).
- `crates/gridops-reconciler/Cargo.toml`: `gridops-core` `test-support` feature for tests.
- `start_tests.rs`: fixtures only (an `exitReason` setting on the stand-in manager, a `gitlab` fake on `Setup`). GitLab tests build on `Setup` and `ManagerLog`, as `tart_outage_tests.rs` does, in a new `gitlab_lifecycle_tests.rs` (or `#[cfg(test)]` in the module) (warning 9).
- The placeholder `GitLabApi` is a private named type in `gitlab_lifecycle.rs` (warning 8).

### Other rules

- Tags: the pool's labels (`runner_pools.labels` JSON); `run_untagged` from `runner_pools.run_untagged`.
- Connection choice mirrors `next_bitbucket_connection`: a `runner_pool_gitlab_connections` membership in `created_at, connection_id` order. GitLab runners are ephemeral, so unlike Bitbucket the selection must not cap at one live runner per connection; it picks deterministically (e.g. the connection with fewest live GitLab runners, ties by `created_at, connection_id`) and allows `min_count` / demand above 1.

### Done when (amended)

Tests with `FakeGitLab` (and the stand-in manager / fleet fixtures) cover: register on the manager path and on the fleet path (tags, `run_untagged`, description marker, row columns, request/material shape, token never in logs or row); rollback after a failed start on both paths (no GitLab runner left, capacity released, pool not wedged, failure counted); cleanup through `delete_runner` on each reconciler exit path (ephemeral `completed` exit, `IdleExpired` on both paths, drain); a failed GitLab delete that does not fail the pass; the orphan sweep (deletes a marked runner with no live row, including after a pool delete; keeps a live/`starting` one; never touches an unmarked one; a listing error does not fail the pass); `RegistrationRefused` on both paths (readable diagnostic, counted failure); admission (`ensure_target_identity_for_pool` / `link_pool_target`) for GitLab targets; the `FakeGitLab` `list_runners` support. `cargo build`, `cargo clippy --all-targets -- -D warnings` and the full test suite pass.

### Round-2 amendments (architecture gate R2)

- **Block 1, the failure circuit for `RegistrationRefused`.** A pool's provision-failure count is reset only on **proof that registration worked**, never at manager-POST time for GitLab:
  - Manager path: `start_admitted_runner`'s reset after a successful `POST /v1/runners` (~`main.rs:2766`) is skipped for a GitLab start. The reset happens when the GitLab runner shows registration worked: it is reported busy, or it exits with `exitReason` `completed` or `idle_expired`. `registration_refused` counts through `record_provision_failure`. So a refused token opens the pool's circuit after the existing threshold instead of cycling 0 → 1 → 0.
  - Fleet path: a GitLab `RegistrationRefused` (on Start or Inspect) counts through the same `record_provision_failure`, and a GitLab fleet runner that goes busy or exits `completed`/`IdleExpired` resets it, so a machines-only pool is not left "blocked" forever after the problem is fixed.
  - Both paths call one shared reset helper (in `gitlab_lifecycle.rs`, wrapping the existing reset query) and the existing `record_provision_failure`. GitHub and Bitbucket keep today's reset behaviour unchanged.
  - Done-when adds: a refused token on the manager path reaches the circuit threshold (no reset in between); after a later successful GitLab run the count is reset; the same on the fleet path.
- **Block 2, the image substitution point (D147-1).** The single substitution point lives in core `crates/gridops-core/src/fleet/targets.rs` (a GitLab arm, which is #139's there): a `pub fn` that, for a pool with `runner_pool_gitlab_connections` rows and an empty `docker_image`, gives the GitLab runner image instead of the GitHub `STOCK_DOCKER_IMAGE`. `pool_docker_image` (used by `save_pool_fleet`) and the reconciler's manager path (`provision_on_host`'s image for a GitLab start) both call it; `gitlab_lifecycle.rs` holds no second copy. Until #147's `GITLAB_RUNNER_IMAGE` is on master, the empty-image GitLab case is a readable error ("GitLab pool has no runner image") — `save_pool_fleet` refuses to save that profile with `FleetError::InvalidRequest`, and a manager-path start is refused before `create_runner` — and a non-empty pool image is used as given. Whichever of #139/#147 merges second switches the empty case to `GITLAB_RUNNER_IMAGE` (one line), as agreed with #147. GitHub and Bitbucket pools keep today's fallback.
- **Block 4, retried fleet starts.** `crates/gridops-core/src/fleet/retry.rs::retry_failed_start` rebuilds the runner row with `INSERT…SELECT`; it must also copy `gitlab_connection_id` (never `gitlab_runner_id`: the retried runner registers afresh). `retry.rs` joins the seam and Owns (that column only). Done-when adds: a retried GitLab fleet start registers a new GitLab runner on the same connection and the old one is deregistered (or left to the sweep).
- **Warning 8, a failed fleet `create_runner`.** In `register_and_start`, if `create_runner` fails: release the placement with `placement::abandon_unstarted` and mark the row failed (counted, per block 1 when the error is a refusal; transient `RateLimited`/`Upstream` follow the existing start-failure handling), so the placement does not sit "registering", hold machine capacity and retry every tick. Before registering, if the row already has a `gitlab_runner_id` (a repeated attempt), delete that GitLab runner first (best effort; the sweep is the backstop).
- **Warning 9.** `crates/gridops-reconciler/src/builtin_host.rs`'s test fixture `Runner` literal gains the new fields (fixtures only).
- **Block 3** is in the sweep bullet above (every `gitlab_connections` row); Done-when adds "the only pool of a connection was deleted, and its marked GitLab runner is swept".

### Host-agent facts from #147 (sibling notes, 2026-11-20)

- **Outcome branching on the fleet path (refines D5).** `RegistrationRefused` arrives on the **Start** result (`Failed { Configuration, retryable: false, diagnostic: RegistrationRefused }`). `IdleExpired`, and a late `RegistrationRefused`, arrive on **Inspect** (`Failed { ResourceUnavailable, retryable: false, diagnostic: … }`), so `reported_gone` still cleans the runtime up; #139 reads the diagnostic to choose: Inspect `IdleExpired` → removed without error; Inspect `RegistrationRefused` (the runner exited 77 after Start had succeeded) → **failed, counted** through `record_provision_failure`, exactly like a Start-time refusal. A **completed** GitLab runner is Inspect `ResourceUnavailable` with **no** diagnostic: removed without error. Manager path: `exitReason` as in D10.
- **D147-1, the runner image.** A GitLab pool's `docker_image` is the gitlab-runner image on both paths; #147 adds `gridops_core::provisioning::GITLAB_RUNNER_IMAGE`. Where #139 substitutes it is set by round-2 block 2 above (core `fleet/targets.rs`), which replaces the earlier `gitlab_lifecycle.rs` helper wording.
- **Docker socket.** The host agent refuses a GitLab Prepare whose profile lacks `requires_docker_socket` (`Configuration`). Setting it when a GitLab pool's fleet profile is saved belongs to the profile-save path (#138/#141). #139 must not wedge on it: a GitLab fleet start on a profile without `requires_docker_socket` is refused before `create_runner` with a readable reason (no GitLab runner created), and the Prepare refusal, if it happens anyway, is a counted failure.

### Don't touch

Shared contract types beyond D17's addition (`gridops-core/src/gitlab/mod.rs` and `gitlab/fake.rs` except `list_runners`/`RunnerSummary`, `fleet/domain.rs`, the agent protocol types, `provisioning.rs`, migrations), the API (`gridops-api`, #138/#140), the manager and agents (#147/#142), the UI (#141), and #148's `gitlab_poll.rs`. If the contract truly needs a change, stop and report it rather than changing it. Mars production is deployed separately; deploy nothing.
