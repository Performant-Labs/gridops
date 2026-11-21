# Handoff-S: Phase 9 - Pool list and pool detail take the destination from the platform registry (#159)

**Date:** 2026-11-21, 1:57 AM PST
**Branch:** claude/gitlab-159-pool-destination (head `f591763`; production source, tests and crates last changed in F's `4db2892`, so nothing under `src`, `tests` or `crates` differs between `4db2892` and head)
**Issue:** #159 (epic #136). Brief: `docs/handoffs/159-brief.md`
**Handoff-T reviewed:** `docs/handoffs/159/handoff-T-green.md` (and `handoff-T-red.md`)
**Handoff-A reviewed:** `docs/handoffs/159/handoff-A.md` (up-front plan review, PASS) and `handoff-A-dup.md` (anti-duplication, PASS)
**Handoff-F reviewed:** `docs/handoffs/159/handoff-F.md`
**Also read:** `handoff-U.md` (PASS), `handoff-D.md`, `wireframe.html`, `decisions.md`, `evidence.md`, the diff `e022a5d..HEAD` over `src`, `tests` and `crates`
**Operator-facing report:** [`phase-9-pool-destination-report.html`](phase-9-pool-destination-report.html) (open it in a browser; images are in [`screenshots/cycle-1/`](screenshots/cycle-1/); the scripts that produced them are in [`screenshots/repro/`](screenshots/repro/))

## Result in one line

**PASS.** The implementation matches the brief, A's plan and the wireframe. GitHub and Bitbucket pools are pixel-identical to the build before the change at 360, 768 and 1280 wide, in both views. A GitLab pool shows its connections and the GitLab icon, an unknown scope shows a neutral fallback, and nothing outside the destination area changes position or size.

## A precondition
Confirmed. `handoff-A.md` is PASS (five warns, none blocking; F adopted warns 1 to 4 and left warn 5 alone as out of scope). `handoff-A-dup.md` is PASS (no duplication, no drift).

## T precondition
Confirmed. `handoff-T-green.md` reports "Blocking issues: None": vitest 52 files / 582 passed, eslint clean, `npm run build` (vite + tsc) exit 0, `cargo fmt --check` exit 0, `cargo test -p gridops-api` 123 + 36 + 143 passed with one environment-dependent failure (`release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`, which fails because the fixture agent reports the compile OS on this arm64 Mac; the script never reads `/runner-pools`). I did not re-run Tier 1 or Tier 2: the tree has not changed since (`git diff --stat 4db2892 HEAD -- src crates tests` is empty).

## Visual-diff-tool precondition
- **Browser tooling.** The `mcp__Claude_in_Chrome__*` tools are not in this session's tool list. As in #141 and #145 I did not return CANNOT-AUDIT: the role says the Chrome MCP viewport is locked and headless Playwright is the only reliable path to 360/768/1280, and the operator's standing rule is headless-only. I rendered the real pages with headless Playwright 1.65.1 (Chromium from the local cache; the npm package is installed in the scratchpad, not in the repo) and ImageMagick 7.1.2-35 (`compare`, `magick`). No window opened. This substitution is mine and the operator may overrule it.
- **Reference.** The only design reference is the low-fidelity wireframe (`handoff-D.md`, Mode (a), a "GL" stand-in for the icon), so a pixel diff against it decides nothing. As in #141 and #145 I diffed this head against the **build before the change** (`e022a5d`, whose source is identical to master tip `99375fb`; `git diff --stat 99375fb e022a5d -- src crates tests` is empty). That is the right reference for the brief's strongest claim ("GitHub and Bitbucket pools look exactly as they do now"). For the new GitLab and unknown-scope states I checked copy and layout against the wireframe by eye (below). **No operator waiver of a wireframe pixel diff is recorded for #159**; flagged for the operator.
- **How I rendered it.** `git archive` of each commit into the scratchpad (nothing in the worktree touched), `vite build` of each with the build number and commit pinned so the version bar matches. Both served by the same API binary (`target/debug/gridops-api`, built from `4db2892`, which is the head's crates) against two identically seeded throwaway SQLite databases, on loopback ports 19411 (before) and 19412 (after), with a hand-minted session cookie (GridOps has no headless login; same approach as U). The seed has nine pools: GitHub repository (1, 3 and 9 repositories), GitHub organization, a Bitbucket pool (a GitHub-scoped pool with a Bitbucket connection), GitLab with one, two and no connections, and one unknown scope (`azure-devops`). Clock fixed, GitHub avatar requests answered with one fixed image, 360x800, 768x1024 and 1280x800, deviceScaleFactor 1. The app scrolls inside its own container, so detail pages were captured with the Destination group in view (page top at 1280, page bottom at 360 and 768, where the group sits below the fold). Both servers were stopped afterwards by PID; the databases and build outputs are in the scratchpad and are not committed.

## Preview / spec sanity check
No defect in the source of truth, so no ADVISORY-HOLD. The wireframe has one `h1` and `h2` frames (no skipped level), adds no interactive control (no touch-target or focus-order question), reuses the existing row and chip tokens (no new contrast pair), and its copy (`—`, "Unknown destination", `GitLab`, `N connections`, "No connections listed", Scope "GitLab" and "Unknown") contradicts no convention. One process note: `handoff-D.md`'s approval line is still unfilled ("To be filled by O"); A and T treated the copy defaults as approved, and the live UI matches them exactly, so the operator can overrule any copy line without a code-structure change.

## Visual diff results

Pixels different (and share of the viewport), build before the change versus this head. "Dest" states are the detail page with the Destination group in view; "top" states are the page as it loads. Per-viewport diff PNGs, composites and wipe-slider comparators are in the report; file names are `screenshots/cycle-1/t3-<state>-<viewport>-{live,reference,diff,composite}-20261121.png`.

| State | 360x800 | 768x1024 | 1280x800 |
|---|---|---|---|
| `list` (nine pools) | 0 (0%) | 0 (0%) | 769 (0.075%) |
| `gh-org` (top) | 0 (0%) | 0 (0%) | <1 (0.000%) |
| `gl-two` (top) | 0 (0%) | 2,330 (0.296%) | 2,368 (0.231%) |
| `gh-many-dest` (GitHub repository, 3 repositories) | 0 (0%) | 0 (0%) | 0 (0%) |
| `gh-nine-dest` (9 repositories, "Show 3 more") | 0 (0%) | 0 (0%) | 0 (0%) |
| `gh-nine-expanded-dest` (9 repositories, "Show fewer") | 0 (0%) | 0 (0%) | 0 (0%) |
| `gh-org-dest` (GitHub organization) | 0 (0%) | 0 (0%) | 0 (0%) |
| `bb-dest` (Bitbucket, GitHub-scoped) | 0 (0%) | 0 (0%) | <1 (0.000%) |
| `gl-one-dest` (GitLab, 1 connection) | 6,847 (2.377%) | 10,243 (1.302%) | 2,129 (0.208%) |
| `gl-two-dest` (GitLab, 2 connections) | 6,513 (2.261%) | 10,488 (1.334%) | 2,368 (0.231%) |
| `gl-zero-dest` (GitLab, no connections) | 6,650 (2.309%) | 9,866 (1.255%) | 1,645 (0.161%) |
| `unknown-dest` (unknown scope) | 5,826 (2.023%) | 10,117 (1.286%) | 1,862 (0.182%) |

The `<1` entries are a single 2 by 2 pixel patch at one rounded panel corner that differs by one shade (re-captured twice: it appears in one run and not the other, so it is renderer noise, not a change).

### Per-section delta description

| Section | Viewport | Status | What is different |
|---|---|---|---|
| List, destination cells of the five GitHub and Bitbucket rows | 1280 | MATCH | Nothing: same avatar, text, spacing. |
| List, destination cells of the GitLab (1, 2, none) and unknown-scope rows | 1280 | DELTA, intended | Before, each showed the installation's GitHub avatar and `acme-labs`. Now: GitLab icon with `Acme / web`, `2 connections` (title `Acme / web, Acme / mobile`) or `GitLab`; dashed empty square with `—` (title `Unknown destination`) for the unknown scope. The four red regions of the diff are exactly these four cells. |
| List at 360 and 768 | 360, 768 | MATCH | The destination column is `hidden ... xl:flex`, an existing rule not touched here (U noted it too). |
| Detail, Destination group of GitHub repository, GitHub organization and Bitbucket pools, including the nine-repository list collapsed and expanded | 360, 768, 1280 | MATCH | Nothing. The "Show 3 more" / "Show fewer" toggle was exercised at all three widths. |
| Detail, Destination group of GitLab pools | 360, 768, 1280 | DELTA, intended | Scope reads `GitLab` (it read `Gitlab`); the Account row with the GitHub avatar and `Runner group #1` are gone; each connection is a two-line item (name, then `baseUrl/scopePath`) in server order; with none, "No connections listed". Documented in F's handoff and in wireframe frame D. |
| Detail, Destination group of the unknown-scope pool | 360, 768, 1280 | DELTA, intended | Only `Scope: Unknown`; the raw scope string, the account and the runner group are no longer shown. |
| Everything else on the detail pages | 360, 768, 1280 | MATCH | Measured element by element (`screenshots/repro/geom.mjs`): every element above the Destination group has the same position and size in both builds, for the GitLab one-connection, GitLab no-connection and unknown-scope pools at 1280 and 360; the GitHub organization pool differs in none. Only the group's own containers (which get shorter with it) and the content below it change. Where the group got shorter (GitLab one or no connection, unknown scope) the page no longer overflows its scroll area by 6 px, and Chromium draws the page's text with slightly different edge smoothing; that, and the different scroll position at the bottom of a shorter page, is what lifts the 360 and 768 figures above. |

The 360 figures of 2.0 to 2.4% fall in the "2 to 5%" band. The driving sections are the Destination group itself (intended) and the whole scrolled page sitting 18 to 68 px lower or higher at the bottom of a taller or shorter page. The change is documented as intentional by the brief ("A GitLab pool shows its GitLab connections ... no Account row with a GitHub avatar, no Runner group row"), by wireframe frame D and by F's handoff, so it is not a REWORK. No state exceeds 5%.

### Checks beyond pixels (first-hand)
- **DOM equality for GitHub and Bitbucket.** For the four GitHub and Bitbucket detail states at all three widths, the Destination group's rows, text, avatar `src` and toggle label are identical between the two builds (`facts.json`, `facts-dest.json`). No horizontal overflow on any live page. Console: zero errors or warnings across all of the roughly 110 page loads of the capture runs.
- **The defect is reproduced on the before build and fixed on this head.** Before: the four non-GitHub pools all list `acme-labs` with the GitHub avatar; their details read `Scope: gitlab` (or `azure-devops`), `Account: acme-labs`, `Runner group: #1`. After: list cells `Acme / web`, `2 connections`, `GitLab`, `—`; details `Scope: GitLab` plus connections, `No connections listed`, or `Scope: Unknown`.
- **API.** `GET /api/v1/runner-pools?page=1` against the seeded head returns `gitlabConnections` as `{id, name, baseUrl, scopeKind, scopePath}`: the two-connection pool lists `Acme / web` then `Acme / mobile` (membership order), and the no-connection GitLab pool and the GitHub pools return `[]`. The detail endpoint returns byte-for-byte the same array for the same pools. The detail returns these connections to any reader of the pool, so the list exposes nothing the detail does not.
- **Wireframe conformance.** Frames A to D rendered and compared by eye with the live states: copy, order, two-line connection items, muted "No connections listed", Scope "Unknown" alone. No divergence.

## Spec compliance

| Spec requirement (brief) | Result |
|---|---|
| Both views work out the destination from the registry; the routes compare no platform id, kind id, `pool.scope`, `githubAvatar` or platform-named field | YES. `runner-pools.index.tsx` and `runner-pools.$poolId.index.tsx` hold no such token; both import `platformForPool` from the registry; `Avatar`, `githubAvatar`, `RepositoryList`, `useState`, `ChevronDown`, `cn` removed from them. `tests/platform-neutral-source.test.ts` covers both. |
| Registry finds the platform that owns a pool's scope (GitHub: `repository`, `organization`; GitLab: `gitlab`; Bitbucket none; unknown: safe fallback, never throws, no GitHub avatar) | YES. One optional `pool` key (`scopes`, `destination`, `Destination`) on `PlatformDescriptor`; `platformForPool` resolves it with a type guard; the unknown fallback is registry-owned, is not in `platforms`, and reads no `name`, so the raw scope is never echoed (A warns 1 and 2 adopted). |
| List destination hook (text, title, optional leading avatar or icon) | YES. `PoolDestinationView { text, title, leading? }`. |
| Detail destination hook as a component; GitHub keeps today's rows exactly; `RepositoryList` moves with it | YES. `ComponentType<{ pool: RunnerPoolDetail }>`; GitHub's rows are byte-identical (pixel and DOM check above), `RepositoryList` and its toggle moved verbatim into `github.tsx`. |
| GitHub and Bitbucket pools unchanged in both views | YES, verified live (0 px at 360, 768, 1280). |
| GitLab list: connection names (`2 connections` with all names in the title) and the GitLab icon, no avatar | YES. |
| GitLab detail: Scope "GitLab", the connections (name; `baseUrl/scopePath`), no Account row with a GitHub avatar, no Runner group row | YES. |
| GitLab with empty or absent connection list: neutral fallback, never the GitHub account | YES (`GitLab` in the list, "No connections listed" in the detail; empty array and absent field both tested). |
| GitLab code in `gitlab.tsx`, GitHub code in `github.tsx`; the "GitLab code has one home (#141)" test passes | YES. Routes do not name GitLab; the registry names it only in its import, array entry and comments. |
| API: `gitlabConnections` on each list item, same shape as the detail, `[]` for none | YES (live and Rust test). |
| One query over the page's pool ids, ordered `membership.created_at, connection.id`, helper in `resources/gitlab.rs` | YES. `pools_gitlab_connections` is one `IN (SELECT value FROM json_each(?))` query ordered `membership.pool_id, membership.created_at, connection.id`, early return on an empty page; the detail and the batch share `POOL_CONNECTIONS` and `POOL_CONNECTION_ORDER`. `runner_pools` calls it once, outside the item loop. |
| `RunnerPool.gitlabConnections?: GitlabPoolConnection[]` optional | YES (type-only import, no cycle, file already on the GitLab allowlist). |
| No migration, no change to any other endpoint | YES. `git diff --stat e022a5d HEAD -- migrations docs/openapi` is empty; the list 200 schema is a bare `object`. |
| Do not change the pool forms, the Targets picker, or `readOnlySummary`'s behaviour | YES. Neither form route nor `targets.ts` is in the diff; both `readOnlySummary` lines are unchanged. |
| No deploy; `CARGO_BUILD_JOBS=2`, `RUST_TEST_THREADS=2` | YES. Nothing deployed; the Rust runs used the caps. |
| Done-when: unit tests for the eight cases (registry, list destination, detail destination) | YES, see Test quality. |
| Done-when: Rust API test (names and order as the detail, `[]` for a GitHub pool, single batched query) | YES. The "single query" is shown structurally (one call over the page's ids), as A agreed; no query counter. |
| Done-when: headless UI walkthrough | YES. U walked it (all eight cases, hard load and SPA, 1280 and 360, clean console) and I reproduced it independently (above). |
| Done-when: lint, typecheck, `npm test`, `cargo fmt --check`, clippy, `cargo test -p gridops-api` | YES per T-green, with the two pre-existing, unrelated items recorded there (clippy findings outside `resources.rs`/`resources/gitlab.rs`/`gitlab_tests.rs`; one host-OS-dependent `fleet_http` test). |

## Quality audit

| Area | Result | Notes |
|---|---|---|
| API consistency | PASS | One new field on the list item, same shape as the detail's, empty array when none; no other endpoint touched; OpenAPI unchanged (bare object). |
| Error handling | PASS | Unknown scope never throws and shows no raw scope; absent or empty `gitlabConnections` falls back neutrally; partial connection data shows only the present parts. SQL: ids bound as one JSON parameter, dynamic text built only from two constants. |
| UI/UX match to spec | PASS | Matches wireframe frames A to D and the brief's contract table; GitHub and Bitbucket pixel-identical. |
| Accessibility | PASS | The GitLab icon and the unknown placeholder are `aria-hidden`; the cell's visible text and `title` carry the meaning; connection items are non-interactive text; the "Show N more" button keeps `aria-expanded`. No new control, no new colour pair. |
| Architecture gate | PASS | A's plan review and anti-duplication gate are PASS; no finding of mine conflicts with either. |
| Code organization | PASS | Platform code lives in `github.tsx` and `gitlab.tsx`; the registry holds only the resolver and the neutral fallback; routes are thinner. No TODO, no commented-out code, no new dependency. |
| Security | PASS | No new input; access is the list's existing installation filter, and the connection fields (`name`, `baseUrl`, `scopeKind`, `scopePath`) are the ones the detail already returns to any reader. |
| Performance | PASS | One extra query per list page regardless of page size; none on an empty page. |
| Visual regression | PASS | See the table above. No Playwright VR suite is configured for this change; pixel diffs were produced as described. |
| Naming consistency | PASS | `platformForPool`, `PoolBindings`, `PoolDestinationView`, `pools_gitlab_connections` follow `platformForDeliveryId`, `ConnectionRowView`, `pool_gitlab_connections`. |
| Test quality (`testing/test-quality.md` §7) | PASS, with two advisory merge notes | See below. |

### Test quality (`testing/test-quality.md` §7)
- **Per test.** All 35 new tests name one behavior and assert what the operator sees or what the API returns, not internals. The 30 jsdom tests render the real routes with the production registry, so they pin output whatever the hooks are named (only `platformForPool`, the name the brief and A give, is pinned). Mentally breaking each targeted behavior turns the matching test red: keep the GitHub avatar and rows (the 8 invariant guards fail), return the GitHub account for GitLab (the `images` and "never the installation's account" assertions fail), echo the raw scope (the `azure-devops` assertions fail), drop `capitalize` from GitHub's Scope (the `shown()` helper reads the rendered casing, so `Scope: Repository` fails), reorder or ungroup in SQL (the Rust test's reversed `created_at` and the second pool's single connection fail). The source guard fails if a route regains a platform token or loses its registry import.
- **Tier.** Pure resolution and rendering at the jsdom tier, SQL ordering and grouping at the Rust integration tier; no browser-tier duplication. The Rust test also compares the list with the detail, which is the contract the brief states.
- **Suite.** 30 jsdom + 4 source-guard + 1 Rust for a feature story that changes a contract, two views and a query: proportionate. No snapshot, no mock-shaped call-count assertion, no flaky-by-design timing.
- **Advisory (non-blocking), because the brief's Done-when enumerates these cases itself:** in `tests/pool-destination.test.ts` the registry `it.each` (lines 218 to 230) resolves eight inputs that flow through three branches (the Bitbucket list item and detail, "several repositories" and "GitLab with two connections or none" repeat a scope already covered), and `exports platformForPool beside ...` (lines 214 to 216) fails only if the eight resolution tests also fail. They are one-line, no-I/O table rows and the Bitbucket rows guard a real invariant (Bitbucket must keep owning no scope), so I am not asking for a rework. If the suite is trimmed later, the export test is the one to delete and the repeated scopes the ones to merge.

## Scope check
Delivered exactly the brief's scope: nine production files (two routes, three platform-connection files plus the contract and registry, one type file, two Rust files) and three test files. Nothing from the brief is missing. Nothing extra: the partial-data and unknown-scope behaviors come from the brief's own fallback bullets and wireframe notes. The runner row (`runner-row.tsx:46, :109, :127`) and the dashboard pool row (`routes/_app/index.tsx:279`) are left untouched, as A advised and the brief implies.

## Verdict

PASS. All acceptance criteria are met, the implementation is spec-compliant, and the quality is acceptable. Ready for O to commit.

## Advisory notes
1. **Unrecorded sibling (third time raised: #141 A round 3, #141 S, #159 A).** The runner row shows a GitLab pool's runners under the installation's GitHub login (`runner-row.tsx:46, :109, :127`), and the dashboard pool row reads "Gitlab" (`routes/_app/index.tsx:279`). Neither the brief nor the issue records them as out of scope, and `gh issue list --search` finds no open issue for them. Recommended: O adds an "Out of scope" line to the brief and files the follow-up in epic #136; the new `platformForPool` and the existing `getPlatform` are the right seams, and neither should be fixed with a platform branch.
2. **Wireframe approval and waiver.** `handoff-D.md`'s approval line is unfilled and no pixel-diff waiver against the wireframe is recorded. The live UI matches the wireframe's copy and layout exactly, so this is a bookkeeping item for O.
3. **Pre-existing, not #159's:** the pool list's destination column is hidden below 1280 px (existing `xl:flex` rule); `runner_pools` reports `totalRunners: 1` for a pool with no runners (LEFT JOIN count, T's note); the local `release_preflight` failure and the clippy findings outside the changed Rust files.
4. **Test trim (optional, see Test quality).** The registry export-smoke test and the repeated-scope rows of its `it.each` are the only redundancy in the suite.
