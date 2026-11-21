# Handoff-S: Phase 9 - GitLab Docker pool default image and no runner group (#172)

**Date:** 2026-11-21, 10:40 AM PST
**Branch:** claude/gitlab-172-pool-image-bb7d36 (head `7642e98`; `git diff 4fb3bbe HEAD -- src crates tests` is empty, so the code is what F wrote and what T-green, A-dup and U verified)
**Issue:** #172 (epic #136). Brief: `docs/handoffs/172-brief.md` (amended in `7afb7c5`)
**Handoff-T reviewed:** `docs/handoffs/172/handoff-T-green.md` (and `handoff-T-red.md`)
**Handoff-A reviewed:** `docs/handoffs/172/handoff-A.md` (plan review, round 2, PASS) and `handoff-A-dup.md` (anti-duplication gate, PASS)
**Handoff-F reviewed:** `docs/handoffs/172/handoff-F.md`
**Also read:** `handoff-U.md` (PASS), `handoff-D.md`, `wireframe.html`, `evidence.md`, `decisions.md`, and the full diff `7afb7c5..HEAD` over `crates`, `src`, `tests`, `docs/gitlab-runners.md` and `CHANGELOG.md`
**Operator-facing report:** [`phase-9-pool-image-report.html`](phase-9-pool-image-report.html) (wipe sliders, diff PNGs, composites and per-section table; images in [`screenshots/s/`](screenshots/s/))

## Result in one line

**PASS.** The implementation matches the brief, A's plan and the approved wireframe. Every GitHub-pool page is pixel-identical (0 px at 360, 768 and 1280) to the build before the change. The only differences are on the GitLab Docker image row the brief asked to change, plus the 20 px the longer help line pushes the rest of the form down. The API behaviour was confirmed live against the real binary, not only through the test harness.

## A precondition
Confirmed. `handoff-A.md` (round 2) is PASS with three warns, all honoured by F (`targets.ts` helpers take the descriptors argument and import no registry; `input.validate(pool.scope == "gitlab")` runs after `pool_access` and before `assert_pool_admin`; `dockerImageOptional` is destructured beside `poolId` and never sent). `handoff-A-dup.md` is PASS with no duplication.

## T precondition
Confirmed. `handoff-T-green.md` reports "Blocking issues: None" (vitest 623/623, `gitlab_tests` 39/39, core 307 + `gitlab_pools` 5, api 130 + 36, fmt clean, tsc and eslint clean). `handoff-U.md` is PASS. I did not re-run Tier 1 or Tier 2 in full; I re-ran the four #172 vitest files at head (4 files, 59 tests, all pass) and rebuilt the API binary from this worktree (compiles).

## Browser-tool precondition (deviation, stated plainly)
The `mcp__Claude_in_Chrome__*` tools are not in my tool list in this run (I have Read, Write, Bash and the structured-output tool), and this run's verdict schema has no CANNOT-AUDIT value. As in #141, #145, #159 and #169 I did not fall back to cascade reasoning or prose descriptions. I rendered the real pages in a real browser instead: headless Chromium driven by Playwright 1.65.1, which is the path the role document itself names as the only reliable one for 360/768/1280, and it follows the operator's standing rule to work headlessly (no window opened; the screenshots are files). This substitution is mine; if O reads the precondition literally (the Chrome MCP specifically), treat this audit as run under it.

## Visual-diff-tool precondition
Confirmed. Playwright 1.65.1 was installed into the session scratch directory only (nothing added to the project; the cached Chromium 1255 launches headless). ImageMagick `compare` and `magick` are at `/opt/homebrew/bin`.

## Preview / spec sanity check
No source-of-truth defect that justifies ADVISORY-HOLD. The wireframe touches one row on four existing screens, adds no new interactive control (touch targets, focus order and heading levels are unchanged), reuses existing tokens, and its copy contradicts no convention. The placeholder is the existing `placeholder:text-faint` style; it repeats information the help line also states, so it is not the sole carrier. Process note: `handoff-D.md`'s approval line is still "To be filled by O". A, T, F and U treated the copy as approved and the live UI matches it exactly, so the operator can still overrule any copy line without a structural change.

