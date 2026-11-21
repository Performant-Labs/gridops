# Handoff-T-red: Phase 4 - GitLab token needs manage_runner; refused deletions become pool warnings (#171, #174)

**Date:** 2026-11-21
**Branch:** claude/gitlab-171-scopes-cbe8b5
**Brief / wireframe reviewed:** docs/handoffs/171-brief.md, docs/handoffs/171/wireframe.html, docs/handoffs/171/handoff-D.md, docs/handoffs/171/handoff-A.md

## A precondition
Confirmed: A returned PASS on the plan (Phase 3, `handoff-A.md`, 0 block / 6 warn). The tests follow A's warns. They assert level `warning` exactly (W3), the event name `GitLab runner not deleted` (W2), `runner_id` linked on per-runner rows and NULL on sweep rows (W2), and sweep-only dedup per pool, connection and reason per hour, tested by back-dating rows (W4). The 502 test now expects a row (W5). `fake.rs` is untouched and the help text is read as a plain string (W6). The tests extend the named homes in place, with no new harness or file.

## Tests authored

Staged: `crates/gridops-core/src/gitlab/http_tests.rs`, `crates/gridops-api/src/resources/gitlab_tests.rs`, `crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs`, `tests/gitlab-descriptor.test.ts`.

### Core (unit, against the in-process mock GitLab): the rule itself
- **`verify_access_reports_the_first_scope_the_token_lacks`** (extended, 8 → 12 cases). This pins the rule: all three of `create_runner`, `manage_runner`, `read_api` are required, checked in that order, and `api` stands in for each.
  - Cases that pass: all three scopes; `api` alone; `api` + `read_user`.
  - `create_runner` + `read_api` (the token from the incident) → `MissingScope("manage_runner")`.
  - The ordering cases: `create_runner` alone, and `create_runner` + `read_repository`, → `manage_runner`; `create_runner` + `manage_runner` (± `read_repository`) → `read_api`; `manage_runner` + `read_api` → `create_runner`.
  - The existing missing-`create_runner` cases are kept.
  - Tier: this is the cheapest tier that sees the real `GitLabClient`. The fake does not model scopes (A W6).

### API (integration through the real routers, FakeGitLab): wiring and message, pass before the feature
- **`gitlab_refusals_on_create_are_400s_that_name_the_reason_and_store_nothing`**: new case "missing manage_runner". The response is 400 with "The GitLab access token is missing the manage_runner scope." No connection or secret is stored; the existing count check covers that.
- **`patch_with_a_new_token_verifies_it_against_the_stored_scope_before_storing_it`**: a refused token-replacement PATCH answers 400 with that exact message. The stored token is unchanged and the new token is not echoed.
- **`verify_answers_204_or_a_400_naming_what_the_token_lacks`**: Verify answers 400 with that exact message.
- **These pass today. They are contract pins, not RED.** They inject `MissingScope("manage_runner")` through `fail_next`, and the API's `MissingScope` message mapping already exists. A told us to leave the fake unchanged, so the API tier cannot fail before the feature. They pin the brief's "Done when" items: the message format, 400 on all three routes, and nothing saved or changed. The RED for the rule lives in the core table above. The core table and these pins together cover the path from rule to response.

