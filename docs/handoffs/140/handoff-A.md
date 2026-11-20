# Handoff-A: Phase 3 - GitLab job webhook ingestion and busy tracking (#140)  (up-front plan review, round 2)

**Date:** 2026-11-20
**Branch:** claude/gitlab-140-webhooks-a9fae5 (base b22c451; brief as of fc85722)
**Brief reviewed:** docs/handoffs/140-brief.md, including "Amendments (architecture review, round 1)", "Brief gate round 1 additions" and "Orchestrator decisions (brief gate round 2 warnings)"   **Reuse map:** N/A. No `survey.md` was provided. The baseline is the analogue the brief names, the GitHub webhook path (`crates/gridops-api/src/webhooks.rs`), its shared seams in `crates/gridops-core/src/db.rs`, and the cross-issue rules recorded on epic #136.   **Wireframe:** N/A (no UI surface)
**Verdict:** BLOCK

The round-1 review (BLOCK) is in git at `0a3d6d1`.

## Summary

BLOCK. The round-1 amendments resolve both earlier blocks: the `webhook_deliveries` row with its GitLab `retry` arm, and the busy write (columns, match and gate). They also settle the warnings on `state.rs`, `Cargo.toml`, reusing the bookkeeping, and the connection lookup. Three new blocks remain. Each sits where #140 touches another issue:

1. **The ordering guard contradicts epic #136.** #136 records one status rule for both `gitlab_jobs` writers. It was posted at 3:30:59 PM PST, 15 seconds after this brief's last commit, and the brief's `gitlab_jobs` guard breaks it.
2. **`status_changed_at` is in the wrong unit.** The brief stores it in epoch seconds. Every timestamp column in the codebase uses milliseconds, and so does #148, which reads and writes this column.
3. **The route line alone fails the test suite.** Registering the route breaks the route-matrix drift test unless `docs/fleet-api.md` gets a new row, and the brief doesn't allow that file. The brief also keeps the OpenAPI "Not served yet" marker, while #138's brief removes its own when its handlers land.

All three are short brief amendments.

### Round-1 findings: status

