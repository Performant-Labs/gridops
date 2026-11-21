# Handoff-S: Phase 9 - GitLab in the shared connections and targets UI (#141), re-audit (cycle 2)

**Date:** 2026-11-20, 7:50 PM PST
**Branch:** claude/gitlab-141-ui-3ae7f5 (head `a746d23`; production source last changed in F's rework commit `675a213`)
**Issue:** #141 (epic #136, wave 1). Brief: `docs/handoffs/141-brief.md` (amended after A rounds 1-3)
**Handoff-T reviewed:** `docs/handoffs/141/handoff-T-green.md` (both sections) and `handoff-T-red.md`
**Handoff-A reviewed:** `docs/handoffs/141/handoff-A.md` (round 3, PASS) and `handoff-A-dup.md` (PASS, re-check after the rework)
**Handoff-F reviewed:** `docs/handoffs/141/handoff-F.md` (both sections, "Rework after S" is the second)
**Also read:** `handoff-U.md` (cycle 2), `handoff-D.md`, `decisions.md`, `evidence.md`, `141-diff-result-r2.md` (the automated diff review), the diff `1406856..HEAD`
**Operator-facing report:** [`phase-9-gitlab-shared-ui-report.html`](phase-9-gitlab-shared-ui-report.html) (open it in a browser; images are in [`screenshots/cycle-2/`](screenshots/cycle-2/); cycle-1 images stay in [`screenshots/cycle-1/`](screenshots/cycle-1/))

## Result in one line

**PASS.** The one defect from my first audit (Edit clipped by the card at 360 and 320 px on GitLab connection rows) is fixed by a one-line class change in the shared row. I re-measured it myself on a fresh build: every button of every row is inside the card, hit-testable at five points, operable by a raw click, and nothing else on any page changed.

## Cycle history

| Cycle | Verdict | What happened |
|---|---|---|
| 1 | REWORK (production) | Edit was clipped at 360 and 320 px (13 of 44 px visible at 360, none at 320) because the shared row's action cluster could not wrap. Full text is in git at `1406856`; its screenshots remain in `screenshots/cycle-1/`. |
| rework | F `675a213` | `ConnectionRow`'s action cluster went from `flex shrink-0 items-center gap-1` to `flex min-w-0 flex-wrap items-center justify-end gap-1`, plus a comment. No GitLab string. |
| re-check | T-green, A-dup, U | All PASS (548/548; one-file diff, no duplication; U re-walked with per-button geometry). |
| 2 | **PASS** | This audit. |

## A precondition
Confirmed. `handoff-A.md` round 3 is PASS. `handoff-A-dup.md` (the re-check after F's rework) is PASS, with one carried warn: `OTHER_PLATFORM_KINDS` in `gitlab.tsx` hardcodes the GitHub and Bitbucket kind ids (tracked follow-up, not a defect here).

## T precondition
Confirmed. `handoff-T-green.md` (re-verification section) reports no blocking issue. I re-ran at head `a746d23`:
- `npx vitest run`: 51 files, **548/548** passed.
- `npx tsc --noEmit`: exit 0.
- `npm run lint` (`--max-warnings=0`): exit 0.
- `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api the_runners_list_carries`: 1 passed.

## Visual-diff-tool precondition
- **Browser tooling.** The `mcp__Claude_in_Chrome__*` tools are not in this session's tool list. As in cycle 1 and #145, I did not return CANNOT-AUDIT: the role says the Chrome MCP viewport is locked and headless Playwright is the only reliable path to 360/768/1280. I used headless Playwright 1.63 (Chromium, from `/Users/andreangelantoni/aftersight-srv/node_modules`, not added to the repo) and ImageMagick 7.1.2-35 `compare` and `magick`. No window opened on the desktop. This substitution is mine; the operator may overrule it.
- **Reference.** The only design reference is the low-fidelity wireframe (`handoff-D.md`, Mode (a), glyph stand-ins), so a diff against it decides nothing. As in cycle 1 and #145 I diffed (a) this head against the pre-#141 build `0086aa9` on every existing surface, and (b) this head against the previous head (`4b284ef`) on every GitLab state, which isolates exactly what the rework changed. **No operator waiver of the wireframe diff is recorded for #141**; I flag that again for the operator.
- **How I rendered it.** `git archive HEAD` into the scratchpad (nothing in the worktree touched), served by `vite dev` on port 3162 (since stopped), every `/api/v1` call answered from the brief's contract (GridOps has no headless login, the same approach as U and cycle 1). Page views use tall viewports (360x2400, 768x1700, 1280x1500) because the app scrolls inside its own container; dialogs and sheets use 360x800, 768x1024, 1280x800. Scripts and raw captures are in the scratchpad (not committed); the report and screenshots are.

## Preview / spec sanity check
No source-of-truth defect that warrants ADVISORY-HOLD. Two convention-adjacent points, both recorded as advisory and neither a spec defect:
- **Touch-target height.** Row action buttons are 28 px high (the shared `Button size="sm"` ghost), below the 44 px guideline. The wireframe is low-fidelity and sets no size; the same 28 px buttons are the pre-existing Edit and Remove on every GitHub and Bitbucket row since #145 (identical in the pre-#141 build). So this is a shared-component property, not something the spec or #141 introduced, and it meets the 24 px WCAG 2.2 AA minimum. Follow-up, not a hold.
- **No-installation message.** See "Automated diff review" below (W-1).

## Visual diff results

### The change under test: GitLab connection rows, previous head vs this head

| Viewport | Pixels different | Whole-page delta % | Diff PNG | Composite |
|---|---|---|---|---|
| 1280x1500 | 8.6 | 0.000% | `screenshots/cycle-2/t3-gitlab-connections-rework-1280-diff-20261120.png` | `screenshots/cycle-2/t3-gitlab-connections-rework-1280-composite-20261120.png` |
| 768x1700  | 8.6 | 0.001% | `screenshots/cycle-2/t3-gitlab-connections-rework-768-diff-20261120.png` | `screenshots/cycle-2/t3-gitlab-connections-rework-768-composite-20261120.png` |
| 360x2400  | 23,987.9 | 2.776% | `screenshots/cycle-2/t3-gitlab-connections-rework-360-diff-20261120.png` | `screenshots/cycle-2/t3-gitlab-connections-rework-360-composite-20261120.png` |

- 1280 and 768: the 8.6 px is one 47x11 px box, the "Verified 8h ago" label becoming "9h ago" between capture runs. I cropped both and read them. The row is otherwise pixel-identical.
- 360: in the 2-5% band, driven only by the GitLab rows' action buttons wrapping (the diff region starts at the Mobile app row and covers everything below it, which moves down). F documented this as the intended fix and it is exactly what my cycle-1 acceptance asked for.
- Every other GitLab state (the 17 scenarios, 3 widths each) differs from the previous head by at most 17 px of clock text, except the 360 captures that show the connection list behind a dialog (form 259 px, reveal 685, rotate 926, verify-error 2,287), whose differing region is the row area behind the dialog. `b4d-edit-row` is 0 px at 360 because the first Edit on the page is the Bitbucket row's, and a row in edit mode hides its actions.

### Every existing surface, pre-#141 build `0086aa9` vs this head (same mock data)

| Surface | 1280 | 768 | 360 | What it is |
|---|---|---|---|---|
| Settings > Connections | 25,401 / 1.323% | 25,401 / 1.946% | 22,278 / 2.579% | New empty GitLab row, rows below move down (intended, wireframe A) |
| Add connection chooser | 7,324 / 0.715% | 8,977 / 1.142% | 5,160 / 1.792% | GitLab as a third choice (intended) |
| New runner pool form | 0 | 0 | 0 | Pixel-identical |
| New pool, Targets picker open | 0 | 0 | 0 | Pixel-identical |
| Repository pool settings | 0 | 0 | 0 | Pixel-identical |
| Webhooks list (GitHub rows) | 177 / 0.009% | 9 / 0.001% | 9 / 0.001% | GitHub badge at 1280 only (hidden below `lg`, wireframe H1); 9 px is clock text |
| Delivery detail sheet | 84 / 0.008% | 84 / 0.011% | 43 / 0.015% | Badge beside the heading (intended) |
| Runners list (GitHub runner) | 4 / 0.000% | 4 / 0.000% | 4 / 0.000% | Clock text only |

These are identical to cycle 1 (the Connections and Add-connection counts match to the pixel: 25,400.8 and 22,278.3). In particular the GitHub and Bitbucket rows are unchanged by the rework: `a1-connections` (no GitLab data) is 0 px against the previous head at all three widths. Each pair has a wipe-slider comparator, a red diff PNG and a side-by-side composite in the report, and section crops named `t3-...-crop-20261120.png`.

### Per-section delta description

| Section | Viewport | Finding | Intended / documented? | Status |
|---|---|---|---|---|
| **GitLab row actions (Edit, Verify, Rotate webhook secret, Remove)** | 360, 320, 390 | The four buttons wrap right-aligned: three on the first line and Remove under them at 360 and 390, two and two at 320. All inside the card. From 414 up all four fit on one line. The GitLab Edit opens its form (name and token only). | Yes (F rework, S cycle-1 item 1) | **MATCH (fixed)** |
| GitLab row: name and path | 1280, 768 | Unchanged from cycle 1: on the Mobile app row the Project badge drops under the name and the path ends with an ellipsis ("gitlab.com/acme/platform..."). The full path is not available on hover (no `title`). | No, F left it by design | DELTA, cosmetic, not blocking (advisory 1) |
| Connections list, Add chooser, Webhooks list, Delivery sheet | all | As cycle 1 | Yes (wireframe A, B, H1, H2) | MATCH |
| New pool form, picker, repository pool settings, Runners | all | Identical to before #141 | n/a | MATCH |
| GitLab-only states (form, errors, reveal, rotate, verify error, picker, Platform options, pool settings, runner sheet, deliveries) | all | Render as the wireframe describes; no overflow or overlap; console shows only the deliberate mocked 400 and an unmocked fleet-profile request (`GET /api/v1/fleet/pools/<id>/profile` on pool settings), which the pre-#141 build makes too | n/a | MATCH |

## Rework verification (cycle-1 acceptance, measured by me)

Cycle-1 acceptance: each of Edit, Verify, Rotate webhook secret and Remove lies inside the card and `elementFromPoint` at its centre returns that button, at 320, 360 and 390 px; no page-level horizontal overflow; Bitbucket and GitHub rows and everything at 768/1280 unchanged.

- **Geometry and hit test.** For 14 widths (320, 340, 360, 375, 390, 414, 430, 480, 560, 639, 640, 768, 1024, 1280) and every row (Bitbucket, Mobile app, Platform group), each button was checked for: inside the card, inside the viewport, hit-tested at five points (centre, left, right, top, bottom), and no clipped label text; plus no page-level overflow. **Zero failures.** Mobile app row: at 360 Edit 61-105, Verify 109-165, Rotate webhook secret 169-323, Remove 255-323 (the card is x 20-340); the cluster is two lines at 320-390 and one line from 414. These match F's and U's tables to the pixel.
- **Raw click.** At 320, 360 and 390 I clicked each button's centre with a raw mouse event (no Playwright actionability helpers, no force): Edit opens its form; Verify sends `POST .../verify`; Rotate webhook secret and Remove each open a confirm dialog and send nothing. All 12 pass. Buttons are 28 px high.
- **Order.** DOM order, visual order and tab order are all Edit, Verify, Rotate webhook secret, Remove; `flex-wrap` adds no `order` shift.
- **Bitbucket and GitHub rows.** Unchanged: 0 px against the previous head on the no-GitLab surface, and the Bitbucket row (Edit, Remove) is inside the card at every width.
- **768 and 1280.** Unchanged (8.6 px of clock text).
- **Process item 2 (U and the next S pass re-walk 360 with per-button geometry and save a 360 screenshot of a GitLab row with all four actions).** Done by U (`handoff-U.md`, matching numbers) and by me; my 320, 360 and 390 crops and the opened-Edit capture are in `screenshots/cycle-2/` and the report.

## Spec compliance

Production code is unchanged except the one class change, and every GitLab state is pixel-identical to cycle 1 (see above), so the cycle-1 findings stand. Re-confirmed this cycle by direct run: the runner link `https://gitlab.com/acme/platform/-/runners/555` at 360 and 1280; the create body; the GitLab Edit form fields.

| Requirement (brief / issue) | Compliant | Evidence |
|---|---|---|
| GitLab descriptor in the registry; one descriptor module plus one line | YES | `gitlab.tsx`; `registry.tsx` imports it and registers it last with the order comment |
| Connection fields: base URL default `https://gitlab.com`, project or group path, token; help names `create_runner`, `read_api`, Owner | YES | Rendered form; help text visible in the opened Edit form at 360; token is `type=password` |
| Scope fixed at create; edit changes only name and optional token; PATCH | YES | Edit form shows only "Connection name" and "New access token"; PATCH body pinned in `gitlab-descriptor.test.ts` |
| Target kinds GitLab project / group; one option per connection; `<baseUrl>/<scopePath>`; standalone; many; excludes GitHub and Bitbucket | YES | Rendered picker (cycle 1, unchanged) |
| Label rules: any character but a comma; cap | YES, amended | The shared 64-character cap wins over the issue's 255 per the brief (A finding 8); advisory 3 |
| `run_untagged` only in Platform options, only when targeting GitLab; POST carries `scope: "gitlab"`, `gitlabConnectionIds`, `runUntagged` | YES | Absent on a GitHub-only form (pixel-identical to before); present after a GitLab pick; body captured by U this cycle |
| Runner link `<base_url>/<scope_path>/-/runners/<id>` | YES | Rendered at 360 and 1280; backend test `the_runners_list_carries...` passes |
| After create: webhook URL and secret once, copy buttons, Job events; row menu rotation; via descriptor hooks | YES | `postCreate` and `actions` hooks; reveal renders at all widths; now also reachable at 360 because Rotate and Verify are no longer pushed off |
| Create toast kept (A round 2 warn 4) | YES | Unchanged |
| Deliveries listed with a platform badge, platform derived generically | YES | `platformForDeliveryId` in the registry; badge at 1280 list and sheet |
| Polling errors | Recorded gap | #148 only logs them; no API lists them; nothing built |
| GitLab-specific code limited to the amended allowlist | YES | `grep -rli gitlab src` lists only `gitlab.tsx`, the GitLab API functions, `schemas.ts`, `runner-pools.functions.ts`, `operations.functions.ts` and the registry; the rework adds no `gitlab` string; `platform-neutral-source` is inside the 548 |
| Hard limits: frontend only plus the one narrow backend edit; no `migrations/`, `docs/openapi/` or other `crates/` | YES | `git diff --name-only 0086aa9..HEAD -- crates migrations docs/openapi` lists only `resources.rs` and `resources/gitlab_tests.rs` |
| No new page, route, nav item or pool-form row | YES | None; pool form pixel-identical with no GitLab data |
| Gaps 1-7 closed generically and recorded in `decisions.md` | YES | F entries (original and rework) |
| Vitest covers descriptor validation, payloads, schema, registry, deliveries, neutral source | YES | 548/548 |
| Headless walkthrough of every GitLab control | YES | U (cycle 1 and re-walk) and my own renders |
| "Tag validation error on a comma" (brief walkthrough) | Not reachable through the UI | The labels field splits on commas, so a comma never reaches the validator; enforced by construction and unit-tested; U walked the 64-character error. Accepted (advisory 4) |

## Automated diff review (`141-diff-result-r2.md`, verdict PASS)
It is not part of the pipeline's gates, but it is a second reader of the same diff, so I dispositioned every finding rather than assume it was handled.
- **W-1 (no installation: `installationId: 0`, message not asserted).** The brief and wireframe F4 say to show "the shared create schema's message for `installationId`, verbatim" and to record what the UI shows. I ran it: a GitLab-only form with no GitHub App installation sends nothing and shows the callout **"Could not create the pool"** with **"Too small: expected number to be >0"**. F matches the spec, so this is not a defect in F's work. The message does not tell the operator what to do; advisory 2 recommends a follow-up (this is also the first time the exact text is recorded, which the brief asked for).
- **W-2 (plan note omits the count).** F documented this deviation (a kind's `planNote` receives only that kind's count, wrong for a mixed project plus group selection). The note states the maximum, `ok` is a status shown by the plan preview, and `raisesMinMaxCount` keeps the Maximum field's floor at the selection so over-maximum is not normally reachable. Advisory 5.
- **W-3 (schema tag rule only runs when ids are present).** Matches where `validateBitbucketConnections` applies the Bitbucket rule, and the settings form always sends `gitlabConnectionIds` for a GitLab pool; a comma tag cannot be typed. No action.
- **NIT-1** (a test seeds create-path option values from an update-pool fixture, `tests/gitlab-descriptor.test.ts:321-326`) is in my nits below. **NIT-2** (`success: action.success ?? "Done."` default Verify never uses) is harmless.
- NV-1 to NV-8 are the reviewer's unverified claims; the ones that matter are settled by the green runs above (cargo runners test, tsc, 548/548).

## Quality audit
| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | GitLab calls use the spec paths and methods (PATCH, 204 verify, rotate); runners JSON gains three always-present nullable keys; `/api/v1/runners` is not in OpenAPI |
| Error handling | PASS | Server message shown verbatim (create, Verify toast at 360 read in this audit); a declined confirm sends nothing; inline validation blocks submit; an older server without `gitlabConnections` still opens |
| UI/UX match to spec | PASS | Cycle-1 defect fixed and measured; cosmetic path truncation at 768 and 1280 remains (advisory 1) |
| Accessibility | PASS at code level | `aria-invalid` with described-by, labelled select and switch; every action is now reachable and tab order matches visual order; 28 px buttons (advisory 1). Not axe-checked (no axe setup in the repo) |
| Architecture gate | PASS | A round 3 and A-dup PASS; nothing here conflicts; the fix is generic in the shared row, which the brief allows |
| Code organization | PASS | The rework is one class string and a two-line comment; no TODO, FIXME, console output or new `eslint-disable` |
| Security | PASS | Token is a password field; secret and webhook URL leave the DOM on close; no secret logged; the server owns authorization (`canManage`) |
| Performance | PASS | No new polling or requests; layout-only change |
| Visual regression | PASS | No Playwright VR suite in the repo; before/after build diffs above are clean for every existing surface and for every GitLab state except the intended 360 re-layout |
| Naming consistency | PASS | `SecretReveal`, `ConnectionRowAction`, `platformForDeliveryId`, `gitlab.project` / `gitlab.group` match the brief |
| Test quality (`testing/test-quality.md` §7) | PASS (nits only) | See below |

### Test-quality audit
No test file changed since cycle 1 (`git diff --stat 1406856..HEAD -- tests` is empty), so the cycle-1 audit stands and I re-checked the cited lines. The suite (87 new or updated tests across 15 production files, a contract, two routes and one SQL join) is proportionate and tiered: pure descriptor and schema logic is unit, row and dialog wiring is jsdom, the join is one backend integration test. T's RED run proved each test fails on the feature's own assertion. I found nothing to delete or merge. Non-blocking nits, all optional:
- `tests/gitlab-descriptor.test.ts:184` asserts `body?.baseUrl ?? DEFAULT_ADDRESS`, so it also passes if the field is omitted; deliberate leniency (the contract defaults a missing address) and the exact body is pinned at `:166`.
- `tests/gitlab-descriptor.test.ts:249-258`: the `available` column is declared for "a new pool" but only asserted for update contexts (`if (context.form === "update")`); create-side availability is asserted at `:241-245`.
- `tests/gitlab-connections.test.ts:238-258`: one test unmounts and re-renders to check two behaviours (chooser order, then form defaults); splitting it would read better.
- `tests/gitlab-descriptor.test.ts:321-326` (diff review NIT-1): create-path option values are seeded from an update-pool fixture; works, misrepresents the real flow.
- No test for the wrapped action cluster, and none is required: layout does not exist under jsdom, so any assertion would be a class-name check (mock-shaped). The measured render above is the verification.

## Scope check
F's rework changed exactly one file, `connections-section.tsx`, by one class string and a comment, with no GitLab string. Over-delivery: none. Under-delivery: none outstanding. The original feature scope was verified in cycle 1 and is unchanged. The polling-errors list stays out of reach by the brief and is recorded.

## Verdict

**PASS** - the cycle-1 defect is fixed and measured, no regression anywhere I diffed, 548/548 green, spec-compliant, quality acceptable. Ready for O to commit.

## Advisory notes
1. **Row detail and button size (shared row, follow-up).** At 768 and 1280 a long GitLab project path is cut with an ellipsis and the Project badge drops under the name; the full path has no hover `title`. Two projects under the same group (for example `acme/platform/mobile-app` and `acme/platform/web-app`) would both read "gitlab.com/acme/platform..." in the row, so the connection name carries the distinction. The row buttons are also only 28 px high (shared `Button size="sm"`, unchanged since #145). Suggest a `title` on the detail or letting it wrap, and a decision on whether row actions should become a menu on narrow widths. Not a #141 blocker; F's note that narrowing the cluster at `sm`+ changes every platform's row is right.
2. **No-installation message.** A GitLab-only pool create with no GitHub App installation shows "Could not create the pool: Too small: expected number to be >0". Spec-compliant (brief, wireframe F4), but it does not say what to do. Follow-up: a plain-language message on the shared `installationId` rule, or lift #138's installation requirement.
3. **Tag cap.** The issue says GitLab tags may be 255 characters; the brief overrides it with the shared 64-character cap (A finding 8). Raising it is a separate server and schema change; file a follow-up if long GitLab tags matter.
4. **Comma tag walkthrough item.** The brief's "tag validation error on a comma" cannot be performed through the UI (the labels field splits on commas). Suggest rewording the brief's walkthrough item to the 64-character error.
5. **Plan note.** The GitLab plan note states the maximum but not the number of selected targets (documented deviation, diff review W-2).
6. **#159 sibling.** A round 3 warn 2 recommended adding the runner row's Repository column (`runner-row.tsx`, `runner.repository ?? runner.accountLogin`) to #159, which still names only the pool list and pool detail. A GitLab runner shows under its installation's GitHub login. O should add it to #159.
7. **Deliveries below `lg`.** The platform badge is hidden below 1024 px (wireframe H1), so a GitLab delivery looks like a GitHub one in the list on a phone or tablet; the sheet shows the badge at every width. Accepted as designed.
8. **Carried from A-dup.** `OTHER_PLATFORM_KINDS` in `gitlab.tsx` hardcodes the GitHub and Bitbucket kind ids for `excludes`; a fourth platform means editing every descriptor.
9. **Pre-existing, not #141.** The Targets dropdown is clipped by its card (only the search box shows when it opens; carried from #145's advisory); the shared Targets description still reads "Repositories from different accounts can share capacity..." on a GitLab pool; the page overflows below 320 px because the app shell has a 320 px minimum width; the bundled Chromium rejects the pool-name `pattern` attribute (environment).
10. **Environment.** The report's comparators are headless Playwright renders against mocked API responses, not a live GridOps. The one thing a mock cannot prove is the server's real `canManage` and the runner-link JSON; the second is covered by the passing backend test, the first by T's tests.
