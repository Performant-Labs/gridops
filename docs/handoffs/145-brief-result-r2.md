## Brief Review (Round 1)

### BLOCK findings

**[B-1]** [resolved, completeness] The acceptance criterion "The submitted payload must be byte-for-byte what the old form would have sent for the same selection" is untestable as written. "Byte-for-byte" is undefined for a JS form submission — it could mean JSON.stringify equality, URL-encoded form data equality, or object deep-equality before serialization. Without specifying the serialization format and the exact comparison point (route action input? fetch body? request payload after framework parsing?), no test can pass or fail deterministically. — Clarify the criterion to name the exact payload representation and comparison point, or replace with "the route action receives a deep-equal object to what the old form produced for the same selection."

**[B-2]** [resolved, completeness] The "Forward-compat" requirement says to "Confirm in survey.md that the descriptor type satisfies each without changes to the page, picker or options components" but the brief defines "docs under `docs/`" as in scope while also saying "No API or schema change. Do not touch `crates/`, `migrations/` or `docs/openapi/`". `survey.md`'s location is not specified in the brief, and the acceptance criteria do not include producing or updating `survey.md`. An unscoped deliverable with no acceptance criterion cannot be verified as complete. — State where `survey.md` lives (e.g. `docs/handoffs/145-platform-neutral-ui/survey.md`), and add an acceptance criterion that the file exists and contains the GitLab forward-compat table.

### Resolved findings

**[R-1]** The acceptance criterion "The submitted payload must be byte-for-byte what the old form would have sent for the same selection" is untestable as written. "Byte-for-byte" is undefined for a JS form submission — it could mean JSON.stringify equality, URL-encoded form data equality, or object deep-equality before serialization. Without specifying the serialization format and the exact comparison point (route action input? fetch body? request payload after framework parsing?), no test can pass or fail deterministically. — The brief now specifies: "the `data` object passed to `createRunnerPoolAction` (new-pool form) or `updateRunnerPoolAction` (pool settings) is deep-equal (Vitest `toEqual`) to the object the old form built for the same selection. This is the comparison point because `api()` sends exactly `JSON.stringify` of the parsed object; key order is not significant, and a key set to `undefined` counts as absent."

**[R-2]** The "Forward-compat" requirement says to "Confirm in survey.md that the descriptor type satisfies each without changes to the page, picker or options components" but the brief defines "docs under `docs/`" as in scope while also saying "No API or schema change. Do not touch `crates/`, `migrations/` or `docs/openapi/`". `survey.md`'s location is not specified in the brief, and the acceptance criteria do not include producing or updating `survey.md`. — The brief now specifies `docs/handoffs/145-platform-neutral-ui/survey.md` and includes an acceptance criterion: "`docs/handoffs/145-platform-neutral-ui/survey.md` exists and contains the GitLab forward-compat table described in the Forward-compat section."

### Needs-verification findings

**[NV-1]** The claim that the descriptor's connection bindings for GitHub (list-only plus existing external links) and Bitbucket (all four list/create/update/remove) accurately reflect the exported function signatures in `src/features/platform-connections/platform-connections.functions.ts` and the GitHub settings functions — evidence would settle it: the full exported function signatures and return types in those files.

**[NV-2]** The claim that extending `SearchableMultiSelect` with an optional group field and group headings is feasible without breaking its existing callers — evidence would settle it: the complete props interface and current listbox rendering implementation in `src/components/ui/searchable-multi-select.tsx`.

**[NV-3]** The claim that the current form auto-adds `tart` when a Bitbucket workspace is picked (line 378) and that this behavior must be preserved through the descriptor's target mapping — evidence would settle it: the exact logic at `src/routes/_app/runner-pools.new.tsx` lines 365-386 showing the auto-add behavior.

**[NV-4]** The claim that `src/features/operations/operations.functions.ts` exports `getSettingsPage()` returning `{ data: { githubApp, installations, user: { role } } }` — evidence would settle it: the function signature and return type in that file.

**[NV-5]** The claim that `pool-plan-preview.tsx` line 50 contains GitHub runner-group text branching on `scope === "organization"` and that this is a platform-branching site in a shared row — evidence would settle it: the full text and surrounding context of line 50 in `src/components/pool-plan-preview.tsx`.

### WARN findings

**[W-1]** The acceptance criterion "Every existing flow still works" is broad and relies on the Playwright walkthrough to exercise "every control on the Connections page and on the new-pool and pool-settings forms." If the walkthrough does not explicitly cover all listed flows (GitHub repository pools, GitHub runner-group pools, Bitbucket workspace pools, Bitbucket connection create/edit/remove, GitHub App management), a regression could pass unnoticed. — Recommend enumerating each flow as a distinct Playwright scenario or adding a checklist in the handoff.

**[W-2]** The brief requires the Connections page to support a post-create one-time reveal dialog generically, but neither GitHub nor Bitbucket uses it. The acceptance criterion covers it with a "test-only descriptor," but this means the production page has a code path exercised only by tests. If the test-only descriptor is not part of the production registry (only injected in tests), the production code path may be dead code. — Clarify whether the test-only descriptor is injected via a test fixture or must be registered in production, and ensure the dialog renders through the same production code path.

**[W-3]** The brief says "GitHub App setup (admin-only) may stay as a section on this page" — the word "may" makes this optional. If the GitHub App setup is not preserved on the Connections page, the "managing the GitHub App" flow in the acceptance criteria will have no UI. — Decide and state definitively whether GitHub App setup remains on the Connections page.

### NIT findings

**[NIT-1]** The descriptor type is named `PlatformDescriptor` and lives in `src/features/platform-connections/registry.tsx`, but the brief does not specify whether the icon component is required or optional. If every descriptor must carry an icon, the type should mark it required; if some platforms legitimately have no icon, the page must handle a missing icon. — Clarify icon optionality.

**[NIT-2]** The brief says "one collapsed 'Platform options' section in the pool forms" but does not specify whether the collapse state is per-platform (if multiple platforms are targeted, does each platform get its own collapsible section, or is there one section listing all platforms' options?). — Clarify the structure when a pool targets multiple platforms.

**[NIT-3]** The redirect requirement says "each redirect points directly at `/settings/connections` — no redirect chains," but the brief does not explicitly state whether the old routes (`/settings/integrations`, `/settings/github`) remain as stub files with redirects or are deleted and the redirect handled at a higher level. — State the intended route structure.

### Verdict

PASS — no BLOCK findings; implementation may proceed.