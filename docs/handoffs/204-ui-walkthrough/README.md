# #204 item 5: fleet health UI walkthrough

**Verdict: FAIL.** Every control and flow works: all 260 functional checks pass, in light and dark at 360 px and 1280 px. The verdict is FAIL only because axe reports critical and serious violations: one critical (`button-name`) and two serious rules (`label-content-name-mismatch`, `color-contrast`). Several come from the fleet health code itself (defects 1 to 3).

- **When:** 2026-11-25, about 11:00 to 11:25 AM PST (America/Los_Angeles).
- **Who:** UI walkthrough agent; it reports only. No application code was changed.

## Environment

| | |
|---|---|
| Build | GridOps API + SPA, **debug** build (`target/debug/gridops-api`) serving the worktree's `dist/` at `http://127.0.0.1:18204` (version bar: `v0.2.0 · build 476 · f5ec6b1`) |
| Commit | `f5ec6b1` (`git rev-parse --short HEAD` in the `fh-204` worktree, branch `claude/fh-204`) |
| Database | Seeded scratch SQLite (`/tmp/fh204-walk/gridops.sqlite`). 5 machines: io, Jupiter, callisto, europa (offline), MacBookPro (CPU budget 0, so it has no score). 24 h fleet score 53, 14 concerns (9 critical). 1 h: 8 concerns and no trend. 7 d score 64. 2 queued jobs (1 unmatched), 2 running. GitHub is not connected (the Overview shows "Finish connecting GridOps"). |
| Auth | Session cookie `gridops_session` for an administrator and for a member (values not recorded) |
| Browser | Playwright 1.62.1, headless Chromium, `timezoneId` America/Los_Angeles, `reducedMotion: reduce`. Viewports 1280×900 and 360×780. |
| Theme | Set with `localStorage['gridops-theme']` = `light`/`dark` (`src/lib/theme.ts`) plus `colorScheme` emulation. `<html data-theme>` was checked on every run. |
| Axe | `@axe-core/playwright`, tags `wcag2a, wcag2aa, wcag21a, wcag21aa, wcag22aa, best-practice`, whole page |
| Path | Every run starts at `/`, opens the page from the Overview tile and from the sidebar **Health** link (through the "Open navigation" drawer at 360 px), and reaches Settings through the legend's "Edit job categories" link and the sidebar **Settings** link. Direct URLs were used only for the deep-link cases and the member check. |
| Data restored | The job-category rules were saved twice (once with an added rule, once restoring the list). The final list matches the pre-walk list exactly (compared by `GET /api/v1/settings/job-categories`). |

Driving scripts live outside the repo, in `/tmp/fh204-walk/pw/`.

## Checks

Each row is one check, run in every listed combination (width-theme; "member" = the member session). The job-categories flow ran in full at 1280-light and 360-dark; the section's axe scan ran in all four combinations.

