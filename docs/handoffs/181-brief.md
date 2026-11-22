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

1. **GitLab pending jobs count as fleet demand.** In `attribute_queued_jobs`, or in whatever it reads its jobs from, the pending GitLab jobs this pool owns (the same ownership `assigned_queued_jobs` counts, so there is no second label-matching rule) must be attributed to the pool's platforms exactly as GitHub jobs are. `choose_platform` must then report the platform as behind while the pool's runners don't cover them, scaled by `queue_scale_factor`, as for GitHub.
   - Read ownership from `queued_job_owners` and keep one source of truth. Do not re-match tags against labels in a new query.
   - A GitLab job has no GitHub repository. It must not be attributed to, or counted against, a repository it doesn't belong to. GitLab runners are registered without a target repository (`target_repository_id` NULL), so an idle GitLab runner covers any of the pool's GitLab jobs, and the `deficit` logic's `is_none_or` already treats it that way. Choose a representation, for example a separate source tag or an `Option` repository, that keeps `repository_for_platform` and `platform_jobs` in `crates/gridops-reconciler/src/fleet.rs` from ever returning a GitHub repository for a GitLab job. Do not use a sentinel id that could collide with a real repository id.
   - Platform matching: a GitLab job's tags are already matched against the pool's configured labels by ownership. Attribute it to the pool's platforms as a job with no platform-specific label requirement. It must not be refused because its tags aren't OS or architecture labels.
   - Prioritisation (`prioritised`) is a GitHub repository concept. GitLab jobs are not prioritised unless they already are elsewhere. Keep it false and say so in a doc comment.
   - Update the doc comments that say GitLab jobs are left out (`owned_queued_jobs`, `queued_job_owners`, `queued_job_assignments`, `attribute_queued_jobs`, `choose_platform`) so they describe the new behaviour accurately.
2. **Scale from `min_count` up to `max_count`.** With the fix, `provision`'s fleet branch places one runner per reconcile pass while the platform is behind, up to the desired count that `maybe_scale_up` already sets (capped at `max_count`). Don't change the `NoDemand` guard itself. The guard is correct; its input was wrong.
3. **GitHub fleet scaling unchanged.** For a pool with only GitHub jobs, every existing ownership, attribution, `choose_platform`, `platform_jobs` and `repository_for_platform` result must be identical. All existing tests in `gridops-core` (`fleet/targets.rs` tests, `autoscaling` tests, `autoscaling_gitlab_tests.rs`, `fleet/admission_tests.rs`) and `gridops-reconciler` (`start_tests.rs`, `gitlab_lifecycle_tests.rs`, `gitlab_poll_tests.rs` and the rest) keep passing without changes to their assertions.

## Tests (required)

- **The #181 regression:** a GitLab pool with `min_count` 0 and `max_count` 2 (or more), linked to a GitLab connection, on a fleet platform (placement `machines`) with an available machine and no runners, and **one** pending GitLab job the pool owns. `choose_platform` returns that platform with `behind == true`, and one reconcile/provision step places **exactly one** fleet runner (one `workload_intents` row or the equivalent the existing reconciler fleet tests assert). A second pass with that runner starting (idle, covering the job) places no second runner.
- **Up to max:** with several pending jobs (more than `max_count`), repeated passes place runners up to `max_count` and no further.
- **No demand stays no demand:** a fleet GitLab pool with min 0 and no pending jobs (or only `created`/`manual` ones that aren't scaler demand) places nothing.
- **Mixed pool:** a pool with both a GitHub repository job and a GitLab job counts both, and `repository_for_platform` never returns a repository for the GitLab job.
- Put the tests next to the existing ones of the same layer (core: `fleet/targets.rs` tests or a sibling `*_tests.rs`; reconciler: the existing fleet provisioning tests). Add no `pub` items for tests only.

## Constraints

- Build and test with `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2` (already in `.claude/settings.local.json`). Use the shared cargo target dir from the untracked `.cargo/config.toml`. Never set `CARGO_TARGET_DIR` and never commit `.cargo/`. Prefer `cargo test -p gridops-core` and `-p gridops-reconciler` runs. Run `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` before the PR.
- No migration is expected. If one turns out to be needed, use the next free number after 0047 and follow the existing migrations' style.
- Add a CHANGELOG entry under Unreleased: fleet GitLab pools now scale above `min_count` for pending GitLab jobs (#181).
- PRs go to Performant-Labs/gridops only.

## Out of scope (decided)

- #182 (no v0.2.0 release) and #183 (no way to retire a fleet target).
- Bitbucket queue-driven scaling (no signal exists; see above).
- Deploying to Mars, or any write to prod.
