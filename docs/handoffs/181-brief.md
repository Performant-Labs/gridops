# [GitLab: a fleet GitLab pool never scales up for pending jobs, only to min_count](https://github.com/Performant-Labs/gridops/issues/181)

Part of epic #136. Found by the GitLab end-to-end check on production (master `42e8474`, build 443) on 2026-11-21. This is a correctness fix in the reconciler's fleet scaling path. It has no UI.

## What happened on prod

The test pool `gl-e2e-prod` had min 0 and max 2 and was pinned to the fleet machine callisto, which has a ready Docker backend and a grant for the GitLab project target. A pipeline with 3 pending jobs started at 5:39 PM PST. The pool event said "3 queued jobs raised desired capacity from 0 to 2", but for 5 minutes no runner was placed, the fleet queue stayed empty and the reconciler logged nothing for the pool. At 5:44 PM PST, `min_count` was raised to 1, and a runner was placed at once and took a job.

## Root cause (traced)

- `provision` in `crates/gridops-reconciler/src/main.rs` (about line 2421) calls `fleet::choose_profile`, which calls `gridops_core::fleet::targets::choose_platform` (`crates/gridops-core/src/fleet/targets.rs`, about line 1466). That returns `(profile, behind)`.
- When `!behind && placement == Placement::Machines` and the pool already has `active >= min_count` runners, `provision` returns `ProvisionAttempt::NoDemand` (main.rs about lines 2440-2453). With min 0 and no runners, `0 >= 0` is true, so nothing is ever placed.
- `behind` comes from `attribute_queued_jobs` (targets.rs about line 1373). It lists the pool's queued jobs with `crate::queued_job_assignments` (`crates/gridops-core/src/autoscaling.rs` about line 920). That function is built on `owned_queued_jobs` (about line 245), which keeps GitHub jobs only, through `KeyedJobOwner::github`. Its doc says so: "GitLab jobs it owns are left out: every view built from this is keyed by GitHub job and repository ids."
- GitLab pending jobs are owned by the same rule (`queued_job_owners` together with `gitlab_job_candidates`, about line 637) and do raise `desired_count` through `assigned_queued_jobs`. They never reach `choose_platform`, so a fleet GitLab pool is never "behind" and never places a runner above `min_count`.
- The built-in host path doesn't go through this check. That is why #143's e2e run on the built-in host didn't catch it.

## Bitbucket (checked: not the same gap)

GridOps has no queued-job signal for Bitbucket at all: there is no Bitbucket jobs table, no Bitbucket candidates in `queued_job_owners`, and `maybe_scale_down` keeps Bitbucket runners because Bitbucket publishes no per-step busy signal. Bitbucket pools never scale on queue demand on any placement. They also can't run on fleet machines: `targets::ensure_target` refuses Bitbucket targets ("GitHub and GitLab targets are stored; Bitbucket targets are refused"), and the reconciler runs Bitbucket runners only on the macOS built-in host. So fleet demand is missing only for GitLab. Do not change Bitbucket. The PR must state this finding.

## Scope

