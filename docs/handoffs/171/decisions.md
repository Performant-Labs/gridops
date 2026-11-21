# Decision journal: #171 / #174

## Phase 3 Architecture (A, up-front plan review) - 2026-11-21T08:46:22-08:00
- **Decided:** PASS, with 0 block and 6 warn findings (docs/handoffs/171/handoff-A.md). The plan tightens `verify_access` in place, which is the one seam behind create, token replacement and Verify. It keeps the API's `MissingScope` message, writes into the existing `runner_events` table, and reuses `PoolEventTable`. There is no parallel path. The warns:
  - (1) write the per-runner event inside `gitlab_lifecycle::deregister`, which gains `pool_id` and `runner_id`. That covers all four callers, including `fleet_material :453`, which the brief does not list. `deregister` is the only place that still holds the typed `GitLabError`.
  - (2) add a runner-id-capable sibling of `pool_event` (with `pool_event` delegating to it) instead of a second hand-written INSERT. Sweep rows leave `runner_id` NULL, because of the foreign key and the cascade. Name the event with a constant, `GitLab runner not deleted`.
  - (3) the level is exactly `"warning"`. The API's `"warn"` at `resources.rs:4899` is shown with the success (green) tone, and that needs a follow-up issue outside #171.
  - (4) the dedup state is read back from `runner_events`, as `record_provider_changes` does: one row per pool × `connectionId` × reason code per hour, for the sweep only.
  - (5) the reason text is `GitLabError`'s `Display`, from core; do not port `gitlab_message`. The remediation sentence appears only for `MissingScope` and `Unauthorized`. The existing 502 test now expects a warning row too.
  - (6) `fake.rs` is unchanged, the help text stays a plain string, and the named test homes are extended in place.
- **Assumed:** `docs/handoffs/171/wireframe.html` is approved as drawn (the D approval line is still empty). There is no `survey.md`, so the analogues the brief names stood in for the Reuse map.
- **Hedged:** the remediation wording for non-scope errors is S's call. A fixed only where the choice lives. The `"warn"` level bug in the API is reported for a follow-up and left out of scope on purpose, because it affects the deferred-cleanup rows of every platform.
- **Evidence:** I read:
  - the brief, the brief gate result, handoff-D and the wireframe;
  - `http.rs` (`verify_access`, `error_for`, `missing_scope`), `gitlab/mod.rs`, `fake.rs`, and `http_tests.rs:1115-1153`;
  - `resources/gitlab.rs` (`cleanup_gitlab_runner`, `gitlab_message`), and in `resources.rs` the pool events endpoint and the "Runner deleted" insert;
  - `gitlab_lifecycle.rs` in full, and in `main.rs` `record_provider_changes`, `defer_pool_capacity`, `system_event`/`pool_event`, the failed-start cleanup at `:2887` and `delete_runner`'s GitLab arm;
  - the insert at `gitlab_poll.rs:343`, `pool-events.tsx`, `gitlab.tsx:300-346`, `tests/gitlab-descriptor.test.ts`, and the reconciler test file (`:736-781`);
  - the migrations `0001`/`0047`, covering the `runner_events` foreign keys, the cascade from runners to pools, and the connection index.

  I grepped for `DELETE FROM runners` and found none. I grepped for the level spellings and for every GitLab scope mention in `crates`, `src`, `docs`, `README` and `CHANGELOG`. I checked the reconciler's `Cargo.toml` dependencies and checked every live worktree branch for overlapping files (none). `git diff origin/master HEAD -- src tests crates` is empty.

