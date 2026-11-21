## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** crates/gridops-reconciler/src/gitlab_lifecycle.rs:568 (fleet_material) — the claim that `placement.workload_id` is the runner row id suitable for the `runner_events.runner_id` foreign key. The evidence that would settle it: the `Placement`/workload struct definition and the fleet insert path in `crates/gridops-reconciler/src/main.rs` showing `workload_id` equals `runners.id`.
- **[NV-2]** workflow — the claim that the headless UI walkthrough ("done when" criterion) is delivered by a later pipeline phase (U/Phase 9) and therefore is not missing from this PR. Evidence: `workflow-coding-pipeline.md` showing the walkthrough runs after the diff gate within the same PR pipeline.
- **[NV-3]** crates/gridops-core/src/gitlab/http.rs:301 — the claim that the pre-existing failing `release_preflight` test fails identically at pre-branch commit `7164fda`. The handoff asserts a detached-worktree rerun; evidence would be CI run logs at `7164fda` (not shown in this prompt).
- **[NV-4]** crates/gridops-reconciler/src/gitlab_lifecycle.rs:912 (warn_pool) — the claim that SQLite `json_extract` is available in the reconciler's sqlx build (the brief cites prior use in gridops-core; not shown here). Evidence: the existing `json_extract` query in the gridops-core fleet registry code, or a passing test run (test run output is claimed but not shown as source).

### WARN findings
- **[W-1]** crates/gridops-reconciler/src/gitlab_lifecycle.rs:293 (`refusal_code`) — `NotFound` has a refusal code but can never reach this function (`deregister` and the sweep treat `NotFound` as success). Harmless dead branch, but it slightly suggests a path that does not exist; a comment or removal would prevent confusion.
- **[W-2]** crates/gridops-reconciler/src/main.rs:677-681 (known issue, unchanged) — `reconcile`'s "Pool reconciliation failed" event still embeds `GitLabError`'s full `Display`, including `Upstream` detail, in a client-visible event row, while this PR's new events deliberately exclude it. The handoff defers it as a follow-up; confirm that follow-up issue is actually filed, or the inconsistency ships.
- **[W-3]** crates/gridops-reconciler/src/gitlab_lifecycle.rs:912 — the dedup read-back query filters on `json_extract(metadata, ...)` with no supporting index; it scans the pool's events each refused sweep delete. Bounded by pool event volume, but if pools accumulate long event histories this is a per-runner scan per sweep. Consider documenting the cost or capping.

### NIT findings
- **[NIT-1]** crates/gridops-reconciler/src/gitlab_lifecycle_tests.rs (`TWO_HOURS`, named in the handoff) — constant named `TWO_HOURS` with comment explaining the double backdate; fine as is; the earlier flagged `HOUR_AND_MORE` name was resolved.
- **[NIT-2]** src/features/platform-connections/gitlab.tsx:342 — "GridOps checks both before saving" now reads ambiguously: "both" originally meant scopes+role, but three scopes are now listed. Consider "GridOps checks these before saving".

### Verdict
PASS — no BLOCK findings; testing may proceed.