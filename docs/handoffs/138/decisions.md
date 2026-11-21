# Decision journal: #138 GitLab platform-connection and pool-target API

## A (Phase 3, up-front plan review): 2026-11-20T15:19:07-08:00
- **Decided:** BLOCK, with 2 block findings and 7 warn findings (see `handoff-A.md`).
  - **(1) The test seam can't be reached.** The brief puts the HTTP tests under `crates/gridops-api/tests/` and has them inject `FakeGitLab` through `AppState`. Every test there runs the `gridops-api` binary as a child process, and the crate has no library target, so the fake can't be injected. The binary wires the `Upstream` placeholder, and `gridops-core/test-support` isn't a dev-dependency.
  - **(2) The authorization item names the wrong file and a rule that can't be built.** Bitbucket's fleet authorization is in `fleet/service.rs` (`ensure_pool_access`, `ensure_target_access`) and `registry/queries.rs`, not in `authorization.rs`, and it depends on `bitbucket_user_grants`. GitLab has no grants table, D11 defers user-scoped GitLab visibility, and migrations are forbidden. `ensure_target_access` also panics on the NULL `bitbucket_connection_id` of a GitLab target once #139 admits GitLab targets.
  - **The warnings:**
    - (3) a `resources/gitlab.rs` submodule;
    - (4) the pool request types are in `gridops-core/src/models.rs`, the wire names aren't pinned, and "no GitHub targets" conflicts with the pool's required installation and scope;
    - (5) the spec declares no 502 and `ApiError` has none, while Bitbucket uses 400;
    - (6) Bitbucket has no webhook URL; the analogue is GitHub's `effective_webhook_url`;
    - (7) PATCH plumbing: the `access.rs` parsers, `docs/fleet-api.md` and the "Not served yet" assertion in `openapi.rs`;
    - (8) the delete rule against the 0047 FK actions;
    - (9) how the secrets are stored, which #140 relies on.
  - **Accepted as consistent:**
    - one shared `Arc<dyn GitLabApi>` in `AppState`, set through a `with_gitlab` builder;
    - the D14 predicate change, whose callers are all in `resources.rs`;
    - handlers copied from the Bitbucket ones, using `json!` builders, `require_system_admin` and `SameOrigin`;
    - cookie-only routes in `session_routes()`.
- **Assumed:**
  - sqlx's `Row::get::<String>` panics on a NULL column. That's its documented behaviour; I didn't run it.
  - In-process tests can build an `AppState` the way the `oauth.rs` tests build `Config` and a temporary database. Nobody builds a full `AppState` in a test today, so this is a new shape built from existing parts.
- **Hedged:**
  - **Finding 2** recommends a system-admin-only rule for GitLab fleet access. That rule is my reading of D11 and of the existing `require_system_admin` gates; O may choose another rule, but must write one down.
  - **Finding 4's mix-rule question** is partly a spec question. I left it as a warning because T and F can pick a working request body, but #141 needs the names pinned.
  - **Finding 5** offers two options. 400 is the established Bitbucket pattern; 502 needs a spec change and an `error.rs` change.
  - **The task text named no earlier BLOCK findings,** so this is a first review.
  - **No `survey.md` or Reuse map was provided,** so the brief's own analogues were the baseline.
- **Evidence:**
  - Read: the brief and the brief-gate result r1; epic #136 and issues #140 and #141 (via `gh issue view`); #137's `handoff-A.md` and `decisions.md`.
  - API: `crates/gridops-api/{Cargo.toml,src/state.rs,src/error.rs,src/main.rs,src/access.rs,src/oauth.rs (tests),src/auth.rs,src/resources.rs (Bitbucket, pool and webhook-URL sections),src/resources/{route_security.rs,tart_host.rs},src/fleet/openapi.rs,tests/control_plane_contracts.rs,tests/fleet_http.rs}`.
  - Core: `crates/gridops-core/{Cargo.toml,src/gitlab/mod.rs,src/gitlab/fake.rs,src/fleet/authorization.rs,src/fleet/service.rs,src/fleet/registry/queries.rs,src/models.rs,src/config.rs,src/crypto.rs,src/api_tokens.rs}`.
  - Schema and docs: `migrations/0001_initial.sql` (`runner_pools`), `migrations/0047_gitlab.sql`, the GitLab paths and schemas of `docs/openapi/gridops-v1.json`, the Bitbucket rows of `docs/fleet-api.md`, and the workspace `Cargo.toml` (resolver 3, edition 2024, `unsafe_code = "forbid"`).