| Round-1 # | Status |
|---|---|
| 1 (deliveries, retry) | Resolved: "GitLab deliveries in `webhook_deliveries`" and the W-1 duplicate decision, which matches `receive` (`webhooks.rs:52-73`). |
| 2 (busy columns) | Resolved: columns, instance-scoped match and the `rows_affected` gate are all pinned. |
| 3 (`state.rs`) | Resolved. The text matches #138's brief (`138-brief.md:123-127`), including the placeholder message. See new warning 6 on D17. |
| 4 (`Cargo.toml`) | Resolved. It matches #138's line. |
| 5 (bookkeeping reuse) | Resolved: `pub(super)` helpers extracted from `receive` are authorised. |
| 6 (ordering guard) | Applied as I suggested in round 1. #136's later rule supersedes that suggestion, so it becomes new finding 1. Round 1 also suggested that #148's poll be insert-only. #148 didn't adopt that: its plan applies guarded updates. |
| 7 (connection lookup) | Resolved for `receive`. The replay path has a remaining gap, which is new warning 4. |

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | block | "Ordering guard (finding 6)": `ON CONFLICT DO UPDATE ... WHERE excluded.status_changed_at > gitlab_jobs.status_changed_at`, with a lifecycle-rank tie rule; "#148's poll is expected to insert only jobs it has not seen"; the Done-when tests "Tie rule" and "Stale concluded event (W-2)" | cross-cutting (an invariant shared by two writers of one table); pattern consistency | **#136 records one rule for both writers.** Its comment "#148 rules (from its plan review), shared by #139/#140" was posted at 3:30:59 PM PST (23:30:59 UTC), 15 s after `fc85722`. It says: "a terminal row never changes; a writer never moves a row backwards along created/waiting_for_resource/preparing → pending → running → terminal; the poll ... inserts with `ON CONFLICT(connection_id, job_id) DO NOTHING`, then applies a guarded update."<br>**The brief's guard breaks both halves.** It orders by time first, so any newer-timestamped event can rewrite a terminal row or move a row backwards, and rank only decides ties. Its rank table also splits `created` < `waiting_for_resource` < `preparing`, which #136 treats as one tier.<br>**#148 is not insert-only.** Issue #148 says "Conclude recorded jobs ... using `GitLabApi::job`". Its plan (`148-brief.md` A-W3, `:103-108`) refreshes statuses and writes `status_changed_at` = its own observation time, from the GridOps clock. The brief's note for #148 says the opposite.<br>**Two failures under the brief's guard:**<br>- **The webhook's `running` loses to the poll.** The poll records `pending` at observation time T. GitLab's `running` event carries a `build_status_changed_at` just before T, because the job started while the poll's request was in flight or because of clock skew. The guard ignores it, so the row stays `pending` (counted as #148 demand) and the runner is never marked busy.<br>- **A late `running` reopens a finished job.** When `build_status_changed_at` is missing or unparsable, the time falls back to the receive time. A `running` that GitLab delivers after `success` then carries the later time. It rewrites the terminal row back into `gitlab_jobs_active_created_idx` and marks the runner busy, and no later event will clear it. | **Adopt #136's rule as the guard.** Apply the upsert's update only when the stored status is not terminal, and either:<br>- the incoming tier is higher than the stored tier, or<br>- the tiers are equal and `excluded.status_changed_at > gitlab_jobs.status_changed_at`.<br>**Use #136's tiers:**<br>- first: `created`, `waiting_for_resource`, `preparing`;<br>- then `pending`;<br>- then `running`;<br>- then terminal: `success`, `failed`, `canceled`, `skipped`.<br>**Place `manual`, `scheduled` and `unknown` explicitly** (they belong in the first tier, because they come before `pending`). Record that placement on #136 so #148 uses the same table.<br>**Keep the `rows_affected` busy gate.**<br>**Apply the same predicate in the pre-fetch check** (round-2 W-3).<br>**Replace the tie-rule and W-2 tests** (see Notes for O).<br>**Delete the "insert only" note for #148.** |
| 2 | block | "`status_changed_at` is stored as Unix epoch seconds ... parsed from `build_status_changed_at`"; the Done-when timestamp-fallback test, "(epoch seconds)" | cross-cutting (timestamp units in a column two writers share); pattern consistency | **Every integer `*_at` value in this codebase is epoch milliseconds:**<br>- `now_millis()` (`gridops-core/src/db.rs:187`);<br>- `date_value` returns `timestamp_millis()` (`webhooks.rs:925-929`);<br>- `webhook_deliveries.received_at` and its 24-hour failure window (`resources.rs:455-461`);<br>- retention (`gridops-reconciler/src/main.rs:3143`);<br>- `JOB_CHECK_AGE_MS = 5 * 60 * 1_000` (`gridops-reconciler/src/main.rs:2036`).<br>**The same row is in milliseconds too.** That `gitlab_jobs` row's own `created_at` and `updated_at` will hold milliseconds.<br>**#148 reads and writes this column in milliseconds.** Its conclusion step selects rows whose `status_changed_at` is older than `GITLAB_JOB_CHECK_AGE_MS` (`148-brief.md:69`, modelled on `JOB_CHECK_AGE_MS`). It writes its observation time into the column (A-W3).<br>**Mixed units break both writers:**<br>- **#148's age gate:** a webhook-written value (about 1.7e9) looks decades old. Every webhook-written active row then enters #148's conclusion step on every pass, and because the step takes the oldest first, these rows fill its 10-row limit.<br>- **#140's guard:** any value the poll writes (about 1.7e12) is larger than every later webhook value, so no same-tier webhook update applies after a poll write. | **Store epoch milliseconds:**<br>- parse `build_status_changed_at` to `timestamp_millis()`. A GitLab-format parser beside `date_value` is a justified new helper;<br>- the fallback is `now_millis()` at receipt, the same instant `received_at` gets;<br>- amend the fallback test to match.<br>**Record "`gitlab_jobs.status_changed_at` is epoch ms" on #136.** |
| 3 | block | "`main.rs`: the route line for `POST /api/v1/webhooks/gitlab` only"; "Do not change ... the OpenAPI spec" | file structure; shared-file ownership (drift tests that pin the route list) | **The route line alone fails the route-matrix test.** `access::tests::the_documented_matrix_lists_exactly_the_registered_routes` (`crates/gridops-api/src/access.rs:276`) parses every `.route(` call in `main.rs`. It fails when a registered route has no row in `docs/fleet-api.md` "Who may call what". `every_route_in_the_documented_matrix_behaves_as_documented` (`tests/fleet_http/token_access.rs:1222`) then calls each documented row with a bearer token. The brief doesn't list `docs/fleet-api.md`.<br>**The spec would say the route isn't served.** The path's OpenAPI description starts "Not served yet: implemented by #140." `the_gitlab_operations_are_specified_before_they_are_served` (`fleet/openapi.rs:286`, `:291-300`) asserts that prefix.<br>**The sibling issue treats both as part of serving a route.** #138's brief removes "the 'Not served yet: implemented by #138.' sentences ... Leave `/api/v1/webhooks/gitlab` (#140's) alone" (`138-brief.md:53`). It also owns "`docs/fleet-api.md` (rows)" and "`openapi.rs` (its test)" (`:113`, `:117`).<br>**Without the same allowance, #140 has no clean option.** F must either leave the suite red or make an unplanned edit to files #138 is editing. | **Add three edits to #140's shared-file list:**<br>- **`docs/fleet-api.md`:** one row, `\| POST \| /api/v1/webhooks/gitlab \| — \| — \| — \| ignored \|` (path in backticks), after the GitHub webhook rows at `:128-129`.<br>- **`docs/openapi/gridops-v1.json`:** remove only the sentence "Not served yet: implemented by #140. " from this path's description. The shapes don't change, so this isn't a contract change.<br>- **`crates/gridops-api/src/fleet/openapi.rs`:** the `#140` entry of the not-served test stops asserting the prefix.<br>**Coordination with #138.** #138 edits the same table and the same test. Rows are add-only, and whichever issue lands second reconciles the test. |
| 4 | warn | "Retry ... re-resolving the connection from the payload; the token is not re-checked" | cross-cutting (identity on replay) | **Retry has no rule for several candidates.** `receive` picks the connection by token, and the brief's tie-break applies only among candidates whose token matched. Retry has no token. The brief says nothing about a payload with several candidates: the same `project_id` on two `base_url`s, or a project connection plus its parent group's connection.<br>**Two ways it goes wrong:**<br>- retry could call `GitLabApi::job` with another instance's credentials;<br>- retry could write the row under a different connection from the one the job's later deliveries match. One GitLab job then has two `gitlab_jobs` rows, and demand counts twice until #148 concludes the stale one. | **Pin one of these, with a test:**<br>- **(a) Carry the matched connection in the delivery id.** An accepted row's id is `gitlab:<connection_id>:<delivery key>`, and a rejected row stays `gitlab:<delivery key>`. The token is checked before the insert, and retry reads the connection from the id. This needs no schema change, and a rejected row can't block a later valid delivery's id.<br>- **(b) Narrow the candidates on retry.** Retry keeps only candidates whose `base_url` is a prefix of the payload's `project.web_url`, and answers 400 when more than one remains. |
| 5 | warn | Where the guard's tier rule lives | abstraction level (the same rule written more than once) | **The GitHub analogue shares such rules from core.** Write rules that the webhook and the poll both apply live in `gridops-core/src/db.rs`: `record_job_report`, `finish_earlier_attempts` and `associate_runner_with_job`. They are called from `webhooks.rs:611-626` and from `gridops-reconciler/src/main.rs:1934-2209`.<br>**Here the rule would be written three times.** Both GitLab briefs forbid `gridops-core` changes, so finding 1's rule would appear in #140's SQL, in #140's Rust pre-fetch check, and in #148's SQL. | **Within #140, write the tier table once:** one Rust function or const, used by the pre-fetch check and as the source of the SQL `CASE`.<br>**Record a follow-up on #136** to move the rule into a core helper that both writers call. That would be first-to-land adds it, like #148's A-W2 credentials loader.<br>Not blocking, because both briefs forbid the core edit for now. |
| 6 | warn | `state.rs` `UnconfiguredGitLab`, "whose every method returns `GitLabError::Upstream(...)`" | dependency wiring across issues | **D17 adds a trait method.** On #136 (3:22:20 PM PST), D17 has #139 add `GitLabApi::list_runners`. #139's brief keeps it out of `gridops-api`.<br>**One merge order breaks the build.** If #138 or #140 lands first, #139's trait change breaks `gridops-api`'s build until `UnconfiguredGitLab` gains the method. | **Record on #136:** whichever of #139 and (#138 or #140) lands second adds the one `list_runners` stub, returning the same `Upstream` error, to `UnconfiguredGitLab`.<br>For the other order, the brief's "every method" already covers it. |
| 7 | warn | Busy tracking (no runner event history mentioned) | cross-cutting (event log) | **The GitHub path logs runner history.** It writes a `runner_events` row whenever a job event touches a runner ("Workflow job <action>", `webhooks.rs:626-650`), and that is the runner's history.<br>**The brief is silent,** so F would decide. | **Decide in writing.** Either #140 writes one `runner_events` row when it sets busy and one when it clears it (for example "GitLab job running" with `jobId`, `projectId` and `pipelineId` metadata), or it writes none. |

