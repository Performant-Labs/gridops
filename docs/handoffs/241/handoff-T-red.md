# Handoff-T-red: Phase 4 - Fleet health wave-1 review fixes (#241)

**Date:** 2026-11-24
**Branch:** claude/fh-241 (head 15e650a = master 73de941 + brief, D revision 2, A PASS)
**Brief / wireframe reviewed:** docs/handoffs/241-brief.md (amended), docs/handoffs/241/wireframe.html (revision 2), docs/handoffs/241/handoff-D.md, docs/handoffs/241/handoff-A.md

## A precondition
Confirmed: A returned PASS on the plan (Phase 3 re-review, 2026-11-24T02:57:59-08:00), with six warnings. Warnings 1 (harness), 3 (no `host` overload), 4 (stale comments) and 5 (hatch `useId`) affect the tests and are applied below. D's Approval line in handoff-D.md is still empty; O fills it in. It does not block RED.

## Files authored or changed (all test-side; no production code touched)

| File | Change |
|---|---|
| `tests/fleet-health-241-sections.test.ts` | **New.** Items 1, 3, 4, 5, 7 (router mocked with `Link` only) |
| `tests/fleet-health-241-router.test.ts` | **New.** Items 2 and 6 on the page path: deep link on load, plus two guards (unknown host, malformed hash) |
| `tests/fleet-health-router.ts` | **New helper** (not collected: not `*.test.ts`). Mounts `FleetHealthPage` under a real `createRouter`/`RouterProvider` on a **browser** history and stubs the health GET |
| `tests/fleet-health-202-detail.test.ts` | The brief's two allowed edits: rewrote the in-page link test (was :145-152) and updated the slot test (was :129-133). Deleted the file's `Link`-only `vi.mock` (A warn 1). Nothing else this file renders imports the router. `MachineDetail` import dropped (the slot is now read through the page) |
| `tests/fleet-health-page.test.ts` | The ruled scaffolding only: the mock gains `useRouterState` applying `select` to `{ location: { hash: "" } }`. No assertion added, removed or changed |
| `tests/fleet-health-203-live.test.ts` | Clock pinned (`vi.useFakeTimers({ now: data.generatedAt, toFake: ["Date"] })`) in "links the oldest queued job's wait to the queue". Once item 7 lands, that test's "7 min" would otherwise depend on the real clock. Today's real clock gives "84 h 7 min", which passes only by accident and breaks within the minute. Pinned at `generatedAt`, the clock's wait equals `waitMs` (7 min). Same outcome before and after the fix |

No `*.contract.test.ts` file was touched.

## Tests authored

