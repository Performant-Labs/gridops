# Handoff-A-dup: Phase 7 - Fleet health wave-1 review fixes (#241)  (anti-duplication gate)

**Date:** 2026-11-24
**Branch:** claude/fh-241
**Diff base:** 73de941   **Diff head:** fd1710f
**Reuse map:** none for this run; the per-item baseline in docs/handoffs/241/handoff-A.md stands in
**Verdict:** PASS

## Summary
F extended the object each item named; no parallel path. Item 1 reuses `RunStatusIcon` with the `StatusBadge` sr-only pattern and adds nothing to `status-icon.tsx`. Item 2 extends `hashHostId` (still the only `#machine-` parser) and uses the `useRouterState({ select })` pattern from `app-shell.tsx:121`; no new listener, store or context. Items 3, 4, 6, 7 are in-place edits (one pure helper beside `formatOldestWait`). Item 5 rebuilds `ResourceBar` in place on the live-strip `BarChart` structure with an exported pure segment function; no shared chart component was invented, and the `<pattern>` is the only one in src.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `src/features/fleet-health/machine-score-table.tsx:30-56` | The table is the first controlled/uncontrolled component, and its uncontrolled hash adjustment duplicates the one now in `page.tsx:131-136`. The brief requires both (the #202 contract test renders `{ data, window }` only), so this is deliberate, not drift. If `onExpandedChange` is omitted while `expanded` is set, toggles are silent no-ops. | Keep as is. Consider a type-level pairing of `expanded` and `onExpandedChange`. |
| 2 | warn | `tests/fleet-health-router.ts:1` | New shared router test helper, where `page-header-nav.test.ts` and `app-shell-nav.test.ts` inline their own router mounts. It differs on purpose (browser history, so jsdom's location follows navigation) and is used by two files. | None needed; fold into `fleet-health-dom.ts` only if a third user appears. |
| 3 | warn | `docs/design/fleet-health.md:352` | The design doc still says sections use "only the router's Link"; the frame now reads the hash. F deferred it to O. | O to update the sentence or record the deferral in the PR body. |

No duplication; extension is clean. No unplanned edits to shared or other-agent-owned code (`status-icon.tsx`, the #202 contract test and `tests/fleet-health-page.test.ts` beyond the ruled mock export are untouched or minimal).

## Notes for F
None required (PASS).
