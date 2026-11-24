## Brief Review (Round 1)

### BLOCK findings
**[B-1]** Item 2 and Item 6 contradict each other on where hash-driven expansion happens. Item 2 (Decided) says: "call `useRouterState(...)` in `HealthSections` ... and pass the hash down as an optional prop" and "Expand through the render-time `seenHash` adjustment **already in `machine-score-table.tsx`** ... not by setting state in an effect." Item 6 (Decided, option B) says: "Move the expansion state up into `HealthSections` (`page.tsx`)". If expansion state lives in the parent, the child's `machine-score-table.tsx` render-time `seenHash` adjustment cannot expand the row: modifying parent state from a child's render body is a side effect during render (precisely the class of thing the repo's `react-hooks/purity` / `set-state-in-effect` discipline guards), and the brief forbids the effect path. The brief must pick one coherent mechanism — e.g. the hash-triggered render-time adjustment moves into `HealthSections` alongside the lifted expansion state, with the hash no longer "passed down" for expansion, or the child invokes a render-safe `onExpand` callback whose purity implications are addressed — and say what happens to the existing child-side `seenHash` code. As written, an implementer cannot satisfy both Decided clauses.

### Needs-verification findings
**[NV-1]** Item 2's claim that the router's `location.hash` has no leading `#`, and that `useRouterState({ select: s => s.location.hash })` is the pattern used at `app-shell.tsx:121` — a TanStack Router source excerpt or the shown `app-shell.tsx:121` line would settle it.
**[NV-2]** Item 2's claim that putting the hook in `MachineScoreTable` "breaks the Link-only `vi.mock` in the `*.contract.test.ts` file" — the `machine-score-table` contract test's mock block, shown, would settle it.
**[NV-3]** Item 1's assumption that a severity-to-`RunState` map (failure, waiting, skipped) plus visually hidden text is sufficient, and that `StatusBadge`/`RunStatusIcon` behave as described — the shown source of `status-icon.tsx` and `StatusBadge` would settle it.
**[NV-4]** Item 5's claim that `live-strip.tsx` contains a stacked `BarChart` structure worth copying — the shown excerpt of `live-strip.tsx` would settle it.
**[NV-5]** Item 7's claim that `formatElapsed` is a pure helper whose `now` defaults to `Date.now()` and that the repo treats `react-hooks/purity` as an error — the shown `live-strip.tsx` helper and the repo's eslint config would settle it.
**[NV-6]** Item 4's claim that `component.deduction` exists and is populated by the API whenever the score is null — the fleet-health API type/response shape (or a shown excerpt of the component type) would settle it.

### WARN findings
**[W-1]** Item 4 has no defined fallback when the score is null *and* `component.deduction` is absent/empty. "Show `component.deduction` when the score is null" is not a complete spec; state what renders in that case (presumably the existing fixed text). This affects both `fleet-score-card.tsx` and `machine-score-table.tsx` (~:120).
**[W-2]** Item 3's "show `—` for a component with no data" and Item 4's null-score case may overlap (a component with no data plausibly has a null score); the brief should say which rendering wins so the two fixes don't conflict in the tooltip/weight paths.
**[W-3]** "Each item has a test that fails before the fix" is the only per-item acceptance criterion. Item 1's accessibility fix ("severity is not colour-only") has no stated *how* of testing — e.g. an assertion on the visually hidden text and the mapped icon — so it risks a test that passes on colour styling alone. Name the observable assertion.

### NIT findings
**[NIT-N]** "New tests go in `tests/fleet-health-241-*.test.ts`" conflicts silently with Item 2's instruction to *rewrite* `tests/fleet-health-202-detail.test.ts` and Item 6's instruction to *update* the same file. The "new tests" rule should be scoped to net-new coverage so it isn't read as forbidding the required edits.

### Verdict
BLOCK — 1 blocking finding; the item-2 / item-6 mechanism for hash-driven expansion must be reconciled before implementation starts.