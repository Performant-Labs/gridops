# Handoff-T-green: Phase 7 - GitLab in the shared connections and targets UI (#141)

**Date:** 2026-11-20
**Branch:** claude/gitlab-141-ui-3ae7f5
**Issue:** #141 (epic #136)
**Handoff-F reviewed:** docs/handoffs/141/handoff-F.md
**Handoff-T-red:** docs/handoffs/141/handoff-T-red.md

## Test-only repair (mine)
F flagged two failing tests in `tests/connections-page.test.ts`. I confirmed F's diagnosis: the GitLab list stub always answered `canManage: true`, and Add connection is offered for any registered platform whose list reports `canManage` (unchanged #145 logic). The server reports `canManage` per user, so the stub was wrong, not production. Fix: the stub now answers `{ canManage: bitbucketList.canManage, items: [] }`, so each test's per-role permission applies to both platforms. Only the stub changed (3 lines added, 2 removed). No production file was touched. `tests/targets-row-forward-compat.test.ts` needs no change (the new `PlatformOptionsSection` props are optional).

## GREEN confirmation
```
$ npx vitest run
 Test Files  51 passed (51)
      Tests  548 passed (548)
```
All 87 RED tests plus every pre-existing test pass. The RED run (87 failures, each on the feature's own assertion, see handoff-T-red.md) is the isolation check: each test failed with the behaviour absent and now passes with it present. I did not re-mutate production code.

## Tier 1 results
| Check | Command | Result |
|---|---|---|
| Build | `npm run build` (vite + `tsc --noEmit`) | PASS |
| Types | `npx tsc --noEmit` | PASS (exit 0) |
| Lint | `npm run lint` (`--max-warnings=0`) | PASS |
| Rust format | `cargo fmt --check -p gridops-api` | PASS |
| Backend GitLab | `cargo test -p gridops-api gitlab` | PASS (34 + 3) |
| Backend runners | `cargo test -p gridops-api runner` | PASS (11 + 2 + 19) |
| Full gridops-api | `cargo test -p gridops-api` | 92 + 36 passed; fleet_http 143 passed, 1 failed (see below) |
| Server/API smoke | the runners-list integration test drives the real router over SQLite | PASS |

F's reported commands reproduce with the same results, except the Vitest count, which is now 0 failures after my stub repair.

The one failure, `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`, is environmental and unrelated to #141. It asserts that the host has no macOS ARM64 machine, and this run is on a macOS ARM64 Mac. The change touches nothing under `crates/gridops-api/tests` and nothing the release preflight reads.

## Tier 2 results
- **Coverage per criterion:** PASS, see below.
- **Test quality (§7):** PASS. Each test names one behaviour, and the RED run proved each fails in isolation for the feature's assertion. Tiers: pure descriptor/schema logic is unit, component wiring is jsdom, the SQL join is one backend integration test. The `it.each` validate cases are table rows of one rule, not duplicates. I found nothing to delete or merge. The suite is large (87 tests) but matches a 15-file feature that spans a contract, two routes and a backend join.
- **Type safety:** PASS. `tsc` clean. The only `any` in the diff is the pre-existing `postCreate(created: any)` line, carrying its existing `eslint-disable` and rationale comment; only its return type was renamed.
- **Error handling:** PASS. Tests cover server rejection shown verbatim (create, Verify), a declined rotate confirm sending nothing, inline validation blocking submit, and an older server missing `gitlabConnections`.
- **Data integrity:** PASS. The LEFT JOIN yields nulls for a GitHub runner or a deleted connection (`ON DELETE set null`); the test pins the present-and-null keys.
- **API contract:** PASS. The `POST` create body, the `PATCH` update body (token only when typed) and the pool wire fields are pinned. `GET /api/v1/runners` is not in the OpenAPI document, so there is no spec drift.
- **Security:** PASS. The token is a `password` field, and the secret and webhook URL leave the DOM when the reveal closes. Client validation is a convenience; the server owns authorization (`canManage` per user, member sees no actions).
- **Migration safety:** N/A. No migration is added; `0047_gitlab.sql` already exists.
- **Playwright:** N/A. The project has no Playwright config. Live-UI behaviour (Alpine-style init, real SPA navigation, `SearchableSelect` and `Switch` in a real browser) is invisible to these jsdom tests; U should walk the connection form, the reveal, the pool forms and the deliveries page.
- **Evidence appendix:** PASS. F's `evidence.md` covers the unchanged-code facts the tests rely on. My one change relies on nothing outside the diff, so I added no entries.

## Acceptance criteria status
- GitLab connection form, fields, defaults, validation: PASS (gitlab-descriptor, gitlab-connections).
- Create reveal: secret, webhook URL with Copy, Job events steps; cleared on close: PASS (gitlab-connections).
- Row actions Verify and Rotate (confirm first), order Edit, Verify, Rotate, Remove, hidden for members: PASS (gitlab-connections, connections-page).
- Target kinds `gitlab.project` / `gitlab.group`, picker, options, exclusion with GitHub/Bitbucket: PASS (gitlab-descriptor, gitlab-pool-forms).
- Pool create and update payloads, `runUntagged` switch, no GitHub-only fields on a GitLab pool: PASS (gitlab-descriptor, gitlab-pool-schema, gitlab-pool-forms).
- Tag rule: PASS (gitlab-descriptor, gitlab-pool-schema).
- Deliveries platform badge, no false "GitHub App": PASS (webhook-delivery-platform).
- Runner link and badge: PASS (gitlab-descriptor, runner-row-platform, backend `the_runners_list_carries...`).
- Neutral-source (GitLab only in its allowlisted files): PASS (platform-neutral-source).
- Registry order and lookup: PASS (platform-registry).

## Blocking issues
None.

## Advisory notes
- F's deviations (plan-note wording, per-kind "not connected" copy) are not pinned by exact copy in the tests beyond what the brief fixes; U/S should compare them with the wireframe.
- Known, tracked elsewhere: polling errors (#148) not listed; GitLab pool and runner still show under the GitHub login in the pool list, the pool detail and the runner Repository column (#159); a GitLab-only create with no GitHub installation sends `installationId: 0` and shows the schema message.
- Not run: no live browser. Hand off to U (this is a UI surface).

---

# Handoff-T-green: Re-verification after S REWORK (cycle 1) - clipped Edit on narrow GitLab rows

**Date:** 2026-11-20
**Branch:** claude/gitlab-141-ui-3ae7f5
**Issue:** #141
**Handoff-F reviewed:** docs/handoffs/141/handoff-F.md (second section, "Rework after S")

## What changed
F's rework is one production class change in `src/features/platform-connections/connections-section.tsx` (action cluster: `flex shrink-0 items-center gap-1` became `flex min-w-0 flex-wrap items-center justify-end gap-1`, plus a comment). No GitLab string. No test file changed by F or by me; F reported no wrong tests and I found none.

## GREEN confirmation (re-run by me at HEAD)
| Check | Command | Result |
|---|---|---|
| Suite | `npx vitest run` | PASS, 51 files, 548/548 |
| Types | `npx tsc --noEmit` | PASS (exit 0) |
| Lint | `npm run lint` (`--max-warnings=0`) | PASS |
| Build | `npm run build` | PASS (exit 0) |
| Backend runners list | `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api the_runners_list_carries` | PASS (1) |
| Backend GitLab | `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api gitlab` | PASS (34 + 3) |

All match F's reported results. `tests/platform-neutral-source.test.ts` (inside the 548) stays green, so the generic class change adds no GitLab name to the shared file.

## Tier 2
- **Test quality / coverage:** unchanged and PASS. The defect is layout (wrapping and clipping of a flex cluster), which does not exist under jsdom, so no Vitest can pin it at a sufficient tier. Adding a class-string assertion would test implementation, not behavior, so I added none. Per-button geometry is covered by F's measured Playwright table (320-1280) and is for U's 360 px re-walk and S's re-audit, as S asked.
- **Playwright suite:** N/A (the project has no Playwright config).
- **Behavior regressions:** the existing component tests that click Edit, Verify, Rotate and Remove (gitlab-connections, connections-page) still pass, so the class change did not alter the action order, labels or handlers.
- **Evidence appendix:** F's rework relies on nothing outside the diff; nothing to add.

## Blocking issues
None.

## Advisory notes
- Live layout at 360 px is unproven by headless jsdom; U must re-walk with per-button geometry (not page-level overflow).
- S's cosmetic note (badge wraps and path truncates at 768 and 1280) remains open by design.