## A (Phase 3, up-front plan review, round 2): 2026-11-20T15:31:08-08:00
- **Decided:** BLOCK, with 1 block finding and 4 warn findings (see `handoff-A.md`).
  - **Round 1 is resolved.** Both block findings and all seven warnings are fixed in the amended brief, and the code bears them out:
    - `AppState::new` keeps its five-argument signature;
    - `crate::session_routes()` is reachable from in-crate test modules;
    - `test-support` exists in gridops-core;
    - the `runner_pool_installations` trigger (0014) keeps `ensure_pool_access` satisfiable for GitLab pools;
    - the `effective_webhook_url`, `access.rs` parser, `openapi.rs` assertion and D17 `delete_runner_resources` citations all match the code.
  - **(1) New block, caused by the amendment.** `scope: "gitlab"` is a third value in a field that every reader treats as two-way (`== "repository"` else GitHub organization).
    - `gridops-core/src/fleet/targets.rs::pool_targets` (`:854-905`) derives a `github_organization` target from `installation_id` for any non-repository pool.
    - `save_pool_fleet` (`:541`, `:630`), which the API serves at `PUT /api/v1/fleet/pools/{pool_id}/profile`, writes that target into `profile_ci_targets`, which admission reads.
    - The reconciler's `fleet.rs:158`/`:176` and `main.rs:1167`, `:1312` and `:2352` treat `"gitlab"` as organization too.
    - The brief's statement that `installationId` "does not make a GitHub target", and its "already inert" hand-off to #139, are therefore false. No wave-1 issue owns `pool_targets`.
    - **Fix:** #138 adds an explicit `"gitlab"` arm that returns no target, records a deviation, owns `pool_targets` only, adds a test, and corrects the #139 hand-off.
  - **The warnings:**
    - (2) force `runner_group_id = 1` for GitLab pools;
    - (3) the brief contradicts itself on `main.rs` (CORS) and on the `resources.rs` keep-list;
    - (4) D17's dispatch should be an explicit `match` with GitLab bookkeeping that never says "GitHub", and `RunnerAccess` needs the GitLab columns; also a reminder that the round-1 `ensure_target_access` no-panic test isn't in the brief.