| Control / flow | Combos | Result | Evidence | Detail (first combo) |
|---|---|---|---|---|
| Theme applied | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | data-theme=light |
| Overview tile: score + critical hint | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | tile="Fleet health 53 9 critical concerns" expected score 53, "9 critical concerns" |
| Overview tile: band tone (53 -> danger) | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | tone class seen: danger |
| Overview tile links to /fleet/health | 1280-dark, 1280-light, 360-dark, 360-light | PASS | overview-tile-1280-light.png, overview-tile-1280-dark.png, overview-tile-360-light.png, overview-tile-360-dark.png | href=/fleet/health |
| Overview tile click navigates to page | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | /fleet/health |
| Sidebar Health link navigates; only Health highlighted | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | aria-current="page" links: 4 (Health + 3 concern rows linking to /fleet/health) |
| Default window is 24h with nothing stored | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - |  |
| Window 1h: pressed, stored, score shown | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | pressed=1h stored=1h band=poor expected score 0 |
| Window 1h: trend chart vs "No trend yet" | 1280-dark, 1280-light, 360-dark, 360-light | PASS | health-1h-1280-light.png | trend points=0 chart=0 noTrend=1 |
| Window 1h: score components + insufficient greyed | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | rows=5 insufficient=4 (expected 5/4) |
| Window 1h: first 5 concerns in API order | 1280-dark, 1280-light, 360-dark, 360-light | PASS | health-1h-1280-light.png, health-1h-1280-dark.png | rows=5 expected 5 |
| Window 7d: pressed, stored, score shown | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | pressed=7d stored=7d band=poor expected score 64 |
| Window 7d: trend chart vs "No trend yet" | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | trend points=1 chart=1 noTrend=0 |
| Window 7d: score components + insufficient greyed | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | rows=5 insufficient=0 (expected 5/0) |
| Window 7d: first 5 concerns in API order | 1280-dark, 1280-light, 360-dark, 360-light | PASS | health-7d-1280-light.png, health-7d-1280-dark.png | rows=5 expected 5 |
| Window 24h: pressed, stored, score shown | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | pressed=24h stored=24h band=poor expected score 53 |
| Window 24h: trend chart vs "No trend yet" | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | trend points=3 chart=1 noTrend=0 |
| Window 24h: score components + insufficient greyed | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | rows=5 insufficient=0 (expected 5/0) |
| Window 24h: first 5 concerns in API order | 1280-dark, 1280-light, 360-dark, 360-light | PASS | health-24h-1280-light.png, health-24h-1280-dark.png, health-24h-360-light.png, health-24h-360-dark.png | rows=5 expected 5 |
| Window persisted across reload (7d) | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - |  |
| Invalid stored window ("30d") falls back to 24h | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - |  |
| "Show all 14" button present | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - |  |
| Show all expands to every concern; button disappears | 1280-dark, 1280-light, 360-dark, 360-light | PASS | concerns-show-all-1280-light.png, concerns-show-all-1280-dark.png | rows=14 |
| Severity glyph aria-hidden + sr-only text matches severity (#241) | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | 14 rows checked; e.g. link text "Criticaleuropa is offlinesince Nov 22, 10:00 AMeuropa has sent no sample for 439" |
| Every concern link href matches API link | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | 14 links |
| Live strip totals + category chips | 1280-dark, 1280-light, 360-dark, 360-light | PASS | live-strip-1280-light.png, live-strip-1280-dark.png | queued=2 running=2 q=build,test r=other,test; "Right now Oldest job waited 1 h 7 min · acme/app · unit tests Queued 2 1 Test 1 Build Running |
| Live strip oldest wait text + link to /queued-jobs | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - |  |
| Category legend: 6 categories + Settings link | 1280-dark, 1280-light, 360-dark, 360-light | PASS | category-legend-1280-light.png, category-legend-1280-dark.png | items=6 |
| Machine table: worst score first, unscored last (fixed sort; no sort control exists) | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | europa > Jupiter > callisto > io > MacBookPro |
| Row expand: aria-expanded=true, detail rendered, detail slot names machine | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | slot="Machine detail Showing io: what it is doing now and what it ran in the window ar" |
| Detail: Now section (online, runners, start failures) + CPU/Memory stacked bars | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | bars=["CPU: 0 cores reserved · 6.2 cores used of 22.8 cores","Memory: 0 MB reserved · 19.5 GB used of 64 GB"] shapes=10 |
| Detail: usage chart (stacked areas by category) | 1280-dark, 1280-light, 360-dark, 360-light | PASS | usage-chart-io-24h-1280-light.png, machine-expanded-io-1280-dark.png | areas=4, usage rows=23 |
| Detail: per-category table | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | rows=4 expected 4 |
| Detail: slowest job keys + speed lines | 1280-dark, 1280-light, 360-dark, 360-light | PASS | machine-expanded-io-1280-light.png, machine-expanded-io-1280-dark.png, machine-expanded-io-360-light.png, machine-expanded-io-360-dark.png | keys=0 expected 0; speed lines=2 |
| Expanding another machine collapses the first (one open) | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | open=1 |
| Collapse: no row open, detail slot asks to expand | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - |  |
| Row toggles by keyboard (Enter/Space) | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - |  |
| Unscored machine (MacBookPro): "No score", empty-state texts | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | — MacBookPro No score Online Not enough data NOW Online Runners running: none No start failures in the last hour. CPU 0 cores reserved · 0.5 |
| Offline machine (europa) shows "Offline · last seen" | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | 0 europa Poor Offline · last seen 3d ago Not enough data |
| Concern link #machine-<europa> on same page expands + scrolls to it | 1280-dark, 1280-light, 360-dark, 360-light | PASS | concern-link-opens-europa-1280-light.png | url=/fleet/health#machine-0f000001-0000-4000-8000-000000000004 inView=true |
| Concern link host-slow:callisto opens callisto | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - |  |
| Concern link fleet-component (/fleet/health) stays on page without error | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | url=/fleet/health; expanded after=1 |
| Concern link /queued-jobs navigates | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - |  |
| Concern link /fleet/machines/<europa> opens the machine page | 1280-dark, 1280-light, 360-dark, 360-light | PASS | machine-page-europa-from-concern-1280-dark.png | h1=["europa"] |
| Machine page "Health" button -> /fleet/health#machine-<id> opens that machine | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - |  |
| Every distinct concern link target serves (SPA 200) | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | 9 distinct: /fleet/machines/…04, /queued-jobs, /fleet/health, /fleet/health#machine-…04, /fleet/machines/…03, /fleet/health#machine-…02, /fl |
| Deep link #machine-0f000001-0000-4000-8000-000000000003 | 1280-dark, 1280-light, 360-dark, 360-light | PASS | deeplink-valid-1280-light.png, deeplink-valid-360-dark.png | page rendered=true open=1 slot="Machine detail Showing callisto: what it is doing now and what it ran " consoleErrs=0 url=/fleet/health#mach |
| Deep link #machine-00000000-0000-4000-8000-000000000999 | 1280-dark, 1280-light, 360-dark, 360-light | PASS | deeplink-missing-1280-light.png | page rendered=true open=0 slot="Machine detail Expand a machine above to see what it is doing now and " consoleErrs=0 url=/fleet/health#mach |
| Deep link #machine- (empty) | 1280-dark, 1280-light, 360-dark, 360-light | PASS | same render as deeplink-pct-1280-light.png | page rendered=true open=0 slot="Machine detail Expand a machine above to see what it is doing now and " consoleErrs=0 url=/fleet/health#mach |
| Deep link #machine-%%% | 1280-dark, 1280-light, 360-dark, 360-light | PASS | deeplink-pct-1280-light.png | page rendered=true open=0 slot="Machine detail Expand a machine above to see what it is doing now and " consoleErrs=0 url=/fleet/health#mach |
| Deep link #machine-../x | 1280-dark, 1280-light, 360-dark, 360-light | PASS | same render as deeplink-pct-1280-light.png | page rendered=true open=0 slot="Machine detail Expand a machine above to see what it is doing now and " consoleErrs=0 url=/fleet/health#mach |
| Hash change on page (location.hash = machine-<Jupiter>) opens it | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - |  |
| Second hash change switches to callisto | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - |  |
| Browser Back to previous hash re-opens Jupiter | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | url=/fleet/health#machine-0f000001-0000-4000-8000-000000000002 |
| Legend "Edit job categories" -> /settings/general with the section | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - |  |
| Sidebar Settings reaches General with Job categories | 1280-dark, 1280-light, 360-dark, 360-light | PASS | - | /settings/general |
| Job categories: Add rule appends a row; preview counts it | 1280-light, 360-dark | PASS | settings-jobcats-added-1280-light.png, settings-jobcats-added-360-dark.png | rows 6->7; preview requests=1; row text="test build check deploy other job_name workflow_name repository labels 0" |
| Job categories: Move up swaps the rule with the one above | 1280-light, 360-dark | PASS | - | rule 6="^walk-thru$", rule 7="^gpu$" |
| Job categories: Move down moves it back | 1280-light, 360-dark | PASS | - |  |
| Job categories: first Move up / last Move down disabled | 1280-light, 360-dark | PASS | - |  |
| Job categories: invalid regex -> error on its row; preview paused | 1280-light, 360-dark | PASS | settings-jobcats-invalid-regex-1280-light.png, settings-jobcats-invalid-regex-360-dark.png | row="test build check deploy other job_name workflow_name repository labels — regex parse error: ([unclosed ^ error: unclosed character clas |
| Job categories: Save with invalid regex -> 422 message on the row, nothing saved | 1280-light, 360-dark | PASS | - | row="test build check deploy other job_name workflow_name repository labels — regex parse error: ([unclosed ^ error: unclosed character clas |
| Job categories: Delete removes the row; preview resumes | 1280-light, 360-dark | PASS | - | rows=6 preview="Most common in other Job names of the last 7 days no rule would match, most jobs first (the 20 commonest). No jobs of th" |
| Job categories: preview network failure -> "Preview unavailable" (not invalid) | 1280-light, 360-dark | PASS | settings-jobcats-preview-unavailable-1280-light.png, settings-jobcats-preview-unavailable-360-dark.png | Most common in other Job names of the last 7 days no rule would match, most jobs first (the 20 commonest). Preview unavailable: GridOps coul |
| Job categories: preview 500 -> "Preview unavailable: <message>" | 1280-light, 360-dark | PASS | - | Most common in other Job names of the last 7 days no rule would match, most jobs first (the 20 commonest). Preview unavailable: Internal ser |
| Job categories: 1025-byte pattern refused before sending (row message, Save disabled) | 1280-light, 360-dark | PASS | - | row="me workflow_name repository labels — The pattern is 1025 bytes long; at most 1024 (1 KiB)." saveDisabled=true previewReqsDuring=0 |
| Job categories: 200 rules -> Add rule disabled; no 201st | 1280-light, 360-dark | PASS | settings-jobcats-200-limit-1280-light.png, settings-jobcats-200-limit-360-dark.png | rows=200 addDisabled=true afterForcedClick=200 |
| Job categories: unsaved edits discarded on reload (no PUT succeeded) | 1280-light, 360-dark | PASS | - | rows=6 |
| Job categories: 201 rules -> list message, Save disabled, no preview/PUT sent | 1280-light, 360-dark | PASS | settings-jobcats-201-refused-1280-light.png, settings-jobcats-201-refused-360-dark.png | msg=1 saveDisabled=true paused=1 preview=0 put=0 |
| Job categories: deleting to 200 clears the limit message and resumes preview | 1280-light, 360-dark | PASS | - | preview reqs=1 |
| Job categories: valid Save -> 200 + toast "Job categories saved." | 1280-light, 360-dark | PASS | - | status=200 |
| Job categories: restore original list (Save after delete) | 1280-light, 360-dark | PASS | - |  |
| Member Overview: no Fleet health tile | 1280-light member, 360-dark member | PASS | - |  |
| Member /fleet/health shows "Administrators only", no sections | 1280-light member, 360-dark member | PASS | member-admin-only-1280-light.png, member-admin-only-360-dark.png | sidebar Health link present=1 |

Notes on the checks:
- **Machine table sort:** the table has no sort control. Per the design doc it sorts worst score first with unscored machines last; that order was verified (europa 0 > Jupiter 59 > callisto 63 > io 90 > MacBookPro —).
- **Slowest job keys:** in the 24 h data only callisto has top keys. io, which was expanded, correctly shows "Not enough data: fewer than 3 job keys qualify." because its `topKeys` are empty.
- **Overview "Queued jobs" 0 vs live strip 2:** this is expected with this seed. The Overview counts queued jobs per user (`visible_queued_jobs!`, through `user_installations`) and health counts them fleet-wide (`fleet_queued_jobs!`). The seeded admin has no installation rows, so this is not a defect.
- **Horizontal overflow at 360 px:** none (`scrollWidth == innerWidth`), including with a machine expanded.

## Axe results

26 page-state scans: overview, health 1 h / 24 h / 7 d, health 24 h with a machine expanded and the settings job-categories section, each at 1280 and 360 in light and dark, plus the member page at 1280-light and 360-dark. Counts are node occurrences summed over all scans; "distinct" counts unique elements.

| Impact | Rule | Occurrences (distinct) | Where | Owner |
|---|---|---|---|---|
| **critical** | `button-name` | 11 (2) | **Health page header "Refresh" button at 360 px**: its label is `<span class="hidden sm:inline">`, so the icon-only button has no name. Every health state at 360, light and dark, member page included. Also the Overview's "Refresh GitHub" button at 360 (same pattern). | `src/features/fleet-health/page.tsx` (new); Overview: `src/routes/_app/index.tsx` (pre-existing) |
| **serious** | `label-content-name-mismatch` | 65 (4) | **Window picker**: `aria-label="1 hour" / "24 hours" / "7 days"` on buttons whose visible text is `1h / 24h / 7d` (WCAG 2.5.3 Label in Name). Every health state, both widths, both themes, member page too. Also the shell's "Search and commands" button at 1280. | `page.tsx` `WindowPicker` (new); shell: `src/components/app-shell.tsx` (pre-existing) |
| **serious** | `color-contrast` | 202 (51) | **Fleet-health-specific:** concern "since <time>" (`text-faint`, 2.68:1 light / 3.3:1 dark); component weights "40%…10%" (`text-faint`); "No trend yet." (`text-faint`); insufficient component rows (`opacity-60` on `text-muted-foreground` / `text-faint`: 2.35:1 and 1.72:1 light, 2.84:1 and 1.94:1 dark); band label "Poor" under the gauge (`--danger` #e5484d on white, 3.91:1); "Poor" badge in the machine table (3.44:1, light); detail headings "Now" / "In the window · …" (`text-faint` on `bg-panel-subtle`, 2.5:1 / 3.16:1); Overview tile hint "9 critical concerns" (`text-danger`, 3.91:1, light). **App-wide (pre-existing):** sidebar group labels, "Control plane online", the Ctrl K hint, settings nav group labels, input unit suffixes ("days", "MB"), Overview tile hints. All of these use `--faint`. | Root cause: the theme tokens `--faint` (#9b9ea4 light, #62666d dark) and `--danger` in `src/styles/app.css`. The fleet health components use them for readable text: `areas-of-concern.tsx`, `fleet-score-card.tsx`, `machine-detail.tsx`, `machine-detail-window.tsx`, `machine-score-table.tsx` (Badge `destructive`) |
| moderate | `region` | 26 (1) | The fixed version bar (`GridOps v0.2.0 · build 476`) is outside any landmark | shell / debug build (pre-existing) |

Per scan (node counts; c = critical, s = serious, m = moderate):

| State | 1280 light | 1280 dark | 360 light | 360 dark |
|---|---|---|---|---|
| overview | contrast 14s, name-mismatch 1s, region 1m | contrast 13s, name-mismatch 1s, region 1m | button-name 1c, contrast 6s, region 1m | button-name 1c, contrast 5s, region 1m |
| health 1h | contrast 20s, mismatch 4s, region 1m | contrast 18s, mismatch 4s, region 1m | button-name 1c, contrast 4s, mismatch 3s, region 1m | button-name 1c, contrast 4s, mismatch 3s, region 1m |
| health 24h | contrast 15s, mismatch 4s, region 1m | contrast 13s, mismatch 4s, region 1m | button-name 1c, contrast 4s, mismatch 3s, region 1m | button-name 1c, contrast 4s, mismatch 3s, region 1m |
| health 7d | contrast 15s, mismatch 4s, region 1m | contrast 13s, mismatch 4s, region 1m | button-name 1c, contrast 4s, mismatch 3s, region 1m | button-name 1c, contrast 4s, mismatch 3s, region 1m |
| health 24h, machine expanded | contrast 9s, mismatch 4s, region 1m | contrast 6s, mismatch 4s, region 1m | button-name 1c, contrast 5s, mismatch 3s, region 1m | button-name 1c, contrast 2s, mismatch 3s, region 1m |
| settings › job categories | contrast 7s, region 1m | contrast 7s, region 1m | contrast 3s, region 1m | contrast 3s, region 1m |
| member /fleet/health | contrast 4s, mismatch 4s, region 1m | — | — | button-name 1c, mismatch 3s, region 1m |

The job-categories section itself had no violations of its own: its settings-page contrast nodes are the shared nav labels and input unit suffixes.

**Serious + critical total: 278 node occurrences** (critical 11, serious 267) across 3 rules.

## Console and network

- **Administrator flows: no console errors and no failed requests**, apart from the ones the walk caused on purpose:
  - 2 × `422` on `job-categories` and `…/preview` (invalid regex, preview and save);
  - 1 × `net::ERR_FAILED` on `…/preview` (Playwright route abort, the network-failure case);
  - 1 × `500` on `…/preview` (Playwright route stub).
  
  Each appeared once per job-categories run (1280-light and 360-dark), with the matching console line "Failed to load resource". None of the deep-link cases (missing, empty, `%%%`, `../x`) logged anything.
- **Member:** each Overview load sends `GET /api/v1/fleet/health?window=24h` and gets a `403`, logged in the console (defect 6). The Overview's `GET /api/v1/fleet/workload?window=24h` also gets a `403`; that one is pre-existing. `/fleet/health` itself gets the expected `403` and renders "Administrators only".
- **Environmental:** the first 1280-light attempt hit a burst of `net::ERR_NETWORK_CHANGED` from the host network (assets and `/api/v1/overview`). It was re-run cleanly, and this is not an app fault.

## Defects

1. **Health page "Refresh" button has no accessible name at mobile widths.** Severity **serious** (axe critical `button-name`).
   - Where: `/fleet/health` header below the `sm` breakpoint (360 px), all windows, member page too.
   - Expected: a named button ("Refresh").
   - Actual: `<button>` containing only an icon and `<span class="hidden sm:inline">Refresh</span>`, so screen readers announce "button".
   - File: `src/features/fleet-health/page.tsx` (`PageHeader` actions; add `aria-label="Refresh"`).
   - The Overview's "Refresh GitHub" button has the same pre-existing issue in `src/routes/_app/index.tsx`.
2. **Window picker accessible names don't contain the visible text** (WCAG 2.5.3 Label in Name). Severity **serious** (axe serious).
   - Where: `/fleet/health` header, every state, both widths.
   - Expected: a voice-control user saying "click 24h" hits the button.
   - Actual: the names are "1 hour", "24 hours" and "7 days" while the buttons show "1h", "24h" and "7d".
   - File: `src/features/fleet-health/page.tsx` (`WindowPicker`, `WINDOW_LABELS`). Use the visible text as the name, or start the name with it ("24h, 24 hours").
3. **Text contrast below 4.5:1 on fleet health content.** Severity **serious** (axe serious).
   - Where (light / dark ratios):
     - concern "since" times: 2.68 / 3.3;
     - component weights: 2.68 / 3.3;
     - "No trend yet.": 2.68 / 3.3;
     - insufficient score components, faded with `opacity-60`: 1.72–2.35 / 1.94–2.84;
     - "Poor" band label: 3.91, light;
     - "Poor" badge: 3.44, light;
     - detail headings "Now" and "In the window": 2.5 / 3.16;
     - Overview tile hint "9 critical concerns": 3.91, light.
   - Expected: ≥ 4.5:1 for 12–13 px text.
   - Files: `src/features/fleet-health/areas-of-concern.tsx`, `fleet-score-card.tsx`, `machine-detail.tsx`, `machine-detail-window.tsx`, `machine-score-table.tsx`, `src/components/ui/badge.tsx` (destructive variant). Underneath them: `--faint` and `--danger` in `src/styles/app.css`, which also fail app-wide in the sidebar, settings and Overview tiles (pre-existing).
4. **Deduction sentence understates the points lost when weight is redistributed.** Severity minor (misleading data).
   - Where: Fleet score card, 1 h window (`health-1h-1280-light.png`). Availability is the only scored component, at 100% weight and score 0, and the fleet score is 0.
   - Expected: the sentence names the points actually lost ("−100"), as the doc's "Deduction sentences: … the points lost" implies.
   - Actual: "…online 79.6% of the window against a 100% target: −10."
   - File: `crates/gridops-core/src/fleet_health/score.rs`. `lost` is computed from the nominal weight (around line 120), before the weights are spread (around lines 506–512).
5. **Long durations in concern text are not rolled up into hours or days.** Severity minor (cosmetic).
   - Where: Areas of concern.
   - Expected: "3 d 1 h" / "1 h 8 min", matching the live strip ("Oldest job waited 1 h 8 min").
   - Actual: "europa has sent no sample for 4391 min 27 s" and "A job has waited 68 min 27 s".
   - File: `crates/gridops-api/src/fleet/health_live.rs` (`span_text`, line ~404: minutes and seconds only).
6. **The Overview requests fleet health for members.** Severity minor.
   - Where: Overview as a member.
   - Expected: no request, since the doc says "the Overview calls `useFleetHealth("24h")`, administrators only".
   - Actual: `GET /api/v1/fleet/health?window=24h` → `403` plus a console error on every Overview load. The tile is correctly absent.
   - File: `src/routes/_app/index.tsx:124` (`useFleetHealth("24h", { enabled: data.authenticated })`; gate on the viewer being an administrator).
7. **Concern rows that link to `/fleet/health` are marked as the current page.** Severity minor (a11y).
   - Where: the `fleet-component:*` concerns (3 rows at 24 h).
   - Actual: the router's `Link` sets `aria-current="page"` on them, so screen readers announce "current page", and activating them does nothing visible.
   - Expected: no `aria-current`. Ideally the link points somewhere that explains the concern (e.g. the score card), or the row is not a link.
   - Files: `src/features/fleet-health/areas-of-concern.tsx` (`ConcernRow` Link); the link value comes from the concern builder (`concerns.rs`).
8. **The usage chart's last x-axis tick is clipped.** Severity cosmetic.
   - Where: machine detail usage chart, right edge ("09:00 AM" cut off; `usage-chart-io-24h-1280-light.png`).
   - File: `src/features/fleet-health/machine-detail-window.tsx` (`AreaChart` margin right 8 / `XAxis` padding).
9. **Job-category row buttons all have the same name.** Severity minor (a11y).
   - Every row has "Move up", "Move down" and "Delete rule", while the selects and input are numbered ("Pattern of rule 3").
   - Expected: "Move rule 3 up", "Delete rule 3".
   - File: `src/features/settings/job-categories-section.tsx`.

## Screenshots

All are PNG viewport or element captures, each under 150 KB, in this directory:

- **Overview tile:** `overview-tile-{1280,360}-{light,dark}`
- **Health page per window:** `health-{1h,24h,7d}-1280-{light,dark}`, `health-24h-360-{light,dark}`
- **Sections:** `concerns-show-all-1280-{light,dark}`, `live-strip-1280-{light,dark}`, `category-legend-1280-{light,dark}`
- **Machine detail:** `machine-expanded-io-{1280,360}-{light,dark}`, `usage-chart-io-24h-1280-light`
- **Links:** `concern-link-opens-europa-1280-light`, `machine-page-europa-from-concern-1280-dark`
- **Deep links:** `deeplink-{valid,missing,pct}-1280-light`, `deeplink-valid-360-dark`
- **Job categories:** `settings-jobcats-{added,invalid-regex,preview-unavailable,200-limit,201-refused}-{1280-light,360-dark}`
- **Member:** `member-admin-only-1280-light`, `member-admin-only-360-dark`

## Defects filed (under #198)

- #256: accessibility (defects 1–3, plus defect 6, the member 403 on the Overview). This blocks #204's "no serious or critical axe violations" until it is fixed.
- #257: deduction sentences use the weight before reweighting (defect 4).
- #258: small UI defects (defects 5, 8 and 9).
- Defect 7, the concerns that link to the page itself, is #253's item 7.