| # | Test (file › name) | Pins (brief item / criterion) | Tier | Why this tier |
|---|---|---|---|---|
| 1 | 241-sections › item 1 › "reads each row's severity out, and draws its severity's glyph" | Item 1. For every one of the 13 fixture rows: (a) **primary**: the row's `.sr-only` texts are exactly `["Critical"\|"Warning"\|"Info"]` for its severity. (b) **secondary** (commented as pinning the visual severity mapping, not SVG structure): exactly one `svg` in the row, with `innerHTML` equal to bare `RunStatusIcon` with `failure`/`waiting`/`skipped` (`renderToStaticMarkup`) and `aria-hidden="true"` | component (jsdom) | Text announced and glyph drawn are DOM facts; no router needed |
| 2 | 241-sections › items 3+4 › "shows each weight as a whole percent, and a dash for a component with no score" | Item 3. A leaf shows `47%` for weight 47.05882352941177 and `25%` for 25, and the row has no `47.0588`. For `score: null` (weight 27.94…) a leaf shows exactly U+2014 and the row contains no `%` | component | Rendered text |
| 3 | 241-sections › items 3+4 › "gives the API's reason for a component with no score, on its line and in its tooltip" | Item 4, score-card site. The row shows `deduction` and not `INSUFFICIENT_REASON`. Focusing the row opens the Radix tooltip, whose `[role="tooltip"]` text is exactly `deduction` | component | Tooltip opens on focus in jsdom (checked against today's code: it opens and shows `INSUFFICIENT_REASON`) |
| 4 | 241-sections › items 3+4 › "falls back to the card's own text when the API gives no reason" | Item 4 fallback (score card): `deduction` null **and** `""` both show `INSUFFICIENT_REASON`. **Guard, green before the fix** (today's behaviour must survive). Without it, a fix that showed `""` for a null reason would pass | component | — |
| 5 | 241-sections › item 4 › "titles a no-data bar with the API's reason, and falls back to the table's own text" | Item 4, machine-table site. MacBookPro bar `title` is `<label>: <deduction>`. With `deduction` null or `""` it is `<label>: not enough data` (per-site fallback). A scored bar stays `<label>: <score>` | component | `title` attribute |
| 6 | 241-sections › item 5 › "splits reserved, used and budget into segments that never overlap" | Item 5's exported pure function. **Contract named here:** `resourceSegments({ budget, reserved, used })` exported from `machine-detail.tsx`, returning `{ usedWithin, usedOver, reservedUnused, free, scale }` (the wireframe's four segments, plus the axis `scale = max(budget, reserved, used ?? 0, 1)` as today). Five exact cases (under, over, null used, over budget, all zero) plus invariants: `usedWithin + reservedUnused === reserved`, and the four segments sum to `max(budget, reserved, used ?? 0)`. Read via the module namespace so a missing export fails as an assertion | unit | Pure function, as `usagePoints`/`slowestKeys` |
| 7 | 241-sections › item 5 › "draws each resource as one row of stacked, non-overlapping segments" | Item 5 in the DOM: io expanded (memory used 21000 > reserved 18432, the bug case). In `section[aria-label="Now"]`, Recharts bar rectangles group into exactly 2 rows (CPU, memory), whether one chart or two. Each row has ≥2 positive-width segments, each starting where the previous ends. If a hatch `<pattern>` exists: unique ids (A warn 5), and every `fill`/`stroke` is `var(--…)` (or `none`) | component (jsdom + sized `ResizeObserver` stub) | Without a size Recharts draws nothing in jsdom. The stub makes it draw. The row measurement was checked against `live-strip.tsx`'s stacked structure, which F is told to copy: it passes there |
| 8 | 241-sections › item 7 › "shows how long the job has waited by now, never less than the response said" | Item 7, clock pinned with `vi.useFakeTimers({ toFake: ["Date"] })`. When queued 8 min ago with `waitMs` 45 s, the link reads "Oldest job waited 8 min". With clock skew (queued 1 min "in the future") it reads "45 s" | component | Behaviour through render. The helper's existence is enforced by `react-hooks/purity`, so the test does not name it |
| 9 | 202-detail › machine detail slot › "explains itself when no machine is expanded, and names the machine once one is" | Item 6 (option B), via the real page. The slot shows the prompt until callisto's row is expanded. It then contains "callisto" and not the prompt, and `[data-machine-detail]` lists only callisto, so the slot does not repeat the detail. Collapsing brings the prompt back | integration (page under real router) | The slot's content follows the table's expansion: a page-level behaviour. No `MachineDetail` prop name is pinned |
| 10 | 202-detail › deep links › "opens the machine when an in-page concern link is clicked" | Item 2: the brief's prescribed test. Page at `/fleet/health` with no hash. Click "Show all 13", click the real concern `Link` to `/fleet/health#machine-<callisto>`, settle. Then `window.location.hash` is that hash (the navigation happened), the row toggle is `aria-expanded="true"`, and `[data-machine-detail]` lists exactly callisto. No second `render()`, no `router.navigate` | integration (real router) | Only a real router plus a real `Link` click reproduces the same-route navigation |
| 11 | 241-router › "opens a deep-linked machine on load, and the detail slot names it" | Items 2+6 on the page's initial-load path, once expansion is lifted into `HealthSections`. At `/fleet/health#machine-<callisto>` the row is expanded, the detail is callisto's, and the slot names callisto | integration | Covers the controlled path's first render, which a lifted-state refactor can regress |
| 12-13 | 241-router › "expands nothing when the hash names no machine / is malformed, and the slot keeps its prompt" (`it.each`) | Wireframe item 2/6 edge rows, and the brief's "malformed-hash test must keep passing on both paths". At `#machine-not-a-machine` and `#machine-%E0%A4` (which the router leaves undecoded): all 5 rows render, none is expanded, there is no detail, and the slot shows the prompt. **Guards, green before the fix** | integration | The page path's malformed case. The existing table-only malformed test (202-detail) covers the other path, unchanged |

## RED confirmation

Run: `npx vitest run tests/fleet-health-241 tests/fleet-health-202-detail tests/fleet-health-203-live.test.ts tests/fleet-health-page`, giving `Tests 10 failed | 27 passed (37)`, identical on 3 consecutive runs. Full suite: `npx vitest run` gives `Test Files 3 failed | 63 passed (66)`, `Tests 10 failed | 691 passed (701)`. The only failures are the 10 RED tests below. `npx tsc --noEmit` (after `vite build` generated the ignored `routeTree.gen.ts`) is clean. `npx eslint` on the six test files exits 0 with `--max-warnings=0`.

Each failure is an assertion on the behaviour the item fixes. None is an import, setup or type error:

| # | Failing output (verbatim) | Why it is the right reason |
|---|---|---|
| 1 | `AssertionError: row host-offline:3f6c1d2e-…5e04 (critical): expected [] to deeply equal [ 'Critical' ]` | The row has no `.sr-only` severity text, only the `aria-hidden` `StatusDot` |
| 2 | `AssertionError: expected [ 'Label fractional', …(3) ] to include '47%'` | The weight renders `47.05882352941177%` |
| 3 | `AssertionError: expected 'Label no-score15%Not enough data in t…' to contain 'Not enough data: only 12 jobs in this…'` | The fixed `INSUFFICIENT_REASON` is shown instead of `deduction` |
| 5 | `AssertionError: expected 'Availability: not enough data' to be 'Availability: Not enough data: no sam…'` | The machine-table title ignores `deduction` |
| 6 | `AssertionError: machine-detail exports resourceSegments: expected 'undefined' to be 'function'` | No exported segment calculation exists yet |
| 7 | `AssertionError: a stacked bar for CPU and one for memory: expected [] to have a length of 2 but got +0` | The Now section draws overlaid `span`s, not a stacked Recharts bar |
| 8 | `AssertionError: expected 'Oldest job waited 45 s · acme/app · i…' to contain 'Oldest job waited 8 min'` | The stale `waitMs` is shown, not the clock's wait |
| 9 | `AssertionError: expected 'Machine detailExpand a machine above …' to contain 'callisto'` | The slot always shows the prompt |
| 10 | `AssertionError: expected 'false' to be 'true'` (at the toggle's `aria-expanded`, after the preceding `window.location.hash` assertion passed) | The navigation happened, but nothing re-rendered the table, so the row stayed collapsed: exactly the #241 bug. Under a **memory** history the old code would fail too, but only because `window.location` never changes. A **browser** history was chosen so the failure is the real one |
| 11 | `AssertionError: expected 'Machine detailExpand a machine above …' to contain 'callisto'` | The row and detail open (today's path), but the slot does not name the machine |

Tests 4, 12 and 13 pass before the fix. They are guards for behaviour the brief and wireframe require to survive (fallback text, unknown or malformed hash on the new page path), not RED tests. Each sits beside the RED test for its item.

## Ready for F

RED is valid. F may implement against these tests. Notes for F:

- **Item 5 contract:** export `resourceSegments({ budget, reserved, used }) => { usedWithin, usedOver, reservedUnused, free, scale }` from `src/features/fleet-health/machine-detail.tsx`, and draw the four segments (not raw `reserved` and `used`) in one stacked row per resource. If a hatch is drawn, give its `<pattern>` a `useId()` id and theme colours only.
- **Item 6:** the slot must name the machine (`host.name`) while it is expanded, without rendering a `[data-machine-detail]` element. The test does not pin `MachineDetail`'s prop name (A suggests `expandedId`).
- **Item 2:** `tests/fleet-health-202-detail.test.ts` no longer mocks the router. `tests/fleet-health-page.test.ts`'s mock already exports `useRouterState`. The #202 contract test is untouched, so `MachineScoreTable` must still work with only `{ data, window }`.
- Do not edit any test. If one looks wrong, route it back to T.
