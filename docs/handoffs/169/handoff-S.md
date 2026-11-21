# Handoff-S: Phase 9 - Runner rows and dashboard take the pool platform from the registry (#169)

**Date:** 2026-11-21
**Branch:** claude/gitlab-169-runner-rows (head 75b0cab; `git diff 83f0a48 HEAD -- src crates tests` is empty, so the code is what T-green and U verified)
**Issue:** #169
**Brief:** docs/handoffs/169-brief.md
**Handoff-T reviewed:** docs/handoffs/169/handoff-T-green.md (and handoff-T-red.md)
**Handoff-A reviewed:** docs/handoffs/169/handoff-A.md (plan review) and handoff-A-dup.md (anti-duplication gate)
**Handoff-F reviewed:** docs/handoffs/169/handoff-F.md
**Also read:** handoff-U.md, handoff-D.md, wireframe.html (as copy source), evidence.md, decisions.md
**Operator-facing report:** [`report.html`](report.html) (wipe sliders, diff PNGs, composites, per-section table; assets in `screenshots/s/`)

## A precondition
Confirmed. handoff-A.md (Phase 3) is PASS with 0 block and 5 warn, all five adopted by F. handoff-A-dup.md is PASS with no duplication finding.

## T precondition
Confirmed. handoff-T-green.md reports no blocking issues. handoff-U.md is PASS.

## Browser-tool precondition (deviation, stated plainly)
The `mcp__Claude_in_Chrome__*` tools are not in my tool list in this run (I have Read, Write, Bash and the structured-output tool). The role document says to return CANNOT-AUDIT in that case, and this run's verdict schema has no CANNOT-AUDIT value. I did not fall back to cascade reasoning or prose descriptions. I rendered the real pages in a real browser instead: headless Chromium 159 driven by Playwright 1.65.1 (the path the role document itself names as the only reliable one for 360/768/1280), against the real Rust API binary and the real built frontend. That also follows the operator's standing rule to work headlessly. If O reads the precondition literally (the Chrome MCP specifically), treat this audit as run under that substitution and say so; I judge the purpose of the precondition (confirmed rendering, no static inference) fully met.

## Visual-diff-tool precondition
Confirmed. Playwright 1.65.1 installed into the scratch directory only (nothing added to the project), cached Chromium launches headless, ImageMagick `compare` and `magick` are at `/opt/homebrew/bin`.

