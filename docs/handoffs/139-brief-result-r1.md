## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** Round-4 amendment claim: "The fleet path is unaffected (`fleet::tick` runs before `load_pools`)" — the ordering of `fleet::tick` relative to `load_pools` inside `reconcile()` is not shown in any excerpt, and the whole argument that the fleet path needs no mid-pass pool reload rests on it — show `reconcile()` in `main.rs` (the call site of `fleet::tick` and `load_pools`) with line numbers.

**[NV-2]** Round-4 W3 claim: "the GitLab deregister treats `GitLabError::NotFound` as done" presupposes `GitLabError` has a `NotFound` variant — no excerpt of `gridops-core/src/gitlab/mod.rs`'s `GitLabError` enum is shown — show the `GitLabError` enum definition (file:line) confirming the variant; if it does not exist, D17-adjacent contract change may be required, which the "Don't touch" section says must stop and be reported.

**[NV-3]** Round-2 excerpt says `main.rs:2811` is `if runner.ci_platform == "bitbucket" {` and the GitHub `else` is at 2818, while round-4 R4-NV-6 repeats this; the excerpt set for `main.rs:2788-2830` was promised in "Source material read" but is not included in SOURCE EXCERPTS — the dispatch-dispatch claim (Done-when item 8) is checked against nothing shown here — show `main.rs:2788-2820`.

**[NV-4]** Claim that `start_tests.rs` contains `a_successful_probe_clears_the_blocked_state_in_the_same_pass` with the asserted preconditions (`provision_failure_count=3`, circuit open, one pass → cleared), on which R4-W-3's "existing test passes unmodified" done-when item rests — show `start_tests.rs:560-600` (promised in "Source material read" but not excerpted).

**[NV-5]** D17 endpoint claim beyond what R4-NV-5 covered: pagination semantics of `GET /projects/:id/runners` / `GET /groups/:id/runners` were "not checked against the docs here" and are assigned to #146 — confirm #146's brief actually owns pagination, otherwise the sweep silently sees only the first page of runners — a citation of #146's brief/handoff assigning the paginated client.

### WARN findings
**[W-1]** Orphan-sweep blind spot for deleted connections: R4-NV-3 records that deleting a `gitlab_connections` row NULLs `runners.gitlab_connection_id` and cascades the membership rows, so the sweep (which iterates `gitlab_connections` rows) can never list or delete GitLab runners belonging to a deleted connection — those leak permanently in GitLab with no coverage path, and the brief records the fact but sets no acceptance criterion or documented limitation — add either a done-when note/PR-description record acknowledging the leak (and that connection deletion is expected to be rare/manual) or a rule that connection deletion is blocked while runners exist.

**[W-2]** The placeholder client's production wiring (a real `PlaceholderGitLabApi` constructed in the production `Reconciler` constructor) has no direct acceptance test — Done-when item 16 tests the *behaviour* via a failing fake, so a regression that wires the fake (or nothing) in production would pass the suite — add a small unit test that the production constructor's `gitlab` field is the placeholder type, or a compile-time assertion.

**[W-3]** Done-when item 4 is conditional on unobservable-at-review-time master state ("if #147's `GITLAB_RUNNER_IMAGE` is on master at implementation"); the PR-description record mitigates this, but a race where #147 lands mid-implementation could leave both forms stale — recommend the implementer pin the observed master commit at branch start and re-check once before merge, as the brief already requires; make that re-check explicit in item 4.

### NIT findings
**[NIT-1]** The brief cites "08c32b3 and #130" and `docs/handoffs/137-brief.md` as rationale without excerpting them; not load-bearing for any done-when item, but a future reader cannot check the wedge precedent from this document alone.

**[NIT-2]** Minor internal inconsistency in excerpt citations: "The seam" says the dispatch `if` is at "~`main.rs:2811`" while round-2 originally wrote 2810; corrected in round 4, but the seam section itself was not updated — harmless.

### Verdict
PASS — no BLOCK findings; implementation may proceed.