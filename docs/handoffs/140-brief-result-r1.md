## Brief Review (Round 1)

### BLOCK findings

**[B-1] Internal contradiction on whether #140 may touch `docs/openapi/gridops-v1.json`.** The Clarifications section says: "Do not change shared contract types (`gridops-core` gitlab module, domain, protocol, the migration, the OpenAPI spec). If the contract truly needs a change, stop and report it." Round 2's "Block 3: route side effects" then instructs: "`docs/openapi/gridops-v1.json`: remove only the sentence 'Not served yet: implemented by #140.' from this path's description." The round-2 amendments declare they supersede only "the named earlier text": the first and third bullets of the ordering guard, the tie-rule and W-2 test items, the pre-fetch check, and "every mention of 'Unix epoch seconds'". The no-spec-change clause is not among the named texts, so an implementer following the brief literally must "stop and report" when Block 3 tells them to edit the spec. The round-2 amendments must explicitly supersede (or carve out an exception to) the "Do not change … the OpenAPI spec" clause, or route the sentence removal through the mechanism the clarifications prescribe.

### Needs-verification findings

**[NV-1]** The GitLab Job Hook payload is assumed to carry `project.path_with_namespace` (group-prefix matching), `project_id`, `build_status_changed_at`, `runner.id`, `runner.tags`, `retries_count`, and `build_name`. — The exact Job events payload schema from GitLab's documentation (or a captured example) showing these fields; none of it is excerpted in this prompt.

**[NV-2]** `X-Gitlab-Event-UUID`, `Idempotency-Key` and `webhook-id` are actually sent on Job Hook deliveries by current GitLab, and `Idempotency-Key` is constant across retries (NV-2 quotes the docs' header table, which describes the headers generally, not per-event-type delivery). — A captured Job Hook delivery's headers, or the GitLab docs table restricted to Job events.

**[NV-3]** `FakeGitLab` (in `crates/gridops-core/src/gitlab/fake.rs`) supports `fail_next` and a call log sufficient for the required tests ("check the call log on `FakeGitLab`", `FakeGitLab::fail_next`). — An excerpt of `fake.rs` showing those facilities; the file was read but not excerpted.

**[NV-4]** The `retry` endpoint gives a specific error for "an unverified row" that the GitLab arm can reuse for `gitlab:unmatched:` ids, and the UI retry button's gating (`signatureValid`, line 122) hides retry for rejected GitLab rows. — The excerpted `webhooks.rs` retry body showing the unverified-row error path (NV-5 shows `retry` at `:113-185` but not that branch's text).

**[NV-5]** `subtle` (or an existing constant-time compare) is available in `gridops-api`'s dependency tree, and `webhook_deliveries` accepts the `gitlab:...` id shape (length/charset of `id`). — `crates/gridops-api/Cargo.toml` dependency list and the `webhook_deliveries` schema excerpt.

### WARN findings

**[W-1]** Delivery-id header order puts `Idempotency-Key` first even though NV-2 reports GitLab's docs name `webhook-id` as preferred. The stated rationale (retry stability) is defensible, but the decision should record that a future GitLab may deprecate `Idempotency-Key`, leaving the fallback order load-bearing. — Recommendation: note the doc preference in the code comment on the header order.

**[W-2]** No acceptance criterion covers the OpenAPI `400` path ("The payload is not a JSON object", per NV-3) — a malformed-body test is missing from both "Done when" lists. — Recommendation: add one test for a non-JSON body.

**[W-3]** `running` events without `runner.id` are unspecified: the scope bullet handles only "`running` with `runner.id`", and no rule says whether such an event upserts the row (with `gitlab_runner_id` NULL) and marks no runner busy. — Recommendation: state that behavior explicitly and add it to Done-when.

**[W-4]** A `pending` delivery whose tag fetch fails leaves no row, and a duplicate arriving later is acknowledged without reprocessing; recovery depends entirely on a human clicking retry (the UI button of `webhooks.tsx:122-125`). If nobody retries, the job never enters `gitlab_jobs` and #148's demand scan misses it. — Recommendation: at minimum record this operational gap (e.g. on #136 or the UI work), or have #148's poll cover it (which round 2 says it now does — confirm).

### NIT findings

**[NIT-1]** The retry parsing rule "strips the `gitlab:` prefix and splits once at the next `:`" assumes `<key>` never contains a colon. True for UUIDs and hex SHA-256, but `Idempotency-Key` is documented only as "unique", not colon-free; a defensive comment or a two-part check is cheap.

**[NIT-2]** The same logical delivery matched by two candidate connections produces two delivery rows (by design, W-4 option (a)) and, if both tokens match different connections, potentially two `gitlab_jobs` rows — the "Outside #140" note records the duplicate-jobs consequence but not the duplicate-delivery one.

### Verdict

BLOCK — 1 blocking finding ([B-1]); must resolve the OpenAPI-spec contradiction before implementation starts.