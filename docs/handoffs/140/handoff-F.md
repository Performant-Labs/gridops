# Handoff-F: Phase 6 - GitLab job webhook ingestion and busy tracking

**Date:** 2026-11-20
**Branch:** claude/gitlab-140-webhooks-a9fae5 (head `96d9c9d`)
**Issue:** #140

## What was done

- `crates/gridops-api/src/webhooks/gitlab.rs` (new): `POST /api/v1/webhooks/gitlab` (`receive_gitlab`).
  - **Matching:** candidate connections are found from the payload's project (`project_id`, or `project.id` for non-`build` hooks) and its `path_with_namespace`. Each candidate's secret is compared with `X-Gitlab-Token` in constant time.
  - **Recording:** the delivery is recorded as `gitlab:<connection>:<key>` or `gitlab:unmatched:<key>`. Duplicates are acknowledged.
  - **Processing:** `process_job_hook` applies `build` events through D18's `record_gitlab_job`. The pre-fetch check, the tag lookup, and the busy set/clear plus the `runner_events` row all happen in one `BEGIN IMMEDIATE` transaction.
  - **Retry:** `recorded_connection` and `reprocess` provide the retry arm.
- `crates/gridops-api/src/webhooks.rs`:
  - adds `mod gitlab;` and the `receive_gitlab` re-export;
  - extracts four delivery bookkeeping helpers from `receive` without changing its behaviour: `record_delivery`, `reject_delivery`, `mark_delivery_processed` and `mark_delivery_failed`;
  - adds the one GitLab dispatch arm in `retry`.
- `crates/gridops-api/src/main.rs`: one route line in `public_routes()`.
- `crates/gridops-api/src/resources/gitlab.rs`: `credentials` is now `pub(crate)` (A round-5 W2).
- `crates/gridops-api/src/resources.rs`: one `pub(crate) use self::gitlab::credentials as gitlab_credentials;` (A round-5 W2).
- `crates/gridops-core/src/gitlab/jobs.rs` (new): D18's writer, byte for byte from `754a72b`. The blob is `c51247dd`, the same as on master since #160 merged.
- `crates/gridops-core/src/gitlab/mod.rs`: `pub mod jobs;`. The file is now byte-identical to master's (blob `96be8825`).
- `docs/openapi/gridops-v1.json`: removed only the sentence "Not served yet: implemented by #140."
- `docs/fleet-api.md`: one "Who may call what" row for `POST /api/v1/webhooks/gitlab` (`— | — | — | ignored`).

## Design decisions

- **The 202 has no body.** It applies to every case: a new delivery, a duplicate, and a delivery whose processing failed. The spec's 202 declares no content, and T pins this.
- **Order of checks.**
  1. Body size (413, as `receive` does).
  2. `X-Gitlab-Event`, through `required_header` (400).
  3. The JSON object check (400 "The payload is not a JSON object.").
  4. The token and candidate match (401).

  The body must be parsed before matching, because the candidates come from the payload's project. A missing token is a 401 recorded as `gitlab:unmatched:<key>` with the error "Missing X-Gitlab-Token header." A wrong or unmatched token is recorded with "No GitLab connection's webhook secret matches the token."
- **Duplicates of an unmatched id** still answer 401, never 202, so a rejected delivery sent again is never acknowledged.
- **Candidate comparison.** Every candidate's secret is compared, with no early exit. The first match in SQL order is kept: `project` before `group`, then `created_at`, then `id`. A candidate whose secret row is missing is a non-match. A secret that fails to decrypt is a 500, as in GitHub's `verify_signature`.
- **The runner id goes to the writer only for `running` events.** That is the brief's "`running` with `runner.id`: record `gitlab_runner_id`". A conclusion therefore clears the runner recorded on the job, never one that only the conclusion's payload names. Alternative considered: pass the runner on every event. Rejected, because a late conclusion could then fill and clear a runner that is now running another job.
- **Busy rules, as the round-4 amendment states them.**
  - **Set:** when `outcome.status == "running"`, `outcome.gitlab_runner_id` is `Some` and `previous_gitlab_runner_id` is `None`, whatever the write variant.
  - **Clear:** when the write is `StatusApplied`, the incoming status is terminal, and the outcome has a runner.
  - **Event row:** a `runner_events` row is written only when `set_gitlab_runner_busy` changed a runner.
- **The fallback status time is the delivery's receive time.** The live path passes the same `now` it stores as `received_at`. The retry arm reads `webhook_deliveries.received_at`, so a retry records the time the first attempt would have. This settles A's round-5 note on the brief gate's NIT-1 in the stable direction. T's test checks it as "between before and after the request", which this satisfies.
- **Malformed `build` events.** A `build` event with no usable `build_id`, `project_id` or `pipeline_id` is processed with no effect, as `process_workflow_job` treats a job without ids.

