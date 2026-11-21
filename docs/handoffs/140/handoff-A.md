# Handoff-A: Phase 3 - GitLab job webhook ingestion and busy tracking (#140)  (up-front plan review, round 5)

**Date:** 2026-11-20
**Branch:** claude/gitlab-140-webhooks-a9fae5 (master `0086aa9` merged in; brief as of `370c679`)
**Brief reviewed:** docs/handoffs/140-brief.md, including "Amendments (architecture review, round 4)"   **Reuse map:** N/A. No `survey.md` was provided. The baseline is the analogue the brief names, the GitHub webhook path (`crates/gridops-api/src/webhooks.rs`), plus #138's GitLab connection code now on master (`crates/gridops-api/src/resources/gitlab.rs`), the `BEGIN IMMEDIATE` writers in `crates/gridops-core/src/fleet/`, and #148's open PR #160 (head `754a72b`), which carries a full copy of the D18 writer.   **Wireframe:** N/A (no UI surface)
**Verdict:** PASS

Earlier rounds are in git: round 1 (BLOCK) at `0a3d6d1`, round 2 (BLOCK) at `8f36d51`, round 3 (BLOCK) at `aaa925a`, round 4 (BLOCK) at `ebab59d`.

## Summary

PASS. The round-4 amendments resolve both round-4 blocks and all three round-4 warnings, and the D18 amendment is posted on #136 (6:27 PM PST, 01:27:44 UTC). No block findings remain. Three warnings remain. All of them come from code that landed or was opened after D18 was written:

1. #148's PR #160 already contains a full, gated `gitlab/jobs.rs`, with tests written to run against either copy. The brief still has F write #140's own.
2. #138 added a GitLab credentials helper that the brief's shared-file list doesn't let #140 reuse.
3. One test name in the brief is out of date.

### Round-4 findings: status

