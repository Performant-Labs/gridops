# Handoff-A-dup: Phase 7 - GitLab job webhook ingestion and busy tracking  (anti-duplication gate)

**Date:** 2026-11-20
**Branch:** claude/gitlab-140-webhooks-a9fae5
**Diff base:** b89e6fb   **Diff head:** a27c7dd
**Reuse map:** docs/handoffs/140-brief.md (brief amendments; GitHub webhook path in `webhooks.rs` as analogue, D18 `gridops-core::gitlab::jobs` as the writer)
**Verdict:** PASS

## Summary
F extended the analogues the brief named instead of building parallel paths. `receive` and `retry` in `webhooks.rs` were extended (bookkeeping extracted into shared helpers, one GitLab arm in `retry`). The `gitlab_jobs` write and runner busy tracking go through D18's `record_gitlab_job` / `set_gitlab_runner_busy`, and the connection credentials come from #138's `credentials` helper.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `crates/gridops-api/src/webhooks/gitlab.rs:434` | Private `external_id(&Map, key)` resembles the private `external_id(Option<i64>)` in `gridops-core/src/gitlab/jobs.rs:355`. The inputs differ (JSON payload vs. DB column), so it is not a parallel path. | None needed now. Fold into a shared helper only if a third caller appears. |
| 2 | warn | `crates/gridops-api/src/webhooks/gitlab.rs:375` | The `runner_events` insert is inline SQL, matching the existing inline inserts in `webhooks.rs:697` and elsewhere; the brief says no helper exists to reuse. | None. |

Checks made:
- `jobs.rs`, `jobs_tests.rs` and `gitlab/mod.rs` are byte-identical (git blob) to `origin/master`. There is no second tier table, upsert or busy SQL in the API crate.
- Delivery recording, rejection, processed and failed updates reuse the helpers extracted from `receive` (behaviour unchanged). `gitlab.rs` does not copy the state machine.
- `retry` gets one dispatch arm keyed on the `gitlab:` id prefix, not a second retry handler. The existing unverified-row refusal still runs first.
- Payload helpers (`string`, `nested_object`, `nested_string`, `i64_value`, `date_value`, `required_header`) come from the parent module.
- The `credentials` helper's visibility change (`pub(crate)` plus one re-export) avoids a third copy of the trailing-slash rule. It is outside the brief's list but follows A's round-5 W2 option and changes visibility only.
- Constant-time compare uses the existing `subtle` dependency, consistent with `crypto.rs` and `credentials.rs`.
- Layering and dependency direction are intact: api module to core `jobs` to the DB. Naming (`webhooks/gitlab.rs`, `receive_gitlab`) matches the neighbours.
- The route is one line in `public_routes()`. The fleet-api.md row and the OpenAPI sentence removal are the amendments' permitted edits.

No duplication; extension is clean.

## Notes for F
None (PASS).
