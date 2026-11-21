## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** src/components/ui/searchable-select.tsx (used in connections-section.tsx select branch) — the claim that `SearchableSelect` accepts `value: string | null`, `onValueChange: (string | null) => void`, `ariaLabel`, `options: {value,label}[]` and `searchable?: boolean` and renders a `button[aria-haspopup="listbox"]` trigger — settle with the component's source in `src/components/ui/searchable-select.tsx`.
- **[NV-2]** src/components/settings-ui (Switch in gitlab.tsx) — the claim that `Switch` supports `aria-labelledby`, `checked`, `onCheckedChange` and renders `[role=switch]` — settle with the `Switch` component source.
- **[NV-3]** node_modules/lucide-react — the handoff asserts the icon `Gitlab` exists in the installed 0.552.0 — settle with the package's export list (`node_modules/lucide-react/dist/lucide-react.d.ts` or the package's icons index).
- **[NV-4]** crates/gridops-api (server create handler) — the claim that a GitLab create with `installationId: 0` (no installation) fails with a useful message rather than something confusing, and that a positive `installationId` is still required — settle with the create-pool handler and `assert_installation_admin` in `crates/gridops-api/src/resources.rs` / `crates/gridops-core`.
- **[NV-5]** tests/fixtures/pool-forms.ts (`createOptions`, `settingsPool`, helpers `pick`, `switchNamed` support fns) — the new tests depend on those fixtures returning installations with id 1 and the helper names claimed — settle with the full `tests/fixtures/pool-forms.ts` (only one hunk shown).
- **[NV-6]** src/routes/_app/webhooks.tsx ~:82-83 — the brief claims existing copy "Verified GitHub App deliveries…" / "GitHub settings" remains; the diff does not show those lines, and the new neutral-source assertions (`not.toMatch(/["']GitHub App["']/)`) must not accidentally ban that copy — settle by reading the untouched region of `webhooks.tsx` alongside the new test.
- **[NV-7]** tests/platform-neutral-source.test.ts (`grep` helper) — the "GitLab code has one home" test relies on `grep([/gitlab/i])` scoping to `src/` only (otherwise the new test files/fixtures themselves would trip it) — settle with the `grep` helper's root definition in that test file.
- **[NV-8]** #140's delivery id scheme — the contract states GitLab delivery ids are `gitlab:<connection_id>:<key>`; if #140 (still in progress) lands a different prefix, `platformForDeliveryId` mislabels GitLab deliveries as GitHub — settle with #140's insertion code for `webhook_deliveries`.

### WARN findings
- **[W-1]** tests/connections-page.test.ts:84 / handoff "Tier 1 self-check" — the handoff reports `2 failed | 546 passed` and frames the GitLab stub fix as something "for T" to apply, yet the diff already applies exactly that fix — the reported run therefore does not reflect the code under review — re-run the full Vitest suite on this tree and record the actual result before the handoff is accepted.
- **[W-2]** src/features/platform-connections/connections-section.tsx (FieldInput select branch) — gap 5 of the contract says `validate` problems are rendered "with `aria-invalid` and a `text-danger` line"; the select branch renders the error line but no `aria-invalid`/`aria-describedby` on the trigger — harmless today (the GitLab select has no `validate`) but the contract's accessibility behavior is only half implemented for selects — add `aria-invalid` to the SearchableSelect branch.
- **[W-3]** src/features/platform-connections/gitlab.tsx (toUpdatePayload) — `runUntagged` is sent only when it differs from the stored value; the contract says update takes `runUntagged?` (optional) without the change-condition, and this is a disclosed wireframe-F1 deviation — confirm T's walkthrough asserts the "unchanged → omitted" case explicitly so the behavior is pinned, not incidental.
- **[W-4]** Amended done-when / Walkthrough — the headless Playwright walkthrough exercising every GitLab control is an explicit acceptance criterion and is not delivered here (deferred to "T's Phase 6") — the branch must not merge before that walkthrough runs; this handoff is Feature-complete only.

### NIT findings
- **[NIT-1]** src/features/platform-connections/github.tsx — the gap-4 list in the brief names `initialPriorityKeys` as needing a fix; no change was needed (the existing ternary already yields `[]` for a non-repository scope), but the handoff's "gap 4" summary should say so rather than imply all four sites changed.
- **[NIT-2]** tests/gitlab-connections.test.ts ("is offered after GitHub and Bitbucket…") — the test unmounts and recreates the root mid-test to re-render; a second `render()` helper call would be clearer than the inline `act(() => root.unmount()); root = createRoot(container)`.
- **[NIT-3]** src/features/platform-connections/gitlab.tsx (`normalizeAddress`) — the address is trimmed/slash-stripped at submit but `gitlabAddressError` validates the raw value; a value like `" https://gitlab.com "` fails validation while normalizing cleanly — pick one canonical form for both.

### Verdict
PASS — no BLOCK findings; testing may proceed.