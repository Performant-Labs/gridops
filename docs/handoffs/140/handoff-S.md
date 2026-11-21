# Handoff-S: Phase 8 - GitLab job webhook ingestion and busy tracking (#140)

**Date:** 2026-11-20
**Branch:** claude/gitlab-140-webhooks-a9fae5 (head `4c9794c`)
**Issue:** #140
**Brief reviewed:** `docs/handoffs/140-brief.md` (through "Amendments (architecture review, round 4)")
**Handoff-T reviewed:** `docs/handoffs/140/handoff-T-green.md` (and `handoff-T-red.md`)
**Handoff-A reviewed:** `docs/handoffs/140/handoff-A.md` (round 5, PASS) and `docs/handoffs/140/handoff-A-dup.md` (PASS)
**Handoff-F reviewed:** `docs/handoffs/140/handoff-F.md`
**Operator-facing report:** N/A (non-visual audit; see below)

## A precondition

Confirmed. `handoff-A.md` is `PASS` (round 5, 0 block findings). `handoff-A-dup.md` is `PASS` (2 warn findings, both "no change needed").

## T precondition

Confirmed. `handoff-T-green.md` reports zero blocking issues. Its one unrelated failure
(`fleet_http::release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`) is host-OS
dependent. #140 touches no machine, host-agent or preflight code.

## Visual-diff-tool precondition

N/A. #140 is a Rust API handler plus a shared writer in `gridops-core`. The diff since the A gate (`b89e6fb..HEAD`)
touches no file under `src/`, no CSS and no template, and the brief says "#140 does not touch the UI". There is no
rendered surface to diff, so the Browser-tool and Visual-diff-tool preconditions do not apply and no HTML report is
required.

## Preview / spec sanity check

No defect found in the source of truth that blocks F's work, so no `ADVISORY-HOLD`.

I checked the brief's two outside-world premises against GitLab's current docs
(`doc/user/project/integrations/webhook_events.md`, "Job events", fetched 2026-11-20):

