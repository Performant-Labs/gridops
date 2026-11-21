# Handoff-F: Phase 5 - GitLab platform-connection and pool-target API (#138)

**Date:** 2026-11-20
**Branch:** claude/gitlab-138-api-44d3a8 (base 85f124f, T's RED)
**Issue:** #138

## What was done

- `crates/gridops-api/src/resources/gitlab.rs` (new): the six GitLab connection handlers: list, create, PATCH, DELETE, verify, and rotate the webhook secret. It also holds:
  - the pool helpers `selected_gitlab_connections`, `pool_gitlab_connection_ids`, `pool_gitlab_connections`, `gitlab_connection_options` and `pool_has_gitlab_targets`;
  - the D17 helper `cleanup_gitlab_runner`;
  - base-URL normalisation, input validation, and the mapping from `GitLabError` to a 400.
- `crates/gridops-api/src/resources.rs`:
  - **Wiring:** the `mod gitlab` line and its re-exports.
  - **Access structs:** `PoolAccess.run_untagged`; `RunnerAccess.gitlab_connection_id` and `gitlab_runner_id`, loaded by both of its queries.
  - **Pool detail:** gains `gitlabConnectionIds`, `gitlabConnections` and `runUntagged`.
  - **Pool update:**
    - the no-mixing check against the stored scope;
    - `runner_group_id = 1` for `gitlab`;
    - `run_untagged` is stored;
    - the GitLab target set is replaced;
    - a change to the GitLab targets or to `run_untagged` sets `runtime_changed`;
    - the audit metadata gains `gitlabConnectionIds` and `runUntagged`.
  - **Pool create:** the same fields and rules, and the GitLab link rows.
  - **Pool options:** gain `gitlabConnections`.
  - **D14:** `starts_belong_to_reconciler(providers, has_gitlab_targets)`, updated at all three callers.
  - **D17:** `delete_runner_resources` is now an explicit `match` on `ci_platform`, with `github`, `bitbucket` and `gitlab` arms. An unknown value is an internal error.
  - **Webhook URL:** `public_webhook_url(base, path)` is the shared builder, and `effective_webhook_url` delegates to it.
- `crates/gridops-api/src/state.rs`: `pub gitlab: Arc<dyn GitLabApi>`; a private `UnconfiguredGitLab` whose every call returns `Upstream("GitLab client not available yet")`; and `with_gitlab`.
- `crates/gridops-api/src/main.rs`: the `routing::patch` import, the four GitLab route lines, and `Method::PATCH` in CORS `allow_methods`. Nothing else.
- `crates/gridops-api/Cargo.toml` and `Cargo.lock`: `async-trait` in `[dependencies]`, for `UnconfiguredGitLab`.
- `crates/gridops-core/src/models.rs`: `gitlab_connection_ids` and `run_untagged` on `CreateRunnerPool` and `UpdateRunnerPool`, with two new validators:
  - `validate_gitlab_targets` enforces the no-mixing rule;
  - `validate_gitlab_connections` applies Bitbucket's id rules without its Tart-only rule;
  - the scope check accepts `"gitlab"`.

  The `models.rs` test literals got the new fields. This is a compile-only edit.
- `crates/gridops-core/src/fleet/targets.rs`: `pool_targets` gains a `"gitlab"` arm that returns `Ok(Vec::new())`.
- `crates/gridops-core/src/fleet/service.rs`:
  - **`ensure_pool_access`:** a pool with `runner_pool_gitlab_connections` rows needs a system admin.
  - **`ensure_target_access`:** explicit `match` arms on `kind`. GitHub and Bitbucket keep their rules. GitLab allows service principals and system admins. An unknown kind is refused.
- `docs/openapi/gridops-v1.json`: the six "Not served yet: implemented by #138." descriptions are replaced with real ones. `/api/v1/webhooks/gitlab` is untouched.
- `docs/fleet-api.md`: six "Who may call what" rows for the GitLab routes, `403 token_not_accepted`.
- `crates/gridops-api/tests/control_plane_contracts.rs`: the `CreateRunnerPool` literal at :702 gains `gitlab_connection_ids: vec![]` and `run_untagged: false`. This is the compile-only edit T's RED handoff assigned to F. No assertion changed.

## Design decisions

- **The handlers live in `resources/gitlab.rs`**, following `tart_host.rs` and the brief's layout. `resources.rs` keeps the keep-list items, amended by A's W5: the D14 predicate, the pool create, update, detail and options plumbing, the D17 dispatch, `RunnerAccess`, and the webhook-URL helper. The GitLab SQL and JSON for those sites are helpers in `gitlab.rs`, called from `resources.rs`.
- **Request bodies use defaulted `String` fields**, not a `ScopeKind` enum. An unknown `scopeKind` or a missing field is then the spec's 400 rather than axum's 422 (T's pin).
- **Credentials.** The base URL is stored normalised, with no trailing slash. The client is given `stored + "/"`, so an instance served under a path prefix (`https://host/gitlab`) joins its API paths beneath the prefix instead of replacing it.
- **Base-URL normalisation is stricter than the brief's minimum.** Besides non-http(s) URLs and URLs with no host, it also refuses:
  - credentials in the URL, which would otherwise be stored as plain text in `base_url`;
  - a query;
  - a fragment.

  The host is lowercased and a default port is dropped, both by `url::Url`.
- **Order of create calls:** `resolve_scope`, then the duplicate check on `(base_url, scope_kind, scope_id)`, then `verify_access`, then storage. The duplicate check needs the resolved id. Running it before `verify_access` saves a GitLab call. A race that loses on the unique index is also a 409.
- **The connection-delete checks run in the same transaction as the delete:** pool memberships, fleet targets and live runners. Nothing can start referencing the connection between the check and the delete.
- **Error mapping.** Every `GitLabError` is a 400:
  - `Upstream` is exactly "GitLab request failed". Its detail goes to `tracing::warn` only.
  - `NotFound` names the scope path.
  - `MissingRole` names "Maintainer (or Owner)".
  - `RateLimited` says "Try again later".
- **PATCH with neither field** returns the connection unchanged, with no audit event and no GitLab call. The spec defines PATCH as a partial update with nothing required.
- **D14 predicate.** `pool_has_gitlab_targets` is true when the pool has link rows or its scope is `gitlab`. The two always agree, because a GitLab pool can't lose its last target. The scope check is belt and braces, so no GitLab pool ever reaches the API's in-request start.
- **`run_untagged` applies only to GitLab pools.** On create it is stored as `scope == "gitlab" && runUntagged`. On update, a non-GitLab pool keeps its stored value whatever the client sends. A non-GitLab pool therefore never gets a meaningless runtime change from it.
- **The D17 GitLab arm never fails.** Each of these is logged at `warn` with no secret and recorded in the runner event as `{"gitlabCleanup": "deferred", "error": <safe reason>}`, with the message "<name> was removed locally; its GitLab registration is left for the orphan sweep":
  - a missing connection id;
  - a missing or invalid runner id;
  - a missing connection row;
  - an unreadable or missing token;
  - any error other than `NotFound`.

  A success, including `NotFound`, is recorded as `{"gitlabCleanup": "complete"}`.
- **The GitHub and Bitbucket arms keep their behaviour byte for byte.** GitHub keeps the cleanup queue, the "GitHub cleanup will retry" text and the `githubCleanup` key. Bitbucket keeps its 503 refusal, its `DELETE FROM github_runner_cleanup` on success and its `githubCleanup` key. Renaming Bitbucket's key is out of scope.
- **The new pool-detail fields are set after the `json!` literal,** as `builtinRunnerHostEnabled` already is. Adding them inside the literal hit the `json!` macro's recursion limit, and the alternative, a crate-level `recursion_limit`, would have touched `main.rs` outside the allowed lines.
- **The signed-out `/runner-pools/options` body is unchanged:** it has no `gitlabConnections` key. `fleet_http::token_access` pins that body exactly, and the UI only loads the form data when signed in. The signed-in body carries `gitlabConnections`.

## Reuse / extend-vs-new

- **Connection handlers.** They mirror the Bitbucket connection handlers in `resources.rs`: `require_system_admin`, `SameOrigin`, `AuthUser`, the `json!` builders, `audit`, sealed `runtime_secrets` and the unique-violation-to-409 mapping. They sit in a new submodule because the brief (A round 1) placed them there: `resources.rs` is over 6,000 lines.
- **Pool wiring.** It extends the existing pool create, update, detail, options and delete paths in place, with no parallel pool path. `selected_gitlab_connections` copies the shape of `selected_bitbucket_connections`.
- **Webhook URL.** GitHub's `effective_webhook_url` is generalised through `public_webhook_url(base, path)`, which both GitHub and GitLab use, as the brief directs: no third URL builder. GitHub's override stays in `effective_webhook_url`, so its test and call site are unchanged.
- **Fleet access.** It extends `ensure_pool_access`, `ensure_target_access` and `pool_targets` in place, and only those functions.

## Architecture notes for A

- **New module:** `gridops-api::resources::gitlab`. It is private, and its handlers are re-exported from `resources`.
- **New dependency:** `async-trait`, a workspace crate gridops-core already uses, in gridops-api's `[dependencies]`.
- **`AppState` gains `gitlab: Arc<dyn GitLabApi>` and `with_gitlab`,** exactly as the brief pins them identically with #140. `with_gitlab` carries `#[cfg_attr(not(test), allow(dead_code))]` because the binary doesn't call it until #146.
- **Public contract changes:**
  - **Request models:** `CreateRunnerPool` and `UpdateRunnerPool` gain `gitlabConnectionIds` and `runUntagged`.
  - **Pool responses:** the pool detail gains `gitlabConnectionIds`, `gitlabConnections` and `runUntagged`; the signed-in pool options gain `gitlabConnections`.
  - **Pool scope:** `"gitlab"` is a new accepted value.
  - **New routes:** six, all cookie-only, in `session_routes()`.
- **No migration** (D16): 0047 already has every column used.
- **Shared-code edits outside `gridops-api`:** `gridops-core` `models.rs`, and in `fleet/service.rs` / `fleet/targets.rs` only the three functions the brief names.
- **Hand-off to #139 (for the PR body and the shared notes, per A W2):**
  - **The `pool_targets` seam:** the `"gitlab"` arm is where #139 returns the pool's GitLab targets. `save_pool_fleet` disables every `profile_ci_targets` row that `pool_targets` doesn't return.
  - **The reconciler's start paths:** `main.rs:1167/1312/2352` and `fleet.rs:158` treat a non-repository scope as a GitHub organization, so #139 must divert `scope='gitlab'` pools first.

## Deviations from spec / wireframe

- **Deviations the brief already records, restated:**
  - The authorization lives in `fleet/service.rs`, not `authorization.rs`. Bot-token scopes are platform-agnostic, so they needed no edit.
  - `pool_targets` is edited.
  - The rotate route is `…/webhook-secret/rotate`.
  - Update is `PATCH`.
- **New here:** the signed-out pool-options body carries no `gitlabConnections`. See the design decisions.
- There is no wireframe (no UI).

## Tier 1 self-check (incl. tests now GREEN)

**Commands:**
```
cargo fmt --all -- --check                      -> clean
cargo test --workspace --no-run                 -> compiles, no errors
cargo clippy --workspace --all-targets          -> no warning in any file F touched
                                                   (remaining warnings are pre-existing, in
                                                    other files/crates: route_security.rs,
                                                    boundary.rs, gridops-manager, tart-agent, ...)
```

**gridops-api:**
```
cargo test -p gridops-api --no-fail-fast
  unittests src/main.rs        test result: ok. 87 passed; 0 failed
     (includes the 26 GitLab handler tests in resources/gitlab_tests.rs, and the changed
      access.rs and fleet/openapi.rs tests)
  control_plane_contracts      test result: ok. 36 passed; 0 failed   (4 new GitLab tests GREEN)
  fleet_http                   test result: FAILED. 143 passed; 1 failed
     the one failure: release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine,
     the pre-existing environment failure T recorded at RED (this Mac is a macOS ARM64 machine);
     unrelated to #138. token_access (PATCH-aware matrix) passes.
```

**gridops-core and the reconciler:**
```
cargo test -p gridops-core --no-fail-fast --tests
  lib: ok. 263 passed (includes the 2 service_gitlab_tests); gitlab_pools: ok. 5 passed;
  every other target ok
cargo test -p gridops-reconciler --no-fail-fast  -> ok. 68 passed; 0 failed
```

## Evidence appendix

`docs/handoffs/138/evidence.md`. It records:

- the 0047 foreign-key actions behind the three delete-409 checks and the SET NULL runner case;
- the 0014 pool-installation trigger;
- `require_system_admin`;
- `random_token`;
- the `BadRequest` body;
- the fake's base-URL keying and its `NotFound` on delete;
- the rotate and scale-down filters that route GitLab runners into the D17 arm.

## Tests that look wrong (for T)

None of the authored tests look wrong. Some behaviour has no test, and T may want to cover it:

- **Base-URL refusals:** the normaliser refuses credentials (`https://user:pw@host`), a query and a fragment, and it drops a default port (`:443`). No test pins any of this.
- **Name length:** a name longer than 80 characters is refused.
- **Empty PATCH:** `{}` returns the connection unchanged with no audit event.
- **Signed-out options:** the body has no `gitlabConnections` key, which `token_access` pins.
- **GitLab cleanup metadata:** the runner event for a GitLab delete carries `gitlabCleanup: "complete" | "deferred"`. The tests check only that the GitHub key and text are absent.

## Known issues

- **No test or capture checks that no log line holds a secret** (T's note). In review:
  - the handlers log only `GitLabError::Upstream` detail and the D17 reason;
  - `GitLabCredentials` has a redacting `Debug`;
  - no `tracing` call gets a token or webhook secret.
- **`fleet_http::release_preflight` fails in this environment.** It is pre-existing and unrelated (see Tier 1).

## Files changed

- `crates/gridops-api/src/resources/gitlab.rs` (new)
- `crates/gridops-api/src/resources.rs`
- `crates/gridops-api/src/state.rs`
- `crates/gridops-api/src/main.rs`
- `crates/gridops-api/Cargo.toml`
- `Cargo.lock`
- `crates/gridops-core/src/models.rs`
- `crates/gridops-core/src/fleet/service.rs`
- `crates/gridops-core/src/fleet/targets.rs`
- `docs/openapi/gridops-v1.json`
- `docs/fleet-api.md`
- `crates/gridops-api/tests/control_plane_contracts.rs` (compile-only struct-literal fields, as T's RED handoff assigned; no assertion changed)
