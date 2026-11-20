## Brief Review (Round 1)

### BLOCK findings

**[B-1]** The Done-when test list was not extended to cover the amended behavior. The amendments add several new, testable behaviors — the equal-`status_changed_at` tie rule (lifecycle-rank comparison), the missing/unparsable `build_status_changed_at` fallback to receive time, multi-candidate token matching with project-scoped/oldest tie-break, group-scoped prefix matching at a `/` boundary, retry through the new GitLab arm in `retry`, and non-`build` event kinds recorded with no effect — but "Done when" still lists only the original seven cases (bad token, unknown project, pending tag lookup, out-of-order, retried job, duplicate delivery, busy set/cleared). The tie rule in particular is the only thing preventing the out-of-order guard from being non-deterministic on ties, and nothing in the brief requires a test of it. Add the amended cases to Done when.

### Needs-verification findings

**[NV-1]** The claim that the deliveries UI labels an unlabeled row "GitHub App" at `src/routes/_app/webhooks.tsx:146` — the excerpt ends at line 131; only `:119` is shown (and it does show the `"GitHub App"` fallback). A line-range excerpt of `webhooks.tsx` covering line 146 would settle it.

**[NV-2]** The claim that GitLab sends `Idempotency-Key` constant across retries (and `X-Gitlab-Event-UUID` as a per-delivery alternative) — a GitLab webhooks documentation excerpt ("Job events" payload/headers page) would settle it; nothing in this prompt shows GitLab's retry header behavior.

**[NV-3]** The claim that `docs/openapi/gridops-v1.json` documents `POST /api/v1/webhooks/gitlab` with required headers exactly `X-Gitlab-Token` and `X-Gitlab-Event` — the OpenAPI path entry would settle it; it is cited but not excerpted.

**[NV-4]** The claim that `gitlab::GITLAB_ACTIVE_JOB_STATUSES` / `JobStatus::is_active()` and the `JobStatus` serialization match migration 0047's partial-index statuses — an excerpt of `gridops-core`'s gitlab module and `migrations/0047_gitlab.sql` would settle it.

**[NV-5]** The claim that the `runners` busy gating "as `webhooks.rs` already does for GitHub" exists, and that delivery bookkeeping helpers can be extracted from `receive` without behavior change — an excerpt of `crates/gridops-api/src/webhooks.rs` showing the guarded upsert and duplicate/invalid-signature recording would settle it.

### WARN findings

**[W-1]** The `pending` tag fetch failing marks the delivery failed with no row written, but a stale `pending` arriving after `running` still triggers the fetch before the guard rejects the update — an unnecessary upstream call on every out-of-order/duplicate pending. Consider noting the fetch is best-effort ordering (or only fetched when the upsert will apply); at minimum confirm the extra call is acceptable.

**[W-2]** "Clear busy only applies to the runner recorded on that job" plus the guard running busy effects only when the upsert changed the row leaves a gap: if a `running` event for runner A is superseded by an out-of-order-ignored state, no busy write occurs for A — fine — but a `failed` event whose upsert is ignored as stale will also skip clearing busy, potentially leaving a runner busy after its job concluded via an older-timestamped delivery that was already processed. Recommend an explicit acceptance test for "concluded-but-stale event does not corrupt busy state."

**[W-3]** `pool_id` "may stay NULL here unless the pool is unambiguous" is untestable as written (ambiguity is undefined — unambiguous by what query?). Specify: NULL always in #140, or fill only when exactly one `runner_pool_gitlab_connections` row matches the connection.

### NIT findings

**[NIT-N]** The group-matching rule changed between the issue text ("the payload's `project` path namespace, or the stored group") and the amendment (`scope_path` prefix at a `/` boundary); the amendment says it wins, but the issue text's `scope_path` column is not listed in the `gitlab_connections` schema summary — add it there for consistency.

### Verdict

BLOCK — 1 blocking finding; must resolve before implementation starts.