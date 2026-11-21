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
