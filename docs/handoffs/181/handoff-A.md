# Handoff-A: Phase 3 - Fleet GitLab pools scale on pending jobs, not only to min_count (#181)  (up-front plan review, re-review of the amended brief)

**Date:** 2026-11-21
**Branch:** claude/gitlab-181-scaling-89ec07 (head a19e9e6; `git diff master -- crates src` is empty, so no code exists yet)
**Brief reviewed:** docs/handoffs/181-brief.md (amended in a19e9e6 after the earlier BLOCK in 233dac5)   **Reuse map:** there is no `docs/handoffs/181/survey.md`. The brief names the analogue itself: GitHub fleet demand through `attribute_queued_jobs` / `choose_platform` / `platform_jobs`, with ownership from `queued_job_owners`. Recommendation: EXTEND.   **Wireframe:** N/A (no UI)
**Verdict:** PASS

## Summary
PASS, with 0 block findings and 2 warns. The amended brief fixes the earlier block (A-B1) by naming the seam:
- the public, GitHub-shaped queue views and `QueuedJob` stay unchanged;
- one `pub(crate)` accessor makes a single `queued_job_owners` call;
- the job type stays internal to attribution;
- `platform_jobs` filters out GitLab jobs after attribution, which is #148's filter-after-ownership pattern.

It also takes up all five earlier warns: one ownership scan, the attribution internals, the test homes, the per-pass totals, and the corrected doc list. I checked the plan against the current code. It extends the right object in the right layer, and every caller of the views it touches is either unchanged or listed in Scope 3. The two warns are details F should settle when choosing the type and the ordering.

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | Scope 1, Seam: "`attribute_queued_jobs` uses a private job type: a struct whose repository is `Option<i64>` … or an enum", while the `autoscaling.rs` accessor "returns both platforms' jobs" | abstraction level; naming and file structure | **The job type has to cross a module boundary, and the struct option duplicates `QueuedJob`.**<br>- The accessor is in `autoscaling.rs` and its caller is in `fleet/targets.rs`. A type that is private to either module can't be the accessor's return type, so the type must be `pub(crate)` and defined beside the accessor. This has precedent: the `pub(crate)` structs in `fleet/store.rs:6, :72`.<br>- A new struct with `repository_id: Option<i64>`, `labels`, `queued_at` and `prioritised` would be a second copy of `QueuedJob` (`autoscaling.rs:31-39`). `platform_jobs` would then have to convert it back to `QueuedJob`. If F also adds a private wrapper type in `targets.rs`, that makes three job shapes. | - Define the type **once**, as `pub(crate)`, next to the accessor in `autoscaling.rs`. Don't re-export it in `lib.rs`. "Private" in the brief should be read as "not public API".<br>- Prefer an enum that wraps the existing type, for example `Github(QueuedJob)` / `Gitlab`. A GitLab entry needs no labels, repository, timestamp or priority, because `attribute_queued_jobs` doesn't filter it by label and `choose_platform` doesn't prioritise it.<br>- `platform_jobs` can then drop GitLab entries with a `github()`-style method, as `KeyedJobOwner::github` (`autoscaling.rs:300-312`) does for owners.<br>- `attribute_queued_jobs` should use this type directly, with no second mapping type in `targets.rs`. |
| 2 | warn | Seam: the accessor "keeps this pool's owners in owner order"; "`queued_job_assignments` calls the same helper, so its output is unchanged"; Scope 3: "every result stays byte-identical" for GitHub-only pools | pattern consistency (shared-helper refactor) | **The shared helper returns jobs in SQL order and can return fewer of them, so the accessor has to merge by id.**<br>- The helper SQL that moves out of `queued_job_assignments` (`autoscaling.rs:920-951`) orders by `wj.created_at, wj.id`. It also re-checks `wj.status='queued' AND wr.status!='completed'`, so an owned id whose job or run changed after the owners were read is silently left out.<br>- For a GitHub-only pool, owner order already equals that SQL order. `github_job_candidates` orders by `wj.created_at, wj.id` (`:616-618`), the merge in `queued_job_candidates` (`:556-571`) runs only when GitLab candidates exist, and `QueuedJobKey::Github` orders by id. Byte-identity holds as long as the accessor keeps owner positions.<br>- That breaks if the accessor appends the helper's GitHub rows and then the GitLab entries, or assumes one row per owned id. Order matters downstream: `attribute_queued_jobs` hands spare runners and room to jobs in input order, after a stable sort on single- versus multi-platform jobs (`fleet/targets.rs:1425-1462`). | - Build the result by walking the owners list. Put each GitHub row in its owner's position by job id, and skip owned ids the helper didn't return, as `queued_job_assignments` effectively does today.<br>- Don't concatenate the two platforms' jobs, and don't re-sort, as the brief already says.<br>- No change to the brief is needed. F should keep this in mind, and A will check it at Phase 7. |

