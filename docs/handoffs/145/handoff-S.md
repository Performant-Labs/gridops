# Handoff-S: Phase 9 - One Connections page and one Targets picker for every CI platform (#145), round 4

**Date:** 2026-11-20
**Branch:** claude/ui-145-platform-neutral-cf5832 (head `2329261`, U's round-4 commit; production source unchanged since `4704c38`)
**Issue:** #145 (brief: `docs/handoffs/145-brief.md`, including the binding "Round-4 operator decisions after the spec audit")
**Handoff-T reviewed:** `docs/handoffs/145/handoff-T-green.md` (round 4 section) and `handoff-T-red.md` (round 4 section)
**Handoff-A reviewed:** `docs/handoffs/145/handoff-A.md` (up-front plan review, PASS) and `docs/handoffs/145/handoff-A-dup.md` (anti-duplication gate, round 4, PASS)
**Handoff-F reviewed:** `docs/handoffs/145/handoff-F.md` (round 4 section)
**Also read:** `handoff-U.md`, `handoff-U-round4.md`, `decisions.md`, `survey.md`, the previous `handoff-S.md` (ADVISORY-HOLD, replaced by this file)
**Operator-facing report:** [`phase-9-platform-neutral-ui-report.html`](phase-9-platform-neutral-ui-report.html) (open it in a browser; screenshots are in [`screenshots/`](screenshots/))

## Result in one line

**REWORK (production, one small pass).** Every round-4 acceptance criterion holds at code level and the earlier criteria still hold, but the rendered screenshots at 360 px show two defects this story introduced on the new Connections page: the connection names collapse (the GitHub organization's name is invisible) and the "Manage App" button is pushed off the screen. At 768 and 1280 everything I looked at is fine.

## What changed since the last S

The previous S returned ADVISORY-HOLD because it could not run the visual gate. The operator's round-4 decision waived the pixel diff, accepted W3 and W4, and added W1 and W2 plus a `postCreate` toast. It made PASS conditional on two things: every criterion holds at code level, and "U's saved screenshots show no defect against this brief".

- **Code level:** holds (below).
- **U's saved screenshots:** there are none. `handoff-U-round4.md` ends "No screenshot files were saved to the project tree", and `docs/handoffs/145/screenshots/` did not exist. The condition could not be checked as written, so I produced the screenshots myself with headless Playwright and judged them. That is where the two defects below came from. U's walkthrough used desktop and 360 on the Connections page only; it never saw the 360 problems because it did not look at the GitHub App section or the row names at that width.

## A precondition
Confirmed. `handoff-A.md` PASS (round 4, five warns, none blocking). `handoff-A-dup.md` PASS (round 4; two warns, neither a parallel path).

## T precondition
Confirmed. `handoff-T-green.md` round 4: GREEN, no blocking issue, 461/461 tests, four mutation checks each failed the matching tests. I re-ran the suite myself (below).

## Visual-diff-tool precondition
- **Pixel diff:** waived by the operator for this story (brief, round-4 decision), because the only reference is the low-fidelity wireframe. I did not produce `compare -metric AE` numbers and I am not returning ADVISORY-HOLD or CANNOT-AUDIT for that, as the decision says.
- **Browser tooling:** the Chrome MCP tools are not in this session's tool list. The role's own guidance is that Playwright headless is the only reliable way to get 360/768/1280, so I used that: Playwright 1.65.1 installed in the scratchpad (not in the repo), cached Chromium 1255, ImageMagick `compare`/`magick` on PATH. No window was opened on the desktop. This is a rendered check, not cascade reasoning.

## How I rendered it
- The real `gridops-api` debug binary (backend unchanged; `git diff 5550cf5 HEAD --stat -- crates migrations docs/openapi` is empty) serving `npm run build` output from this worktree at HEAD (footer reads "build 459 · 2329261").
- Throwaway SQLite databases, seeded with two users (administrator, member), two GitHub installations (an organization and a user account), four repositories, two Bitbucket connections, two pools (one repository pool targeting two repositories and a Bitbucket workspace, one organization pool) and two runners. One database has the GitHub App configured, one does not.
- A loopback fake of the GitHub API (installation tokens, repositories, runner groups) so the pickers load real options. `GRIDOPS_GITHUB_API_URL` pointed at it.
- A baseline copy of the frontend built from `5550cf5` (`git archive`, in the scratchpad) against the same API, to separate regressions from pre-existing behavior.
- Viewports 360x800, 768x1024, 1280x800 (taller viewports for the full-length captures, because the app scrolls inside its own container). The scripts are in `screenshots/repro/` (paths inside are this run's scratchpad; adjust).
- Servers were stopped by port afterwards; the throwaway databases live only in the scratchpad. One slip to disclose: early on I ran `pkill -f gridops-api`, which matches any `gridops-api` process on the machine, not only mine. Only my own instance was running at that moment as far as I could see, but I cannot rule out that I stopped another session's server. I used port-specific kills from then on.

## Visual results

| Viewport | Pixel diff | Horizontal overflow | Defects introduced by #145 | Console errors or warnings |
|---|---|---|---|---|
| 360x800 | waived | none (page 360 / 360) | **2** (D1, D2) | none |
| 768x1024 | waived | none | 0 | none |
| 1280x800 | waived | none | 0 | none |

Screenshots (60+ files) are in `docs/handoffs/145/screenshots/`, named `s-<surface>-<width>.png`; `base-*` files are the `5550cf5` baseline. Capture logs: `s-capture-log.txt` and `s-capture-log-forms.txt`. (The first log lists a failed Bitbucket-form step on a `Back` selector that matched two elements; `capture2.mjs` re-took those three shots and the chooser shots were re-taken after the dialog animation settled. Both are in the final set.)

Surfaces covered, each at 360, 768 and 1280: Connections (nothing connected, administrator; nothing connected, member; populated, administrator; populated, member), Add connection (chooser with nothing connected and with connections; GitHub step with the App configured and not configured; Bitbucket form; Back), new-pool form (empty, picker open, repository picked, repository plus Bitbucket with Platform options open), pool settings (repository pool with a Bitbucket workspace, picker open, Platform options open; organization pool).

### Per-section delta description

| Section | Viewport | Finding | F documented it? | Status |
|---|---|---|---|---|
| **D1.** Connections rows | 360 | The name column collapses. Measured widths: `acme-labs` 0 px (invisible; the row shows avatar, "GitHub" badge and "Organization"), `ada-admin` 20 of 67 px ("a..."), `Acme Bitbucket` 45 of 98 px ("Acm..."), `Platform Team` 45 of 90 px ("Platf..."). The old pages showed these names (`acme-...`, `ada-admin`, `Acme Bitbucket`, `Platform Team`). Cause: the name shares a line with the platform badge and `view.badges`, and the actions and status text are `shrink-0` (`connections-section.tsx:214-226`). | No | **REWORK** |
| **D1b.** Connections rows, member | 360 | "Owner approval required" is drawn over "Organization" in the GitHub row. | No | **REWORK** |
| **D2.** GitHub App section header | 360 | The title and paragraph are squeezed into a 73 px column (title wraps to two lines, paragraph about twenty lines) because the three header actions (Refresh access, Install, Manage App) are `shrink-0` beside them (`settings-ui.tsx` `SettingsSection` header, used by `github-app-section.tsx:46-52`). "Manage App" spans 327 to 442 px on a 360 px screen, so an administrator cannot reach it. The old Installations section had two actions and kept both on screen. | No | **REWORK** |
| Targets dropdown | 360, 768, 1280 | The open dropdown is clipped by its card (`absolute` list inside `overflow-hidden` `SettingsSection`): in the new-pool form and pool settings only the search box shows, not the options, group headings or disabled reasons. **Pre-existing:** the baseline's repository dropdown is clipped the same way (options hit-testable: new-pool 0 at 360, 1 at 768 and 1280; old pool settings 0 at 360 and 1280; now 0 everywhere). Not introduced by #145, but the single-row Targets card leaves less room than the old three-row card. | No | DELTA, advisory (see notes) |
| Everything else listed above | 360, 768, 1280 | Matches the brief and the wireframe intent. | n/a | MATCH |

## Spec compliance (brief acceptance criteria, round-4 decisions first)

| Criterion | Compliant | Evidence |
|---|---|---|
| Visual gate waived; no ADVISORY-HOLD for it | YES | Followed. |
| **W1** Add connection lists every platform through its descriptor (`chooserDescription`, `connectLink`) | YES | Code: `AddConnectionDialog` lists descriptors with `create` or `connectLink` whose list says `canManage`, registry order, no platform ids. Rendered: chooser shows GitHub and Bitbucket with the brief's exact copy (all three widths). GitHub step shows the description, a primary "Install GitHub App" external link (App configured) or a disabled button plus "Create the GitHub App first (below on this page)." (not configured), and Back returns to the chooser from both steps. Tests: four W1 cases. |
| `externalLinks` deleted, survey updated | YES | `grep -rn externalLinks src docs/handoffs/145/survey.md` prints nothing; the survey has the new rows and statements. |
| **W2** One generic empty state; no GitHub empty row while the App is unconfigured | YES | Rendered: administrator sees "No connections yet" + "Connect a CI platform to start assigning it to runner pools." + one Add connection button; member sees "A GridOps administrator can add platform connections." with no button; no per-platform rows under it. Code and five tests cover the loading, failed-list and configured-App cases. |
| `postCreate` failure becomes a toast, no reveal dialog | YES (code and test) | `onCreated` try/catch with `toast.error`; two tests (throws, rejects). Not exercised live: no shipped descriptor has the hook (same as the reveal dialog). |
| W3 accepted as built, W4 accepted as built with `runnerLink().label` as `title` | YES | `runner-row.tsx` renders `title={link.label}`; test `names the link with the descriptor's runnerLink label`; U confirmed `title="Open on GitHub"` on the live link. |
| Every existing flow still works; redirects; no in-app link to a redirecting route; `?appError=` | YES | Six payload-equality cases against constants frozen from `5550cf5` (unchanged). I re-ran the brief's exact redirect grep (`grep -rn -e "settings/github" -e "settings/integrations" src --exclude=routeTree.gen.ts | grep -v -E '<declaration-line pattern>'`): prints nothing. |
| No platform-specific rows or branching in shared rows | YES | `grep -niE "github|bitbucket"` over `targets-row.tsx`, `platform-options-section.tsx`, `connections-section.tsx` prints nothing. |
| Forward-compat survey with both statements | YES | `survey.md` re-read; every named field exists in `types.ts`. |
| `npm run lint`, `typecheck`, `npm test`, `npm run build` | YES | I re-ran at HEAD: `vitest` 46 files, **461/461**; lint clean (`--max-warnings=0`); `tsc --noEmit` clean; `npm run build` built and type-checked. |
| No change under `crates/`, `migrations/`, `docs/openapi/` | YES | `git diff --stat 5550cf5 HEAD -- crates migrations docs/openapi` is empty. |
| Headless walkthrough with saved screenshots at 360, 768, 1280 (operator's round-4 deliverable) | **NO (by U), done by S** | U saved none. S produced them (above). They show D1 and D2 at 360. |

## Quality audit

| Area | Result | Notes |
|---|---|---|
| API consistency | N/A | No API, DB or OpenAPI change. |
| Error handling | PASS | List failure keeps InlineError with Retry and blocks the generic empty state; Add errors stay inline; `postCreate` failures toast after close. Previous advisory (post-close error landing in dialog state) is fixed. |
| UI/UX match to spec | **FAIL at 360** | 768 and 1280 match. D1, D1b and D2 at 360 (table above). |
| Accessibility | PASS at code level | Labelled dialog steps, disabled install keeps a visible reason, the runner link keeps its visible name with a `title`. Not axe-checked (no axe setup in the repo). The off-screen "Manage App" (D2) is also a reachability problem. |
| Architecture gate | PASS | A and A-dup PASS; nothing I found conflicts. The D1/D2 fixes touch layout only. |
| Code organization | PASS | Round 4 only modified existing objects; the generic empty state and dialog sit in the shared section; descriptors supply copy. |
| Security | PASS | No new endpoint; Add stays admin-only; the install link is `target="_blank" rel="noreferrer"`; no secret in the query cache. |
| Performance | PASS | Accepted as is by the brief (runner groups load per organization installation on mount). |
| Visual regression | **FAIL at 360** | No Playwright VR suite in the repo; rendered audit above. |
| Naming consistency | PASS | `chooserDescription`, `connectLink`, nullable `emptyText` as the brief names them. |
| Test quality (`testing/test-quality.md` §7) | PASS | 16 round-4 tests: one behavior each, each failed under T's mutations (empty-state flag, `connectLink` ignored, toast swallowed, empty text always shown), cheapest sufficient tier (rendered component under jsdom; the contract case is `expectTypeOf`, gated by `tsc`). No tautological or duplicate-signal test and no "delete or merge" finding. Two notes, neither blocking: the survey-text test reads `docs/handoffs/145/survey.md`, so the suite depends on a handoff file (the brief asked for it); and **no jsdom test can see D1, D2 or the dropdown clip**, because layout does not exist there. I do not recommend a class-name assertion for them (mock-shaped); the check for these is a rendered one. |

## Scope check
F delivered the round-4 scope exactly (W1, W2, the toast, the `title`), with no new files, no API, DB or OpenAPI change and no platform names in shared code. Under-delivery: none against the written criteria. The defects D1 and D2 are in surfaces the story created or rewired (the Connections rows and the GitHub App section header), and the brief had no narrow-width criterion; the wireframe notes say narrow widths were not drawn. The operator's own visual condition and the org's mobile-first standard (360 first) are what make them blocking.

## Verdict

**REWORK**: the following must be addressed before commit. Production change only (`reworkKind: production`); no test change is required.

1. **D1 and D1b: Connections rows at 360.** In `src/features/platform-connections/connections-section.tsx` (the row at about lines 214-226 and the GitHub row's status text from `view.aside` / `view.badges`), make each row show the connection's name at 360, for example by letting the actions and status wrap below the text under the `sm` breakpoint and letting the name line wrap, instead of squeezing the name between the badges and `shrink-0` controls. Acceptance, checked in a render at 360: every name in the seeded data (`acme-labs`, `ada-admin`, `Acme Bitbucket`, `Platform Team`) is fully readable or wraps, none is cut to fewer than about 12 characters, and for a member "Owner approval required" does not overlap "Organization". 768 and 1280 must look as they do now.
2. **D2: GitHub App section header at 360.** The three actions (Refresh access, Install, Manage App; `github-app-section.tsx:46-52`) sit in the shared `SettingsSection` header (`src/components/settings-ui.tsx`: `flex items-end justify-between` with a `shrink-0` actions block). At 360 the actions must wrap below the title and paragraph (or the header must stack), "GitHub App" must not wrap into a column about 70 px wide, and "Manage App" must be fully inside the 360 px screen for an administrator. If the fix is in the shared `SettingsSection`, grep its other users first and check that their headers do not change at 768 and 1280.
3. **Verification, in the same pass.** Re-run the capture in `docs/handoffs/145/screenshots/repro/` (or an equivalent headless run) and save the 360 screenshots of the Connections page (populated administrator, populated member, empty) to `docs/handoffs/145/screenshots/`, per the operator's round-4 decision. U should do this at its re-walk; the scripts need only the API binary, a seeded throwaway database and a loopback fake of the GitHub API.

Items 1 and 2 are small CSS-level changes. After they land, the pipeline can go back through T-green, A-dup and U as before. S's non-visual result is unchanged and stands.

## Advisory notes

1. **Targets dropdown is clipped by its card (pre-existing, all widths).** `SearchableMultiSelect` draws its list `absolute` inside the `overflow-hidden` `SettingsSection` card, so in the new-pool form and pool settings the user sees only the search box; the grouped options and disabled reasons, which are the point of this story's Targets row, are not visible without the card scrolling itself. The baseline was clipped the same way, so this is not a #145 regression and not a reason to block, but it is the most user-visible issue I found. Recommend a follow-up issue (let the Targets card not clip, or portal the list), or fold it into the same F pass if the layout fix there makes it cheap. This is the one call I leave to the operator; I did not make it a blocker because it predates the story and the brief says "every existing flow still works", which holds as well as it did before.
2. U's round-4 pass left no saved evidence and did not look at the 360 App section; the operator's saved-screenshot requirement should be restated to U for the re-walk.
3. A-dup's two warns stand: the GitHub App section keeps its own Install link beside the Add connection link (same URL, different surfaces); pool list and detail pages still branch on `pool.scope === "repository"` (track for #141).
4. Carried from the last S: the new-pool form loads runner groups for every organization installation on mount (accepted by the brief); the Pool-name `pattern` console warning under Chromium's `v` flag is pre-existing and untouched.
5. Process: `pkill -f gridops-api` slip described under "How I rendered it".
