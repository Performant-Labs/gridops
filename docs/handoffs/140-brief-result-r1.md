## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** NV-5's claims about `crates/gridops-api/src/webhooks.rs` (`:52-66` insert with `ON CONFLICT(id) DO NOTHING`, `:68-73` duplicate answer, `:74-81` invalid-signature recording, `:113-185` inline retry updates, `:531` the `run_attempt` guard, `:613-625` the re-read busy gate, and "retry refuses an unverified row" with a specific error) — no excerpt of `webhooks.rs` is attached; the exact file:lines shown would settle each.
- **[NV-2]** NV-3's claims about `docs/openapi/gridops-v1.json` (exactly two required headers, the "Not served yet" sentence, the `400`/`401`/`202` responses, `security: []`) — the path entry itself is not excerpted.
- **[NV-3]** NV-4's claim that `JobStatus` uses `#[serde(rename_all = "snake_case")]` with `#[serde(other)] Unknown` and that `is_active()`/`is_terminal()`/`as_str()` behave as described (`mod.rs:144-145` shows the `other` but not the rename attribute or the impl bodies at `:191`) — excerpt `mod.rs:159-` onward would settle it.
- **[NV-4]** `src/routes/_app/webhooks.tsx:146` carrying the "GitHub App" fallback label — the excerpt ends at line 130; the line itself would settle it.
- **[NV-5]** The claim that `access::tests::the_documented_matrix_lists_exactly_the_registered_routes` and `the_gitlab_operations_are_specified_before_they_are_served` exist and pass with the proposed `docs/fleet-api.md` row (`— | — | — | ignored` in the right column order) — the test source and the doc's current table would settle it.
- **[NV-6]** The GitLab header semantics in NV-2 (`Idempotency-Key` constant across retries, `webhook-id` "preferred", UUID behavior for recursive webhooks) — the cited GitLab docs page; only the brief's paraphrase is in the prompt.
- **[NV-7]** The round-2 Block 1 claim that epic #136 states the tier table (0–3) and "Writers never store `unknown`" — the epic comments of 2026-11-20 are not excerpted; the quoted comment text would settle it.
- **[NV-8]** `state.rs` currently having no `gitlab` field and `AppState::new`'s current signature (relevant to the "add exactly one field" instruction) — `crates/gridops-api/src/state.rs` is listed as read but not excerpted.

### WARN findings
- **[W-1]** No acceptance test for the OpenAPI's `400` ("The payload is not a JSON object") response path — the brief commits the handler to match the spec's response shapes but the "Done when" lists omit a malformed-payload test. Add one.
- **[W-2]** Busy clear assumes one job per runner: a concluded event sets `runners.status='online'` and `busy=false` for the matched runner, which is wrong if the runner executes concurrent jobs (GitLab runners can be configured with `concurrent > 1`). Either scope the clear to "no other active `gitlab_jobs` row on that runner" or record the assumption explicitly.
- **[W-3]** Duplicate rows from overlapping project+group connections (acknowledged under "Outside #140") still feed this issue's busy tracking: two `gitlab_jobs` rows for one job, each with a `running` and a terminal event, will set and clear busy on the same runner twice, and one row's terminal event clears busy while the other row is still `running`. Deferring dedup to #148 leaves a known busy-state inconsistency inside #140's own scope; at minimum the handler should ignore a candidate set that yields more than one connection, or record the double-count on #136 with this consequence spelled out.
- **[W-4]** The delivery id `gitlab:<connection_id>:<key>` requires the retry arm to parse the connection_id back out; the format is underspecified if keys or ids can contain `:` — pin the parsing rule (e.g. exactly two segments after `gitlab:`) so retry cannot mis-resolve a connection.

### NIT findings
- **[NIT-1]** Migration line 39 makes `(base_url, scope_kind, scope_id)` unique, so two candidates with the same `project_id` on the same instance cannot exist; the "several candidates" cases reduce to different `base_url`s or project-vs-group — the tests already cover this, but the brief's "several candidates can exist" prose could name that constraint.
- **[NIT-2]** `webhook_secret_key` is unique per connection (line 41), so "more than one candidate matches" requires identical secret values under different keys; the project-scoped/oldest tie-break is unreachable in practice unless secrets are deliberately duplicated — fine as defensive spec, but tests for it will need hand-crafted data.

### Verdict
PASS — no BLOCK findings; implementation may proceed.