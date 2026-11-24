# Handoff-A: Phase 3 - Fleet health wave-1 review fixes (#241)  (up-front plan review, re-review)

**Date:** 2026-11-24
**Branch:** claude/fh-241 (worktree fh-241, head 87b73b7 = master 73de941 + the amended brief + D's revision 2)
**Brief reviewed:** docs/handoffs/241-brief.md (amended)   **Reuse map:** none for this run (no `survey.md` in docs/handoffs/241/; the baseline table below stands in for it)   **Wireframe:** docs/handoffs/241/wireframe.html (revision 2; Approval line in handoff-D.md still to be filled in by O)
**Verdict:** PASS

## Summary

PASS. The round-1 BLOCK is resolved. Item 6 now follows option B: the expansion state moves into `HealthSections`, so the `machine-detail` slot always has a child. Neither #197's page assertion nor the design doc's slot rule changes. All ten round-1 warnings were also folded into the brief. Each of the seven items now extends the object already responsible for it, and nothing in the plan is a parallel path. I read the router source to check the one dependency I had hedged last round. In router-core 1.171.15, a `Link` whose `to` is `/fleet/health#machine-<id>` ends up with `location.hash` = `machine-<id>`, so the `HealthSections` subscription will see a concern-link click. Six warnings follow. They cover test harness placement, prop naming, comments and docs that would go stale, a duplicate SVG id, and the guard's wording trap.

## Round-1 findings, re-checked against the amended brief

| Round-1 # | Was | Now |
|---|---|---|
| 1 (block) | Item 6 left the `machine-detail` slot empty, which fails `tests/fleet-health-page.test.ts:66-68` | Resolved: item 6 "Decided (option B)". The slot shows the prompt or the expanded machine's name and is never empty. Page-test assertions are unchanged; only the ruled `useRouterState` mock export is added |
| 2 | Where `useRouterState` lives | Resolved: in `HealthSections`, passed down as an optional prop; the contract file is untouched |
| 3 | setState in an effect | Resolved: render-time adjustment, only the scroll stays in the effect |
| 4 | Second hash parser | Resolved: `hashHostId(hash)` is extended |
| 5 | Real-router test | Resolved: the click goes through the real concern `Link` under `createRouter`/`RouterProvider` (see warn 1 below for where it can live) |
| 6 | `SeverityIcon` export | Resolved: severity → `RunState` map, `.sr-only` text, nothing added to `status-icon.tsx` (`RunState` is already exported, `status-icon.tsx:3`) |
| 7 | `Date.now()` in render | Resolved: pure helper with a `now` default |
| 8 | Chart structure | Resolved: live-strip structure, exported segment function, hatch in `<defs>` with theme variables |
| 9 | `machine-score-table.tsx:120` sibling | Resolved: added to item 4 with its own per-site fallback |
| 10 | Test placement | Resolved: Done-when puts new tests in `tests/fleet-health-241-*.test.ts` and none in a contract file |
| 11 | `web/` path | Resolved: `src/` |

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | Item 2: "Rewrite `tests/fleet-health-202-detail.test.ts` (~:145-152) … under a real `createRouter`/`RouterProvider`" | file structure / test harness | That file mocks `@tanstack/react-router` for the whole module with `Link` only (`:17-23`). Under a Vitest factory mock, importing `createRouter`/`RouterProvider` throws. A mocked `Link` is a plain anchor, which bypasses the router's history, so the click never reaches `useRouterState`. If the mock is kept, or "extended" with `importActual` while `Link` stays mocked, the rewritten test cannot pass after the fix either. Also, `HealthSections` is not exported (`page.tsx:120`). | Delete that file's `vi.mock`. It is safe: nothing the file renders imports the router. Only `areas-of-concern.tsx`, `live-strip.tsx`, `category-legend.tsx` and `components/page.tsx` do, and the 202 tests render only `MachineScoreTable` and `MachineDetail`. Render `FleetHealthPage` as the root route's component under `RouterProvider` at `/fleet/health`, as `tests/page-header-nav.test.ts:38-45` mounts its router, with a fetch stub like the page test's `serve()` (`tests/fleet-health-page.test.ts:46-53`). Do not export `HealthSections` just for the test. The fixture's four `/fleet/health#machine-…` concerns are rows 9-12 of 13, so the test clicks "Show all 13" before clicking one of them. |
| 2 | warn | Items 2 and 6: `MachineScoreTable`'s optional expansion props | naming / abstraction level | Lifting the state makes `MachineScoreTable` the codebase's first component that works both controlled and uncontrolled. The brief describes the behaviour ("expansion props", "the change callback", "the hash prop") but names neither the props nor the mode switch. | Follow the repo's `on<Thing>Change` naming (`command-menu.tsx:28` `onOpenChange`, `dashboard/queued-jobs.tsx:58` `onPageChange`, `runner-pools/targets-row.tsx:83` `onSelectionChange`): `expanded?: string \| null`, `onExpandedChange?: (id: string \| null) => void`, `hash?: string`. Decide the mode by one test, `expanded !== undefined`, and run the table's own `seenHash` adjustment only when uncontrolled. Add a one-line doc comment saying why both modes exist: the #202 contract test renders `{ data, window }` alone. A scratch lint run of this exact shape, together with the `HealthSections` render-time adjustment, under the repo's react-hooks 7.1.1 `recommended` rules, reports nothing. A control file in the same run is flagged for `set-state-in-effect` and `purity`, which shows the rules are active. |
| 3 | warn | Item 6: "a new prop on `MachineDetail`" (handoff-D) | contract shape | The brief says "name the expanded machine". D says that needs a new prop. Neither says what the prop is. | Use `expandedId?: string \| null`, the same id the table's `expanded` carries. The no-host branch (`machine-detail.tsx:15-24`) resolves it against `data.hosts`, which `MachineDetail` already receives. An unknown id shows the prompt. Do not overload `host`: that renders the full detail. |
| 4 | warn | Comments and design doc after the data-flow change | documentation drift | Four places would describe the old flow. `page.tsx:56-60` says each section takes `{ data, window }`. `machine-score-table.tsx:13-18` and `:22-24` say "the router re-renders this section … no location subscription", which this fix proves wrong. `machine-detail.tsx:8-13` says "where no machine is expanded yet". `docs/design/fleet-health.md:352` says each section takes "`{ data, window }` and using only the router's `Link`". | Update the three code comments in files this change already edits. For the design doc, O decides. One option is to add a sentence to "The page": the frame reads the router's hash and owns the machine expansion, and the machine sections still render with `{ data, window }` alone. That leaves alone the slot-content rule the brief protects. The other is to open a follow-up. |
| 5 | warn | Item 5: the optional hatch `<pattern>` | pattern consistency | The CPU and memory charts render in the same `MachineDetail` (`machine-detail.tsx:56-57`), so a pattern with a fixed `id` appears twice in the document. | Only if the hatch is drawn: give the pattern its id with `useId()` (precedent: `settings/refresh-preference-row.tsx:11`, `runner-pools/platform-options-section.tsx:23`). Follow the one existing chart `<defs>` (`routes/_app/index.tsx:226-232`, colours as `var(--<tone>)`). |
| 6 | warn | The PR description | cross-cutting (the `Fleet health guard`) | #241 is not in `docs/design/fleet-health-owners.json`, so the guard passes it only while the PR body puts no wave-1 issue number (190-195, 199-203) after a closing verb. `CLOSING` (`scripts/fleet-health-guard.mjs:22`) would match prose such as "fixes #202's deep-link test". The guard would then check every changed file against #202's list and fail on `page.tsx`, `areas-of-concern.tsx`, `live-strip.tsx`, the page test and the new 241 test files. | Use "Closes #241" as the only closing reference. Mention #195, #202 and #203 without "close", "fix" or "resolve" in front of them. |

### Reuse baseline per item (stands in for the missing Reuse map; Phase 7 checks against it)

| Item | Object to extend | What would count as a parallel path |
|---|---|---|
| 1 | `RunStatusIcon` with a `Record<Severity, RunState>` map and the `StatusBadge` sr-only pattern, in `areas-of-concern.tsx` | new SVG geometry, copied glyph paths, or a new export in `status-icon.tsx` |
| 2 | `hashHostId` in `machine-format.tsx` (now taking the hash string); the existing render-time adjustment pattern, moved into `HealthSections`, which subscribes through `useRouterState({ select })` (`app-shell.tsx:121`); the table's one scroll effect | a second `#machine-` parser; a `hashchange` listener; `useRouterState` inside a section; a new expansion store, context or hook |
| 3, 4 | `ComponentRow` in `fleet-score-card.tsx` (`INSUFFICIENT_REASON` kept as its fallback) and `ComponentBars`' `title` in `machine-score-table.tsx` (its own "not enough data" fallback) | a separate weight or reason component, or one fallback string shared across both places |
| 5 | `ResourceBar` in `machine-detail.tsx`, rebuilt on the `live-strip.tsx` stacked `BarChart` structure, with the segment calculation exported from `machine-detail.tsx` | a new shared chart component under `src/components/charts/` for one caller |
| 6 | `MachineDetail`'s existing placeholder branch, plus one id prop; state in `HealthSections` | a second slot component; a hidden dummy child put there only to satisfy `childElementCount` |
| 7 | `OldestQueuedWait` and `formatOldestWait` in `live-strip.tsx`, plus a pure `now`-defaulted helper beside them (as `formatElapsed`, `agent-state.ts:78`) | a second duration formatter; a new timer hook or interval |

## Notes for O

None required: the verdict is PASS. All six findings are warnings. Findings 1 and 2 are the ones T and F will hit first. Pass them along with the brief to the next phase, and settle finding 4's design-doc sentence before the PR. Fill in D's Approval line in `handoff-D.md` before T starts.

## Patterns referenced

- `docs/design/fleet-health.md` ("The page", :343-377; "Ownership and the guard", :409-424); `docs/design/fleet-health-owners.json`; `scripts/fleet-health-guard.mjs:22, :113-148`
- `src/features/fleet-health/page.tsx:109-140`, `machine-score-table.tsx:19-38`, `areas-of-concern.tsx:39-51` (render-time state adjustment); `src/components/app-shell.tsx:121` (`useRouterState` with `select`)
- `tests/fleet-health-202-detail.test.ts:17-23` (the file's `Link`-only mock), `tests/page-header-nav.test.ts:38-45` (real-router mount), `tests/fleet-health-page.test.ts:21-27, :46-69`
- `@tanstack/router-core` 1.171.15 (installed copy in the gridops-202 worktree): `router.ts` `buildLocation`, which keeps a `#` inside `to` in the href (`encodePathLikeUrl` does not encode `#`), and `parseLocation` (`hash: decodePath(hash.slice(1)).path`); `@tanstack/history` `parseHref`, which splits off the `#…`
- `src/features/fleet-health/live-strip.tsx:32-44`, `machine-detail-window.tsx:26, :133`, `src/routes/_app/index.tsx:226-232` (chart `<defs>` with theme variables)
