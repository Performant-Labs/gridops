# Handoff-A: Phase 3 - GitLab job webhook ingestion and busy tracking (#140)  (up-front plan review, round 4)

**Date:** 2026-11-20
**Branch:** claude/gitlab-140-webhooks-a9fae5 (base b22c451; brief as of ba0a441)
**Brief reviewed:** docs/handoffs/140-brief.md, including "Amendments (architecture review, round 3)", "Brief gate additions" and "Amendments (contract decision D18)"   **Reuse map:** N/A. No `survey.md` was provided. The baseline is the analogue the brief names: the GitHub webhook path (`crates/gridops-api/src/webhooks.rs`), its shared write helpers in `crates/gridops-core/src/db.rs`, the GitHub sync in `crates/gridops-reconciler/src/main.rs`, the read-then-guarded-write transactions in `crates/gridops-core/src/fleet/`, and the cross-issue rules on epic #136 (D18 posted 4:15:49 PM PST, 00:15:49 UTC).   **Wireframe:** N/A (no UI surface)
**Verdict:** BLOCK

Earlier rounds are in git: round 1 (BLOCK) at `0a3d6d1`, round 2 (BLOCK) at `8f36d51`, round 3 (BLOCK) at `aaa925a`.

## Summary

BLOCK, with two block findings, both in how #140 uses the new D18 writer. The round-3 block and all three round-3 warnings are resolved. D18's placement and its column rules are consistent with the codebase.

1. **The busy rule leaves out `Inserted`, and the outcome can't say whether this call set the runner.** A `running` event that arrives before any row exists (out of order, or after a failed `pending` fetch) never marks its runner busy. The rule depends on "newly set by this call", but `JobWriteOutcome` doesn't carry the runner id from before the write.
2. **No transaction boundary.** The busy effects only run when the guarded write changed the row. So a failure after `record_gitlab_job` commits can't be fixed by a retry, because the replayed write returns `Ignored`. The GitHub path recovers on retry only because its upsert is unconditional.

Both fixes are a sentence each, plus one additive field in D18 that has to be posted on #136.

### Round-3 findings: status

