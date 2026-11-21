# [GitLab: connection accepts a token without manage_runner, so runners are never deleted](https://github.com/Performant-Labs/gridops/issues/171) (+ [#174](https://github.com/Performant-Labs/gridops/issues/174))

Part of epic #136. Found by #143's live end-to-end run against gitlab.com (check C6). This one PR fixes #171 and #174.

## Need

1. **#171, verification.** `verify_access` in `crates/gridops-core/src/gitlab/http.rs` (~line 299) checks only `create_runner` and `read_api` (or `api`). GitLab 17.1+ requires `manage_runner` for `DELETE /runners/:id`, so a token without it passes save, token replacement and **Verify**, and every runner GridOps registers stays registered in GitLab after its job. On 2026-11-21 all 7 runners of pipeline 3097773323 were orphaned that way.
2. **#171, runtime visibility.** When a deletion does fail at runtime (`deregister_runner` in `crates/gridops-reconciler/src/gitlab_lifecycle.rs` ~line 233, the failed-fleet-start cleanup ~line 516, `main.rs` ~line 2890, and the orphan sweep ~line 705), the reason reaches only a `tracing::warn!` in the reconciler log. An operator never sees it.
3. **#171, docs and form.** `docs/gitlab-runners.md` (~line 27) says GridOps does not check `manage_runner`. The connection form's token help (`src/features/platform-connections/gitlab.tsx`, ~line 342) names only `create_runner` and `read_api`.
4. **#174.** The same help text says the token's owner "needs the Owner role on the project or group". `verify_access` accepts Maintainer or higher on a project and requires Owner on a group (docs already say this).

## Scope

### Verification (core)

- `verify_access` requires all three scopes, checked in the order `create_runner`, `manage_runner`, `read_api`; `api` satisfies each. A missing one returns `GitLabError::MissingScope("<scope>")` as today.
- The fake (`crates/gridops-core/src/gitlab/fake.rs`) models the same rule if it models scopes; keep fake and HTTP client behaviour identical.
- The API already maps `MissingScope` to "The GitLab access token is missing the {scope} scope." (`crates/gridops-api/src/resources/gitlab.rs` ~line 799). Create (~line 139), token replacement (~line 254) and **Verify** (~line 317) all go through `verify_access`, so they all reject. Keep that message format; the docs render the scope name as code.
- Update the module/doc comments that say `api` "grants both" and the `gitlab_live.rs` header comment to list all three scopes.

### Runtime visibility (reconciler)

- When GitLab refuses a runner deletion with an error other than `NotFound` (in particular `MissingScope`, `Unauthorized`), record a **warning** `runner_events` row on the runner's pool (with `runner_id` where there is one), using the existing `pool_event`/`system_event` helper or the same insert shape. The event name is stable (e.g. "GitLab runner not deleted"), and the message names the connection, the GitLab runner id and the client-safe reason, e.g. "GitLab could not delete runner 59802498 of connection gridops-e2e: the GitLab access token is missing the manage_runner scope. Give the connection a token with manage_runner (or api); the orphan sweep retries it." It must never contain the token.
- Cover every deletion path listed in Need 2. For the orphan sweep (no runner row), record the warning on each pool linked to that connection (`runner_pool_gitlab_connections`), and **deduplicate**: a sweep that keeps failing for the same connection and reason must not write a row per runner per sweep. At most one row per pool per connection per reason within an hour (or only when the reason changes) — pick one, document it in a comment, test it.
- Keep the existing `tracing::warn!` lines. The event write's own failure is logged and never aborts deletion or the sweep.
- No migration: `runner_events` already has `level`, `event`, `message`, `metadata`, `pool_id`, `runner_id`. The pool page's Events list (`getRunnerPoolEvents`) shows them; confirm a warning row appears there.

### Form and docs

- Token help text in `gitlab.tsx`: names `create_runner`, `manage_runner` and `read_api` (or `api`), and says the token's user needs the Maintainer role or higher on a project, or the Owner role on a group, and that GridOps checks both before saving. Keep it one short paragraph.
- `docs/gitlab-runners.md`: replace the "but not `manage_runner`" sentence with: GridOps checks all three (or `api`) when you save the connection, replace its token and choose **Verify**. Mention that a deletion GitLab still refuses at runtime shows as a warning in the pool's events. Sweep the rest of `docs/` and `README.md`/`CHANGELOG.md` for any other GitLab scope list and make it match (add a CHANGELOG line under Unreleased if the file has one).

## Out of scope (other sessions own these — do not touch)

- Pool validation and defaults (#172): the runner-pool forms and pool API validation.
- The Tart image script and agent (#173): `scripts/`, the Tart agent code and `docs/macos-runners.md`.

## Constraints

- No deploy. Nothing GitLab ships to Mars until epic #136 closes.
- Rust builds share this Mac: keep `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2` (`.claude/settings.local.json`).
- No token in any error, log line, event, audit row or test output.

## Done when

- `http_tests.rs`'s scope table (~line 1134) covers: `create_runner`+`manage_runner`+`read_api` passes; `api` alone passes; `create_runner`+`read_api` fails with `MissingScope("manage_runner")`; and the existing missing-`create_runner`/`read_api` cases still name the right scope.
- API tests (`crates/gridops-api/src/resources/gitlab_tests.rs`) show a token with `create_runner` and `read_api` only is refused with 400 "The GitLab access token is missing the manage_runner scope." on **create**, on **token replacement** and on **Verify**, and nothing is saved/changed on the refused create and replacement.
- Reconciler tests show: a runner whose GitLab deletion fails with `MissingScope("manage_runner")` writes one warning `runner_events` row on its pool naming the connection, runner id and `manage_runner`; a `NotFound` writes none; the orphan sweep failing repeatedly for the same connection writes no more than the dedup rule allows; the event text never contains the token.
- Vitest covers the help text: it names all three scopes and "Maintainer" for a project and "Owner" for a group, and no longer says Owner for a project.
- The headless UI walkthrough opens the GitLab connection form and confirms the help text, and opens a GitLab pool's events list showing a deletion warning (seeded).
- `docs/gitlab-runners.md` no longer says `manage_runner` is unchecked.
- `npm run lint`, `npm run typecheck`, `npm test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p gridops-core`, `cargo test -p gridops-api` and `cargo test -p gridops-reconciler` pass.
