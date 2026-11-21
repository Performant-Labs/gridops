# Handoff-A-dup: Phase 7 - SQLITE_BUSY reconnect storm (#179)  (anti-duplication gate)

**Date:** 2026-11-21
**Branch:** claude/lock-storm-179-17e061
**Diff base:** 1d06c81   **Diff head:** 7799405
**Reuse map:** docs/handoffs/179-brief.md (extend-in-place; the brief names the objects to extend)
**Verdict:** PASS

## Summary
F extended the objects the brief named and built no parallel path. The two hot-path selects were edited in place and moved into `pub(super)` consts (the `STARTED`/`LIVE_WORKLOADS` pattern). `connect_database_path`, each binary's existing `EnvFilter` construction, and `supervisor.rs` next to `backoff()` were extended. No new module, service, client method, migration or `pub` item.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `crates/gridops-host-agent/src/supervisor.rs:~288` | `lease_expires_at` is a local in the heartbeat loop, not an `Authority` field as brief text says. This follows A's R3 warn 1 (stale `current` after an inventory refresh; only this loop uses it), and F lists it as a deviation. Not drift. | None; O may align the brief wording. |
| 2 | warn | `crates/gridops-core/src/fleet/retry.rs:53` | `LAST_FAILURE` is a sweep-driven change outside the two named selects. It is covered by the brief's "fix the class" clause, uses an existing index, and adds no migration. | None. Confirm T's Phase 6 plan assertion covers it. |
| 3 | warn | `commands.rs`, `retry.rs` | The new consts are `pub(super)` rather than the private consts of the cited precedents. This is required so the registry tests can reach them, and the brief asks for `pub(super)`. | None. |

No duplication; extension is clean.
- One retry helper (`resend_while_leased`), beside the existing `backoff()`, which it reuses. No second retry layer on logs or output. `client.rs` is unchanged.
- The `default_log_filter()` per binary is the same construction as before, extracted so it can be tested.
- The `log` dependency is declared through `[workspace.dependencies]` and is the version sqlx already resolves.
- Layering is respected: SQL stays in core registry, retry stays in the agent session loop, and no crate reaches into another.

## Notes for F
None (PASS).
