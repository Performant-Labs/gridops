## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** http.rs (`JobResponse`) — the claim that GitLab's job JSON always contains a non-null `pipeline.id`, so the required (non-`#[serde(default)]`) `pipeline: IdOnly` field never spuriously fails decode — the evidence that would settle it is GitLab's `GET /projects/:id/jobs` REST documentation (or a captured response) stating whether `pipeline` can be absent/null for jobs in the queried states.

**[NV-2]** http.rs (`transport_error`) — the claim that `reqwest::Error`'s `Display` includes the URL, justifying discarding the message entirely — a `reqwest` source excerpt (or docs page on `reqwest::Error` Display) would settle it.

**[NV-3]** http.rs (`resolve_scope`, group) — the claim that GitLab v4 still honours deprecated `with_projects=false` on `GET /groups/:id` — GitLab's `GET /groups/:id` REST documentation would settle it.

### WARN findings
**[W-1]** http.rs (`MAX_PAGES = 100`) — the brief specifies a hard page cap "e.g. 1000 pages"; the implementation uses 100. "e.g." makes this advisory, but the handoff presents it as "matching GitHub" without flagging it as a deviation from the brief's number — it should be listed under "Deviations" and confirmed with A, since 100 pages × 100/page = 10,000 items may be a real ceiling for large groups. Recommendation: confirm the intended cap with A, or raise it, and record the decision.

**[W-2]** crates/gridops-core/tests/gitlab_live.rs:1 — the brief's ownership rule says the only edit allowed outside `http.rs` is the two `mod.rs` lines, and mock tests must live "in `http.rs` … or a sibling test module file under `src/gitlab/`". This new integration test file is outside both. It does not conflict with concurrent wave-1 sessions (a brand-new file), but it is a strict spec-drift the handoff does not list as a deviation. Recommendation: get A's sign-off, or move the ignored test into `http_tests.rs`.

**[W-3]** Gate `cargo clippy --all-targets -- -D warnings` fails (autoscaling.rs:1615, 1701, confirmed by the shown excerpts). The brief's "Done when" requires all gates to pass. The failures are pre-existing and outside #146's ownership, so they are not introduced by this diff, but the issue cannot be declared done while a gate is red. Recommendation: escalate to O/A for a ruling before merge.

**[W-4]** http.rs (`verify_access` step 3) — a 403 from the membership endpoint is silently mapped to `MissingRole` via the generic 403 mapping (unless the body says `insufficient_scope`). That is probably the desired outcome, but the brief only specifies 404 and low `access_level` → `MissingRole`; a 403 could also mean the token lacks `read_api`-adjacent rights and would be misreported as a role problem. Recommendation: note the behaviour in the handoff and confirm it is acceptable.

### NIT findings
**[NIT-1]** http_tests.rs (`LAST_PAGE = 1001` comment) — the comment says "one past the brief's largest cap (1000)", but the implementation's cap is 100 (W-1); the test's 2..=1000 page-count assertion passes with either cap, so the comment describes a cap this code does not have.

**[NIT-2]** http_tests.rs (`error_statuses_map_to_gitlab_errors`) — the 400 case checks only that the message contains "400"; a fixed-format assertion (`"GitLab returned 400 Bad Request"`) would pin the message the contract's `Upstream` Display produces.

### Verdict
PASS — no BLOCK findings; testing may proceed.