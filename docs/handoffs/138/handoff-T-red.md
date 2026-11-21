# Handoff-T-red: Phase 4 - GitLab platform-connection and pool-target API (#138)

**Date:** 2026-11-20
**Branch:** claude/gitlab-138-api-44d3a8 (head f2032e7)
**Brief / wireframe reviewed:** `docs/handoffs/138-brief.md` (with its Clarifications), `docs/handoffs/138/handoff-A.md` (round 3) and `docs/handoffs/138/decisions.md`. There is no wireframe because #138 has no UI surface.

## A precondition

Confirmed: A returned **PASS** on the plan in round 3, with 0 block findings and 5 warnings. The brief was not amended after round 3, so this suite applies A's warnings as follows:

- **W1, the fleet access tests.** I placed them where they can observe the decision. See the deviation below.
- **W3, `runtime_changed` and the pool audit fields.** The tests pin both. They mirror Bitbucket and are flagged as assumed.
- **W4, `runnerGroupId`.** Bodies send a positive value. The tests assert that 1 is stored.
- **W5, the keep-list.** No test constrains file layout.
- **W2** is a hand-off note and needs no test.

**Deviation from A's W1.** A suggested driving `ensure_target_access` and `ensure_pool_access` through `FleetService::submit_intent` in `admission_tests.rs`. That path can't show the decision:

- `submit_intent_in` runs `ensure_target_identity_for_pool` first (via `ensure_target_identity`, `service.rs:250`). Its match ends in `_ => false`, so it refuses every GitLab target until #139 adds the GitLab arm.
- `load_operation` and the recheck run it in `ensure_current_intent_policy` (`service.rs:420`).

So the two tests sit in a test-only child module of `service`, which can call the private functions directly. No visibility is widened.

**A's assumption that `:931` panics does not hold.** On the branch, `ensure_target_access` for `Principal::Reconciler` on a GitLab target returns `Ok` without panicking. sqlx decodes the NULL `bitbucket_connection_id` as a value. The real defects that are observable today:

- a system administrator is refused;
- an installation-admin member passes `ensure_pool_access`.

## Tests authored

