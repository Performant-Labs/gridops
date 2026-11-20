## Brief Review (Round 1)

### BLOCK findings
None.

### Resolved findings
**[R-1]** Earlier **[B-1]**: "The Done-when test list was not extended to cover the amended behavior… Add the amended cases to Done when." — Resolved: "Brief gate round 1 additions" adds tests for the tie rule (both orders), timestamp fallback (missing and unparsable), multiple candidates with tie-breaks, group prefix at a `/` boundary, retry through the GitLab arm, non-`build` events, and the W-2 stale-concluded-event case.

### Needs-verification findings
**[NV-1]** The delivery-id derivation relies on GitLab's `Idempotency-Key` retry behavior (NV-2 excerpt summarizes the docs page but the page text itself is not shown verbatim) — a verbatim excerpt of GitLab's webhook header table for `Idempotency-Key` and `X-Gitlab-Event-UUID` would settle it.
**[NV-2]** The claim that the OpenAPI entry documents exactly the two required headers and the 202/400/401 responses — the NV-3 excerpt reports an extraction result but `docs/openapi/gridops-v1.json` itself is not excerpted here; the path entry's `parameters` and `responses` blocks would settle it.
**[NV-3]** The claim (NV-5 excerpt) that `retry` (`webhooks.rs:113-185`) is structured so one GitLab arm keyed on the `gitlab:` prefix can be added without touching GitHub's arm — an excerpt of the `retry` function body would settle it.

### WARN findings
**[W-1]** Duplicate semantics mismatch: the brief says "A delivery whose `id` already exists with status `processed` is acknowledged without reprocessing, the way `receive` handles a duplicate," but the shown excerpt (NV-5) notes `receive` today treats **any** existing `id` as a duplicate regardless of status. If `gitlab.rs` copies that behavior, a GitLab retry of a `failed` delivery is silently acknowledged 202 and never reprocessed; if it implements the "status = processed only" rule, it diverges from `receive`. State which one, and how the reuse of extracted helpers accommodates the difference.
**[W-2]** (unresolved from round 1) `pool_id` "may stay NULL here unless the pool is unambiguous from `runner_pool_gitlab_connections`" is still untestable as written. Specify: always NULL in #140, or fill only when exactly one `runner_pool_gitlab_connections` row matches the connection.
**[W-3]** (round-1 W-1, unaddressed) A stale `pending` still performs the `GitLabApi::job` fetch before the guard rejects the upsert — the amendments pin the fetch-before-upsert ordering ("happens before the upsert… if it fails, the delivery is marked failed"), so a duplicate `pending` delivery (already ruled a 202 no-op elsewhere) that fails the fetch would be marked `failed` with no row written, contradicting the duplicate rule. Reconcile: either skip the fetch for deliveries answered as duplicates, or accept failed-marked stale pendings explicitly.

### NIT findings
**[NIT-1]** Earlier NIT resolved: `scope_path` is now listed in the `gitlab_connections` schema summary. Remaining nit: the NV-2 excerpt notes GitLab docs name `webhook-id` as preferred over `Idempotency-Key`; the brief's header order is unchanged — fine, but the choice could be recorded in `decisions.md`.

### Verdict
PASS — no BLOCK findings; implementation may proceed.