1. **GitLab pending jobs count as fleet demand.** `choose_platform` must count the pending GitLab jobs this pool owns, attributed to the pool's platforms, and report the platform as behind while the pool's runners don't cover them, scaled by `queue_scale_factor`, as for GitHub. Ownership comes from `queued_job_owners` (the same ownership `assigned_queued_jobs` counts), with no second label-matching rule.
   - **Seam (A-B1, binding).**
     - Unchanged and GitHub-only: `QueuedJob` (including `repository_id: i64`), `queued_job_assignments`, `owned_queued_jobs`, `repository_capacities`, `provider_capacities`, `next_runner_provider`, `next_runner_repository`, `queued_job_ownership`, and `platform_jobs`' signature (`-> Vec<QueuedJob>`). Their callers see no change: in gridops-api, the webhook scale target (`webhooks.rs`) and the resources provider choice; in the reconciler, the built-in `maybe_scale_up`, `evict_surplus_runner` and `provision_on_host`. This supersedes only one clause of #148's A-B2 (`docs/handoffs/148-brief.md:72-77`), "fleet placement is untouched". The rest of A-B2 still holds.
     - Add one `pub(crate)` accessor in `autoscaling.rs`, for example `fleet_queued_jobs(database, pool_id)`. It calls `queued_job_owners` **once**, keeps this pool's owners in owner order (already sorted by `queued_at`, then `QueuedJobKey`) and returns both platforms' jobs. GitHub jobs get their details from a private helper that holds the SQL and row mapping `queued_job_assignments` uses today and takes the owned ids. `queued_job_assignments` calls the same helper, so its output is unchanged. GitLab entries carry no repository and are not prioritised. Do not query `gitlab_jobs` again and do not re-sort by a separately read timestamp.
     - `attribute_queued_jobs` uses a private job type: a struct whose repository is `Option<i64>` (`None` only for GitLab), or an enum. Inside it, a GitLab job counts on every profile (it is not filtered by `platform_supports_labels`, so a tag such as `linux` doesn't narrow it). Only a spare runner with no repository (`None`) covers it, never one registered to a GitHub repository. It adds nothing to `waiting` or `prioritised` in `choose_platform`; it adds to `queued[index]` only. For a GitHub-only pool every result stays byte-identical.
     - State in `attribute_queued_jobs`' doc comment that a `None` spare runner means either an organization GitHub runner or a GitLab runner, and why that is safe: a pool linked to GitLab places only GitLab fleet runners (`fleet::provision` checks for a GitLab connection first, `crates/gridops-reconciler/src/fleet.rs` about lines 163-170). Do not add `ci_platform` to `pool_runners`.
     - `platform_jobs` drops GitLab entries after attribution and returns GitHub jobs only, so `repository_for_platform` can never see one.
   - Prioritisation stays a GitHub repository concept. GitLab jobs are never prioritised; say so in the accessor's doc comment.
   - Update these doc comments so they describe the new behaviour: `queued_job_owners` ("Only [`assigned_queued_jobs`] counts them; every other view lists GitHub jobs only", `autoscaling.rs` about lines 336-340), `attribute_queued_jobs`, `choose_platform`, `platform_jobs` (say it returns GitHub jobs only), the new accessor, and the module doc of `autoscaling_gitlab_tests.rs` (lines 1-5, "Only `assigned_queued_jobs` (room and scale) sees GitLab jobs"; change the doc line only, not an assertion). `owned_queued_jobs` and `queued_job_assignments` stay accurate; at most add a pointer to the new accessor. No change to `docs/gitlab-runners.md`, which already describes the intended behaviour.
2. **Scale from `min_count` up to `max_count`.** With the fix, `provision`'s fleet branch places runners while the platform is behind, up to the desired count that `maybe_scale_up` already sets (capped at `max_count`). One reconcile pass calls `provision` `desired - active` times (`main.rs` about lines 1076-1078), and each call runs `choose_profile` again against the `starting` runners the earlier calls inserted, so one pass can place up to min(pending jobs × factor, `max_count`) runners. Don't change that loop or the `NoDemand` guard. The guard is correct; its input was wrong.
3. **GitHub fleet scaling and the built-in host unchanged.** For a pool with only GitHub jobs, every existing ownership, attribution, `choose_platform`, `platform_jobs` and `repository_for_platform` result must be identical. Built-in host scaling, eviction and provider choice do not change for pools linked to GitLab. All existing tests keep passing without changes to their assertions, in particular: `gridops-core` (`fleet/admission_tests.rs`, `autoscaling` tests, `autoscaling_gitlab_tests.rs`, `tests/gitlab_pool_fleet.rs`), `gridops-reconciler` (`start_tests.rs`, `gitlab_lifecycle_tests.rs`, `gitlab_poll_tests.rs` and the rest) and `gridops-api` (`tests/fleet_http/recovery.rs`, `tests/fleet_http/placement.rs`).

## Tests (required)

- **The #181 regression (reconciler):** a GitLab pool with autoscaling on, `min_count` 0 and `max_count` 2 or more, linked to a GitLab connection, on a fleet platform (placement `machines`) with an available machine and no runners, and **one** `pending` GitLab job the pool owns. One reconcile pass places **exactly one** fleet runner (one `workload_intents` row or the equivalent the existing fleet tests assert). A later pass, with that runner starting and covering the job, places no second runner. Extend the existing `Fleet` fixture (`gitlab_lifecycle_tests.rs` about lines 1713-1800; today it sets `min_count=1` and uses `gitlab_pool` with `autoscaling_enabled=0`) with a min/max/autoscaling parameter or a second constructor; do not build a second fleet fixture. Seed `pending` rows the way `gitlab_poll_tests.rs` about lines 190-196 does.
- **Up to max:** with more pending jobs than `max_count`, check the runner total after each pass. It never exceeds `max_count`, and once the jobs are covered a later pass places nothing. Do not assume one runner per pass.
- **No demand stays no demand:** a fleet GitLab pool with min 0 and no `pending` jobs (only `created` or `manual` ones, which aren't scaler demand) places nothing.
- **Core:** `choose_platform` returns the platform with `behind == true` for the min-0 pool with one pending GitLab job, and `false` once a spare `None` runner covers it. Put these in `crates/gridops-core/tests/gitlab_pool_fleet.rs`, or next to `jobs_any_platform_can_run_go_where_machines_have_room` in `fleet/admission_tests.rs` (`targets.rs` has no test module; a new file would be `fleet/targets_gitlab_tests.rs`, wired with `#[cfg(test)] #[path = …] mod` as `autoscaling_gitlab_tests.rs` is).
- **Mixed pool:** a pool with both a GitHub repository job and a GitLab job: `choose_platform` counts both, and `targets::platform_jobs` returns only the GitHub job. Test through `platform_jobs`; don't widen `repository_for_platform`'s visibility.
- Add no `pub` items for tests only.

## Constraints

- Build and test with `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2` (already in `.claude/settings.local.json`). Use the shared cargo target dir from the untracked `.cargo/config.toml`. Never set `CARGO_TARGET_DIR` and never commit `.cargo/`. Prefer `cargo test -p gridops-core` and `-p gridops-reconciler` runs. Run `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` before the PR.
- No migration is expected. If one turns out to be needed, use the next free number after 0047 and follow the existing migrations' style.
- Add a CHANGELOG entry under Unreleased: fleet GitLab pools now scale above `min_count` for pending GitLab jobs (#181).
- PRs go to Performant-Labs/gridops only.

## Out of scope (decided)

- #182 (no v0.2.0 release) and #183 (no way to retire a fleet target).
- Bitbucket queue-driven scaling (no signal exists; see above).
- Deploying to Mars, or any write to prod.