## Notes for O

Amend `docs/handoffs/140-brief.md` as below, then start a **fresh** run. Don't resume this one.

### Block 1: ordering guard

**Replace the first bullet of "Ordering guard (finding 6)" with:**

> The out-of-order guard is one conditional upsert on `(connection_id, job_id)` that follows epic #136's one status rule for `gitlab_jobs` writers. The update applies only when `gitlab_jobs.status` is not terminal (`success`, `failed`, `canceled`, `skipped`) **and** either `tier(excluded.status) > tier(gitlab_jobs.status)`, or the tiers are equal and `excluded.status_changed_at > gitlab_jobs.status_changed_at`.
>
> The tiers are:
> - 0: `created`, `waiting_for_resource`, `preparing`, `manual`, `scheduled`, `unknown`;
> - 1: `pending`;
> - 2: `running`;
> - 3: the terminal statuses.
>
> The tier table is written once in Rust, and the SQL `CASE` is built from it. `status_changed_at` is epoch milliseconds, parsed from `build_status_changed_at`; a missing or unparsable value falls back to `now_millis()` at receipt.

**Confirm or change the tier-0 placement** of `manual`, `scheduled` and `unknown`, then post the tier table on #136.

**Replace the third bullet** (the #148 note) with: "#148's poll applies the same rule (epic #136)."

**Change the W-3 decision's pre-fetch check** to use the same predicate.

**In "Done when", replace the "Tie rule" and "Stale concluded event (W-2)" items with:**

- **Terminal is final.** After `success`, a `running` (or any) event with a newer `status_changed_at` changes neither the row nor any runner.
- **Never backwards.** After `running`, a `pending` event with a newer `status_changed_at` is ignored, and makes no `GitLabApi::job` call.
- **A later tier wins over time.** A row stored as `pending` with `status_changed_at` T, as #148's poll writes it, followed by a `running` event with an older `status_changed_at`: the event applies and marks the runner busy.
- **Within a tier, time decides.** `created`, then `preparing`: with an older `status_changed_at` it is ignored, and with a newer one it applies.
- **A stale terminal event changes nothing.** After `success` cleared runner A, a `failed` event delivered later with an older `status_changed_at` touches neither the row nor A.

### Block 2: units

- **Change "Unix epoch seconds" to "epoch milliseconds"** throughout, including the timestamp-fallback test ("= the delivery's receive time, `now_millis()`").
- **Post "`gitlab_jobs.status_changed_at` is epoch ms" on #136.**

### Block 3: route side effects

Add to "Shared files (findings 3 to 5)":

- **`docs/fleet-api.md`:** one "Who may call what" row for `POST /api/v1/webhooks/gitlab` (`— | — | — | ignored`), after the GitHub webhook rows.
- **`docs/openapi/gridops-v1.json`:** remove only the sentence "Not served yet: implemented by #140." from this path's description. Nothing else in the spec changes.
- **`crates/gridops-api/src/fleet/openapi.rs`:** the #140 entry of `the_gitlab_operations_are_specified_before_they_are_served` no longer asserts the prefix. #138 edits the same function and table, and whichever issue lands second reconciles.

### Warnings: cheap to settle in the same amendment

4. **Retry's connection.** Pick option (a), the connection id carried in the delivery id, or option (b), narrowing by `project.web_url` with a 400 when ambiguous, and add a test.
5. **The tier table.** It is written once in Rust (already in the block 1 text). Record a follow-up on #136 to move it into a `gridops-core` helper.
6. **D17.** Record on #136 who adds `list_runners` to `UnconfiguredGitLab` when #139 lands second.
7. **`runner_events`.** State whether #140 writes a `runner_events` row on busy set and clear.

### Outside #140, for #136 and #148

**Overlapping connections give one GitLab job two rows.** A project connection plus its parent group's connection, both with webhooks or both polled, produce two `gitlab_jobs` rows for one job, because identity is `(connection_id, job_id)`. That counts demand twice. The fix lies in the contract or in #148's demand scan (for example, deduplicate on `(base_url, job_id)`), not in #140.

## Patterns referenced

- `crates/gridops-api/src/webhooks.rs`:
  - `receive`, `:25-111`, and `retry`, `:113-185`;
  - `process_workflow_job` and its `runner_events` write, `:562-650`;
  - `date_value`, `:925-929`.
- `crates/gridops-core/src/db.rs`:
  - `now_millis`, `:187`, and `associate_runner_with_job`, `:195-245`;
  - `record_job_report` and `finish_earlier_attempts`, the write rules the webhook and the poll share. The reconciler calls them at `gridops-reconciler/src/main.rs:1934-2209`; its `JOB_CHECK_AGE_MS` is at `:2036`.
- The route drift tests:
  - `crates/gridops-api/src/access.rs:184-291`;
  - `crates/gridops-api/tests/fleet_http/token_access.rs:1143-1280`;
  - `crates/gridops-api/src/fleet/openapi.rs:247-300`;
  - `docs/fleet-api.md:120-130`.
- Epic #136's comments, read with `gh api`:
  - the D1-D16 table;
  - D17;
  - "#148 rules", including its timestamp.
- `migrations/0047_gitlab.sql`, `crates/gridops-core/src/gitlab/mod.rs`, and the in-flight sibling briefs: `gitlab-148-scaling-71c514/docs/handoffs/148-brief.md` (A-W3, `:69`), `gitlab-138-api-44d3a8/docs/handoffs/138-brief.md` (`:53`, `:113-127`) and `gitlab-139-lifecycle-53c8ce/docs/handoffs/139-brief.md` (D17, `:118`).