| # | File | Test | What it pins (brief item) | Tier and why |
|---|---|---|---|---|
| 1 | `crates/gridops-api/src/resources/gitlab_tests.rs` | `creating_a_connection_resolves_the_scope_stores_both_secrets_and_shows_the_secret_once` | **Create:** `resolve_scope` then `verify_access`, both on the connection's instance. The base URL is normalised (no trailing slash). The token and a random webhook secret are sealed in `runtime_secrets` under the row's two keys. **Response:** 201 `GitlabConnectionCreated` with `webhookUrl = <base>/api/v1/webhooks/gitlab`. **Audit:** the create is audited. **List:** GET returns the same connection without either secret. | In-crate HTTP with `FakeGitLab`. This is the cheapest tier that reaches the handler with an injected GitLab, per the brief's test seam. |
| 2 | same | `a_connection_defaults_to_gitlab_com_and_can_be_a_group` | `baseUrl` defaults to `https://gitlab.com`, and `scopeKind: group` resolves a group. | in-crate HTTP |
| 3 | same | `the_base_url_is_normalised_and_anything_but_http_with_a_host_is_refused` | **Normalised:** host lowercased, port and path kept, trailing slash dropped. **Refused with 400 before GitLab is called:** `ftp://`, a scheme-less value, `https://` with no host, `file://`, and a value that is not a URL. | in-crate HTTP |
| 4 | same | `invalid_create_input_is_a_400_before_gitlab_is_called` | A blank or whitespace-only `name`, an unknown `scopeKind`, a blank `scopePath` and a blank `accessToken` each give a 400 `ApiErrorBody` (not a 422). No GitLab call is made and no row is stored. | in-crate HTTP |
| 5 | same | `a_duplicate_base_url_and_scope_is_a_409_and_stores_nothing_more` | **Duplicate:** a duplicate `(baseUrl, scope)`, with the base URL written differently, is a 409 that stores no new secret and doesn't echo the token. **Not a duplicate:** the same path on another instance is allowed. | in-crate HTTP |
| 6 | same | `gitlab_refusals_on_create_are_400s_that_name_the_reason_and_store_nothing` | **Done-when "bad token, missing scope, missing role":** every `GitLabError` maps to a 400 whose message names the reason:<br>- `Unauthorized`: "rejected the access token"<br>- `MissingScope(s)`: names `s`<br>- `MissingRole`: "Maintainer"<br>- `NotFound`: names the path<br>- `RateLimited`: "later"<br>- `Upstream`: "GitLab request failed", without the upstream detail<br>Nothing is stored and the token is never echoed. | in-crate HTTP |
| 7 | same | `only_a_system_admin_manages_connections_and_a_signed_out_caller_gets_401` | **Done-when "missing role", on the GridOps side:**<br>- a member gets 403 on POST, PATCH, DELETE, verify and rotate, and a signed-out caller gets 401;<br>- a refused caller never reaches GitLab and changes nothing;<br>- the list is 200 with `canManage: false` for a member and 401 when signed out. | in-crate HTTP |
| 8 | same | `patch_renames_without_calling_gitlab_and_leaves_the_token` | **Rename:** a PATCH with only `name` renames without calling GitLab or touching the token, and is audited. **Errors:** a blank `name` or `accessToken` is a 400, and an unknown id is a 404. | in-crate HTTP |
| 9 | same | `patch_with_a_new_token_verifies_it_against_the_stored_scope_before_storing_it` | **Refused:** a refused new token is a 400 naming the missing scope, and the old token stays. **Accepted:** an accepted one runs `verify_access` only (no second `resolve_scope`), overwrites under the same `access_token_key` and keeps the name. | in-crate HTTP |
| 10 | same | `verify_answers_204_or_a_400_naming_what_the_token_lacks` | **Success:** `POST …/verify` is a 204 with no body and calls `verify_access` on the connection's instance. **Refusals:** 400s naming the refused token, the missing scope or the missing role. **Unknown id:** 404. | in-crate HTTP |
| 11 | same | `rotating_overwrites_the_webhook_secret_so_the_old_one_stops_working` | `POST …/webhook-secret/rotate` is a 200 `GitlabWebhookSecret` with a new secret. It overwrites under the same `webhook_secret_key`, so the old secret is no longer stored anywhere and no new row is added. The rotation is audited. An unknown id is a 404. | in-crate HTTP |
| 12 | same | `deleting_a_connection_removes_it_and_both_stored_secrets` | A delete is a 204 that removes the row and both secrets and is audited. A second delete is a 404. | in-crate HTTP |
| 13 | same | `a_connection_a_pool_targets_cannot_be_deleted` | **Done-when "delete-while-targeted" (pool):** a 409 that leaves the connection, its secrets and the pool link in place. | in-crate HTTP |
| 14 | same | `a_connection_a_fleet_target_references_is_refused_with_409_not_500` | Delete-while-targeted, fleet target. `fleet_ci_targets` is NO ACTION in 0047, so an unchecked delete would be a 500. | in-crate HTTP |
| 15 | same | `a_connection_with_a_live_runner_cannot_be_deleted_until_the_runner_is_gone` | Delete-while-targeted, live runner: 409. Once the runner is deleted (`deleted_at` set) the delete succeeds. | in-crate HTTP |
| 16 | same | `no_response_but_create_and_rotate_carries_a_secret_and_no_audit_event_does` | **Done-when "secret non-disclosure":** no secret (either token, either webhook secret) and no `webhookSecret` key appears in:<br>- the list, rename, token replace, verify, the delete 409, the pool detail or the pool options;<br>- any audit label or metadata;<br>- the connection row in plain text. | in-crate HTTP |
| 17 | same | `a_gitlab_pool_is_created_with_its_targets_and_hands_its_starts_to_the_reconciler` | **Pool create:** `scope: gitlab`, `gitlabConnectionIds` and `runUntagged` are stored, `runner_group_id` is forced to 1 although 5 was sent, and the link rows are written. **D14:** with the built-in host on, the create gives `handedToReconciler: 1`, `provisioned: []`, starts no runner and makes no `create_runner` call. **Pool detail:** carries `runUntagged` and `gitlabConnections` as exactly `{id,name,baseUrl,scopeKind,scopePath}`. **Audit metadata:** carries `gitlabConnectionIds` and `runUntagged` (A W3). | in-crate HTTP |
| 18 | same | `creating_a_gitlab_pool_needs_a_system_admin_and_existing_connections` | **Default:** `runUntagged` defaults to false (checked on a control pool). **Refusals:** an installation-admin member who is not a system admin gets a 403, and an unknown connection id is a 400. Only the control pool exists afterwards. | in-crate HTTP |
| 19 | same | `updating_a_gitlab_pool_replaces_its_targets_and_rolls_runners_only_when_they_change` | **Targets:** replaced as a set on update. **`rollingReplacement` (A W3):** true when the target set or `runUntagged` changes, false for an identical body or a different `runnerGroupId`, which is forced to 1. **Left out:** fields left out are kept. **Detail and audit:** the pool detail reflects the update, and the `runner_pool.updated` metadata carries both fields. | in-crate HTTP |
| 20 | same | `an_update_cannot_move_a_pool_between_gitlab_and_other_targets` | **"An update may not move a pool between GitLab and non-GitLab scope."** Each of these is a 400 with nothing changed:<br>- an org pool gaining a GitLab target;<br>- a GitLab pool with `gitlabConnectionIds: []`;<br>- a GitLab pool gaining a Bitbucket target (on Tart, so Bitbucket's own rules pass);<br>- an unknown connection id. | in-crate HTTP. The scope comes from the stored pool, so this check lives in the handler. |
| 21 | same | `reconciling_a_gitlab_pool_hands_its_starts_to_the_reconciler` | D14 at the second call site (`reconcile_pool`): a 200 with `handedToReconciler: 1` and `provisioned: 0`. No runner row is created. | in-crate HTTP |
| 22 | same | `deleting_a_gitlab_runner_deletes_it_at_its_gitlab_instance` | **D17:** `delete_runner` is called on the connection's instance and exactly that runner is removed at GitLab. **GridOps:** the row is deleted. **Bookkeeping:** no `github_runner_cleanup` row, and no "GitHub" text or `githubCleanup` key in the event. | in-crate HTTP |
| 23 | same | `a_runner_gitlab_no_longer_knows_is_still_deleted` | D17: a `NotFound` counts as success, and the row is deleted with GitLab bookkeeping. | in-crate HTTP |
| 24 | same | `a_runner_is_deleted_even_when_gitlab_is_unreachable` | D17: an `Upstream` failure still deletes the row, and nothing is queued for GitHub. | in-crate HTTP |
| 25 | same | `a_runner_whose_connection_is_gone_is_deleted_without_calling_gitlab` | D17: when `gitlab_connection_id` is NULL or `gitlab_runner_id` is NULL, the row is deleted without calling GitLab. | in-crate HTTP |
| 26 | same | `deleting_a_gitlab_pool_completes_while_gitlab_fails` | D17: a pool delete with a GitLab runner completes while GitLab fails. GitLab is asked once and nothing is queued for GitHub. | in-crate HTTP |
| 27 | `crates/gridops-api/src/fleet/openapi.rs` | `the_gitlab_operations_are_specified_and_only_140s_await_a_handler` (renamed from `…_specified_before_they_are_served`) | The four #138 paths no longer say "Not served yet"; #140's webhook still does. | unit (spec text) |
| 28 | `crates/gridops-api/src/access.rs` | `the_gitlab_connection_routes_are_documented_session_routes` (new). The `documented()` and `routes_in()` parsers now read PATCH. | **Session routes:** the six GitLab routes are registered in `session_routes()`, so a bearer token is refused. **Docs:** each is documented in `docs/fleet-api.md` as `403 token_not_accepted`. **PATCH:** the existing matrix test now also covers PATCH rows. | unit (route table) |
| 29 | `crates/gridops-api/tests/control_plane_contracts.rs` | `gitlab_connection_routes_are_served_to_admins_only_with_spec_shaped_refusals` | **Binary level:** each of the 6 routes is a 401 when signed out and a 403 for a member, except that the list is a 200 with `canManage: false`. **Spec:** every body validates against the spec. | binary. It proves the routes are mounted in `main.rs` behind the real middleware. |
| 30 | same | `gitlab_connection_list_shows_seeded_connections_and_delete_answers_404_and_409` | **List:** the seeded rows, with `createdAt` and `updatedAt` as instants. **Delete:** 404 for an unknown id, 409 when a pool targets the connection, and 204 for one without references. All bodies are spec-valid. | binary |
| 31 | same | `without_a_gitlab_client_a_create_is_a_400_and_invalid_input_never_gets_that_far` | **Placeholder:** with the binary's `UnconfiguredGitLab`, a create is a 400 "GitLab request failed". **Invalid input:** an invalid base URL is a 400 with a different message. **Storage:** nothing is stored. | binary |
| 32 | same | `a_cors_preflight_from_the_app_allows_patch` | A CORS preflight from the app's origin for PATCH lists PATCH in `access-control-allow-methods`. | binary. CORS is a `main.rs` layer. |
| 33 | `crates/gridops-api/tests/fleet_http/token_access.rs` | (parser change only) `matrix_rows()` and `request()` handle PATCH | The documented-matrix walk now exercises the GitLab PATCH row against the running server: the bearer token is refused and a foreign origin is refused. | binary (existing test) |
| 34 | `crates/gridops-core/tests/gitlab_pools.rs` | `a_gitlab_pool_on_linux_docker_is_valid_with_or_without_untagged_jobs` | A `scope: gitlab` pool on Linux Docker with 1 or 2 targets is valid, with or without `runUntagged` (no Tart-only rule). | unit (model, via the wire JSON) |
| 35 | same | `a_pool_never_mixes_gitlab_targets_with_github_or_bitbucket_ones` | **No mixing**, each refused after a valid control case:<br>- GitLab scope without targets (`[]` or the field missing);<br>- GitLab scope with `repositoryIds`, `repositoryId` or `bitbucketConnectionIds`;<br>- organization or repository scope with GitLab targets. | unit (model) |
| 36 | same | `gitlab_target_ids_follow_the_bitbucket_rules` | **Allowed:** as many targets as `maxCount`. **Refused:** a non-UUID, a repeat, more targets than `maxCount`, and more than 100. | unit (model) |
| 37 | same | `an_update_checks_gitlab_target_ids_like_a_create` | The same id rules on `UpdateRunnerPool`. Both fields may be left out. | unit (model) |
| 38 | same | `saving_a_gitlab_pools_fleet_settings_links_no_github_target` | The `pool_targets` `"gitlab"` arm: after `save_pool_fleet`, a `scope='gitlab'` pool on an Organization installation has no `github_*` row in `profile_ci_targets`. The control case is an org pool on the same installation, which does get `github_organization`. | integration (core DB) |
| 39 | `crates/gridops-core/src/fleet/service_gitlab_tests.rs` (mounted from `service.rs`) | `a_gitlab_target_is_allowed_to_service_principals_and_system_admins_only` | `ensure_target_access` on a GitLab target allows Reconciler, Autoscaler, FixAgent and a system admin, and refuses an installation-admin member with `Forbidden`. | unit inside `service`. These are private functions; see the deviation above. |
| 40 | same | `a_pool_with_gitlab_targets_needs_a_system_admin` | `ensure_pool_access` on a pool with `runner_pool_gitlab_connections` rows allows the Reconciler and a system admin, and refuses an installation-admin member. | unit inside `service` |

**Shared helper:** `crates/gridops-api/tests/support/openapi_schema.rs`, `check_response(path, method, status, body)`. It is included with `#[path]` by #1-#16 and #29-#31.

- It fails unless the spec declares `status` for the operation and the body matches that response's schema.
- Objects are checked strictly, so an undeclared property such as `webhookSecret` on `GitlabConnection` is a violation.
- I sanity-checked it in the scratch copy. It accepts valid bodies and rejects:
  - a leaked property;
  - a wrong `const`;
  - `scopeId: 0`;
  - a non-RFC-3339 date;
  - an undeclared status;
  - a body on a 204.

**Test wiring T added (no production logic):**

- `#[cfg(test)] mod gitlab_tests;` in `crates/gridops-api/src/resources.rs`;
- the test module mount in `crates/gridops-core/src/fleet/service.rs`;
- `gridops-core = { path = "../gridops-core", features = ["test-support"] }` in the `crates/gridops-api/Cargo.toml` `[dev-dependencies]`, as the brief pins it.

## RED confirmation

The authoritative run is on the branch. The tests that need the injection seam can't compile until it exists, so I also ran a seam-only scratch copy for runtime evidence.

**1. Branch, `cargo test -p gridops-core --no-fail-fast --tests`.** Only the 7 new tests fail. All 261 other lib tests and every other core integration target pass.

```
fleet::service::gitlab_access_tests::a_gitlab_target_is_allowed_to_service_principals_and_system_admins_only
  panicked at service_gitlab_tests.rs:117: a system administrator may act on a GitLab target
fleet::service::gitlab_access_tests::a_pool_with_gitlab_targets_needs_a_system_admin
  panicked at service_gitlab_tests.rs:143: an installation admin who is not a system admin is refused
gitlab_pools: a_gitlab_pool_on_linux_docker_is_valid…     left: Err("Runner pool scope is invalid.") right: Ok(())
gitlab_pools: a_pool_never_mixes…                         the control case: left: Err("Runner pool scope is invalid.")
gitlab_pools: gitlab_target_ids_follow_the_bitbucket_rules the control case: left: Err("Runner pool scope is invalid.")
gitlab_pools: an_update_checks_gitlab_target_ids_like_a_create  update: an id that is not a UUID must be refused
gitlab_pools: saving_a_gitlab_pools_fleet_settings…       a GitLab pool is never admitted to a GitHub target: ["github_organization"]
test result: FAILED. 261 passed; 2 failed (lib) | FAILED. 0 passed; 5 failed (gitlab_pools) | all other targets ok
```

**2. Branch, `cargo test -p gridops-api --test control_plane_contracts --test fleet_http --no-fail-fast`.** These run against the binary, which already compiles:

```
gitlab_connection_routes_are_served_to_admins_only…  signed out get /api/v1/platform-connections/gitlab: left: 404 right: 401
gitlab_connection_list_shows_seeded…                 left: 404 right: 200
without_a_gitlab_client_a_create_is_a_400…           left: 404 right: 400
a_cors_preflight_from_the_app_allows_patch           access-control-allow-methods: "GET,POST,PUT,DELETE"
control_plane_contracts: 32 passed; 4 failed (exactly the 4 new tests)
fleet_http: 143 passed; 1 failed
```

`every_route_in_the_documented_matrix_behaves_as_documented`, which now parses PATCH, passes.

The one `fleet_http` failure is `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`. T did not touch that file. It fails in this environment because its preflight output reports "macos-machine: macOS ARM64 machine ready … fleet-host", which that test asserts should not happen. It has nothing to do with #138, and T will re-check it in Phase 6.

**3. Branch, `cargo test -p gridops-api --no-run`.** The in-crate test target fails to compile on exactly one missing symbol, the injection seam the brief pins:

```
error[E0599]: no method named `with_gitlab` found for struct `AppState` in the current scope
   --> crates/gridops-api/src/resources/gitlab_tests.rs:119:10
error: could not compile `gridops-api` (bin "gridops-api" test) due to 1 previous error
```

**4. Scratch copy with only the seam added.** In `/private/tmp/claude-501/…/scratchpad/stub`, `state.rs` got `pub gitlab`, the private `UnconfiguredGitLab`, `with_gitlab`, and `async-trait`. It has no handlers. The command was `cargo test -p gridops-api --bins`.

The result was **59 passed, 28 failed**. Every existing bin test passed, and the 28 failures are exactly the new and changed ones. Each fails on its feature assertion:

```
access::…the_gitlab_connection_routes_are_documented_session_routes :: GET /api/v1/platform-connections/gitlab is not a session route
fleet::openapi::…only_140s_await_a_handler :: get /api/v1/platform-connections/gitlab is served by #138: "Not served yet: implemented by #138."
creating_a_connection_… / a_connection_defaults_… / the_base_url_is_normalised_… / invalid_create_input_… / gitlab_refusals_… :: left: 404 (route not served)
a_duplicate_…, only_a_system_admin_…, patch_*, verify_*, rotating_*, deleting_a_connection_*, a_connection_* (delete 409s), no_response_… :: Error: create …: 404 Not Found
a_gitlab_pool_is_created_… / creating_a_gitlab_pool_needs_… :: left: 400 {"error":"Runner pool scope is invalid."}
updating_a_gitlab_pool_replaces_… :: swap the target: left: [CONNECTION_1] right: [CONNECTION_2]
an_update_cannot_move_a_pool_… :: an organization pool gains a GitLab target: {"configurationVersion":2,"ok":true,"rollingReplacement":true} (200, wanted 400)
reconciling_a_gitlab_pool_… :: left: 500 (the API tried to start the runner itself; D14 missing)
deleting_a_gitlab_runner_… / a_runner_gitlab_no_longer_knows_… / a_runner_is_deleted_even_when_… / a_runner_whose_connection_is_gone_… :: <runner> is never queued for GitHub cleanup (left: 1)
deleting_a_gitlab_pool_completes_… :: GitLab was asked once: left: [] (delete_runner never called)
```

I fixed one wrong-reason failure before confirming the RED. The harness's built-in-host setting insert hit `UNIQUE constraint failed: settings.key`, because a migration seeds that key. It is now an upsert, and every failure above is a feature assertion.

Formatting and lint checks:

- `cargo fmt --all -- --check` is clean.
- `cargo clippy --all-targets` reports no warnings in any file T wrote or edited. I ran it on the branch for core and on the scratch copy for api.

## Ready for F

**RED is valid.** F may implement against these tests. What F needs to know:

- **The only symbol the tests need F to add** is `AppState::with_gitlab(self, Arc<dyn GitLabApi>) -> Self`, plus the `gitlab` field and `UnconfiguredGitLab`, exactly as the brief pins them. `gridops-api` will also need `async-trait` in `[dependencies]` for `UnconfiguredGitLab`. Everything else is reached over HTTP or through existing signatures.
- **Keep these signatures:** `ensure_target_access(tx, principal, target_id)` and `ensure_pool_access(tx, principal, pool_id)` in `fleet/service.rs`. Test #39-#40 call them directly.
- **Keep the test mounts** in `resources.rs` and `service.rs`.
- **Existing struct literals.** When `CreateRunnerPool` and `UpdateRunnerPool` gain `gitlab_connection_ids` and `run_untagged`, these literals need the two fields added (`vec![]` / `None`, `false` / `None`) to compile:
  - `control_plane_contracts.rs` (`shared_provisioning_contract_…`, :702);
  - the `models.rs` test module (:571, :677, :718).

  The brief assigns this compile-only edit to the implementation. F may make it as long as no assertion changes, and T will review it in Phase 6. The same applies to `resources::tests::settings_report_the_effective_webhook_url` if `effective_webhook_url` gains a path parameter.
- **Choices the tests pin that the brief leaves open (see `decisions.md`):**
  - 400 (not axum's 422) for a well-formed body with an invalid value, including an unknown `scopeKind`;
  - a whitespace-only name is invalid;
  - the list is open to any signed-in user;
  - a non-system-admin creating a GitLab pool gets 403;
  - A W3: `rollingReplacement` for target-set and `runUntagged` changes, and both fields in the pool audit metadata;
  - error messages contain the substrings listed in #6.
- **What isn't covered by a test:**
  - No log line contains a secret. There is no tracing capture in the suite, so S should review this in the diff.
  - Token-level credential correctness. `FakeGitLab` never sees the token (a documented limit), so "the right credentials" is asserted as "the connection's base URL".