| Round-3 # | Status |
|---|---|
| 1 (column rules) | Resolved. Round-3 Block 1 and D18's rules: `gitlab_runner_id` is `COALESCE`-filled on any non-terminal row, even when the status write is rejected. `tag_list` is replaced only by non-empty tags and is filled on a rejected write when the stored value is `[]`. Terminal rows are frozen. The three Done-when tests cover them. #148's brief at `572f828` (`:155-191`) adopts the same rules. |
| 2 (where the rule lives) | Resolved by D18: one writer in `gridops-core`, owned by #140, with #148 calling it. Posted on #136 and adopted by #148. |
| 3 (busy match) | Resolved. `set_gitlab_runner_busy` matches on `base_url` (D18 rules, #136). |
| 4 (pins) | Resolved. `runner_events.message` and the retry id split are both pinned. NIT-1 also pins the split when the key contains a colon. |

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | block | D18 "How #140 uses it": "`running` with `runner.id`: ... When the outcome is `StatusApplied` or `FilledOnly`, `status = running`, and the outcome's `gitlab_runner_id` was newly set by this call, mark that runner busy". D18 API: `JobWriteOutcome { write, previous_status, status, gitlab_runner_id }`. | contract shape (the cross-crate D18 API); pattern consistency with the round-1 busy gate | **(a) The rule leaves out `JobWrite::Inserted`.**<br>- A `running` event whose job has no stored row is `Inserted`, and under this rule its runner is never marked busy.<br>- The brief creates that state itself in two ways. GitLab delivers out of order, which is the issue's premise (Scope: "GitLab can deliver them out of order"). And a failed `pending` fetch "leaves no `gitlab_jobs` row" (Brief gate W-4).<br>- The later `pending` event is then rejected by the guard and makes no fetch. #148's poll inserts only jobs GitLab lists as pending, so nothing ever sets busy for that job.<br>- With `busy` = 0 on a runner that is running a job, `maybe_scale_down` stops returning early for the pool (`gridops-reconciler/src/main.rs:1408`).<br>- The newer text also contradicts the round-1 gate ("Busy effects run only when the guarded `gitlab_jobs` upsert changed the row"), and an insert does change the row.<br>**(b) "Newly set by this call" can't be derived from `JobWriteOutcome`.**<br>- The struct carries the runner id after the write, not before it.<br>- The ambiguous case is `StatusApplied` from `running` to `running`. #148's poll stores `running` with a NULL runner at its local observation time T. The webhook's `running` event then carries a later GitLab `build_status_changed_at`, which happens with clock skew, as #136's 23:43:53 UTC comment warns ("GitLab's clock and the poll's local clock can skew").<br>- The caller can't tell that fill apart from a second `running` event for a runner that was already recorded. F has to choose between missing the busy set and writing it twice, with a duplicate `runner_events` row.<br>- #148 codes against these "exact signatures", so the shape has to be fixed now, not in #140's diff. | **D18 amendment (additive; post it on #136 so #148's local copy matches):**<br>- `JobWriteOutcome` gains `previous_gitlab_runner_id` (the stored value before the call; None when inserted).<br>**One busy-set rule, whatever the `write` variant:**<br>- Mark the runner busy, and write the `runner_events` row, when `outcome.status == "running"`, `outcome.gitlab_runner_id` is `Some`, and `outcome.previous_gitlab_runner_id` is `None`.<br>- This covers `Inserted`, `StatusApplied` and `FilledOnly`, and it is idempotent, because `COALESCE` never overwrites the runner.<br>**The terminal clear rule** stays as written: `StatusApplied` into a terminal status with a runner id.<br>**Tests:** see Notes for O. |
| 2 | block | D18 "Callers own transactions, `retry_when_busy`, `runner_events` rows ..."; round-1 "Busy effects run only when the guarded `gitlab_jobs` upsert changed the row"; the GitLab `retry` arm | cross-cutting (transactions, error recovery) | **#140 never says which writes share a transaction, and the busy gate makes a partial commit unrecoverable.**<br>**The GitHub analogue doesn't need a transaction.** `process_workflow_job` (`webhooks.rs:583-649`) runs its writes one by one on the pool. A failure part way through is repaired by `retry`, because the job upsert is unconditional and `associate_runner_with_job` (`db.rs:195-245`) runs again.<br>**#140's busy effects run only when `record_gitlab_job` changed the row.** Suppose the row commits and then `set_gitlab_runner_busy` or the `runner_events` insert fails. That can happen when the 5 s busy timeout runs out (`db.rs:32`), which is why the reconciler has `retry_when_busy` (`main.rs:482-500`). The delivery is marked `failed`. An admin's retry through the GitLab arm replays the same event:<br>- **a `running` event:** same tier, same time, runner already stored, so `Ignored`, and busy is never set (the same effect as finding 1);<br>- **a terminal event:** the terminal row is frozen, so `Ignored`, and busy is never cleared. #148's conclusion step scans active rows only (`148-brief.md:120`). So the stale `busy` = 1 makes `maybe_scale_down` return early for that pool (`main.rs:1408`) for as long as the runner row lives.<br>**Codebase precedent for this shape (a guarded write followed by dependent writes):**<br>- `gridops-core/src/fleet` runs a read and its guarded write in a short `BEGIN IMMEDIATE` transaction (`fleet/registry/mod.rs:6-7`, `fleet/targets.rs:542`, `fleet/admission.rs:169`, `fleet/service.rs:135`);<br>- the GitHub sync wraps its job writes in one transaction inside `retry_when_busy` (`gridops-reconciler/src/main.rs:1963-1971`);<br>- 13 `gridops-api` handlers use `state.database.begin()`.<br>**`record_gitlab_job` reads the stored row and then writes** (finding 5). Under a deferred `BEGIN`, a commit by another writer between the read and the write fails at once (WAL snapshot), and the busy timeout doesn't help. | **Add to "How #140 uses it":**<br>- After any `GitLabApi::job` call (no network call inside the transaction), #140 opens one `state.database.begin_with("BEGIN IMMEDIATE")` transaction.<br>- It runs `record_gitlab_job`, then `set_gitlab_runner_busy` and the `runner_events` insert in that transaction, and commits.<br>- The `webhook_deliveries` status updates stay outside it, as in `receive`.<br>- Any failure rolls back the `gitlab_jobs` row as well, so the GitLab `retry` arm reapplies everything.<br>**Post on #136, for #148:** its conclusion write and its busy clear run in one transaction inside the `retry_when_busy` closure, as the GitHub sync does at `main.rs:1963-1971`. `148-brief.md:131` says "in the same `retry_when_busy` write". Without a transaction, a rerun of the closure after a partial commit gets `Ignored` and the clear is lost.<br>**Test:** see Notes for O. |
| 3 | warn | Round-1 "Shared files", `state.rs`: "`main.rs` needs no wiring change for this; #146 adds `.with_gitlab(...)` with the real client." | cross-cutting (who owns a shared file's edit) | **That premise is now false.**<br>- #146 merged as PR #153 at 4:08 PM PST (00:08:38 UTC). It changed only `gitlab/http.rs`, its tests and two lines in `gitlab/mod.rs` (`pub mod http;`, `pub use http::GitLabClient;`).<br>- Its brief forbade any other edit. Its own plan review recorded that "No wave-1 issue owns wiring `Arc::new(GitLabClient::new()?)` into the API's and the reconciler's `main.rs`", and left that for #136. It isn't on #136 yet (the last comment is D18, 00:15:49 UTC).<br>- Master (`e8b18bf`) has no GitLab wiring in `gridops-api/src/main.rs`, in `state.rs`, or in `gridops-reconciler/src/main.rs`. #138's brief (`:126`) rests on the same premise.<br>**Effect.** Once #138 lets admins create connections, every `pending` delivery that matches a connection calls `UnconfiguredGitLab`. It fails, gets marked `failed`, and writes no row. #148's poll can't recover it either, because the reconciler has no wired client.<br>**Why only a warning.** Production wiring isn't in #140's issue text ("Tests use `FakeGitLab`"), and the gap is epic-wide. | **Correct the brief, then pick one owner and record it on #136.** Recommended:<br>- **API side:** whichever of #138 and #140 lands first adds `.with_gitlab(Arc::new(GitLabClient::new()?))` where `main.rs` builds `AppState`, and the second keeps one copy. Both already edit `main.rs`.<br>- **Reconciler side:** #139 and #148 do the same for `Reconciler.gitlab`.<br>**Branch note.** This branch is based on `b22c451`. F should merge master first, because `pub mod jobs;` goes next to #146's new lines in `gitlab/mod.rs`. |
| 4 | warn | D18 API: `project_id: u64`, `job_id: u64`, `pipeline_id: u64`, `gitlab_runner_id: Option<u64>`; `set_gitlab_runner_busy(.., gitlab_runner_id: u64, ..) -> Option<(String, Option<String>)>` | naming and types (the contract module's id type); pattern consistency | **A third way of representing GitLab ids, inside the module that already has a typed one.**<br>- `gitlab::Job`, `Scope` and `CreatedRunner` use `ExternalId` (`gitlab/mod.rs:59-60,104,121-123`). That type is bounded to `1..=i64::MAX` precisely so that it fits an SQLite `integer` (`fleet/domain.rs:37-52`).<br>- Core decodes GitLab ids from rows as `ExternalId` (`fleet/admission.rs:1447,1456`).<br>- `sqlx-sqlite` 0.9 supports `u64` for decoding only ("INTEGER (Decode only)"), so every bind of these `u64` fields needs a fallible `i64::try_from`.<br>- #148 holds `Job { id: ExternalId, .. }` and would have to unwrap it to `u64`.<br>- Separately, `runners.pool_id` is `text NOT NULL` (`0047_gitlab.sql:124`), while `associate_runner_with_job` returns `(String, String)`. | **In the same D18 amendment as finding 1:**<br>- use `ExternalId` for `project_id`, `job_id`, `pipeline_id`, `gitlab_runner_id` and `previous_gitlab_runner_id`, both in `JobObservation` / `JobWriteOutcome` and in `set_gitlab_runner_busy`;<br>- return `Option<(String, String)>`.<br>The webhook parses payload ids into `ExternalId` at the parse boundary. A missing or zero id is recorded and marked processed with no effect, like the brief's other malformed-`build` cases. |
| 5 | warn | D18 "Rules (one SQL statement per `record_gitlab_job` call ...)"; Tests: "against an in-memory migrated database" | abstraction level; pattern consistency (tests) | **(a) One statement can't produce the outcome D18 promises.** SQLite's `RETURNING` gives post-update values, and no row when the `DO UPDATE ... WHERE` is false. `previous_status`, the `Inserted` / `StatusApplied` / `FilledOnly` split, and `status` on `Ignored` all need the stored row read first.<br>**(b) No core test uses an in-memory database.** Core tests use a temp-dir file database from `connect_database_path`, which runs the private `MIGRATOR` (`db.rs:17,326-329`, `autoscaling.rs:1035-1039`, `fix_agent.rs:214-216`). An in-memory pool gives each connection its own empty database. | **Restate:**<br>- (a) "one write statement, preceded by a read of the stored row on the same connection". Finding 2's transaction makes the two atomic.<br>- (b) the unit tests use `connect_database_path` on a temp directory, as the neighbouring core tests do. |

### Accepted as consistent

- **`crates/gridops-core/src/gitlab/jobs.rs`** as the home of the shared writer. Core domain modules own their SQL (`fleet/targets.rs`, `fleet/admission.rs`, `fleet/registry/*`). `gitlab/` already holds more than the contract (#146's `http.rs`).
- **The D18 exception** is scoped to one new file plus `pub mod jobs;`.
- **`&mut SqliteConnection` parameters.** They match the reconciler's sync writers (`main.rs:1886,1993`), and they are needed because the function runs several statements.
- **The `base_url` busy match**, which writes only `busy`, `status`, `last_heartbeat_at` and `updated_at`.
- **The tier table, rules and test list** in `gridops-core`, with end-to-end webhook tests in `gridops-api`.

## Notes for O

Amend `docs/handoffs/140-brief.md` as below, post the two #136 notes, then start a **fresh** run. Don't resume this one.

### Block 1: D18 outcome and busy rule (add under a new "Amendments (architecture review, round 4)")

> **`JobWriteOutcome` gains `pub previous_gitlab_runner_id: Option<ExternalId>`**: the stored runner id before the call, `None` when the call inserted the row. This is an additive D18 amendment, recorded on #136.
>
> **Busy set (replaces the `running` bullet's condition in "How #140 uses it"):** whatever the `write` variant (`Inserted`, `StatusApplied` or `FilledOnly`), when `outcome.status == "running"`, `outcome.gitlab_runner_id` is `Some(R)` and `outcome.previous_gitlab_runner_id` is `None`, mark R busy with `set_gitlab_runner_busy` and write the `runner_events` row. Otherwise, no busy effect. The terminal clear rule is unchanged.

**Add to Done when:**

- **First-seen `running`.** A `running` event with `runner.id` R for a job with no row: the event inserts the row, marks R busy and writes one `runner_events` row. A `pending` event for the same job delivered afterwards is ignored and makes no `GitLabApi::job` call.
- **`running` after a failed `pending` fetch.** `FakeGitLab::fail_next(.., FakeMethod::Job, ..)` fails the `pending` delivery, so no row is written. The `running` event that follows marks R busy.
- **The fill is recognised under clock skew.** A row stored as `running` with a NULL runner at time T, as #148's poll writes it, followed by a webhook `running` event with R and a time **later** than T: the outcome is `StatusApplied`, R is marked busy, and one `runner_events` row is written.
- **No double set.** A second `running` event for the same job with the same R and a later time writes no second busy change and no second `runner_events` row.

### Block 2: transaction boundary (same section)

> **One transaction per delivery.** After any `GitLabApi::job` call, open `state.database.begin_with("BEGIN IMMEDIATE")`. Run `record_gitlab_job`, then `set_gitlab_runner_busy` and the `runner_events` insert where the rules call for them, and commit. Any error rolls back all three, so the delivery is marked `failed` with nothing written, and the GitLab `retry` arm reapplies it in full. The `webhook_deliveries` status updates stay outside the transaction, as in `receive`. (Precedent: `gridops-core/src/fleet/targets.rs:542`, `fleet/admission.rs:169`.)

**Add to Done when:**

- **Partial failure is recoverable.** A test adds a temporary SQLite trigger that aborts inserts into `runner_events`. A `running` event is then delivered. The delivery ends `failed`, there is no `gitlab_jobs` row, and R is not busy. After the trigger is dropped, a retry through the GitLab arm writes the row, marks R busy, writes one `runner_events` row and ends `processed`. The same test with a terminal event leaves R busy until the retry, which clears it.

### Post on #136

1. **D18 amendment:** add `previous_gitlab_runner_id` to `JobWriteOutcome`. If warning 4 is adopted, also switch the id fields to `ExternalId` and the pool id to `String`. #148's local copy changes to match.
2. **For #148:** the conclusion write and its busy clear run in one transaction inside the `retry_when_busy` closure, as the GitHub sync does at `gridops-reconciler/src/main.rs:1963-1971`. A rerun after a partial commit otherwise gets `Ignored`, because a terminal row is frozen.
3. **Client wiring (warning 3):** name the owner of `.with_gitlab(Arc::new(GitLabClient::new()?))` in the API's `main.rs` and of `Reconciler.gitlab` in the reconciler. #146 merged without either.

### Warnings: cheap to settle in the same amendment

3. Replace "#146 adds `.with_gitlab(...)`" with the owner chosen on #136. If that owner is #140, add the one `main.rs` line to the shared-file list. Note that F merges master before starting.
4. Switch the D18 id types to `ExternalId`, and `set_gitlab_runner_busy` to return `Option<(String, String)>`.
5. Reword "one SQL statement" as "one write statement after a read of the stored row". Make the core unit tests use `connect_database_path` on a temp directory.

## Patterns referenced

- **`crates/gridops-api/src/webhooks.rs`:**
  - `receive`, `:25-111`, and `retry`, `:113-185`;
  - `process_workflow_job`, `:562-654`, which writes in sequence with no transaction and repairs itself on retry.
- **`crates/gridops-core/src/db.rs`:**
  - `:17` and `:326-329` (`MIGRATOR` and the temp-file test database);
  - `:32` (5 s busy timeout);
  - `:195-245` (`associate_runner_with_job`).
- **`crates/gridops-core/src/fleet/`:**
  - `registry/mod.rs:6-7`, `targets.rs:542`, `admission.rs:169,1447`, `service.rs:135` (`BEGIN IMMEDIATE`, and `ExternalId` from rows);
  - `domain.rs:15-58` (`ExternalId`).
- **`crates/gridops-core/src/gitlab/mod.rs`:** `:59-60`, `:104`, `:120-126` (`ExternalId` in the contract).
- **`crates/gridops-reconciler/src/main.rs`:**
  - `:482-500` (`retry_when_busy`);
  - `:1396-1438` (`maybe_scale_down`);
  - `:1963-1971` (the GitHub sync's transaction inside `retry_when_busy`).
- **Epic #136's comments** (D18 at 00:15:49 UTC).
- **The sibling plans:** `gitlab-148-scaling-71c514/docs/handoffs/148-brief.md` at `572f828` (`:120`, `:131`, `:155-191`), and `gitlab-138-api-44d3a8/docs/handoffs/138-brief.md:124-127`.
- **#146 on master:** PR #153's file list, and `docs/handoffs/146/decisions.md:15,24`.
