# Handoff-F: Phase 5 - GitLab token needs manage_runner; refused deletions become pool warnings (#171, #174)

**Date:** 2026-11-21
**Branch:** claude/gitlab-171-scopes-cbe8b5
**Issue:** #171 (+ #174)

## Rework: diff gate round 1 (Phase 6, 2026-11-21)

Addresses `docs/handoffs/171-diff-result-r1.md`, finding by finding:

- **B-1 (fixed, production).** In `crates/gridops-reconciler/src/gitlab_lifecycle.rs`, a new `client_safe_reason(&GitLabError)` builds the event message's reason. `Upstream(_)` becomes plain "GitLab request failed", the same text the API's `gitlab_message` uses. Every other variant keeps its fixed `Display` text. `MissingScope`'s scope name is filtered by the HTTP client to lowercase letters, `_` and spaces (see evidence.md). `refused_deletion_message` no longer interpolates `{error}`. The full error, detail included, is still returned from `deregister` and logged by its callers, and the sweep's `tracing::warn!` still logs `error = ?error`. The match lists every variant with no wildcard, so a new variant added later must choose its client-safe text. The F evidence entry that claimed `Display` was safe for `Upstream` is now corrected.
- **W-1 (fixed, production).** `warn_linked_pools` now hands each pool to a new `warn_pool`, which runs the dedup check and then the insert. If one pool fails, that failure is logged with `pool_id` and the loop moves on to the next pool, so a single bad pool no longer suppresses the other pools' warnings. Only a failure of the linked-pools query itself is still returned, and `sweep_connection` logs it.
- **B-2 (not production; delivered by Phase 9).** The headless UI walkthrough is U's Phase 9. The pipeline runs it after the diff gate and the A-dup gate (`workflow-coding-pipeline.md`: "the diff gate fires after Phase 7 (T-green), before U/S"). So it was never deferred out of this PR. It simply had not run yet when the gate reviewed. My earlier wording "left to later phases" made it look deferred, and Known issues now says this plainly. To help U, here is the seed for the deletion warning. Run it against the walkthrough database, with an existing GitLab pool's id in place of `<pool-id>`. It is the same shape `runner_event` inserts (`main.rs:3981`):

      INSERT INTO runner_events (id,pool_id,runner_id,level,event,message,metadata,created_at)
      VALUES (lower(hex(randomblob(16))),'<pool-id>',NULL,'warning','GitLab runner not deleted',
        'GitLab could not delete runner 59802498 of connection gridops-e2e: the GitLab access token is missing the manage_runner scope. Give the connection a token with manage_runner (or api); the orphan sweep retries it.',
        '{"connectionId":"seed","connection":"gridops-e2e","gitlabRunnerId":59802498,"reason":"missing_scope:manage_runner"}',
        CAST(strftime('%s','now') AS INTEGER)*1000);

- **NV-1 to NV-4.** Settled with source evidence in `docs/handoffs/171/evidence.md` under "F (Phase 6 rework)":
  - NV-1: the preflight test also fails at the pre-branch commit `7164fda`, run in a detached worktree on this Mac.
  - NV-2: the fake's `verify_access` has no scope model.
  - NV-3: the stored-nothing assertion runs after all seven cases.
  - NV-4: grep output shows no stale two-scope list.
- **W-2 (intent confirmed, no change).** This behaviour is intended. A sweep that keeps hitting `RateLimited` or `MissingRole` writes at most one row per pool, per connection, per reason code, per hour. That caps it at 24 rows a day per pool, and the warning stays visible for as long as cleanup is blocked. `MissingRole` and `RateLimited` get the generic remedy ("The orphan sweep retries it."), because neither has a token fix the operator can act on in the connection form.
- **NIT-1 (test-only, for T).** `HOUR_AND_MORE` lives in `gitlab_lifecycle_tests.rs`. F does not edit tests. See "Tests that look wrong".
- **NIT-2 (declined).** The help sentence is word for word the copy in the operator-approved wireframe (`wireframe.html:36`, "One short paragraph"). Splitting it would be a deviation from the wireframe, so I left it.

## What was done
- `crates/gridops-core/src/gitlab/http.rs`: `verify_access` now requires `create_runner`, `manage_runner` and `read_api`, checked in that order. `api` satisfies each one. The comment that said "`api` grants both" now explains why each scope is needed.
- `crates/gridops-core/tests/gitlab_live.rs`: the header comment lists all three scopes. This is a comment-only change.
- `crates/gridops-reconciler/src/main.rs`:
  - New `runner_event(database, pool_id, runner_id, level, event, message, metadata)` helper. `pool_event` now delegates to it with `runner_id = None`, so none of its existing callers change.
  - The failed-start cleanup (`start_admitted_runner`) and `delete_runner`'s GitLab arm now pass `pool.id`, and the cleanup also passes the runner id, to `gitlab_lifecycle`.
