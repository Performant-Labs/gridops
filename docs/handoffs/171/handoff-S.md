# Handoff-S: Phase 10 - GitLab token needs manage_runner; refused deletions become pool warnings (#171, #174)

**Date:** 2026-11-21
**Branch:** claude/gitlab-171-scopes-cbe8b5 (head 0444979; code unchanged since 665c126)
**Issue:** #171 (+ #174)
**Brief:** docs/handoffs/171-brief.md
**Handoff-T reviewed:** docs/handoffs/171/handoff-T-green.md (and handoff-T-red.md)
**Handoff-A reviewed:** docs/handoffs/171/handoff-A.md (plan) and docs/handoffs/171/handoff-A-dup.md (anti-duplication gate)
**Handoff-F reviewed:** docs/handoffs/171/handoff-F.md
**Also read:** handoff-D.md, handoff-U.md, evidence.md, wireframe.html
**Operator-facing report:** [`handoff-S-report.html`](handoff-S-report.html)

## A precondition
Confirmed. A's plan gate returned PASS (0 block, 6 warn, all six taken up by F and T) and the A-dup gate on the finished diff returned PASS.

## T precondition
Confirmed. T-green reports "Blocking issues: None" after the round-1 re-entry. The one failing test it records (`gridops-api` `fleet_http release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`) is host-dependent, fails at the pre-branch commit 7164fda too (F ran that baseline), and lives in code this diff does not touch. I accept that and do not count it against this change.

## Visual-diff-tool precondition
Confirmed, with one honest deviation from the role text:
- The `mcp__Claude_in_Chrome__*` tools are not in my tool list. I did not use them. The renders were taken in headless Chromium through Playwright, which is the route the role document itself names for the 360/768/1280 captures, and which keeps to the user's headless rule (nothing opened on the desktop).
- Playwright (`playwright-core`, installed in the scratchpad, not in the project) used the cached Chromium headless shell revision 1255. ImageMagick `compare` is on PATH.
- There is no Playwright VR suite or baseline set in this project, so I could not run one.

