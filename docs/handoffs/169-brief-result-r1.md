## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** The claim that the existing `LEFT JOIN gitlab_connections gc` is already present in the runners list query (~lines 1925–2010 of `crates/gridops-api/src/resources.rs`), so `gitlabConnectionName` needs no new join — an excerpt of that handler's SQL showing the join and the selected columns would settle it.
**[NV-2]** The claim that `totalRunners` uses `COUNT(CASE WHEN r.deleted_at IS NULL THEN 1 END)` over `LEFT JOIN runners r` (~line 1329 of `resources.rs`) and therefore counts the NULL row for empty pools — the actual line 1329 SQL text would settle it.
**[NV-3]** The claim that the other `LEFT JOIN runners` counts (`online`/`busy`, dashboard counts, `DISTINCT r.id` at ~line 342) already handle NULL rows correctly — the SQL at those lines would settle it; the brief itself hedges ("Confirm, don't assume"), which is appropriate but leaves it open.
**[NV-4]** The claim that `runner.repository ?? runner.accountLogin` appears in three places in `src/features/runners/runner-row.tsx` (~lines 46, 109, 127) — shown file lines would settle it.
**[NV-5]** The claim that #159 merged as `ea3ec47` and added `platformForPool(pool: { scope })` in `src/features/platform-connections/registry.tsx` plus the optional `pool?: PoolBindings` descriptor key — excerpts from `registry.tsx` and `types.ts` would settle it. Also NV-5a: the claim that the unknown-scope fallback already exists and "never echoes the raw scope" — the fallback's code in `registry.tsx` would settle it.
**[NV-6]** The claim that the dashboard renders `{pool.scope} · {pool.mode}` at ~line 279 of `src/routes/_app/index.tsx` — the shown line would settle it.
**[NV-7]** The claim that `Runner` type lives in `src/features/operations/operations.functions.ts` — an excerpt of that file would settle it.
**[NV-8]** The claim that `CARGO_BUILD_JOBS=2` / `RUST_TEST_THREADS=2` are set in `.claude/settings.local.json` — that file's contents would settle it.

### WARN findings
**[W-1]** Fallback ambiguity for missing `poolScope`: the brief offers two implementations ("resolve as GitHub's `repository` scope, or keep today's text, chosen so GitHub/Bitbucket output is unchanged") but leaves the choice open. These differ observably: a GitHub *organization*-pool runner without `poolScope` shows `accountLogin` today but would show "Unknown" if the unknown fallback applied. The brief's constraint that GitHub output is "exactly today's" forces the repository-scope resolution for legacy GitHub runners, which conflicts with the general rule that unknown scope never echoes raw scope — the brief should pin down: missing `poolScope` → resolve as `repository` scope. Recommend resolving this before implementation.
**[W-2]** GitLab destination fallback chain ends in the literal "GitLab", which for the right-hand column title may be indistinguishable from a real connection named "GitLab"; acceptable, but consider a distinct title (e.g. "GitLab runner") for the no-data case.
**[W-3]** The "headless UI walkthrough" acceptance criterion has a GitLab pool with runners implied but no stated fixture setup (a GitLab pool with a runner requires seed data or a live GitLab connection). The brief should specify how the walkthrough obtains GitLab data, or the criterion is not reliably executable.

### NIT findings
**[NIT-1]** Vitest matrix omits a GitLab runner whose connection row was deleted (`gitlabConnectionName` null but scope gitlab with base URL present) as a distinct case — it's arguably covered by "with only an address", but the API-returning-null-due-to-gone-connection path differs from runner-having-no-connection; worth one extra case.
**[NIT-2]** Done-when says dashboard test shows "repository"/"organization" "unchanged" lowercase — fine, but note the `capitalize` CSS means the assertion should be on the raw text, not rendered text; the criterion doesn't say which.

### Verdict
PASS — no BLOCK findings; implementation may proceed.