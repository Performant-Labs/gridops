# Handoff-A: Phase 3 - GitLab reconciler runner lifecycle (#139)  (up-front plan review, round 4)

**Date:** 2026-11-20
**Branch:** claude/gitlab-139-lifecycle-53c8ce (head 03c596f; master b22c451)
**Brief reviewed:** docs/handoffs/139-brief.md (blob a8a9e8e: the round-3 amendments plus brief-gate rounds 1-4)   **Reuse map:** N/A. No `survey.md` was provided, so the brief's named analogues are the baseline: `next_bitbucket_connection`, the Bitbucket and GitHub arms of `provision_on_host` / `start_admitted_runner` / `delete_runner`, the GitHub fleet path in `fleet.rs`, `record_provision_failure` and `start_tests.rs`.   **Wireframe:** N/A (no UI surface)
**Verdict:** BLOCK

## Summary

BLOCK, with 1 block finding and 3 warnings.

The round-3 amendment resolves both round-3 blocks and all three round-3 warnings. The count and the reset are now tied to a guarded, one-time transition on each path. The manager-path image rule applies only to Docker, and `clear_provision_failures` has a single home.

One gap is left, and it is the read half of round 3's stale-pool point. On the manager path, the plan counts or resets the circuit in `reconcile_pool`'s status loop, part-way through the pool's pass. The rest of that pass still decides with the `Pool` it loaded before the write. So the pass that counts a refusal starts a replacement at once: the 30 s and 60 s backoffs are skipped, and on the third refusal a start goes through the circuit that just opened. The state written at the end of the pass can also overwrite `backoff` or `blocked`.

## Status of round-3 findings

| Round 3 | Status |
|---|---|
| Block 1 (count or reset once per runner) | **Resolved for the write.** On the fleet path the trigger is the guarded `UPDATE … WHERE status NOT IN ('deleted','failed')` affecting one row (`follow` `:530`, `finish` `:608`). On the manager path it is the previous status. `record_provision_failure` gets a freshly loaded `Pool`, and Done-when 10 covers it. The fleet path counts inside `fleet::tick`, which `reconcile()` runs before `load_pools` (`main.rs:526`, `:566`), so the fleet path has no stale read. The manager path does: block 1 below. |
| Block 2 (Tart image) | **Resolved.** The core image function applies only when the provider is `docker`, and a Tart start passes `tart_image` unchanged (`provisioning.rs:66-74`). Done-when 3 covers it. |
| Warning 3 (reset helper) | Resolved. `clear_provision_failures` sits next to `record_provision_failure`, `:2766` calls it, and the unmodified `a_successful_probe_clears_the_blocked_state_in_the_same_pass` guards the extraction. |
| Warning 4 (where busy is seen) | Resolved. The fleet reset happens on exit only, the manager busy reset is optional, the `blocked` display lag is accepted, and the dependency on #140 is recorded. |
| Warning 5 (fleet `create_runner` failures) | Resolved. Every failure counts once, through the guarded update. |