| Round-4 # | Status |
|---|---|
| 1 (busy set left out `Inserted`; outcome had no prior runner) | Resolved. `JobWriteOutcome.previous_gitlab_runner_id` was added. One busy-set rule applies to every write variant: `status == "running"`, `gitlab_runner_id` is `Some(R)` and `previous_gitlab_runner_id` is `None`. #148's copy fills the field from the stored row, and its `Ignored` outcome reports previous equal to current, so a no-op can't set busy. Four Done-when tests cover it (first-seen `running`, after a failed fetch, clock skew, no double set). |
| 2 (no transaction boundary) | Resolved. One `BEGIN IMMEDIATE` per delivery wraps `record_gitlab_job`, `set_gitlab_runner_busy` and the `runner_events` insert. The fetch stays outside it, and so do the `webhook_deliveries` status writes, as in `receive`. This matches `fleet/targets.rs:542`, `fleet/admission.rs:169` and `fleet/placement.rs:217`. There is a partial-failure Done-when test, and the same rule is posted on #136 for #148. |
| 3 (client wiring owner) | Resolved. PR #161 (`claude/gitlab-138-followup`) adds `.with_gitlab(Arc::new(GitLabClient::new()?))`, and its description says #140 won't add it. #140's `main.rs` change is one route line. |
| 4 (`u64` ids; `Option<String>` pool) | Resolved. The ids are `ExternalId`. `set_gitlab_runner_busy` returns `Option<(String, String)>`, which is correct because `runners.pool_id` is `text NOT NULL` (`0047_gitlab.sql:124`). |
| 5 (one statement; in-memory tests) | Resolved. The writer reads the row, then runs one write statement. Its tests use `connect_database_path` on a temporary directory. |

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | D18: "#140 adds the new file `crates/gridops-core/src/gitlab/jobs.rs` and the one line `pub mod jobs;`"; "#148 may stub against these exact signatures. Whichever merges second removes its local copy."; D18 "Tests: the module has its own unit tests" | pattern consistency (reuse-first / one implementation of one rule) | **The object #140 is told to write already exists, written and reviewed.**<br>- PR #160 (#148, OPEN, MERGEABLE, head `754a72b`) adds `gitlab/jobs.rs`, `gitlab/jobs_tests.rs` and the `mod.rs` lines. They follow the amended D18 exactly: `ExternalId` ids, `previous_gitlab_runner_id`, `(String, String)`, a read then one guarded write, and a re-read when another writer changed the row in between.<br>- That PR passed #148's A-dup and S gates.<br>- `jobs_tests.rs` is mounted from `mod.rs` "so they keep running against whichever copy of `jobs.rs` (#140's or #148's) lands on master" (#148 `gitlab/mod.rs:23-27`). It has 10 tests, one per D18 rule.<br>**If F writes `jobs.rs` from scratch, and T writes a second D18 suite:**<br>- the two implementations of D18's "one rule" diverge in their unspecified edges (for example the `Ignored` outcome when no row exists, or the concurrent-write retry);<br>- both branches add the same files, so the merge conflicts;<br>- whichever branch merges second throws away its copy and re-runs its webhook tests against code it was never developed against.<br>D18 exists so the two writers *can't* disagree.<br>**Why only a warning.** D18 settles in writing what happens on master ("whichever merges second takes master's file"), so this is a planned duplicate, not drift. | **Before T starts:**<br>- If PR #160 has merged, merge master and use master's `jobs.rs`, `jobs_tests.rs` and `mod.rs` lines unchanged.<br>- If it hasn't, copy those three byte for byte from `754a72b`. A byte-identical add/add merges cleanly in either order.<br>**Then:**<br>- T writes only the webhook tests in `gridops-api`. The D18 core tests are `jobs_tests.rs`.<br>- If #140 needs any change to the writer, it raises it as a D18 change on #136 first.<br>- Widen the D18 exception from "the one line `pub mod jobs;`" to the `mod.rs` lines #148 added (`pub mod jobs;` and the `#[cfg(test)] #[path = "jobs_tests.rs"] mod jobs_tests;` block) and the new `jobs_tests.rs`. |
| 2 | warn | Clarifications D2: "The handler builds the credentials from the matched connection's `base_url` and the access token stored under its `access_token_key`."; the shared-file list (which doesn't name `resources.rs` or `resources/gitlab.rs`) | pattern consistency (helper duplication); naming (one GitLab credentials convention in the API crate) | **#138 put the API crate's way of building GitLab credentials in a private helper**: `resources/gitlab.rs:714-724`, `fn credentials(base_url, access_token) -> ApiResult<GitLabCredentials>`.<br>- It appends `/` to the stored base URL, which is normalised without a trailing slash (`0047_gitlab.sql:23-24`), and maps a bad URL to `ApiError::Internal`.<br>- It is used by `verify_gitlab_connection` (`:295-318`), and `remove_gitlab_runner` (`:583-621`) repeats the same load, `state.runtime_secret` and `credentials` sequence inline.<br>- The `gitlab` submodule is private (`resources.rs:89`).<br>**#140 needs the same thing in two places**: the HTTP path and the GitLab `retry` arm.<br>**The brief allows edits only to the files it lists**, so F can either write a third credentials builder or make an unlisted edit to #138's file. Either one is the kind of Phase-7 duplication or drive-by edit this review exists to prevent.<br>**The trailing slash matters for tests.** `FakeGitLab` keys everything by exact `base_url` (`Url`) equality (brief NV-3), so a copy that drops the `/` breaks fake seeding for instances under a path prefix. `GitLabClient::api_url` itself accepts either form (`http.rs:402-415`). | **Add to the shared-file list:**<br>- in `resources/gitlab.rs`, make `credentials` `pub(crate)`;<br>- in `resources.rs`, add one `pub(crate) use self::gitlab::credentials as gitlab_credentials;`.<br>This follows `resources.rs:42` (`pub(crate) use self::route_security::SameOrigin;`), and `agent.rs:34-37` already imports from `resources`.<br>**`webhooks/gitlab.rs`** then loads `base_url` / `access_token_key`, calls `state.runtime_secret`, and builds the credentials with `gitlab_credentials`.<br>**Or**, if O would rather not touch #138's file, say in the brief that #140 keeps a local copy with the same trailing-slash rule and a comment pointing to `resources/gitlab.rs:714`. |
| 3 | warn | Round-2 Block 3: "the #140 entry of `the_gitlab_operations_are_specified_before_they_are_served` stops asserting that prefix. #138 edits the same function and table, and whichever lands second reconciles." | naming (stale reference) | #138 landed first and renamed the test to `the_gitlab_operations_are_specified_and_only_140s_await_a_handler` (`fleet/openapi.rs:289-307`). Its `else` branch now asserts that #140's description starts with "Not served yet: implemented by #140". So #140 is the second to land, and reconciling means removing that branch: every GitLab operation is then served, and the name and doc comment should say so. The brief's intent is clear, but the name it cites won't match anything when F searches for it. | Correct the name in the brief. **"#140 changes the `else` branch so that no GitLab operation's description contains 'Not served yet', and renames the test and its doc comment to match."** |

### Accepted as consistent

- **`webhooks.rs` + `webhooks/gitlab.rs`**, following `resources.rs` + `resources/gitlab.rs`.
  - As a child module, `webhooks/gitlab.rs` can call the parent's private payload helpers (`string`, `nested_object`, `i64_value`, `date_value`, `required_header`) and its delivery bookkeeping without any visibility change. It should reuse them rather than add new parsers.
  - `date_value` (`webhooks.rs:925`) parses RFC 3339. That is the format GitLab documents for `build_status_changed_at` (`"2021-02-23T02:41:37.886Z"`).
- **The transaction shape.**
  - The `runner_events` insert is inline SQL inside the transaction, like every other `runner_events` writer (`webhooks.rs:636`, `resources.rs:4503`, the reconciler at `main.rs:3748`). There is no helper to reuse.
  - The webhook secrets and the token are read through `AppState::runtime_secret` (`state.rs:153`) before the transaction opens. No pool write happens while `BEGIN IMMEDIATE` holds the write lock, so it can't deadlock itself.
- **D18's `connection_id: &str`.** It matches the SQL-level writers in `db.rs` and `resources/gitlab.rs`. `GitlabConnectionId` is used only by the `CiTarget` domain.
- **Retry.** `retry`'s unverified-row refusal (`webhooks.rs:134-138`) and its `require_system_admin` for a NULL `installation_id` cover the GitLab rows.
- **The fallback timestamp.** "`now_millis()` at processing when the field is missing" is how the GitHub path behaves (`webhooks.rs:516`, `:552`, `:725`: `date_value(..).unwrap_or(now)`).

## Notes for O

PASS, so nothing has to change before T. These are the cheap amendments for the three warnings, plus one note outside my dimension.

- **W1 (highest value).** Add one paragraph under the D18 heading:
  > "D18's writer is already in PR #160 at `754a72b` (`gitlab/jobs.rs`, `gitlab/jobs_tests.rs`, `gitlab/mod.rs`). #140 takes those files byte for byte, from master if #160 has merged and from `754a72b` if not. It writes no D18 implementation or D18 unit tests of its own."
  
  Widen the D18 exception to cover those `mod.rs` lines and `jobs_tests.rs`. The D18 "Tests" subsection is then met by `jobs_tests.rs`.
- **W2.** Add `resources/gitlab.rs` (`credentials` made `pub(crate)`) and `resources.rs` (one `pub(crate) use`) to the shared-file list, or explicitly allow a local copy that keeps the trailing slash.
- **W3.** Replace the stale test name, as in finding 3.
- **Outside architecture, for the brief gate's NV-4.**
  - GitLab's Job-events docs (`doc/user/project/integrations/webhook_events.md` on GitLab's master, fetched 2026-11-20) say `build_status_changed_at` was "introduced in GitLab 19.7". The payload fields the brief relies on are all in the documented example: `build_id`, `build_name`, `build_status`, `pipeline_id`, `retries_count`, `runner.id`, `project_id` and `project.path_with_namespace`.
  - If 19.7 isn't released yet, self-managed instances never send the field, so the `now_millis()` fallback is the normal path there, not an edge case. The tier-first rule still keeps cross-tier ordering correct.
  - That makes the brief gate's NIT-1 (pin the fallback to `webhook_deliveries.received_at`, so that a retry doesn't move the time) somewhat more relevant. It is still not an architecture block, because GitHub's path behaves the same way.

## Patterns referenced

- **`crates/gridops-api/src/webhooks.rs`:**
  - `receive` (`:25-111`) and `retry` (`:113-185`);
  - `process_workflow_job` (`:562-654`);
  - the payload helpers (`:889-938`), including `date_value` (`:925`).
- **`crates/gridops-api/src/resources/gitlab.rs`:** `credentials` (`:714-724`), `verify_gitlab_connection` (`:295-318`) and `remove_gitlab_runner` (`:583-621`). Also `resources.rs:42`, `:89-97` (the precedent for re-exporting from a private submodule).
- **`crates/gridops-core/src/fleet/`:** `targets.rs:542`, `admission.rs:169` and `placement.rs:217` (`BEGIN IMMEDIATE`), and `domain.rs:15-58` (`ExternalId`).
- **PR #160 at `754a72b`:** `crates/gridops-core/src/gitlab/jobs.rs` (in full), `jobs_tests.rs` (test list) and `gitlab/mod.rs:21-27`.
- **`migrations/0047_gitlab.sql`:** `:23-82` (`gitlab_connections`, `gitlab_jobs`) and `:122-145` (`runners.pool_id NOT NULL`).
- **`crates/gridops-api/src/fleet/openapi.rs`:** `:247-307`.
