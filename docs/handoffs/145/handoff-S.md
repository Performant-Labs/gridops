# Handoff-S: Phase 9 - One Connections page and one Targets picker for every CI platform (#145), round 5

**Date:** 2026-11-20 (4:06 PM PST)
**Branch:** claude/ui-145-platform-neutral-cf5832 (head `93b59b4`; production source unchanged since F's round-5 commit `d21be14`)
**Issue:** #145 (brief: `docs/handoffs/145-brief.md`, including the binding "Round-4 operator decisions after the spec audit")
**Handoff-T reviewed:** `docs/handoffs/145/handoff-T-green.md` (round 5 section) and `handoff-T-red.md`
**Handoff-A reviewed:** `docs/handoffs/145/handoff-A.md` (up-front plan review, PASS) and `docs/handoffs/145/handoff-A-dup.md` (anti-duplication gate, round 5, PASS)
**Handoff-F reviewed:** `docs/handoffs/145/handoff-F.md` (round 5 section)
**Also read:** `handoff-U.md` (round 5 re-walk), `decisions.md`, `evidence.md`, `survey.md`, the previous `handoff-S.md` (round-4 REWORK, replaced by this file; it is in git history at `dd934c3`)
**Operator-facing report:** [`phase-9-platform-neutral-ui-report.html`](phase-9-platform-neutral-ui-report.html) (open it in a browser; the images are in [`screenshots/`](screenshots/))

## Result in one line

**PASS.** The three 360 px defects my round-4 REWORK named (D1 connection names collapsed, D1b status text drawn over "Organization", D2 "Manage App" pushed off-screen) are fixed, confirmed by my own render of the real app, and nothing else moved: at 768 and 1280, and on every other settings page and both pool forms at all three widths, the only differing pixels against the pre-fix build are the footer build string and a port number. Two non-blocking notes need an owner (below); the main one is the Targets dropdown clip, which is pre-existing and still open.

## What changed since the last S

F's round 5 (`d21be14`) changed two production files, layout classes only: `src/components/settings-ui.tsx` (`SettingsSection` header wraps its actions below the heading only when they would squeeze the title under 12rem) and `src/features/platform-connections/connections-section.tsx` (`ConnectionRow`: below `sm` the status and actions go to their own right-aligned line and the badges wrap under the name). No test, API, database or OpenAPI file changed (`git diff --stat dd934c3 HEAD -- tests` and `... 5550cf5 HEAD -- crates migrations docs/openapi` are both empty). T-green and A-dup passed; U re-walked at 360 and saved screenshots.

## A precondition
Confirmed. `handoff-A.md` PASS; `handoff-A-dup.md` round 5 PASS (two files modified in place, no new file, prop, export or dependency; three warns carried, none blocking).

## T precondition
Confirmed. `handoff-T-green.md` round 5: GREEN, no blocking issue, 461/461 tests, one mutation (`showActions = false`) failed three tests and was reverted. I re-ran at HEAD: `npm run lint` clean (`--max-warnings=0`), `npx tsc --noEmit` clean, `npx vitest run` 46 files / **461/461**, `npm run build` built and type-checked.

## Visual-diff-tool precondition
- **Pixel diff against the wireframe: waived** by the operator for this story (brief, round-4 decision), because the only reference is the low-fidelity wireframe. I did not return ADVISORY-HOLD or CANNOT-AUDIT for that. What I did instead, to satisfy the gate's intent, was a pixel diff of the fixed build against a **pre-fix build** (source identical to `dd934c3` for both changed files, checked with `diff`), so the numbers below say exactly what round 5 changed.
- **Browser tooling:** the Chrome MCP tools are not in this session's tool list, and the role says Playwright headless is the only reliable path to 360/768/1280, so I used it. Playwright (in the scratchpad, not in the repo), cached Chromium, ImageMagick `compare`/`magick` on PATH. No window opened on the desktop.

## How I rendered it
- The real `gridops-api` debug binary (backend unchanged since `5550cf5`) serving `npm run build` output of this worktree at HEAD (footer "build 464 · 93b59b4"), and a second instance serving a pre-fix `dist/` (source of `dd934c3`) for the before side.
- Throwaway SQLite databases on fresh ports (19301, 19302 fixed build; 19311, 19312 pre-fix build), seeded with `docs/handoffs/145/screenshots/repro/seed.mjs` (two users, two GitHub installations, four repositories, two Bitbucket connections, two pools, two runners; one database with the GitHub App configured, one without) and a loopback fake of the GitHub API on 19303.
- Viewports 360, 768, 1280 for the full surface sweep (tall viewports because the app scrolls inside its own container); 320, 360, 480, 639, 640, 768, 1024, 1280 for the Connections geometry measurement, as administrator and member. Scripts: `screenshots/repro/s5-cap.mjs`, `s5-measure.mjs`, `s5-make-report.mjs` (paths inside are this run's scratchpad; adjust).
- All five servers were stopped by port afterwards and confirmed down. (This time no `pkill -f`.) The throwaway databases live only in the scratchpad.

## Visual diff results

Before-fix build against fixed build, Connections page, populated data. "Delta %" is over the cropped content area (360 x 2300, 768 x 1500, 1280 x 1500).

| Viewport | Role | Pixels different | Delta % | What it is | Diff PNG | Composite |
|---|---|---|---|---|---|---|
| 360 | administrator | 27,992 | 3.381 | The intended fix (rows, GitHub App header, Add connection button reflow) | `screenshots/s5-connections-populated-admin-360-diff.png` | `...-360-composite.png` |
| 360 | member | 25,131 | 3.035 | The intended fix | `screenshots/s5-connections-populated-member-360-diff.png` | `...-360-composite.png` |
| 768 | administrator / member | 398 / 398 | 0.035 | Footer build text + port number inside the two Endpoints URLs | `screenshots/s5-connections-populated-admin-768-diff.png` | `...-768-composite.png` |
| 1280 | administrator / member | 398 / 398 | 0.021 | Same | `screenshots/s5-connections-populated-admin-1280-diff.png` | `...-1280-composite.png` |

Connections with nothing connected (administrator and member): 398 px at 360, 768 and 1280, i.e. unchanged. All counts are in `screenshots/s5-pixel-counts.csv`.

**Blast radius of the shared `SettingsSection` change** (the A-dup warn and my own round-4 instruction to grep its users): General, AI, Members, Preferences, Runner host, API tokens, new pool, pool settings (repository and organization pool), each at 360, 768 and 1280, whole page, fixed against pre-fix: **359 px at every width** (the footer string), except Members at 1280 (500 px: the footer plus a "last signed in 1m ago / 2m ago" label that ticked between the two captures). No layout change anywhere outside the Connections page. No console error on any of the 39 captures.

### Per-section delta description

| Section | Viewport | Finding | F documented it? | Status |
|---|---|---|---|---|
| Connections rows (administrator) | 360 | Names `acme-labs` 66 px, `ada-admin` 67, `Acme Bitbucket` 98, `Platform Team` 90, none clipped (before: 0, 20, 45, 45). Configure, Edit and Remove on their own right-aligned line under the text. Same widths hold at 320, 480, 639, 640, 768, 1024, 1280. | Yes (intended) | MATCH (D1 fixed) |
| Connections rows (member) | 360 | "Owner approval required" no longer overlaps "Organization" (`ownerOverlapsOrg: false` at every width); sits on its own line. | Yes | MATCH (D1b fixed) |
| GitHub App header | 360, 320 | Title block 320 px wide (before about 73); Refresh access and Install on one line and Manage App on the next, all inside the screen (my off-screen scan finds only the hidden "Skip to content" link at every width; U measured Manage App at 20 to 135 px). At 480 the actions wrap below too; at 640 and up they sit beside the title as before. | Yes | MATCH (D2 fixed) |
| Connections header, "Add connection" | 360 | The button now sits under the description instead of squeezing it into a 178 px column. | Yes | MATCH |
| Inline edit of a Bitbucket row | 360, 1280 | Edit form opens under the row, inputs 286 px wide at 360, no overflow. | n/a | MATCH |
| Add connection dialog | 360 | Chooser and Bitbucket form unchanged, no console output. | n/a | MATCH |
| Member Bitbucket rows | 360 | About 8 px of blank space under each row's text, because the descriptor's "Verified ... ago" aside is `hidden sm:inline` but its wrapper still takes the row gap. | Yes (Known issues) | DELTA, cosmetic, not blocking |
| Very long connection name (test row I added, 80 characters) | 1280 | The name is cut with an ellipsis and the "Bitbucket" badge wraps under it instead of sitting beside it, because the name line now wraps at every width, not only below `sm`. Realistic names (the seeded four) are identical to before at 768 and 1280. | No | DELTA, minor, not blocking |
| Targets dropdown (new pool, pool settings) | 360, 768, 1280 | Still clipped by the `overflow-hidden` card: only the search box and one group heading show; with the list closed at rest none of the options is hit-testable at 360 or 1280 in the new-pool form. **Pre-existing** in the shared selects (the old repository dropdown clipped the same way, measured in round 4). | n/a | DELTA, pre-existing, see Advisory note 1 |
| Everything else | 360, 768, 1280 | Pixel-identical to the pre-fix build apart from the footer string. | n/a | MATCH |

## Spec compliance (brief acceptance criteria and round-4 decisions)

Round 4's code-level audit stands, because production source differs from it only in the two files above (layout classes, no behavior, prop or type change). I re-ran the mechanical checks at HEAD instead of relying on that.

| Criterion | Compliant | Evidence |
|---|---|---|
| Visual gate waived; no ADVISORY-HOLD for it; S returns PASS when criteria hold at code level and U's saved screenshots show no defect against the brief | YES | Followed. U's screenshots (`u-*.png`, 360 plus one 1280) and F's (`f-*.png`) show no defect against the brief; the one clip they show is pre-existing and not a brief criterion. |
| W1 Add connection lists every platform through its descriptor | YES | Unchanged since round 4 (four tests); chooser rendered at 360 in this round, no console output. |
| `externalLinks` deleted, survey updated | YES | `grep -rn externalLinks src docs/handoffs/145/survey.md` prints nothing. |
| W2 one generic empty state, no GitHub empty row while the App is unconfigured | YES | Rendered at 360, 768, 1280 as administrator and member (398 px = footer and port against the pre-fix build, i.e. unchanged). |
| `postCreate` failure becomes a toast | YES | Unchanged; two tests. |
| W3, W4 accepted as built | YES | Unchanged. |
| Redirects; no in-app link or redirect to a redirecting route | YES | The brief's exact grep (`grep -rn -e "settings/github" -e "settings/integrations" src --exclude=routeTree.gen.ts | grep -v -E '<declaration-line pattern>'`) prints nothing. `grep -rln appError src` prints only `src/routes/settings.index.tsx`. |
| No platform-specific rows or branching in shared rows | YES | `grep -niE "github|bitbucket"` over `targets-row.tsx`, `platform-options-section.tsx`, `connections-section.tsx` prints nothing; `grep -nE '"github"\|"bitbucket"\|github\.repository\|scope ===\|pool\.scope\|bitbucketConnectionIds\.length'` over both pool forms, `tart-base-vm-row.tsx`, `pool-plan-preview.tsx`, `runner-row.tsx` prints nothing; no `platform ===` in `runner-row.tsx`. |
| Payload equality, registry, targets mixing rules, create-button gate, partial load failure, permission gating, reveal dialog, forward-compat test | YES | Covered by the 461 passing tests; unchanged this round (`git diff dd934c3 HEAD -- tests` empty). |
| Forward-compat survey with both statements | YES | Unchanged. |
| `npm run lint`, `typecheck`, `npm test`, `npm run build` | YES | Re-run at HEAD (above). |
| No change under `crates/`, `migrations/`, `docs/openapi/` | YES | `git diff --stat 5550cf5 HEAD -- crates migrations docs/openapi` is empty. |
| Headless walkthrough with saved screenshots at 360, 768, 1280 (operator's round-4 deliverable) | YES | Round-4 S set (`s-*`, all three widths, all surfaces) plus F's and U's 360 sets after the fix, plus this round's `s5-*` and `t3-*` files. My round-4 REWORK item 3 (360 Connections shots: populated administrator, populated member, empty) is met by F's `f-connections-*-360.png`, U's `u-connections-populated-*-360.png` and my `s5-*` set. |
| S's round-4 REWORK acceptance for D1/D1b | YES | Every seeded name fully readable (66/67/98/90 px, none under 12 characters), no overlap for the member, 768 and 1280 look as before. |
| S's round-4 REWORK acceptance for D2 | YES | "GitHub App" is no longer a ~70 px column, "Manage App" fully inside the 360 px screen; `SettingsSection` users grepped and their headers checked at 768 and 1280 (table above). |

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | N/A | No API, DB or OpenAPI change. |
| Error handling | PASS | Unchanged; the `showActions` hoist is the same expression as before, so Edit and Remove stay gated by `canManage && !editing`. |
| UI/UX match to spec | PASS | D1, D1b, D2 fixed; 768/1280 unchanged; the Targets dropdown clip is pre-existing (Advisory 1). |
| Accessibility | PASS at code level | No role, label or focus-order change; "Manage App" is reachable again. Not axe-checked (no axe setup in the repo). |
| Architecture gate | PASS | A and A-dup PASS; nothing I found conflicts. The edit was made in the shared `SettingsSection` and `ConnectionRow`, not beside them. |
| Code organization | PASS | Two files, layout classes and one regrouped wrapper; one explanatory comment in each; no TODO, FIXME, console output or dead code. |
| Security | PASS | No new endpoint or input; no secret touched. |
| Performance | PASS | Static classes only. |
| Visual regression | PASS | No Playwright VR suite in the repo; rendered audit above (fixed against pre-fix build, 13 surfaces x 3 widths plus 8-width Connections geometry). |
| Naming consistency | PASS | Unchanged. |
| Test quality (`testing/test-quality.md` §7) | PASS | Round 5 adds no test and needs none: D1/D2 are layout, and layout does not exist under jsdom, so any assertion would be a class-name check (mock-shaped). T's mutation (`showActions = false`) failed three behavior tests, so the restructured wrapper is still pinned for Edit/Remove. The suite is proportionate (461 tests for a feature story with a tiered structure), with no tautological, duplicate-signal or coverage-padding test found in the round-4 audit and none added since. No "delete or merge" finding. Standing note, unchanged: the survey-text test reads `docs/handoffs/145/survey.md`, so the suite depends on a handoff file (the brief asked for it). |

## Scope check
F delivered exactly the round-5 scope: the two production files S named, no test, API, database or OpenAPI change, no new file, no platform name in shared code. Over-delivery: none (the extra evidence files are under `docs/handoffs/145/`). Under-delivery: none against the criteria. F did not fold in the Targets dropdown clip, which S's round-4 note left as the operator's call and which F correctly declined as a separate, larger change.

## Verdict

**PASS** — all acceptance criteria met, spec-compliant, quality acceptable. Ready for O to commit and open the pull request. No rework is required; `reworkKind` does not apply.

## Advisory notes

1. **Targets dropdown clipped by its card (pre-existing, all widths; needs an owner).** `SearchableMultiSelect` and `SearchableSelect` draw their list `absolute` inside the `overflow-hidden` `SettingsSection` card (`searchable-multi-select.tsx:223`, `searchable-select.tsx:186`; `settings-ui.tsx:38`). In the new-pool form and pool settings only the search box and one group heading show when the picker opens; in my render none of the options was hit-testable at 360 or 1280 (Playwright can still click one, because it scrolls the clipped container programmatically; a mouse user cannot). The old repository picker was clipped the same way, so this is not a #145 regression and not a brief criterion, which is why it does not block. It is, however, the most user-visible fault on the story's central control. It was named in my round-4 note and F's round-5 note as needing a follow-up issue; nobody has filed one. I cannot file it from this role: **O (or the operator) should file it before or with the pull request**, scoped to the shared selects (portal the list, or stop the card clipping), because every settings page sits behind the same `SettingsSection` card.
2. **Cosmetic:** member Bitbucket rows at 360 carry about 8 px of blank space under the text (empty trailing wrapper, `gap-y-2`). The `3rem` in `flex-[1_1_calc(100%-3rem)]` stands in for the width of the leading avatar or dot; it works for both today (28 px avatar + 12 px gap, 8 px dot + 12 px gap) but would need a look if a descriptor supplies a wider `leading` element. A name of about 80 characters wraps its badge under it at 1280 (see the table). None of these is worth a rework cycle.
3. **Carried from A-dup:** the GitHub App section keeps its own Install link beside the Add connection link (same URL, different surfaces), and the pool list and detail pages still branch on `pool.scope === "repository"` (track for #141).
4. **Carried:** the new-pool form loads runner groups for every organization installation on mount (accepted by the brief); the Pool-name `pattern` console warning under Chromium's `v` flag is pre-existing and untouched.
5. **Process:** U's round-5 re-walk used fake GitHub App credentials, so its lazy repository fetch returned a 500 (an environment limit it documented). My run used a loopback fake of the GitHub API and saw no console errors at all.
