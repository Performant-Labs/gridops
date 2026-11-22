# Decision journal: #181 fleet GitLab pools scale on pending jobs

## Phase 3 Architecture (A, up-front plan review) - 2026-11-21T18:05:07-08:00
- **Decided:** BLOCK, with 1 block and 5 warn findings (`docs/handoffs/181/handoff-A.md`).
  - **The block: the brief doesn't name the seam.** It says to change `attribute_queued_jobs` "or … whatever it reads its jobs from" (`queued_job_assignments`), and offers an `Option` repository as one representation.
    - `queued_job_assignments` also feeds, through `provider_capacities` and `next_runner_provider`: the GitHub webhook's scale target (`webhooks.rs:870`), the reconciler's `maybe_scale_up` (`main.rs:1316`) and `evict_surplus_runner` (`main.rs:1457`), and the built-in provider choice (`resources.rs:4368`).
    - #148's binding A-B2 keeps these views and `QueuedJob` public and GitHub-only.
    - An `Option` on the public `QueuedJob` breaks the A-B2 pins (`autoscaling_gitlab_tests.rs:383-394, :440-445`) and `gridops-api/tests/fleet_http/recovery.rs:921-922`. Scope 3 doesn't list the latter.
    - Fix: one `pub(crate)` accessor built on a single `queued_job_owners` call, and a private job type inside `attribute_queued_jobs`. `platform_jobs` filters its output to GitHub jobs after attribution.
  - **The warns:**
    - (2) run `queued_job_owners` once per attribution, not twice;
    - (3) attribution internals: a GitLab job counts on every profile, only a `None` spare runner covers it, and it is never in `waiting` or `prioritised`;
    - (4) test homes: `targets.rs` has no test module; the `Fleet` fixture's pool is manual with `min_count=1`; `repository_for_platform` is private to `fleet`;
    - (5) the pass calls `provision` `desired - active` times, so it doesn't place one runner per pass;
    - (6) the corrected doc-comment list.
- **Assumed:** with no `survey.md`, the brief's named analogue (GitHub fleet demand through `attribute_queued_jobs`, `choose_platform` and `platform_jobs`, with ownership from `queued_job_owners`) serves as the Reuse map. #181 knowingly reverses only A-B2's "fleet placement is untouched" clause.
- **Hedged:** finding 1 is a block, not a warn, because the brief invites T to pin a public shape before any code exists. Scope 3's tests would eventually force F onto the internal seam, but only after T's tests had pinned the wrong shape.
- **Evidence:** I read:
  - the brief and the brief gate result;
  - #148's brief amendments A-B2 to A-B4 and its handoff-A;
  - `autoscaling.rs` (ownership, candidates, the queue views, `provider_capacities`, `next_runner_provider`);
  - `autoscaling_gitlab_tests.rs` (fixtures and the A-B2 pins);
  - `fleet/targets.rs:1120-1561`;
  - reconciler `fleet.rs:1-270`, `main.rs` (the provisioning loop, `maybe_scale_up`, `evict_surplus_runner`, `provision`), and `gitlab_lifecycle.rs` (`next_connection`, `fleet_target`);
  - `gitlab_lifecycle_tests.rs` (`gitlab_pool`, `Fleet`) and `gitlab_poll_tests.rs` (pool and row seeders);
  - `gridops-api` `webhooks.rs`, `resources.rs` and `tests/fleet_http/recovery.rs`;
  - `gridops-core/tests/gitlab_pool_fleet.rs` and `docs/gitlab-runners.md`.

  `git diff master -- crates src` is empty, and `git worktree list` shows no live overlap.

## Phase 3 Architecture (A, re-review of the amended brief) - 2026-11-21T18:16:53-08:00
- **Decided:** PASS, with 0 block and 2 warn findings (`docs/handoffs/181/handoff-A.md`, which replaces the earlier BLOCK review in 233dac5).
  - **Earlier block A-B1 resolved:** the amended brief (a19e9e6) names the seam.
    - The public GitHub-only views and `QueuedJob` stay unchanged.
    - One `pub(crate)` accessor with a single `queued_job_owners` call.
    - The job type stays internal to attribution.
    - `platform_jobs` filters GitLab entries out after attribution.
    - Only A-B2's "fleet placement is untouched" clause is superseded.
    - The gridops-api fleet tests are in Scope 3.
  - The earlier warns 2-6 are all in the brief.
  - **Warn 1:** the job type crosses from `autoscaling.rs` to `fleet/targets.rs`, so it must be `pub(crate)` beside the accessor. Prefer an enum that wraps `QueuedJob` (`Github(QueuedJob)` / `Gitlab`) and a `github()`-style filter like `KeyedJobOwner::github`. Avoid a struct that copies `QueuedJob`'s fields, or a second mapping type in `targets.rs`.
  - **Warn 2:** the shared helper returns rows in SQL order and can leave out owned ids, so the accessor must place GitHub rows in owner order by id and skip missing ids. For a GitHub-only pool, owner order already equals the helper's order (`github_job_candidates` orders by `wj.created_at, wj.id`), so byte-identity holds.
- **Assumed:** with no `survey.md`, the brief's named analogue serves as the Reuse map (GitHub fleet demand through `attribute_queued_jobs`, `choose_platform` and `platform_jobs`, with ownership from `queued_job_owners`). This is a fresh run on an amended brief, not a resumed one.
- **Hedged:**
  - Warn 1 is a warn and not a block: the brief allows either shape, and the type is not public, so T can't pin it.
  - Warn 2 is guidance that A will check at Phase 7, not a change to the brief.
- **Evidence:** I re-read:
  - the amended brief, and its diff against 048c53a;
  - `autoscaling.rs`: ownership, candidates and their ordering, `assign_queued_jobs`, the queue views, and the `QueuedJob` definition;
  - `fleet/targets.rs:1290-1561`;
  - reconciler `fleet.rs:50-200`, and `main.rs` (the pass loop, `maybe_scale_up`, `provision`/`NoDemand`);
  - the `Fleet` fixture and `gitlab_pool`;
  - the pending `row` seeder in `gitlab_poll_tests.rs`;
  - `tests/gitlab_pool_fleet.rs`;
  - the `queue_scale_factor` default (`migrations/0003_autoscaling.sql:2`, DEFAULT 1);
  - #148's A-B2 text;
  - every caller of `choose_platform` and `platform_jobs`.

  `git diff master -- crates src` is empty. No live worktree overlaps: `cool-davinci` is clean, and `gridops-134` is stale (its work landed as 80e6f1b).
