# Handoff-A: Phase 3 - GitLab reconciler runner lifecycle (#139)  (up-front plan review, round 3)

**Date:** 2026-11-20
**Branch:** claude/gitlab-139-lifecycle-53c8ce (head d88acd5; master b22c451)
**Brief reviewed:** docs/handoffs/139-brief.md (working tree, blob 6d826e7: round-2 amendments, block 1-4 and warnings 5-9)   **Reuse map:** N/A. No `survey.md` was provided, so the brief's named analogues are the baseline: `next_bitbucket_connection`, the Bitbucket and GitHub arms of `provision_on_host` / `start_admitted_runner` / `delete_runner`, the GitHub fleet path in `fleet.rs`, `record_provision_failure` and `start_tests.rs`.   **Wireframe:** N/A (no UI surface)
**Verdict:** BLOCK

## Summary

BLOCK, with 2 block findings and 3 warnings.

The round-2 amendment resolves all four round-2 blocks in substance. The sweep now covers every connection, `retry.rs` is in the seam, the image substitution point lives in core where the fleet profile is saved, and a GitLab start no longer clears the circuit when the manager POST succeeds.

Two parts of the new text are still wrong when checked against the code:
1. **Counting and reset aren't tied to one event per runner.** On the fleet path they land in `finish` and `follow`, which revisit the same placement on every tick. `finish` keeps revisiting a failed or cleaned placement for as long as it exists. Counted per tick, a single refusal blocks the pool permanently. Reset per tick, refusals never add up to the threshold.
2. **The manager-path half of the image rule refuses Tart.** A Tart-only pool always has an empty `docker_image`, so the rule as written refuses every manager-path GitLab Tart start. For Docker starts on the manager path the rule never fires.

## Status of round-2 findings