## Reuse / extend-vs-new

- **Extended** the GitHub webhook path (`webhooks.rs`), the analogue the brief names. `gitlab.rs` is a child module that reuses the parent's payload helpers (`string`, `nested_object`, `nested_string`, `i64_value`, `date_value`, `required_header`) and the delivery bookkeeping now extracted from `receive`. Brief amendment round 1, "`webhooks.rs` (finding 5)", allows that extraction. `retry` got one arm, not a second retry handler.
- **D18's writer** is #148's reviewed file, taken unchanged (A round-5 W1). No second implementation.
- **The GitLab credentials** come from #138's `credentials` helper (A round-5 W2, the primary option), not a third copy.
- **The `runner_events` insert** is inline SQL, like every other writer (A: "There is no helper to reuse").

## Architecture notes for A

- **New module:** `gridops-api::webhooks::gitlab`. It depends on `gridops_core::gitlab::jobs` (D18), `resources::gitlab_credentials` and `AppState::{runtime_secret, gitlab}`.
- **One new public handler**, `webhooks::receive_gitlab`. There are no schema changes.
- **Shared code touched:**
  - `webhooks.rs`: the bookkeeping extraction and the `retry` arm;
  - `resources.rs` and `resources/gitlab.rs`: a visibility change only. This is A's W2 recommendation. The brief was not amended after round 5, so this is an edit outside the brief's list, made on A's suggested fix. See "Deviations".
- **Transaction:** `BEGIN IMMEDIATE` on the API pool, following `fleet/targets.rs:542`. The `GitLabApi::job` call is never inside it. The `webhook_deliveries` status updates stay outside it.

## Deviations from spec / wireframe

- **`resources.rs` and `resources/gitlab.rs`** are not in the brief's shared-file list. A's round-5 W2 gave two options: make `credentials` `pub(crate)` and re-export it, or keep a commented local copy. I took the first, because the reuse-first rule otherwise means a third copy of the trailing-slash rule. The change is two lines and changes visibility only.
- **Master has moved.** #160 (D18) and #161 (client wiring) have merged into master since T's RED. I did not merge master, because the driver owns commits. `jobs.rs`, `jobs_tests.rs` and `gitlab/mod.rs` are byte-identical to master's, so the add/add merges cleanly.
- There is no wireframe (no UI).

## Tier 1 self-check (incl. tests now GREEN)

All commands ran with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`.

```
cargo build -p gridops-api                                   -> Finished, no warnings
cargo test -p gridops-api --bin gridops-api                  -> test result: ok. 121 passed; 0 failed
   (the 30 webhook tests + renamed OpenAPI test, formerly 31 failing, now pass;
    includes access::...the_documented_matrix_lists_exactly_the_registered_routes)
cargo test -p gridops-core --lib                             -> test result: ok. 291 passed; 0 failed
   (includes the 10 D18 jobs_tests)
cargo test -p gridops-api --test control_plane_contracts     -> test result: ok. 36 passed; 0 failed
cargo test -p gridops-api --test fleet_http                  -> 143 passed; 1 failed (see Known issues)
cargo fmt --all -- --check                                   -> clean
cargo clippy -p gridops-api -p gridops-core --all-targets    -> no warning in any file this change touches
```

## Evidence appendix

`docs/handoffs/140/evidence.md`

## Tests that look wrong (for T)

None.

## Known issues

- **One unrelated local failure:** `fleet_http::release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on this machine.
  - **Why:** the fixture's "Linux" machine is the local host agent, which reports its OS from `cfg!(target_os)` (`gridops-host-agent/src/native.rs:201`). On this macOS ARM64 dev box the preflight prints `PASS  macos-machine: ... fleet-host`.
  - **Why it is unrelated:** nothing in this diff touches machines, the host agent or the preflight. It should pass on Linux CI. T can confirm on base.
- **Not covered by tests:** the constant-time compare, the delivery-id header-order comment, and the fact that no log line carries the token. All three are in the diff for S: `delivery_key` and `matching_connection` in `webhooks/gitlab.rs`. The only `tracing` call logs `delivery_id`, `event` and the error.

## Files changed

- `crates/gridops-api/src/webhooks/gitlab.rs` (new)
- `crates/gridops-api/src/webhooks.rs`
- `crates/gridops-api/src/main.rs`
- `crates/gridops-api/src/resources.rs`
- `crates/gridops-api/src/resources/gitlab.rs`
- `crates/gridops-core/src/gitlab/jobs.rs` (new, byte-identical to `754a72b` and master)
- `crates/gridops-core/src/gitlab/mod.rs`
- `docs/openapi/gridops-v1.json`
- `docs/fleet-api.md`