- `build_status_changed_at` exists (introduced in GitLab 19.7) and is documented as the field to order job events by.
  The docs also say it can be `null` ("If the job is no longer in one of these statuses ... Events with `null` in these
  fields cannot be ordered this way"). The brief's fallback to receipt time covers that case. See advisory 2.
- The header facts in the brief (NV-2) match the docs. The brief's header order and its required code comment are in
  place.

The OpenAPI entry (`202` no content, `400` "not a JSON object", `401`, `security: []`) is consistent with the brief.

## Verification I ran myself (all with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`)

| Check | Result |
|---|---|
| `cargo test -p gridops-api --bin gridops-api -- webhooks::gitlab_tests fleet::openapi access::` | 42 passed, 0 failed |
| `cargo test -p gridops-core --lib -- gitlab::` | 34 passed, 0 failed (includes the 10 D18 `jobs_tests`) |
| `cargo fmt --all -- --check` | clean |
| `git rev-parse` blobs of `gitlab/jobs.rs`, `jobs_tests.rs`, `gitlab/mod.rs` vs `origin/master` (`105b589`) | identical (`c51247dd`, `0d7ccf4b`, `96be8825`) |
| `git merge-tree --write-tree HEAD origin/master` | clean, no conflicts |
| Built and tested the merge result (tree `25e1ae5d`, exported to a scratch directory) | `webhooks::gitlab_tests`, `fleet::openapi` and `access::` all pass (42 passed). Master moved twice since the branch merged it (#160, #161). |
| Mutation checks (below) | 8 of 9 mutants killed |

The tree was clean before and after every mutation (restored with `git checkout`, `git status` empty).

**Mutation checks** on `webhooks/gitlab.rs`, beyond T-green's single spot-check:

| Mutation | Killed by |
|---|---|
| Group match without the `/` boundary | `a_group_connection_matches_projects_under_its_path_at_a_slash_boundary` |
| Busy set without `previous_gitlab_runner_id.is_none()` | `a_second_running_event_for_the_same_runner_sets_nothing_again` |
| Delivery-key header order reversed | `the_delivery_key_is_the_idempotency_key_then_webhook_id_...` |
| Pre-fetch check removed (`&& true`) | `a_pending_event_after_running_is_ignored_without_a_lookup`, `a_first_seen_running_event_...`, `after_a_conclusion_no_later_event_...` |
| Clear busy on any write, not only `StatusApplied` | `a_stale_conclusion_changes_neither_the_job_nor_the_runner` |
| `retries_count` not passed | `a_retried_job_is_a_new_row` |
| Status time always the receive time | 8 tests |
| **Runner id passed on every event, not only `running`** | **survived** (advisory 1) |

## Spec compliance

| Requirement (source) | Compliant | Where it is checked |
|---|---|---|
| `POST /api/v1/webhooks/gitlab` route, one line in `main.rs` (issue; round 4) | YES | `public_routes()`; `access::...documented_matrix_lists_exactly_the_registered_routes` |
| Connection from the payload's project or group; project id unique only per instance, so a candidate set (round 1, finding 7) | YES | `matching_connection`; tests 7, 8 |
| Group prefix at a `/` boundary (round 1 additions) | YES | `substr(?2,1,length(scope_path)+1)=scope_path||'/'`; test 9, mutation killed |
| Constant-time token compare against every candidate; no match is 401 | YES | `secret.as_bytes().ct_eq(token.as_bytes())` over every candidate, no early exit, first match in preference order kept (project, then `created_at`, then `id`). `subtle` is the existing dependency. Reviewed in the diff; timing is not observable in a test. |
| Same 401 for unknown project and bad token (no enumeration oracle) | YES | test 1, 2 |
| Delivery recorded in `webhook_deliveries`: `id`, `event`, `action`, NULL ids, `signature_valid` (round 1, "GitLab deliveries") | YES | test 3 (every column), test 1 |
| Delivery id `gitlab:<connection_id>:<key>`, `gitlab:unmatched:<key>` for a rejected one (W-4 a) | YES | tests 1, 3, 29 |
| Key order `Idempotency-Key`, `webhook-id`, `X-Gitlab-Event-UUID`, SHA-256 of the body, plus the required code comment (NIT-1, W-1) | YES | `delivery_key`; test 4; the doc comment states all three points the brief lists |
| Duplicate: any existing id is acknowledged (202), not reprocessed (W-1 decision) | YES | test 5 (processed and failed duplicates) |
| Rejected delivery recorded, then 401; refused by `retry` (NV-4) | YES | test 1 |
| `pending`: `GitLabApi::job` for `tag_list`, before the upsert, outside any transaction; failure marks the delivery failed with nothing written | YES | `process_job_hook`; tests 3, 22, 28 |
| Pre-fetch check skips the lookup when the guard would reject (round 2) | YES | `pending_applies`; tests 11, 21, 26; mutation killed |
| `running` with `runner.id`: record the runner and mark that runner busy | YES | tests 19, 21-25 |
| `running` without `runner.id`: row recorded, no busy, no event (W-1) | YES | test 20 |
| `success`/`failed`/`canceled`/`skipped`: conclude and clear busy on the runner recorded on the job | YES | test 19, mutation on the clear arm killed (T-green and mine) |
| Busy write touches only `busy`, `status`, `last_heartbeat_at`, `updated_at`; never `current_job_id`, `last_job_id`, `github_runner_id` | YES | test 19 asserts the NULLs; the writer is D18's, unchanged |
| Runner match by `gitlab_runner_id`, live GitLab runners, same `base_url` | YES | D18 `set_gitlab_runner_busy`; test 19 (other instance and other id untouched) |
| One status rule: tiers, terminal is final, time within a tier (round 2, Block 1) | YES | tests 11-13, 26, 27; D18 core tests |
| `status_changed_at` in epoch ms; fallback `now_millis()` for a missing, null or unparsable value (Block 2) | YES | test 14 |
| `unknown` status is never stored (Block 1) | YES | test 15 |
| Retried job (new `build_id`) is a new row (issue) | YES | test 10 |
| `pool_id` always NULL (W-2 decision) | YES | test 3 asserts NULL |
| `tag_list` written only from the `pending` fetch; `[]` on a non-pending insert; tags survive later events (round 3) | YES | tests 17, 18 |
| Runner fill after the poll wins; same event against a terminal row changes nothing (round 3) | YES | test 23 |
| Busy set whatever the write variant; no double set; first-seen `running`; `running` after a failed fetch; clock skew (round 4) | YES | tests 21, 22, 24, 25 |
| `runner_events` row on busy set and clear, with the pinned `level`, `event`, `message`, `metadata` (W-7, NIT-1) | YES | test 19, 21 (`Job` fallback) |
| One `BEGIN IMMEDIATE` transaction per delivery; rollback leaves the delivery `failed`; retry reapplies in full | YES | `process_job_hook`; test 30 (both a `running` and a conclusion) |
| Non-`build` events recorded and `processed`, nothing written; project read from `project.id` when there is no `project_id` | YES | test 16 |
| Malformed body: 400 `ApiErrorBody`, nothing written | YES | test 6 (non-JSON and `[]`) |
| `retry`: one GitLab arm keyed on the `gitlab:` prefix; uses the recorded connection; splits once at the first `:` after the prefix; never reaches GitHub's `process` | YES | `recorded_connection`, `reprocess`; tests 28, 29 |
| Retry fallback time is the stored receive time | YES (design choice, settles A round-5's note) | `reprocess` reads `received_at` |
| 202 carries no body, for new, duplicate and failed deliveries (OpenAPI) | YES | `check_response` in tests 3, 5, 28 |
| `docs/fleet-api.md` row; OpenAPI sentence removed; `fleet/openapi.rs` test updated | YES | `docs/fleet-api.md:130`; `docs/openapi/gridops-v1.json` (one sentence); test 31 |
| D18 module: `jobs.rs` and its tests, byte-identical to #148's reviewed file | YES | blobs identical to master |
| `state.rs`, `Cargo.toml`, migration, `gridops-core` contract types unchanged | YES | not in the diff since `b89e6fb` |

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | Route, status codes and bodies follow the OpenAPI entry and `check_response` validates them in the tests. The handler reuses `receive`'s bookkeeping instead of copying it. |
| Error handling | PASS | 400 / 401 / duplicate 202 / failed-delivery 202 with `failed` status; the failure text is the lookup error ("GitLab job N lookup failed: ..."), which `GitLabError`'s `Display` keeps free of the token. A failed write rolls back and is retryable. |
| UI/UX match to spec | N/A | No UI surface. |
| Accessibility | N/A | No UI surface. |
| Architecture gate | PASS | A round 5 PASS; A-dup PASS. No quality finding conflicts with either. |
| Code organization | PASS | `webhooks/gitlab.rs` is 439 lines in two layers: the HTTP edge (`receive_gitlab`, `matching_connection`, `delivery_key`) and a narrow `process_job_hook(&SqlitePool, &dyn GitLabApi, ...)`. Doc comments say why, not what. No TODO, no `unwrap`/`expect`/`dbg!` in the file. |
| Security | PASS | Parameterised SQL throughout; constant-time compare of every candidate; the token is never stored (the payload row holds the body only) and never logged (the only `tracing` call logs the delivery id, event and error); one 401 for "no candidate" and "wrong token"; the delivery id of a rejected delivery cannot collide with an accepted one (`unmatched` vs a connection id). `retry` of a GitLab row needs a system admin (NULL `installation_id`). See advisories 3 and 4. |
| Performance | PASS | One candidate query by project id / path, one secret read per candidate, indexed lookups (`gitlab_jobs` unique key, `runners_gitlab_runner_id_idx` via D18). The tag fetch is skipped for a stale `pending`. |
| Visual regression | N/A | No UI; the project runs no Playwright check for this change. |
| Naming consistency | PASS | `receive_gitlab`, `webhooks/gitlab.rs`, `gitlab_credentials` follow the neighbouring modules. |
| Test quality (`testing/test-quality.md` §7) | PASS | See below. |

### Test-quality audit

- **Per test.** Each of the 30 webhook tests names one behaviour in its title and goes through the router
  (`public_routes()` plus the real `retry`), so none binds a private function. Each fails for the right reason: the
  mutation runs above show a named test going red for each targeted behaviour. The tier is the cheapest that reaches
  the handler with an injected `FakeGitLab`.
- **Per suite.** 30 webhook tests plus 10 D18 core tests for one handler, one retry arm and one shared writer. This is a
  feature story, so a tiered suite is proportionate. Tests 21, 23, 24 and 25 look alike but pin four different cells of
  the round-4 busy rule (first-seen, fill after the poll wins, clock skew, no double set); the busy mutation killed
  exactly one of them, which confirms they are not duplicates. The D18 suite is #148's file, byte for byte, and is not
  re-written.
- **Smells.** None found: no assertion-free or tautological test, no unreachable outcome, no mock-shaped assertion
  (`FakeGitLab`'s call log is observable behaviour), no snapshot padding. The 1,661-line file is mostly harness (seeding,
  secrets, row readers). No "delete or merge" finding.
- **One gap**, advisory 1 below.

## Scope check

F delivered the brief's scope and nothing more.

- **Within scope:** the handler and its module; the shared delivery bookkeeping extracted from `receive` (allowed by
  round-1 finding 5); the one `retry` arm; one route line; the D18 module and its tests (the D18 exception); the
  `docs/fleet-api.md` row and the one OpenAPI sentence (round-2 Block 3, B-1 exception).
- **One edit outside the brief's list:** `credentials` in `resources/gitlab.rs` became `pub(crate)` and `resources.rs`
  gained one `pub(crate) use` line. This is visibility only, it is A's round-5 W2 first option, F recorded it under
  "Deviations", and A-dup accepted it. It avoids a third copy of the trailing-slash rule.
- **Not done, correctly:** nothing added to `state.rs`, `Cargo.toml`, `main.rs` client wiring or the UI.
- **Under-delivery:** none. Every "Done when" item in the brief maps to a passing test (T-green's table, re-checked
  above).

## Verdict

PASS. Every acceptance criterion and every amendment is met and tested, the quality audit is clean, and the
implementation builds and passes on top of current master. Ready for O to commit.

## Advisory notes

None of these blocks the commit.

1. **One unpinned design decision (test gap, optional).** `process_job_hook` passes `runner.id` to the writer only for
   `running` events (F's "Design decisions" bullet 3; D18's "How #140 uses it" says the same). If a later change
   passed it on every event, no test would fail: I made that change and all 30 tests stayed green. The case that tells
   the two apart is a poll-stored `running` row with a NULL runner, then a `success` that names runner R. Today the
   job concludes and R is left alone; the mutant would record R and clear R. A short test next to test 27 would pin it.
   I did not block on it because the brief states the rule only as "clear the runner recorded on that job" and the
   mutant is arguably within that wording, so the gap is in F's safer reading, not in a stated requirement.
2. **`build_status_changed_at` can be `null` in real payloads** (GitLab docs, above). The brief's fallback is receipt
   time, which F implements. A late event with a `null` time therefore looks newest, and the status rule only protects
   it by tier and by the terminal freeze. Within tier 0 (`created`, `preparing`, ...) a late `null`-time event could
   overwrite a newer one. This follows the brief, so it is not a defect in #140; worth one line on #136.
3. **Unauthenticated storage.** A request with no valid token still stores its body (up to the 25 MB limit) as a
   `rejected` `gitlab:unmatched:<key>` row. The brief requires that ("recorded the way `receive` records an invalid
   signature") and the GitHub endpoint does the same, with the existing retention index on `received_at`. Noting it for
   the operator, no change asked.
4. **One undecryptable candidate secret fails the whole delivery with a 500**, even when another candidate would
   have matched. It matches GitHub's `verify_signature` behaviour and surfaces a real configuration fault, so I left it.
5. **A missing `X-Gitlab-Event` header is a 400** through the shared `required_header`, which no test covers and the
   OpenAPI `400` text ("The payload is not a JSON object") does not mention. It matches the GitHub handler and the spec
   lists the header as required.
6. **Local-only failure to ignore:** `fleet_http::release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`
   fails on a macOS ARM64 dev box regardless of #140 (T-green's note). It is expected to pass on Linux CI.
