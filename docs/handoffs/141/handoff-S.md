# Handoff-S: Phase 9 - GitLab in the shared connections and targets UI (#141)

**Date:** 2026-11-20 (evening, Pacific Time)
**Branch:** claude/gitlab-141-ui-3ae7f5 (head `4b284ef`; production source unchanged since F's commit `699880b`)
**Issue:** #141 (epic #136, wave 1). Brief: `docs/handoffs/141-brief.md` (amended after A rounds 1-3)
**Handoff-T reviewed:** `docs/handoffs/141/handoff-T-green.md` and `handoff-T-red.md`
**Handoff-A reviewed:** `docs/handoffs/141/handoff-A.md` (round 3, PASS) and `handoff-A-dup.md` (PASS)
**Handoff-F reviewed:** `docs/handoffs/141/handoff-F.md`
**Also read:** `handoff-U.md`, `handoff-D.md`, `decisions.md`, `evidence.md`, `wireframe.html` (frame H notes), the full diff against `0086aa9`
**Operator-facing report:** [`phase-9-gitlab-shared-ui-report.html`](phase-9-gitlab-shared-ui-report.html) (open it in a browser; images are in [`screenshots/cycle-1/`](screenshots/cycle-1/))

## Result in one line

**REWORK (production).** Everything in the brief is implemented and behaves correctly except one layout defect in the shared connection row: with GitLab's four row actions, the **Edit** button is clipped by the card at 360 px wide (13 of its 44 px visible; nothing at 320 px) and cannot be read or tapped. U's 360 pass reported all four buttons visible; per-button geometry shows otherwise (below).

## A precondition
Confirmed. `handoff-A.md` round 3 PASS (three warns, all taken up or recorded); `handoff-A-dup.md` PASS (one warn, tracked as a follow-up: `OTHER_PLATFORM_KINDS` hardcodes the GitHub and Bitbucket kind ids).

## T precondition
Confirmed. `handoff-T-green.md`: no blocking issue, 548/548 Vitest. I re-ran at HEAD: `npx vitest run` 51 files / **548/548**; `npx tsc --noEmit` exit 0; `npm run lint` (`--max-warnings=0`) clean; `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api the_runners_list_carries` 1 passed.

## Visual-diff-tool precondition
- **Browser tooling.** The `mcp__Claude_in_Chrome__*` tools are not in this session's tool list. I did not return CANNOT-AUDIT for that: the role itself says the Chrome MCP viewport is locked and headless Playwright is the only reliable path to 360/768/1280, and #145's S made the same call. I used headless Playwright 1.63 (Chromium, from `/Users/andreangelantoni/aftersight-srv/node_modules`, not added to the repo) and ImageMagick `compare`/`magick` (on PATH). No window opened on the desktop. This substitution is mine; the operator may overrule it.
- **Reference for the pixel diff.** The only design reference is the low-fidelity wireframe (`handoff-D.md`: Mode (a), glyph stand-ins), so a pixel diff against it decides nothing. #145's operator waived the wireframe diff for that story only; **no waiver is recorded for #141**. I applied the #145 precedent and diffed **HEAD against the pre-#141 build (`0086aa9`)** with identical GitHub + Bitbucket mock data, which shows exactly what #141 changed on every existing surface. GitLab-only states have no earlier build, so they are shown in the report for review (not diffed). Operator: please confirm this is acceptable for #141 (it is the same reasoning as #145).
- **How I rendered it.** `git archive` snapshots of `0086aa9` and of HEAD in the scratchpad (nothing in the worktree touched), each served by `vite dev` (ports 3141 and 3142, since stopped), with every `/api/v1` call intercepted and answered from the brief's contract (the same approach as U, since GridOps has no headless login). The app scrolls inside its own container, so page views use tall viewports (360x2400, 768x1700, 1280x1500) and dialogs/sheets use 360x800, 768x1024, 1280x800. Scripts and raw captures are in the scratchpad (not committed); the report and screenshots are.

## Visual diff results

Before (`0086aa9`) against after (HEAD), same mock data. Counts are differing pixels over the whole capture.

| Surface | 1280 | 768 | 360 | What it is |
|---|---|---|---|---|
| Settings > Connections | 25,401 px / 1.323% | 25,401 px / 1.946% | 22,278 px / 2.579% | The new empty GitLab row, and everything below it moves down one row (intended, wireframe frame A) |
| Add connection chooser | 7,324 / 0.715% | 8,977 / 1.142% | 5,160 / 1.792% | GitLab listed as a third choice (intended) |
| New runner pool form | 0 | 0 | 0 | Pixel-identical |
| New pool, Targets picker open | 0 | 0 | 0 | Pixel-identical |
| Repository pool settings | 0 | 0 | 0 | Pixel-identical |
| Webhooks list (GitHub rows) | 169 / 0.009% | 0 | 0 | GitHub badge in each row at 1280; hidden below `lg` by design (wireframe H1) |
| Delivery detail sheet | 84 / 0.008% | 84 / 0.011% | 43 / 0.015% | Badge beside the heading (intended) |
| Runners list (GitHub runner) | 0 | 0 | 0 | Pixel-identical |

Each pair has a wipe-slider comparator, a red diff PNG and a side-by-side composite in the report (`screenshots/cycle-1/t3-<surface>-<width>-{ref,live,diff,composite}-20261120.png`, crops named `t3-...-crop-20261120.png`). All counts are under 3%, and every nonzero one is a delta the brief or wireframe asks for. **The REWORK is not a pixel-threshold result; it comes from the GitLab-only state, which has no reference, and was found by measuring the render.**

### Per-section delta description

| Section | Viewport | Finding | Intended / documented? | Status |
|---|---|---|---|---|
| Connections list | all | One new row "GitLab - No GitLab projects or groups are connected yet."; nothing else changes | Yes (wireframe A) | MATCH |
| Add connection chooser | all | GitLab entry after Bitbucket | Yes (wireframe B) | MATCH |
| Webhooks list | 1280 | GitHub badge before the destination; 768 and 360 unchanged | Yes (wireframe H1, note 3) | MATCH |
| Delivery sheet | all | Badge beside the heading | Yes (H2) | MATCH |
| New pool, picker at rest, pool settings (repository), Runners | all | Identical to before | n/a | MATCH |
| **GitLab row: Edit button** | **360, 320** | Edit's left edge is at x=-11 (360) and x=-51 (320); the card starts at x=20, so 13 of 44 px show at 360 and none at 320. `elementFromPoint` at its centre does not return the button. Verify, Rotate webhook secret and Remove are inside the card. All four fit from 390 px up (measured at 320, 360, 390, 414, 480, 560, 639, 640, 768). | No | **REWORK** |
| GitLab row: name and path | 1280, 768 | The Project/Group badge drops to its own line and the path is cut ("gitlab.com/acme/platform..."), because four actions plus "Verified" take the row. Same cause as above | No | DELTA, cosmetic, not blocking |
| GitLab-only states (form, reveal, rotate, picker, platform options, pool settings, runner sheet, deliveries) | all | Render as the wireframe describes; no overflow, overlap or console error beyond the mocked 400s | n/a | MATCH |

**U versus S on the 360 row.** `handoff-U.md` item 20 says "Edit/Verify/Rotate/Remove all visible (wrap to a second line)". The actions do move to their own line below `sm` (the #145 round-5 layout) but they do not wrap inside it, and the first button is clipped. U checked page-level horizontal overflow, which stays false because the card clips. I measured each button's rectangle and hit-tested it. I treat my measurement as authoritative; U should measure per-button geometry on its re-walk.

## Spec compliance

| Requirement (brief / issue) | Compliant | Evidence |
|---|---|---|
| GitLab descriptor in the registry, one line plus the descriptor module | YES | `gitlab.tsx`; `registry.tsx` imports it and registers it last with the order comment |
| Connection fields: base URL (default `https://gitlab.com`), project or group path, token; help names `create_runner`, `read_api` and the Owner role | YES | Rendered form at 360/768/1280; token is `type=password`; help text read in the DOM |
| Scope fixed at create; edit changes only name and optional token; PATCH | YES | `editableOnUpdate`; PATCH body pinned in `gitlab-descriptor.test.ts` |
| Target kinds GitLab project / group, one option per connection, `<baseUrl>/<scopePath>`, standalone, many | YES | Rendered picker; after one GitLab pick I read the options: 5 GitHub/Bitbucket options `aria-disabled` with "Can't combine with GitLab targets" (U's wording on this was loose; the behaviour is correct) |
| Label rules: any character but a comma; cap | YES, with amended cap | Shared 64-character cap wins over the issue's 255 per the brief (A finding 8); see Advisory 1 |
| `run_untagged` only in Platform options, only when the pool targets GitLab | YES | Absent with GitHub-only selection (pool form pixel-identical to before); appears after a GitLab pick; POST body carries `scope: "gitlab"`, `gitlabConnectionIds`, `runUntagged: true`, `installationId: 1`, `repositoryIds: []`, `runnerGroupId: 1` (captured from the rendered form) |
| Runner link `<base_url>/<scope_path>/-/runners/<id>` | YES | Rendered runner sheet: `https://gitlab.com/acme/platform/-/runners/555` at 360 and 1280; backend test passes |
| After create, webhook URL and secret once, copy buttons, Job events setting; row menu rotation; both via descriptor hooks | YES | `postCreate` and `actions` hooks; reveal rendered at all widths with two Copy buttons; secret leaves the DOM on close (test) |
| Create toast kept (A round 2 warn 4) | YES | "GitLab connection added." seen behind the reveal |
| Deliveries listed with a platform badge, platform derived generically | YES | `platformForDeliveryId` in the registry; GitLab and GitHub badges at 1280; sheet badge at all widths; Destination "-" for a GitLab delivery |
| Polling errors | Recorded gap | #148 only logs them; no API lists them; nothing built (brief, F Known issues) |
| GitLab-specific code limited to the amended allowlist | YES | `grep -rli gitlab src` lists only `gitlab.tsx`, the API functions, `schemas.ts`, `runner-pools.functions.ts`, `operations.functions.ts` and the registry import/entry; zero `gitlab` strings in every other shared file; `platform-neutral-source` green |
| Hard limits: frontend only plus the one narrow backend edit; no `migrations/`, `docs/openapi/` or other `crates/` | YES | `git diff --name-only 0086aa9..HEAD -- crates migrations docs/openapi` lists only `resources.rs` and `resources/gitlab_tests.rs`; the SQL change is the select, one LEFT JOIN and three JSON keys, as the brief states |
| No new page, route, nav item or pool-form row | YES | None added; new-pool form pixel-identical when GitLab is absent |
| Gaps 1-7 closed generically and recorded in `decisions.md` | YES | F entry lists each gap and its fix; the diffs match |
| Vitest covers descriptor validation (tags, URL, path), payloads, schema, registry, deliveries, neutral source | YES | 87 new/updated tests, 548/548 |
| Headless walkthrough of every GitLab control | YES (U) and independently re-done here | Add, error states, verify, edit, rotate, delete, pick as target, toggle, runner link, delivery badge all exercised in my render; the only defect found is the 360 Edit clipping |
| Pool list / pool detail show a GitLab pool as a GitHub account | Out of scope | #159 (open) covers these two; the runner row's Repository column is a third view not in #159's body (Advisory 3) |
| "Tag validation error on a comma" (brief walkthrough) | Not reachable through the UI | The Additional labels field splits on commas, so a comma can never reach the validator; D flagged this (open question 1). The rule is enforced by construction, in `gitlabLabelError` and in the schema, and unit-tested. U walked the 64-character error instead. Accepted (Advisory 2) |

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | GitLab calls use the spec paths and methods (PATCH, 204 verify, rotate); runners JSON gains three always-present nullable keys; `/api/v1/runners` is not in OpenAPI |
| Error handling | PASS | Server message shown verbatim (create, verify); declined confirm sends nothing; inline validation blocks submit; older server without `gitlabConnections` still opens |
| UI/UX match to spec | **FAIL** | Edit clipped at 360 and 320 (REWORK item 1); cosmetic badge wrap and path truncation at 768/1280 |
| Accessibility | PASS at code level | `aria-invalid` plus described-by on invalid fields, select has an accessible name, switch labelled; the clipped Edit is also a reachability defect. Not axe-checked (no axe setup in the repo) |
| Architecture gate | PASS | A round 3 and A-dup PASS; nothing here conflicts. The fix belongs in the shared row, which the brief allows for descriptor-contract gaps |
| Code organization | PASS | One descriptor module plus generic shared extensions; no TODO, FIXME, console output or `eslint-disable` added (the only one is the pre-existing `postCreate` line) |
| Security | PASS | Token is a password field; secret and webhook URL leave the DOM on close; no secret logged; server owns authorization (`canManage`) |
| Performance | PASS | No new polling; the settings picker fetches the GitLab list once through the existing lazy-load seam |
| Visual regression | **FAIL (one defect)** | No Playwright VR suite in the repo; before/after build diff above is clean for every existing surface; the GitLab row defect is REWORK item 1 |
| Naming consistency | PASS | `SecretReveal`, `ConnectionRowAction`, `platformForDeliveryId`, `gitlab.project` / `gitlab.group` match the brief |
| Test quality (`testing/test-quality.md` §7) | PASS (nits only) | See below |

### Test-quality audit
Suite size (87 new or updated tests across 15 production files, a contract, two routes and a SQL join) is proportionate and tiered: pure descriptor and schema logic is unit, row and dialog wiring is jsdom, the SQL join is one backend integration test. T's RED run proved each test fails for the feature's own assertion, and the two `connections-page` stub failures were a test-stub error, repaired test-only. I found nothing that should be deleted or merged. Non-blocking nits, all optional:
- `tests/gitlab-descriptor.test.ts:184` asserts `body?.baseUrl ?? DEFAULT_ADDRESS`, so it also passes if the field is omitted. This is deliberate leniency (the contract defaults a missing address), and the exact sent body is pinned at `:166` and in `gitlab-connections.test.ts`, so the line adds little; fine to leave or tighten.
- `tests/gitlab-descriptor.test.ts:249-259`: the `available` column is declared for the "a new pool" row but only asserted for `update` contexts (`if (context.form === "update")`), so the title overstates for that row. Create-side availability is asserted at `:241`.
- `tests/gitlab-connections.test.ts` ("is offered after GitHub and Bitbucket..."): one test unmounts and re-renders to check two behaviours (chooser order, then form defaults). Splitting it would read better.
- There is no test for the clipped Edit, and none is required: layout does not exist under jsdom, so any assertion would be a class-name check (mock-shaped). The 360 render is the verification.

## Scope check
F delivered the brief's scope: the descriptor, the GitLab API functions, the generic gap fixes, the wire fields, the optional runner fields and the one narrow backend edit with its test. No `migrations/`, OpenAPI or other `crates/` change; no new dependency; no new page, route, nav item or pool-form row. Over-delivery: none. Under-delivery: the 360 layout of the new row actions (REWORK item 1). The polling-errors list is out of reach by the brief and recorded.

## Verdict

**REWORK** - the following must be addressed before commit (production change; `reworkKind: production`):

1. **Edit is clipped at 360 and 320 px on GitLab connection rows.** In `src/features/platform-connections/connections-section.tsx`, `ConnectionRow` (the action cluster `<div className="flex shrink-0 items-center gap-1">`, line 267, inside the `w-full ... justify-end` wrapper) cannot wrap. With Edit, Verify, "Rotate webhook secret" and Remove it is about 334 px wide, wider than the row below about 390 px, and `justify-end` pushes the overflow off the left edge where the card's `overflow-hidden` clips it. Make the cluster fit generically (for example let it wrap, right-aligned), with no GitLab string in the shared file. Acceptance, measured per button in a real render at 320, 360 and 390 px: each of Edit, Verify, Rotate webhook secret and Remove lies inside the card and `elementFromPoint` at its centre returns that button; no page-level horizontal overflow; Bitbucket and GitHub rows and everything at 768/1280 unchanged (the cosmetic badge wrap and path truncation there may improve but need not). No test change is required, and T's 548 tests should stay green; `platform-neutral-source` must keep passing.
2. **Process (U and the next S pass):** re-walk 360 with per-button geometry and a hit test, not only page overflow, and save a 360 screenshot of a GitLab row with all four actions.

## Advisory notes

1. **Tag cap.** The issue says GitLab tags may be 255 characters; the brief overrides it with the shared 64-character cap (A finding 8), so a GitLab tag longer than 64 characters cannot be used. Raising it is a separate server and schema change; worth a follow-up issue if long GitLab tags matter.
2. **Comma tag walkthrough item.** The brief's "tag validation error on a comma" cannot be performed through the UI (the labels field splits on commas). The rule is enforced by construction and in code, and covered by Vitest. Suggest rewording the brief's walkthrough item to the 64-character error.
3. **#159 sibling.** A round 3 warn 2 recommended adding the runner row's Repository column (`runner-row.tsx`, `runner.repository ?? runner.accountLogin`) to #159, which still names only the pool list and pool detail. A GitLab runner will show under its installation's GitHub login. O should add it to #159.
4. **Deliveries below `lg`.** The platform badge is hidden below 1024 px (wireframe H1 says so), so on a phone or tablet a GitLab delivery looks like a GitHub one in the list; the sheet shows the badge at every width. Accepted as designed; worth revisiting if mobile operators use this page.
5. **Targets row copy.** On a GitLab pool the Targets description still reads "Repositories from different accounts can share capacity... must cover every repository." It is the shared text and not a #141 regression.
6. **Pre-existing Targets dropdown clip** (card `overflow-hidden`, only the search box shows when it opens): carried from #145's advisory note 1; still unfiled as far as I can tell. In the picker tests above Playwright can still click options, a mouse user cannot.
7. **Carried from A-dup:** `OTHER_PLATFORM_KINDS` in `gitlab.tsx` hardcodes the GitHub and Bitbucket kind ids for `excludes`; a fourth platform means editing every descriptor.
8. **No-installation create.** A GitLab-only create with no GitHub App installation sends `installationId: 0` and shows the schema message (brief and wireframe F4); no new copy.
9. **Environment.** U's finding 1 (the bundled Chromium rejecting the pool-name `pattern` attribute) is pre-existing and did not affect my render: submitting the new-pool form with the valid name `gitlab-builds` went through and posted the expected body.