- `crates/gridops-reconciler/src/gitlab_lifecycle.rs`:
  - `deregister` gains `pool_id` and `runner_id`. When GitLab refuses the delete with anything other than `NotFound`, it writes a `warning` "GitLab runner not deleted" row through `record_refused_deletion`, linked to the runner row. It still returns the original error. If writing the event fails, that failure is only logged.
  - All four per-runner callers pass the pool and row: `deregister_runner`, the failed manager start, `fleet_material`'s earlier-attempt delete and `fleet_rollback`.
  - New `warn_linked_pools` is called from `sweep_connection`'s refusal arm. It fans out over `runner_pool_gitlab_connections` and deduplicates to one row per pool, connection and reason code per hour, reading back from `runner_events`. Its failure is logged and never stops the sweep.
  - New constants `DELETION_REFUSED_EVENT` and `SWEEP_WARNING_WINDOW_MS`, and helpers `refusal_code` and `refused_deletion_message`.
  - The module docs are updated.
- `src/features/platform-connections/gitlab.tsx`: the token help is now one sentence plus "GridOps checks both before saving.". It names `create_runner`, `manage_runner` and `read_api` (or `api`), and asks for Maintainer or higher on a project or Owner on a group.
- `docs/gitlab-runners.md`: the "but not `manage_runner`" sentence is replaced. GridOps now checks all three scopes (or `api`) on save, on token replacement and on **Verify**. A deletion GitLab refuses at runtime shows as a **GitLab runner not deleted** warning in the pool's events. The sweep section describes the per-pool hourly dedup.
- `CHANGELOG.md`: added a "Fixed" entry under Unreleased.

## Design decisions
- **Where the per-runner event is written.** It is written inside `deregister`, following A's W1. That is the only place that still has the typed `GitLabError`, and all four callers go through it. Local failures are only returned, not recorded as GitLab refusals. These are a missing connection, an unreadable vault token and an invalid id.
- **Message.** The message reuses `deregister`'s existing prefix: "GitLab could not delete runner N of connection X: {client-safe reason}." The reason comes from `client_safe_reason`: "GitLab request failed" for `Upstream`, the fixed `Display` text otherwise (since the round-1 rework). The remedy sentence depends on the error:
  - `MissingScope(scope)`: "Give the connection a token with {scope} (or api); the orphan sweep retries it."
  - `Unauthorized`: "Give the connection a valid token with create_runner, manage_runner and read_api (or api); …"
  - Anything else: "The orphan sweep retries it."

  This follows A's W5, so a 502 does not point the operator at `manage_runner`. The text never has a token in it, and no upstream detail either (B-1).
- **Dedup rule.** The rule is a one-hour window per pool, `connectionId` and reason code, and it applies to sweep rows only (`runner_id IS NULL`). It is read back from `runner_events` with `json_extract` on `metadata`. The repo already uses `json_extract` in `gridops-core` fleet registry queries.
  - Alternative considered: re-warning only when the reason changes. Rejected per A's W4: a delete that succeeds writes nothing, so a token that is fixed and later breaks again the same way would never warn.
  - The reason is stored as a stable code from the variant (`missing_scope:<scope>`, `unauthorized`, `upstream`, …), not as message text.
  - The comment on `warn_linked_pools` documents all of this.
- **Metadata.** `{connectionId, connection, gitlabRunnerId, reason}`. The dedup reads `connectionId` and `reason`; the other two are for display.
- **Sweep rows leave `runner_id` NULL.** The marker's row may be gone, and `runner_id` is a foreign key.

