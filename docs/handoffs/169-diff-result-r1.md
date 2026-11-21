## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1] crates/gridops-api/src/resources.rs:1330 — the brief required sweeping "the other `LEFT JOIN runners` counts in `resources.rs` for the same shape," and the handoff says it relied on "A's confirmed check" without a code change — the excerpt shows only lines 1325–1349, which contain the pool-list query, not the other counts (e.g. the ~342 `DISTINCT r.id` site and the dashboard counts). — The rest of `resources.rs` (the dashboard query and any other `COUNT(CASE WHEN r.deleted_at IS NULL …)` over `LEFT JOIN runners`) would settle whether any sibling count still counts the NULL row. (The visible siblings at :1331–1334 are safe in this prompt: each adds a comparison on `r.status`/`r.busy`/`r.configuration_version` that is false/NULL on the all-NULL row.)
**[NV-2] tests / UI — the brief's Done-when requires a headless UI walkthrough of `/runners`, a GitLab pool's detail page and `/` for GitHub and GitLab pools; the implementor explicitly did not run it (Tier 2/3, owned by T and U/S). — A recorded walkthrough run (or T's confirmation) would settle it. Not a block: the jsdom tests in the diff cover the same rendered output.
**[NV-3] crates/gridops-api — the handoff reports `cargo clippy -p gridops-api --all-targets -- -D warnings` failing on 5 pre-existing `unused-async-trait-impl` sites and one failing `fleet_http::release_preflight` test, both claimed pre-existing/environmental. The brief requires clippy `-D warnings` and `cargo test -p gridops-api` to pass. — CI output (or `git stash` + re-run on `main`) showing these failures exist without this diff would settle it. The shown excerpt `probe.rs:16-24` does confirm the `cfg!(target_os = "macos")` mechanism cited for the preflight failure.
**[NV-4] src/features/runners/runner-row.tsx — the handoff claims GitHub/Bitbucket/older-server rendered output is byte-identical to today's. The diff replaces the three inline expressions with a hook whose GitHub branch returns the same `repository ?? accountLogin`, which is consistent, but pixel-identical rendering is a runtime claim. — The headless walkthrough ([NV-2]) or a snapshot diff would settle it.

### WARN findings
**[W-1] tests/runner-row-platform.test.ts:73 — `olderServer` derives its fixture from `runner("github")`, whose shape lives above the shown diff hunk; the test self-checks (`"poolScope" in legacy` throws) so a bad fixture fails loudly, but the fixture itself is unverifiable from this prompt — keep that guard. — Recommendation: none needed beyond keeping the assertion.
**[W-2] src/features/platform-connections/github.tsx:323 — `scopeLabel: (pool) => pool.scope` returns the raw scope, which the dashboard's `capitalize` renders correctly for "repository"/"organization" but would render a hypothetical future GitHub scope (e.g. "runner-group") capitalized mechanically. — Recommendation: acceptable per the brief; note it if GitHub grows a multi-word scope.

### NIT findings
**[NIT-1] src/features/runners/runner-row.tsx:41 — `RunnerDetails` computes `destination.title` but never uses it (only `destination.text`); harmless, but the unused half is dead in that component.
**[NIT-2] tests/dashboard-pool-scope.test.ts:99 — `scopeText` selects the span by matching `/ · /` in `textContent`; brittle if a sibling span ever gains a "·". Acceptable for now.

### Verdict
PASS — no BLOCK findings; testing may proceed.