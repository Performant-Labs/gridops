# Handoff-T-green: Phase 7 (GREEN + Tier 2) - One Connections page and one Targets picker for every CI platform (#145)

**Date:** 2026-11-20 (about 2:10 PM PST)
**Branch:** claude/ui-145-platform-neutral-cf5832 (on top of `4bdc58a`, F's commit)
**Issue:** #145
**Handoff-F reviewed:** `docs/handoffs/145/handoff-F.md` ("Tests that look wrong (for T)": None)
**Handoff-T-red:** `docs/handoffs/145/handoff-T-red.md`

## GREEN confirmation
    $ npx vitest run  -> Test Files 46 passed (46), Tests 445 passed (445)

`git diff b8cbf8d HEAD --stat -- tests crates migrations docs/openapi` is empty: F touched no test, backend, migration or OpenAPI file.

Mutation spot-checks (production edited, suite run, edit reverted; `git status` afterwards shows only my test change below):
- GitHub create partial takes `installationId` from first-clicked instead of lowest `loadIndex` -> `pool-target-payloads` "click order kept, installation from the loaded list's first selected repository" FAILS.
- `minMaxCount` returns a constant 1 -> `pool-targets-form` "raises the Maximum floor for selected repositories only" FAILS.
- `/settings/github` redirects to `/settings` -> `connections-routes` "/settings/github replace-redirects to /settings/connections" FAILS.

## Test repair made this phase (test-only)
`tests/pool-target-payloads.test.ts` line 105: the Bitbucket list stub matched the URL as the literal string `/api/v1/platform-connections/bitbucket`. That is an API path, not an import, but it made the brief's invariant (ii) grep `grep -nE "runner-pools/targets|platform-connections/(registry|github|bitbucket|types)" <file>` print one line instead of nothing. I changed the stub to the equivalent regex `/^\/api\/v1\/platform-connections\/bitbucket$/.test(url)`. The grep now prints nothing, the six cases still pass, and no frozen constant or provenance comment was touched. Consequence for the review gate: `git diff <T-red commit> HEAD -- tests/pool-target-payloads.test.ts` is no longer empty; the only hunk is that one stub line, and it exists to satisfy invariant (ii).

## Tier 1 results
| Check | Result |
|---|---|
| `npm run build` (vite build, then tsc) | PASS, built, exit 0 |
| `npm run lint` (`--max-warnings=0`) | PASS, no output |
| `npm run typecheck` | PASS |
| `npx vitest run` (full, existing tests included) | PASS 445/445 |
| Redirect grep (`settings/github`, `settings/integrations`, minus the two declaration lines) | PASS, prints nothing |
| `grep appError src` | PASS, only `settings.index.tsx:7-9` |
| Platform-name grep in `targets-row.tsx`, `platform-options-section.tsx`, `connections-section.tsx` | PASS, prints nothing |
| Server start / API smoke | N/A: no API change (nothing under `crates/`, `migrations/`, `docs/openapi/`); the forms and pages are exercised under jsdom against stubbed `fetch` |

F's reported commands all reproduced with identical results.

## Tier 2 results
- **Coverage per criterion:** PASS (see below).
- **Test quality:** PASS. Each file names a behavior; the payload cases drive the rendered forms against frozen constants; the forward-compat file uses an injected descriptor; the source-scan file turns the brief's greps into tests. Mutation checks above show the tests fail when behavior is removed. Proportion is about 80 tests for a registry, page, picker, options section, redirects and payload preservation; I found no redundant case worth merging.
- **Type safety:** PASS with one advisory. The only `any` in changed source is `postCreate?: (created: any)` in `types.ts:204`, with a justifying comment and a scoped eslint-disable (the created value's type is per-descriptor). The other disable is `react-hooks/exhaustive-deps` in `targets-row.tsx:59`.
- **Error handling:** PASS. Inline create errors, per-platform load error with Retry, partial-load failure, an unavailable runner-groups list (manual entry) and the Bitbucket-only submit (zod rejects, no POST) are each tested.
- **Data integrity / migration:** N/A, no schema or backend change.
- **API contract:** PASS. Payload-equality cases pin the request bodies at `createRunnerPoolAction` / `updateRunnerPoolAction` to the frozen `5550cf5` constants.
- **Security:** PASS. No new endpoints; admin/member gating of Add, Edit, Remove, Create App and Refresh is tested; the secret reveal dialog ignores Escape/outside click and removes the value from the DOM on close.
- **Playwright:** the project has no Playwright setup (`package.json` has no reference), so there is no e2e suite to run and nothing to count as skipped.

## Acceptance criteria status
All criteria that Vitest can back are PASS with the tests listed in `handoff-T-red.md` (registry/rules/payload/form/forward-compat/connections-page/routes/source-scan/runner-row/tart-image files). The brief's final criterion, a headless browser walkthrough of every control, has no automated test (not in T's suite, and the project has no browser runner). It is the job of U. Surfaces U must walk live because only jsdom covers them: per-kind picker load states (loading, error with Retry, not connected), the Add connection dialog's Back button, the runner details "CI platform" link, `/settings/connections` reached by SPA navigation, and the three old-URL redirects.

## Blocking issues
None.

## Advisory notes
- `postCreate`'s `any` parameter could be tightened to a generic descriptor type later; not blocking.
- Evidence appendix: F's `evidence.md` already covers the unchanged source facts my tests rely on (`api()` signal/ApiError, zod parse in `createRunnerPoolAction`, `confirmAction` fallback, `useAction` router invalidate, `TooltipProvider`); I added none.


---

# Handoff-T-green, Round 4: Add connection offers every platform; one generic empty state (#145)

**Date:** 2026-11-20 (about 3:15 PM PST)
**Branch:** claude/ui-145-platform-neutral-cf5832 (on top of `8c961a9`, F's round-4 commit)
**Issue:** #145
**Handoff-F reviewed:** `docs/handoffs/145/handoff-F.md`, section "Round 4" ("Tests that look wrong (for T)": None)
**Handoff-T-red:** `docs/handoffs/145/handoff-T-red.md`, section "Round 4"

## GREEN confirmation
    $ npx vitest run  -> Test Files 46 passed (46), Tests 461 passed (461)

The 16 round-4 tests (15 behavioural, 1 typecheck-gated) all pass, plus the 445 earlier ones. `git diff 2fb904c HEAD --stat -- tests crates migrations docs/openapi` is empty: F touched no test, backend, migration or OpenAPI file. I changed no test this phase.

Mutation spot-checks (production edited, suite run, edit reverted; `git status` is clean afterwards):
- `nothingConnected` forced false -> 4 empty-state tests FAIL (admin, member, loading, failed-then-retry).
- `addable` ignores `connectLink` -> 4 W1 tests FAIL (chooser lists GitHub, install link step, disabled install, registry-driven LinkCI).
- `postCreate` catch swallows the error instead of `toast.error` -> both toast tests (throws, rejects) FAIL.
- Per-platform empty text rendered regardless of `showEmpty` -> the admin and member generic-empty-state tests FAIL.

## Tier 1 results
| Check | Result |
|---|---|
| `npm run typecheck` | PASS (this gates the `expectTypeOf` contract test) |
| `npm run lint` (`--max-warnings=0`) | PASS, no output |
| `npm run build` (vite build, then tsc) | PASS, built in 1.16s |
| `npx vitest run` (full) | PASS 461/461 |
| Platform-name grep in `targets-row.tsx`, `platform-options-section.tsx`, `connections-section.tsx` | PASS, prints nothing |
| `grep -rn externalLinks src docs/handoffs/145/survey.md` | PASS, prints nothing |
| Redirect grep (`settings/github`, `settings/integrations`, minus the two declaration lines) | PASS, prints nothing |
| Server start / API smoke | N/A: no API change; exercised under jsdom against stubbed `fetch` |

F's reported commands all reproduced with identical results.

## Tier 2 results
- **Coverage per criterion:** PASS (below).
- **Test quality:** PASS. Each case names one behaviour and fails for the behavioural reason (mutations above). Tier is the cheapest sufficient: a rendered-dialog component test for W1/W2 and the toast, unit/type tests for the contract. No redundancy worth merging. Advisory: the survey-doc test reads `docs/handoffs/145/survey.md`, which couples the suite to a handoff file; the brief asks for it, so kept.
- **Type safety:** PASS. No new `any` in the round-4 diff; `connectLink` and `emptyText` signatures are pinned by `expectTypeOf` and `tsc`.
- **Error handling:** PASS. A failed list shows InlineError with Retry and blocks the generic empty state; a `postCreate` throw or rejection becomes a toast with no reveal dialog; a `create` error stays inline in the dialog.
- **Data integrity / migration:** N/A, no schema or backend change. F's note about keeping fetching rows mounted under the generic empty state (so a post-create `invalidateQueries` refetches) is sound; the reload-after-create path is covered by the existing create case.
- **API contract / Security:** PASS. No new endpoint; Add connection stays hidden from non-admins (existing test) and the chooser lists only `canManage` platforms; the install link is `rel="noreferrer"` `target="_blank"`.
- **Playwright:** the project has no Playwright setup, so no e2e suite exists and nothing is skipped.
- **Evidence appendix:** `evidence.md` "Round 4" already holds the unchanged-source facts the round-4 tests rely on (`githubApp: null` in the page type, `InlineError` `role="alert"`, `Callout` role by tone, `EmptyState` defaults). I copied the first two against source: they match verbatim. Added none.

## Acceptance criteria status
- W1 chooser lists GitHub then Bitbucket with descriptions: PASS (`lists GitHub then Bitbucket ...`).
- W1 GitHub link step (installUrl, new tab, noreferrer), Back from link and form: PASS (`GitHub's step links ...`).
- W1 disabled install plus unavailable text with no App: PASS.
- W1 registry-driven (`connectLink`-only listed, neither-binding not listed): PASS.
- W1 `externalLinks` removed, `chooserDescription` on every descriptor, contract types: PASS (`platform-registry` cases, `tsc`).
- W2 generic empty state for admin and member with the exact copy: PASS.
- W2 no GitHub empty row while the App is unconfigured; row shown when configured: PASS.
- W2 not shown while loading or after a failed list: PASS.
- `postCreate` failure toast, no reveal dialog: PASS (2 cases).
- Runner link `title` equals the descriptor label: PASS.
- Survey names `chooserDescription` / `connectLink`, not `externalLinks`: PASS.
- `platform-neutral-source` guard on `connections-section.tsx`: PASS.

## Blocking issues
None.

## Advisory notes
- Not covered by jsdom and left to U: the visual layout of the chooser entries and the generic empty state at 360/768/1280, the new-tab behaviour of the install link, and the chooser reached by SPA navigation.
- F's two deviations (one-paragraph GitHub description instead of the wireframe's description plus callout; neutral admin empty copy without the GitHub/Bitbucket sentence) follow the brief's round-4 text and are not defects.

---

# Round 5 (S rework: D1, D2 at 360 px) - GREEN

**Date:** 2026-11-20 (PST)
**Branch:** claude/ui-145-platform-neutral-cf5832 (head `d21be14`, F's round-5 commit)
**Issue:** #145
**Handoff-F reviewed:** `docs/handoffs/145/handoff-F.md` ("Round 5"); input was `handoff-S.md` (REWORK, production only)

## GREEN confirmation
F's round 5 changed two production files (`src/components/settings-ui.tsx`, `src/features/platform-connections/connections-section.tsx`), layout classes plus one restructure of the trailing status/actions wrapper. No test file changed (`git diff dd934c3 HEAD -- tests` is empty). F flagged no wrong tests.

| Command | Result |
|---|---|
| `npx vitest run` | 46 files, 461/461 passed (same as F reported) |
| `npm run lint` | clean (`--max-warnings=0`) |
| `npx tsc --noEmit` | exit 0 |
| `npm run build` | built; typecheck step clean |

Spot-check (behavior, not implementation): I temporarily forced `showActions = false` in `ConnectionRow`. Three tests failed (`shows Add connection, Edit and Remove ...`, `edits only the editable fields ...`, `removes a workspace ...`), so the restructured wrapper is still pinned for Edit/Remove. Reverted; the tree is clean.

## Tier 2 results
- **Test coverage / quality:** PASS. The brief's criteria are still each backed by a test; round 5 adds no behavior, so no new test is owed. D1, D2 and the dropdown clip are layout and cannot be seen under jsdom; a class-name assertion would be mock-shaped, so I am not adding one (agrees with S and F). They are left to U's rendered re-walk. No redundant test found.
- **Type safety:** PASS. No prop, type or export change; no new `any`.
- **Error handling / data integrity / migration / API contract:** N/A or unchanged. No API, DB or OpenAPI change.
- **Security:** PASS. Edit/Remove stay gated by `canManage && !editing`; the condition is only hoisted into `showActions`, same expression.
- **Shared-code blast radius:** `SettingsSection` is shared. F listed its `actions` users (fleet-profile Save, Connections Add, GitHub App) and reported 768/1280 pixel-identical to the pre-fix build; those pixel checks are F's, not re-run by me (no headless browser run in T). Flag for U.
- **Playwright:** none in this project; nothing skipped.
- **Evidence appendix:** F's round-5 entries cover the unchanged-source facts (other `SettingsSection` users, 20 px page padding, Bitbucket's `hidden sm:inline` aside). My tests rely on none of these. No additions.

## Acceptance criteria status
All earlier criteria PASS, unchanged (461 tests). S's D1/D1b/D2 are layout criteria checked by render: F reports 360 measurements fixed (names 66/67/98/90 px, none clipped; "Manage App" on screen; no overlap), unverified by T; U must re-walk at 360 and 768/1280.

## Blocking issues
None.

## Advisory notes
- Known, accepted by F: member Bitbucket rows are about 8 px taller at 360; the Targets dropdown clip is pre-existing and needs a follow-up issue (not filed by T).
- The suite still depends on `docs/handoffs/145/survey.md` (survey-text test); unchanged advisory.