## Method
- **Reference** = the frontend at `7afb7c5` (the brief amendment; no `src` change since #169), **live** = the frontend at HEAD. Both from `git archive`, built with Vite using the same build number and commit so the version bar matches.
- Both served by the **same** API binary, built from this worktree at HEAD (`CARGO_BUILD_JOBS=2`; I rebuilt it because the binary in the shared cargo target was newer than U's run and recompiled to a different size, so it had apparently been built from another worktree's sources), each on its own throwaway SQLite database seeded identically: one GitHub installation with one repository, a GitHub repository pool (`pool-gh`), a GitLab pool with an empty Docker image (`pool-gl-empty`), a GitLab pool with an image set (`pool-gl-set`), and one GitLab connection "Acme / web". Hand-minted session cookie, as in earlier rounds (GridOps has no headless login). Old frontend on the new API is exactly "today's UI".
- Clock fixed to 2026-11-21 10:00 UTC, reduced motion, all external requests blocked. Widths 360, 768, 1280. Heights are 2600, 1900 and 1500 so a whole form fits (the app scrolls inside its own shell, so a 800 px full-page capture would cut the form off); percentages are of the captured area.
- Servers stopped, ports confirmed down, throwaway databases deleted. The worktree gained only this handoff, the report and `screenshots/s/`.

## Visual diff results

Pixels different (share of the captured area), reference versus live. Diff PNGs, composites and wipe sliders are in the report; files are `screenshots/s/t3-<page>-<width>-{reference,live,diff,composite}-20261121.png` (diff and composite exist only for pairs that differ; a 0 px pair has no red to show).

| Page | 360 px | 768 px | 1280 px | Status |
|---|---|---|---|---|
| New pool, no target (1a) | 0 (0%) | 0 (0%) | 0 (0%) | MATCH |
| New pool, GitLab target (1b) | 17,603 (1.881%) | 14,835 (1.017%) | 6,821 (0.355%) | DELTA (intended) |
| Settings, GitHub pool (2c) | 0 (0%) | 0 (0%) | 0 (0%) | MATCH |
| Settings, GitLab pool, empty image (2a) | 18,609 (1.988%) | 16,171 (1.108%) | 8,182 (0.426%) | DELTA (intended) |
| Settings, GitLab pool, image set | 19,442 (2.077%) | 17,010 (1.166%) | 9,073 (0.473%) | DELTA (intended) |
| Detail, GitHub pool (3d) | 0 (0%) | 0 (0%) | 0 (0%) | MATCH |
| Detail, GitLab pool, empty image (3c) | 434 (0.046%) | 643 (0.044%) | 315 (0.016%) | DELTA (intended) |
| Detail, GitLab pool, image set (3d) | 0 (0%) | 0 (0%) | 0 (0%) | MATCH |

The one number over the 2% line is the GitLab pool settings page with an image set at 360 px (2.08%); the empty-image one is 1.99%. Both are the same cause and are documented: the Docker image row changes as the wireframe specifies (frames 1b and 2a), and the help line wraps onto a second line, so the rest of the form moves down 20 px. I checked this rather than assuming it: sliding the part of the page below the row back up by 20 px makes it pixel-identical again at 360 (under 1 px) and at 768 on the settings pages; the few hundred to 1,440 px left at 768 (new pool) and at 1280 sit in the fixed sidebar and in the strip where the capture cuts the page off, not in the form. No section other than the Docker image row differs in content.

### Per-section delta description

| Section | Viewport(s) | What differs | Documented as intentional | Status |
|---|---|---|---|---|
| New pool, Docker image row, no target or GitHub | 360, 768, 1280 | Nothing. Required, prefilled `ghcr.io/actions/actions-runner:latest`, help "OCI image for each Linux runner." | Brief ("restores today's behaviour"), wireframe 1a | MATCH |
| New pool, Docker image row, GitLab project selected | 360, 768, 1280 | Before: the field held the GitHub image and was required, so a GitLab pool got a GitHub image (the defect). Now: empty, grey "optional" beside the label, help "...Leave empty to use gitlab/gitlab-runner:ubuntu-v19.6.0.", grey placeholder of the same image. Everything below moves down 20 px. | Brief, wireframe 1b, F "no deviations" | DELTA, accepted |
| Settings, GitHub pool | 360, 768, 1280 | Nothing, including the Fleet execution section. | Brief ("GitHub/Bitbucket pools pass exactly what they pass today") | MATCH |
| Settings, GitLab pool, empty image | 360, 768, 1280 | Empty required box becomes an empty optional box with the help and placeholder above; rest moves down 20 px. | Brief, wireframe 2a | DELTA, accepted |
| Settings, GitLab pool, image set | 360, 768, 1280 | The stored image still shows verbatim; the row gains "optional" and the longer help (the field is optional for any GitLab pool); rest moves down 20 px. | Brief ("a GitLab pool's Docker image field is optional with the same help") | DELTA, accepted |
| Pool detail, Image property, GitLab, empty image | 360, 768, 1280 | Blank line becomes "Default (gitlab/gitlab-runner:ubuntu-v19.6.0)", clipped with an ellipsis in the sidebar as a long GitHub image is today; the title attribute carries the whole text. | Brief, wireframe 3c ("Truncates as today") | DELTA, accepted |
| Pool detail, Image property, GitHub or GitLab with an image | 360, 768, 1280 | Nothing. | Brief | MATCH |

### DOM ground truth (same capture run, read from the live pages)

| State | Docker image box (value / required / placeholder) | Label and help | Fleet "Add platform" starts from |
|---|---|---|---|
| New pool, no target | `ghcr.io/actions/actions-runner:latest` / required / none | "Docker image"; "OCI image for each Linux runner." | n/a |
| New pool, GitLab target, reference build | `ghcr.io/actions/actions-runner:latest` / required / none | same as above (the defect) | n/a |
| New pool, GitLab target, live | empty / optional / `gitlab/gitlab-runner:ubuntu-v19.6.0` | "Docker image optional"; "OCI image for each Linux runner. Leave empty to use gitlab/gitlab-runner:ubuntu-v19.6.0." | n/a |
| Settings, GitHub pool (both builds) | `ghcr.io/actions/actions-runner:latest` / required | "Docker image"; "OCI image for each Linux runner." | `ghcr.io/actions/actions-runner:latest` (both builds) |
| Settings, GitLab pool, empty, reference build | empty / required | as GitHub | `ghcr.io/actions/actions-runner:latest` (the defect T and A found) |
| Settings, GitLab pool, empty, live | empty / optional / the GitLab image | "optional" + the longer help | **raw** `gitlab/gitlab-runner:ubuntu-v19.6.0` (never "Default (...)") |
| Settings, GitLab pool, image set, live | `registry.example.com/ci/gitlab-runner:1.2` / optional | as above | `registry.example.com/ci/gitlab-runner:1.2` |
| Detail, GitLab pool, empty, live | n/a | Image row text and title both "Default (gitlab/gitlab-runner:ubuntu-v19.6.0)" (reference: blank) | n/a |

No horizontal overflow at 360 on any page (`scrollWidth` equals `clientWidth`). Both builds log the same 18 console errors (500s from the repository and runner-group lists because the seeded GitHub installation has no real encrypted token, and one blocked avatar image); none is new, and none is on the #172 surface.

### Live API checks (real binary, HEAD frontend's server, seeded DB)

| Request | Result |
|---|---|
| POST gitlab + docker, `image`/`dockerImage` empty, no `runnerGroupId` | 201; stored `image=''`, `docker_image=''`, `runner_group_id=1` |
| POST same with `runnerGroupId: 0` | 201; stored the same |
| POST gitlab with `" x"` (padded) image | 400 "Docker image must contain 1-300 non-padding characters." |
| POST organization pool, empty image, group 1 | 400 "Docker image (dockerImage) is required for a pool that uses Linux · Docker." |
| POST organization pool, image given, no `runnerGroupId` | 400 "Runner group ID must be positive." (was serde's 422; the accepted change) |
| PUT complete body with a bad name to a missing pool | 404 "Runner pool does not exist or is not accessible." (accepted precedence change) |
| PUT the same body to an existing GitLab pool | 400 pool-name message (validation runs against the stored scope) |
| PUT GitLab pool, empty image, no `runnerGroupId` | 200 |
| PUT GitHub repository pool, empty image / `runnerGroupId: 0` | 400 Docker-image message / 400 "Runner group ID must be positive." |

## Spec compliance

| Requirement (brief) | Checked | Compliant |
|---|---|---|
| `validate_pool_configuration` takes `gitlab: bool` (the `macos_runtime` precedent); create derives it from `scope == "gitlab"`; `UpdateRunnerPool::validate(gitlab)` takes it; every caller updated | `models.rs:223,313,403`; `grep` of `.validate(` over `crates` finds no unconverted update caller (the one `pool().validate()` is `CreateRunnerPool`); the whole workspace compiles | YES |
| A GitLab pool may send an empty `image`/`dockerImage`, stored empty, resolved by `docker_runner_image(.., true)` to `GITLAB_RUNNER_IMAGE`; a non-empty one still goes through `valid_image` | `models.rs:466-473` (`is_empty() && !gitlab`; `!is_empty() && !valid_image`); live POST + padded 400; Rust test asserts the resolver output; the reconciler (`main.rs:2559-2561`) and `save_pool_fleet` already resolve through the same function | YES |
| `runnerGroupId` optional/0/any for GitLab; `#[serde(default)]` on both structs; reuse the existing normalisation; no new normalisation, no migration | `models.rs:83-84,269-270,489`; handler normalisation untouched (`resources.rs:1742,2675`); `git diff` touches no `migrations/`; live stored value 1 | YES |
| GitHub `organization` keeps "Runner group ID must be positive." exactly; GitHub body with no group is that 400 | live cases above | YES |
| Update validates after `pool_access` with `pool.scope == "gitlab"` (not `is_gitlab_pool`), before `assert_pool_admin`; invalid body to a missing pool is a 404 | `resources.rs:1603-1609`; live 404; Rust test also pins the reader (400 for invalid, 403 for valid) so the only precedence change is the accepted one | YES |
| Tart image rule unchanged | `models.rs` diff leaves the Tart block untouched | YES |
| GitHub and Bitbucket pools validate exactly as before | live GitHub cases; T's guard tests; pixel-identical GitHub pages | YES |
| No platform comparison in `validateProvider`, `runner-pools.new.tsx`, `runner-pools.$poolId.settings.tsx`, `fleet-profile-section.tsx` | grep of those files and `targets.ts` for gitlab/GitLab is clean; `platform-neutral-source.test.ts` covers all four (the two routes were already in `SHARED`; `fleet-profile-section.tsx` added) plus a `validateProvider`-body check | YES |
| Top-level optional `PlatformDescriptor.defaultDockerImage`; GitLab sets it, GitHub and Bitbucket leave it unset; pure helper in `targets.ts` over the descriptors; one helper composes "Default (...)" | `types.ts`, `gitlab.tsx` (one key; connection fields and text untouched), `targets.ts` (`defaultDockerImage`, `dockerImageHelp`, `shownDockerImage`); no registry import in `targets.ts`, tested | YES |
| Vitest reads `provisioning.rs` and asserts the descriptor equals `GITLAB_RUNNER_IMAGE` | `tests/gitlab-pool-default-image.test.ts` first test; re-run green | YES |
| New pool form: optional while a default applies, not prefilled with the GitHub default, help/placeholder name the default, empty sent empty, switching back restores today's behaviour without clobbering a typed image | live DOM above; jsdom tests for 1a, 1b, 1c/1d; U's live typed-survives-switch check | YES |
| Settings form: GitLab pool's field optional with the same help, empty saved as empty; read-only view and detail show "Default (...)" | live DOM; U's real save round trip (`configurationVersion` 1 to 2, image stays empty); my live PUT 200 | YES |
| Fleet editor starts from the descriptor's **raw** default for an empty GitLab image; `fleet-profile-section.tsx` platform-free; GitHub/Bitbucket pass what they pass today | live DOM after clicking "Add platform" (table above); `fleet-profile-section.tsx` not in the diff | YES |
| `schemas.ts` imports no registry or descriptor; neutral `dockerImageOptional` parameter; create derives it from `scope`; update through `parseUpdateRunnerPoolInput(values, { dockerImageOptional })`; the exported schema keeps the GitHub rules | `schemas.ts` diff; `updateRunnerPoolAction` destructures the flag beside `poolId` and never puts it in the PUT body (tested); guard test on the import | YES |
| Runner group on the wire is option A (form still sends 1; GitLab `toCreatePayload` may keep it) | F kept it (the no-op choice); existing pins at `tests/gitlab-descriptor.test.ts:315` and `tests/gitlab-pool-forms.test.ts:124` stay green | YES |
| `docs/gitlab-runners.md`: image may be left empty and then uses the GitLab image; a GitLab pool has no runner group; CHANGELOG under Unreleased | diff reviewed; both statements match the behaviour I confirmed live; the Docker-socket paragraph now says "when the pool's Docker image is empty" | YES |
| GitLab-specific code only in `gitlab.tsx`; "GitLab code has one home (#141)" keeps passing; no connection-form or verify code, Tart script or agent touched | `gitlab.tsx` changed by one pool-level key; re-ran `platform-neutral-source` green | YES |
| Done-when list | Rust API tests (5), schema tests (+4), jsdom form/settings/fleet/descriptor tests (12), neutrality, headless UI walkthrough: all present and green, mapped one-to-one in `handoff-T-green.md`; U walked the create and save round trips; I re-confirmed the same states in a second independent run | YES, with the clippy caveat below |
| `cargo clippy --all-targets -- -D warnings` passes | Does not pass on this host's rustc 1.99.1, only on `unused_async_trait_impl` and `assert_is_empty` in files #172 does not touch (T and F both recorded it; same finding as #169's S). With those two allowed the workspace is clean | NO, pre-existing and outside the diff |

## Quality audit

| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | `runnerGroupId` becomes optional on both `POST` and `PUT`; the two behaviour changes (422 to 400, 400 to 404) are in the CHANGELOG. Error strings unchanged. |
| Error handling | PASS | Padded image on a GitLab pool, GitHub empty image, runner group 0 and invalid-body-to-missing-pool all return the specified status and message (live). Validation stays before the admin check, so no authorisation check is weakened or reordered past a reader. |
| UI/UX match to spec | PASS | Rendered text, label tag, help and placeholder match the wireframe exactly at three widths; GitHub UI is pixel-identical. |
| Accessibility | PASS | No new control. The Docker image input keeps its `<label for>` and `id`; "optional" is real text inside the label (not colour alone); `required` is dropped exactly when the field is optional, so assistive technology and the browser agree. |
| Architecture gate | PASS | A round 2: PASS; A-dup: PASS. I found no quality finding that conflicts with either. |
| Code organization | PASS | One factory instead of a second update schema; display and help copy are pure helpers beside `defaultDockerImage`; new-pool image state holds only typed text (`null` = untouched), which is what makes the typed image survive a target switch. Comments are accurate; no dead code, commented-out code or TODOs added. One cosmetic line wrap in `docs/gitlab-runners.md` ("using" alone on a line) that T already noted. |
| Security | PASS | No new input path or auth change; `dockerImageOptional` is never sent; a non-empty GitLab image is still validated; the stored empty image only ever resolves to a pinned constant, never to user text. |
| Performance | PASS | A schema built per call only when `dockerImageOptional` is true; one registry lookup per render in the routes. |
| Visual regression | PASS | Pixel pass above. The project has no Playwright VR suite or axe configuration to run (`T` recorded the same). |
| Naming consistency | PASS | `defaultDockerImage`, `dockerImageHelp`, `shownDockerImage`, `dockerImageOptional` follow the existing `labelRules`, `readOnlySummaries` and `poolId` naming. |
| Test quality (`testing/test-quality.md` section 7) | PASS | See below. |

### Test-quality audit (section 7 rubric)
- **One behaviour, fails for the right reason.** T-red recorded each RED failing on the feature assertion, and T-green's two mutations (drop `&& !gitlab` in `models.rs`; drop `!dockerImageOptional &&` in `validateProvider`) turned exactly the matching tests red. The jsdom tests read rendered text, `required`, `placeholder` and the outgoing request body, not hook or helper names, so they assert behaviour.
- **Cheapest sufficient tier, no re-proving across tiers.** Rules that depend on the handler (scope, stored value, status precedence) go through the real router in Rust; schema rules at unit level; form, read-only, detail and fleet in jsdom with the production registry; neutrality at source level. The Rust tests do not repeat what the schema tests prove or the reverse.
- **Proportionate.** 5 Rust tests, 4 schema cases (plus a 2-case `it.each`), 12 jsdom tests and 3 source guards for one fix that touches create, update, three read/write forms and a fleet fallback. The GitHub-keeps-its-400s Rust test covers 2 scopes by 2 rules by 2 verbs in one test; each cell is a distinct branch (image rule vs runner-group rule, create vs update validator entry), so none is a repeat.
- **Smells.** No assertion-free, tautological, unreachable-outcome, mock-shaped or snapshot test. Guard tests (those that pass before F's change, by design) are labelled as guards in T-red.
- **One merge candidate, not blocking.** `tests/gitlab-pool-schema.test.ts:151` ("the exported update schema keeps the GitHub rule") duplicates the signal of the third assertion in `:144` (`parseUpdateRunnerPoolInput` with no option, which parses through the same exported `updateRunnerPoolSchema`). It is four lines, it pins a clause the brief names explicitly ("The exported `updateRunnerPoolSchema` keeps the GitHub rules"), and it adds the `path: ["dockerImage"]` assertion the other lacks, so I judge it a small, defensible guard rather than over-production. If O wants the suite at its minimum, fold its `path` assertion into `:144` and delete `:151`.

## Scope check
F delivered the brief's scope: Rust validation and the update-handler ordering, one descriptor key, three pure helpers, the neutral schema parameter, the action contract, the two forms, the read-only view, the pool detail row, the fleet fallback, the docs and the CHANGELOG, plus T's tests. 12 production and doc files, 5 test files (plus the 13 call updates in the `models.rs` test module). No over-delivery: no change to the connection form, verify code, Tart script or agent (#171/#173/#174 territory), no migration, no new file or module under `src` or `crates`, no GitHub or Bitbucket behaviour change. Nothing under-delivered.

## Verdict

PASS - all acceptance criteria met, spec-compliant, quality acceptable. Ready for O to commit.

## Advisory notes
Non-blocking; none is part of #172's scope as the brief wrote it, and none was applied.

1. **API reference docs now say something untrue for GitLab pools.** `docs/openapi/gridops-v1.json` (the `POST /api/v1/runner-pools` description, "Every provider in `providers` needs its image ... or the request is refused with `400`") and `docs/fleet-automation.md` lines 353-363 ("adding Docker without `dockerImage` likewise") do not mention that a GitLab Docker pool may leave its Docker image empty or its `runnerGroupId` out. Those pages do not describe GitLab pools at all today (no `gitlabConnectionIds` or `runUntagged` in them), so this belongs with the epic's API-docs sweep rather than this issue. Suggested branch `claude/docs-api-gitlab-pools`.
2. **CHANGELOG wording.** The "Changed" bullet names a GitHub *organization* pool without `runnerGroupId`. A GitHub *repository* pool without it also moves from serde's 422 to the same 400 (the server validates before normalising to 1), so "a GitHub pool" would be exact. The form always sends 1, so only direct API clients see it.
3. **Pool detail Image line is clipped.** "Default (gitlab/gitlab-runner:ubuntu-v19.6.0)" shows as "Default (gitlab/gitla..." in the 1280 sidebar (and "Default (gitlab/gitlab-runner:..." at 360), the way a long GitHub image is clipped today; the hover title and the read-only settings view carry the whole text. The wireframe accepts this ("Truncates as today"), so no change was made; if O wants it fully visible, that is a copy/layout decision for the property rows in general, not for #172.
4. **Pre-existing CI hygiene (outside this diff).** The two rustc-1.99.1 clippy lints and the macOS-only `release_preflight` test failure (`the_release_preflight_fails_early_without_a_macos_arm64_machine`, which probes the build host) were already recorded by F, T and #169's S; they deserve their own issue. I did not re-run the Rust suite, because no code has changed since T-green.
5. **U's two environment notes stand.** The headless-Chromium rejection of the pool-name `pattern` attribute (`v`-flag character class) and the seed-fidelity 500s on the repository and runner-group lists are not #172 defects; my capture run saw the same 500s in both builds.
