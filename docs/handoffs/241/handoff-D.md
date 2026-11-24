# Handoff-D: Phase 2 - Design (#241, fleet health wave-1 review fixes)

**Date:** 2026-11-24
**Branch:** fh-241 worktree (base master 73de941)
**Mode:** (a) generated low-fi
**Wireframe:** docs/handoffs/241/wireframe.html

## Screens & states covered
One page, `/fleet/health`; seven sections, one per brief item. Page-level states (loading, forbidden, error, stale callout) are unchanged and not redrawn.

1. **Areas of concern, many (one of each severity):** distinct icon shape per severity plus visually hidden "Critical" / "Warning" / "Info" text inside the row, ahead of the title. Critical is a filled red circle with a cross (`failure` glyph). Warning is an amber outlined circle with clock hands (`waiting` glyph). Info is a grey outlined circle with a slash (`skipped` glyph). Empty state ("No areas of concern" with a green check) is unchanged.
2. **Same-page concern link (behaviour only):** a hash change on the same route expands and scrolls to the matching machine row. Cases covered: unknown host (no-op), hash cleared (leave expansion alone), user collapses by hand (stays collapsed until the hash changes), initial deep link (unchanged).
3. **Fleet score card, weights:** rounded whole percent; "—" for a component with no score (the API gives such a component weight 0 after redistribution).
4. **Fleet score card, no-data reason:** the tooltip and the in-row sentence show `component.deduction` when the score is null. If `deduction` is null they fall back to the existing generic text.
5. **Machine detail, CPU and memory:** Recharts horizontal stacked `BarChart` per resource, with the existing label line kept. Four non-overlapping segments:
   - used within reservation: `min(used, reserved)`
   - used over reservation (warning hatch): `max(used - reserved, 0)`
   - reserved, not used: `max(reserved - used, 0)`
   - free budget: `max(budget - max(reserved, used), 0)`

   States drawn: used below reserved, used above reserved (the bug case), used null (reserved plus free only), and demand above budget (no free segment).
6. **Machine detail slot:** the placeholder is removed in every state. The slot wrapper stays in the DOM and renders nothing.
7. **Live strip, oldest wait:** `max(waitMs, Date.now() - queuedAt)` through the existing `formatOldestWait`. States drawn: fresh, cached/stale (shows 8 min where waitMs says 45 s), and queue clear. Clock skew falls back to `waitMs`.

## Existing components/patterns reused
- `src/components/status-icon.tsx` glyph vocabulary (`failure`, `waiting`, `skipped`) for severity. A small `SeverityIcon` export reusing those paths is suggested, with no new SVG geometry.
- The existing row layout, `Callout`/`Tooltip`, `formatOldestWait`, the `--info`/`--warning`/`--danger`/`TRACK_COLOR` tokens and the overlay bar's colour language (info solid and info/45).
- Recharts only, already a dependency (`LineChart` in the score card); no new dependency.
- The `data-health-section` slot names (design doc: slots keep their names).

## Open questions for approval
None blocking. Calls made that the operator may overrule:
- **Info icon:** the grey slash circle reads as "skipped" elsewhere in `status-icon.tsx`. The alternative is a new "i" glyph, which means new geometry. I chose reuse.
- **Item 6:** I chose "hide" rather than "name the expanded machine". The table footer already carries the hint, and naming the machine would mean lifting the expansion state into `page.tsx`.
- **Item 5:** the brief's "(reserved, used, budget)" is realised as non-overlapping segments (see above) so reserved is always visible as a width. The chart gets `role="img"` and an `aria-label` carrying the numbers.
- **Item 7:** the wait is computed at render with no new timer; it updates on each refresh or re-render.

## Verification
Rendered headlessly with Chrome to a scratchpad PNG and looked at it. All seven sections, the severity glyphs (`✕` `◷` `⊘`) and the stacked-bar segments render complete and on-canvas. Icons are Unicode stand-ins at low-fi, not production glyphs. I did not zoom in on the small severity glyphs beyond the full-page render.

## Approval
[To be filled by O: "Approved by operator <ISO timestamp>" - D does not self-approve.]
