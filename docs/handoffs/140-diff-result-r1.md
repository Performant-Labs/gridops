## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** crates/gridops-api/src/webhooks/gitlab.rs (`receive_gitlab`, 401 path) — the claim that the 401 body satisfies the OpenAPI `ApiErrorBody` shape rests on `Self::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string())` (error.rs:35), but the `Unauthorized` variant's `#[error(...)]` message text is not shown in this prompt, so the body's `error` field content cannot be confirmed — a showing of the variant declaration in `crates/gridops-api/src/error.rs` (lines above the excerpt, ~:10-22) would settle it.

**[NV-2]** crates/gridops-api/src/webhooks/gitlab.rs (`delivery_key`) — claim: header order `Idempotency-Key` → `webhook-id` → `X-Gitlab-Event-UUID` → body hash matches GitLab's docs (that `webhook-id` is "preferred" and `Idempotency-Key` constant across retries). The prompt's NV-2 excerpt of the GitLab docs supports the semantics; nothing further to verify beyond confirming the docs page still says this.

**[NV-3]** crates/gridops-api/src/webhooks/gitlab.rs (`matching_connection`) — claim: `subtle::ConstantTimeEq` on byte slices of different lengths returns `Choice(0)` (no panic). The `subtle` crate source (impl of `ConstantTimeEq for [u8]`) would settle it; the excerpt set here does not include it.

**[NV-4]** crates/gridops-api/src/webhooks/gitlab.rs — claims that `required_header` (webhooks.rs) returns a 400 `BadRequest` for a missing header, and that `ExternalId::new` rejects ids that do not fit SQLite (`external_id` relies on `.ok()` treating those as absent). Neither helper's source is excerpted; `crates/gridops-api/src/webhooks.rs` (`required_header`) and `crates/gridops-core/src/fleet/…` (`ExternalId::new`) would settle both.

**[NV-5]** crates/gridops-core/src/gitlab/jobs.rs — the handoff's claim that this file is byte-identical to master's blob `c51247d` (from #160) cannot be checked from this prompt; a `git cat-file` / blob comparison on master would settle it. The rules in the file match the brief's D18 text as shown, so no contradiction is demonstrable here.

**[NV-6]** crates/gridops-api/src/webhooks/gitlab_tests.rs — tests rely on `FakeGitLab::with_job(base_url, Job)` existing with that signature and on `AppState::with_gitlab` being on master from #138; `fake.rs` excerpts shown cover `fail_next`/`calls` but not `with_job`, and `state.rs` excerpts do not show the builder. The `fake.rs` and `state.rs` source would settle it.

### WARN findings
**[W-1]** crates/gridops-api/src/webhooks/gitlab.rs (`process_with_connection`) — the connection's access token is fetched (and a missing/undecryptable token produces a 500 `Internal`, failing the delivery) **before** `process_job_hook` decides whether the payload is a `build` event at all. A non-`build` delivery (Pipeline Hook) or a `build` with an `unknown` status on a connection whose access-token row is missing would end `failed` instead of `processed`-with-no-effect, contrary to the spirit of "Non-`build` events are recorded and marked processed with no effect". Recommendation: move the credentials construction inside/lazily, or at least after the `object_kind` check (the narrow processing function already takes `credentials`; a cheap fix is to check `object_kind` in the caller or make credentials optional for no-effect paths).

**[W-2]** crates/gridops-api/src/resources.rs:94, resources/gitlab.rs:714 — the `pub(crate)` visibility change and re-export are outside the brief's shared-file list ("Do not change shared files beyond…"), a disclosed deviation justified by round-5 W2. It is visibility-only and low risk, but it is spec drift the brief never amended. Recommendation: record it on #136/with the orchestrator so the deviation is owned, as was done for the other route-side-effect exceptions.

**[W-3]** crates/gridops-api/src/webhooks/gitlab.rs (`receive_gitlab`) — a missing `X-Gitlab-Token` is answered 401, but the OpenAPI declares the header `required: true` and its 400 description is "The payload is not a JSON object"; the spec has no case naming a missing required header. The choice is defensible (matches "no match is a 401") and tested, but it is an interpretation the spec does not pin. Recommendation: none needed beyond awareness; if the spec ever gains a 400-for-missing-header case, revisit.

### NIT findings
**[NIT-1]** crates/gridops-api/src/webhooks/gitlab.rs (`reprocess`) — the `UNMATCHED` refusal inside `reprocess` is dead code on the reachable path (retry's existing `signature_valid` check at webhooks.rs:197-201 already refuses `gitlab:unmatched:` rows, per the Evidence Appendix and the test). Harmless defense-in-depth; a one-line comment already notes it.

**[NIT-2]** crates/gridops-api/src/webhooks/gitlab.rs (`record_delivery` call) — `action` is `string(object, "build_status")`, so non-`build` deliveries record a NULL `action` even when a comparable action exists (e.g. a pipeline hook's `object_attributes.status`). Cosmetic; consistent with "no effect" handling.

### Verdict
PASS — no BLOCK findings; testing may proceed.