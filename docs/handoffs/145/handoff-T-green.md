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
