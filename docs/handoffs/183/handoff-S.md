# Handoff-S: Phase 8 - Retire a GitLab connection's fleet targets on removal (#183)

**Date:** 2026-11-22
**Branch:** claude/gitlab-183-retire-target (head 365f684)
**Issue:** #183 (epic #136)
**Brief:** docs/handoffs/183-brief.md
**Handoff-T reviewed:** docs/handoffs/183/handoff-T-green.md (and handoff-T-red.md)
**Handoff-A reviewed:** docs/handoffs/183/handoff-A.md (plan review R2, PASS) and handoff-A-dup.md (anti-duplication gate, PASS)
**Handoff-F reviewed:** docs/handoffs/183/handoff-F.md
**Operator-facing report:** not produced. This is a non-visual audit; see below.

## A precondition
Confirmed. A returned PASS on the plan (R2, 0 block / 4 warn) and PASS on the anti-duplication gate (0 block / 2 warn). No BLOCK.

## T precondition
Confirmed. `handoff-T-green.md` reports "Blocking issues: None". T repaired two stale tests (test-only), and the suite is green apart from one pre-existing macOS-only failure (`release_preflight`, not touched by this diff).

## Visual-diff-tool precondition
Not applicable to this story, and I did not return CANNOT-AUDIT. The only rendered-surface change is one added sentence in a confirmation string (`removeConfirm` in `src/features/platform-connections/gitlab.tsx`). There is no layout, CSS, component or new UI surface, so there is no wireframe or preview to diff against at 360/768/1280, and a pixel diff would compare identical layouts. The brief says "a copy change only, with no new UI surface", and A and T both recorded the wireframe/UI gate as N/A. The operator's standing rule is also headless-only work with no browser or screenshots unless asked. The copy is verified as text (below), and the frontend tests that cover that file were run. No HTML report is required for a non-visual audit.

## Preview / spec sanity check
No source-of-truth defect found, so no ADVISORY-HOLD.
- The brief's build items 1 and 2 still offer options ("`ON DELETE SET NULL`", "the architect decides") that the binding "Architecture decisions" section settles. A flagged this as warn 1 and told T and F to follow the decisions; both did. No contradiction in the contract itself.
- The schema design (NO ACTION FK kept, CHECK widened only for a retired row, a single retire transition in the trigger) is consistent with the existing fleet-host retire/delete precedents (`0040`) and 0047's rebuild style.
- API contract: DELETE answers 204/404/409 as before; a grant on a retired target answers 404 `not_found`, the same as a retired host. No new route. Consistent.

