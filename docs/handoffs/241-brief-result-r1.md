## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** Item 1's claims about `status-icon.tsx` (`RunStatusIcon` at :53, `RUN_STATES` keys, `aria-hidden` SVG at :55, `StatusBadge`'s `sr-only` label at :89, `StatusDot` aria-hidden at :78) — the file is not in the SOURCE EXCERPTS; the brief's "verified verbatim" prose is the author's assertion, not shown text. Evidence: excerpts of `src/components/status-icon.tsx` lines 1–91.
- **[NV-2]** Item 1's claims about `areas-of-concern.tsx` (`StatusDot` at :85, `SEVERITY_TONES` at :11, the :44-51 state-adjustment pattern) and that the fixture covers every severity — evidence: excerpts of `src/features/fleet-health/areas-of-concern.tsx` and the concern section of `fixture.json`.
- **[NV-3]** Item 2's claims about `machine-score-table.tsx` (:19, :21, :25-31, :34-38) and `machine-format.tsx` (`hashHostId` at :48-56) — evidence: excerpts of those files.
- **[NV-4]** Item 2's claims about TanStack Router behaviour: `@tanstack/router-core` 1.171.15 `location.hash` excluding the leading `#` and being passed through `decodePath` (`src/location.ts`, `src/router.ts` `parseLocation`, `src/utils.ts`), and that a same-route hash navigation does not re-render `MachineScoreTable` — evidence: the cited library source excerpts or TanStack Router documentation; also `package-lock.json`'s pinned version.
- **[NV-5]** Item 2/7's claim that `eslint-plugin-react-hooks` 7.1.1's recommended preset sets `purity`, `set-state-in-effect` and `set-state-in-render` to error (read from the published build, `node_modules` absent) — evidence: the plugin's published `cjs/eslint-plugin-react-hooks.development.js` preset definitions.
- **[NV-6]** Item 3/4's claims about `fleet-score-card.tsx` (:13, :113, :126, :134-142), `types.ts` (:44-50, :110-116), `category-legend.tsx:37`, and `fixture.json`'s weights/null-score components — evidence: excerpts of those files.
- **[NV-7]** Item 5's claims about `machine-detail.tsx:93-115` (`ResourceBar`, both spans absolutely positioned at the left edge) and `live-strip.tsx:33-43` (stacked `BarChart` structure) — evidence: excerpts of those files.
- **[NV-8]** Item 6's claims about `machine-detail.tsx:14-26` (placeholder text, `data-machine-detail`) and `page.tsx`/`HealthSections` structure — evidence: excerpts of `machine-detail.tsx` and `page.tsx`.
- **[NV-9]** Item 2's claim that today's mock in `tests/fleet-health-page.test.ts` (:21-27) is the same Link-only factory — that file is not excerpted. Evidence: excerpt of `tests/fleet-health-page.test.ts` lines 1–45.
- **[NV-10]** Item 2's claim about Vitest throwing when code reads an export a factory mock does not define (Vitest 4.1.10) — evidence: Vitest documentation on module mocks / the pinned version's source.

### WARN findings
- **[W-1]** Item 5's hatch criterion is conditional ("Without a hatch there is nothing to assert for it"). "Hatching is optional" leaves an unfalsifiable branch: an implementer can skip the hatch and the fill-var assertion never runs. Recommendation: either decide the hatch (in or out) in the brief, or make the criterion "if any `<pattern>` exists inside the chart's `<defs>`, every `fill`/`stroke` is a `var(--…)` value" so it is always executable against whatever is built.
- **[W-2]** Item 2's dual-path design (child keeps its own `seenHash`/`globalThis.location.hash` adjustment while `HealthSections` passes router-hash-driven expansion props) means the same machine can be expanded by two independent sources with subtly different hash encodings (router hash has no `#` and is `decodePath`-ed; `globalThis.location.hash` has `#` and raw). The brief acknowledges this but gives no acceptance criterion covering what happens when both paths act, or which wins. Recommendation: state the precedence, and note that the malformed-hash test only covers the child path.
- **[W-3]** Item 3's "—" (U+2014) for null-score weight and item 4's per-site fallback texts are specified, but no criterion covers a null-score component whose `deduction` is non-null *and* the tooltip/reason line — i.e. the interaction with `title` on `machine-score-table.tsx:120`'s numeric branch is specified while the null branch's title content is only implied. Minor; recommend naming the exact expected `title` string for the null-score machine-table case.

### NIT findings
- **[NIT-1]** "Done when" does not explicitly cover item 7's clock pinning (it is specified in the item body: "Pin the clock in the test") — the criterion is testable, just scattered.
- **[NIT-2]** Item 1's test compares `svg` `innerHTML` for glyph equality; on some jsdom/React versions `innerHTML` serialisation of attributes can vary between the two renders. Safer to compare child element tag names and `class`/`fill` attributes; the brief already restricts the comparison, so this is a robustness nit only.

### Verdict
PASS — no BLOCK findings; implementation may proceed.