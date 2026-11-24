# Handoff-F: Phase 6 - Fleet health wave-1 review fixes (#241)

**Date:** 2026-11-24
**Branch:** claude/fh-241 (worktree fh-241, on top of 3772d77 = T-red PASS)
**Issue:** #241

## What was done
- `src/features/fleet-health/areas-of-concern.tsx`: item 1. `SEVERITY_TONES` + `StatusDot` are replaced by a `Record<Severity, RunState>` map (critical `failure`, warning `waiting`, info `skipped`) rendered through `RunStatusIcon`, plus a `.sr-only` "Critical" / "Warning" / "Info" span in the row (the `StatusBadge` icon-only pattern). Nothing was added to `status-icon.tsx`.
- `src/features/fleet-health/fleet-score-card.tsx`: items 3 and 4. The weight renders `` `${Math.round(weight)}%` ``, or exactly `—` when `score === null`. A null-score row's reason line and tooltip show `component.deduction`, falling back to `INSUFFICIENT_REASON` when it is null or empty.
- `src/features/fleet-health/machine-score-table.tsx`: item 4's sibling (bar `title` is `<label>: <deduction>`, falling back to `<label>: not enough data`). Item 2/6: optional `expanded` / `onExpandedChange` / `hash` props. When they are passed (`expanded !== undefined`) the table renders from them and runs no hash adjustment of its own. With only `{ data, window }` it keeps its own state and its `seenHash` adjustment, as before. The one scroll effect keys on the router `hash` when given, otherwise on `seenHash`. Stale comments updated (A warn 4).
- `src/features/fleet-health/machine-format.tsx`: `hashHostId(hash = globalThis.location?.hash ?? "")` now takes the hash and accepts it with or without the leading `#` (`/^#?machine-(.+)$/`). It is still the only `#machine-` parser.
- `src/features/fleet-health/page.tsx`: item 2/6. `HealthSections` reads `useRouterState({ select: (state) => state.location.hash })`, holds the expansion and the last hash it saw, and on a hash change expands `hashHostId(hash)` while rendering (no state set in an effect). It passes `expanded` / `onExpandedChange` / `hash` to the table and `expandedId` to the detail slot. The frame's doc comment now says so.
- `src/features/fleet-health/machine-detail.tsx`: item 5. New exported pure `resourceSegments({ budget, reserved, used }) => { usedWithin, usedOver, reservedUnused, free, scale }` (T's named contract). `ResourceBar` is now a Recharts horizontal stacked `BarChart` (the live strip's structure) of those four segments: used in `var(--info)`, used past the reservation as a hatch in `<defs>` (`var(--warning)` / `var(--panel)`, id from `useId()`), reserved but unused in `var(--info)` at 0.45 opacity, free budget in `var(--selected)`. The chart wrapper is `role="img"` with an `aria-label` repeating the label line. Item 6: `MachineDetail` takes `expandedId?: string | null`; with no `host` it names that machine ("Showing **<name>**: what it is doing now and what it ran in the window are in its row above.") or shows the unchanged prompt when the id is absent or not in `data.hosts`.
- `src/features/fleet-health/live-strip.tsx`: item 7. New exported pure `oldestWaitMs(oldest, now = Date.now()) = max(waitMs, now - queuedAt)`, used for the "Oldest job waited" text. `formatOldestWait` is unchanged.

