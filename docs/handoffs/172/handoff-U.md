# Handoff-U: Phase 8 - UI walkthrough for #172 (GitLab Docker pool default image, no runner group)

**Date:** 2026-11-21
**Branch:** claude/gitlab-172-pool-image-bb7d36
**Issue:** #172
**Handoff-T-green reviewed:** docs/handoffs/172/handoff-T-green.md (GREEN)
**Handoff-F reviewed:** docs/handoffs/172/handoff-F.md
**Handoff-A-dup reviewed:** docs/handoffs/172/handoff-A-dup.md (PASS)
**Approved wireframe:** docs/handoffs/172/wireframe.html + docs/handoffs/172/handoff-D.md

## T precondition
Confirmed. `handoff-T-green.md` and `handoff-A-dup.md` are both PASS at the current head (`0c4bbec`), no `src`/`crates` diff since F's last commit.

## Wireframe conformance
No divergence. Every frame the wireframe specifies was reproduced live and matched exactly:

- **1a / no target or GitHub target:** Docker image row required, no "optional" tag, prefilled with `ghcr.io/actions/actions-runner:latest`.
- **1b / GitLab target selected, untouched:** label "Docker image optional", help "OCI image for each Linux runner. Leave empty to use gitlab/gitlab-runner:ubuntu-v19.6.0.", field **empty** (GitHub default never shown), placeholder names the default.
- **1c/1d / typed value survives a target switch:** typed `registry.example.com/ci/gitlab-runner:1.2` while GitLab was selected, then removed the GitLab target — field became required again (no "optional" tag) and **kept the typed text** verbatim.
- **2a/2b / settings form, GitLab pool:** same optional label/help/placeholder; saved with the field left empty.
- **2c / settings form, GitHub pool:** unchanged — required, no "optional" tag, prefilled, no help-text change.
- **2d / Fleet execution, GitLab pool, empty image:** the pool's single Docker platform's image field shows the **raw** reference `gitlab/gitlab-runner:ubuntu-v19.6.0` (never "Default (…)", never `ghcr.io/actions/actions-runner:latest`).
- **2e / Fleet execution, GitHub pool:** platform image unchanged at `ghcr.io/actions/actions-runner:latest`.
- **3a/3c / read-only settings + pool detail, GitLab pool, empty image:** both show "Default (gitlab/gitlab-runner:ubuntu-v19.6.0)" (title attribute carries the same string on the detail page).
- **3b/3d / any pool with an image set:** shows the literal stored value, never "Default (…)" (`registry.example.com/ci/gitlab-runner:1.2` on `pool-gl-set`).
- **No runner-group control** exists anywhere on the GitLab pool's new-pool or settings forms, as the wireframe requires.

