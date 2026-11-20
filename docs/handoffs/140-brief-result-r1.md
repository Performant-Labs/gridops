## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** Claim: a retried job carries a new `build_id`, so it lands as a new row under the unique key `(connection_id, job_id)` — the evidence that would settle it is the GitLab Job events webhook documentation (payload schema for `build_id` / `retries_count` on retries), confirming retries produce a distinct job/build id rather than reusing the original.
**[NV-2]** Claim: GitLab deliveries can be recorded "in the same table as GitHub deliveries" with duplicate-delivery detection — the evidence that would settle it is the migration/schema of that deliveries table (file:line of its CREATE TABLE and any unique key on a delivery/event id) plus confirmation that GitLab build payloads carry a usable per-delivery idempotency key (GitHub's `X-GitHub-Delivery` has no shown GitLab analogue; `build_id` + status + timestamp is a weaker substitute).
**[NV-3]** Claim: `object_kind: "build"` events always populate `build_status_changed_at` (the ordering rule depends on it; `skipped` jobs in particular are commonly never scheduled) — the evidence is the GitLab webhook payload documentation or a captured payload excerpt for each of `pending`, `running`, `success`, `failed`, `canceled`, `skipped`.
**[NV-4]** Claim: matching a group-scoped connection via "the payload's `project` path namespace" correctly identifies membership — evidence would be a sample build payload showing `project.namespace` / full path structure, confirming the stored group is derivable and that nested-subgroup paths resolve unambiguously.
**[NV-5]** Claim: the request/response shapes of `POST /api/v1/webhooks/gitlab` in `docs/openapi/gridops-v1.json` exist and are implementable as described — evidence is the path definition in that file (line references for the request body and error responses, including whether 401 is the specified response for no matching connection).

### WARN findings
**[W-1]** The group-matching rule is stated as an unresolved disjunction: "the payload's `project` path namespace, **or** the stored group." Two different implementations satisfy this sentence, and the "unknown project" acceptance test cannot distinguish them. Pick one rule before implementation.
**[W-2]** Busy clearing on conclusion depends on `gitlab_runner_id` having been recorded by an earlier `running` event. The brief has no error path or acceptance test for concluding a job whose runner id is unknown (running event lost, or never sent for `skipped`). Specify the behavior (no-op clear is fine, but say so) and consider a test.
**[W-3]** No error path for the `GitLabApi::job` tag fetch during `pending` (network failure, 404, deleted job). Specify: does the delivery fail (5xx / retry), or is the job upserted with an empty `tag_list`? Currently untestable.
**[W-4]** Out-of-order interplay with retries is unspecified: a stale event for a retried job has an old `build_id` and lands on the *old* row by the unique key — presumably harmless, but if the old row was already concluded and a stale `running` arrives for it, the handler would "mark busy" on a runner based on a dead job. State that stale events for concluded/replaced rows are ignored (or are not, and why).
**[W-5]** "Ignore an event older than the stored status" doesn't state tie-breaking for equal `status_changed_at` (GitLab re-sends). The duplicate-delivery test partially covers this only if dedup is by delivery key; if it's by timestamp, equal timestamps need a rule.

### NIT findings
**[NIT-1]** The Done-when list omits a token-mismatch test distinct from no-connection: "a bad token" may mean either; state both are covered (valid token wrong secret vs. no connection at all), since both must return 401 but via different lookups.
**[NIT-2]** "Route registration in `main.rs` is shared with #138. Keep it to the added lines" is vague about merge-order reconciliation for `state.rs` (the fallback instructions are good; add the same explicit "whichever lands second reconciles" note for the route line, which you did only for the `AppState.gitlab` field).

### Verdict
PASS — no BLOCK findings; implementation may proceed.