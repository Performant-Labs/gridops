## Brief Review (Round 1)

### BLOCK findings
None.

### Resolved findings
**[R-1]** Earlier B-1: "Internal contradiction on how many `webhook_deliveries` rows one delivery produces. … When one delivery matches two candidate connections, it produces two `webhook_deliveries` rows … Under W-4(a) as written, a single HTTP delivery cannot produce two rows … Fix NIT-2 to describe two deliveries, not one." The round-4 gate addition "Outside #140 (added, NIT-2)" now states: "One incoming HTTP delivery always resolves to exactly **one** connection and writes exactly **one** `webhook_deliveries` row … The handler never writes a second row for the same request. Two rows arise only from **two separate deliveries** of the same GitLab job event". The misstatement is corrected and the mechanism now matches W-4 option (a).

### Needs-verification findings
**[NV-1]** Claim: `AppState.gitlab`, `UnconfiguredGitLab` and `with_gitlab` are on master in the pinned shape (#138 merged as `0086aa9`), so #140 adds nothing to `state.rs` — `crates/gridops-api/src/state.rs` at that commit would settle it; the file is cited as read but not excerpted in this prompt.
**[NV-2]** Claim: `retry`'s unverified-row branch (`signature_valid` check) runs before dispatch and therefore refuses `gitlab:unmatched:` rows — `crates/gridops-api/src/webhooks.rs:113-185` excerpt would settle it; the shown excerpts do not include `retry`.
**[NV-3]** Claim: `receive`'s duplicate rule is "any existing id is a duplicate, acknowledged 202, not reprocessed" (NV-5 in the brief) — `webhooks.rs:25-111` excerpt would settle it; not shown here.
**[NV-4]** Claim: the GitLab Job-hook payload carries `project.path_with_namespace`, `build_status_changed_at`, `build_name`, `retries_count` in the assumed shapes — GitLab's "Job events" payload documentation or a captured payload would settle it; group prefix matching, timestamp parsing and the message format all depend on it.
**[NV-5]** Claim: `ExternalId` "already fits SQLite" (no raw `u64` bound) — `crates/gridops-core/src/fleet/mod.rs`'s `ExternalId` definition and its sqlx impls would settle it.
**[NV-6]** Claim: `set_gitlab_runner_busy` returns `Option<(String, String)>` for (runner id, pool id), implying `runners.pool_id` is NOT NULL — `runners` table definition in the migrations would settle it; the round-3 `Option<String>` shape and the round-4 `String` cannot both be right, and only the `runners` schema distinguishes them (0047's `gitlab_jobs.pool_id` is nullable, but that is a different table).
**[NV-7]** Claim: `Idempotency-Key` is constant across retries and `webhook-id` is preferred — the GitLab webhook header table (paraphrased as NV-2 in the brief, not reproduced) would settle it.
**[NV-8]** Claim: #148's poll inserts missing pending jobs with `tag_list` from `Job.tag_list` (cited `gitlab-148-brief.md:36` at `c48f951`) — that file's line 36 would settle it; the failed-fetch recovery argument depends on it.

### WARN findings
**[W-1]** Unspecified edge case: behavior when `X-Gitlab-Event` is absent. The OpenAPI marks it `required: true` and `webhook_deliveries.event` is `NOT NULL` (`0001_initial.sql:204`), but the brief never says what the handler does with a request missing it (400? empty string?). Add an acceptance criterion.
**[W-2]** The malformed-body (400) path says it "writes nothing to `gitlab_jobs` or `runners`" but never states whether a delivery row is recorded, unlike the 401 path which explicitly records one. State it, or cite the GitHub `receive` precedent.
**[W-3]** The status/write rules now span five amendment layers with partial supersessions ("replaces the corresponding D18 lines", "replaces the running bullet's condition", withdrawn bullets). An implementer reading linearly can follow a stale rule (round-1 tie rule, round-2 epoch seconds, round-3 `Option<String>` pool id). Recommend consolidating into one canonical rule section before the diff gate.
**[W-4]** D18 has #140 and #148 coding against exact shared signatures in parallel worktrees with "whichever merges second removes its local copy" as the only reconciliation plan. Record on #136 who owns the final signature diff and what happens if they drift.

### NIT findings
**[NIT-1]** The `now_millis()` fallback for a missing/unparsable `build_status_changed_at` is re-evaluated on retry, so a retried failed delivery can apply a different (later) timestamp than first receipt, potentially flipping whether the status write applies. Pin the fallback to the stored `webhook_deliveries.received_at`.
**[NIT-2]** The issue text's "Owns" paragraph ("Keep it to the added lines") is stale next to D18 (a new `gridops-core` module, `jobs.rs`). Annotate or supersede it explicitly.

### Verdict
PASS — no BLOCK findings; implementation may proceed.