
## Phase 2 (Design) - D

- **Decided:** Wireframe is a single low-fi HTML file with one section per brief item. Item 6 hides the placeholder (slot wrapper kept, empty). Item 5 uses four non-overlapping stacked segments so reserved is always visible.
- **Assumed:** A component with no score has weight 0 from the API's redistribution, so "—" is shown for it. The `skipped` glyph is acceptable for the info severity.
- **Hedged:** The wait in item 7 is computed at render with no timer. If a live-ticking wait is wanted, that is a follow-up.
- **Evidence:** `src/features/fleet-health/{areas-of-concern,fleet-score-card,machine-detail,machine-score-table,live-strip,page}.tsx`, `src/components/status-icon.tsx`, `docs/design/fleet-health.md`. The wireframe was rendered headlessly and looked at.
