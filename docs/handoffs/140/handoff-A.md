# Handoff-A: Phase 3 - GitLab job webhook ingestion and busy tracking (#140)  (up-front plan review, round 3)

**Date:** 2026-11-20
**Branch:** claude/gitlab-140-webhooks-a9fae5 (base b22c451; brief as of a4ed945)
**Brief reviewed:** docs/handoffs/140-brief.md, including "Amendments (architecture review, round 2)"   **Reuse map:** N/A. No `survey.md` was provided. The baseline is the analogue the brief names: the GitHub webhook path (`crates/gridops-api/src/webhooks.rs`), its shared write helpers in `crates/gridops-core/src/db.rs`, the GitHub sync in `crates/gridops-reconciler/src/main.rs`, and the cross-issue rules on epic #136.   **Wireframe:** N/A (no UI surface)
**Verdict:** BLOCK

Round 1 (BLOCK) is in git at `0a3d6d1`, and round 2 (BLOCK) at `8f36d51`.

## Summary

BLOCK, with one block finding. All three round-2 blocks are resolved, and so are the four round-2 warnings.

**The new block is a gap in the shared write rule.** The round-2 amendments made the status rule govern the whole `gitlab_jobs` row. But each of the two writers lacks one column that only the other can supply:

- **The webhook can't supply `tag_list`** without a fetch.
- **#148's poll can't supply `gitlab_runner_id`**, because the contract's `Job` has no runner field.

Whichever writer wins a same-tier race therefore leaves that column wrong for good. One case is a tagged job counted as untagged demand. The other is a running job whose runner is never marked busy.

**The GitHub analogue never meets this problem.** Both of its writers carry every column, and it fills runner identity with `COALESCE`. The fix is two column rules and a note on #136.

### Round-2 findings: status