### Reconciler (integration: real DB, FakeGitLab, stand-in manager / fleet agent)
Shared helpers: `DELETION_REFUSED` (`"GitLab runner not deleted"`), `refusals(pool)`, `names_number`, and `assert_refusal`.
- `assert_refusal` checks that the level is exactly `warning` and that the message names the connection, the GitLab runner id (as a whole number) and the reason.
- It also checks that neither the message nor the metadata holds `glpat-` (the connection's access token) or `glrt-`.
- No test asserts metadata keys, because D marked them illustrative.

The tests:
- **`a_failed_gitlab_delete_does_not_fail_the_pass_and_the_sweep_retries_it`** (extended, A W5). An `Upstream` 502 on delete now writes one warning linked to the runner, naming "Acme API", runner 1 and "GitLab request failed". The message does **not** mention `manage_runner`, because the remediation sentence is only for scope and auth problems.
- **`a_refused_gitlab_delete_is_a_pool_warning_and_a_runner_already_gone_is_not`** (new; the `deregister_runner` path).
  - Phase 1: an exited runner's delete answers `NotFound` → no row.
  - Phase 2: the replacement's delete answers `MissingScope("manage_runner")` → exactly one warning on `pool-gl`, `runner_id` = that row, naming Acme API, runner 2 and `manage_runner`.
  - The GitLab runner is left for the sweep, and no token is stored anywhere.
- **`a_refused_delete_after_a_failed_manager_start_is_a_pool_warning`** (new; the `main.rs` failed-start cleanup). The manager fails the start and the cleanup delete is refused → one warning linked to the failed row (runner 1, `manage_runner`).
- **`a_refused_delete_in_a_fleet_rollback_is_a_pool_warning`** (new; `fleet_rollback`). The bootstrap store fails, so the rollback deletes the runner, and that delete is refused → one warning linked to the runner row.
  - The rollback keeps the registration when the delete fails. The row's later `delete_runner` then removes it successfully and writes no second row.
- **`a_refused_delete_of_an_earlier_attempts_runner_is_a_pool_warning`** (new; `fleet_material`'s earlier-attempt delete, the fourth caller A found).
  - Setup: the admitted row is given an earlier GitLab runner (77), as if a previous attempt registered and stopped.
  - Result: the refused delete writes one warning naming runner 77, linked to the row, and the start still goes on (one `start_runner`).
- **`the_sweeps_refused_deletions_warn_each_linked_pool_once_an_hour_per_reason`** (new; the orphan sweep plus the dedup rule). Acme is linked to pool-a and pool-b; Other is linked to pool-a. Acme has 2 orphans and Other has 1.
  - Sweep 1, all refused for `manage_runner`: pool-a gets 2 rows (one per connection, not one per runner) and pool-b gets 1. Every row has `runner_id` NULL and names its connection, a refused runner id and `manage_runner`.
  - Sweep 2, same reason within the hour: retried (6 delete calls), no new rows.
  - Sweep 3, Acme refused `Unauthorized` within the hour: one new row on each pool, giving (3, 2).
  - Rows back-dated by 2 h, then the same reason: one more row per pool and connection, giving (5, 3).
  - A trigger makes the warning insert fail: the sweep still deletes Acme's other orphan and goes on to sweep Other. This guards "the event write's failure never aborts the sweep".
  - No token is stored.
  - Tier: the dedup state is read back from `runner_events` (A W4), so the window is tested by back-dating rows, with no clock.

### Frontend (Vitest, unit on the descriptor)
- **`names the create_runner, manage_runner and read_api (or api) scopes in the token help`** (replaces the old Owner/scopes test). The help names all three scopes and a standalone `api`.
- **`asks for Maintainer or higher on a project and Owner on a group, never Owner on a project`** (#174). The help matches `/Maintainer[^.]*project/` and `/Owner[^.]*group/`, and does **not** match `/Owner[^.,;]*project/`.
  - I checked the regexes against a candidate text that satisfies the brief (all four hold) and against the old text (it fails the new checks).

### Deliberately not authored
- **No docs test.** The `docs/gitlab-runners.md` and CHANGELOG wording is prose. Pinning it would test implementation text. T-green will grep it, and S judges the wording.
- **No pool-events UI test.** `PoolEventRow` already gives the amber tone for `level === "warning"`. The reconciler tests pin that exact level. U walks the seeded warning row live.

## RED confirmation

Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`.

```
$ cargo test -p gridops-core --lib gitlab::http::tests
test gitlab::http::tests::verify_access_reports_the_first_scope_the_token_lacks ... FAILED
panicked at crates/gridops-core/src/gitlab/http_tests.rs:1159:9:
assertion `left == right` failed: scopes ["create_runner", "read_api"]
  left: Ok(())
 right: Err(MissingScope("manage_runner"))
test result: FAILED. 18 passed; 1 failed
```

```
$ cargo test -p gridops-reconciler gitlab_lifecycle_tests
a_failed_gitlab_delete_does_not_fail_the_pass_and_the_sweep_retries_it   :840  refused.len()  left: 0 right: 1
a_refused_gitlab_delete_is_a_pool_warning_and_a_runner_already_gone_is_not :902 "one warning for the one refusal: []"  left: 0 right: 1
a_refused_delete_after_a_failed_manager_start_is_a_pool_warning          :946  refused.len()  left: 0 right: 1
the_sweeps_refused_deletions_warn_each_linked_pool_once_an_hour_per_reason :1413 "one per pool and connection, not one per runner: [] []"  left: (0, 0) right: (2, 1)
a_refused_delete_in_a_fleet_rollback_is_a_pool_warning                   :2128 refused.len()  left: 0 right: 1
a_refused_delete_of_an_earlier_attempts_runner_is_a_pool_warning         :2180 refused.len()  left: 0 right: 1
test result: FAILED. 25 passed; 6 failed
```

Every reconciler failure is the refusal-row assertion. Each test's setup assertions pass before it: the delete was refused, the runner left service, the start went on.

```
$ npx vitest run tests/gitlab-descriptor.test.ts
FAIL names the create_runner, manage_runner and read_api (or api) scopes in the token help
  AssertionError: expected 'The token needs the create_runner and…' to contain 'manage_runner'
FAIL asks for Maintainer or higher on a project and Owner on a group, never Owner on a project
  AssertionError: expected 'The token needs the create_runner and…' to match /Maintainer[^.]*project/
Tests  2 failed | 51 passed (53)
```

```
$ cargo test -p gridops-api --bins gitlab_tests
test result: ok. 64 passed; 0 failed   (the three API pins pass, as expected; see above)
```

The test code itself is clean:
- `cargo fmt --check` is clean (`cargo fmt` changed only my two Rust test files).
- `npx eslint tests/gitlab-descriptor.test.ts` is clean.
- `npm run typecheck` is clean, once a `vite build` generates the git-ignored `src/routeTree.gen.ts` in this fresh worktree.
- I ran clippy on the three crates the way CI does: `cargo +1.98.1 clippy … --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl`. It is clean.

## Ready for F
RED is valid. F may implement against these tests. The core table pins the rule, and the six reconciler tests pin the warning on every deletion path plus the sweep dedup. Two Vitest tests pin the help text.

Notes for F and later phases:
- **The local default clippy (0.1.99) fails `-D warnings`, so use the 1.98.1 toolchain.** The failures are in code this cycle does not touch: `unused_async_trait_impl`, and the "assert! to check that a value is empty" lint in `autoscaling.rs`, `overview_queue.rs` and elsewhere. CI pins 1.98.1 and allows `unused_async_trait_impl` (`.github/workflows/ci.yml:93,113`). Run clippy as `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl`.
- **The fleet-rollback test expects exactly one warning.** The refused rollback delete writes it; the later successful delete from `delete_runner` must write none.
- **The sweep test needs dedup within a single sweep as well as across sweeps.** Two refused orphans of one connection in one sweep yield one row per pool. The dedup key must include the connection: pool-a gets one row per connection.
