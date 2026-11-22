## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** Claim that `vitest.config.ts` only includes `tests/**/*.test.ts` and therefore frontend contract tests must live under `tests/` (D-2) — the actual `vitest.config.ts` include pattern and any `tsx` handling would settle it. If the include glob is `.test.ts` only, a `.contract.test.tsx` (JSX) file under `tests/` would not be collected; the brief asserts the pattern but shows no excerpt.
- **[NV-2]** Claim that the `INSERT INTO workflow_jobs` statements in `crates/gridops-core/src/autoscaling.rs` are all `#[cfg(test)]` and not production insert sites (D-1) — showing `autoscaling.rs` around each INSERT would settle it. If wrong, #197's chosen writers miss a production write path.
- **[NV-3]** Claim that `gitlab_jobs` in `migrations/0047_gitlab.sql` has no `started_at`/`completed_at` and is not STRICT — the text of `0047_gitlab.sql` would settle it; D-1's migration design rests on it.
- **[NV-4]** Claim that `webhooks/gitlab.rs` "already reads" `build_name` and `project.path_with_namespace` — the relevant lines of `crates/gridops-api/src/webhooks/gitlab.rs` would settle it.
- **[NV-5]** Claim that axum "matches the specific route first regardless of order" for the `/api/v1/fleet/{*path}` catch-all in `fleet/mod.rs` — the actual `router()` chain plus axum's documented routing precedence would settle it.
- **[NV-6]** Adopted findings "W-6 to W-13 are adopted as written in `docs/handoffs/197/handoff-A.md`" — several decisions (source trait shape, `run_pass` placement, sentinels, nav, guard test runner) are defined only by reference to a document not reproduced in this prompt; the handoff text would settle their exact content.
- **[NV-7]** Claim that `require_admin` exists at `fleet/profiles.rs:330-349` following the `FleetUser<Read>` pattern — that excerpt would settle it.
- **[NV-8]** The "under a minute" Mars-copy migration criterion is only measurable against a live 2.9 GB copy; the criterion is testable but its feasibility is unverifiable here (last data point: 93 s `VACUUM INTO` at 1.5 GB).
- **[NV-9]** The vitest include claim also underpins D-2's "existing tests use createElement … no testing-library" style claim — an excerpt of an existing frontend test would settle the mandated style.

### WARN findings
- **[W-1]** The brief never states that migration 0050 adds a `category` column to `workflow_jobs`. D-1 has #197 adding the `category` bind to the three `workflow_jobs` writers and #191/#190 tests reading categories, but the migration section of D-1 lists only `gitlab_jobs` columns. State explicitly that 0050 adds `workflow_jobs.category` (nullable) — otherwise the wiring D-1 mandates cannot compile/behave. — Recommendation: add one sentence to D-1's migration list.
- **[W-2]** D-3(c) has #197 edit `src/routes/_app/index.tsx`, while the "Why this story is shaped this way" section forbids wave-1 edits to "…`src/lib/navigation.ts` or `page.tsx`'s composition". Whether `index.tsx` counts as a `page.tsx` is ambiguous and the two sentences can be read as disagreeing. — Recommendation: name the exempted files exactly (e.g. "`src/routes/_app/index.tsx` composition is #197-only").
- **[W-3]** "The page and the Overview tile hide on 403" (D-4) is a behavior with no test named in any "Done when" list (#195's list doesn't include it; #194's covers only the 403 response). — Recommendation: either add it to #195's contract tests or explicitly mark it *(not a test)*.
- **[W-4]** Guard semantics for #197's own edits are unspecified: #197 edits many files no wave-1 issue owns (`main.rs`, `navigation.ts`, CI, migration), yet the guard "fails on a test PR that edits a file outside an issue's list". Presumably #197's own changes are exempt, but neither the criterion nor the owners-file spec says how. — Recommendation: state the guard's scope (wave-1 ownership only; #197 exempt or self-owned).
- **[W-5]** Tension between #197's body ("one [contract test file] per wave-1 issue") and adopted W-13 ("one contract-test file per crate for #190", which spans core/reconciler/api crates). The decision overrides, but the copied acceptance criterion and the decision disagree on its face. — Recommendation: note the override in the plan, as done for the migration number.
- **[W-6]** The two guard-check throwaway PRs are only ever run against throwaway branches; nothing in CI proves the guard behaves this way on a real wave-1 PR (that's deferred to wave 1 by design). Acceptable, but the acceptance criterion is discharged by a one-time manual demonstration that leaves no permanent artifact beyond the PR description. — Recommendation: record head SHAs as specified and move on.
- **[W-7]** The #195/#199/#202/#203 "*(not a test)*" Playwright items (screenshots, console errors, horizontal scroll) are listed under the wave-1 "Done when" lists this story turns into contract tests, but the brief doesn't say where the non-test items live for this PR — #197 itself has Playwright items only implicitly. — Recommendation: state explicitly that all *(not a test)* items stay in the wave-1 PRs and out of this story's Done-when list.

### NIT findings
- **[NIT-1]** "The Mars database timing check" recipe "has not been dry-run for this story" is candid, but the expected wall time for `VACUUM INTO` at 2.9 GB is extrapolated from one 2026-11-15 data point; consider noting the extrapolation risk in the PR.
- **[NIT-2]** The brief says re-check the migration number "(`0050` today)" but D-1 already hard-codes "Migration 0050"; if the re-check yields a different number, D-1's prose goes stale. Reference "the new migration" in D-1 instead.
- **[NIT-3]** Acceptance criterion "every file in its 'Owns' list exists" is a grep check discharged by a guard-script test — good — but the criterion as copied says "grep check" while the mechanism is a node test reading the owners JSON; the wording should say the automated test is the check.

### Verdict
PASS — no BLOCK findings; implementation may proceed.