# Handoff-S: Phase 9 - One Connections page and one Targets picker for every CI platform (#145)

**Date:** 2026-11-20
**Branch:** claude/ui-145-platform-neutral-cf5832 (head `6f7c422`, U's commit; worktree clean)
**Issue:** #145 (brief: `docs/handoffs/145-brief.md`)
**Handoff-T reviewed:** `docs/handoffs/145/handoff-T-green.md` (and `handoff-T-red.md` for the frozen-constant rules)
**Handoff-A reviewed:** `docs/handoffs/145/handoff-A.md` (up-front plan review, PASS) and `docs/handoffs/145/handoff-A-dup.md` (anti-duplication gate, PASS)
**Handoff-F reviewed:** `docs/handoffs/145/handoff-F.md`
**Also read:** `docs/handoffs/145/handoff-U.md` (UI walkthrough, PASS), `handoff-D.md` + `wireframe.html` (revision 4), `survey.md`
**Operator-facing report:** none produced. The visual gate could not be run (see below), and without screenshots there is nothing to put in the comparators.

## Result in one line

**ADVISORY-HOLD (this is the schema's nearest form of CANNOT-AUDIT).** The role's mandatory Tier-3 visual gate cannot be run in this session, and the only visual reference is a low-fidelity wireframe that is not pixel-comparable. The non-visual audit is complete and found **no brief acceptance criterion failing**. It did find several wireframe deviations that F's handoff does not list, and the operator should decide whether to accept or rework them (section "Operator decisions").

## A precondition
Confirmed. `handoff-A.md` returned PASS (round 4, five warns, none blocking). `handoff-A-dup.md` returned PASS (two warns, neither a parallel path).

## T precondition
Confirmed. `handoff-T-green.md` reports zero blocking issues, 445/445 tests, and F's commands reproduced.

## Browser-tool precondition (visual / web project): NOT MET
This is a web UI project (TanStack Router SPA). The role requires the `mcp__Claude_in_Chrome__*` tool family, a tab group from `tabs_context_mcp`, and a successful `navigate`.
- That tool family is **not in this session's tool list** (I have Read, Write, Bash and the structured-output tool only).
- The operator's global instructions also forbid driving a browser that surfaces on the desktop unless asked, so the Chrome MCP path is closed by policy as well as by tool availability.
- Per the role, I did not fall back to cascade or prose reasoning for the visual verdict.

## Visual-diff-tool precondition: NOT MET, for two reasons
1. **Tooling.** ImageMagick `compare`, `magick` and `convert` are on PATH (`/opt/homebrew/bin`). Playwright is not in this worktree's `node_modules` (T also notes the repo has no Playwright setup), though Chromium builds are cached under `~/Library/Caches/ms-playwright`, so it would be installable. Capturing the live pages would also need the Rust API, a seeded DB and a minted session, which is what U did.
2. **No valid reference (the stronger reason).** The only visual source of truth for this story is `wireframe.html`, which `handoff-D.md` declares "Mode (a) generated low-fi" with "Icons are Unicode glyphs only ... the wireframe's glyphs are placeholders". The old pages are gone (redirect-only), and the new pages have no prior baseline. A pixel diff of the live app against a glyph-placeholder wireframe would report a near-total delta by construction and decide nothing. Installing Playwright would not change that, so I did not.

## Preview / spec sanity check
- **Reference not pixel-comparable** (above). Convention: a pixel-diff gate needs a high-fidelity reference or an accepted baseline. Smallest fix: either capture and commit U's live screenshots as the accepted baseline for a later regression gate, or waive the pixel diff for this story.
- **No saved visual evidence.** `handoff-U.md` says no screenshot files were saved; they live only in that session's tool output. So the operator cannot review what U saw. Smallest fix: if the walkthrough is to stand as the visual record, re-run it with screenshots saved under `docs/handoffs/145/screenshots/` at 360, 768 and 1280.
- **Viewport coverage.** U walked desktop and 360 only, on the Connections page and the new-pool form. 768 was never walked, and the pool settings and runner views were walked on desktop only.
- **Known, accepted:** the D7 state shows the schema's raw message ("Invalid input") in the "Could not create the pool" callout. The brief accepts this explicitly (wireframe open question 7), so it is not a finding.
- No heading, contrast, touch-target or breakpoint defect is visible in the wireframe's own text; I could not assess those on a render.

## Visual diff results
Not produced. No pixel counts, no diff PNGs, no wipe-slider report.

## Spec compliance (brief acceptance criteria)

Evidence below is code reading plus commands I ran at HEAD, not a render.

| Criterion | Compliant | Evidence |
|---|---|---|
| Every existing flow still works (GitHub repo pool, runner-group pool, Bitbucket pool, Bitbucket create/edit/remove, manage the GitHub App) | YES (tests + U; see note 1) | Six payload-equality cases drive the rendered forms against frozen constants; `connections-page.test.ts` covers create (inline error then retry), edit (blank token keeps the current one), remove, Refresh/Install/Manage/Create App. U reconfirmed live. |
| `/settings/integrations`, `/settings/github`, `/platform-connections` redirect (replace) to the Connections page | YES | The three route files are redirect-only and point straight at `/settings/connections`; `connections-routes.test.ts`; U walked all four redirects live. |
| No in-app link or redirect targets a redirecting route | YES | I ran the brief's grep (`settings/github` / `settings/integrations` minus the two declaration lines): prints nothing. Nav, webhooks link and `?appError=` redirect all point at `/settings/connections`. |
| `?appError=` path | YES | `settings.index.tsx`: toast guarded by `!preload`, replace-redirect to `/settings/connections` with no `search`; tests at `connections-routes.test.ts:73-95`. `grep -rn appError src` matches only `settings.index.tsx:7-9`. |
| No platform-specific rows or branching in shared rows | YES | My grep for platform ids, kind ids, `pool.scope`, `scope ===` and `bitbucketConnectionIds` over both forms, `tart-base-vm-row.tsx`, `pool-plan-preview.tsx` and `runner-row.tsx` prints nothing; `platform-neutral-source.test.ts` enforces it. Priority is gated by `prioritySupported` (a descriptor flag). |
| `runner-row.tsx` reads its badge from the registry | YES | `getPlatform(runner.platform)` for the badge, the details line and the delete-confirm name; the old ternary is gone. |
| Vitest: registry, mixing rules (a)-(e), gate cases (a)-(f), partial load failure incl. the production-registry case, permission gating, reveal dialog | YES | Test names in `platform-registry`, `pool-targets-form`, `pool-targets-rules`, `connections-page`, `targets-row-forward-compat` map one-to-one onto the criteria. I re-ran the whole suite at HEAD: 46 files, 445 tests, all pass. |
| Payload-equality constants frozen from `5550cf5` | YES | Six constants (the brief asks for five; the sixth is the manual runner-group entry), each preceded by a `5550cf5` provenance comment naming the route file and lines. Invariant (ii) grep prints nothing; `toEqual` is used and explained. `git diff b8cbf8d HEAD -- tests` is one hunk, a stub URL matcher, no constant touched (T documented it). |
| `PostCreateReveal` shape and `expectTypeOf` assertion | YES | `types.ts:16-23` matches the brief field for field; the assertion is at `platform-registry.test.ts:105`; `npm run typecheck` shows no errors at HEAD. |
| `survey.md` with the table and both statements | YES | Every row names a property that exists in `types.ts`; the #137 statement and the contract-features statement are present. |
| Forward-compat test and the platform-name grep | YES | `targets-row-forward-compat.test.ts`; `grep -niE "github|bitbucket"` over the three shared files prints nothing. |
| `npm run lint`, `typecheck`, `npm test`, `npm run build` | YES (build per T/F) | I re-ran lint, typecheck and the suite at HEAD: clean. I did not re-run the build; T and F both record it passing. |
| Headless Playwright walkthrough of every control | PARTIAL | U drove a real browser against the real API (`handoff-U.md`) and the state matrix is thorough, but there is no saved artifact and no 768 pass; whether it was headless is not stated. See "Sanity check". |

**Note 1.** Live, U never completed a successful pool create or save (only the D7 no-request case and Bitbucket 400/409 errors); the success payloads are proven only under jsdom against the frozen constants. That is what the brief specifies, so it is not a gap, but it is the reason I weight the payload-equality tests heavily.

## Wireframe conformance: deviations F's handoff does not list

F's "Deviations" section names four items. The following also differ from the approved wireframe (revision 4) or its stated defaults. None contradicts a brief criterion, and the brief wins where they conflict, but each is a visible difference a pixel diff would have surfaced.

| # | Where | Wireframe | Built | Severity |
|---|---|---|---|---|
| W1 | Add connection chooser (C1/C2, open question 6) | GitHub listed as a link-out ("connect by installing the App", disabled when the App is not configured) | Chooser lists only descriptors with a `create` binding, so Bitbucket only. `externalLinks` is declared on the descriptor and filled by GitHub but **read nowhere** in `src/` (dead field). Install is still reachable from the GitHub App section. U raised this. | DELTA, non-blocking |
| W2 | Empty states (B3/B3b, B1b, open question 9) | Generic "No connections yet" state with an Add connection CTA (admin) or "A GridOps administrator can add platform connections." (member); the GitHub empty row only while another platform has rows and the App is configured | `ConnectionRows` always draws a per-platform empty row. With nothing connected the page shows "GitHub: No installations are visible to @{login}." and "Bitbucket: No Bitbucket workspaces are connected yet.", and the GitHub row shows even when the App is not configured. Empty state is handled, just not as drawn. U did not walk the empty page. | DELTA, non-blocking |
| W3 | List chrome (B1/B2/B4) | Section heading "Connections · N"; GitHub rows show an "Active" status | Heading has no count; GitHub rows show a status only when Suspended. (Bitbucket rows have the dot plus "Verified N ago".) The brief asks for "platform badge, scope and status". | DELTA, non-blocking |
| W4 | Runner link (G2, open question 2) | "Open on {platform}" item in the runner actions menu, shown only when the builder returns a URL | No menu item. The link is the platform name in a new "CI platform" row of the details sheet. `runnerLink().label` ("Open on GitHub") is never rendered. U's report calls G2 matched, which is generous. | DELTA, non-blocking |

F's own listed deviations (first section titled "GitHub App" not "Connection"; separate "Destination" section in `ReadOnlyPool`; `limit` takes form limits not context; no "N selected across M accounts" line) are reasonable and agree with the brief or the round-3 block.

## Quality audit

| Area | Result | Notes |
|---|---|---|
| API consistency | N/A | No API, DB or OpenAPI change; `git diff 5550cf5 HEAD --stat` shows nothing under `crates/`, `migrations/`, `docs/openapi/`. `getBitbucketConnections(signal?)` hits the same endpoint. |
| Error handling | PASS | Inline create/edit errors, per-platform load error with Retry, per-kind picker load states, manual runner-group fallback, submit-time schema error with no request (D7). A `postCreate` that throws would land its message on a dialog already closed (`connections-section.tsx:62-66`); no current descriptor uses the hook, so advisory only. |
| UI/UX match to spec | NOT ASSESSED visually; code-level PASS with W1-W4 | See wireframe table. |
| Accessibility | PASS at code level, not rendered | `aria-selected`, `role=option/group`, `aria-expanded` and `aria-controls` on Platform options, labelled inputs, Escape and outside click ignored on the reveal dialog, disabled options keep a visible reason. Not checked with a browser or axe (no axe setup in the repo). |
| Architecture gate | PASS | A and A-dup both PASS. I found nothing that conflicts with either. |
| Code organization | PASS | Matches the Reuse map; `targets.ts` is pure; the three shared UI files carry no platform text. Dead field `externalLinks` (W1) and the unused `runnerLink` label (W4) are the only loose ends. |
| Security | PASS | No secret is stored in the query cache (plain call plus component state); the token input is `type=password` with `autoComplete=off`; the reveal dialog drops the value from the DOM on close; admin gating for Add, Edit, Remove, Create App and Manage App is tested; no new endpoints; server remains the authority. |
| Performance | PASS with one advisory | The new-pool form now loads runner groups for **every organization installation** at mount (`github.tsx:128-135`, `Promise.all`). The old form loaded one installation's groups, only after the user chose organization scope. N installations means N server-side GitHub calls on every form open. Brief-conformant (per-kind loads), but worth a lazy-on-open follow-up. |
| Visual regression | NOT RUN | The project has no Playwright VR suite; the role's pixel-diff gate was not run (see preconditions). |
| Naming consistency | PASS | `github.repository`, `github.runner-group`, `bitbucket.workspace`; the descriptor fields match the brief and the survey. |
| Test quality (`testing/test-quality.md` §7) | PASS | About 80 new tests across 9 files for a registry, a page, a picker, an options section, redirects and payload preservation. Each names one behavior, and the payload cases drive the rendered forms against frozen constants rather than the new code. T's three mutation spot-checks (installation from lowest `loadIndex`, `minMaxCount`, redirect target) show the tests fail when behavior is removed. The `expectTypeOf` case is assertion-free at run time by the brief's own design (typecheck is its gate). I found no tautological, duplicate-signal or coverage-padding test and no "delete or merge" candidate. Gap, not a defect: nothing asserts an empty Connections page, which is where W2 lives. |

## Scope check
F delivered the brief's scope: the neutral registry, one Connections page, one Targets row, the Platform options section, registry-driven runner badges, the redirects and the survey. Nothing under `crates/`, `migrations/` or `docs/openapi/` changed, and no GitLab code was added. Over-delivery: the "CI platform" badge on every runner row (wireframe open question 3, default drawn) and a "CI platform" details row. Under-delivery: W1-W4 above, and the saved-evidence gap in U's walkthrough.

## Verdict

**ADVISORY-HOLD.** Pipeline paused at S pending an operator decision. This is the closest schema verdict to the role's CANNOT-AUDIT; it is not a PASS and not a rework request.

- **Defect:** the role's mandatory visual gate (Chrome-MCP render plus pixel-level diff at 360/768/1280 and the operator report) cannot be performed. The Chrome MCP tools are not available to this session, and the only reference is a declared low-fi glyph wireframe that cannot be pixel-compared. U's walkthrough left no saved screenshots and skipped 768.
- **Convention violated:** a pixel-diff gate needs a comparable reference and reviewable evidence; the brief's "headless Playwright walkthrough" criterion names no place for its evidence.
- **Proposed fix (smallest):** the operator waives the pixel diff for this story on the strength of U's live walkthrough plus this audit, and records that waiver. If a stricter visual record is wanted instead, re-run U's walkthrough saving screenshots at 360/768/1280, then re-spawn S in an environment with the Chrome MCP tools.
- **Why not REWORK:** F's implementation meets every brief acceptance criterion I can check, and the environment gap is not F's.

### Operator decisions needed
1. **Visual gate:** waive the pixel diff for #145, or supply the environment and reference described above. If waived, this audit's non-visual result stands and O may proceed as if PASS.
2. **Wireframe deviations W1-W4:** accept as documented, or send a single small F rework. If reworked, the minimal set is: (W1) list GitHub in the Add connection chooser as an `externalLinks` link-out, or delete the unused field; (W2) add the generic empty state and show the GitHub empty row only when the App is configured; (W3) add the "Connections · N" count and an "Active" status on GitHub rows; (W4) add the "Open on {platform}" menu item and render the builder's `label`. None is required by the brief. I recommend accepting W3 and W4 and reworking W1 and W2 only if the operator cares about first-run UX.

## Advisory notes
1. Remove or consume the `externalLinks` descriptor field; today it is an unused contract surface that #141 would copy.
2. Consider making the runner-group list load lazily (on first open of the picker, or only once an organization group is chosen) to avoid N GitHub calls per form open.
3. `AddConnection.submit` puts a `postCreate` failure into `error` after the dialog has closed; surface it as a toast when the first descriptor uses the hook.
4. The Priority section's copy ("repositories") is GitHub-worded inside a shared row. It is gated neutrally, so not a violation, but #141 will want it supplied by the kind.
5. U's pre-existing console warning on the Pool-name `pattern` attribute (`[a-z0-9-]` under Chromium's `v`-flag validation) is real, untouched by this story, and worth a backlog item.
6. A-dup's note stands: the pool list and detail pages still branch on `pool.scope === "repository"`; track for #141.
