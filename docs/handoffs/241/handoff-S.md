# Handoff-S: Phase 9 - Fleet health wave-1 review fixes (#241)

**Date:** 2026-11-24
**Branch:** claude/fh-241 (head a4b1bd5; production code unchanged since F's 6b7ca49)
**Issue:** #241 (brief: docs/handoffs/241-brief.md)
**Handoff-T reviewed:** docs/handoffs/241/handoff-T-green.md (also handoff-T-red.md)
**Handoff-A reviewed:** docs/handoffs/241/handoff-A.md (plan, PASS) and handoff-A-dup.md (anti-duplication, PASS)
**Handoff-F reviewed:** docs/handoffs/241/handoff-F.md (also handoff-U.md, decisions.md, wireframe.html)
**Operator-facing report:** [`handoff-S-report.html`](handoff-S-report.html) (wipe sliders, diffs, composites, per-section table)

## A precondition
Confirmed. A returned PASS on the plan (handoff-A.md, Phase 3 re-review) and PASS on the anti-duplication gate (handoff-A-dup.md). Neither reported a BLOCK.

## T precondition
Confirmed. handoff-T-green.md reports zero blocking issues: 701/701 vitest, lint and build/typecheck clean, 3 mutation spot-checks each failing the intended test. U (handoff-U.md) also PASS.

## Visual-diff-tool precondition (visual / web projects only)
Met, with two deviations from the role text that the operator should know about.

- **Playwright and ImageMagick.** Playwright 1.65.1 was installed with `npm install --no-save` in a scratch directory outside the project (the repo has no Playwright dependency, and none was added). It drove the cached Chromium 1255 headless. ImageMagick `compare`/`magick` are at `/opt/homebrew/bin`. Captures were at 360×800, 768×1024 and 1280×800 (width fixed; height extended to the page's full scroll height, because the app shell is a fixed-position container and `--full-page` would capture only the viewport).
- **Deviation 1, browser tool.** The `mcp__Claude_in_Chrome__*` tools are not in my tool list, and the global headless-first rule forbids driving a visible browser. I did not fall back to cascade reasoning or prose: every claim below is from a rendered headless-Chromium page of the real built web app, with pixel diffs. I judge the intent of precondition 3 (a real rendered browser) met by Playwright, which the role names as the only reliable tool for the three breakpoints. Say so if you read the precondition more strictly.
- **Deviation 2, the reference.** The role's reference is "preview HTML, design-spec slice, or prior baseline". The only preview, `wireframe.html`, is a low-fidelity multi-state sheet with its own CSS and Unicode stand-in glyphs (D says so in decisions.md), not a render of the page, and the repo has no Playwright baselines. A pixel diff against it would be around 90% by construction. The reference used is the prior baseline: master `73de941` built with the same `vite build` and rendered the same way. The wireframe is used for intent (frames 1, 5 and 6 are compared in words below).
- **How the page was served.** `git archive` of 73de941 and of HEAD into scratch directories, `vite build` for each, a static server with SPA fallback, and `page.route` answering `/api/v1/auth/session` (an admin) and `/api/v1/fleet/health` (the repo's `fixture.json`). This is not the real Rust API (U ran that, with the same fake source behind it). The clock was pinned to the fixture's `generatedAt` so the capture is deterministic; that hides item 7 in pixels, so item 7 was checked separately with the real clock (below).
- **Noise floor.** The same build captured twice differs by 0 to 15 pixels at every viewport and state. One exception, not a code change: the time labels on the "In the window" chart vary between captures of the same build at 360 (about 140 px). It is in unchanged code and is worth knowing about before anyone adds Playwright baselines.

## Visual diff results (visual / web projects only)

Primary state: `/fleet/health#machine-<callisto>` (a machine open, so the concern list, score card, live strip, expanded detail with both bars, the detail slot and the legend are all on screen). Reference = master 73de941, live = this branch. Metrics are computed on the unquantised captures with `compare -metric AE`.

| Viewport | Pixels different | Whole-page delta % | Diff PNG | Composite (live left, reference right) |
|---|---|---|---|---|
| 1280×800 (page 1280×2292) | 7,863 | 0.27% | screenshots/cycle-1/t3-health-1280-diff-20261124.png | screenshots/cycle-1/t3-health-1280-composite-20261124.png |
| 768×1024 (page 768×2612) | 7,738 | 0.39% | screenshots/cycle-1/t3-health-768-diff-20261124.png | screenshots/cycle-1/t3-health-768-composite-20261124.png |
| 360×800 (page 360×3675) | 7,886 | 0.60% | screenshots/cycle-1/t3-health-360-diff-20261124.png | screenshots/cycle-1/t3-health-360-composite-20261124.png |

Second state, nothing expanded (`/fleet/health`): 6,476 px (0.34%) at 1280, 6,477 px (0.47%) at 768, 6,508 px (0.69%) at 360. All six numbers are under the 2% MATCH line.

Third state, concern link clicked (page opened with no hash, "Show all 13", click "Jupiter scores 63"): 128,201 px (3.53%) at 1280, 57,447 (2.40%) at 768, 88,091 (5.60%) at 360. This is a behaviour check, not a regression number. The two builds end in different states on purpose (master leaves the row closed, which is the #241 bug; the branch opens it), so the pages differ in height (2,038 vs 2,832 px at 1280) and a whole-page count is not meaningful. What it shows instead: master `aria-expanded="false"` and the slot prompt; branch `aria-expanded="true"`, `[data-machine-detail]` for Jupiter and the slot reading "Showing Jupiter: …". Composite: `screenshots/cycle-1/t3-health-clicked-1280-composite-20261124.png`.

The images committed under `screenshots/cycle-1/` are palette-quantised (128 colours) to keep the repo small; they are visually the captures but are not the files the numbers were measured on.

### Per-section delta description (driven by red regions in the diff PNGs)

Section boxes come from the live DOM (`data-health-section`, `data-concern-key`, and the two bars' `role="img"`), AE counted inside each box on the primary state.

| Section | 1280 / 768 / 360 | Status | What is different |
|---|---|---|---|
| Areas of concern (rows) | 3.56% / 2.81% / 4.71% | DELTA, intended (item 1; F handoff; wireframe frame 1) | The 8 px dot becomes a 14 px shaped icon: red filled circle with a cross (critical), amber clock (warning), grey circle with a slash (info), and the title and sentence start about 6 px further right. All three severities are distinct in the "Show all 13" list. Nothing else in the rows moved. |
| Memory bar (callisto) | 10.63% / 10.62% / 10.59% (a box 8 px tall) | DELTA, intended (item 5; F handoff) | Used 11.7 GB over a reservation of 8 GB: master drew used over reserved, so the reservation vanished; the branch draws used up to 8 GB, then an orange hatch for the 3.7 GB above it, then free budget. |
| CPU bar (callisto) | 0.19% / 0.21% / 0.24% | MATCH | Used inside the reservation looks the same as before. |
| Machine detail box | 1.15% / 1.53% / 2.71% | DELTA, intended (item 6; wireframe frame 6) | Names the open machine, solid border; with nothing open it is the unchanged dashed prompt. |
| Fleet score card, live strip, category legend | 0.00% | MATCH | The demo data's weights are whole numbers and every null-score component already has a deduction, so items 3 and 4 move no pixels here. The clock was pinned, so item 7 moves none either. |
| Rest of the machine table | 0.03% to 0.07% | MATCH | The residue is the two bars above and, at 360 only, about 140 px of the chart-label jitter described under the noise floor. |

The 1280 primary-state sum (concern rows 6,476 + memory bar 395 + CPU bar 7 + slot 981) is 7,859 of 7,863 differing pixels: effectively all of the delta is in the three intended places. Single boxes above 5% are the memory bar (a 464×8 px strip) and, at 360, none of the sections (concern rows reach 4.71%). The memory-bar delta is the item 5 fix itself and is documented in the brief, the wireframe and F's handoff.

### Other rendered checks
- **Item 7 with the real clock** (no pinning): master "Oldest job waited 7 min"; branch "Oldest job waited 84 h 54 min", from `queuedAt` against now.
- **Item 2 scroll.** After the in-page link click the machine's row is in view (top 20 px at 1280 and at 360; the open row is taller than the viewport, so centring clamps to the top).
- **axe-core** (WCAG 2.x A and AA plus best practice), run on the open-machine page with all concerns shown, at 360 and 1280: the same single violation on master and on the branch, `color-contrast` (12 nodes at 360, 17 at 1280, the `text-faint` timestamps and weights). Nothing new.
- **Glyph contrast** on the card background rgb(15, 16, 17): critical `#eb5757` 5.47:1, warning `#f2994a` 8.55:1, info `#8a95a5` 6.28:1, all above the 3:1 non-text floor.
- **Console:** no errors or warnings on either build in any state; no horizontal overflow at 360.

### Preview / spec sanity check
No source-of-truth defect found, so no ADVISORY-HOLD. The brief and wireframe agree, the wireframe's own "stays collapsed until the hash changes again" rule is a stated choice, and the one convention question (the info severity reusing the `skipped` slash glyph) is a visual mapping only: the announced text is "Info". The brief says no new glyph and the decision is D's and the brief's.

## Spec compliance

| # | Requirement (brief) | Checked | Compliant |
|---|---|---|---|
| 1 | Severity not by colour alone: `RunStatusIcon` through a severity-to-`RunState` map (failure, waiting, skipped) plus `.sr-only` "Critical"/"Warning"/"Info", nothing added to `status-icon.tsx` | `areas-of-concern.tsx` diff; `status-icon.tsx` untouched; rendered glyphs distinct per severity; test asserts both (a) `.sr-only` text and (b) glyph children equal to the bare `RunStatusIcon`, `aria-hidden="true"`, for all 13 rows | YES |
| 2 | Same-page concern link expands the row. `useRouterState({ select })` in `HealthSections` passed down as an optional prop; expansion by render-time `seenHash` adjustment (no state set in an effect); only the scroll in the effect; `hashHostId(hash)` extended, no second parser; table works with `{ data, window }` alone; rewritten 202 test clicks a real `Link` under a real router; page mock gains only `useRouterState` | `page.tsx`, `machine-score-table.tsx`, `machine-format.tsx` diffs; 202 contract test untouched; `fleet-health-page.test.ts` diff is the mock export only; real-browser before/after (master stays closed, branch opens) | YES |
| 3 | Weight as `Math.round(weight)%`, exactly `—` (U+2014, no `%`) when `score === null` | `fleet-score-card.tsx:134`; test uses a 47.05882352941177 weight, a 25, and a null score | YES |
| 4 | Show `deduction` for a null score, per-site fallback: card `INSUFFICIENT_REASON` (line and tooltip), table `title` "not enough data" | `fleet-score-card.tsx` (`||`, so null and `""` both fall back), `machine-score-table.tsx:138`; test covers API reason on the line and in the opened tooltip, both fallbacks per site, and a scored bar unchanged; no other fixed no-data reason string remains (the "Not enough data" at `machine-format.tsx:38` and `machine-detail-window.tsx:147` are different statements, not component reasons) | YES |
| 5 | Recharts horizontal stacked `BarChart` copying the live strip's structure; exported pure segment function; hatch only in `<defs>`, theme variables only | `machine-detail.tsx`: `resourceSegments` exported and pinned by a pure test, `ResizeObserver`-stubbed DOM test pins one non-overlapping row per resource; hatch `<pattern>` in `<defs>`, `useId()` id (two charts in one detail get distinct ids), fills `var(--warning)`/`var(--panel)`; no dependency added (`package.json` unchanged). One small omission from the live strip's structure: the cursor-only `ChartTooltip`, which an 8 px bar has no use for | YES (immaterial omission) |
| 6 | Detail slot always has a child: names the expanded machine or shows the unchanged prompt; state lifted into `HealthSections`; table works with only `{ data, window }`; `fleet-health-page.test.ts` assertions and the design doc's slot rule unchanged | `machine-detail.tsx` (`expandedId`), `page.tsx`; unknown id falls back to the prompt; rendered for three machines (U) and for callisto and Jupiter (here) | YES |
| 7 | `max(waitMs, now - queuedAt)` through a pure helper whose `now` defaults to `Date.now()`; clock pinned in the test | `live-strip.tsx` `oldestWaitMs`; lint (`react-hooks/purity`) clean per T; test pins the clock and covers the skew clamp; real-clock check above | YES |

**Done when:** each item has a test that fails before the fix (T-red: 10 RED, verbatim failures in handoff-T-red.md; items 4's two sites and 5's two tests and 2/6 included); build, lint and tests pass (T-green); Recharts only, no new dependency (`package.json` and `package-lock.json` not in the diff); new tests are in `tests/fleet-health-241-*.test.ts` and no `*.contract.test.ts` file is touched (the diff has none).

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | N/A | Web only; no endpoint or schema changed. |
| Error handling | PASS | Unknown and malformed hashes expand nothing and keep the prompt (page-path guards plus the table-only malformed test); null/empty deduction falls back per site; clock skew clamped; null `used`, over-budget and zero-budget cases in `resourceSegments`. |
| UI/UX match to spec | PASS | See visual results: the only pixel deltas are the three intended regions, all matching the wireframe's description. |
| Accessibility | PASS | Severity now has announced text and a shape; no new axe violation versus master; glyph contrast 5.47 to 8.55:1. Advisory 2 below (chart tab stops). |
| Architecture gate | PASS | A: PASS (plan) and PASS (anti-duplication). Nothing in this audit contradicts either: the controlled/uncontrolled table and the new test helper are the warnings A already recorded. |
| Code organization | PASS | Seven production files, each a few lines, comments updated where the data flow changed; no TODO, `console`, `@ts-`, `any` or `eslint-disable` added (diff scan). The scope cap was weighed by F and was one page and one gated plan. |
| Security | N/A | The hash is only matched against known host ids; nothing reaches a sink. |
| Performance | PASS | One extra router subscription (hash only, via `select`) and two small charts per expanded machine; no polling or timer added. |
| Visual regression | PASS | No Playwright suite or baselines in the repo. Before/after diff against master as above: 0.27% to 0.69%, all in intended regions. axe: no new violations. |
| Naming consistency | PASS | `expanded` / `onExpandedChange` / `hash` and `expandedId` follow A's recommendation and the repo's `on<Thing>Change`; `resourceSegments` and `oldestWaitMs` sit beside `usagePoints`, `slowestKeys` and `formatOldestWait`. |
| Test quality (`testing/test-quality.md` §7) | PASS | See below. |

### Test quality (testing/test-quality.md §7)
I ran my own mutation pass in a scratch copy of HEAD (the worktree was not touched), 12 single-line mutations against the 41 tests in the touched files plus the 202 contract test:

| Mutation | Result |
|---|---|
| item 5: `stackId` removed from the bars | caught by "draws each resource as one row of stacked, non-overlapping segments" |
| item 5: `usedWithin = used` (overlap returns) | caught by "splits reserved, used and budget into segments that never overlap" |
| item 3: `Math.round` dropped | caught by "shows each weight as a whole percent, and a dash …" |
| item 3: dash for null score removed | caught by the same test |
| item 4: card `||` to `??` (empty reason shows blank) | caught by "falls back to the card's own text …" |
| item 4: table title ignores `deduction` | caught by "titles a no-data bar with the API's reason …" |
| item 2: `hashHostId` requires a leading `#` | caught by the 202 in-page link test and the 241 deep-link test |
| item 6: slot never names the machine | caught by the 202 slot test and the 241 deep-link test |
| item 7: clamp removed | caught by "shows how long the job has waited by now, never less than the response said" |
| item 1: `.sr-only` text removed | caught by the item 1 test |
| item 1: all three severities map to one glyph | caught by the same test (assertion (b)) |
| item 2: scroll effect removed | **not caught** (see advisory 3) |

- **Per test:** each names one behaviour, fails in isolation for the right reason (the mutation table plus T-red's verbatim failures), sits at the cheapest tier that can show it (pure function for the segment split; component render for text; a real router only where a same-route navigation is the point), and asserts behaviour, not implementation. The one implementation-coupled test is the chart DOM test, which reads Recharts' own rectangle classes; the brief mandates Recharts and the test failed correctly under the `stackId` mutation, so it earns its place.
- **Per suite:** proportionate for a seven-item fix story: 13 new cases in two files (10 RED, 3 guards beside their item), a non-collected router helper used by two files, two rewrites the brief allows in the 202 file, the ruled one-export mock change, and a clock pin in the 203 live test. The 203 pin is not one of the brief's enumerated edits, but without it that test would read the real clock after item 7 and pass only by accident ("84 h 7 min" contains "7 min"); T documented it and its outcome is unchanged before and after. No zero-behaviour test, no snapshot, no assertion-free or unreachable-outcome test.
- **Smells / "delete or merge":** none blocking. The invariant loop in "splits reserved, used and budget…" (`tests/fleet-health-241-sections.test.ts`, the `cases` array) re-runs four of the five exact cases above it to assert two sums; it is cheap and it guards the stated invariant, so I would leave it, but it is the one place a duplicate signal could be trimmed.

## Scope check
F delivered exactly the seven items. Over-delivery, all small and in the brief's spirit: the optional hatch (the brief allows it), an `aria-label` and `role="img"` on each chart wrapper, and the free-budget segment drawn as a segment (the wireframe lists four segments). Under-delivery: none. No other file, shared component, dependency or contract test was touched, and `docs/design/fleet-health.md` was left for O (A warn 4).

## Verdict

PASS — all seven items are implemented as the brief decided, the suite pins each of them (11 of 12 of my mutations are caught; the twelfth is a jsdom limit I covered in a real browser), the rendered diff against master is 0.27% to 0.69% per viewport with every changed pixel in a region the change was meant to alter, and axe reports nothing new. Ready for O to commit.

## Advisory notes
Non-blocking, for O or a follow-up.

1. **Design doc.** `docs/design/fleet-health.md:352` says the sections each take `{ data, window }` and use only the router's `Link`. That still holds for the sections, but the frame (`HealthSections`) now reads the router's hash and owns the expansion. A and T both left the sentence for O. Suggested addition under "The page": "The frame reads the router's hash and owns which machine is expanded, and passes it to the machine table and the detail slot; the sections still render with `{ data, window }` alone." Or record the deferral in the PR body.
2. **Chart tab stops.** Each new bar chart is wrapped in `role="img"`, and Recharts 3 adds a focusable `svg` (`tabindex="0"`, `role="application"`) inside it, so an expanded machine adds two tab stops that a screen reader may not expose. The live strip has the same default (nothing in `src` sets `accessibilityLayer`), the brief said to copy its structure, and axe does not flag it, so this is consistent and not a #241 defect. A one-line follow-up, `accessibilityLayer={false}` on the two `BarChart`s, would remove it.
3. **Scroll effect has no unit test.** jsdom has no `scrollIntoView`, so removing the effect passes the suite. In a real browser the row is in view after the link click (checked at 1280 and 360). Not worth a jsdom stub.
4. **Same hash, collapsed by hand.** If a user opens a machine from a concern link, collapses the row, then clicks the same concern again, the hash is unchanged and nothing re-expands. This is the wireframe's stated rule ("stays collapsed until the hash changes again"), so it is by design.
5. **Controlled table without a callback.** `MachineScoreTable` given `expanded` but no `onExpandedChange` makes toggles silent no-ops (A-dup warn 1). The only caller passes both.
6. **Pre-existing contrast.** The `text-faint` timestamps and weights fail axe `color-contrast` on master and on the branch alike (12 to 17 nodes). Out of scope here; the new "—" uses the same token as the weight it replaces.
7. **PR body.** Use "Closes #241" as the only closing reference and no closing verb before #195, #202 or #203, or `scripts/fleet-health-guard.mjs` will check the diff against a wave-1 owner list (A warn 6).
8. **Chart-label jitter.** The "In the window" chart's time labels vary between captures of the same build at 360. Not caused by this change; it matters if anyone adds pixel baselines for this page.