## Checks I ran myself
`CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, `-p <crate>` runs, shared target dir from the untracked `.cargo/config.toml`:
- `cargo test -p gridops-core --test fleet_schema --test gitlab_pool_fleet`: `fleet_schema` 17 passed, `gitlab_pool_fleet` 8 passed.
- `cargo test -p gridops-core --lib retired`: 5 passed (includes `a_retired_gitlab_target_is_not_valid_for_its_pool_and_is_never_linked`).
- `cargo test -p gridops-api --test control_plane_contracts`: 39 passed, 0 failed (includes the three new HTTP tests and the extended DELETE test).
- `cargo test -p gridops-api --bin gridops-api`: 130 passed, 0 failed (the stale test is gone; the repaired OpenAPI test passes).
- Frontend: `vitest run tests/connections-page.test.ts tests/gitlab-connections.test.ts tests/gitlab-descriptor.test.ts`: 87 passed. No test pins the old GitLab confirm text (the one match, in `connections-page.test.ts:245`, is Bitbucket's string, which is unchanged).
- `git status` is clean after my runs; I changed nothing except this handoff.
- I did not re-run the full workspace, clippy or the redocly lint; T and F both report them clean, and the diff since T's run is only A's handoff commit.

## Spec compliance

| # | Spec requirement (brief) | Verified how | Compliant |
|---|---|---|---|
| 1 | Migration `0048`: retired state on `fleet_ci_targets`, 0047-style rebuild (PRAGMA off/on, trigger re-created, no `IF NOT EXISTS`) | Read `migrations/0048_fleet_target_retire.sql`: `-- no-transaction`, `PRAGMA foreign_keys` off, own `BEGIN`/`COMMIT`, `_fleet_0048_fk_guard`, table STRICT, trigger re-created under its name and message. `retired_at INTEGER` appended last. `the_0048_rebuild_keeps_every_target_and_reference` passes. | YES |
| 1 | A retired GitLab target no longer blocks deleting its connection; history keeps its references | Retire is one UPDATE (`retired_at` set, `gitlab_connection_id` NULL); FK stays NO ACTION (binding decision 1). `retiring_a_gitlab_target_frees_its_connection_and_is_final` and HTTP test (b) pass. | YES |
| 1 | Trigger refuses identity changes, allows only the retire transition, never un-retiring | WHEN clause: `OLD.retired_at IS NULL AND NEW.retired_at IS NOT NULL`, every identity column `IS OLD`, connection same or NULL. Test refuses 4 pre-retire and 6 post-retire statements. | YES (see advisory 1) |
| 1 | `FLEET_ADDED_COLUMNS` gains `retired_at`; `canonical_key` uniqueness stated | `fleet_schema.rs` updated. Key stays UNIQUE and embeds the connection's UUID v4 (decision 2, confirmed in handoff-A). | YES |
| 2 | Same `BEGIN IMMEDIATE` transaction as the delete; today's refusals in today's order (pools, then live runners) | Handler uses `begin_with("BEGIN IMMEDIATE")`; pool refusal, runner refusal, then the core function. Refusal-order test pins pool, runner, live work. | YES |
| 2 | New refusal while a placement (`p.target_id`) is not terminal or holds an unreleased allocation, or an intent is `queued`; one shared fragment with `LIVE_WORKLOADS` | `live_placement!()` `macro_rules!` fragment feeds both `LIVE_WORKLOADS` (behaviour unchanged) and `LIVE_CONNECTION_WORK` through `concat!`; both stay `&'static str`. | YES |
| 2 | An admitted intent is history and does not block; the 409 names what is running; nothing cancelled silently | Query counts only placement liveness and `status='queued'`. Test step 6 returns 204 with a leftover admitted intent. 409 reads "Fleet work of pool gl-live still runs or waits on Acme app. Stop it or let it finish first." `workload_intents.pool_id` is NOT NULL, so the pool join cannot drop live work. | YES |
| 2 | Retire every target of the connection; revoke `host_target_grants`, `host_session_target_grants`, `profile_ci_targets` (`revoked_at`, `enabled=0`, `revision+1` on grants) | `retire_gitlab_connection_targets` runs the three UPDATEs per target, then the retire UPDATE. Shape matches `registry/delete.rs:165-171`. Tests (a) and (b) assert revision 1 to 2 (a) and the link `(0, revoked)`. | YES |
| 2 | Audit `gitlab.connection_deleted` with retired target ids and revoke counts | Handler adds `retiredTargetIds`, `hostGrantsRevoked`, `sessionGrantsRevoked`, `profileLinksRevoked`; asserted in tests (a) and (b). Written after commit, as before. | YES |
| 2 | One `fleet_events` row per retired target (kind per the architect) | `fleet.target.retired`, `target_id` set, no host or placement, actor = deleting user, camelCase detail, same transaction. Asserted in test (a). | YES |
| 3 | Retired targets invisible to new work, guarded at the choke points (decision 6) | Inline `retired_at IS NULL` predicates: `list_targets`, `set_host_grant` existence check, `ensure_target_identity_for_pool`, `link_pool_target`, `save_pool_fleet` upsert, and `target_id_for_key` (the key path). No helper, no new module. | YES |
| 3 | `ensure_target` / `gitlab_connection_target` / `sync_gitlab_targets` never revive a retired row | `target_id_for_key` reads only unretired rows; the insert is `ON CONFLICT DO NOTHING`; `gitlab_connection_target` returns `Ok(None)`; `sync_gitlab_targets` ignores `None`. `a_retired_gitlab_target_is_never_revived_offered_or_relinked` passes, and T's mutation spot-check proved it can fail. | YES |
| 3 | A grant on a retired target answers a clear 4xx | 404 `not_found`, asserted over HTTP with no state change. | YES |
| 3 | Sweep every `fleet_ci_targets` / `target_id` read in `crates/` (outside tests) with a verdict | F's table is in handoff-F.md. I re-ran the grep: every production `fleet_ci_targets` site (about 40 lines) is in the table. The one gap is a grant writer, not a `fleet_ci_targets` read; I found it and verified it safe (advisory 2). | YES, with one documented omission |
| 4 | No separate target route | Fleet router unchanged; no `DELETE /api/v1/fleet/targets/{id}`. | YES |
| 5 | OpenAPI DELETE description updated | `docs/openapi/gridops-v1.json`: description and the 409 text now say what blocks removal and that removal retires targets, revokes machine grants and platform links, and keeps history. The only copy of this spec text is that file. | YES |
| 5 | Delete confirm copy says the same in one sentence; copy only | `removeConfirm` gains "Its fleet targets are retired and their machine grants revoked; their fleet history is kept." One sentence, no new UI. | YES |
| 5 | `docs/gitlab-runners.md` "Removing a connection" section | Present: what blocks removal (pool, live runner, live fleet work), the order (delete or edit the pool, let runners leave service, remove the connection), and what removal does. The "Settings -> Connections" path and the "Remove" label match the UI. | YES |
| 5 | CHANGELOG entry | An Upgrading entry (migration 0048) and a Fixed entry under Unreleased, each citing #183. | YES |
| Tests (a) | Prod's shape: pool deleted, then connection deleted | HTTP test (a): 204, target retired with NULL connection, grant revoked with revision bump, target missing from `GET /api/v1/fleet/targets`, one `fleet.target.retired` event, grant POST 404. The grant is created through the real grant route with all flags off. | YES |
| Tests (b) | History case via the pool update | HTTP test (b): 204, intent, placement and event still name the target, link and grant revoked, other connection's target untouched. | YES |
| Tests | Never-used (list-created) target deletes | Extended `gitlab_connection_list_shows_seeded_connections_and_delete_answers_404_and_409`. | YES |
| Tests | Live placement / queued intent refused with 409, names what runs, no change | `a_gitlab_connection_is_not_removed_while_anything_still_runs_through_it`: queued intent, running placement, cleaned placement with a committed allocation; snapshot-compared before and after each 409. | YES |
| Tests | Pool and live-runner refusals unchanged | Steps 1 and 2 of the same test. | YES |
| Tests | Retired target not offered, refuses a grant write, never admitted or placed | List and grant over HTTP; `ensure_target_identity_for_pool` answers `NotFound` (submit, admission re-check and operation access all go through it); admission also needs unrevoked grants and enabled links, which the retire revokes. | YES |
| Tests | Migration rebuild keeps rows and FKs; trigger refuses identity updates and un-retiring | Three new `fleet_schema` tests plus the `FLEET_ADDED_COLUMNS` change. | YES |
| Out of scope | No deploy, no #182, no GitHub/Bitbucket retirement, no GC | None touched. The trigger's tolerance of a non-GitLab retire is a hedge, not a feature (advisory 1). | YES |

