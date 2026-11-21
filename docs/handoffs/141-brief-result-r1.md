## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** The claim that the shared webhooks list currently labels repository-less rows "GitHub App" at `src/routes/_app/webhooks.tsx` ~:119 and ~:146 — showing those lines (or the current source of `webhooks.tsx`) settles it; if the labeling has changed, the generic-prefix-derivation fix needs re-scoping.
- **[NV-2]** The claim that `SecretRevealDialog`'s `details` field already supports copy buttons, deciding gap-fix 3's "use details" vs "extend the reveal type" branch — the component's source (`src/features/platform-connections/…` secret reveal dialog) would settle it.
- **[NV-3]** The claim that `ConnectionBindings` today exposes only Edit and Remove (gap 1's premise) — the current `types.ts` / `connections-section.tsx` would settle it.
- **[NV-4]** The claim that `platformOptions.Component` currently receives no props (gap 2's premise) — `platform-options-section.tsx` and the descriptor type in `types.ts` would settle it.
- **[NV-5]** The pool-wire claims taken from #138's in-progress branch (`create handler still calls assert_installation_admin`, `installationId` positive, partial takes first installation) — excerpts from `crates/gridops-core/src/models.rs`, `resources.rs` and the create handler on branch `claude/gitlab-138-api-44d3a8` would settle them; if the branch moves before this work merges, the contract section silently drifts.
- **[NV-6]** Dependencies "on master" (`e8b18bf` for #145, `b22c451` for #137) — git log on master would settle it.

### WARN findings
- **[W-1]** Internal tension on polling errors: the Scope section says the operations view "lists GitLab webhook deliveries and polling errors," while the Contract section says polling errors are out of reach (#148 logs only) and to "add nothing." The stated precedence rule resolves it, but the brief should delete the polling-errors sentence from Scope so a reader of Scope alone builds the wrong thing.
- **[W-2]** Walkthrough coverage gaps vs. the contract: (a) the pool-settings route (`runner-pools.$poolId.settings.tsx`) sending `gitlabConnectionIds` as a set-replacement is required by the Target-kinds section but never exercised in the walkthrough or listed in Vitest payload-partial tests beyond "update" abstractly; (b) the no-installation submit failure (contract says "record what the UI shows") is in neither Validation nor Walkthrough, so its acceptance is untestable. Add both to the walkthrough list.
- **[W-3]** "The only GitLab-specific frontend code is the descriptor and the API functions it calls" (Done when) contradicts the Rules section's carve-outs (tag rule in `schemas.ts`, test fixtures) and the shared-file allowlist which must stay gitlab-string-free. Reword Done when to match Rules, otherwise a literal reviewer fails the diff.
- **[W-4]** Label rule "any character but a comma, at most 255 characters" has no stated rule for empty tags or whitespace-only tags in the *descriptor* scope section, while Validation adds "non-empty after trim" only under Vitest. State one rule once; two phrasings invite divergent implementation.
- **[W-5]** `runnerGroupId: 1` is hard-coded in the contract create payload with no explanation of where `1` comes from or whether it's validated; if #138 changes it, #141's mocked walkthrough will pass while real submits fail. Mark it as mocked-spec value, not truth.

### NIT findings
- **[NIT-1]** "no leading/trailing slash, no `..`, no whitespace" for scope path is partly redundant with the `segment(/segment)*` shape — say which is authoritative for the zod rule.
- **[NIT-2]** The error-substring list ("later", "GitLab request failed", etc.) is asserted from #138's in-progress work; note that the Vitest mocks should key on the stable substrings, not full messages, to avoid brittleness.

### Verdict
PASS — no BLOCK findings; implementation may proceed.