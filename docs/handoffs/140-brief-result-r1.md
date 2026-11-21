## Brief Review (Round 1)

### BLOCK findings

**[B-1]** Internal contradiction on how many `webhook_deliveries` rows one delivery produces. Round-2 amendment "Warnings settled (W-4): option (a)" fixes the delivery id as `gitlab:<connection_id>:<key>` where `<connection_id>` is the **single** connection whose secret matched the token (with a deterministic preference when several match), so one incoming delivery resolves to exactly one id and one row. But the round-3 gate addition "Outside #140 (added, NIT-2)" states: "When one delivery matches two candidate connections, it produces two `webhook_deliveries` rows (`gitlab:<connection_a>:<key>` and `gitlab:<connection_b>:<key>`)". Under W-4(a) as written, a single HTTP delivery cannot produce two rows — only one candidate's secret matches, and the preference rule picks one when several do. Two rows arise from two *separate deliveries* (GitLab sending the event to a project hook and a group hook), which is the mechanism described in the earlier "Outside #140" note about duplicate `gitlab_jobs` rows. NIT-2 misstates the mechanism and, read alone, invites an implementer to write both rows. Fix NIT-2 to describe two deliveries, not one.

### Needs-verification findings

**[NV-1]** Claim: `Idempotency-Key` is constant across GitLab retries and `webhook-id` is preferred (NV-2, header-order decision) — the GitLab webhook documentation header table (fetched page) would settle it; the excerpt is not reproduced in this prompt, only paraphrased.

**[NV-2]** Claim: `AppState.gitlab`, `UnconfiguredGitLab` and `with_gitlab` exist on master "in exactly the shape pinned" (#138 merged as `0086aa9`) — `crates/gridops-api/src/state.rs` at that commit would settle it; the file is listed as read but not excerpted.

**[NV-3]** Claim: `retry`'s unverified-row branch runs before dispatch and refuses `gitlab:unmatched:` rows (NV-4 cites `webhooks.rs:134-138`) — the shown source excerpt of `webhooks.rs` does not include those lines; the file text at `:113-185` would settle it.

**[NV-4]** Claim: `receive`'s bookkeeping behaves as NV-5 describes (insert `ON CONFLICT DO NOTHING`, duplicate 202, rejected-then-401, error truncated to 2,000 chars) — `crates/gridops-api/src/webhooks.rs:25-111` excerpt would settle it.

**[NV-5]** Claim: the GitLab Job webhook payload carries `project.path_with_namespace`, `build_status_changed_at`, `build_name` and `retries_count` in the shapes the brief assumes (group prefix matching and timestamp parsing depend on these) — GitLab's "Job events" payload documentation, or a captured payload, would settle it.

**[NV-6]** Claim: the contract's `ExternalId` "already fits SQLite" (no raw `u64` bound) — `crates/gridops-core/src/fleet/mod.rs`'s `ExternalId` definition and its sqlx `Encode`/`Type` impls would settle it.

**[NV-7]** Claim: `set_gitlab_runner_busy` returns `(String, String)` for `(runner id, pool id)` — `runners.pool_id` nullability in the migrations would settle it; if `pool_id` is nullable the round-3 `Option<String>` shape was right and the round-4 change loses information.

**[NV-8]** Claim: #148's poll "inserts any job missing from `gitlab_jobs` that GitLab still lists as pending, with `tag_list` from `Job.tag_list`" (cited `gitlab-148-brief.md:36` at `c48f951`) — that file's line 36 would settle it; the W-4 failed-fetch recovery path depends on it.

### WARN findings

**[W-1]** Missing edge case: behavior when `X-Gitlab-Event` is absent. The OpenAPI (NV-3) marks it `required: true`, `webhook_deliveries.event` is `NOT NULL`, and the id/event columns both consume it — but the brief never says what to do with a request lacking the header (400? default value?). Add a criterion.

**[W-2]** Missing edge case: non-JSON / non-object body handling is specified (400 with `ApiErrorBody`), but the interaction with the delivery id (SHA-256 of raw body) and whether a rejected-for-parse delivery is *recorded* is left implicit — the 401 path records a row, the 400 path says only "writes nothing to gitlab_jobs or runners". State whether a 400 body is recorded in `webhook_deliveries` (GitHub's `receive` precedent would settle it).

**[W-3]** D18 makes #140 and #148 share exact signatures across two parallel worktrees ("#148 may stub against these exact signatures; whichever merges second removes its local copy"). This is a live merge-conflict risk with no acceptance criterion covering the reconciliation; recommend recording on #136 who owns the final signature diff.

**[W-4]** The tie/terminal status rule now spans four revision layers (rounds 1–4 plus D18), and several earlier rounds are only partially superseded ("replace the corresponding D18 lines", "replaces the running bullet's condition"). An implementer reading linearly can follow stale rules (e.g., round-1 tie rule, round-2 epoch-seconds, round-3 `Option` pool id). Recommend the brief be re-published as a single consolidated rule set before the diff gate.

### NIT findings

**[NIT-1]** "Timestamp fallback equals the receive time (`now_millis()`)" makes the fallback non-deterministic across retries: a retried failed delivery would parse a *different* fallback timestamp than the original receipt, potentially changing whether the status write applies. Consider pinning the fallback to the stored `webhook_deliveries.received_at`.

**[NIT-2]** The "keep it to the added lines" instruction in the issue text is long since overtaken by D18 (a new `gridops-core` module) — delete or annotate the original "Owns" paragraph to avoid a stale ownership claim.

### Verdict

BLOCK — 1 blocking finding (B-1); must resolve before implementation starts.