## Phase 4 Tests, RED (T) - 2026-11-21T09:05:45-08:00
- **Decided:** RED is valid (docs/handoffs/171/handoff-T-red.md). It pins:
  - the scope rule in the `http_tests.rs` table (12 cases);
  - the refused-deletion warning on all four per-runner deletion paths plus the orphan sweep, in `gitlab_lifecycle_tests.rs` (5 new tests, and the 502 test extended);
  - the token help text in `tests/gitlab-descriptor.test.ts` (2 tests).

  It extends the API refusal tests in place, for create, PATCH and Verify. They pin the 400 message and that nothing is saved. They **pass before the feature**, because they inject the error through the fake. A ruled the fake unchanged, so they are contract pins and not RED. The rule's RED is the core table. The sweep dedup is tested by back-dating `runner_events.created_at`. The key is per pool, connection and reason, and one sweep with two refused orphans must write one row.
- **Assumed:**
  - The brief's example remediation sentence is wording S judges. The tests assert only: the connection name, the GitLab runner id (as a whole number), the reason (`manage_runner` or "GitLab request failed"), level `warning`, and that no `glpat-`/`glrt-` token appears.
  - A 502 warning must not mention `manage_runner` (A W5).
  - Metadata keys are not asserted (D: illustrative).
- **Hedged:**
  - No docs test: prose is S's to judge, and T-green greps it.
  - No pool-events UI test: the existing `level === "warning"` tone covers it, and U walks it.
  - The fleet-rollback test asserts the delete was attempted, not that the GitLab runner survives. I first assumed the runner survives, but `delete_runner` later removes it successfully, which is behavior this brief does not change. I corrected the test.
- **Evidence:**
  - Ran `cargo test -p gridops-core --lib gitlab::http::tests` (1 failed: `["create_runner","read_api"]` gives `Ok(())` where `MissingScope("manage_runner")` is expected).
  - Ran `cargo test -p gridops-reconciler gitlab_lifecycle_tests` (6 failed, each at the refusal-count assertion, with setup assertions passing).
  - Ran `npx vitest run tests/gitlab-descriptor.test.ts` (2 failed on the feature text).
  - Ran `cargo test -p gridops-api --bins gitlab_tests` (64 passed).
  - `cargo fmt --check` and `npx eslint` are clean. `npm run typecheck` is clean after `vite build` generated `src/routeTree.gen.ts`.
  - `cargo +1.98.1 clippy -p gridops-core -p gridops-api -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` is clean. The default local clippy 0.1.99 fails on untouched files; CI pins 1.98.1 (`ci.yml:93,113`).

## Phase 5 Implementation (F) - 2026-11-21T09:14:36-08:00
- **Decided:**
  - `verify_access` requires `create_runner`, `manage_runner` and `read_api`, in that order. `api` satisfies each.
  - The per-runner "GitLab runner not deleted" `warning` is written inside `gitlab_lifecycle::deregister`, which now takes `pool_id` and `runner_id`. This covers all four callers. It goes through a new `runner_event` helper that `pool_event` delegates to.
  - The sweep's warning fans out to every linked pool and leaves `runner_id` NULL. It is deduplicated per pool × `connectionId` × reason code per hour, read back from `runner_events` with `json_extract`.
  - The reason text is `GitLabError`'s `Display`. The remedy sentence appears only for `MissingScope` and `Unauthorized`.
  - Writing the event can fail; that failure is logged and never propagated.
- **Assumed:**
  - A refused delete inside `deregister` writes its event before returning the error. Callers still log their existing `tracing::warn!` lines.
  - "Fixed" is an acceptable new subsection under Unreleased in `CHANGELOG.md`.
- **Hedged:**
  - The remedy wording for `Unauthorized` and for other errors is S's to judge.
  - The dedup applies only to sweep rows, so a per-runner row does not suppress a sweep row. At most one extra row per hour results.
- **Evidence:**
  - `cargo test -p gridops-core --lib gitlab::` passed: 36 tests.
  - `cargo test -p gridops-reconciler gitlab_lifecycle_tests` passed: 31 tests, all 6 RED tests now GREEN.
  - fmt and clippy (1.98.1) are clean.
  - `npm run lint`, `npm run typecheck` and `npm test` (603 tests) pass.
  - Full-crate results are in handoff-F.md.