## Preview / Spec sanity check
No defect in the source of truth. The wireframe's one "illustrative" element (metadata keys) was settled by A and F; the help copy is plain text, which the existing string `help` slot requires. The brief's `"warn"` level hazard (the API's own "Runner deleted" row renders green) is a defect that predates this story and is out of its scope (see Advisory notes).

## Visual diff results
The only reference with pixel meaning is the **pre-branch frontend**. The wireframe is a low-fidelity sketch in a different visual style and was never rendered, so it was used for copy and structure only. I built the frontend from `git archive 7164fda` ("before") and from this tree ("after"), served each with the same API binary against a throwaway database with identical seed data (one admin session, one pool, one "GitLab runner not deleted" warning with `runner_id` NULL, one info row), and captured both at 360x800, 768x1024 and 1280x800.

The top 18 px strip (the build stamp) is blacked out in every image: my before build has no git metadata and reads "build unknown", which is a property of my harness, not of the change.

| Surface | Viewport | Pixels different | Delta % | Diff PNG | Composite |
|---|---|---|---|---|---|
| Add connection > GitLab (the changed help paragraph) | 360x800 | 7,016 | 2.436% | screenshots-s/t3-connection-form-360-diff-20261121.png | screenshots-s/t3-connection-form-360-composite-20261121.png |
| Add connection > GitLab | 768x1024 | 3,929 | 0.500% | screenshots-s/t3-connection-form-768-diff-20261121.png | screenshots-s/t3-connection-form-768-composite-20261121.png |
| Add connection > GitLab | 1280x800 | 10,342 | 1.010% | screenshots-s/t3-connection-form-1280-diff-20261121.png | screenshots-s/t3-connection-form-1280-composite-20261121.png |
| Pool overview with the seeded deletion warning | 360x800 | 0 | 0.000% | screenshots-s/t3-pool-events-360-diff-20261121.png | screenshots-s/t3-pool-events-360-composite-20261121.png |
| Pool overview | 768x1024 | 0 | 0.000% | screenshots-s/t3-pool-events-768-diff-20261121.png | screenshots-s/t3-pool-events-768-composite-20261121.png |
| Pool overview | 1280x800 | 0 | 0.000% | screenshots-s/t3-pool-events-1280-diff-20261121.png | screenshots-s/t3-pool-events-1280-composite-20261121.png |

(AE counts from IM7 come out fractional; they are rounded here. The percentages use width x height of the viewport.)

### Per-section delta description
- **Access token help paragraph, all three viewports: DELTA, intended.** The new sentence is one line longer (4 to 5 lines at 360, 2 to 3 lines at 768 and 1280). It is the text the operator-approved wireframe specifies word for word, and F lists it under "What was done". Not a regression. The 360 viewport's 2.436% is in the 2-5% band, so by the role's rule it passes only because every red region is accounted for: the paragraph itself and the content that moves because of it.
- **Content below the paragraph (the "Connect GitLab" / "Back" row): MATCH in content,** about 20 px lower because of the extra line.
- **Fields above the paragraph at 360 and 1280: MATCH in content.** They appear shifted because the dialog scrolls inside itself and both runs scroll the help paragraph to the centre (same step both sides), so the longer paragraph changes the offset. At 768 the dialog does not need to scroll and the fields sit in the same place.
- **Nothing clipped, nothing overflowing sideways, no horizontal scroll** at any viewport on either side (checked with `scrollWidth > clientWidth`).
- **Pool overview: no visible differences,** and 0 px at all three viewports. The amber warning dot, the event name "GitLab runner not deleted", the truncated message, the em dash in the Runner column for a sweep row and "2h" render exactly as before. `pool-events.tsx` is unchanged by this diff, which this confirms.

The seeded warning row in my capture also confirms the brief's "confirm a warning row appears there" requirement independently of U.

## Spec compliance
Checked against `docs/handoffs/171-brief.md`, line by line, in the source.

| Requirement | Compliant | Evidence |
|---|---|---|
| `verify_access` requires `create_runner`, `manage_runner`, `read_api`, in that order; `api` satisfies each; first missing is `MissingScope("<scope>")` | YES | `http.rs:316`, `for needed in ["create_runner", "manage_runner", "read_api"]`, `has` accepts `api`. Table `verify_access_reports_the_first_scope_the_token_lacks`, 12 cases |
| Create, token replacement and Verify all reject | YES | `resources/gitlab.rs:139, 254, 317` all call `verify_access`; three API tests |
| API message format unchanged | YES | `gitlab_message` untouched; tests assert "the gitlab access token is missing the manage_runner scope." |
| Fake behaves like the client "if it models scopes" | YES | It does not model scopes (`fake.rs` never inspects a token); left unchanged on purpose (A W6) |
| Comments updated ("api grants both", `gitlab_live.rs` header) | YES | `http.rs:304-307`, `gitlab_live.rs:5-6`; no "grants both" remains in `crates/`, `src/`, `docs/` |
| Deletion refused with an error other than `NotFound` records a warning `runner_events` row on the pool, with `runner_id` where there is one | YES | `deregister`'s `Err` arm calls `record_refused_deletion`; level is exactly `"warning"`; `runner_id` bound for per-runner rows, NULL for sweep rows |
| Stable event name; message names connection, GitLab runner id, client-safe reason; never the token | YES | `DELETION_REFUSED_EVENT = "GitLab runner not deleted"`. For `MissingScope` the text is exactly the brief's example ("...missing the manage_runner scope. Give the connection a token with manage_runner (or api); the orphan sweep retries it."). `client_safe_reason` keeps `Upstream` detail out. Tests assert no `glpat-`/`glrt-` and no upstream detail |
| Covers every path in Need 2 | YES | One write inside `deregister` covers `deregister_runner`, the failed-start cleanup (`main.rs:2887`), `fleet_rollback` and `fleet_material`'s earlier-attempt delete (the fourth caller the brief missed); the sweep writes from `sweep_connection`. One test per caller |
| Sweep: warn each pool linked to the connection; deduplicate; rule documented in a comment; tested | YES | `warn_linked_pools`/`warn_pool`: one row per pool, `connectionId` and reason code per hour, read back from `runner_events`; doc comment on `warn_linked_pools`; test backdates rows |
| Keep the `tracing::warn!` lines; event-write failure logged, never aborts deletion or the sweep | YES | Lines kept; both failure paths logged and swallowed; tested with failing-insert triggers (all pools, and pool-a only) |
| No migration; confirm a warning row appears in the pool's Events list | YES | No `migrations/` change. Appears in the list: U and I both rendered a seeded row |
| Token help text: three scopes (or `api`), Maintainer or higher on a project, Owner on a group, "GridOps checks both", one short paragraph | YES | `gitlab.tsx:342`; rendered in the live Add dialog (U and S) |
| `docs/gitlab-runners.md` no longer says `manage_runner` is unchecked; mentions the runtime warning | YES | Lines 27-33 and 157-161 |
| Sweep the rest of `docs/`, README, CHANGELOG for other scope lists; CHANGELOG line | YES | Grep of `docs/` (outside handoffs), `README.md`, `CHANGELOG.md`, `src/`, `crates/` finds no other GitLab scope list; the table at `gitlab-runners.md:23-25` already listed all three; new "Fixed" entry under Unreleased |
| #174: help no longer says Owner for a project | YES | Vitest negative assertion; live text |
| Out of scope untouched (#172 pool forms/validation, #173 `scripts/`, Tart agent, `docs/macos-runners.md`) | YES | `git diff 7164fda HEAD --stat` lists none of them |
| No deploy; no token anywhere | YES | No deployment files; token assertions in the tests |

Done-when:
- `http_tests.rs` table: YES (all three pass; `api` alone; `create_runner`+`read_api` fails with `manage_runner`; the old missing-`create_runner`/`read_api` cases still name the right scope).
- API tests: YES, 400 with the exact message on create, token replacement and Verify, nothing stored or changed.
- Reconciler tests: YES (one warning naming connection, runner id and `manage_runner`; none for `NotFound`; sweep dedup; no token).
- Vitest: YES.
- Headless UI walkthrough: YES (U's Phase 8 and my own capture above; U's gap on the live refusal toast is covered below).
- Docs: YES.
- Commands: re-run by me, see below. `cargo test -p gridops-api` has one host-dependent failure that predates the branch (see T precondition).

### Independent re-runs (CARGO_BUILD_JOBS=2, RUST_TEST_THREADS=2)
- `cargo test -p gridops-reconciler gitlab_lifecycle_tests` -> 32 passed.
- `cargo test -p gridops-core --lib gitlab::` -> 36 passed.
- `cargo test -p gridops-api --bins gitlab_tests` -> 64 passed.
- `npx vitest run tests/gitlab-descriptor.test.ts` -> 53 passed.
- I did not re-run fmt, clippy, lint, typecheck or the full `npm test`. F and T report them clean with matching numbers, no production code has changed since, and re-running them would only repeat T's gate.

## Quality audit
| Area | Result | Notes |
|------|--------|-------|
| API consistency | PASS | The 400 and its message are unchanged; the new case is one more scope through the same path. |
| Error handling | PASS | A GitLab refusal writes the event; a local failure (connection gone, vault unreadable) does not and is only returned. The event's write failure is logged and never hides the deletion error or stops the sweep. One failing pool does not silence the others (round-1 W-1 fix, test-pinned). |
| UI/UX match to spec | PASS | Help text and warning rows match the approved wireframe; headless captures at 360, 768, 1280 show no clipping or overflow. |
| Accessibility | PASS | No new control. The help is the same paragraph element, and the warning row reuses the existing table (level shown by dot plus the event name, expandable row unchanged). |
| Architecture gate | PASS | A: PASS (plan) and PASS (A-dup). No S finding conflicts with either. |
| Code organization | PASS | One seam for the per-runner event, one helper for the sweep, one shared insert (`pool_event` delegates to `runner_event`). Doc comments carry the dedup rule and its reason. |
| Security | PASS | No token reaches an event, log field or metadata (asserted for `glpat-` and `glrt-`). `Upstream`'s free-form detail is deliberately kept out of operator-visible text (`client_safe_reason`, round-1 B-1 fix, pinned by the 502 test). `MissingScope`'s scope name is filtered by the HTTP client to lowercase letters, `_` and spaces (evidence.md). |
| Performance | PASS | Sweep dedup is one `EXISTS` per linked pool per refused delete, on indexed columns (`pool_id`, then `created_at`/`json_extract` on a small set). Worst case is bounded by the hourly rule. |
| Visual regression | PASS | See the pixel table. No Playwright VR suite exists in the project. |
| Naming consistency | PASS | `runner_event`/`pool_event`, `DELETION_REFUSED_EVENT`, `refusal_code`; terms match the brief and docs ("GitLab runner not deleted"). |
| Test quality (`testing/test-quality.md` §7) | PASS | See below. |

### Test quality (§7)
- **Core table (12 cases):** one behaviour (the three-scope rule and its order). Fails on the actual rule (T-red showed `Ok(())` where `MissingScope("manage_runner")` was expected). Cheapest tier that sees the real client. Proportionate: each case is a distinct branch of the order or of `api`.
- **API pins (3):** these pass before the feature, as T-red said they would, because the fake does not model scopes. They are not RED tests but they are not smells either: each pins a different route's 400, the exact message and "nothing stored", and none repeats the core table's rule. Cheapest tier for that wiring.
- **Reconciler (7 new or extended):**
  - The four per-caller tests (`deregister_runner`, failed-start cleanup, `fleet_rollback`, `fleet_material`) share one branch in `deregister` but each guards a distinct wiring risk: a wrong `pool_id` or `runner_id` at that call site, which the `runner_id` foreign key would make fail silently. The brief asks for each path. Keep.
  - `a_failed_gitlab_delete_...` (extended): pins both "502 writes a row without the `manage_runner` remedy" and, since round 1, that no upstream detail leaks. This is the test that gives B-1 teeth.
  - `a_pool_whose_sweep_warning_cannot_be_written_...`: separates "continue" from "stop" for one failing pool (round-1 W-1). T showed by mutation that it fails without the fix.
  - The big sweep test asserts five states of one dedup state machine (fan-out, same-reason-within-hour, new reason, hour passed, unwritable event). It is long and could in principle be five tests, but each phase builds on the previous phase's rows, so splitting would multiply setup. It asserts exact row counts per pool, so removing the dedup, the connection key or the reason key all fail it. Acceptable; noted below.
- **Vitest (2):** one for the three scopes, one for the role wording (positive for project/group, negative for Owner-on-project). Behaviour of the user-visible string; reads the help as plain text, as the existing test does.
- **No smells found:** no assertion-free or tautological test, no mock-shaped call-count test, no snapshots, no coverage padding. No "delete or merge" finding. Deliberately not authored (and I agree): a docs test and a pool-events UI test.

## Scope check
Exactly the brief's scope. 11 files changed: the core rule and its two comments, the reconciler's event and sweep logic (`gitlab_lifecycle.rs`, `main.rs`), the help string, the docs page, one CHANGELOG entry and four test files. No migration, no new component, no `fake.rs` change, nothing under #172's or #173's paths. Two behaviours go slightly beyond the brief's literal text and are both settled by A and visible to the reader: the per-runner event is written in `deregister` (which also covers `fleet_material`'s earlier-attempt delete), and the remedy sentence is only added for `MissingScope`/`Unauthorized`.

## Verdict

PASS — all acceptance criteria met, spec-compliant, quality acceptable. Ready for O to commit.

## Advisory notes
Non-blocking; none is a condition of the verdict.
1. **U's open gap (Screen 1c, the live refusal toast).** Neither U nor I reproduced the 400 on a live server: `verify_access` calls the real GitLab API and none was available. The message is fixed by the API's `gitlab_message` (unchanged) and pinned by the three API tests plus the core table. I accept this as covered at the right tier. The first real-world proof is epic #136's live end-to-end run against gitlab.com (the same check C6 that found the defect); someone should re-run it with a token that has `create_runner` and `read_api` only and confirm the create, token replacement and Verify refusals, and again with all three scopes to confirm the runners are deleted.
2. **`"warn"` level on the API's "Runner deleted" event (`crates/gridops-api/src/resources.rs:4899`).** It renders green in the pool's Events list (only `"warning"` is amber). A noted this in W3; it is a pre-existing defect and out of this brief's scope, so O should open a follow-up issue.
3. **Host-dependent `release_preflight` test.** It fails on macOS hosts with and without this change; worth its own issue.
4. **"GridOps checks both before saving"** (A-dup W2): with three scopes listed, "both" could be read as the two scope groups. The brief and the operator-approved wireframe state the sentence this way, and it reads naturally as "the scopes and the role", so I leave it. If the operator wants it clearer, "checks these" is a one-word copy change.
5. **`refusal_code`'s `NotFound` arm** (A-dup W1) is unreachable because `deregister` and the sweep both treat `NotFound` as success. Harmless; the match has no wildcard on purpose so a new `GitLabError` variant must choose a code.
6. **Reconciler's "Pool reconciliation failed" event** (`main.rs:677-681`, F's note) writes `error.to_string()` and can carry `GitLabError`'s `Upstream` detail. It predates #171. The real client only puts fixed text in `Upstream`, so nothing sensitive reaches it today. A follow-up issue is reasonable if the "no upstream detail in operator text" rule should extend to reconciler events.
7. **Capture harness (for whoever re-runs this):** servers were started from a copy of the existing debug binary under a unique name, on loopback ports 19401 and 19402, with random secrets and throwaway SQLite files in the session scratchpad; all were stopped and removed afterward. Nothing in the project outside `docs/handoffs/171/` was modified, and no shared or account-wide state was touched.