| Round-2 # | Status |
|---|---|
| 1 (ordering guard vs #136) | Resolved. Block 1 is #136's rule word for word: terminal is final, a higher tier always applies, and at an equal tier the newer `status_changed_at` wins. Its tier table matches the #136 comments of 3:43:53 PM and 3:45:29 PM PST (23:43:53 / 23:45:29 UTC). `unknown` is never stored. The pre-fetch check uses the same predicate. The note that #148 is insert-only is withdrawn. |
| 2 (units) | Resolved. Block 2 uses epoch ms, parsed from `build_status_changed_at`, with `now_millis()` as the fallback. This is recorded on #136. |
| 3 (route side effects) | Resolved. Block 3 adds the `docs/fleet-api.md` row, the OpenAPI sentence removal and the `openapi.rs` entry. I checked the row against the behavioural test: `token_access.rs:1263` asserts, for an "ignored" row, only that `code` isn't `token_not_accepted` and that `authenticated` isn't `false`. `ApiError::Unauthorized` renders `{"error": ...}` (`error.rs:35,51`), so the handler's 400 or 401 passes. |
| 4 (retry's connection) | Resolved by option (a). The existing `retry` already refuses a `signature_valid = 0` row with "Only verified webhook payloads can be retried." (`webhooks.rs:134-138`), so a `gitlab:unmatched:` row is refused before any GitLab arm runs. |
| 5 (one tier table) | Resolved for #140 (one Rust table, with the SQL built from it). The #136 follow-up is posted. See warning 2 on #148's newer proposal. |
| 6 (D17) | Resolved and recorded on #136. |
| 7 (`runner_events`) | Resolved. See warning 4 for the one column the decision leaves out. |

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | block | Round-2 Block 1: "The update applies only when [rule]" (the whole upsert is gated by the status rule). Scope: `pending` fetches `tag_list`, and other statuses are silent on it. Done-when: "Within a tier, time decides. `created`, then `preparing`" (the webhook inserts tier-0 rows). | cross-cutting (column ownership between the two writers of one table); pattern consistency with the GitHub analogue | **Each writer lacks one column, and the whole-row guard throws away the other writer's copy.**<br>**(a) `gitlab_runner_id`, which only the webhook knows.**<br>- #148's conclusion step calls `job` and "write[s] its status" (`148-brief.md:37`). A `running` answer outranks a stored `pending` or tier-0 row, so the poll writes `running` with `status_changed_at` = its observation time (`:124-126`).<br>- `Job` has no runner field (`gridops-core/src/gitlab/mod.rs:120-126`), so `gitlab_runner_id` stays NULL.<br>- The webhook's `running` event is then the same tier. Its `build_status_changed_at` is the job's actual start, which is before the poll's observation time, so the guard rejects the whole update.<br>- The runner is never marked busy. The terminal clear "applies only to the runner recorded on that job". #148's A2-W4 clear also needs a non-null `gitlab_runner_id` (`148-brief.md:131`). So nothing ever marks or clears that runner for this job, and `maybe_scale_down` sees the pool as idle (`gridops-reconciler/src/main.rs:1408`).<br>- Two paths reach this: a delayed delivery, and the GitLab `retry` arm. With the retry arm, an admin retries a failed `running` delivery after the poll has moved the row. The retry ends `processed` and does nothing, which looks like a fix.<br>**(b) `tag_list`, which only the poll has in hand.**<br>- The webhook inserts tier-0 rows (`created` and the others) without a fetch, so they get the column default `[]` (`migrations/0047_gitlab.sql:69`). #148 reads `[]` as **untagged**, which "matches only pools with `run_untagged = 1`" (`148-brief.md:27`). In this case `[]` means "not fetched yet", which is a different fact stored as the same value.<br>- The poll "refresh[es] the status" when GitLab lists the job as pending (`:36`), which moves the row to tier 1. The webhook's own `pending` is the same tier with an older time, so the pre-fetch check skips `GitLabApi::job`, and the tags are never fetched.<br>- The window is the webhook's delivery latency inside each 30 s poll. That is a few percent of every job that has a `created` event, which covers every later-stage job in a pipeline. Such a job is matched as untagged, so it scales the wrong pool or none.<br>- The brief also doesn't say whether a non-fetch update (`running`, terminal, tier 0) overwrites `tag_list`.<br>**Why the GitHub pattern doesn't carry over unchanged.** Both GitHub writers carry `runner_id` and `labels` on every write: the webhook at `webhooks.rs:586-591` and the sync at `gridops-reconciler/src/main.rs:2003-2005`. Runner identity is filled with `github_runner_id=COALESCE(github_runner_id,?)` (`gridops-core/src/db.rs:210,225`). For GitLab, neither writer has the full row. | **Add two column rules to the guard,** in the same Rust source as the tier table.<br>**(a) `gitlab_runner_id`:**<br>- A `running` event with `runner.id`, arriving at a stored `running` row whose `gitlab_runner_id` is NULL, fills it with `COALESCE`, even when the status rule rejects the status write.<br>- `status` and `status_changed_at` keep the values the status rule decides.<br>- The fill counts as "the upsert changed the row" for the busy gate. It marks that runner busy and writes the `runner_events` row.<br>- A terminal row is never filled.<br>- No writer clears `gitlab_runner_id`, and the poll never writes it.<br>**(b) `tag_list`:**<br>- #140 writes `tag_list` only from a `pending` fetch.<br>- An insert at any other status writes `[]`.<br>- An update without a fetch keeps the stored value.<br>**Post on #136, for #148:**<br>- A poll write that moves a row into `pending` also writes `tag_list` from `Job.tag_list`.<br>- The poll never writes `gitlab_runner_id`.<br>**Tests:** see Notes for O. |
| 2 | warn | "The tier table is written once in Rust in `webhooks/gitlab.rs` ... Moving the table into a `gridops-core` helper shared with #148 is a follow-up" | abstraction level; dependency direction (one rule, two crates) | **#148's round-3 plan review proposes doing that move now.** Its warning 1 (`gitlab-148-scaling-71c514/docs/handoffs/148/handoff-A.md:39`, at 3:52 PM PST) proposes `record_gitlab_job` and `conclude_gitlab_job` in `gridops-core`, beside `associate_runner_with_job`, added by whichever of #140 and #148 lands first.<br>**The two briefs would then disagree.** If #148's O adopts that and #140's brief is unchanged, #140 writes a second copy while `gridops-core` already has the helper. Finding 1's column rules make the shared rule bigger, so the cost of two copies goes up.<br>**Why only a warning.** #140's brief records the two-copy choice in writing, and #148's brief at `c48f951` hasn't adopted the helper. | **Decide once for both issues, and record it on #136:**<br>- **Either keep two copies.** Then finding 1's column rules go into both #140's and #148's text.<br>- **Or adopt the core helper.** Then add `crates/gridops-core/src/db.rs` and its `lib.rs` re-export to #140's shared-file list, with "first to land adds it, the second calls it", the same as #148's A-W2 `gitlab_credentials`. Also relax "Do not change shared contract types" for that one non-contract helper. |
| 3 | warn | "Match. Runners are matched by `gitlab_runner_id` among `ci_platform = 'gitlab'`, non-deleted runners whose `gitlab_connection_id` is a connection on the matched connection's `base_url`" | cross-cutting (one busy flag, two matching rules) | **#148 clears busy with a narrower match.** Its A2-W4 matches the runner on the row's own connection (`148-brief.md:131`). Its round-3 review pins that as `runners.gitlab_connection_id = gitlab_jobs.connection_id` (`148/handoff-A.md:42`). #140 matches across every connection on the instance.<br>**The narrower match misses a case.** A job delivered through a project connection's webhook can run on a runner #139 registered under the parent group's connection. #148's clear then misses that runner, which stays busy, and `maybe_scale_down` returns early for the pool every pass (`main.rs:1408`). #140's own clear is correct. | **Post on #136:** both writers find the busy runner by `gitlab_runner_id` among live GitLab runners on the same `base_url`, as #140's brief states. No #140 change is needed. |
| 4 | warn | W-7 `runner_events` decision; W-4 delivery id `gitlab:<connection_id>:<key>` | pattern consistency; naming | **Two small points the brief leaves to F.**<br>- **`runner_events.message` is `NOT NULL`** (`migrations/0001_initial.sql:89`), and the W-7 decision doesn't give a value for it. The GitHub row uses `"{name} · run {run_id}"` (`webhooks.rs:642-645`).<br>- **The retry arm has to split `gitlab:<connection_id>:<key>`.** The key comes from a header and can itself contain `:` (brief-gate r1 W-4). | **Pin both:**<br>- `message` = `"{build_name} · pipeline {pipeline_id}"`;<br>- the retry arm strips `gitlab:`, then splits once at the first `:`. Everything before it is the connection id, and everything after it is the key. |

## Notes for O

Amend `docs/handoffs/140-brief.md` as below, then start a **fresh** run. Don't resume this one.

### Block 1: column rules (add under "Amendments (architecture review, round 3)")

> **Column rules for the guarded upsert.** The status rule (round 2, Block 1) decides `status` and `status_changed_at`. Two columns follow their own rules, because each writer lacks one of them:
>
> - **`gitlab_runner_id`:** a `running` event that carries `runner.id`, arriving at a stored `running` row whose `gitlab_runner_id` is NULL, fills it (`COALESCE`), even when the status rule rejects the status write. `status` and `status_changed_at` are left as the status rule decides. That fill counts as a change for the busy gate: it marks that runner busy and writes the `runner_events` row. A terminal row is never filled. No writer clears `gitlab_runner_id`.
> - **`tag_list`:** written only from a `pending` event's `GitLabApi::job` fetch. An insert at any other status writes `[]`, and an update without a fetch keeps the stored value.
>
> Both rules live in the same Rust source as the tier table.

**Add to "Done when":**

- **The runner is filled after the poll wins.** A row stored as `running` with `gitlab_runner_id` NULL and `status_changed_at` T, as #148's poll writes it, followed by a webhook `running` event with `runner.id` R and an older `status_changed_at`. Afterwards `gitlab_runner_id` = R, the runner with `gitlab_runner_id` R is busy, `status` and `status_changed_at` are unchanged, and one `runner_events` row is written. The same event against a terminal row changes nothing.
- **The tags survive a later event.** After a `pending` event's fetch stored tags `["a","b"]`, a `running` event and then a `success` event leave `tag_list` = `["a","b"]`.
- **A non-`pending` insert writes `[]`.** A first-seen `created` event inserts a row with `tag_list` = `[]` and makes no `GitLabApi::job` call.

**Post on #136, for #148:**

- A poll write that moves a `gitlab_jobs` row into `pending` also writes `tag_list` from `Job.tag_list`. The webhook stores `[]` for rows it hasn't fetched yet, and its own `pending` loses the same-tier race to the poll and skips the fetch.
- The poll never writes `gitlab_runner_id`. Only the webhook knows the runner.

### Warnings: cheap to settle in the same amendment

2. **Where the rule lives.** Make one decision with #148's O about its round-3 warning 1, the `gridops-core` helper, and record it on #136. If the helper is adopted, add `crates/gridops-core/src/db.rs` and `lib.rs` to #140's shared files on a "first to land adds it" basis.
3. **The busy match.** Post on #136 that both writers match the busy runner by `gitlab_runner_id` among live GitLab runners on the same `base_url`.
4. **Two pins.**
   - The `runner_events.message` is `"{build_name} · pipeline {pipeline_id}"`.
   - The retry arm strips `gitlab:` and splits once at the next `:`.

## Patterns referenced

- `crates/gridops-api/src/webhooks.rs`:
  - `receive`, `:25-111`, and `retry`, `:113-185`, including the verified-only check at `:134-138`;
  - `process_workflow_job`, `:562-650`, with its whole-row upsert of `runner_id` and `labels` at `:586-591` and its `runner_events` write at `:635-650`.
- `crates/gridops-core/src/db.rs:195-245`: `associate_runner_with_job` and its `COALESCE(github_runner_id,?)`.
- `crates/gridops-reconciler/src/main.rs`:
  - the GitHub sync's job upsert at `:2003-2005`;
  - `maybe_scale_down`'s busy check at `:1396-1438`.
- Schema and the matrix test:
  - `migrations/0047_gitlab.sql:62-82`, the `gitlab_jobs` columns, with `tag_list` defaulting to `[]`;
  - `migrations/0001_initial.sql:83-94`, `runner_events`;
  - `crates/gridops-core/src/gitlab/mod.rs:120-126`, `Job`;
  - `crates/gridops-api/tests/fleet_http/token_access.rs:1195-1290`, the matrix assertions for "ignored".
- Epic #136's comments, read with `gh api`, with timestamps (3:30:59 PM, 3:43:53 PM and 3:45:29 PM PST).
- The sibling plan: `gitlab-148-scaling-71c514/docs/handoffs/148-brief.md` at `c48f951` (`:27`, `:36-37`, `:120-131`), and its round-3 `handoff-A.md` (`:39`, `:42`).