Also checked against the tree and found consistent:
- **Docker-socket refusal (R3-W-3).** It sits in `fleet::provision` and returns an `Err`, the way that function's existing refusals do (`fleet.rs:163`, `:180`).
- **Drain (R4-W-2).** Fleet rows that `follow` or `finish` mark `failed` are deleted by `reconcile_pool`'s failed loop (`main.rs:828-835`, which uses `runners()` and so includes fleet rows) and by drain (`:991-1003`), both through `delete_runner`. GitLab deregistration on exit and drain therefore stays in one place, the GitLab arm of `delete_runner`, with no parallel cleanup path.
- **Sweep cadence.** It matches `maybe_sync_github` (`:1555`).

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | block | Seam, `main.rs`: "`reconcile_pool`'s exited→failed mapping (… `registration_refused` → … counted through `record_provision_failure` …)". Round-3: "Manager path: act only when the row's previous status was not already `failed`/`deleted` … `record_provision_failure` and the reset are called with a `Pool` loaded by id at that moment, not the pass's stale copy." | cross-cutting (failure accounting); pattern consistency (record a failure, then stop or re-read) | **The manager path writes the circuit part-way through a pass, and the rest of that pass reads the old copy.** The status loop runs at `main.rs:794-826`. After it, `reconcile_pool` keeps using the `pool` it was handed:<br>- `retry_deferred` is computed at `:867` and decides `provisioning_blocked` at `:870`.<br>- The end-of-pass state is built at `:1026`.<br><br>**What this does.**<br>- The pass that counts refusal *n* deletes the refused runner, sees `active < desired`, and goes into the start loop (`:947`) with the old `provision_retry_at`. It creates a GitLab runner and POSTs a new start straight away. The 30 s and 60 s backoffs are skipped.<br>- On the third refusal, the start goes through the circuit `record_provision_failure` has just opened.<br>- If that pass starts nothing, `PassOutcome::for_pool(pool)` still carries the old `circuit_open` and `failure_count`. `finish_pool_pass` (`:1133`) then overwrites the `backoff` or `blocked` that `record_provision_failure` wrote.<br>- A reset in the status loop stays `blocked` or deferred for one more pass.<br><br>**The codebase pattern it breaks.** Both production call sites of `record_provision_failure` end the pool's pass right after recording: `:942` → `return Err` at `:944`, and `:2503` → `Err` → `?` at `:949`. Where the circuit does change mid-pass, the pass re-reads it before computing state (`:1028-1041`: "a probe that works shows the pool as it is now").<br><br>**Why the tests won't catch it.** Done-when 10 checks counts only, so the bypass would pass T and F unnoticed. | **Refresh the pool after the status loop.**<br>- **Loader.** Add `load_pool(database, pool_id) -> Result<Option<Pool>>` next to `load_pools` (`:771`). Share its SELECT, for example a `WHERE p.id=?` variant built from the same column list, so there is no second copy of that 10-line column list. Every GitLab count and reset uses it, on both paths.<br>- **Refresh.** In `reconcile_pool`, if the status loop counted or reset anything, reload the pool and use the fresh copy for the rest of the pass (`:867`, `:870`, `:1026`).<br>- **Done-when (manager path).** The pass that counts a GitLab refusal makes no further start for that pool while the new backoff holds: no `create_runner` and no `POST /v1/runners`. It ends `backoff`, or `blocked` at the threshold. The pass that resets ends without `blocked`. |
| 2 | warn | Round-2/3 block 1: a GitLab start that succeeds at POST "is skipped" for the reset; the reset waits for proof of registration. | cross-cutting (the circuit's half-open probe) | **The GitLab probe isn't one attempt.**<br>- The circuit's design is "once [the cool-down] passes, one attempt probes whether the cause has cleared" (`:868`). For GitHub and Bitbucket that holds: the first start's success resets the circuit, and its failure aborts the loop (`Err`).<br>- A GitLab start that succeeds at POST does neither. With the circuit open and `provision_retry_at` already past, the start loop at `:947` goes on to start up to `desired - active` runners in the probe pass. The fleet loop does the same through `fleet::provision`. Each of those runners creates, is refused by, and deletes a GitLab runner.<br>- It is bounded: once started, they count as active. But each cool-down costs `desired` refused runners rather than one. | Either end the start loop for the pass after the first GitLab start made while `pool.provision_circuit_open` is set (as `HeldBack` ends it), on both paths, or record in the brief that a GitLab probe is `desired - active` starts, as a deliberate choice. |
| 3 | warn | One ordering per path, fleet: "`gitlab_runner_id` is set by UPDATE … If storing the bootstrap or issuing the start command fails after `create_runner`, the GitLab runner is deleted (best effort …)". Plus Done-when 6. | pattern consistency / duplication (two deregistration sites for one runner) | **On the fleet path the same GitLab runner is deregistered twice.**<br>- The fleet rollback deletes the GitLab runner inline, after `gitlab_runner_id` has been stored, and marks the row `failed`.<br>- `reconcile_pool`'s failed loop (`:828-835`) then sends that row through `delete_runner`. Its GitLab arm sees `gitlab_runner_id` and deletes again, which gets `NotFound` and logs a warning.<br>- The analogues avoid this in two ways. The manager Bitbucket arm stores `bitbucket_runner_uuid` only in the post-POST UPDATE, so `delete_runner` never repeats its inline rollback (`:2690-2716`). The GitHub fleet arm has no inline rollback; `delete_runner` deregisters `github_runner_id` for the failed row (`:407`, `:2828`). | Pick one on the fleet path:<br>(a) drop the inline delete. Mark the row failed and let `delete_runner`'s GitLab arm deregister it, as the GitHub fleet arm does.<br>(b) after a successful inline delete, set `gitlab_runner_id = NULL` in the same failure UPDATE.<br><br>In either case, the GitLab deregister helper treats `NotFound` as done, as `remove_manager_container` treats 404 (`:2950`). |
| 4 | warn | Brief-gate W-3: "the reconciler pass returns without error for that pool and still reconciles the other pools". | cross-cutting (error boundary) | **"Without error" can be read as swallowing the failure.** Today a counted manager-path start failure makes `reconcile_pool` return `Err` for that pool. `reconcile()` logs it as "Pool reconciliation failed" and goes on to the next pool (`:595-608`). That is also how "exactly as the Bitbucket arm does" behaves. Read literally, W-3 asks the GitLab arm to swallow `StartOutcome::Failed`, which would diverge from the Bitbucket arm. | Reword W-3 to say: "`reconcile()` returns `Ok`; that pool's `Err` is logged and recorded as today, and other pools in the same pass are reconciled". |

## Notes for O

Amend `docs/handoffs/139-brief.md`. Add a "Round-4 amendments (architecture gate R4)" section, then start a **fresh** run. Do not use `resumeFromRunId`.

1. **Finding 1 (block).** Add these to the round-3 block-1 bullets:
   - "`load_pool(database, pool_id)` is added next to `load_pools`, sharing its SELECT (no second copy of the column list). Every GitLab count and reset uses it, on both paths."
   - "On the manager path, if `reconcile_pool`'s status loop counted or reset anything, the pool is reloaded with `load_pool` and the fresh copy is used for the rest of that pass: `retry_deferred`/`provisioning_blocked` (`main.rs:867-870`) and `PassOutcome::for_pool` (`:1026`). This follows the existing re-read at `:1028-1041`. The fleet path needs no reload, because `fleet::tick` runs before `load_pools`."

   Add to Done-when 10: "Manager path: the pass that counts a GitLab refusal starts no runner for that pool while the backoff holds (no `create_runner`, no `POST /v1/runners`) and ends `backoff`, or `blocked` at the threshold. The pass that resets ends without `blocked`."
2. **Warnings 2-4.** Add one line each:
   - Warning 2: end the start loop after the first GitLab start made while the circuit is open (the probe), on both paths. Or state that a GitLab probe is `desired - active` starts.
   - Warning 3: on the fleet path, either let `delete_runner`'s GitLab arm do the rollback deregistration, or null `gitlab_runner_id` after an inline delete. The deregister helper treats `NotFound` as done.
   - Warning 4: W-3's "returns without error" means `reconcile()` returns `Ok`. The pool's own `Err` is logged as today.

## Patterns referenced

- `crates/gridops-reconciler/src/main.rs`:
  - `reconcile` (`:522-608`: `fleet::tick` before `load_pools`, and the per-pool `Err` boundary);
  - `load_pools` (`:771`);
  - `reconcile_pool`: the status loop `:794-826`, the failed loop `:828-835`, `retry_deferred`/`provisioning_blocked` `:867-870`, the start loop `:947-990`, drain `:991-1003`, and the mid-pass re-read `:1028-1041`;
  - `pool_state`, `finish_pool_pass` (`:1103`, `:1132`);
  - the Bitbucket start arm's rollback (`:2686-2716`);
  - `delete_runner` (`:2788-2897`);
  - `record_provision_failure` and its two production call sites (`:3608`, `:942`, `:2503`).
- `crates/gridops-reconciler/src/fleet.rs`: `tick` (`:246-274`), `register_and_start` (`:335-479`, `github_runner_id` stored at `:407`), `follow` (the guarded UPDATE at `:530`), `finish` (the guarded UPDATE at `:608`).
- `crates/gridops-core/src/fleet/retry.rs` (`retry_failed_start` retries only a row still `starting`), `crates/gridops-core/src/fleet/start_health.rs` (`is_start_failure`: `configuration` is not the machine's fault).
- `crates/gridops-core/src/provisioning.rs` (`for_selected_provider`, `:44-84`).
