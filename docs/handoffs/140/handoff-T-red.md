# Handoff-T-red: Phase 4 - GitLab job webhook ingestion and busy tracking (#140)

**Date:** 2026-11-20
**Branch:** claude/gitlab-140-webhooks-a9fae5 (head `b89e6fb`)
**Brief / wireframe reviewed:** `docs/handoffs/140-brief.md` (as of `370c679`, through "Amendments (architecture review, round 4)"), `docs/handoffs/140/handoff-A.md` (round 5) and `docs/handoffs/140/decisions.md`. There is no wireframe: #140 has no UI surface.

## A precondition

Confirmed: A returned **PASS** on the plan in round 5 (`b89e6fb`), with 0 block findings and 3 warnings. The brief was not amended after round 5, so this suite applies them as follows:

- **W1 (D18's writer already exists in PR #160).** PR #160 is still OPEN at `754a72b` (checked with `gh pr view 160`). T takes #148's D18 suite, `crates/gridops-core/src/gitlab/jobs_tests.rs`, **byte for byte** from `754a72b` (blob `0d7ccf4b29f53cdea3f664607d0d3bd0997665aa` on both sides). It covers every rule in the brief's D18 "Tests" list. T writes no second D18 suite. Its mount in `gitlab/mod.rs` is the same block #148 has, so once F adds `pub mod jobs;` right after `pub mod http;`, `mod.rs` lines 17-30 match `754a72b` exactly.
- **W2 (#138's `credentials` helper).** No test constrains it. All connections in the tests use host-only base URLs, so `Url`'s trailing slash is the same whichever way F builds the credentials.
- **W3 (stale test name).** T made the change A describes: `the_gitlab_operations_are_specified_and_only_140s_await_a_handler` becomes `the_gitlab_operations_are_specified_and_all_are_served`. Its `else` branch is gone, so no GitLab operation may say "Not served yet".

## Tests authored

All 30 webhook tests are in-crate HTTP tests in `crates/gridops-api/src/webhooks/gitlab_tests.rs`, mounted from `webhooks.rs` with `#[cfg(test)] mod gitlab_tests;`.

**How they run:**

- Each test sends requests through `crate::public_routes()` merged with `crate::session_routes()`, using `tower::ServiceExt::oneshot`, with `FakeGitLab` injected through `AppState::with_gitlab`.
- Each test seeds its own migrated temporary SQLite database. Connections are seeded with their access token and webhook secret sealed in `runtime_secrets`, as #138 stores them.
- Payloads follow GitLab's documented Job Hook (`doc/user/project/integrations/webhook_events.md`, "Job events", fetched 2026-11-20): `object_kind`, `build_id`, `build_name`, `build_status`, `build_status_changed_at` (RFC 3339 with milliseconds), `retries_count`, `pipeline_id`, `runner` (`null` or `{id, …}`), `project_id` and `project.path_with_namespace`.

**Why this tier:** it is the cheapest one that reaches the handler with an injected GitLab. The same tier also checks the route's registration in `public_routes()` (the `main.rs` line), `retry`'s new GitLab arm, and the 202, 400 and 401 shapes against `docs/openapi/gridops-v1.json`, via `tests/support/openapi_schema.rs::check_response`.

**Why not a unit test of the processing function:** the brief does not pin its name or signature, so going through HTTP keeps the tests on behaviour rather than on F's internals.

| # | Test | What it pins (brief item) |
|---|---|---|
| 1 | `a_bad_or_missing_token_is_a_401_recorded_as_rejected_that_cannot_be_retried` | **Done when: bad token.** A wrong token is a 401 whose body passes the spec check. The delivery is recorded as `gitlab:unmatched:<key>` with `signature_valid` false and status `rejected` (W-4). A missing token is also a 401. Nothing is written and no GitLab call is made. `retry` refuses the row with 400 "Only verified webhook payloads can be retried". |
| 2 | `an_unknown_project_is_a_401_recorded_as_rejected` | **Done when: unknown project.** The token is a real connection's secret, so the 401 comes from the project lookup. |
| 3 | `a_pending_job_is_looked_up_for_its_tags_and_recorded` | **Done when: pending job looked up for tags.** **Lookup:** one `GitLabApi::job` call, on the matched connection's instance. **Row:** every `gitlab_jobs` column except the surrogate `id` and the row timestamps (tags from the lookup, `status_changed_at` in ms, `pool_id` NULL per W-2). **Delivery row:** `id` = `gitlab:<connection_id>:<key>`, `event` = `Job Hook`, `action` = `pending`, `hook_id`, `installation_id` and `repository_id` NULL, `signature_valid` true, status `processed`. **Response:** a 202 with **no body**, because the spec's 202 declares no content. |
| 4 | `the_delivery_key_is_the_idempotency_key_then_webhook_id_then_event_uuid_then_the_body_hash` | Orchestrator NIT-1: the header order `Idempotency-Key` > `webhook-id` > `X-Gitlab-Event-UUID` > lowercase hex SHA-256 of the raw body. |
| 5 | `a_duplicate_delivery_is_acknowledged_without_reprocessing` | **Done when: duplicate delivery.** A second delivery of a `processed` id is a spec-valid 202 with no second lookup and no write. A second delivery of a `failed` id is also a duplicate and is not reprocessed (W-1). |
| 6 | `a_body_that_is_not_a_json_object_is_a_400_and_writes_nothing` | Brief-gate W-2: a non-JSON body and `[]` are each a 400 with a spec-valid `ApiErrorBody`. No job or runner is written. |
| 7 | `among_several_candidates_the_one_whose_secret_matches_the_token_is_used` | **Candidates (B-1):** a project connection, its group's connection, and a project connection on another instance with the same `project_id`. **Selection:** each token selects its own connection, and the `pending` lookup goes to that connection's instance. **No match:** 401 `gitlab:unmatched:<key>`. |
| 8 | `when_several_candidates_match_a_project_connection_wins_then_the_oldest` | B-1 preference: project-scoped first, then the oldest `created_at`. The group connection is the oldest of all and loses, and the older of the two project connections wins. One setup tells apart "oldest regardless of kind", "project then newest" and "project then oldest". |
| 9 | `a_group_connection_matches_projects_under_its_path_at_a_slash_boundary` | B-1: `acme` matches `acme/app` and `acme/sub/app`, and not `acme-other/app` (401). |
| 10 | `a_retried_job_is_a_new_row` | **Done when: retried job.** A new `build_id` with `retries_count` 1 is a new row, and the original is unchanged. |
| 11 | `a_pending_event_after_running_is_ignored_without_a_lookup` | Round-2 "Never backwards" and W-3: a newer `pending` after `running` leaves the row unchanged, makes no lookup and is `processed`. |
| 12 | `a_later_tier_applies_even_with_an_older_time` | Round-2 "A later tier wins over time": a polled `pending` at T, then a webhook `running` at T-1s applies and marks the runner busy. |
| 13 | `within_a_tier_only_a_newer_event_applies` | Round-2 "Within a tier, time decides": `created`, then `preparing` older (ignored), then `preparing` newer (applies). |
| 14 | `the_status_time_is_the_payloads_in_epoch_ms_or_else_the_receive_time` | Round-2 "Timestamp unit": an exact ms value (`…40.123Z` gives T+123). When the field is absent, `null` or unparsable, the time falls within `now_millis()` taken before and after the request. |
| 15 | `a_build_event_with_an_unrecognised_status_writes_no_row` | Round-2 "`unknown`": no row, and the delivery is `processed`. |
| 16 | `a_non_build_event_is_recorded_and_processed_with_no_effect` | Brief-gate "Non-`build` events": a real-shaped **Pipeline Hook** (project only under `project`, no top-level `project_id`) is recorded and `processed`, with no job, runner or event written. |
| 17 | `a_first_seen_non_pending_event_stores_no_tags_and_looks_nothing_up` | Round-3: a first-seen `created` stores `[]` and makes no lookup. |
| 18 | `the_fetched_tags_survive_later_events` | Round-3: the tags `["a","b"]` from `pending` survive `running` and `success`. |
| 19 | `running_marks_the_runner_busy_and_a_conclusion_clears_it_on_that_runner_only` | **Done when: busy set and cleared on the right runner** (round-1 "Busy tracking", W-7, NIT-1). **Set:** the runner becomes `busy`/`busy`, while runner 55 on another instance and runner 56 on this one are unchanged. `current_job_id`, `last_job_id` and `github_runner_id` stay NULL. **Clear:** the runner returns to `online`/not busy. **Events:** two `runner_events` rows, `info` "GitLab job running" and "GitLab job success", with message `rspec · pipeline 7` and metadata `{jobId, projectId, pipelineId, connectionId}`. |
| 20 | `running_without_a_runner_records_the_job_and_touches_no_runner` | D18 W-1: the row is `running` with a NULL runner, no runner changes and no event is written. |
| 21 | `a_first_seen_running_event_marks_the_runner_busy_and_a_later_pending_is_ignored` | Round-4 "First-seen `running`": insert, busy, one event (message `Job · pipeline 7` when `build_name` is absent). A later `pending` is ignored and makes no lookup. |
| 22 | `running_after_a_failed_pending_lookup_marks_the_runner_busy` | Round-4: `fail_next(Job)` makes the `pending` delivery `failed` with no row. The following `running` marks the runner busy. |
| 23 | `an_older_running_event_fills_the_runner_the_poll_did_not_know` | **Round-3 "runner filled after the poll wins":** only `gitlab_runner_id` changes (status and time stay as the poll wrote them), the runner becomes busy and one event is written. **Terminal row:** the same event against a `success` row changes nothing. |
| 24 | `a_newer_running_event_onto_a_runnerless_running_row_marks_the_runner_busy` | Round-4 "fill under clock skew": the event applies, the runner is busy, one event. |
| 25 | `a_second_running_event_for_the_same_runner_sets_nothing_again` | Round-4 "No double set": the runner row is byte-identical afterwards and there is still one event. |
| 26 | `after_a_conclusion_no_later_event_changes_the_job_or_a_runner` | Round-2 "Terminal is final": a newer `running` or `pending` changes neither the row nor any runner, and the `pending` makes no lookup. |
| 27 | `a_stale_conclusion_changes_neither_the_job_nor_the_runner` | Round-2 "A stale terminal event changes nothing": `failed` with a time before the applied `success`. |
| 28 | `a_failed_delivery_is_retried_through_the_gitlab_arm` | Brief-gate "Retry through the GitLab arm": a failed `pending` gets a spec-valid 202. `retry` answers 200, writes the row with the tags and ends `processed`. Today `retry` sends such a row to GitHub's `process`, which marks it `processed` without writing a row (confirmed by T's sanity run, below), so this test separates the two arms. |
| 29 | `a_retry_uses_the_recorded_connection_even_when_the_key_contains_colons` | W-4 option (a) and NIT-1: two project connections on different instances share `project_id`. The delivery matched B under key `gl:retry:1`. Retry calls `job` on B again (`[B, B]`) and writes one row under B. The candidate preference would pick A, so re-resolving instead of using the recorded connection fails this test. |
| 30 | `a_failure_inside_the_write_rolls_it_all_back_and_a_retry_applies_it` | **Round-4 "Partial failure is recoverable":** a temporary trigger aborts `runner_events` inserts. **`running`:** the delivery is `failed`, with no row and no busy. Once the trigger is dropped, the retry gives a row, busy, one event and `processed`. **Terminal event:** under the trigger the row stays `running` and the runner stays busy. The retry concludes it, clears busy and adds the second event. |

**Changed unit test:**

| # | File | Test | What it pins |
|---|---|---|---|
| 31 | `crates/gridops-api/src/fleet/openapi.rs` | `the_gitlab_operations_are_specified_and_all_are_served` (renamed from `…_and_only_140s_await_a_handler`, A W3) | No GitLab operation's description contains "Not served yet". This is brief round-2 Block 3: remove that one sentence. |

**D18 core suite**, taken byte for byte from #148 (`754a72b`). These are 10 unit/integration tests against a temp-file database, the tier D18 requires:

| # | File | Tests |
|---|---|---|
| 32-41 | `crates/gridops-core/src/gitlab/jobs_tests.rs` | `status_tier_ranks_every_status_into_its_d18_tier`, `a_status_write_applies_only_upward_or_at_the_same_tier_when_newer_and_never_to_a_terminal_row` (every tier pair × older/equal/newer), `a_new_row_gets_a_uuid_and_the_given_values_or_the_defaults`, `unknown_writes_nothing`, `a_terminal_row_never_changes_in_any_column`, `the_runner_id_is_filled_once_and_never_cleared_or_overwritten`, `tags_are_replaced_only_by_non_empty_tags_and_filled_on_a_rejected_write`, `retries_count_and_pipeline_move_only_with_an_applied_status_write`, `set_gitlab_runner_busy_matches_the_runner_id_on_any_connection_of_the_same_instance`, `the_outcome_reports_the_runner_stored_before_the_call` |

**Test wiring T added (no production logic):**

- `#[cfg(test)] mod gitlab_tests;` in `crates/gridops-api/src/webhooks.rs`;
- the `#[cfg(test)] #[path = "jobs_tests.rs"] mod jobs_tests;` block, with #148's comment, in `crates/gridops-core/src/gitlab/mod.rs`.

The `gridops-core` `test-support` dev-dependency the brief asks for is already on master (#138).

## RED confirmation

**1. `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api --bin gridops-api`: 90 passed, 31 failed.** The 31 failures are exactly the 30 new webhook tests plus the renamed OpenAPI test. All 90 existing bin tests pass, including `access::…the_documented_matrix_lists_exactly_the_registered_routes`.

Every webhook test fails at its first delivery because `POST /api/v1/webhooks/gitlab` isn't served. The path falls through to the GET-only `/api/v1/webhooks/{delivery_id}`, which answers 405:

```
a_pending_job_is_looked_up_for_its_tags_and_recorded        left: 405  right: 202
a_bad_or_missing_token_is_a_401_…                           left: 405  right: 401
an_unknown_project_is_a_401_recorded_as_rejected            left: 405  right: 401
a_body_that_is_not_a_json_object_is_a_400_…                 left: 405  right: 400
a_duplicate_delivery_… / a_non_build_event_… / the_delivery_key_… /
a_failed_delivery_is_retried_… / a_retry_uses_the_recorded_connection_…   left: 405  right: 202
the other 21 (via Harness::deliver)                         Error: delivering {…}: 405 Method Not Allowed
fleet::openapi::…the_gitlab_operations_are_specified_and_all_are_served
  post /api/v1/webhooks/gitlab is served by #140: "Not served yet: implemented by #140. The delivery is matched to a connection by its `X-Gitlab-Token`."
test result: FAILED. 90 passed; 31 failed
```

**2. Harness sanity, to show the 405 isn't hiding a setup defect.** T temporarily added one throwaway test, ran it, and deleted it. It is not in the diff. It exercised every harness piece the 30 tests depend on, and it passed (`1 passed`):

- **Seeding:** three connections (project, group, other instance), the runners and a polled `gitlab_jobs` row.
- **Secrets:** they open with the harness vault.
- **Admin session:** it reaches `retry`, which answers 404 (not 401 or 403) for an unknown id with colons.
- **Verified `gitlab:` row retried today:** it goes through GitHub's `process`, ends `processed`, and writes no `gitlab_jobs` row. That is why test 28 separates the two arms.
- **Unverified row:** refused with the expected message.
- **The abort trigger:** it blocks `runner_events` inserts and drops cleanly.
- **The event comparison helper:** it matches a hand-written row.
- **`FakeGitLab`:** it answers the seeded job and logs the call.
- **The RFC 3339 helper:** it round-trips T+123 ms.

**3. `cargo test -p gridops-core --lib --no-run`: one compile error, the D18 module F adds.**

```
error[E0432]: unresolved import `crate::gitlab::jobs`
   --> crates/gridops-core/src/gitlab/jobs_tests.rs:34:20
error: could not compile `gridops-core` (lib test) due to 1 previous error
```

**4. The D18 suite can be satisfied on this branch.** T temporarily placed #148's `jobs.rs` (`754a72b`) on the branch and added `pub mod jobs;`. `cargo test -p gridops-core --lib -- gitlab::jobs_tests` reported `10 passed; 0 failed`. Both files were then reverted, and `git status` shows neither.

**5. Formatting and lint.** `cargo fmt --all -- --check` is clean. `cargo clippy -p gridops-api --bin gridops-api --tests` reports no warning in any file T wrote or edited. One `#[allow(clippy::duplicate_mod)]` was added, with a comment, because `openapi_schema.rs` is included a second time and #138's include sits in a private module. `jobs_tests.rs` is #148's file and passed #148's lint.

## Ready for F

**RED is valid.** F may implement against these tests.

The tests need **no new production symbol** to compile. Everything is reached through the router, `retry`, the database and the pinned D18 API.

**What F must do to turn them green:**

- **The route.** Put `POST /api/v1/webhooks/gitlab` in `public_routes()` in `main.rs`. The tests build the router from `crate::public_routes()`.
- **D18.** Take `crates/gridops-core/src/gitlab/jobs.rs` byte for byte from `754a72b` (A W1), or from master if PR #160 merges first. Add `pub mod jobs;` directly after `pub mod http;` in `gitlab/mod.rs`. T's mount block already sits there, so the lines then match #148's.
- **The 202 has no body.** The spec's 202 declares no content, and `check_response` rejects a body on it. Do **not** copy `receive`'s `{"accepted": true}` JSON. This applies to duplicates and to failed processing too.
- **Status codes.** A missing `X-Gitlab-Token` is a **401**, not `required_header`'s 400. The spec's only 400 is "payload is not a JSON object".
- **A Pipeline Hook (or any non-`build` event)** has no top-level `project_id`. Its project is under `project.id` and `project.path_with_namespace`. The candidate lookup has to read the project from there when `project_id` is absent, otherwise test 16 gets a 401.
- **`runner_events.metadata` ids are JSON numbers** (`jobId`, `projectId`, `pipelineId`), as in the GitHub path. `connectionId` is the connection's id string.
- **Delivery status writes.** A failed delivery is `failed`, with the 202 above. `retry` answers 200 for a GitLab row it reprocesses.
- **The fleet docs row.** `docs/fleet-api.md` needs the "Who may call what" row for the new route (brief round-2 Block 3). Otherwise the existing `access::…the_documented_matrix_lists_exactly_the_registered_routes` fails once the route is registered.
- **The OpenAPI sentence.** Removing "Not served yet: implemented by #140." from the spec turns test 31 green.

**Choices the tests pin that the brief leaves open** (listed under "Assumed" in `decisions.md`; O may overrule any of them):

- the empty 202 body;
- 401 for a missing token;
- the project for a non-`build` event comes from `project.id`;
- numeric metadata ids;
- the fallback time is checked as "between `now_millis()` before and after the request", not pinned to `received_at`.

**Not covered by a test:**

- The `Delivery-id` header-order **code comment** (brief-gate W-1). S checks it in the diff.
- The constant-time compare (`subtle`). It is observable only as correct matching. S checks it in the diff.
- No log line carrying the token. The suite doesn't capture tracing, so S checks it in the diff.