**Checked and consistent (no finding):**
- **Earlier block A-B1 is resolved.** Scope 1's Seam bullet does three things:
  - it keeps `QueuedJob`, `queued_job_assignments`, `owned_queued_jobs`, `repository_capacities`, `provider_capacities`, `next_runner_*`, `queued_job_ownership` and `platform_jobs`' signature GitHub-only;
  - it records that #181 supersedes only A-B2's "fleet placement is untouched" clause. I checked this against `docs/handoffs/148-brief.md:72-77`;
  - it routes fleet demand through one new `pub(crate)` accessor.

  The public views' callers keep their inputs:
  - the webhook scale target (`gridops-api/src/webhooks.rs`);
  - the resources provider choice;
  - in the reconciler, `maybe_scale_up` (`provider_capacities`, `main.rs:1316`), `evict_surplus_runner` and `provision_on_host`.
- **Callers of `choose_platform` and `platform_jobs`.** The only callers are:
  - reconciler `fleet.rs:71, :97`;
  - `fleet/admission_tests.rs:3347`;
  - `gridops-api/tests/fleet_http/recovery.rs:836, :901, :913`.

  All are listed in Scope 3, and neither public signature changes.
- **Layer and dependency direction.** `fleet::targets` already depends on `autoscaling` (`crate::queued_job_assignments`, `targets.rs:1405`), and the new accessor keeps that direction. The fix stays in core, where placement decisions live. The reconciler's `provision` loop (`main.rs:1076-1078`) and its `NoDemand` guard (`main.rs:2442-2453`) stay as they are, so no GitLab special case is added to the reconciler.
- **Hot-path cost.** One `queued_job_owners` call per attribution, the same as today's `queued_job_assignments` → `owned_queued_jobs` path. No new query on `gitlab_jobs`.
- **Attribution internals match the code.**
  - In the `deficit` closure, a `None` runner is always "useful" (`targets.rs:1509-1524`), so a starting GitLab runner covers a GitLab job once that job adds to `queued[index]` only.
  - A GitLab entry never reaches `waiting` or `prioritised` (`:1491-1499`), so repository-keyed prioritisation is unchanged.
  - Scope 2's formula matches `busy + queued × factor − active`.
- **`queue_scale_factor` default.** It is 1 (`migrations/0003_autoscaling.sql:2`), so the regression's "exactly one runner for one job" holds for fixtures that don't set it. This settles the brief gate r1 W-1 from the architecture side; T owns the test wording.
- **Test homes exist and fit.**
  - `tests/gitlab_pool_fleet.rs` is a migrated-database integration test that already seeds a `scope='gitlab'` pool with connections. It can reach `choose_platform` and `platform_jobs`, which are both `pub`, with no new `pub` items.
  - The `Fleet` fixture (`gitlab_lifecycle_tests.rs:1710-1800`) composes SQL through `gitlab_setup`, so a min/max/autoscaling variant needs no second fixture.
  - The `pending` seeder `row(...)` is at `gitlab_poll_tests.rs:189-197`.
  - The `#[cfg(test)] #[path] mod` wiring is at `autoscaling.rs:1993-1995`.
- **Doc list.** `queued_job_owners` (`autoscaling.rs:336-340`) and the `autoscaling_gitlab_tests.rs:1-5` module doc are the comments that become false. `owned_queued_jobs` and `queued_job_ownership` stay accurate.
- **Coexistence.** No live worktree edits these files.
  - `cool-davinci-1830ec` (#143) has a clean tree.
  - `gridops-134` is a stale branch: its work landed in master as 80e6f1b (#144), and its tree is clean.

## Notes for O
None required for PASS. Pass warn 1 (one `pub(crate)` enum that wraps `QueuedJob`, defined beside the accessor) and warn 2 (place jobs in owner order by id, and skip ids the helper didn't return) to F as implementation guidance. A will check both at the Phase 7 anti-duplication gate.

Out of scope but noticed (unchanged from the earlier review): a pool linked to GitLab whose fleet profile is Docker without `requires_docker_socket` already fails in `gitlab_lifecycle::fleet_target` (`gitlab_lifecycle.rs:539-547`). After #181 a pending job can trigger that failure as well as `min_count` can. That is existing behaviour, not drift.

## Patterns referenced
- `crates/gridops-core/src/autoscaling.rs:228-348` covers `assigned_queued_jobs`, `owned_queued_jobs`, `QueuedJobKey`, `KeyedJobOwner::github` and `queued_job_owners`. `:545-714` covers the candidates and their ordering, and `:759-951` covers `assign_queued_jobs`, `provider_capacities`, `next_runner_provider` and `queued_job_assignments`.
- `crates/gridops-core/src/fleet/targets.rs:1302-1561` covers `platform_jobs`, `pool_runners`, `attribute_queued_jobs` and `choose_platform`. `crates/gridops-core/src/fleet/store.rs:6-81` is the precedent for `pub(crate)` types and functions.
- `docs/handoffs/148-brief.md:72-77` is the A-B2 GitHub-only public views and the filter-after-ownership pattern.
- In the reconciler: `fleet.rs:60-190` (`choose_profile`, `repository_for_platform`, `provision`), and in `main.rs`, `:1076-1110` (the pass loop), `:1282-1331` (`maybe_scale_up`) and `:2421-2470` (`provision` and `NoDemand`).
- Test homes: `crates/gridops-core/tests/gitlab_pool_fleet.rs`, `gridops-reconciler/src/gitlab_lifecycle_tests.rs:135-150, :1700-1800`, `gitlab_poll_tests.rs:180-196`, and `gridops-api/tests/fleet_http/recovery.rs:805-923`.
