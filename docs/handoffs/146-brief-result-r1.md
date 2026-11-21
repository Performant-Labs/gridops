## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** GitLab role requirements: "GitLab's `POST /user/runners` requires Owner for `group_type` but only Maintainer for `project_type`" — evidence: GitLab REST API docs page for `POST /user/runners` (the permission-requirements section), excerpted.
- **[NV-2]** GitLab response shapes assumed throughout: `POST /user/runners` returns `{ id, token }` exactly once; `/user/runners` 403 body contains `"error": "insufficient_scope"` and a `"scope"` field; project JSON has `path_with_namespace`, group JSON has `full_path`; job JSON has `pipeline.id` and no top-level `project_id`; `GET /personal_access_tokens/self` returns a `scopes` array — evidence: the corresponding GitLab REST docs pages with response examples.
- **[NV-3]** `GET /projects/:id/members/all/:user_id` / `GET /groups/:id/members/all/:user_id` exist, return `access_level`, and 404 for non-members (including admins who aren't members) — evidence: GitLab REST docs for "members all" endpoints; if an admin bypasses membership and the endpoint 404s for an admin user, verify_access step 2 would report MissingRole despite the "admin passes" rule, so this interaction needs the docs' statement of behavior.
- **[NV-4]** `reqwest` behavior claims: `Policy::none()` prevents any redirect (no token-bearing re-request is sent), `HeaderValue::set_sensitive` suppresses the value in reqwest/tracing debug output — evidence: reqwest source/docs for `redirect::Policy::none` and `HeaderValue::set_sensitive`.
- **[NV-5]** The referenced contract and handoff exist and match what is described: `crates/gridops-core/src/gitlab/mod.rs` on master (trait shape, `GitLabError`, `Scope`, `CreatedRunner`, `SecretString`), the "Contract deviations shipped by #137" table on #136, and `docs/handoffs/137-brief.md` B-6 — evidence: those files/comments excerpted.
- **[NV-6]** The 429 `Retry-After` rule (HTTP-date → `None`, `-1` → `None`, trimmed decimal → `Some`) is a project-local rule from 137-brief B-6; if it is instead meant to track RFC 9110 (HTTP-date SHOULD be honored), it diverges from the standard — evidence: 137-brief B-6 excerpt and the RFC 9110 §10.2.3 text, to confirm the deviation is intentional.

### WARN findings
- **[W-1]** "A request timeout is set" is unspecified (no duration) and not covered by any test in the coverage list — pick a concrete value and drop or test the criterion; otherwise it is untestable as written.
- **[W-2]** Hand-written HTTP/1.1 responses on a bare `tokio::TcpListener` (chunked encoding, keep-alive, HEAD/Content-Length edge cases) is a meaningful hidden scope for a mock server — recommend explicitly bounding it (connection-per-request, fixed Content-Length, no chunked) in the brief so tests don't grow an HTTP/1.1 parser.
- **[W-3]** The coverage list enumerates method-by-method cases but omits `group_projects` (D4) explicitly — "every trait method's success path" arguably covers it, but an explicit bullet (including its pagination and subgroup filter `include_subgroups=true`) would prevent it slipping through.
- **[W-4]** Cross-origin `Link` refusal: coverage lists the case, but there is no case for a same-origin `Link` next URL that *rewrites the path* to something unexpected (e.g. drops the API prefix). Recommend deciding whether the check is scheme/host/port only (as specified — path ignored) and stating it, since the spec as written permits following a next URL with any path on the same host.
- **[W-5]** The hard page cap ("e.g. 1000 pages") leaves the exact number and whether it's configurable unresolved — fix a concrete constant so the test can assert the cap triggers an `Upstream` error without issuing 1000 requests (e.g. cap configurable via test-only constructor or small default).

### NIT findings
- **[NIT-1]** "Report this as a deviation from the issue text" (project-scope Maintainer vs issue's "below Owner") — specify *where* it is reported (a comment in #146? the epic's deviations table?) so the deliverable is checkable.
- **[NIT-2]** `pending_jobs` should also state that pages are aggregated across concurrent mutations (a job may move from pending between pages); current spec silently accepts duplicates/gaps — a one-line stance would help the test expectations.

### Verdict
PASS — no BLOCK findings; implementation may proceed.