## Run environment
GridOps has no e2e seed script and its only login is GitHub OAuth (same situation prior U rounds for #141/#145/#159 recorded), so this pass built and served the real Rust API against a throwaway SQLite DB, seeded directly, with a hand-minted session cookie (same technique as those prior rounds):

- **Build:** `cargo build -p gridops-api --bin gridops-api` (`CARGO_BUILD_JOBS=2`), binary at the shared cargo-target cache (`~/.cache/cargo-target/gridops/debug/gridops-api`, mtime after F's last commit); `dist/` (vite build) already current (`git diff --stat HEAD -- src crates` empty).
- **Serve:** the binary on `127.0.0.1:19302`, with `GRIDOPS_DATABASE_PATH=<worktree>/.u172-scratch/u172.sqlite` (inside the worktree, not the session scratchpad — the scratchpad's mount rejected SQLite's WAL/shm files with "disk I/O error" under this session's sandboxing), `GRIDOPS_BASE_URL=http://127.0.0.1:19302`, `GRIDOPS_API_BIND=127.0.0.1:19302`, `GRIDOPS_WEB_ROOT=<worktree>/dist`, random `GRIDOPS_SESSION_SECRET` / `GRIDOPS_ENCRYPTION_KEY` (base64, 32 bytes), `GRIDOPS_FLEET_ALLOW_LOOPBACK_HTTP=true`, dummy `GITHUB_CLIENT_ID`/`GITHUB_CLIENT_SECRET` (only required so `validate_api()` passes at startup; no GitHub API call was made).
- **DB:** migrated automatically on first connection (`sqlx::migrate!`), then seeded via a throwaway Node+sqlite3 script (session-scratch `seed.mjs`, modeled on `docs/handoffs/159/screenshots/repro/seed.mjs`'s session-signing approach) with:
  - one admin user + session;
  - one GitHub installation (`acme-labs`, organization) with 1 repository;
  - one GitLab connection (`Acme / web`, id a real UUID — `gitlabConnectionIds` is validated as `.uuid()` by the update schema, which a short test slug fails);
  - three runner pools: a GitHub repository pool with its stored image set (`pool-gh`), a GitLab pool with its Docker image stored empty (`pool-gl-empty`), and a GitLab pool with its Docker image stored set (`pool-gl-set`). Each pool's `docker_image` column (distinct from the legacy `image` column, added in migration `0017`) was explicitly seeded — omitting it defaults to the GitHub image and looks like a #172 bug until traced to the schema.
- **Login:** session cookie minted with the same HMAC-SHA256(`GRIDOPS_SESSION_SECRET` raw bytes)-over-token scheme `auth.rs::create_session` / `Vault::sign` use, set via `browser_evaluate` `document.cookie`, confirmed by a hard load of `/` rendering the authenticated shell.
- **Driver:** Playwright MCP, headless Chromium, direct `mcp__playwright__*` tool calls (not `u-drive.mjs` — that helper assumes an HTMX email/password form and HTMX-swap timing; GridOps is a React/TanStack-Router SPA with no HTMX, the same reasoning recorded by prior GridOps U rounds). Viewports: 1280×900 (desktop) and 360×800 (mobile).
- **Cleanup:** `pkill -f cargo-target/gridops/debug/gridops-api` (confirmed down via `curl` returning no response), throwaway SQLite DB and its WAL/shm files removed (`.u172-scratch/` deleted).
- **State safety:** no shared/account-wide setting was touched — this was a dedicated throwaway DB and admin user created for this pass only, not a shared operator account.

## State matrix walked

| # | Page | State | Reached via | Result |
|---|---|---|---|---|
| 1 | `/runner-pools/new` | No target yet (1a) | Hard load | PASS — Docker image required, no "optional" tag, prefilled `ghcr.io/actions/actions-runner:latest` |
| 2 | `/runner-pools/new` | GitLab target selected, untouched (1b) | SPA (open Targets, select "Acme / web") | PASS — label "Docker image optional", help names the default, field **empty**, placeholder names the default; body still carries `runnerGroupId: 1` per F's note (not independently inspectable from the UI, confirmed by the create round-trip below storing `runnerGroupId: 1`) |
| 3 | `/runner-pools/new` | GitLab selected, user types an image (1c) | SPA (type into the field) | PASS — typed text held verbatim |
| 4 | `/runner-pools/new` | Switch GitLab → no target, typed image untouched (1d) | SPA (remove the GitLab target chip) | PASS — field is required again (no "optional" tag), **typed value survived** the switch, exactly as the wireframe requires |
| 5 | `/runner-pools/new` → create | GitLab pool, empty image, submit | SPA (fill pool name, submit) | PASS — 201, navigated to the new pool's settings page; API confirms `scope=gitlab`, `dockerImage=""`, `image=""`, `runnerGroupId=1` |
| 6 | `/runner-pools/pool-gl-empty/settings` | GitLab pool, stored image empty (2a) | SPA (sidebar → detail → Configuration tab) | PASS — same optional label/help/placeholder as new-pool form |
| 7 | `/runner-pools/pool-gl-empty/settings` → save | Save with image left empty | SPA (click "Save changes") | PASS — real round trip: `configurationVersion` 1→2, `dockerImage` stays `""`, navigated to the detail page on success |
| 8 | `/runner-pools/pool-gl-empty` | Detail, Image row after the save above (3c) | SPA (post-save navigation) | PASS — "Default (gitlab/gitlab-runner:ubuntu-v19.6.0)" |
| 9 | `/runner-pools/pool-gl-set` | Detail, Image row, image set (3d) | Hard load | PASS — literal `registry.example.com/ci/gitlab-runner:1.2`, never "Default (…)" |
| 10 | `/runner-pools/pool-gh/settings` | GitHub pool (2c) | Hard load | PASS — "Docker image" (no "optional" tag), help "OCI image for each Linux runner.", prefilled `ghcr.io/actions/actions-runner:latest`; Fleet execution platform image unchanged at the same value (2e) |
| 11 | `/runner-pools/pool-gl-empty/settings` | Fleet execution, GitLab pool, empty image (2d) | SPA | PASS — the pool's single Docker platform's image field shows the **raw** reference `gitlab/gitlab-runner:ubuntu-v19.6.0`, never "Default (…)" and never the GitHub image |
| 12 | `/runner-pools` list | All 3 pools, Overview widget | Hard load + SPA | PASS — GitLab pools show "GitLab" scope, GitHub pool unaffected |
| 13 | 360px mobile | `/runner-pools/pool-gl-empty` detail | resize | PASS — no horizontal overflow (`scrollWidth === clientWidth === 360`), "Default (…)" text still renders correctly |
| 14 | 360px mobile | `/runner-pools/pool-gl-empty/settings` | resize | PASS — settings rows stack vertically (`flex-col sm:flex-row`), no overflow |

**Zero/empty state:** walked and mandatory-covered (rows 2, 5, 6, 7, 8, 11 — the empty-image GitLab pool, both as stored data and through a live create+save round trip). **Error state:** covered structurally by T's Rust/vitest tests (padded image still 400, GitHub empty image/runner-group still 400); not independently re-driven in the browser since the brief marks this change as single-field, not list-based, and the error Callout rendering is unchanged UI (`Could not create the pool: …`).

**Console / state assertions:** no console error on the #172 surface itself at any checkpoint (list, both GitLab pools, GitHub pool, new-pool form, both viewports) beyond the two pre-existing, out-of-scope issues below, which reproduce identically on the GitHub pool's own pages (proving they predate #172 and are not caused by it):
- Headless Chromium in this session rejects the pre-existing `pool-name` input's `pattern="[a-z0-9][a-z0-9-]*[a-z0-9]"` attribute (`Invalid regular expression: /.../v: Invalid character class`), which silently blocks the native form submission on **both** the GitLab and the GitHub settings pages. Worked around in-session (`removeAttribute('pattern')`) to reach the real save path for verification; this is a browser/toolchain quirk, not a #172 defect, and F should not touch it.
- `GET /api/v1/runner-pools/repositories` and `GET /api/v1/installations/:id/runner-groups` return 500 ("invalid encrypted value") on **both** the GitLab and the GitHub new-pool/settings pages, because this seed's GitHub installation carries no real encrypted OAuth access token (same limitation prior GridOps U rounds recorded). Unrelated to the Docker-image/runner-group feature under test.

No Alpine/HTMX concept applies to this stack (React/TanStack Router SPA, confirmed by prior GridOps U rounds); "no dead component" was verified by reading the live DOM/API state directly rather than a framework introspection API.

## Findings
None blocking #172.

1. All wireframe frames (1a-1d, 2a-2e, 3a-3d) were reproduced live, including two real network round-trips (a settings save and a pool create), not just static reads of seeded data — this catches the class of bug a read-only walkthrough would miss (e.g. a schema that *looks* right in source but 400s on submit).
2. The GitHub pool's Docker image row, its Fleet execution platform default, and the runner-group behavior are byte-for-byte unchanged, confirmed live.
3. No regression on SPA nav vs hard load for any of the pages walked.
4. Two pre-existing, out-of-scope issues (noted above and in `decisions.md`) — a headless-Chromium `pattern`-attribute quirk and a seed-fidelity 500 from the installation's OAuth token — are advisory only; both reproduce identically on the GitHub pool's unmodified pages, so they are not #172 regressions.
5. Two of my own seeding mistakes were caught and fixed in-session before being misread as product defects: the `docker_image` column needs explicit seeding (distinct from the legacy `image` column), and `gitlabConnectionIds` must be real UUIDs per the update schema's `.uuid()` validation.

## Evidence
- Screenshots: `docs/handoffs/172/screenshots/u-list-1280.png` (pool list, 1280px), `docs/handoffs/172/screenshots/u-detail-gl-empty-360.png` (GitLab pool detail, 360px, no overflow).
- Live API cross-checks (`curl` against the live server, same seeded DB):
  - `GET /api/v1/runner-pools/pool-gl-empty` before/after the settings save: `configurationVersion` 1→2, `dockerImage`/`image` stay `""`.
  - `GET /api/v1/runner-pools/<new-pool-id>` after the live create: `scope=gitlab`, `dockerImage=""`, `image=""`, `runnerGroupId=1`.
  - `GET /api/v1/runner-pools/pool-gl-set`: `dockerImage="registry.example.com/ci/gitlab-runner:1.2"`.
- Live DOM captures (via `browser_evaluate`/snapshot) of every Image/Docker-image row's text, help copy, placeholder, "optional" tag presence, and the Fleet execution platform's image value — see the state-matrix table above for the exact values captured.

## Verdict
**PASS** — every state the brief's Done-when and the approved wireframe specify (new-pool form optional/required/typed-survives-switch, settings save round trip, read-only/detail "Default (…)" display, Fleet execution raw-image fallback, GitHub pool unchanged) was independently reproduced in a real headless browser, including two live network round-trips (not just reads), on both the SPA-navigation and hard-load paths, at desktop (1280px) and mobile (360px). No behavioral defect, no wireframe divergence. Two pre-existing, out-of-scope issues noted for the record (advisory only, not #172 regressions). Ready for S.