## Reuse / extend-vs-new
- The scope rule extends `verify_access` in place, and the API's existing `MissingScope` message is reused unchanged.
- Events go into the existing `runner_events` table through the reconciler's shared insert. `pool_event` now delegates to the new runner-id-capable `runner_event` (A's W2), so there is no second hand-written INSERT.
- The UI reuses `PoolEventTable` as it is. The help text uses the existing string `help` slot.
- `fake.rs` is unchanged (A's W6). There is no new table, no migration and no new component.

## Architecture notes for A
- Two reconciler-internal signatures changed: `gitlab_lifecycle::deregister` and `deregister_runner` (both `pub(super)`) now take the pool and runner ids.
- There is one new private helper in `main.rs` (`runner_event`). No dependency direction changed and no public interface or schema changed.
- The contract change is in the core rule. `verify_access` now refuses tokens it used to accept: `create_runner` + `read_api` without `manage_runner`.

## Deviations from spec / wireframe
None.
- The CHANGELOG line is under a new "### Fixed" heading in Unreleased.
- The help text is plain text with no inline code, which A's W6 settled.

## Tier 1 self-check (incl. tests now GREEN)
Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`:

```
cargo test -p gridops-core --lib gitlab::            -> test result: ok. 36 passed; 0 failed
cargo test -p gridops-reconciler gitlab_lifecycle_tests -> test result: ok. 31 passed; 0 failed
  (all six RED tests now ok: a_failed_gitlab_delete_..., a_refused_gitlab_delete_is_a_pool_warning_...,
   a_refused_delete_after_a_failed_manager_start_..., a_refused_delete_in_a_fleet_rollback_...,
   a_refused_delete_of_an_earlier_attempts_runner_..., the_sweeps_refused_deletions_warn_each_linked_pool_...)
cargo fmt --all --check                               -> clean
cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl -> clean
npm run lint                                          -> clean (--max-warnings=0)
npm run typecheck                                     -> clean
npm test                                              -> Test Files 53 passed (53), Tests 603 passed (603)
```

Full-crate runs (`cargo test -p gridops-core`, `-p gridops-api`, `-p gridops-reconciler`): 24 test binaries, 838 passed, 1 failed.

The one failure was already failing before this change, and it depends on the host:
- **Test:** `gridops-api --test fleet_http release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`.
- **Failure:** on this Mac, the fixture's "Linux" machine is reported as `PASS  macos-machine: macOS ARM64 machine ready ... fleet-host`, so the expected `FAIL  macos-machine` line never appears.
- **Baseline:** I reverted `http.rs`, the only file in this diff that is in gridops-api's dependency graph, to HEAD and reran the test alone. It still fails (`0 passed; 1 failed`). I then restored `http.rs`.
- **Scope:** the test is fleet and release-preflight code that this diff does not touch. Every GitLab test in gridops-api passes, including the three `manage_runner` refusal pins.

Round-1 rework re-run (CARGO_BUILD_JOBS=2, RUST_TEST_THREADS=2):

```
cargo fmt --all --check                                -> clean
cargo test -p gridops-reconciler                       -> test result: ok. 119 passed; 0 failed
cargo +1.98.1 clippy -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl -> clean
baseline at 7164fda (pre-branch): cargo test -p gridops-api --test fleet_http release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine
                                                       -> FAILED. 0 passed; 1 failed (panic at release_preflight.rs:234:5)
```
No TypeScript, core or API production file changed in the rework, so the npm, core and API results above still hold.

## Evidence appendix
docs/handoffs/171/evidence.md

## Tests that look wrong (for T)
- None are wrong. One test-only nit from the diff gate (NIT-1) is T's call: in `the_sweeps_refused_deletions_warn_each_linked_pool_once_an_hour_per_reason`, `HOUR_AND_MORE` is `2 * 3_600_000`, which the name undersells. A rename or a comment would make it clearer. Behaviour is unaffected.

## Known issues
- `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on this macOS host, both with and without this change (see Tier 1). It is unrelated to #171.
- The `"warn"` level on the API's "Runner deleted" row (`resources.rs:4899`) is out of scope, per A's W3; it needs a follow-up issue.
- The headless UI walkthrough is delivered by Phase 9 (U), which the pipeline runs after the diff gate and A-dup. It is not deferred out of this PR. The seed SQL for the deletion warning is in the Rework section above.
- A sibling the diff does not touch: `reconcile` writes "Pool reconciliation failed" with `error.to_string()` (`main.rs:677-681`). For a failed registration, that string includes `GitLabError`'s `Display`, `Upstream` detail and all. This predates #171. The real HTTP client only ever puts fixed text into `Upstream` (`http.rs:440-495`: `transport_error`, "GitLab returned {status}", the URL error), and no body or token reaches it. I did not change it, because that would be a drive-by edit outside the brief. It is a candidate for a follow-up issue if the API's "no upstream detail in client-facing text" rule should also cover reconciler events.

## Files changed
- crates/gridops-core/src/gitlab/http.rs
- crates/gridops-core/tests/gitlab_live.rs
- crates/gridops-reconciler/src/gitlab_lifecycle.rs
- crates/gridops-reconciler/src/main.rs
- src/features/platform-connections/gitlab.tsx
- docs/gitlab-runners.md
- CHANGELOG.md
