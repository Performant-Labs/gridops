# Handoff-T-green: Phase 7 - Pool list and pool detail take the destination from the platform registry (#159)

**Date:** 2026-11-21
**Branch:** claude/gitlab-159-pool-destination
**Issue:** #159
**Handoff-F reviewed:** docs/handoffs/159/handoff-F.md
**Handoff-T-red:** docs/handoffs/159/handoff-T-red.md

## GREEN confirmation
- `npx vitest run`: Test Files 52 passed (52), Tests 582 passed (582). RED baseline was 26 failed / 556 passed; all 26 new tests are now GREEN. Same result as F reported.
- `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api`: unit 123 passed, control_plane_contracts 36 passed, fleet_http 143 passed / 1 failed. The Rust test `resources::gitlab_tests::the_pool_list_carries_each_pools_gitlab_connections_in_detail_order_and_none_for_a_github_pool` passes.
- The one failure is `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`. It matches F's report: the fixture agent reports the compile OS (`native.rs:201`), so on this arm64 Mac the "Linux" machine counts as macOS ARM64. `scripts/release-preflight.sh` has 0 references to `runner-pools`, and the diff touches nothing it exercises. Environment-dependent and pre-existing; not a #159 defect.
- Spot-check that tests pin behavior: the Phase-4 RED run already proved each new test fails with the feature absent (26 failures on the feature's own assertions, none on import/setup). The tests render the real routes with the production registry, so they fail if a route stops calling the hooks or a descriptor changes its output.

## Tier 1 results
- `npm run lint` (eslint, --max-warnings=0): clean. PASS
- `npm run build` (vite build + tsc --noEmit): built, typecheck exit 0. PASS
- `cargo fmt --check -p gridops-api`: exit 0. PASS
- `cargo clippy -p gridops-api --all-targets -- -D warnings`: only pre-existing findings (host-agent helper.rs/native.rs, api fleet/boundary.rs, fleet/enrollment.rs, resources/route_security.rs); none in resources.rs, resources/gitlab.rs or gitlab_tests.rs. PASS for #159's files.
- Full existing suites still pass (above). PASS

## Tier 2 results
- Coverage per criterion: every Done-when item has a test (below). PASS
- Test quality: each test names one behavior; the 8 GitHub/Bitbucket guards are deliberate invariants (green before and after); the Rust test is the cheapest tier for SQL ordering/grouping; no redundant tests found. Suite is proportionate (30 jsdom + 4 source-guard + 1 Rust). PASS
- Type safety: tsc clean; `PoolBindings` makes the two renderers mandatory together; `platformForPool` returns a type-guarded `PoolPlatform`, so routes use no `?.` and no `any`. PASS
- Error handling / fallback: unknown scope returns the registry-owned fallback and never throws or echoes the raw scope; absent or empty `gitlabConnections` shows "GitLab" / "No connections listed". PASS
- Data integrity (API): one `json_each(?)` query over the page's pool ids, ordered `pool_id, created_at, connection.id`; an empty page returns early; a pool with none gets `[]`; the detail shares the same SELECT/ORDER constants, so output is unchanged. PASS
- API contract: items gain `gitlabConnections: [{id,name,baseUrl,scopeKind,scopePath}]`; OpenAPI 200 schema is a bare object, so no spec change. `RunnerPool.gitlabConnections?` is optional. PASS
- Security: no new inputs; ids are bound as a JSON parameter, and the dynamic SQL is built only from two constants. PASS
- Migration: none. PASS
- Neutral-source guard: both routes contain no platform id, `pool.scope`, `githubAvatar` or platform-named field, and import the registry; the "GitLab code has one home" tests pass. PASS
- Playwright: the repo has no Playwright run in scope for this change; not run.
- Evidence appendix: F's evidence.md already covers the unchanged-code facts the tests rely on (`pool_connection_json` shape, GROUP BY p.id, `PropertyRow` dt/dd, `unknownPlatform`, `native.rs` OS). Re-read against source; nothing to add.

## Acceptance criteria status
- Registry resolves owning platform (GitHub repo/org, Bitbucket to GitHub, GitLab, unknown fallback): PASS (pool-destination.test.ts, Registry group, 10 tests).
- List destination: GitHub/Bitbucket unchanged with the GitHub avatar; GitLab connection names with the GitLab icon and no avatar; GitLab empty/absent shows "GitLab": PASS (List groups).
- Detail destination: GitHub/Bitbucket rows unchanged (including "Show N more"); GitLab Scope "GitLab" + connections, no Account or Runner group: PASS (Detail groups).
- Neutral-source test covers both routes: PASS (platform-neutral-source.test.ts).
- Rust pool list carries `gitlabConnections` in detail order, `[]` for GitHub: PASS (gitlab_tests.rs). The "single query" is structural by inspection of `pools_gitlab_connections`, as A agreed.
- lint, typecheck/build, vitest, cargo fmt, cargo test: PASS, with the one pre-existing environment-dependent fleet_http failure above. cargo clippy has pre-existing findings only.
- Headless UI walkthrough of `/runner-pools` and a GitHub and a GitLab pool's detail: not a T check; handed to U.

## Blocking issues
None.

## Advisory notes
- The jsdom route tests cover rendering, but the live SPA path (Alpine-free React hydration, real navigation) is U's walkthrough: please walk `/runner-pools` and one GitHub and one GitLab pool detail.
- Pre-existing: `release_preflight` fixture is host-OS dependent; `runner_pools` reports `totalRunners: 1` for a pool with no runners (LEFT JOIN count). Neither is in scope for #159.