| Round 2 | Status |
|---|---|
| Block 1 (circuit reset rule) | **Resolved in substance.** The count resets only on proof of registration, the reset at POST success is skipped for GitLab, both paths count, and there is one shared reset helper. A new gap remains: block 1 below (count and reset once per runner). |
| Block 2 (image substitution point) | **Resolved for the fleet path.** The substitution is in core `targets.rs`, and `pool_docker_image` and `save_pool_fleet` call it. The manager-path half is wrong for Tart: block 2 below. |
| Block 3 (sweep misses the deleted-only-pool connection) | **Resolved.** The sweep covers every `gitlab_connections` row, and Done-when includes the case. |
| Block 4 (`retry.rs`) | **Resolved.** `retry.rs` copies `gitlab_connection_id` but not `gitlab_runner_id`, and it is in the seam, in Owns and in Done-when. |
| Warning 5 (`RunnerSummary.id`) | Resolved. `u64` is kept because the published D17 says so, it is converted in one place (the sweep), and the seeding method is `with_runner`. |
| Warning 6 (sweep cadence) | Resolved. It follows the `maybe_sync_github` pattern and is called next to `retry_github_runner_cleanup`, wrapped so an error only warns. |
| Warning 7 (instance discriminator) | Resolved. The marker is `gridops-runner:<instance>:<uuid>`, and the sweep deletes only markers with its own instance. |
| Warning 8 (failed fleet `create_runner`) | Resolved: `abandon_unstarted`, the row marked failed, and the old registration deleted before a repeat attempt. The counting rule is still unclear: warning 5 below. |
| Warning 9 (`builtin_host.rs` fixture) | Resolved. |

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | block | Round-2 block 1: on the fleet path, "a GitLab `RegistrationRefused` (on Start or Inspect) counts through the same `record_provision_failure`, and a GitLab fleet runner that … exits `completed`/`IdleExpired` resets it". The manager-path counterpart is in `reconcile_pool`'s mapping of exited runners to failed. | cross-cutting (failure accounting); pattern consistency (fleet.rs steps are idempotent per tick) | **The plan says what counts and what resets, but not when. The places it would run revisit the same placement on every tick.**<br>- **Start refusals.** A refused Start fails the placement in core (`registry/commands.rs::apply_to_placement` → `fail_placement`). The reconciler only sees that through `fleet::finish`. `tick` calls `finish` on every `failed`, `cleaned` and `uncertain` placement on every pass (`fleet.rs:265`). Those states are terminal; placements are removed only when their pool is deleted (`targets.rs:800`). Counted on the diagnostic in `finish`, one refusal adds 1 every tick. The circuit opens within about three passes, and `record_provision_failure` pushes `provision_retry_at` 15 minutes out each time (`main.rs:3608`). The pool stays `blocked` for as long as the pool exists. An operator retry is undone on the next tick, and no start runs that could prove registration and reset it. That is the 08c32b3 / #130 wedge the issue forbids.<br>- **Inspect refusals.** `follow` keeps seeing `exited` on every tick until the Cleanup lands, so the same over-count happens unless the count sits inside its `!deleted` branch.<br>- **Resets.** A reset placed where `finish` or `follow` reads a "completed / `IdleExpired`" result fires on every tick for every old placement. Refusals then can't add up to the threshold, which is round-2's 0→1→0 cycle again.<br>- **Manager path.** `reconcile_pool`'s status loop re-maps a container that is still listed as `exited` on every pass. That happens whenever `delete_runner` errors (for example, `remove_manager_container`).<br>- **Stale pool.** `record_provision_failure` writes `pool.provision_failure_count + 1` from the `Pool` it is given. Two refusals recorded against a pool loaded at the start of the pass count once. | **Edge-trigger both sides: one count or one reset per runner, when its row leaves service.**<br>- **Fleet path:** count or reset only when the guarded `UPDATE runners SET status='failed' … WHERE status NOT IN ('deleted','failed')` affects a row. That is the `!deleted` branch in `follow` and the final UPDATE in `finish`; read `rows_affected() == 1`.<br>- **Manager path:** count or reset only when the runner's previous status was not already `failed`.<br>- **Fresh pool:** pass `record_provision_failure` a `Pool` loaded by id at that moment.<br>- **Done-when:** one refusal seen over several ticks or passes leaves `provision_failure_count == 1` with the circuit closed. An old completed placement does not reset a count that later refusals raised. Both on each path. |
| 2 | block | Round-2 block 2: "`pool_docker_image` … and the reconciler's manager path (`provision_on_host`'s image for a GitLab start) both call it … a manager-path start is refused before `create_runner`" whenever a GitLab pool's `docker_image` is empty. The issue says "This holds for any provider (Docker or Tart)". | correctness / layering (the substitution is keyed on the pool, but on the manager path the image depends on the provider) | **On the manager path, the rule as written does nothing for Docker and refuses every GitLab Tart start.**<br>- `for_selected_provider` uses `docker_image` only when the provider isn't `tart`. For `tart` it uses `tart_image`, or no image for the native runtime (`provisioning.rs:66-74`).<br>- A pool that uses Docker must have a non-empty `docker_image` (`models.rs:419-427`, #103), so the empty case never happens for a Docker start on the manager path.<br>- A pool that doesn't use Docker stores `''` (`selected_image`, `models.rs:510-527`; migration `0044` cleared the old default).<br>- A GitLab pool with a Tart provider, the macOS path #142 serves, therefore always meets "a GitLab pool with an empty `docker_image`", and would be refused "GitLab pool has no runner image" before `create_runner` on every start.<br>- The fleet half is right. `pool_docker_image` runs only for a Docker-runtime platform saved without an image (`targets.rs:496-503`). | **Scope the manager-path call to the Docker provider, or drop it.**<br>- Apply the core function only to the `docker_image` argument of `for_selected_provider`, and only when the selected provider is `docker`. Given #103's validation, that is only a defensive guard.<br>- A Tart start passes `tart_image` unchanged and is never refused because `docker_image` is empty.<br>- **Done-when:** a manager-path GitLab start on a Tart-only pool, which has `docker_image = ''`, calls `create_runner` and sends the request with `tart_image`. |
| 3 | warn | Round-2 block 1: "one shared reset helper (in `gitlab_lifecycle.rs`, wrapping the existing reset query)", while "GitHub and Bitbucket keep today's reset behaviour" | file structure / cohesion | The circuit's write side, `record_provision_failure` and `provision_failure_policy`, lives in `main.rs`. Today's reset is an inline UPDATE at `main.rs:2766`, followed by `set_pool_wait_reason(None)`. If a GitLab module "wraps" that query while `start_admitted_runner` keeps its inline copy, the same pool-circuit UPDATE exists twice. The GitLab-agnostic half of the circuit would also sit in a platform module. | Move the UPDATE and `set_pool_wait_reason(None)` into `clear_provision_failures(database, pool_id)` next to `record_provision_failure` in `main.rs`. `start_admitted_runner`'s non-GitLab branch calls it (no behaviour change), and the GitLab arms call it from `gitlab_lifecycle.rs` and `fleet.rs` through `super::`. If it stays in `gitlab_lifecycle.rs`, `main.rs:2766` must call it instead of keeping the inline query. |
| 4 | warn | Round-2 block 1: the count resets when the runner "is reported busy" | completeness (no named place to observe it) | **Busy is observable on the manager path only.** For a GitLab runner, `runners.busy` comes from #140's webhook (#148: "a busy signal (from #140)"). The manager path sees it in `reconcile_pool`'s status loop (`"running" if runner.busy => "busy"`, `main.rs:807`). The fleet path never reads `busy`: `follow` reads only `status`. <br>**The pool reads `blocked` after a successful probe.** Once the cool-down ends, a GitLab probe that works leaves `provision_circuit_open` set until that runner exits. So a working pool shows `blocked` for up to one job, or 600 s idle. | Name the observation points. On the manager path, the status loop resets when a GitLab runner's computed status first becomes `busy`. On the fleet path, `follow` reads `runners.busy`, or the brief says the fleet reset is exit-only. Record that the `blocked` display lag is accepted. |
| 5 | warn | Round-2 warning 8: on a failed fleet `create_runner`, "counted, per block 1 when the error is a refusal; transient `RateLimited`/`Upstream` follow the existing start-failure handling" | cross-cutting (failure accounting, manager/fleet parity) | **The fleet path has no counted start-failure handling to follow.** It never calls `record_provision_failure`. The manager path counts *any* registration error through `StartOutcome::Failed` (`main.rs:2503`). <br>**`create_runner` errors aren't "refusals" either.** `create_runner` returns `GitLabError` (`Unauthorized`, `MissingScope`, `MissingRole`, `NotFound`, `RateLimited`, `Upstream`). `RegistrationRefused` is an agent diagnostic. <br>**In production this produces churn.** With the placeholder client, every GitLab fleet start fails with `Upstream` and is not counted. Each pass then submits a new intent and has a machine prepare an environment, only to abandon it. | Count every fleet `create_runner` failure once, through `record_provision_failure` with a fresh `Pool`, as the manager path does. Its backoff (30 s, doubling, then a 15-minute circuit) is then what limits the churn while the placeholder is in place. If transient errors should stay uncounted, say what limits the churn instead. |

## Notes for O

Amend `docs/handoffs/139-brief.md` (the "Round-2 amendments" section), then start a **fresh** run. Do not use `resumeFromRunId`.

1. **Finding 1.** Add to block 1: "Each refusal counts once, and each proof of registration resets once, per runner, when that runner's row first leaves service.
   - On the fleet path, act only when the guarded `UPDATE runners SET status='failed' … WHERE status NOT IN ('deleted','failed')` in `follow` (its `!deleted` branch) or in `finish` affects a row. `finish` and `follow` revisit the same placement on every tick.
   - On the manager path, act only when the runner's previous status was not `failed`.
   - `record_provision_failure` gets a `Pool` loaded by id at that moment."

   Add to Done-when: "one refusal seen over several ticks or passes leaves the count at 1 and the circuit closed, on each path; an old completed placement does not reset a count that later refusals raised".
2. **Finding 2.** Replace "the reconciler's manager path (`provision_on_host`'s image for a GitLab start) … a manager-path start is refused before `create_runner`" with: "on the manager path the core function applies only when the selected provider is `docker`, to the `docker_image` argument of `for_selected_provider`. A Tart start passes `tart_image` unchanged and is never refused because `docker_image` is empty."

   Add to Done-when: "a manager-path GitLab start on a Tart-only pool (`docker_image = ''`) calls `create_runner` and sends the request with `tart_image`".
3. **Warnings 3-5.** One line each in the brief:
   - Warning 3: the reset helper's home, `clear_provision_failures` next to `record_provision_failure`, with `main.rs:2766` calling it.
   - Warning 4: the places where busy is observed, or "the fleet reset is exit-only", and that the `blocked` display lag is accepted.
   - Warning 5: every fleet `create_runner` failure counts once, as on the manager path.

## Patterns referenced

- `crates/gridops-reconciler/src/fleet.rs`:
  - `tick` (`:265`, which revisits failed, cleaned and uncertain placements);
  - `follow` (the `!deleted` guarded UPDATE), `finish` (`:568`, the guarded final UPDATE), `register_and_start`.
- `crates/gridops-reconciler/src/main.rs`:
  - `reconcile_pool`'s status loop (`:794-826`);
  - `start_admitted_runner`'s reset (`:2766`), `StartOutcome::Failed` → `record_provision_failure` (`:2503`, `:3608`);
  - `provision_failure_policy`, `pool_state`.
- `crates/gridops-core/src/fleet/`:
  - `registry/commands.rs`: `apply_to_placement`, where a Start failure fails the placement;
  - `placement.rs`: `placements`, `reported_gone`, `abandon_unstarted`;
  - `retry.rs`, `start_health.rs`: `is_start_failure`, where `configuration` is not retried;
  - `targets.rs`: `pool_docker_image`, `save_pool_fleet`, the placement delete in `delete_pool`.
- `crates/gridops-core/src/provisioning.rs` (`for_selected_provider`'s image choice), `crates/gridops-core/src/models.rs` (`selected_image`, the #103 rule that a Docker pool needs an image), `migrations/0044_clear_unused_provider_images.sql`.
- Issue #148, which says the GitLab busy signal comes from #140.
