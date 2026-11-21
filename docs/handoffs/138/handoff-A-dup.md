# Handoff-A-dup: Phase 7 - GitLab platform-connection and pool-target API (#138)  (anti-duplication gate)

**Date:** 2026-11-20
**Branch:** claude/gitlab-138-api-44d3a8
**Diff base:** b22c45134c94ddaa745e8de577f61d6669e15e78 (master)   **Diff head:** bcd177c9239c5b94fda00265210b57f50c77b0fa
**Reuse map:** docs/handoffs/138-brief.md (Clarifications: code layout, pools, D14/D17, webhook URL)
**Verdict:** PASS

## Summary
F extended the analogous objects the brief named rather than building parallel paths. Pool create, update, detail, options and delete are edited in place. The generalised `public_webhook_url` replaces a third URL builder. The fleet access functions and `pool_targets` gain explicit arms. The only new object is `resources/gitlab.rs`, which the brief (A round 1) required, following `tart_host.rs`. Edits outside the owned list are limited to the brief's named functions.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `crates/gridops-api/src/resources/gitlab.rs:670` | `store_secret` is a fourth copy of the `runtime_secrets` upsert (also `github_app.rs:202`, `agent.rs:357`, `state.rs:241`). There is no shared helper to extend, so this is not a parallel path. | Optional follow-up: one shared `upsert_runtime_secret` helper. |
| 2 | warn | `crates/gridops-api/src/resources/gitlab.rs:741` | A local `is_unique_violation(&ApiError)` helper, where Bitbucket and pool code inline the check (`resources.rs:3995`, `:1811`). No dominant shared pattern. | Optional follow-up: hoist it if a third caller appears. |
| 3 | warn | `crates/gridops-api/src/resources.rs:1636` | The update-time no-mixing check restates part of `validate_gitlab_targets` (models.rs). Intentional: an update carries no scope, so the handler checks against the stored scope, as the models.rs doc comment says. | None. |
| 4 | warn | `crates/gridops-api/src/resources.rs:1867,2704` | The `runner_pool_gitlab_connections` insert loop is repeated in create and update, mirroring Bitbucket's existing duplicated loops. | None; matches neighbours. |

Verified against the Reuse map:
- **Connection handlers:** they mirror the Bitbucket ones (`require_system_admin`, `SameOrigin`, `audit`, sealed secrets, unique-violation to 409) in the brief-mandated submodule. `resources.rs` keeps only the mod line, re-exports, pool plumbing, the D14 predicate, the D17 match and the URL helper.
- **Pool plumbing:** create, update, detail and options are extended in place. `selected_gitlab_connections` copies the shape of `selected_bitbucket_connections`. There is no second pool path.
- **Webhook URL:** `effective_webhook_url` delegates to the generalised `public_webhook_url(base, path)`, and GitLab calls the same builder.
- **D17:** `delete_runner_resources` is one explicit `match` on `ci_platform`. The GitHub and Bitbucket arms keep their behaviour, the GitLab arm is added, and no second delete function exists.
- **Fleet:** `ensure_pool_access`, `ensure_target_access` and `pool_targets` are edited in place and only in the functions the brief names. The `models.rs` change is the brief's fields plus two validators beside `validate_bitbucket_connections`.
- **State:** `AppState` gains `gitlab`, `UnconfiguredGitLab` and `with_gitlab` exactly as pinned with #140. `main.rs` changes only the route lines, the `patch` import and PATCH in CORS.
- **Dependency direction:** the layering is unchanged. Handlers call state and sqlx as Bitbucket's do, and `gridops-core` does not import `gridops-api`.
- **Dependency:** `async-trait` in gridops-api is an existing workspace dependency, needed for the `UnconfiguredGitLab` impl.

No duplication; extension is clean.

## Notes for F
None (PASS). The warn items are optional follow-ups for O to weigh.