## Quality audit
| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | Same route, same 204/404/409 codes, same `ApiError::Conflict` style; the three refusals share one phrasing family. DELETE responses are checked against the OpenAPI schema in the HTTP tests. |
| Error handling | PASS | Every 409 leaves state unchanged (the transaction is dropped before any write; test compares a before/after snapshot). A core error maps to `ApiError::Internal`. No 500 path for a referenced target remains. |
| UI/UX match to spec | PASS | One added sentence, matching the brief's wording. No layout change. |
| Accessibility | N/A | No markup or control change. |
| Architecture gate | PASS | A: PASS twice (plan R2, anti-duplication). No quality finding conflicts with A's verdicts. |
| Code organization | PASS | One new public function and two public types in `fleet::targets` beside `delete_pool`; one private `ensure_live_target`; the handler gained no fleet SQL. Comments explain why. No dead code, TODOs or commented-out code in the diff. |
| Security | PASS | Handler still `require_system_admin` plus `SameOrigin`. No new route. Secrets are deleted as before; audit metadata holds ids and counts only. |
| Performance | PASS | The liveness query is one statement. A per-target loop of 5 statements runs over at most a handful of targets (one per connection). `BEGIN IMMEDIATE` takes the write lock up front, consistent with the webhook path and the lock-storm work in #179. |
| Visual regression | N/A | No visual surface changed (see precondition note). |
| Naming consistency | PASS | `retired_at` follows `fleet_hosts` retire naming; `fleet.target.retired` follows `fleet.host.retired` / `fleet.host.deleted`; audit keys are camelCase like the existing `baseUrl`, `scopePath`. |
| Test quality (`testing/test-quality.md` §7) | PASS | See below. |

