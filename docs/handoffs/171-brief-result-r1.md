## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** GitLab 17.1+ requires `manage_runner` scope for `DELETE /runners/:id` — the GitLab API documentation for the runners delete endpoint (a docs.gitlab.com excerpt of the runner delete API's required scopes) would settle it.
**[NV-2]** `verify_access` currently checks only `create_runner` and `read_api` (or `api`) — an excerpt of `crates/gridops-core/src/gitlab/http.rs` around line 299.
**[NV-3]** `MissingScope` maps to "The GitLab access token is missing the {scope} scope." and create/token-replacement/Verify all call `verify_access` — excerpts of `crates/gridops-api/src/resources/gitlab.rs` lines ~139, ~254, ~317, ~799.
**[NV-4]** `runner_events` already has `level`, `event`, `message`, `metadata`, `pool_id`, `runner_id` and `getRunnerPoolEvents` surfaces warnings — the schema definition/migration for `runner_events` and the pool events query.
**[NV-5]** `verify_access` accepts Maintainer+ on a project and requires Owner on a group — the token introspection / role-check code in `http.rs`.
**[NV-6]** The deletion sites listed in Need 2 exist and funnel through `deregister_runner`-style paths with only `tracing::warn!` — excerpts of `gitlab_lifecycle.rs` ~233, ~516 and `main.rs` ~2890, ~705.

### WARN findings
**[W-1]** The dedup rule is specified as a choice ("at most one row per pool per connection per reason within an hour (or only when the reason changes) — pick one"), leaving the acceptance criterion "writes no more than the dedup rule allows" untestable until the choice is made. Recommendation: pick one rule now (reason-change-only is simpler and needs no time source in tests) and state it explicitly.
**[W-2]** The runtime-warning event name is given as "e.g. 'GitLab runner not deleted'" — "e.g." makes the stable event name ambiguous while the Done-when tests depend on it. Fix the exact event string.
**[W-3]** Edge cases around dedup and the reason-change option are thin: if "only when the reason changes" is chosen, the reconciler needs persistent state of the last emitted reason (event row? in-memory?), which is unspecified. Recommendation: specify that the last warning row itself is the dedup state, or accept the one-hour window.
**[W-4]** The orphan sweep warning is recorded on each pool linked to the connection; a connection with many pools could itself become noisy. Consider whether the dedup window applies per pool (as stated) or per connection overall.

### NIT findings
**[NIT-1]** Scope-check order is specified as `create_runner`, `manage_runner`, `read_api`, but the Done-when test only requires `MissingScope("manage_runner")` for the create_runner+read_api case; the other two orderings are covered by "existing cases still name the right scope" — fine, but worth an explicit assertion that the order is deterministic.
**[NIT-2]** "Give the connection a token with manage_runner (or api); the orphan sweep retries it" — the claim that the orphan sweep retries deletion is asserted but not covered by a test in Done-when (only that the sweep writes a warning). Consider adding a test or softening the message.

### Verdict
PASS — no BLOCK findings; implementation may proceed.