## Preview / spec sanity check
No source-of-truth defect that justifies ADVISORY-HOLD. The wireframe is low-fi shorthand and the brief's copy is consistent with the registry's existing conventions (#159's unknown-scope fallback, `platformForDeliveryId`'s legacy default). Two copy smells, neither a convention violation and both decided up front by D (open question 1) and the brief, are recorded under Advisory notes: the details sheet keeps the label "Repository" over a GitLab connection name, and two GitLab addresses sharing a host truncate identically in the 144px column. No change in this issue touches contrast, touch targets or heading order.

## Method
Both builds come from `git archive` of a commit (not the working tree), with identical build info so the version bar does not differ:
- **Reference** = frontend at `acd2afd` (the brief's base, before #169).
- **Live** = frontend at HEAD `75b0cab`.
- Both served by the same HEAD API binary (rebuilt incrementally with `CARGO_BUILD_JOBS=2`), each on its own throwaway SQLite DB seeded with the same data (U's seed: GitHub repository pool, GitHub organization pool, Bitbucket runner on a GitHub-scoped pool, a GitLab pool with 4 runner states, an unknown-scope pool, plus an empty GitLab pool), hand-minted session cookie, browser clock frozen at 2026-11-21T09:05Z, reduced motion.
- Old frontend on a new API ignores the two added fields, so the reference is exactly "today".
- A third run loaded the HEAD frontend with `poolScope` and `gitlabConnectionName` stripped from `/api/v1/runners` responses in the browser. This produces the "older server" case live, which U could not (U recorded that gap).
- Servers stopped and throwaway DBs deleted afterwards (ports 19311/19312 confirmed down). The project worktree has only this handoff, `report.html` and `screenshots/s/` added.

## Visual diff results

| Page | Viewport | Pixels different | Whole-page delta | Where | Status |
|---|---|---|---|---|---|
| Runners list | 360x800 | 606 | 0.2104% | line under 5 runner names | DELTA (intended) |
| Runners list | 768x1024 | 0 | 0.0000% | destination not shown at md..xl | MATCH |
| Runners list | 1280x800 | 449 | 0.0439% | right-hand column, 5 rows | DELTA (intended) |
| GitLab pool detail | 360x800 | 549 | 0.1905% | line under 4 runner names | DELTA (intended) |
| GitLab pool detail | 768x1024 | 0 | 0.0000% | not shown at this width | MATCH |
| GitLab pool detail | 1280x800 | 401 | 0.0392% | right-hand column, 4 rows | DELTA (intended) |
| Overview (tall viewport) | 360x3200 | 0 | 0.0000% | scope text hidden below sm | MATCH |
| Overview (tall viewport) | 768x2600 | 148 | 0.0074% | 3 pool rows, scope text | DELTA (intended) |
| Overview (tall viewport) | 1280x2000 | 150 | 0.0059% | 3 pool rows, scope text | DELTA (intended) |
| Details sheet, GitLab runner | 360x800 | 219 | 0.0761% | "Repository" value | DELTA (intended) |
| Details sheet, GitLab runner | 768x1024 | 219 | 0.0279% | "Repository" value | DELTA (intended) |
| Details sheet, GitLab runner | 1280x800 | 219 | 0.0214% | "Repository" value | DELTA (intended) |

Diff PNGs, composites and wipe sliders for all twelve are in `report.html`; files are `screenshots/s/t3-<page>-<width>-{reference,live,diff,composite}-20261121.png`, with per-section crops `t3-<page>-delta-<width>-crop-20261121.png`.

Every delta is under 2% for the page and for its changed region (the largest region, the 360px runner lines, is about 1.4% of its bounding box). Every changed pixel is in a cell the brief says must change; nothing else on any page differs (sidebar, header, badges, other columns, GitHub and Bitbucket rows, the pool page's Destination sidebar group are all identical). Note that the "Overview" capture needed a tall viewport: the app scrolls inside its shell, so a standard-height full-page capture does not include the Runner pools section and shows a false 0.

### Older server (HEAD frontend, the two fields stripped) against the reference
Runners list, GitLab pool page, GitHub organization pool page and unknown-scope pool page: **0 pixels different at all three widths** (12 of 12). Details sheet for two runners: 0 at 360, and at most 0.06 of one pixel at 768/1280 (a 1/255 rounding step in the sheet's fade; not a change). In the DOM, all eight runners then show the installation login (or `repository`) in all three places, exactly as before #169.

### Per-section delta description
| Section | Viewport(s) | What differs | Documented as intentional | Status |
|---|---|---|---|---|
| Runner rows, destination (column or line under the name) | 360, 1280 | Four GitLab rows now show "Acme / web", `https://gitlab.example.com/acme/web`, `https://gitlab.example.com`, "GitLab" instead of "acme-labs"; the unknown-scope row shows "—" instead of "acme-labs". The GitHub org, GitHub repository and Bitbucket rows are identical. | Brief, wireframe frames A/B/E, F "no deviations" | DELTA (intended), accepted |
| Runner rows | 768 | None (destination not rendered between md and xl) | n/a | MATCH |
| Overview pool rows, scope text | 768, 1280 | "Gitlab" reads "GitLab" (gl-pool, empty-pool); "Quantum" reads "Unknown" (odd-pool). "Repository" / "Organization" rows identical. | Brief, wireframe frame C | DELTA (intended), accepted |
| Overview pool rows | 360 | None (text hidden below sm) | n/a | MATCH |
| Details sheet "Repository" row | 360, 768, 1280 | Value "Acme / web" instead of "acme-labs"; label unchanged | D open question 1 | DELTA (intended), accepted |

## DOM ground truth (HEAD frontend, current API, same capture run)
Read from the live DOM at 1280 and 360 (right-hand column text and `title`, line under the name, details sheet "Repository" value) and agreeing in all three places for every runner:

| Runner (case) | Text | Title (column) |
|---|---|---|
| 7f3a GitHub repository pool | `acme-labs/web` | `acme-labs/web` |
| 7f3b GitHub organization pool, no repository | `acme-labs` | `acme-labs` |
| 7f3c Bitbucket runner, GitHub-scoped pool | `acme-labs/web` (identical to 7f3a; badge says Bitbucket) | same |
| 7f3d GitLab, connection named | `Acme / web` | same |
| 7f3e GitLab, no name, full address | `https://gitlab.example.com/acme/web` | same |
| 7f3f GitLab, no name, base URL only | `https://gitlab.example.com` (no stray `/`) | same |
| 7f3g GitLab, no connection | `GitLab` (never the login) | `GitLab` |
| 7f3h unknown scope `quantum` | `—` (raw scope never shown) | `Unknown destination` |

Overview scope text (browser `innerText`, which applies the row's `capitalize`): `Repository · Ephemeral`, `Organization · Ephemeral`, `Repository · Ephemeral` (Bitbucket pool), `GitLab · Ephemeral`, `Unknown · Ephemeral`, `GitLab · Ephemeral` (empty pool). Browser console: 0 errors and 0 warnings at all three widths in all three runs.

API, against the real binary on the seeded DB: `/api/v1/runners` items carry `poolScope` for all eight and `gitlabConnectionName` as `"Acme / web"` for the named GitLab connection, `""` for the two connections seeded with an empty name (the raw `gc.name`; the row treats empty as no name and falls back to the address), and null for the GitHub runners, the Bitbucket runner and the GitLab runner with no connection; `/api/v1/runner-pools` `totalRunners`: web-ci 1, org-pool 1, bb-pool 1, gl-pool 4, odd-pool 1, empty-pool **0**; after inserting a soft-deleted runner into web-ci, web-ci stays **1** and the deleted runner is absent from the runners list.

## Spec compliance
| Requirement (brief) | Checked | Compliant |
|---|---|---|
| Runners list adds `poolScope` (`p.scope`) and `gitlabConnectionName` (`gc.name` from the existing LEFT JOIN; null for non-GitLab or a gone connection) | diff of `runners` in resources.rs; live API output; Rust test | YES |
| Both fields on `Runner` in `operations.functions.ts`, optional | diff (`poolScope?: string`, `gitlabConnectionName?: string \| null`) | YES |
| `total_runners` counts 0 for a pool with no runners | diff (`r.id IS NOT NULL AND r.deleted_at IS NULL`); live empty pool = 0; Rust test | YES |
| Deleted runner not counted; right count with runners | live: 1 live + 1 deleted = 1; gl-pool 4; Rust test | YES |
| Sweep the other `LEFT JOIN runners` counts; confirm, don't assume | I re-read the count expressions at resources.rs 342-350 (`DISTINCT … r.id`), 533-534, 1331-1334, 5431-5432 and webhooks.rs 836 (all test non-null columns, so a NULL row never counts); `targets.rs:1038` is a join, not a count. Matches A and T. Only `:1330` was wrong. | YES |
| No migration; no OpenAPI change | `git diff acd2afd HEAD` touches no `migrations/`, `docs/openapi/`, `deploy/`, host-agent or core | YES |
| `PoolBindings` extended (one key) with a runner destination hook and a scope label hook | `types.ts`: `runnerDestination(runner)` returning named `RunnerDestinationView = Pick<PoolDestinationView, "text" \| "title">`, `scopeLabel(pool)`; both required | YES |
| GitHub hook returns exactly today's `repository ?? accountLogin` for text and title; scope label = raw scope | `github.tsx`; DOM rows 7f3a/7f3b; pixel-identical rows | YES |
| GitLab hook: connection name, else `gitlabBaseUrl/gitlabScopePath` with no stray `/`, else "GitLab", never the login | `gitlab.tsx` reuses `connectionLabel`/`connectionAddress` (widened to nullable parts); DOM rows 7f3d-7f3g; Vitest also pins scope-path-only and base-URL-only | YES |
| GitLab scope label "GitLab" | dashboard DOM and pixels | YES |
| Unknown-scope fallback supplies both hooks: "—" / "Unknown destination" / "Unknown"; never echoes the raw scope | `registry.tsx` `unknownPoolPlatform`; DOM row 7f3h and `odd-pool`; `quantum` appears nowhere | YES |
| `runner-row.tsx` resolves with `platformForPool({ scope: runner.poolScope })` and uses the hook in all three places (sheet row, narrow line, right-hand column + title); older server looks as today; no platform comparison in the component | diff: one local helper called once per component; absent-scope default lives in `registry.tsx` (`platformForPool` accepts `scope?: string \| null`, an empty string is unknown); older-server run is pixel-identical (0 px) | YES |
| `index.tsx` uses `platformForPool(pool).pool.scopeLabel(pool)` in place of `pool.scope`; `capitalize` span kept | diff line 280; DOM `innerText` | YES |
| Keep the existing `getPlatform(runner.platform)` uses | `runner-row.tsx` lines 84, 93, 146 untouched | YES |
| Bitbucket runner on a GitHub-scoped pool looks exactly as today | DOM 7f3c equals 7f3a; its row is outside every changed bbox | YES |
| GitLab-specific code only in `gitlab.tsx`, GitHub-specific only in `github.tsx`; "GitLab code has one home (#141)" keeps passing | grep: `gitlabConnectionName` read only in `gitlab.tsx` (plus the type); Vitest guard re-run green | YES |
| Do not change pool list/detail behaviour from #159, the pool forms or the Targets picker | diff touches only the listed files; the pool page's Destination sidebar group is pixel-identical; `tests/pool-destination.test.ts` re-run green; U checked `/runner-pools` | YES |
| No deploy | no deploy, Dockerfile or compose change | YES |
| Source guard covers `runner-row.tsx` and `index.tsx` (no platform/kind id, no `.scope` for display, no `accountLogin`/`repository` as destination outside the descriptors; both import the registry; exact greppable patterns) | `platform-neutral-source.test.ts`: both files in `SHARED`; new patterns `["'](repository\|organization)["']` and `\b(runner\|pool)\.(repository\|accountLogin)\b`; a line-rule that `poolScope` appears only on a line with `platformForPool(`; import-and-call assertion. Re-run: green. | YES |
| Vitest matrix for GitHub repo / org, Bitbucket-on-GitHub, GitLab name / address only / neither, unknown, older server; sheet row, narrow line, column text and title | `tests/runner-row-platform.test.ts` (11 cases + job-name test) | YES |
| Vitest dashboard scope text for repository, organization, gitlab, unknown | `tests/dashboard-pool-scope.test.ts` | YES |
| Rust API tests (runners list fields, null for GitHub; `totalRunners` 0 / N / with a deleted runner) | `gitlab_tests.rs`, two tests; T ran them RED then GREEN; behaviour re-confirmed live above | YES |
| Headless UI walkthrough: `/runners`, GitLab pool detail, `/`, GitHub and GitLab pools | U's walkthrough plus my 3-width pixel pass | YES |
| lint, typecheck, npm test, fmt, clippy, cargo test pass | T-green: lint, typecheck, vitest 602/602, fmt, cargo unit 125/125 and control_plane_contracts 36/36 pass. Clippy and one `fleet_http` test do not pass on this Mac for reasons outside this diff (see Quality audit). I re-ran the four #169 Vitest files (70/70) on HEAD; I did not re-run the Rust suite, because no code has changed since T-green. | YES, with the caveat recorded below |

## Quality audit
| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | New keys follow `poolId`/`poolName` and `gitlabBaseUrl` naming; optional on the client type for an older server; nulls explicit. |
| Error handling | PASS | Absent scope, empty-string scope, unknown scope, gone connection and partial address parts are each handled and tested; nothing throws on a missing field. |
| UI/UX match to spec | PASS | Rendered output matches the wireframe copy exactly in all three places, at three widths. |
| Accessibility | PASS | No new interactive element, label or role; the right-hand column keeps its `title`; the details sheet opened on click and closed with Escape in the capture run. |
| Architecture gate | PASS | A: PASS (Phase 3) and A-dup: PASS. I found no quality finding that conflicts with either. |
| Code organization | PASS | One local helper in the row, hooks beside their descriptors, `ConnectionParts` reused instead of a second address join. The `hasPoolBindings` guard is a narrowing, not a cast. Comments are accurate and no dead code or TODOs were added. |
| Security | PASS | No new input or auth path; the runners list already required the same access; the fields added are a pool scope and a connection name already visible on the pool list. |
| Performance | PASS | One registry lookup per row render; one extra column in an existing query; the `total_runners` change adds a null test inside an existing aggregate. |
| Visual regression | PASS | Pixel pass above; the project has no Playwright VR suite or axe configuration to run. |
| Naming consistency | PASS | `runnerDestination`, `scopeLabel`, `RunnerDestinationView`, `poolScope`, `gitlabConnectionName` follow the existing `destination` / `Destination` / `PoolDestinationView` names. |
| Test quality (`testing/test-quality.md` section 7) | PASS | See below. |

**Pre-existing, outside this diff (not a #169 defect, but the brief's "pass" list is not literally green on this Mac):** `cargo clippy -p gridops-api --all-targets -- -D warnings` fails on five `clippy::unused-async-trait-impl` sites in files this diff does not touch (`fleet/boundary.rs`, `fleet/enrollment.rs`, `resources/route_security.rs`, host-agent `helper.rs` and `native.rs`; new lint in toolchain 1.99.1), and `fleet_http::release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails because the fixture's "Linux" host agent probes the build host with `cfg!(target_os = "macos")` (`gridops-host-agent/src/probe.rs:17`, untouched). I confirmed `git diff acd2afd HEAD` has no change under `gridops-host-agent`, `gridops-core` or the fleet routes. The preflight test has not been seen passing on a Linux runner in this run; CI is where that confirms.

### Test-quality audit (section 7 rubric)
- **Names one behaviour, fails for the right reason.** Rust: the runners-list test fails on the missing `poolScope` field and the pool-list test fails on `1 != 0` for the empty pool (T-red's recorded failures); the 2/1 cases guard the fix. Runner row: each `it.each` case names a platform/scope situation and asserts exact text and title; T's mutation (GitLab fallback to `accountLogin`) turned exactly the "no name and no address shows GitLab" case red. Dashboard: four rows, each pinning exact text plus the `capitalize` class.
- **Tier-correct and non-duplicative.** Rust goes through the real router because the bug is in SQL; the row and the dashboard are jsdom renders (the cheapest tier that sees all three places); the source guard pins the registry contract. The GitHub, Bitbucket and older-server cases overlap in output (`acme/web`) but exercise different resolution paths (explicit scope, Bitbucket badge on a GitHub pool, absent scope), so none re-proves another branch.
- **Behavioural.** Assertions read rendered text and `title`, not hook names; F's own hook names are not pinned.
- **Proportionate.** 2 Rust tests, 11 + 1 row cases, 4 dashboard cases and a handful of guard patterns for 3 changed behaviours (row destination, dashboard label, count) is proportionate.
- **Smells.** None: no assertion-free, tautological, unreachable, mock-shaped or snapshot test. I found nothing to delete or merge.

## Scope check
F delivered the brief's scope: eight production files, three Vitest files (one new) and the Rust `gitlab_tests.rs`, and nothing else. No over-delivery (no label rename, no extra count change, no change to the #159 surfaces) and nothing under-delivered. The older-server case F and U could not show live is now shown live (0 px).

## Verdict

PASS - all acceptance criteria met, spec-compliant, quality acceptable. Ready for O to commit.

## Advisory notes
Non-blocking; none is part of #169's scope and each needs a copy decision from O, so none was applied.
1. **Details sheet label.** A GitLab runner reads "Repository: Acme / web". The label is GitHub's word (D open question 1, kept because the hook returns only `{ text, title }` and GitHub output must not change). Decision for O: rename the row to "Destination" for all platforms or give the hook a label. Suggested branch `claude/runner-details-destination-label`.
2. **Narrow column.** In the 144px (`w-36`) column, two GitLab runners on the same host both read `https://gitlab.example.c...`; the full address is in the hover title. The brief fixed the text as `baseUrl/scopePath`. Decision for O: leave, or show the scope path first. Suggested branch `claude/runner-row-destination-width`.
3. **Pre-existing CI hygiene.** The five `unused-async-trait-impl` clippy sites and the macOS-only `release_preflight` test (above) deserve their own issue; T-green made the same note.
