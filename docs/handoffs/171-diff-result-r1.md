## Implementation Review (Round 1)

### BLOCK findings

**[B-1]** crates/gridops-reconciler/src/gitlab_lifecycle.rs (fn `refused_deletion_message`, the `format!("GitLab could not delete runner {gitlab_runner_id} of connection {connection_name}: {error}. {remedy}")`) — the message interpolates `GitLabError`'s full `Display`, which for `Upstream` includes the arbitrary inner detail string — The handoff asserts "the reason text … carries only a fixed description, a scope name or a status, never a response body or a token" (Evidence Appendix, F), but the shown excerpt of `GitLabError` (`crates/gridops-core/src/gitlab/mod.rs:236-237`, `#[error("GitLab request failed: {0}")] Upstream(String)`) carries a free-form string, and the API's own code (`crates/gridops-core`-adjacent `crates/gridops-api/src/resources/gitlab.rs:795-811`) deliberately strips that detail from client-facing messages with the comment "The upstream detail of a failed request stays in the log; no message carries the token." The reconciler now puts that same detail into a `runner_events.message` row that the pool Events UI renders. The brief's constraint is "No token in any error, log line, event, audit row"; an upstream body echoed into an event can carry exactly what the API keeps out of client messages — the shown excerpts contradict the safety claim this design rests on. The diff's own test proves it: `assert_refusal(&refused[0], ACME.name, 1, "GitLab request failed")` passes because the full `{0}` detail is in the message — remediation: map the error to a client-safe reason in the event message the way `gitlab_message` does (e.g. plain "GitLab request failed" for `Upstream`, no `{error}` interpolation), keeping the full detail in `tracing::warn!` only.

**[B-2]** Brief "Done when" (headless UI walkthrough bullet) — the acceptance criterion "The headless UI walkthrough opens the GitLab connection form and confirms the help text, and opens a GitLab pool's events list showing a deletion warning (seeded)" is not delivered and not marked out of scope in the brief — Known issues says it is "left to later phases" with no authorization in the brief's Out-of-scope list (which names only #172 and #173). A stated Done-when item deferred unilaterally is an unmet acceptance criterion — remediation: run and document the walkthrough (with the seeded deletion warning) in this PR, or get the criterion's deferral recorded in the issue before merge.

### Needs-verification findings

**[NV-1]** crates/gridops-api/src/resources/gitlab_tests.rs (`release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`) — the claim that this 1-in-838 failure is pre-existing and host-dependent (baseline: reverting `http.rs` still fails) — the exact evidence that would settle it: a CI run of `cargo test -p gridops-api` on a non-macOS host (or a run at HEAD without the branch) showing the same failure. The Done-when requires `cargo test -p gridops-api` to pass; this must be confirmed environmental, not regressed.

**[NV-2]** crates/gridops-core/src/gitlab/fake.rs — the claim "fake.rs is unchanged" because the fake does not model token scopes (it only stubs per-method failures, per the tests' `fail_next` usage) — evidence: an excerpt of `fake.rs` showing no scope-set logic in `VerifyAccess`, settling that fake and HTTP client behaviour stay identical as the brief requires.

**[NV-3]** gitlab_tests.rs create-refusal path — the claim that the table-driven create test also asserts nothing is stored on the refused create (Done-when: "nothing is saved/changed on the refused create") — evidence: the remainder of `gitlab_refusals_on_create_are_400s_that_name_the_reason_and_store_nothing` showing the stored-nothing assertion covers the new seventh case.

**[NV-4]** docs/, README.md — the claim that all other GitLab scope lists were swept to match — evidence: grep output of `docs/` and `README.md` for `create_runner`/`manage_runner`/`read_api` showing no stale two-scope list.

### WARN findings

**[W-1]** crates/gridops-reconciler/src/gitlab_lifecycle.rs (`warn_linked_pools` loop) — `record_refused_deletion(...).await?` uses `?` inside the per-pool loop, so an event-insert failure on the first pool aborts the warnings for all remaining pools of that connection (logged by `sweep_connection`, so the sweep survives, but pools later in `ORDER BY pool_id` silently get no warning) — recommendation: log the per-pool failure and continue the loop, so one bad pool cannot suppress the others' warnings.

**[W-2]** crates/gridops-reconciler/src/gitlab_lifecycle.rs (`refusal_code`) — `MissingRole` (and `RateLimited`) refusals from the sweep produce reason codes (`missing_role`, `rate_limited`) that dedup per hour but whose messages get the generic remedy; a `RateLimited` sweep storm will write one warning per pool per hour indefinitely — acceptable, but consider whether rate-limit noise warrants suppression; at minimum confirm the intent.

### NIT findings

**[NIT-1]** crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs (`the_sweeps_refused_deletions_warn_each_linked_pool_once_an_hour_per_reason`) — `HOUR_AND_MORE` is `2 * 3_600_000` but named "hour and more"; the second backdate subtracts from already-backdated rows, so the arithmetic works but the name/comments are easy to misread.

**[NIT-2]** src/features/platform-connections/gitlab.tsx:342 — the help sentence is 41 words in one breath; the brief asked for "one short paragraph" — consider splitting after "(or api)".

### Verdict
BLOCK — 2 blocking finding(s); must resolve before testing starts.