- **Assumed:**
  - The UI (#141) or an API client saves fleet settings for a GitLab pool through the existing `PUT /api/v1/fleet/pools/{pool_id}/profile`, which is the only way a pool gets fleet profiles.
  - A GitLab pool can sit on an Organization installation. When the installation is a User account, `pool_targets` already returns nothing.
- **Hedged:**
  - **Finding 1's fix gives the `pool_targets` arm to #138** because #138 introduces the value. O could instead extend D11 to give it to #139, but the brief must then name the owner and describe the interim state, which can't be "already inert".
  - **This is the second Phase-3 BLOCK.** One more means escalation to the operator.
  - **The task text named no earlier BLOCK findings** to re-mark, so round 1's status is recorded in a table in `handoff-A.md`.
- **Evidence:**
  - Read: the amended brief and its diff against d8da721; the brief-gate result r1; round 1's `handoff-A.md`; epic #136's body and comments (D11, D14, D16, D17); issue #139.
  - Core: `crates/gridops-core/src/{models.rs,fleet/targets.rs,fleet/service.rs,gitlab/mod.rs}` and `Cargo.toml` (features).
  - API: `crates/gridops-api/src/{state.rs,main.rs,access.rs,oauth.rs (tests),resources.rs (pool create/update/delete, delete_runner_resources, effective_webhook_url),fleet/mod.rs,fleet/profiles.rs,fleet/openapi.rs}` and `Cargo.toml`.
  - Reconciler: `crates/gridops-reconciler/src/{fleet.rs,main.rs}`.
  - Schema and spec: `migrations/0014_runner_pool_installations.sql`, `migrations/0047_gitlab.sql`, and the runner-pool paths of `docs/openapi/gridops-v1.json`.

## A (Phase 3, up-front plan review, round 3): 2026-11-20T15:42:02-08:00
- **Decided:** PASS, with 0 block findings and 5 warn findings (see `handoff-A.md`).
  - **Round 2 is resolved.**
    - The block: the `"gitlab"` arm of `fleet/targets.rs::pool_targets`, the deviation, Owns, the `save_pool_fleet` test, and the rewritten #139 hand-off.
    - The three warnings: forcing `runner_group_id = 1`, CORS listed consistently across the `main.rs` edits, the keep-list extension, D17's explicit `match` with platform-neutral text, and the `RunnerAccess` columns.
  - **The code hasn't moved since round 2.** `git diff b22c451 a88360c` touches only `docs/handoffs/`.
  - **The warnings:**
    - (1) The `ensure_target_access` test is listed among the gridops-api in-crate tests, but that function and `ensure_pool_access` are private to gridops-core. They are reached only through `FleetService::submit_intent` / `load_operation`, which gridops-api never calls. Move both tests to `gridops-core/src/fleet/admission_tests.rs`, following `:2867`; add that file to Owns as add-only; don't widen visibility.
    - (2) `save_pool_fleet` disables every `profile_ci_targets` row that `pool_targets` doesn't return. The hand-off to #139 must say that #139 fills the `"gitlab"` arm with the pool's GitLab targets.
    - (3) The pool update should count changes to the GitLab connection set and to `run_untagged` in `runtime_changed`, as it does `bitbucket_connections_changed`. Add `gitlabConnectionIds` and `runUntagged` to the pool audit metadata.
    - (4) "Whatever the client sends" contradicts `validate_pool_configuration` (`runner_group_id > 0`), and `UpdateRunnerPool` has no scope. Follow the repository precedent: validate a positive value, store 1.
    - (5) The `resources.rs` keep-list omits the pool detail (`:1471`), the options (`:2388/:2419`) and the audit metadata.
- **Assumed:**
  - #141 or an API client saves fleet settings for a GitLab pool through `PUT /api/v1/fleet/pools/{pool_id}/profile`, as in round 2.
  - GitLab fixes `run_untagged` when the runner registers, so an idle runner keeps the old value until it is rotated.
- **Hedged:**
  - **Warn 1 is a warning, not a block.** The production plan is right, the fix is one placement line, and T reads this handoff. It would become a block only if F or T widened a private authorization function to reach it.
  - **Warn 3's rule is my reading of the Bitbucket precedent.** O may decide that a `run_untagged` change applies to new runners only, but should say so.
  - **The task text named no earlier BLOCK findings to re-mark,** so round 2's status is recorded in a table in `handoff-A.md`.
- **Evidence:**
  - Read: the amended brief and its diff against a88360c; the brief-gate result r1; round 2's `handoff-A.md`; epic #136's D11 and D14 (via `gh issue view`).
  - Core: `crates/gridops-core/src/{fleet/targets.rs (sync, ensure_target, save_pool_fleet, link_pool_target, pool_targets),fleet/service.rs (submit_intent, ensure_* access and identity),fleet/admission_tests.rs:2867,fleet/mod.rs,models.rs:40-520,autoscaling.rs:468-493}`.
  - API: `crates/gridops-api/src/{resources.rs (RunnerAccess, selected_bitbucket_connections, pool detail, update, options, create and delete, D14 predicate and callers, the rotate and scale-down filters, delete_runner_resources, :5281),state.rs,webhooks.rs:423/780,oauth.rs:633,resources/ (layout)}`.
  - Reconciler: `crates/gridops-reconciler/src/{fleet.rs:158,346,main.rs:1167,1312,2352,2669}`.
  - Schema and spec: `migrations/{0001_initial.sql (runner_pools.scope has no CHECK),0014_runner_pool_installations.sql,0047_gitlab.sql}`; the runner-pool and GitLab paths of `docs/openapi/gridops-v1.json` (the pool bodies are loose objects).

## T (Phase 4, author tests / RED): 2026-11-20T16:17:19-08:00
- **Decided:** RED is valid. 40 tests were authored or changed (see `handoff-T-red.md`):
  - 26 in-crate HTTP tests with `FakeGitLab` in `crates/gridops-api/src/resources/gitlab_tests.rs`;
  - 2 changed unit tests in `fleet/openapi.rs` and `access.rs`, with the PATCH parsing added to the `access.rs` and `token_access.rs` matrix parsers;
  - 4 binary-level tests in `control_plane_contracts.rs`;
  - 5 model and `pool_targets` tests in `crates/gridops-core/tests/gitlab_pools.rs`;
  - 2 fleet-access tests in `crates/gridops-core/src/fleet/service_gitlab_tests.rs`.
  - **Spec validation:** a shared strict validator, `crates/gridops-api/tests/support/openapi_schema.rs`, checks every GitLab connection response against the spec's declared status and schema.
  - **Fleet access tests (deviation from A's round-3 W1):** they call `ensure_target_access` and `ensure_pool_access` from a test-only child module of `service`. A suggested going through `submit_intent` in `admission_tests.rs`, but that path can't show the decision: `ensure_target_identity_for_pool` refuses every GitLab target first (`service.rs:250`, `:420`) until #139 adds its arm. No visibility is widened.
  - **Wiring added:** `crates/gridops-api/Cargo.toml` gets the `gridops-core` `test-support` dev-dependency, `resources.rs` gets the `#[cfg(test)] mod gitlab_tests;` mount, and `service.rs` gets the test mount.
  - **One wrong-reason RED fixed:** the harness's `settings` insert collided with a migration-seeded key and is now an upsert.
- **Assumed:** these are pins the brief leaves open. F implements against them; O may overrule any of them.
  - A well-formed body with an invalid value (including an unknown `scopeKind`) is a 400, as the spec declares, not axum's 422. A whitespace-only name is invalid.
  - `GET /api/v1/platform-connections/gitlab` is open to any signed-in user, with `canManage: false`. This is Bitbucket parity, and the spec declares no 403 on it.
  - A non-system-admin creating a GitLab pool gets 403, mirroring `selected_bitbucket_connections`.
  - From A's W3:
    - a change to the GitLab target set or to `runUntagged` sets `rollingReplacement`;
    - a `runnerGroupId` change does not, because it is forced to 1;
    - the pool `created` and `updated` audit metadata carry `gitlabConnectionIds` and `runUntagged`.
  - Each `GitLabError` message contains a stable substring: "rejected the access token", the scope name, "maintainer", the path, "later", "gitlab request failed". Matching is case-insensitive. The binary-level test pins the exact string "GitLab request failed" for the placeholder.
  - The webhook secret is at least 32 characters. Create, update, rotate and delete each add at least one audit event for the connection; action names are not pinned.
  - A runner with `deleted_at` set does not block a connection delete.
- **Hedged:**
  - "The right credentials" for `delete_runner` is checked as "the connection's base URL". `FakeGitLab` never sees the token, which is its documented limit.
  - "No log line contains a secret" has no test because the suite doesn't capture tracing. S should review it in the diff.
  - A's round-2 and round-3 statement that `ensure_target_access` panics at `:931` on a GitLab target is not what happens. On the branch, the Reconciler call returns `Ok` without panicking: sqlx decoded the NULL. The observable defects are that a system admin is refused and an installation-admin member passes `ensure_pool_access`, and the tests are RED on those.
  - `fleet_http::release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails in this environment. Its preflight sees a ready macOS ARM64 machine. T didn't touch that file and it is unrelated to #138; T will re-check it in Phase 6.
- **Evidence:**
  - `cargo test -p gridops-core --no-fail-fast --tests`: only the 7 new tests fail (lib 261 passed and 2 failed; `gitlab_pools` 0 passed and 5 failed); every other target passes.
  - `cargo test -p gridops-api --test control_plane_contracts --test fleet_http --no-fail-fast`: control_plane_contracts 32 passed and 4 failed (the new ones: 404 instead of 401/200/400, and CORS methods "GET,POST,PUT,DELETE"); fleet_http 143 passed and 1 failed (the unrelated `release_preflight`); the PATCH-aware matrix test passes.
  - `cargo test -p gridops-api --no-run`: the one error is `E0599 no method named with_gitlab`.
  - Seam-only scratch copy (only `with_gitlab`, the `gitlab` field and `UnconfiguredGitLab` added), `cargo test -p gridops-api --bins`: 59 passed and 28 failed. All existing bin tests pass, and the 28 failures are exactly the new and changed tests, each on its feature assertion (route 404, "Runner pool scope is invalid.", GitHub cleanup queued for a GitLab runner, `delete_runner` never called, a 500 from an in-request start).
  - `cargo fmt --all -- --check` is clean. `cargo clippy --all-targets` reports no warnings in T's files.

## F (Phase 5, implement against RED): 2026-11-20T16:40:15-08:00
- **Decided:** implemented #138 against T's suite; every authored test is GREEN (see `handoff-F.md`).
  - **Handlers:** in a new `resources/gitlab.rs`. `resources.rs` keeps the pool create/update/detail/options plumbing, the D14 predicate, the D17 `match` on `ci_platform` and the generalised `public_webhook_url`.
  - **`AppState`:** gains `gitlab`, `UnconfiguredGitLab` and `with_gitlab`, exactly as pinned with #140. `main.rs` changes only the route lines, the `patch` import and PATCH in CORS.
  - **Core:** pool models gain `gitlabConnectionIds` and `runUntagged` with the no-mixing rule. `pool_targets` gains a `"gitlab"` arm that returns nothing. `ensure_pool_access` and `ensure_target_access` get explicit GitLab rules: system admin, or a service principal for targets.
  - **D17:** the GitLab arm never fails a delete. A runner it can't remove is recorded as `gitlabCleanup: deferred` and left for #139's orphan sweep.
- **Assumed:**
  - Storing the base URL without a trailing slash and handing the client `stored + "/"` is what #146's client expects. That is what lets a path-prefixed instance join API paths correctly.
  - A `{}` PATCH is a no-op 200 (the spec's partial update with nothing required).
  - `runUntagged` only means something for a GitLab pool. A non-GitLab pool keeps its stored value whatever the client sends.
- **Hedged:**
  - **Base-URL normalisation is stricter than the brief.** It refuses userinfo, a query or a fragment, so no credential is ever stored in plain text in `base_url`. No test pins this; it is flagged for T.
  - **The signed-out `/runner-pools/options` body is unchanged,** with no `gitlabConnections` key, because `fleet_http::token_access` pins it exactly. The signed-in body carries the field.
  - **The new pool-detail fields are set after the `json!` literal** because the macro hit its recursion limit. This is the same pattern as `builtinRunnerHostEnabled`.
  - **One compile-only edit to a test file:** the `CreateRunnerPool` literal in `control_plane_contracts.rs` gains the two new fields. T's RED handoff assigned this edit; no assertion changed.
- **Evidence:**
  - `cargo test -p gridops-api`: bin 87/87; `control_plane_contracts` 36/36; `fleet_http` 143/144. The one failure is the pre-existing environment-only `release_preflight`, which T recorded at RED.
  - `cargo test -p gridops-core --tests`: all ok (lib 263; `gitlab_pools` 5/5).
  - `cargo test -p gridops-reconciler`: 68/68.
  - `cargo fmt --check` is clean, and `cargo clippy --workspace --all-targets` reports nothing in F's files.
  - Source facts are in `docs/handoffs/138/evidence.md`.

## T (Phase 7, verify GREEN + Tier 2): 2026-11-20
- **Decided:** the suite is GREEN with no blocking Tier 2 issue; returned PASS. Added 4 tests for behaviors F flagged as untested (base-URL credential/query/fragment refusal and default port, 80-character name boundary, empty PATCH, `gitlabCleanup` metadata).
- **Assumed:** `release_preflight` fails only because this host is a ready macOS ARM64 machine, so it is environmental. It failed identically at RED, before F's code.
- **Hedged:** the four added tests were not RED-first; they pin behavior that already existed. Two were confirmed by temporary mutation of production code (restored byte-for-byte). The other two were not mutation-checked.
- **Evidence:** `cargo test -p gridops-api` bin 91/91 (87 + 4), `control_plane_contracts` 36/36, `fleet_http` 143/144 (environmental); `cargo test -p gridops-core --tests` all ok; `gridops-reconciler` 68/68; fmt clean; clippy clean in changed files. Mutation run: 28 passed, 2 failed, as expected.

## A (Phase 7, anti-duplication gate): 2026-11-20
- **Decided:** PASS. F extended the named analogous objects in place (pool create/update/detail/options, `delete_runner_resources`, the URL helper, fleet access functions); the only new object is the brief-mandated `resources/gitlab.rs`.
- **Assumed:** the repeated `runtime_secrets` upsert and the unique-violation helper have no shared seam to extend, so local copies are not parallel paths.
- **Hedged:** four `warn` findings (secret-upsert duplication, local `is_unique_violation`, update-time no-mixing check, repeated link-row loop), none blocking.
- **Evidence:** read the full diff of `resources.rs`, `state.rs`, `main.rs`, `models.rs`, `fleet/service.rs`, `fleet/targets.rs`, `Cargo.toml`, and `resources/gitlab.rs` in full; see `handoff-A-dup.md`.

## S (Phase 8, spec audit): 2026-11-20
- **Decided:** PASS (see `handoff-S.md`). All 27 spec items from the brief and the OpenAPI spec check out against the code; the test suite passes the `test-quality.md` section 7 rubric with no "delete or merge" finding. No HTML report: #138 has no visual surface.
- **Assumed:**
  - `fleet_http::release_preflight` fails only because this host is a ready macOS ARM64 machine. The diff doesn't touch that file, and T saw the same failure at RED.
  - Mapping every `GitLabError`, outage included, to a 400 is the project's decision (the spec declares no 502; A weighed it in round 1), so it is an advisory note and not an `ADVISORY-HOLD`.
- **Hedged:**
  - No test captures logs to prove no secret is logged. I covered it by reading every `tracing` call in the diff, and none receives a token or webhook secret.
  - The signed-out pool-options body has no `gitlabConnections` key (F's deviation, because `token_access` pins the body). Advisory only; #141 should treat the key as optional.
- **Evidence:** re-ran `cargo test -p gridops-api` (bin 91, `control_plane_contracts` 36, `fleet_http` 143 of 144), `cargo test -p gridops-core --tests` (all ok) and `cargo fmt --check` (clean), matching T's numbers. Read `resources/gitlab.rs` in full, the diffs of `resources.rs`, `state.rs`, `main.rs`, `models.rs`, `fleet/service.rs`, `fleet/targets.rs`, `access.rs`, `fleet/openapi.rs`, the OpenAPI GitLab sections and `docs/fleet-api.md`, plus the key in-crate, binary-level and core tests. `git diff b22c451 HEAD` over migrations, `gitlab/`, `fleet/domain.rs` and the reconciler is empty.
