## Brief Review (Round 1)

### BLOCK findings
**[B-1]** Item 4's acceptance test is stated as "the speed check on the synthetic week stays well under budget" — this is not a testable criterion. No numeric threshold is given anywhere in the brief ("well under" is unfalsifiable), and the "Done when" item "The speed check passes" likewise names no threshold. The only number in the brief (p95 < 300 ms cold) is explicitly scoped to #204, not this issue. Fix: state an explicit budget (e.g. "live path under X ms p95 on the synthetic week" and what the synthetic week is), so the test can be written and pass/fail adjudicated.

### Needs-verification findings
**[NV-1]** Claim that `host_live_reads` → `counts()` at `crates/gridops-core/src/fleet/registry/queries.rs` ~:1243 filters only on `host_id` — the excerpt of that function would settle it.
**[NV-2]** Claim that a `workload_placements_host_idx(host_id, state)` index exists and is usable for the live variant — the migration/index definition in the repo would settle it.
**[NV-3]** Claim that retention never deletes placements — the retention job's source would settle it.
**[NV-4]** Claim that `health.rs` ~:53-56 and ~:147-154 hold a single mutex across all windows' computations — the shown source of those lines would settle it.
**[NV-5]** Claim that `health_live.rs` ~:174-178 uses `host_resource_domains.disk_bytes` while `resources.rs` ~:5918-5950 uses observed root capacity with a heartbeat condition — both excerpts would settle the claimed divergence.
**[NV-6]** Claim that `set_gitlab_runner_busy` (`gitlab/jobs.rs` ~:363) never sets `current_job_id`, so `health_live.rs` ~:82-93 never counts busy runners — both excerpts would settle it.
**[NV-7]** Claim that the request body limit is 25 MB and that no other limit exists on rule count/pattern length — the API's body-limit configuration and current `parse_rules` would settle it.
**[NV-8]** Claim that "every other settings save writes an audit event" — the other settings handlers' source would settle it.
**[NV-9]** Claim that SQLite hangs under a #179-style lock storm (motivating the 2 s timeout) — the #179 issue/recorded behavior would settle it.
**[NV-10]** Claim that the preview UI shows one generic message for blank-row, 5xx, and network errors, and has no abort — `web/src/features/settings/job-categories-section.tsx` would settle it.

### WARN findings
**[W-1]** Item 3 offers two alternative fixes ("once per request cycle (or cache it for 30 s)") without choosing one or giving a criterion to decide — pick one so the "test that fails before the fix" is determinate.
**[W-2]** Item 2's stale-vs-503 policy is unspecified: it says "serving stale or 503 meanwhile" without stating which, for how long stale is acceptable, or whether stale is served at all when there is no prior value. Define the degradation policy so the failure-mode tests are unambiguous.
**[W-3]** Item 1's per-window lock fix plus item 3's shared live computation interact: once live stats are shared across windows, the lock-per-window refactor must avoid reintroducing a shared lock on the live part. Note the intended locking topology (e.g. per-window cache entry + separately locked/shared live cache) in the brief.

### NIT findings
**[NIT-1]** The OpenAPI criterion ("updated for any new 422 details") should also cover whether the 503/stale error response from item 2 gets a documented status and schema.

### Verdict
BLOCK — 1 blocking finding (B-1, untestable speed criterion); resolve the threshold before implementation starts.