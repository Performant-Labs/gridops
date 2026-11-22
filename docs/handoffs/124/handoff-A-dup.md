# Handoff-A-dup: Phase 7 - Flaky tests: tart-agent slot wait and API auth boundary (#124)  (anti-duplication gate)

**Date:** 2026-11-22
**Branch:** claude/fix-124-flaky-tests
**Diff base:** 9e357e0   **Diff head:** 8233328
**Reuse map:** none (no survey.md); the brief and handoff-A.md name the analogues
**Verdict:** PASS

## Summary
F extended the analogous objects and built no parallel path. The Tart fix reuses the fake's existing `always-busy` branch plus `FakeTart::flag` and a `clear` helper beside it. The API fix puts the guard in one shared helper (`tests/support/api_process.rs`, included with `#[path]` by both binaries, after the `openapi_schema.rs` precedent). The old `wait_until_healthy`, `SESSION_TOKEN` and the fleet copy of the bind/health loop are removed, so no second copy remains. Only test code changed.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `crates/gridops-tart-agent/src/main.rs` (FAKE_TART logger, `FakeTart::logged`) | The fake's per-entry log files are a heavier mechanism than the brief's expected shape. It is still an extension of the existing `calls` / `paths` logs (same names, same readers `calls()` / `logged_paths()`), so not a parallel path. It is justified in handoff-F by the sweep and by measured stress results. | None needed. |
| 2 | warn | `crates/gridops-api/tests/control_plane_contracts.rs:164,207` | `session_cookie` (instance) and `session_cookie_for` (free fn) both exist. They split the work (own token vs token given at seed time), so not duplicates. | Optional: have `session_cookie` call `session_cookie_for`. |

Checks that passed:
- One `pub fn` in the shared helper, used by both binaries (no `dead_code`, no blanket allow).
- Both A-plan constraints honoured: `restart` offers one fixed port and cannot move; the DB is seeded once before any attempt; fleet `Fixture` constructors and `cookie: String` field kept (#134); the log-secret assertion now uses the fixture's own token.
- Sweep sites (`gitlab_runner_script.rs:675`, `tart_outage_tests.rs refused_url()`) reuse the existing discard-port convention (`127.0.0.1:9`) rather than adding a third way.
- No drive-by edits of shared or other-agent files; no production code, dependencies or contracts changed.
- Regression tests live in one binary each, next to the fixture they exercise.

## Notes for F
None; no block.
