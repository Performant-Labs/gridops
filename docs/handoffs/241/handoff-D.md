# Handoff-D: Phase 2 - Design (#241, fleet health wave-1 review fixes), revision 2

**Date:** 2026-11-24
**Branch:** claude/fh-241 (base master 73de941)
**Mode:** (a) generated low-fi (revision 2, after A's BLOCK on item 6 and the amended brief)
**Wireframe:** docs/handoffs/241/wireframe.html

## What changed from revision 1
- **Item 6** now follows the brief's Decided option B: the slot always has content (the prompt, or the expanded machine's name). Revision 1's "hide the placeholder" is gone.
- **Item 1** reuses `RunStatusIcon` through a severity-to-`RunState` map plus `.sr-only` text. The `SeverityIcon` export suggested before is dropped: nothing is added to `status-icon.tsx`.
- **Item 2** now shows the `HealthSections` hash subscription and the render-time expansion. No state is set in an effect, and `hashHostId` takes the hash string.
- **Item 3** says exactly "—" for any component with `score === null`, whatever its weight. The earlier assumption that such a component has weight 0 is removed.
- **Item 4** adds the machine-table bar `title` and its per-site fallback.
- **Item 5** states the optional-hatch rule (theme variables only), the live-strip chart structure to copy, and the pure exported segment function.
- **Item 7** says the wait comes from a pure helper with a `now` parameter.

## Screens & states covered
One page, `/fleet/health`, seven sections, one per brief item. Page-level states (loading, forbidden, error, stale callout) are unchanged and not redrawn.

1. **Areas of concern, many (every severity):** a distinct glyph per severity plus visually hidden "Critical" / "Warning" / "Info" text in the row. critical = `failure` glyph (red), warning = `waiting` (amber), info = `skipped` (grey). The empty state is unchanged.
2. **Same-page concern link (behaviour only):** the hash change expands and scrolls to the machine. Cases drawn: unknown host (no-op), hash cleared or non-machine hash (expansion left alone), malformed hash (no throw), manual collapse (stays collapsed until the hash changes), initial deep link, table rendered alone with `{ data, window }`.
3. **Score card weights:** whole percent through `Math.round` (47.0588... shows "47%", 25 shows "25%"). "—" for `score === null`.
4. **No-data reason:** for a null score, the reason line and tooltip on the score card show `component.deduction`. If it is null or empty they fall back to the card's `INSUFFICIENT_REASON`. The machine-table bar `title` shows `<label>: <deduction>`, falling back to `<label>: not enough data`. The two fallbacks stay separate.
5. **Machine detail CPU and memory:** Recharts horizontal stacked `BarChart` with four non-overlapping segments: used within reservation, used over reservation (optional hatch), reserved not used, free budget. States: used below reserved, used above reserved (the bug case), used null, demand above budget.
6. **Detail slot:** the prompt when nothing is expanded, when the expanded id is not in the data, and when there are no machines (copy unchanged). The machine's name when one is expanded: "Showing <host.name>: ... are in its row above". The full detail still renders only in the row.
7. **Live strip oldest wait:** `max(waitMs, now - queuedAt)`. States: fresh, cached or stale (8 min where `waitMs` says 45 s), queue clear, clock skew (falls back to `waitMs`).

## Existing components/patterns reused
- `RunStatusIcon` and the `StatusBadge` icon-only `sr-only` label pattern (item 1).
- `useRouterState({ select })` as in `app-shell.tsx:121`, the render-time adjust-state pattern, and `hashHostId` (extended, not a second parser) (item 2).
- `ComponentRow` and `INSUFFICIENT_REASON`; `ComponentBars` title in the machine table (items 3 and 4).
- `live-strip.tsx`'s stacked `BarChart` structure, and exported pure helpers like `usagePoints` and `slowestKeys` (item 5).
- The existing `MachineDetail` placeholder card and the `data-health-section` slots (item 6).
- `formatOldestWait` and `formatElapsed(..., now = Date.now())` (item 7).

## Open questions for approval
None blocking. Calls the operator may overrule:
- **Item 6 wording and shape:** the brief says "name the expanded machine" without copy. I drew the same dashed card with solid border, the heading "Machine detail" and "Showing **mac-mini-02**: what it is doing now and what it ran in the window are in its row above." The slot does not repeat the detail. It needs a new prop on `MachineDetail`, not the existing `host`.
- **Item 6 fallback:** an expanded id that is not in the data (unknown hash host) shows the prompt, since there is no name.
- **Item 5:** the brief's "(reserved, used, budget)" is realised as non-overlapping segments so reserved stays visible as a width. The chart gets `role="img"` with an `aria-label` carrying the numbers. The hatch is optional.
- **Item 1:** the grey slash glyph (`skipped`) stands for info, as the brief specifies.

## Verification
Rendered headlessly with Chrome to PNG and looked at it, in full and at 2x on the header and item 1. All seven sections lay out on-canvas, and the three severity glyphs and segment bars are complete. The Unicode glyphs in the wireframe are stand-ins. The real glyphs come from `RunStatusIcon` at build time, so U must check them in the built UI. The wireframe's literal hatch colours are drawing-only: production uses `var(--…)`.

## Approval
[To be filled by O: "Approved by operator <ISO timestamp>" - D does not self-approve.]
