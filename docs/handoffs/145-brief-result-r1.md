## Brief Review (Round 1)

### BLOCK findings

**[B-1]** The acceptance criterion "The submitted payload must be byte-for-byte what the old form would have sent for the same selection" is untestable as written. "Byte-for-byte" is undefined for a JS form submission — it could mean JSON.stringify equality, URL-encoded form data equality, or object deep-equality before serialization. Without specifying the serialization format and the exact comparison point (route action input? fetch body? request payload after framework parsing?), no test can pass or fail deterministically. — Clarify the criterion to name the exact payload representation and comparison point, or replace with "the route action receives a deep-equal object to what the old form produced for the same selection."

**[B-2]** The "Forward-compat" requirement says to "Confirm in survey.md that the descriptor type satisfies each without changes to the page, picker or options components" but the brief defines "docs under `docs/`" as in scope while also saying "No API or schema change. Do not touch `crates/`, `migrations/` or `docs/openapi/`". `survey.md`'s location is not specified in the brief, and the acceptance criteria do not include producing or updating `survey.md`. An unscoped deliverable with no acceptance criterion cannot be verified as complete. — State where `survey.md` lives (e.g. `docs/handoffs/145-platform-neutral-ui/survey.md`), and add an acceptance criterion that the file exists and contains the GitLab forward-compat table.

### Needs-verification findings

**[NV-1]** The claim that `settings.integrations.tsx` "actually the Bitbucket page (`BitbucketSettings`, titled 'Bitbucket')" — evidence would settle it: `src/routes/settings.integrations.tsx` shown in full, particularly the exported component name and page title.

**[NV-2]** The claim that the GitHub settings functions and `src/features/platform-connections/platform-connections.functions.ts` expose list/create/update/remove bindings sufficient to implement the descriptor's connection bindings generically — evidence would settle it: the exported function signatures in `src/features/platform-connections/platform-connections.functions.ts` and the GitHub settings functions file.

**[NV-3]** The claim that the existing searchable select component (referenced via `tests/searchable-select.test.ts`) can be extended for grouped multi-select without a parallel widget — evidence would settle it: the component source in `src/components/` and its props/API.

**[NV-4]** The claim that a pool cannot mix incompatible platforms/kinds and that GitHub repository vs organization runner group are distinct "kinds" captured by the old form — evidence would settle it: the schema validation in `src/features/runner-pools/schemas.ts` and the form logic in `src/routes/_app/runner-pools.new.tsx` (Scope/Installation/Repositories/Runner-group rows, ~line 369 Bitbucket picker).

**[NV-5]** The claim that the post-create hook (one-time webhook secret reveal) is already supported by the existing API or can be rendered without API changes — evidence would settle it: the existing create-connection response types in `platform-connections.functions.ts` and whether any response field can carry a one-time secret.

### WARN findings

**[W-1]** The brief says shared runner-pool rows "no longer branch on platform" but does not enumerate which shared rows currently do branch (beyond "Additional labels" help at ~line 448 and `TartBaseVmRow bitbucket=…`). The forward-compat test in acceptance criteria ("no `bitbucket`/`github` branching in shared rows") cannot be checked without a baseline list of all current branching sites. — Recommend listing every current platform-branching location in the handoff so the diff gate can verify each is removed or moved into descriptors.

**[W-2]** The brief requires the descriptor to carry "whether the current user can manage" for connection fields, but the acceptance criteria only require Vitest coverage of "lookups, fallback, every descriptor well-formed, the targets picker's mixing rules and payload mapping, and the conditional Platform options section." The manage-permission logic and the post-create hook rendering path on the Connections page have no test requirement. — Add Vitest or Playwright acceptance criteria covering permission-gated edit/remove visibility and the one-time reveal dialog path.

### NIT findings

**[NIT-1]** The descriptor type is required to live in `src/features/platform-connections/` but the brief says "e.g. `registry.ts`/`registry.tsx`" — the "e.g." leaves the exact filename ambiguous. If the registry contains JSX (icon components, optional platform-options slots), `.tsx` is mandatory; if pure data/functions, `.ts` suffices. — Pick one and name it in the handoff.

**[NIT-2]** The redirect requirement covers `/settings/integrations`, `/settings/github`, and `/platform-connections`, but the starting-point `_app/platform-connections.tsx` is described as already redirecting to `/settings/integrations` — a redirect-to-a-redirect. The brief should specify whether the existing redirect is updated in place to point directly at `/settings/connections`, or whether a chain is acceptable temporarily.

### Verdict

BLOCK — 2 blocking finding(s); must resolve before implementation starts.