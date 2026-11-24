# Handoff-U: Phase 8 - UI walkthrough for #241 (fleet health wave-1 review fixes)

**Date:** 2026-11-24
**Branch:** claude/fh-241 (head 6b7ca49, F's implementation; T-green confirmed GREEN at the same head)
**Issue:** #241
**Handoff-T-green reviewed:** docs/handoffs/241/handoff-T-green.md (GREEN)
**Handoff-F reviewed:** docs/handoffs/241/handoff-F.md
**Approved wireframe:** docs/handoffs/241/wireframe.html + docs/handoffs/241/decisions.md

## T precondition
Confirmed. `handoff-T-green.md` is GREEN (701/701 vitest, lint, build/typecheck all PASS) at head 6b7ca49, with no further diff since.

## Wireframe conformance
No divergence. Every frame the wireframe (`docs/handoffs/241/wireframe.html`) specifies was reproduced live and matched exactly:
- Severity glyphs/shapes (critical = filled red cross circle, warning = amber outlined clock circle, info = grey outlined slash circle) with hidden text exactly "Critical" / "Warning" / "Info" (wireframe lines 64-68) — verified byte-for-byte against the live SVG markup.
- The same-page concern link (`#machine-<id>`) expanding the machine row in place (wireframe lines 77-82) — verified live.
- The resource bar as a non-overlapping stacked set with a hatch for "used over reservation" (wireframe lines 134-166) — verified live, including a real hatch case (callisto's memory bar).
- The "Machine detail" slot always has content — the unchanged prompt when nothing is expanded, "Showing **<name>**: ... in its row above." when one is (wireframe lines 176-194) — verified live, exact wording, for three different machines (Jupiter, MacBookPro, europa).
No pixel/WCAG judgment is made here (that is S's); this is structural/behavioral/copy conformance only.

## Run environment
GridOps has no e2e seed script and its only login is GitHub OAuth, so this pass followed the same technique recorded in prior U rounds (#145, #159): built and served the real Rust API against a throwaway SQLite DB, seeded directly with a hand-minted session cookie. The fleet-health endpoint (`GET /api/v1/fleet/health`) is still served entirely from `fleet_health::fake::FakeFleetHealthSource` (the real DB source, `DbFleetHealthSource`, is a stub pending #204 per `crates/gridops-api/src/fleet/health_source.rs`), so no fleet-specific DB rows were needed — only an admin user + session to pass `require_admin`.

- **Build:** `CARGO_BUILD_JOBS=2 cargo build -p gridops-api` (debug profile). Shared cargo target dir (`~/.cache/cargo-target/gridops`, per the untracked `.cargo/config.toml`); binary at `~/.cache/cargo-target/gridops/debug/gridops-api`. `dist/` (vite build) reused as-is (F's self-check artifact, mtime after F's last commit).
- **Serve:** the binary on `127.0.0.1:19341`, with:
  `GRIDOPS_DATABASE_PATH=<scratch>/u241.sqlite`, `GRIDOPS_BASE_URL=http://127.0.0.1:19341`, `GRIDOPS_API_BIND=127.0.0.1:19341`, `GRIDOPS_WEB_ROOT=<worktree>/dist`, `GRIDOPS_SESSION_SECRET=u241-session-secret`, `GRIDOPS_ENCRYPTION_KEY=<random base64>`, `GRIDOPS_FLEET_ALLOW_LOOPBACK_HTTP=true`, `GITHUB_CLIENT_ID=u241-client`, `GITHUB_CLIENT_SECRET=u241-secret` (only so `validate_api()` passes at startup; no GitHub API call was made).
- **DB:** migrated automatically on first connection (`sqlx::migrate!`, confirmed via `200` on `/`), then seeded via a throwaway Node+sqlite3 script (`<scratch>/seed.mjs`, same HMAC-SHA256 session-signing scheme as `docs/handoffs/145/screenshots/repro/seed.mjs`) with exactly one admin user (`user-admin`) and one session, nothing else — no runner pools / repositories / connections needed, since the health endpoint reads the fake fixture, not the DB.
- **Login:** session cookie `gridops_session=<token>.<HMAC-SHA256(GRIDOPS_SESSION_SECRET, token) base64url>` set via `document.cookie` (Playwright MCP `browser_evaluate`), confirmed by a subsequent load of `/` rendering the authenticated shell (not a login redirect).
- **Driver:** Playwright MCP, headless Chromium, direct `mcp__playwright__*` tool calls (not `u-drive.mjs` — GridOps is a React/TanStack-Router SPA with no HTMX, the same reasoning prior U rounds for #141/#145/#159 recorded). Viewports: default desktop and 360×800 mobile.
- **Cleanup:** server process killed (confirmed down: `lsof -i :19341` empty, `curl` returns no response), throwaway SQLite DB and seed script removed from the session scratchpad. `target/`/`~/.cache/cargo-target` build artifacts and `dist/` are pre-existing build outputs, left in place (not session-owned).
- **State safety:** dedicated throwaway DB and admin user created for this pass only — no shared/account-wide setting was read or touched.

## State matrix walked

| # | Page/control | State | Reached via | Result |
|---|---|---|---|---|
| 1 | Overview → Health tile | fleet score 74, "2 critical concerns" | Hard load (`/`) | PASS — tile text matches the fixture's fleet score/critical count |
| 2 | Nav → "Health" link | — | **SPA nav** (sidebar link click, URL updates to `/fleet/health` without reload) | PASS — page renders fully, 0 console messages |
| 3 | Areas of concern — severity glyph/label (item 1) | critical (europa offline), warning (callisto disk), info (category-other) — **all three severities present**, zero/empty not applicable (list always has ≥1 concern in the fixture) | SPA (on page) | PASS — each row's `.sr-only` text is exactly "Critical"/"Warning"/"Info"; each row's `svg` is `aria-hidden="true"` with a **distinct** glyph: critical = filled red circle + cross path (`fill="var(--danger)"`), warning = outlined amber circle + clock path (`stroke="var(--warning)"`), info = outlined grey circle + slash path (`stroke="var(--neutral)"`) — shape differs per severity, not colour alone |
| 4 | Areas of concern — "Show all 13" | many (13 total, 5 shown by default) | SPA (click) | PASS — all 13 render, same severity-glyph rule holds across the full list |
| 5 | Fleet score card — component weight (item 3) | 5 components, weights 40/20/15/15/10 | SPA (on page) | PASS — rendered as whole percents (`40%`, `20%`, `15%`, `15%`, `10%`), no raw float, sums to 100 |
| 6 | Fleet score card / machine score components — API deduction text (item 4) | every component has a non-empty `deduction` in the fixture (no null-deduction case exists in fixture — the fallback path is source-verified by T, not independently reproducible live without a custom fixture) | SPA | PASS — card reason lines and machine-row `title` tooltips show the **real per-component API text** (e.g. "Queue p90 was 4 min 30 s against a 1 min target: −16.", "Not enough data: no samples from this machine in the window."), not a fixed generic string |
| 7 | Machines table — same-page concern link expands row (item 2, the core regression) | zero→one: no machine expanded → Jupiter expanded via the concern "Jupiter scores 63" link (`/fleet/health#machine-<id>`), a **same-route** navigation | **SPA nav** (real `Link` click, URL hash changes without reload) | **PASS — this is the regression the brief describes, confirmed fixed.** Before clicking: Jupiter's row button `aria-expanded="false"`. After clicking the in-page concern link: `aria-expanded="true"`, `[data-machine-detail]` = Jupiter's id, "Machine detail" slot reads "Showing Jupiter: ... in its row above." All live-read, not from a second render |
| 8 | Same, hard-reload spot check | deep-link `/fleet/health#machine-<Jupiter-id>` loaded directly (`page.goto`) | **Hard load** | PASS — identical result to the SPA path: Jupiter pre-expanded, same detail text. No discrepancy between SPA-nav and hard-load |
| 9 | Hash deep link — unknown/malformed host (error state) | `/fleet/health#machine-does-not-exist` | Hard load | PASS (zero/error state) — no row expands, "Machine detail" falls back to the unchanged prompt, 0 console errors |
| 10 | Machine row toggle — uncontrolled/manual click (not via hash) | callisto expanded by clicking its row button directly | SPA (click) | PASS — only callisto's `aria-expanded` flips true; Jupiter and others remain false (single-expansion works); URL hash is left as-is (untouched by a direct row click, matching F's controlled/uncontrolled design) |
| 11 | Resource bars — CPU/Memory stacked segments, no overlap (item 5) | Jupiter: used (74.9) + reserved-unused (21.75, opacity 0.45) + free (367.3) = 464 (exact track width) — **used ≤ reservation, no hatch** | SPA | PASS — three non-overlapping `<path>` segments, `var(--info)` / `var(--selected)` fills only, correctly summing to the full track width — the old bug (used drawn over reserved) is gone |
| 12 | Resource bars — hatch case, **used > reservation** | callisto Memory: 11.7 GB used of 8 GB reserved / 16 GB total | SPA | **PASS — the hatch (over-reservation) segment renders live**: `fill="url(#resource-over-_r_23_)"` for the over-reservation width, plus the straight used-within and free segments; pattern `<rect>` children use only `var(--warning)` / `var(--panel)` fills (no hex/rgb/named colour) |
| 13 | Resource-bar hatch pattern id uniqueness | CPU chart vs Memory chart on the same expanded row | SPA | PASS — distinct ids (`resource-over-_r_q_`, `resource-over-_r_r_` for Jupiter; `resource-over-_r_23_` for callisto's memory) — no id collision |
| 14 | "Machine detail" slot — no machine expanded (item 6, zero state) | default load | Hard load + SPA | PASS — "Expand a machine above to see what it is doing now and what it ran in the window." (unchanged copy) |
| 15 | "Machine detail" slot — machine expanded (item 6) | Jupiter, MacBookPro, europa each expanded in turn | SPA | PASS — "Showing **<name>**: what it is doing now and what it ran in the window are in its row above." for each, machine name substituted correctly each time |
| 16 | Live strip — "Oldest job waited" (item 7) | oldest queued job | SPA | PASS — "Oldest job waited 84 h 35 min · acme/app · integration tests (2/4)" rendered; value advanced (84 h 35 min → 84 h 37 min) across the session as the clock moved, confirming it is computed against `now`, not a fixed `waitMs` |
| 17 | Window picker (1h/24h/7d) | toggling window | SPA (click "1 hour") | PASS — `aria-pressed` moves to "1h", page content updates, 0 console errors |
| 18 | 360px mobile — full page | all sections | resize + reload | PASS — no horizontal overflow (`scrollWidth === clientWidth === 360`); screenshot saved |
| 19 | 360px mobile — machine row expand/detail slot | europa expanded | SPA (click, at 360px) | PASS — same expansion + detail-slot text as desktop, no overflow, 0 console errors |

**Zero/empty/error states walked (mandatory):** row 9 (unknown/malformed hash → no expansion, unchanged prompt); row 14 (no machine expanded → unchanged prompt); MacBookPro (row 6) is the fixture's only null-score machine, walked for the deduction-fallback text on every one of its five components.

**Console / state assertions:** `browser_console_messages` (warning level, cumulative, `all: true`) returned **0 messages** at every checkpoint across the whole session — initial load, every SPA nav, every click, both window changes, both viewports, and the malformed-hash case.

## Findings
None blocking. Zero behavioral defects, zero wireframe divergence.

1. **Item 2's core regression (same-page concern link not expanding the row) is confirmed fixed, live, on the exact path the brief describes**: a same-route `Link` navigation to `#machine-<host>` now expands that machine's row and updates the "Machine detail" slot, with no reload — and the hard-reload deep-link path produces the identical result.
2. **Item 5's hatch segment (used-over-reservation) was independently reproduced live** (callisto's memory bar), not just inferred from source — confirming T-green's hedge is resolved.
3. **Item 1's severity glyphs are visually and structurally distinct per severity** (filled cross vs. outlined clock vs. outlined slash), each paired with the correct hidden "Critical"/"Warning"/"Info" text, live in the DOM.
4. **Item 6's named-machine copy is correct per machine name**, verified for three different machines, and the unexpanded/unknown-hash fallback keeps its original wording.
5. **No console error or warning anywhere in the session**, across every interaction, both viewports, and the hard-reload spot check.
6. One pre-existing, out-of-scope note (not a #241 defect, carried from #197/#194's design, confirmed unaffected): the areas-of-concern links for host-offline/disk/uncertain/start-gate concerns navigate to `/fleet/machines/<id>` (a different route), not an in-page hash — this is correct per the fixture's own `link` field (only the host-score/host-slow/host-idle-while-queued concerns use the in-page hash), and matches the wireframe's distinction.

## Evidence
- Screenshot: `docs/handoffs/241/screenshots/u-health-mobile-360.png` — full `/fleet/health` page at 360px, no overflow.
- Live DOM captures (via `browser_evaluate`) of: every concern row's `.sr-only` text + `svg.innerHTML` (3 distinct severities); the fleet-score component list's weight percentages and deduction text; the machine table's per-component `title` tooltips (MacBookPro's five null-score components); `aria-expanded` state across all 5 machine rows before/after each interaction; the `[data-machine-detail]` attribute and "Machine detail" slot text for Jupiter/MacBookPro/europa; the resource-bar `<path>`/`<pattern>` fills, widths and opacities for Jupiter (no hatch) and callisto (hatch present) — see the state-matrix table above for exact values captured.
- `browser_console_messages` (`all: true`): `Total messages: 0 (Errors: 0, Warnings: 0)` at the final checkpoint, covering the entire session.
- API sanity cross-check (`curl` against the live server, seeded session cookie): `GET /api/v1/fleet/health?window=24h` returns 200 with the expected fixture-shaped JSON (score 74, components with weights/deductions) — confirming the UI's rendered values trace back to the real endpoint, not a mock.

## Verdict
**PASS** — every item in the brief (1 severity glyph+text, 2 same-page concern link expansion incl. hard-reload parity, 3 whole-percent weights, 4 API deduction text with fallback, 5 non-overlapping stacked resource bars incl. the hatch case, 6 named-machine detail slot, 7 clock-relative oldest-wait) was independently reproduced live in a real headless browser, on both the SPA-navigation and hard-load paths, at desktop and 360px mobile, with a clean console throughout. No behavioral defect, no wireframe divergence. Ready for S.
