## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** The claim that the shared label cap is 64 characters at `schemas.ts:13` and mirrored in the server's `models.rs` — an excerpt of `src/features/runner-pools/schemas.ts` around line 13 and of `crates/gridops-core/src/models.rs`'s label validation would settle it; the whole tag-validation criterion hangs on it.
- **[NV-2]** The claim that `github.tsx`'s `initialSelection` (:244-255), `initialPriorityKeys` (:256), `readOnlySummary` (:257) and `toUpdatePayload` (:225-243) branch on "not repository" rather than on explicit scopes — shown source for those four functions would confirm gap 4 exists as described.
- **[NV-3]** The claim that `createRunnerPoolAction`/`updateRunnerPoolAction` send zod-parsed input and that `z.object` strips unknown keys (`runner-pools.functions.ts:127-138`) — shown code for those lines and the schema parse would settle whether the wire-field work in `schemas.ts` is actually required.
- **[NV-4]** The claim that `webhooks.tsx` (~:119, :146) currently labels delivery rows with no repository/installation as "GitHub App" — shown excerpt of `src/routes/_app/webhooks.tsx` at those lines.
- **[NV-5]** The claim that `#138`'s create handler calls `assert_installation_admin` for non-repository scopes and reads the first installation from `context.options.installations` — shown excerpt from `crates/gridops-core` (branch `claude/gitlab-138-api-44d3a8`) covering the create handler and `validate_gitlab_connections`.
- **[NV-6]** The claim that `buildCreatePayload` merges with `Object.assign` in registry order and that GitHub's create partial always sets `scope`, `installationId`, `repositoryIds`, `runnerGroupId` — shown excerpt of `registry.tsx`'s `buildCreatePayload` and `github.tsx`'s `toCreatePayload`/partial would settle the ordering constraint (gap 6).
- **[NV-7]** The claim that the OpenAPI contract (#137 output in `docs/openapi/gridops-v1.json`) matches the listed endpoints, status codes (201/204/409, PATCH semantics) and `ApiErrorBody` error substrings — the spec file itself.
- **[NV-8]** The claim that `SecretRevealDialog` already supports copy buttons (gap 3 hinges on whether `details` suffices) — shown excerpt of the dialog component.

### WARN findings
- **[W-1]** The criterion "if none exists the GitLab kinds are still offered but submit fails with the server's/validator's message — record what the UI shows" is weakly testable and unbounded: it asks the implementer to observe rather than assert anything. Convert it to a concrete expectation (e.g. "the form shows the server's error text and submit is retriable") or drop the observation clause.
- **[W-2]** Frontend compatibility when the server has #137 but not #138 is addressed only for pool fields; nothing covers the case where the connection API itself is absent (GET …/gitlab 404s or the platform registry entry fails to load connections). The walkthrough mocks the spec, so this path goes untested. State explicitly that the missing-backend-connection-API path is out of scope, or add a mocked error-state check.
- **[W-3]** "Done when" (original) and the amended Done-when overlap confusingly with the Scope section's "no pool-form rows" while the wire-field amendment adds `run_untagged` as a Platform-options toggle inside the pool form. The amendment is clear, but a reader of only the Scope section could construe the toggle as a new pool-form row. One sentence in Scope pointing at the amendment would prevent that.
- **[W-4]** Missing error path in the walkthrough: no case for `DELETE` returning 404, `PATCH` 400 (bad replacement token is covered only under "edit name and replace token" without an asserted failure state), or webhook-secret rotate failing. Add explicit mocked 400s for PATCH and rotate.

### NIT findings
- **[NIT-1]** The Scope section still says labels are "at most 255 characters" while the amended contract says the shared 64-char cap wins; the Contract header says it wins, but Scope could cross-reference to avoid a stale spec being implemented from.
- **[NIT-2]** Gap 1's action type is given inline as an example (`e.g. actions?: …`) rather than as a normative signature; pin the exact type in the decisions doc so the descriptor and `connections-section.tsx` implementations can't drift.

### Verdict
PASS — no BLOCK findings; implementation may proceed.