### Test-quality audit (§7), per test and per suite
- **`removing_a_gitlab_connection_retires_its_fleet_target_and_revokes_its_grant`** (HTTP): names prod's case. It fails for the right reason if the 409 refusal returns, the retire is skipped, the grant is not revoked or the list still offers the target. Cheapest tier that covers handler, transaction, audit row and fleet routes together; behaviour only (rows and responses, no call counts).
- **`removing_a_gitlab_connection_keeps_its_fleet_history_and_revokes_its_platform_link`** (HTTP): names the history case. It is not a duplicate of (a): it is the only test that proves intents, placements and events keep their `target_id` and that an enabled profile link gets revoked.
- **`a_gitlab_connection_is_not_removed_while_anything_still_runs_through_it`** (HTTP): one scenario with six ordered steps. It is long, but each step names a distinct branch (pool, runner, queued intent, running placement, unreleased allocation, history-only admitted intent) and the before/after snapshot proves "no change". Not a smell: the steps are the refusal order the brief pins.
- **Extended `gitlab_connection_list_..._404_and_409`**: adds only the list-created-target case, which no other test owns.
- **`a_retired_gitlab_target_is_never_revived_offered_or_relinked`** (core integration): four assertions on one retired row across the key path, `list_targets`, `pool_targets` and `save_pool_fleet`. T's mutation spot-check (removing the `list_targets` and `target_id_for_key` filters) turned it red, so it can fail.
- **`a_retired_gitlab_target_is_not_valid_for_its_pool_and_is_never_linked`** (core unit, in-crate because the check is `pub(crate)`): the `NotFound` assertion proves the identity-check predicate, since without it the GitLab arm would answer `Forbidden`. The `link_pool_target` count of 0 would also hold without the `t.retired_at IS NULL` predicate (a retired GitLab row has a NULL connection, so the GitLab join already excludes it). A and T both call this a backstop, and the assertion states an invariant rather than an implementation detail, so I do not call it a smell. Noted in advisory 3.
- **Schema tests** (`the_0048_rebuild_keeps_every_target_and_reference`, `retiring_a_gitlab_target_frees_its_connection_and_is_final`, `the_0048_foreign_key_guard_fails_on_a_dangling_reference`, plus the `FLEET_ADDED_COLUMNS` entry): migration-tier work that no other tier can do. The existing 0046-to-latest upgrade test and the new 0047-to-latest test both check that `retired_at` was appended, but only the latter starts from a database holding a GitLab target with grant, link and event, so the signals differ. The FK-guard test shares a helper with 0047's test, so the refactor removed duplication instead of adding it.
- **Deleted test** `a_connection_a_fleet_target_references_is_refused_with_409_not_500`: it pinned the refusal this story removes. The surviving tests (extended DELETE test, HTTP test (a)) cover the new contract, as the §8 rule requires. T said so in handoff-T-green.
- **Repaired test** in `fleet/openapi.rs`: a minimal text update of an existing assertion, to the new contract. It adds no new wording test.
- **Proportionality (§5):** 10 new or changed tests for one migration, one core function, one handler and six guards, all in a feature story. Each one maps to a named branch or schema rule. No smell found: no assertion-free tests, no mocks, no snapshot dumps, no coverage padding, no wall-clock dependence.

## Scope check
F delivered exactly the brief's scope: migration 0048, the core function and guards, the handler change, OpenAPI text, one copy sentence, the docs section and the CHANGELOG. The 15 changed non-handoff files all fall inside the brief's list (the two test-only repairs by T are `gitlab_tests.rs` and `fleet/openapi.rs`). No route, UI surface, deploy step or retention logic was added. No over-delivery and no under-delivery.

## Verdict

PASS. All acceptance criteria are met, the implementation matches the binding architecture decisions, and the quality bar is met. Ready for O to commit.

## Advisory notes
Non-blocking. None needs F or T to act before the pull request opens.

1. **Trigger tolerance (carried from A-dup warn 2 and F's hedge).** The 0048 trigger allows a retire that keeps `gitlab_connection_id`, and a retire of a non-GitLab row. Production always nulls the connection and nothing retires GitHub or Bitbucket rows, so it is reachable only by hand-written SQL. One side effect worth knowing if it is ever done by hand: a retired GitHub row would make `ensure_target` answer `NotFound`, and `sync_github_targets` propagates that error, so `list_targets` would fail. If GitHub or Bitbucket retirement is ever added, add a kind clause to the trigger and a retired-aware path in `sync_github_targets` then.
2. **F's sweep table omits one writer.** `registry/mod.rs:1396` (the host-approve path, an `INSERT INTO host_target_grants` from the enrollment scope) is not itemised, and neither is the sibling `:1183` insert in `set_host_grant` (that one is guarded). I checked it: before it writes, the approve path calls `ensure_target_admin` (`registry/mod.rs:2075`), whose branches cover only GitHub installation targets and Bitbucket connections and end in `else { false }`, so any GitLab target, live or retired, is refused with `TargetScopeForbidden`. That confirms A's "enrollment has no GitLab arm". No code change is needed; for the record, the verdict for `:1396` is "not reachable for a GitLab target". If `ensure_target_admin` ever gains a GitLab arm, it needs a `retired_at IS NULL` predicate.
3. **Backstop guards have no isolated failing test.** The `save_pool_fleet` upsert predicate and the `link_pool_target` predicate cannot be the only thing that turns a test red, because the key path and the GitLab connection join already exclude retired rows before they are reached. A's warn 3 named both as backstops, so this is by design. The real gates (`list_targets`, key path, `set_host_grant`, `ensure_target_identity_for_pool`) each have a test that fails without them.
4. **F's handoff prose is stale on one point.** "Tests that look wrong" says the stale unit test was not edited; T deleted it afterwards (handoff-T-green item 1). Prose only.
5. **Pre-existing, unrelated:** `fleet_http::release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on macOS hosts and passes on Linux CI. This diff touches neither it nor `fleet_http`.
6. **Brief wording.** If this brief is reused, delete the superseded option text in build items 1 and 2 (A's warn 1).
