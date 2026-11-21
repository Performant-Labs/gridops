# Handoff-A-dup: Phase 7 - GitLab demand matching, scale-down and a pending-jobs poll (#148)  (anti-duplication gate)

**Date:** 2026-11-20
**Branch:** claude/gitlab-148-scaling-71c514
**Diff base:** 572f828 (the reviewed plan)   **Diff head:** f873139
**Reuse map:** N/A (no survey.md). The baseline is the brief's Scope and amendments A-B1 to A2-W7 and round 3 (D18), plus the GitHub analogues named there.
**Verdict:** PASS

## Summary
F extended the analogous objects the brief named. It built a new object only where the brief required one, and each of those follows a GitHub analogue. No parallel path.

- **Queue ownership extends the existing object.** `queued_job_candidates`, `assign_queued_jobs`, `queued_job_owners` and `waiting_job_pools` in `autoscaling.rs` gained a GitLab arm, with a private `QueuedJobKey`. The room rule was lifted into `pool_room!()` and shared by both arms rather than copied. `PoolStanding::of` and `pool_platforms!()` are reused. The public signatures are unchanged and every GitHub-keyed output filters GitLab out (`KeyedJobOwner::github`).
- **New objects, all brief-sanctioned.**
  - `gitlab_poll.rs` is a new module (Scope, A-B1). It copies the shape of `github_sync_worker` and `unconfirmed_jobs`/`confirm_jobs`: its own worker, `retry_when_busy` around writes, a 5 min / 10 row bounded check, and a mark set before each request.
  - `gitlab/jobs.rs` is D18's agreed local copy. It is #140's module at the identical path, and the brief names it.
- **One writer for `gitlab_jobs`.** The poll writes the table only through `record_gitlab_job` / `set_gitlab_runner_busy`, plus the single re-check `UPDATE` the brief allows (status-guarded). No second status-merge exists.
- **Edits to shared files** stay inside A-W1 / A2-W7: `mod gitlab_poll;`, the `gitlab` field, the placeholder, the spawn, `gitlab_credentials` next to `runtime_secret`, and the two test-constructor lines. `gitlab_lifecycle.rs` is untouched. The shared contract types (the `gitlab` module's existing items, `0047_gitlab.sql`) are unchanged; `mod.rs` gained only `pub mod jobs;` and the test mount.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `crates/gridops-reconciler/src/gitlab_poll.rs:329` | The poll's `runner_events` row is a free-standing `INSERT`, the same shape as `webhooks.rs:636` (the one place that writes a runner-scoped event). The reconciler's `pool_event` takes no `runner_id` and no transaction, so F's reason for not reusing it is real. A third copy of this `INSERT` shape now exists across crates. | Not blocking. Follow-up: a runner-aware event helper in `gridops-core` (next to `associate_runner_with_job` in `db.rs`) that both the webhook and the poll call, once #140 lands. |
| 2 | warn | `crates/gridops-core/src/autoscaling.rs` (`github_job_candidates` / `gitlab_job_candidates`) | The two arms are deliberately separate queries. Their row mapping is shared (`QueuedJobCandidate::from_row`) and so is the room subquery (`pool_room!()`). The pool join and tag match differ by platform, so this is not a parallel path. The `queued_at`, key and pool-order tail of the ORDER BY is repeated in each arm, as the Rust merge sort depends on it. | None needed. Note only. |
| 3 | warn | `crates/gridops-core/src/gitlab/jobs.rs`, `gitlab/mod.rs` | Add/add conflict risk with #140's `jobs.rs`. This is planned and written in the brief and the round-4 A review (warning 3). F put the tests in `jobs_tests.rs`, mounted from `mod.rs`, so the second merger can keep both sets. | Whoever merges second takes master's `jobs.rs` and runs the union of both test sets. |
| 4 | warn | `crates/gridops-reconciler/Cargo.toml` | The dev-dependency on `gridops-core` with `test-support` (for `FakeGitLab`) is outside the brief's file list. It reuses the existing fake rather than adding a new one, so it is not duplication. The diff-review NV-5 feature-unification question is for the verification phases. | None for this gate. |

No duplication; extension is clean. No drift in layering (the poll depends on `main.rs` helpers through `super::`; `main.rs` refers to the poll only for the spawn and the placeholder) or in dependency direction (`gridops-reconciler` to `gridops-core`).

## Notes for F
Not required (PASS).
