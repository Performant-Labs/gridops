## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** tests/fleet-health-202-detail.test.ts:170 — the claim that the fixture's concern list contains an in-page Link to `/fleet/health#machine-<callisto>` (and that in-page machine links sit past the first five concerns), which the rewritten item-2 test clicks — the evidence that would settle it is `tests/fixtures/…/fixture.json`'s `concerns[].link` entries (not in this prompt).
- **[NV-2]** tests/fleet-health-241-sections.test.ts:18 — the claim that the fixture has all three severities so the item-1 test covers each glyph — `fixture.json`'s `concerns[].severity` set (the brief asserts it only via the design doc's Decisions, which is not shown).
- **[NV-3]** tests/fleet-health-203-live.test.ts:29-36 — the claim that pinning `Date.now()` to `data.generatedAt` keeps "7 min" stable, i.e. that `oldestQueued.waitMs === generatedAt - queuedAt` in the fixture — `fixture.json`'s `live.oldestQueued.queuedAt`/`waitMs` and `generatedAt`.
- **[NV-4]** src/features/fleet-health/page.tsx:130 — the claim that a same-route hash-only navigation causes `useRouterState({ select })` to re-render `HealthSections` (the whole premise of item 2's fix) — the installed `@tanstack/react-router`/`@tanstack/router-core` source for `useRouterState` and its store subscription (published source not shown in this prompt; only `location.hash`'s shape is evidenced).
- **[NV-5]** src/features/fleet-health/live-strip.tsx:104-106 — the claim that a defaulted `now = Date.now()` parameter passes `react-hooks/purity` at error severity — the installed `eslint-plugin-react-hooks` 7.1.1 rule source for the `purity` rule's handling of default parameters.
- **[NV-6]** tests/fleet-health-202-detail.test.ts:17 and tests/fleet-health-241-sections.test.ts:21 — the claims that `./fleet-health-dom` exports `button`, `settle` and that nothing else rendered in 202-detail imports `@tanstack/react-router` once the mock is removed — `tests/fleet-health-dom.ts` (not excerpted).

### WARN findings
- **[W-1]** src/features/fleet-health/machine-score-table.tsx:46 — in controlled mode, if `expanded` is passed but `onExpandedChange` is omitted, every toggle is a silent no-op (`onExpandedChange?.(next)`), with no guard or warning — make `onExpandedChange` required when `expanded` is defined (type-level), or assert it in dev.
- **[W-2]** src/features/fleet-health/machine-score-table.tsx:30-31 — the uncontrolled `seenHash` adjustment still runs its `setSeenHash` on a hash change even when `hashHostId()` yields null, which is harmless but re-renders; the parent's version (`page.tsx:131-135`) has the same shape. No action needed; noting the parity so the two don't drift.
- **[W-3]** tests/fleet-health-241-sections.test.ts:185 — `vi.stubGlobal("ResizeObserver", …)` is never restored (no `vi.unstubAllGlobals()` / `onTestFinished`), so it leaks to the item-7 tests later in the file — restore it after the chart test.
- **[W-4]** docs/design/fleet-health.md:352 — the shown excerpt still says every section takes `{ data, window }` "using only the router's `Link`", which the diff changes for the frame (it now reads the hash and owns expansion). The handoff defers this to O; the doc should be updated before merge or the deferral recorded in the PR body.

### NIT findings
- **[NIT-1]** src/features/fleet-health/machine-detail.tsx:103 — `resourceSegments` has no explicit return type; the test file re-declares the shape (`Segments`) locally, so a drift in the export would fail as a runtime mismatch rather than a type error.
- **[NIT-2]** tests/fleet-health-241-sections.test.ts:185-188 — reading `resourceSegments` through `machineDetail as Record<string, unknown>` rather than a named import loses type checking of the function's signature for no clear gain over `expect(typeof machineDetail.resourceSegments).toBe("function")` with a typed import.

### Verdict
PASS — no BLOCK findings; testing may proceed.