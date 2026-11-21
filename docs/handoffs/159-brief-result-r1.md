## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** The claim that the list endpoint `runner_pools` currently returns no GitLab fields — excerpt of `crates/gridops-api/src/resources.rs` around line 1292 showing the list item's serialized fields.
- **[NV-2]** The claim that `pool_gitlab_connections` / `pool_connection_json` in `crates/gridops-api/src/resources/gitlab.rs` already return `{ id, name, baseUrl, scopeKind, scopePath }` and that the detail orders them by `membership.created_at, connection.id` — the relevant file:line excerpts.
- **[NV-3]** The claim that `readOnlySummary` takes a `RunnerPoolDetail` while the list rows only have `RunnerPool` (`src/features/operations/operations.functions.ts`) — type signatures at those lines.
- **[NV-4]** The claim that a `scope: "gitlab"` pool still stores an installation id (from #138) so both views currently mis-render it as a GitHub account — the #138 requirement text or the pool schema/model excerpt showing installation id is required on all scopes.
- **[NV-5]** The existence and current coverage of the "GitLab code has one home (#141)" test (`tests/platform-neutral-source.test.ts`), which the brief says must keep passing — the test file excerpt.
- **[NV-6]** The claim that a `platformForDeliveryId`-style registry function exists in `src/features/platform-connections/` as the pattern to follow — the registry source excerpt.

### WARN findings
- **[W-1]** The unknown-scope case is specified for rendering ("safe fallback, never throws, no GitHub avatar") but the Done-when unit-test list only asserts "the registry resolves the owning platform as described" for it — it does not require testing the fallback list cell or fallback detail rendering. Add unknown-scope rendering assertions to the vitest criterion.
- **[W-2]** "Bitbucket owns no scope (its pools are GitHub-scoped)" couples Bitbucket rendering permanently to the GitHub descriptor. If Bitbucket later gains its own pool scope, the registry mapping must change; the brief should note the `ownsPool` mapping is owned by the descriptors, not hardcoded in the routes/registry (it mostly implies this, but the Bitbucket-descriptor-has-no-scope rule should live in the descriptor test).
- **[W-3]** The platform-neutral source test forbids "a scope comparison" in the routes, which is not mechanically greppable the way a string like `pool.scope` is. Specify the exact forbidden tokens so the test is deterministic (e.g. `.scope`, `scope ===`), or the criterion is untestable as written.
- **[W-4]** Missing edge case: a GitLab pool where `gitlabConnections` is present but a connection lacks `baseUrl` or `scopePath` (older/partial server data) — the detail row's `baseUrl/scopePath` formatting should define behavior for absent parts.

### NIT findings
- **[NIT-1]** "names are the implementer's/architect's call" combined with "this is the intended shape" is slightly self-contradictory; fine, but the Done-when tests should reference the hooks by behavior, not name (they currently do — good).
- **[NIT-2]** The list-cell GitLab text spec (`Acme / web`, `2 connections` with full names in title) doesn't say what happens with exactly one connection that has an empty name.

### Verdict
PASS — no BLOCK findings; implementation may proceed.