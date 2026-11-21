# Handoff-S: Phase 8 - GitLab platform-connection and pool-target API (#138)

**Date:** 2026-11-20
**Branch:** claude/gitlab-138-api-44d3a8 (head caef262, diff base b22c451 = master)
**Issue:** #138
**Spec:** `docs/handoffs/138-brief.md` (issue text plus the orchestrator's clarifications) and `docs/openapi/gridops-v1.json` (the spec wins where the issue text differs)
**Handoff-T reviewed:** `docs/handoffs/138/handoff-T-green.md` (and `handoff-T-red.md`)
**Handoff-A reviewed:** `docs/handoffs/138/handoff-A.md` (up-front plan review, round 3, PASS) and `docs/handoffs/138/handoff-A-dup.md` (anti-duplication gate, PASS)
**Handoff-F reviewed:** `docs/handoffs/138/handoff-F.md`
**Operator-facing report:** N/A. No visual surface; an HTML report is required only for visual audits.

## A precondition
Confirmed. Round 3 of A's up-front review returned PASS (rounds 1 and 2 were BLOCK, both resolved in the amended brief). A's anti-duplication gate on the finished diff (`handoff-A-dup.md`) returned PASS, with four optional `warn` items.

## T precondition
Confirmed. `handoff-T-green.md` reports "Blocking issues: None". Its only red is `fleet_http::release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`, which is environmental (this host is a ready macOS ARM64 machine, which the test asserts it is not). I re-ran it and got the same single failure. The diff doesn't touch `release_preflight.rs`; the only file under `tests/fleet_http/` it changes is `token_access.rs`. T recorded the same failure at RED, before F's code.

## Visual-diff-tool precondition
N/A. #138 is an API and persistence change with no rendered output. The browser and visual-diff preconditions don't apply.

## Preview / spec sanity check
Done (two minutes, as the role asks). The contract is consistent with this API's own conventions and I found nothing that justifies `ADVISORY-HOLD`:
- The six GitLab routes follow the Bitbucket naming (`/api/v1/platform-connections/{provider}`), the status codes are the usual ones (201 create, 204 delete and verify, 404, 409 duplicate), and the secrets are write-only.
- Two choices could draw a second look, but both are deliberate, documented, and shared with Bitbucket, so they are advisory notes below and not a hold:
  - every `GitLabError` (including a GitLab outage) is a 400, because the spec declares no 502 and `ApiError` has none; A considered this in round 1 and the brief fixed it;
  - `PATCH` where Bitbucket uses `PUT`.

## Independent re-run (confirmation only; Tier 1/2 is T's job)

| Command | Result |
|---|---|
| `cargo test -p gridops-api --bins gitlab` | 33 passed, 0 failed |
| `cargo test -p gridops-api --no-fail-fast` | bin 91 passed; `control_plane_contracts` 36 passed; `fleet_http` 143 passed, 1 failed (the environmental `release_preflight`) |
| `cargo test -p gridops-core --no-fail-fast --tests` | every target ok (lib 263; `gitlab_pools` 5; `gitlab_contract` 14) |
| `cargo fmt --all -- --check` | clean |

These match F's and T's numbers exactly.

## Spec compliance

Checked each requirement against the code in `crates/gridops-api/src/resources/gitlab.rs`, the `resources.rs` / `state.rs` / `main.rs` / `models.rs` / `fleet/service.rs` / `fleet/targets.rs` diffs, and the spec. YES = compliant.

| # | Requirement (source) | Compliant | Where / how checked |
|---|---|---|---|
| 1 | `GET/POST/PATCH/DELETE …/platform-connections/gitlab`, plus verify and rotate (issue, spec) | YES | Six handlers; routes in `main.rs` `session_routes()` (cookie-only, `SameOrigin`). Rotate is `…/webhook-secret/rotate` (spec wins over the issue's name); update is `PATCH`. |
| 2 | Create: `resolve_scope`, then `verify_access`, token and generated webhook secret stored by key, webhook URL and secret returned once (issue, D2, D3) | YES | `create_gitlab_connection`. `ScopeKind` comes from the request's `scopeKind`; credentials are built per call from the stored base URL. Both secrets are `vault.seal`ed into `runtime_secrets` under `access_token_key` and `webhook_secret_key`, in one transaction with the connection row. The webhook secret is `random_token(32)`. |
| 3 | No other response carries either secret; the connection schema has no token, secret or webhook URL (spec) | YES | `connection_json` emits only the ten `GitlabConnection` fields. Test `no_response_but_create_and_rotate_carries_a_secret_and_no_audit_event_does` covers list, rename, token replace, verify, a refused delete, pool detail, pool options, every audit row and the connection row. |
| 4 | Duplicate `(base_url, scope_kind, scope_id)` is 409 (issue, spec) | YES | Pre-check after `resolve_scope` (it needs the resolved id), plus the unique-index race mapped to 409. |
| 5 | `baseUrl` default `https://gitlab.com`; normalised (scheme, host, optional port and path, no trailing slash); reject non-http(s) or no host (brief) | YES, stricter | `normalised_base_url`. It additionally refuses userinfo, a query and a fragment. That fits the spec's own "scheme, host, optional port and path" and keeps a credential out of the plain-text `base_url` column. See the advisory notes. |
| 6 | Delete: 204, 404, 409 while a pool, a fleet target or a live runner references it; removes both stored secrets (brief, D16, 0047 FK actions) | YES | `delete_gitlab_connection` checks all three inside the deleting transaction, then deletes the row and both `runtime_secrets` keys. Three separate tests plus one for the removal of the secrets. A soft-deleted runner doesn't block (T's pin). |
| 7 | `POST …/verify`: 204, or 400 naming the refused token, scope or role (spec) | YES | `verify_gitlab_connection`. A missing stored token is a 400 "Replace the token". |
| 8 | `POST …/webhook-secret/rotate`: 200 `GitlabWebhookSecret`, overwrites under the same key so the old secret stops working at once (brief) | YES | `rotate_gitlab_webhook_secret` uses an upsert on `webhook_secret_key`. The response re-discloses the new secret, as the spec says (brief W-3). |
| 9 | `PATCH`: partial `{name?, accessToken?}`; a new token is verified against the stored scope before it is stored, under the same key (spec) | YES | `update_gitlab_connection`. `resolve_scope` is not called again. A `{}` body returns the connection unchanged with no audit event and no GitLab call (F's assumption, consistent with the spec's "nothing required"). |
| 10 | Every `GitLabError` is a 400 with the stated message; no secret in a response, log or audit event (brief) | YES | `gitlab_message`. The test table asserts all six variants (W-2) and that `Upstream` detail never reaches the body. I read every `tracing` call in the diff (`gitlab.rs:575, 599, 607, 768`): they log the runner id, a fixed reason, a sqlx or vault error, or the upstream detail. None receives a token or webhook secret, and `Vault::open`'s errors carry no plaintext. Audit metadata carries name, base URL, scope and a `tokenReplaced` flag only. This closes T's request that S review logging in the diff. |
| 11 | Webhook URL is the public URL of `POST /api/v1/webhooks/gitlab`; generalise GitHub's helper, no third builder (brief) | YES | `public_webhook_url(base, path)`; `effective_webhook_url` delegates to it, GitHub's override unchanged. Asserted as `https://gridops.example/api/v1/webhooks/gitlab` on create and rotate. |
| 12 | Create, update, rotate and delete each emit an audit event without secrets (brief) | YES | Four distinct actions; `audit_count` asserted. |
| 13 | Pool create and update accept `gitlabConnectionIds` and `runUntagged`; replaced as a set on update; validation mirrors Bitbucket's without the Tart-only rule (brief) | YES | `models.rs` (`validate_gitlab_connections`), `resources.rs` create and update. Up to 100 unique UUID ids, no more than `maxCount`. |
| 14 | No mixing: a GitLab pool has `scope: "gitlab"`, no repositories, no Bitbucket connections; `scope: "gitlab"` needs connections; an update can't move between GitLab and non-GitLab (brief) | YES | `validate_gitlab_targets` on create; the update handler checks against the stored scope. Four update refusals and the model matrix are tested. |
| 15 | `runner_group_id` forced to 1 for `scope: "gitlab"` on create and update so it never registers as a runtime change (brief) | YES | `matches!(scope, "repository" \| "gitlab")` at both sites. The update test sends `runnerGroupId: 3` and expects `rollingReplacement: false`. |
| 16 | A change to the GitLab target set or to `run_untagged` sets `runtime_changed`; audit metadata gains both fields (A's W3) | YES | `gitlab_connections_changed` and `pool.run_untagged != run_untagged`; asserted both ways. |
| 17 | Pool detail gains `runUntagged`, `gitlabConnectionIds`, `gitlabConnections`; pool options gain `gitlabConnections` (brief) | YES | Tested with the exact `{id, name, baseUrl, scopeKind, scopePath}` shape. The signed-out options body is unchanged (F's recorded deviation, see the advisory notes). |
| 18 | D14: `starts_belong_to_reconciler` true for a pool with GitLab targets; `provision` never starts a GitLab pool's runners (D14) | YES | New signature `(providers, has_gitlab_targets)`. All three callers updated (`resources.rs:2734, 2889, 4717`), and these are the only three `provision` call sites. Create and reconcile are tested. |
| 19 | D17: `delete_runner_resources` is an explicit `match` on `ci_platform`; a GitLab runner never reaches `cleanup_github_runner`, `github_runner_cleanup` or the `githubCleanup` key; the GitLab arm never fails a delete (D17) | YES | Diffed line by line. The GitHub arm keeps its queue, text and key; the Bitbucket arm keeps its 503 refusal and its `DELETE FROM github_runner_cleanup`; the GitLab arm records `gitlabCleanup: complete or deferred`. An unknown platform is an internal error (the 0047 CHECK allows only the three). Tests: success with the right id and base URL, `NotFound` as success, `Upstream` as a deferred delete, a missing connection with no GitLab call, and a pool delete while GitLab fails. |
| 20 | `RunnerAccess` gains `gitlab_connection_id` and `gitlab_runner_id`, loaded by both queries (D17) | YES | Both `runner_access` and `runners_for_pool` select them. |
| 21 | Fleet authorization: GitLab is system-admin-only until grants exist; explicit arms in `ensure_target_access` (GitLab: service principals and system admins), `ensure_pool_access` requires a system admin for a pool with GitLab rows (brief, deviation) | YES | `fleet/service.rs`. The catch-all `else` is gone; an unknown kind is refused. Two in-crate tests in `service_gitlab_tests.rs`. `authorization.rs`, `readable_targets_cte` and API token scopes are untouched, as the brief directs (recorded deviation from the issue text). |
| 22 | `pool_targets` gets an explicit `"gitlab"` arm returning no target, so `save_pool_fleet` never links a `github_*` row (architecture round 2) | YES | One-arm edit in `targets.rs`. `saving_a_gitlab_pools_fleet_settings_links_no_github_target` includes a control case (the organization pool on the same installation does get its target). |
| 23 | `AppState`: `pub gitlab: Arc<dyn GitLabApi>`, private `UnconfiguredGitLab` in `state.rs`, `with_gitlab`; `main.rs` gets no wiring line (brief, pinned with #140) | YES | Matches the pinned shape. `main.rs` edits: the `patch` import, four route lines, `Method::PATCH` in CORS. |
| 24 | PATCH plumbing: `access.rs` parsers, `fleet-api.md` rows, CORS, the `openapi.rs` "Not served yet" assertion (brief) | YES | All present, plus the `token_access.rs` matrix parser. `/api/v1/webhooks/gitlab` still says "Not served yet: implemented by #140". |
| 25 | Remove "Not served yet: implemented by #138." from the four paths (six operations) (brief) | YES | Replaced with real descriptions; the webhook path is untouched. |
| 26 | Shared contract not touched (`gitlab/mod.rs`, `fake.rs`, `fleet/domain.rs`, agent protocol, migrations, reconciler) | YES | `git diff b22c451 HEAD` over those paths is empty. No migration added (D16). |
| 27 | Done-when: HTTP tests cover create, duplicate, bad token, missing scope, missing role, update, verify, rotate, delete-while-targeted, secret non-disclosure; `control_plane_contracts.rs` covers the new routes; responses validate against the spec | YES | 30 in-crate HTTP tests with `FakeGitLab`, 4 binary-level tests, and `openapi_schema::check_response` validating bodies against the spec on both. |

Spec-level edge cases the brief names are handled: duplicate race, delete by pool, fleet target and live runner (each a 409, never a 500), the SET NULL runner after a connection delete, a missing token, and an unreachable GitLab on any delete.

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | Same shape as the Bitbucket connection routes: `require_system_admin`, `SameOrigin`, `AuthUser`, `json!` builders, `audit`, 400 for upstream errors, GET open to any signed-in user with `canManage`. Verbs follow the spec (PATCH for update, 204 for verify and delete, 201 for create). |
| Error handling | PASS | Every `GitLabError` maps to a named 400; `Upstream` detail is logged, not returned. Handlers refuse before any GitLab call on invalid input. A failed create stores nothing (rows and secrets in one rolled-back transaction). The D17 arm degrades to a warning plus a deferred event and never fails a delete. |
| UI/UX match to spec | N/A | No UI. #141 builds against the pinned wire names. |
| Accessibility | N/A | No UI. |
| Architecture gate | PASS | A round 3 PASS and A-dup PASS. A-dup's four `warn` items are optional follow-ups and I don't contradict them. |
| Code organization | PASS | One new submodule following `tart_host.rs`; `resources.rs` keeps only the plumbing the brief lists. Functions are small and commented with the why. No dead code, `TODO`, `unwrap()` or `dbg!` in the diff. `#[cfg_attr(not(test), allow(dead_code))]` on `with_gitlab` is justified (the binary calls it from #146) and is the only suppression. |
| Security | PASS | Admin-only and cookie-only (a bearer token is refused, in both the `access.rs` matrix and the `token_access` test). Token and webhook secret are sealed at rest and never in a response (apart from create/rotate for the secret), log or audit row. The base-URL normaliser keeps credentials out of the plain-text column. Connection-delete checks run in the deleting transaction, so no reference can appear between check and delete. |
| Performance | PASS | No N+1 on a hot path. `selected_gitlab_connections` queries per id, capped at 100 and mirroring Bitbucket's. `pool_has_gitlab_targets` is one indexed `EXISTS` per reconcile or delete. |
| Visual regression | N/A | No visual surface. |
| Naming consistency | PASS | Matches the spec's terms (`gitlabConnectionIds`, `runUntagged`, `gitlabConnections`, `webhookUrl`, `webhookSecret`), `snake_case` in Rust, and the Bitbucket analogues. |
| Test quality (`testing/test-quality.md` §7) | PASS | See below. No "delete or merge" findings. |

### Test-quality audit (rubric §7 over the tests in the diff)

About 43 tests were added or changed: 30 in-crate HTTP in `resources/gitlab_tests.rs`, 4 binary-level in `control_plane_contracts.rs`, 5 model and `pool_targets` tests in `gridops-core/tests/gitlab_pools.rs`, 2 fleet-access tests in `service_gitlab_tests.rs`, and 2 changed unit tests (`access.rs`, `fleet/openapi.rs`).

- **Names one behavior / fails in isolation for the right reason:** PASS. Names state the behavior. The table-driven tests loop over distinct branches, not distinct values of one branch (`gitlab_refusals_on_create…` is one row per `GitLabError` variant and per call that failed; the update-no-mixing test is one row per distinct refusal rule; the `gitlab_pools.rs` matrices carry an explicit control case so a refusal can't pass for the wrong reason, e.g. an unknown scope). T mutation-checked two of the four late additions (the query refusal and the name boundary) and both failed as intended. I re-read the other two from the code and they hit one distinct line each: the empty-PATCH early return and the `gitlabCleanup` key in the runner event.
- **Tier-correct, non-duplicative:** PASS. Model validation sits in core unit-level tests; handler behavior with `FakeGitLab` sits in-crate (the only place the fake can be injected, because the crate has no library target and the integration tests run the binary); the binary-level tests keep to what needs no GitLab answer (401, 403, bearer refusal, 400 on invalid input, list over seeded rows, delete 404 and 409, CORS PATCH, the placeholder's mapped 400). I looked for the same branch pinned at two tiers and found none. The D17 runner-delete tests (success, `NotFound`, `Upstream`, missing connection, pool delete) are five distinct branches of `remove_gitlab_runner` and of the pool-delete path. `a_gitlab_runner_delete_records_…` overlaps the `Upstream` case's setup but pins a different observable (the event's `gitlabCleanup` metadata), so it stays.
- **Behavioral, not structural:** PASS. Assertions are on status, body (validated against the spec), persisted rows, secrets by key, audit rows and runner events. The `FakeGitLab` call log is used for effects the caller can't see otherwise ("GitLab was asked to resolve, then verify", "no `CreateRunner` call", "the delete targeted this id on this instance"), which is the case §2 allows.
- **Proportionate:** PASS. This is a feature story with six new routes, pool plumbing across create, update, detail and options, a predicate change, a delete-path rewrite and an authorization change. About 43 tests for that many distinct branches isn't over-production, and I found no fan-out.
- **Smells (§6):** none found. No assertion-free or tautological test, no unreachable-outcome assertion, no snapshot, no coverage padding, and the `FakeGitLab` state is per-test (no shared mutable state, no wall-clock dependence). Minor note only: the `saving_a_gitlab_pools_fleet_settings…` test removes its temp SQLite file at the end of the test body, so a failed assertion would leave the file behind (the in-crate harness uses a `Drop` guard). That is hygiene, not a rubric failure.
- **Gap, accepted:** no test captures log output to prove no secret is logged. I covered it by inspection above; a tracing-capture harness would be disproportionate for four `warn!` calls whose arguments are visibly secret-free.

## Scope check

F delivered the brief's scope, no more and no less:
- **All of it:** routes, handlers, pool plumbing, D14, D17, authorization, `pool_targets`, `AppState` injection, the PATCH plumbing, and spec and doc edits.
- **Edits outside the "Owns" list, all justified and declared by F:**
  - `async-trait` added to gridops-api's `[dependencies]` (and `Cargo.lock`). `UnconfiguredGitLab` has to `impl` a trait declared with `#[async_trait]`, so the brief's dev-dependency alone can't compile it. An existing workspace crate, so no new third-party dependency.
  - The compile-only `CreateRunnerPool` literal fields in `control_plane_contracts.rs` and the `models.rs` test literals, which the RED handoff assigned to F. No assertion changed.
  - A stricter base-URL normaliser than the minimum (see the advisory notes).
- **Untouched, as required:** migrations, `gitlab/mod.rs`, `fake.rs`, `fleet/domain.rs`, the reconciler, webhooks, UI.
- **Hand-offs for #139 and #141** are recorded in F's handoff and need to be copied into the PR body: the `pool_targets` `"gitlab"` arm is the seam #139 fills; the reconciler's start paths at `main.rs:1167/1312/2352` and `fleet.rs:158` must divert `scope='gitlab'` pools first (until then `fleet.rs:176` fails closed); #141 builds on `gitlabConnectionIds`, `runUntagged`, `gitlabConnections`.

## Verdict

PASS — all acceptance criteria met, spec-compliant, quality acceptable. Ready for O to commit.

## Advisory notes
Non-blocking; none needs a rework cycle.

1. **Upstream outages are 400s.** `GitLabError::Upstream` and `RateLimited` answer 400 because the spec declares no 502 and `ApiError` has none, matching Bitbucket's pattern. A GitLab outage is semantically a 5xx. If the epic wants true 502s, that is a spec plus `error.rs` change for a later issue.
2. **Signed-out `/runner-pools/options` has no `gitlabConnections` key**, though it carries `bitbucketConnections: []`. F kept it off because `token_access.rs:372` pins that body exactly and the UI loads the form data only when signed in. #141 should read the key as optional.
3. **Base-URL normalisation is stricter than the brief's minimum** (credentials, query and fragment refused; default port dropped). It matches the spec's description of the normal form and is pinned by a test. If an operator ever needs a query string in a GitLab base URL, this is where to look.
4. **Spec wording nit:** the DELETE `409` response description in `gridops-v1.json` still says "while a pool or a fleet target references it"; the operation description now also names a live runner. Worth aligning in a spec touch-up.
5. **A-dup's optional follow-ups:** one shared `upsert_runtime_secret` helper (there are now four copies of the upsert) and hoisting `is_unique_violation` if a third caller appears.
6. **Environment:** `fleet_http::release_preflight` should be confirmed green on CI (a non-macOS-ARM64 runner). It is unrelated to #138 and fails the same way without this diff.
7. **Pool-level actions on a GitLab pool** (pause, delete, runner actions) still follow installation membership through `pool_access`, as for every pool; only the connection routes, pool create and update, and the fleet access functions require a system admin. That matches the brief ("system-admin-only until grants exist") and the Bitbucket parity, but #141 and #139 should know it when they add user-scoped GitLab visibility.
