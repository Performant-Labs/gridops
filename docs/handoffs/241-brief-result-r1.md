## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** Claim that `StatusDot` is `aria-hidden` and the row lacks severity text in `areas-of-concern.tsx` (~:85) — the actual source of `web/src/features/fleet-health/areas-of-concern.tsx` around line 85 (and the `StatusDot` component definition).
**[NV-2]** Claim that a same-route TanStack Router hash change does not re-render `machine-score-table.tsx` (~:25-31), so the hash is only read on re-render — an excerpt of `machine-score-table.tsx` lines ~1-60 and TanStack Router documentation/source confirming hash changes outside the subscribed state do not trigger re-renders.
**[NV-3]** Claim that `fleet-score-card.tsx` (~:126) renders raw float weights like "47.05882352941177%" and hardcodes the "not enough data" tooltip (~:13, :135, :140) ignoring `component.deduction` — excerpts of `fleet-score-card.tsx` at those lines, and the API type showing a `deduction` field exists on the component payload.
**[NV-4]** Claim that `machine-detail.tsx` draws "used" over "reserved" so reserved disappears (~:93-114) and that the placeholder text is unconditional (~:15-23) — excerpt of `machine-detail.tsx`.
**[NV-5]** Claim that `live-strip.tsx` (~:97) shows a stale `oldest.waitMs` fixed at response-build time rather than time since `queuedAt` — excerpt of `live-strip.tsx` and the strip payload type showing `queuedAt` is available client-side.
**[NV-6]** Claim that `tests/fleet-health-202-detail.test.ts` (~:145-152) re-renders manually via `render()` and thereby masks the hash-navigation bug — the test file excerpt.
**[NV-7]** Claim that issue #197 has a router mock that can be extended — the mock's location and current API (needed to judge the "in scope" statement's feasibility, though not blocking).

### WARN findings
**[W-1]** Item 7's fix "Show `max(waitMs, Date.now() - queuedAt)`" mixes a snapshot value with a live computation but the brief doesn't say whether the value should tick/refresh periodically; if computed once at render, it goes stale again. Recommend the brief state the refresh model (effect + interval, or recompute on re-render cadence) and a matching test.
**[W-2]** Item 5 says "as the issue asked" for a stacked BarChart but no excerpt of the original issue text is provided; if the bar order/segment semantics (reserved, used, budget) matter, cite the original issue so the diff gate can check conformance. Also "Recharts only, no new dependency" presumes Recharts is already a dependency — not shown here.
**[W-3]** Item 3's "show '—' for a component with no data" and item 4's "show `component.deduction` when the score is null" conflate "no data" and "null score" — the brief should state whether these are the same condition or distinct states, since a test "fails before the fix" needs the distinction pinned.
**[W-4]** Item 2's test rewrite requires failing before the fix via a real `RouterProvider`; the "Done when" criterion "each item has a test that fails before the fix" is testable only at diff time, which is fine, but item 6 ("Hide it, or name the expanded machine") is a disjunctive fix with no stated acceptance for which branch is chosen — pick one so the criterion is unambiguous.

### NIT findings
**[NIT-N]** Line references use "~" and "master at 73de941"; fine now, but the diff gate will need exact anchors — consider requiring the PR to re-verify them against the PR base.

### Verdict
PASS — no BLOCK findings; implementation may proceed.