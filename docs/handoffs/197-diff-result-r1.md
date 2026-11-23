## Implementation Review (Round 1)

### BLOCK findings

**[B-1]** docs/handoffs/197/handoff-F.md (Tier 1 self-check) — The brief's acceptance criterion "Every wave-1 'Done when' test exists here, ignored or skipped with its issue's marker, and **fails against the stubs once its marker is removed. Prove it by removing all markers on a scratch branch**" is not met: the handoff reports only GREEN runs with markers in place, plus a list of test files that don't compile ("Tests that look wrong", items 1–2 stop core/api/reconciler test targets from compiling until T applies fixes). No scratch-branch run with markers removed is reported, so nothing in this prompt shows the contract tests actually fail against the stubs rather than merely being ignored. Why it blocks: this is a named, purely local test the brief demands, and its absence means the core promise of the PR (wave-1 tests fail before their issues land) is unproven — worse, items 1–2 show several unmarked test targets currently fail to compile at all. Remediation: on a scratch branch, remove every `#[ignore = "contract: #N"]` / `it.skip("[contract: #N] …")` marker and run the workspace test suite (applying the test-file fixes T already has), recording which tests fail and confirming every unmarked contract test fails (and no unrelated test breaks); report the run in the handoff.

**[B-2]** src/features/fleet-health/page.tsx / tests/ — The acceptance criterion "`/fleet/health` renders it with every placeholder card" has no test. The diff adds no test that renders `FleetHealthPage` (or the `/fleet/health` route) against the fixture; `tests/fleet-health-shared.test.ts` tests the schema, fetch, theme and tiles only, and the #195 contract tests are skipped and test the section stubs individually, not the page. Why it blocks: a brief-mandated test of a shipped behavior is missing, so a regression that breaks the page frame (e.g. the window picker, the 403 "Administrators only" state, section composition) would pass CI. Remediation: add a #197-owned Vitest test that mocks `fetch` to return the fixture, renders `FleetHealthPage`, and asserts every placeholder section ("Coming in #195/#202/#203") appears with the fixture's data loaded (plus the administrators-only empty state on 403).

### Needs-verification findings

**[NV-1]** crates/gridops-api/src/fleet/health.rs:89 — the claim that `RequestContext` is `Copy` (so `context.error(…)` after `require_admin(&state, &context, …)` compiles) — the shown excerpt of `fleet/error.rs:29-35` shows `error(self, …)` by value but not the `Copy` derive on `RequestContext`; `crates/gridops-api/src/fleet/error.rs`'s struct definition would settle it (if it compiles in CI, this is moot).

**[NV-2]** src/routes/_app/index.tsx:123 — `data.authenticated` is read off `DashboardOverview` to gate the health hook — `src/features/dashboard/types.ts`'s `DashboardOverview` definition would settle whether that field exists.

**[NV-3]** src/lib/navigation.ts — the claim that shortcut `t` is unique — the existing `navigation.test.ts` uniqueness test passing (per the handoff) supports it; the full `navigation.ts`/test run in CI settles it.

**[NV-4]** crates/gridops-core/src/fleet_health/mod.rs (`Severity` ordering) — "worst first: `Critical < Warning < Info`" via derive is asserted by tests in the diff; a green `fleet_health_contract.rs` run settles it.

**[NV-5]** crates/gridops-api/src/fleet/health_test_support.rs — `gridops_core::crypto::hash_token` and the `users`/`sessions` insert column lists match the migrated schema — the schema in `migrations/` and `crates/gridops-core/src/crypto.rs` would settle it.

### WARN findings

**[W-1]** docs/handoffs/197/handoff-F.md (Known issues) — The two throwaway guard test PRs against `Performant-Labs/gridops` ("Fleet health guard fails on a PR editing a file outside an issue's list, and on one editing a contract test, then both closed") are deferred to O with only the guard's 13 unit tests as cover. Recommendation: they are a brief acceptance item and must be opened, observed failing, closed and branch-deleted (from the final head after the last guard/owners/contract-test change) before merge; record the head SHAs.

**[W-2]** docs/handoffs/197/handoff-F.md (Known issues) — The Mars-database timing check ("migration applies to a 2.9 GB copy in under a minute", with load recorded and copies cleaned up even on failure) is deferred to O. Recommendation: it must be run before merge per the brief, including the failure-path cleanup of remote and local copies.

**[W-3]** crates/gridops-core/src/fleet_health/speed_check.rs — the check only times a write transaction from its first row change, so a lock held by `BEGIN IMMEDIATE` before any write, or writes to `WITHOUT ROWID` tables, escape the 50 ms limit (the module doc admits this). Recommendation: acceptable for #197, but record the limitation in the design doc's testing section explicitly as a non-goal so wave-1 rollup authors don't rely on it catching BEGIN-held locks.

**[W-4]** crates/gridops-api/src/fleet/health.rs — until #194 lands, `?window=` is silently ignored (any value returns the 24 h fixture) while the OpenAPI document already promises a 400 for bad windows. Recommendation: fine as staged, but the wiring test should note that a bad window currently returns 200 so nobody mistakes the published 400 for live behavior.

**[W-5]** crates/gridops-core/src/fleet_health/fake.rs — `FakeFleetHealthSource` is compiled in production (deliberately, per the design doc, until #204). Recommendation: when #204 lands, move it behind `test-support` as the design doc already promises; add a reminder in #204's owners entry or issue comment.

### NIT findings

**[NIT-1]** docs/design/fleet-health.md (Job categories) — the table's patterns contain escaped pipes (`test\|spec\|…`) which is correct Markdown but easy to mis-copy; the migration and `default_rules()` use the unescaped form.

**[NIT-2]** crates/gridops-api/src/resources.rs — `#[cfg_attr(not(test), allow(unused_imports))]` on the re-export will become a permanent fixture once #201 uses it; consider dropping the attribute when #201 lands.

**[NIT-3]** src/features/fleet-health/api.ts — `useFleetHealth`'s error branch keeps stale `data` from a previous window only via the spread of `previous` when `previous.window === window`; a comment making the intended "keep last good data per window" semantics explicit would help #195/#203.

### Verdict

BLOCK — 2 blocking finding(s); must resolve before testing starts.