## Design decisions
- **Controlled vs. uncontrolled table.** A warn 2's shape exactly: the mode is decided by `expanded !== undefined`. In controlled mode the toggle calls `onExpandedChange(expanded === id ? null : id)` with the current render's value, not a functional updater, since the parent owns the state. The alternative, always lifting the state and making the props required, would break the #202 contract test, which renders `{ data, window }` only.
- **Hash adjustment in `HealthSections`, not the child.** The brief requires this: a child that set the parent's state during render would be a cross-component render update. The adjustment mirrors the existing table pattern. An unknown host still sets `expanded` to the unknown id, which matches no row and which `MachineDetail` resolves to the prompt. A cleared or non-machine hash leaves the expansion alone, as the wireframe says.
- **Free budget drawn as a segment** (`var(--selected)`, the old track colour) rather than left as empty track, so the four segments the wireframe lists are all drawn. `barCategoryGap={0}` and a `rounded-full overflow-hidden` wrapper keep the old 8 px pill.
- **Hatch drawn** (optional in the brief) so "used over reservation" reads by pattern as well as colour, in the spirit of item 1. Per A warn 5 its id comes from `useId()` (React 19.2's `_r_N_` ids are valid in `url(#…)`). A probe confirmed two distinct ids in one detail.
- **`oldestWaitMs` exported** beside `formatOldestWait`, so it is a named, testable pure helper like `formatElapsed`. The tests pin behaviour only.
- **Reason fallback uses `||`**, so both a null and an empty `deduction` fall back, per site: `INSUFFICIENT_REASON` on the card and "not enough data" in the table.

## Reuse / extend-vs-new
This follows A's per-item Reuse baseline (handoff-A.md). Item 1 extends `RunStatusIcon` with the `StatusBadge` sr-only pattern; no new export or glyph. Item 2 extends `hashHostId` (no second parser), uses `useRouterState({ select })` as `app-shell.tsx:121` does, and moves the existing render-time adjustment into `HealthSections`. There is no `hashchange` listener, store, context or new hook. Items 3 and 4 extend `ComponentRow` and `ComponentBars` in place. Item 5 rebuilds `ResourceBar` in place on the live-strip structure, with the segment function exported from `machine-detail.tsx`. There is no new shared chart component. Item 6 extends `MachineDetail`'s existing placeholder branch with one prop. Item 7 adds one pure helper beside `formatOldestWait`.

## Architecture notes for A
- `MachineScoreTable`'s props are now `FleetHealthViewProps & { expanded?, hash?, onExpandedChange? }`. It is the codebase's first component that works both controlled and uncontrolled, and its doc comment says why.
- `MachineDetail`'s props gain `expandedId?: string | null`.
- `HealthSections` (`page.tsx`) now imports `useRouterState` from `@tanstack/react-router` and owns the machine expansion.
- `hashHostId` gains an optional parameter. Existing zero-argument calls behave as before.
- New exports: `resourceSegments` (`machine-detail.tsx`) and `oldestWaitMs` (`live-strip.tsx`).
- No new dependencies. Recharts is already a dependency, and `machine-detail.tsx` now imports it.
- **For O, design doc (A warn 4):** `docs/design/fleet-health.md:352` still says each section takes `{ data, window }` "using only the router's `Link`". The frame now reads the hash and owns the expansion, and the machine sections still work with `{ data, window }` alone. A left the doc sentence for O to decide; I did not edit the design doc.
- **For O, PR body (A warn 6):** use "Closes #241" as the only closing reference. Mention #195 / #202 / #203 without a closing verb, or `scripts/fleet-health-guard.mjs` will check the diff against a wave-1 owner list.

## Deviations from spec / wireframe
None. The wireframe's open question for item 6 copy ("Showing <name>: …") is followed verbatim. The machine name is in `font-medium text-foreground` rather than bold. The named-machine card has a solid border and the prompt keeps its dashed border, as D drew them.

## Tier 1 self-check (incl. tests now GREEN)
```
$ npx vitest run tests/fleet-health
 Test Files  10 passed (10)
      Tests  66 passed (66)

$ npx vitest run
 Test Files  66 passed (66)
      Tests  701 passed (701)
(no "Cannot update a component while rendering" or other React warnings in the output)

$ npm run lint
> eslint src tests *.config.ts eslint.config.js --max-warnings=0
(exit 0, no findings: react-hooks purity / set-state-in-effect / set-state-in-render clean)

$ npm run build
✓ built in 1.18s
> tsc --noEmit
(exit 0)
```
T's 10 RED tests (handoff-T-red.md table rows 1-11) all pass now, and the 3 guards still pass. A throwaway probe, deleted afterwards and never staged, rendered io's expanded detail. It showed CPU as three segments (used, reserved-unused, free) and memory as three (used, hatched `url(#resource-over-_r_6_)`, free), two `<pattern>`s with distinct ids and only `var(--…)` fills, and `aria-label`s "CPU: 9 cores reserved · 7.4 cores used of 22.8 cores" / "Memory: 18 GB reserved · 20.5 GB used of 64 GB". T-red's hedge (that the DOM test can't tell real segments from raw reserved + used) is answered by the chart's data: it is `resourceSegments(...)`, with `scale` as the axis domain.

## Evidence appendix
`docs/handoffs/241/evidence.md` (8 entries: `RunStatusIcon` glyphs, the `StatusBadge` sr-only pattern, router-core's hash shape, the `useRouterState` precedent, the page test's non-empty-slot rule, the score component and oldest-queued schemas, and the live-strip chart structure).

## Tests that look wrong (for T)
None.

## Known issues
None against the acceptance criteria. The scope cap was weighed: 7 production files, but one page and one feature directory, each item a few lines, and the brief, A and T had already sized and gated it as one unit. Splitting after RED would have stranded T's suite. Visual and WCAG checks of the real glyphs and the stacked bar in the built UI are U's (Tier 3).

## Files changed
- src/features/fleet-health/areas-of-concern.tsx
- src/features/fleet-health/fleet-score-card.tsx
- src/features/fleet-health/live-strip.tsx
- src/features/fleet-health/machine-detail.tsx
- src/features/fleet-health/machine-format.tsx
- src/features/fleet-health/machine-score-table.tsx
- src/features/fleet-health/page.tsx
