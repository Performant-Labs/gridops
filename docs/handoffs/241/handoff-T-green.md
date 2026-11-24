# Handoff-T-green: Phase 7 - Fleet health wave-1 review fixes (#241)

**Date:** 2026-11-24
**Branch:** claude/fh-241 (head 6b7ca49 = F's implementation)
**Issue:** #241
**Handoff-F reviewed:** docs/handoffs/241/handoff-F.md
**Handoff-T-red:** docs/handoffs/241/handoff-T-red.md

## GREEN confirmation
- `npx vitest run`: Test Files 66 passed (66), Tests 701 passed (701). F reported the same numbers.
- `npx vitest run tests/fleet-health`: 10 files, 66 tests passed. Same as F.
- All 10 RED tests and the 3 guards pass. No React warnings ("Cannot update a component while rendering", act) appear in the full-suite output.
- Mutation spot-check (temporary, reverted with `git checkout`; worktree clean afterwards): (a) `oldestWaitMs` replaced by `waitMs` fails the item 7 test; (b) removing the `.sr-only` severity span fails the item 1 test; (c) making `HealthSections` ignore the hash fails the 202-detail in-page concern-link test. 3 failed, 63 passed, each for the intended reason, so the tests pin behaviour.
- T-red's hedge (item 5's DOM test cannot tell real segments from raw reserved + used): resolved by reading the source. `ResourceBar` calls `resourceSegments(...)` and draws four stacked `Bar`s (used, hatched over-reservation, reserved-unused, free) from its output, with `useId()` for the hatch id and `var(--…)` fills only. The pure function itself is pinned by test 6.

## Tier 1 results
| Check | Result |
|---|---|
| `npx vitest run` | PASS, 701/701 |
| `npm run lint` (eslint, `--max-warnings=0`) | PASS, exit 0 |
| `npm run build` (vite build, `tsc --noEmit`) | PASS, exit 0 |
| API smoke / server start | N/A: web-only change, no endpoint touched; the page is exercised under a real `RouterProvider` by tests 9-13 |

## Tier 2 results
- Coverage per criterion: PASS (table below).
- Test quality: PASS. Each test names one behaviour; guards (4, 12, 13) sit beside the RED test of their item and protect fallback/malformed-hash behaviour. No duplicated tier: pure logic is unit-tested (6), the rest at component or page level. The suite is proportionate (2 new files, 1 helper, 3 small edits).
- Type safety: PASS. `tsc --noEmit` clean; no `any`, `as unknown` or `@ts-` added in the `src` diff.
- Error handling: PASS. Unknown and malformed hashes (12, 13), null/empty deduction fallback (4, 5), clock skew clamp (8), null `used` and over-budget segments (6).
- Data integrity / migrations / DB: N/A (web only).
- API contract: PASS. No API shape change; `MachineScoreTable`'s `{ data, window }`-only use still works (#202 contract test untouched and green).
- Security: N/A (no input reaches a sink; the hash is only matched against known host ids).
- Playwright: the project has no Playwright suite. Headless-only; real glyph, hatch and stacked-bar rendering is a Tier 3 / U concern.
- Evidence appendix: F's `evidence.md` entries cover the unchanged facts the tests rely on (`RunStatusIcon` glyphs, `StatusBadge` sr-only, router-core hash shape, `useRouterState` precedent). Nothing to append.

## Acceptance criteria status
1. Severity not by colour alone: PASS (test 1).
2. Concern link opens the machine in-page: PASS (tests 10, 11; guards 12, 13).
3. Weight as whole percent / dash: PASS (test 2).
4. API reason shown with fallback: PASS (tests 3, 4, 5).
5. Resource bar segments do not overlap: PASS (tests 6, 7).
6. Detail slot names the expanded machine: PASS (tests 9, 11).
7. Oldest wait read against the clock: PASS (test 8, plus the pinned clock in the 203 test).

## Blocking issues
None. F reported no tests that look wrong; none changed in this phase.

## Advisory notes
- For O: `docs/design/fleet-health.md:352` still says each section takes `{ data, window }` "using only the router's `Link`" (A warn 4). PR body should use "Closes #241" only and no closing verb before #195/#202/#203 (A warn 6).
- U/S: this change touches a UI surface; walk the severity glyphs, the stacked resource bars (hatch) and the in-page concern link live.
