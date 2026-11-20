# Handoff-A-dup: Phase 7 - One Connections page and one Targets picker for every CI platform (#145)  (anti-duplication gate)

**Date:** 2026-11-20
**Branch:** claude/ui-145-platform-neutral-cf5832
**Diff base:** ef65745   **Diff head:** 932879f
**Reuse map:** docs/handoffs/145-brief.md ("Reuse map" + round-3 block), docs/handoffs/145/survey.md
**Verdict:** PASS

## Summary
F extended or moved the analogous objects the map named and built no parallel path. `SearchableMultiSelect`, `SecretDialog` (extracted to `SecretRevealDialog`), `validateBitbucketConnections` (exports `bitbucketLabelError`), the Bitbucket and GitHub settings page logic, `tart-base-vm-row`, `pool-plan-preview` and the settings nav were all extended in place. The new modules are exactly the NEW set the map lists, and the old per-platform pages and pool-form rows are gone rather than kept beside them.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `src/routes/_app/runner-pools.index.tsx:81`, `src/routes/_app/runner-pools.$poolId.index.tsx:130` | The pool list and pool detail pages still branch on `pool.scope === "repository"`, which is GitHub-shaped. Not a duplicate path and outside the brief's scope (it names the Connections page, the Targets row, Platform options and runner badges). #145 does not read the registry here, so #141 will have to revisit these. | Track as follow-up: render the destination through the descriptor's `readOnlySummary`. |
| 2 | warn | `src/features/platform-connections/bitbucket.tsx:4` | The descriptor imports `bitbucketLabelError` from `runner-pools/schemas`, while `runner-pools/targets-row.tsx` imports the registry. The dependency is feature to feature in both directions, but at the module level it is acyclic: `schemas.ts` imports nothing from `platform-connections`. | None needed. Keep `schemas.ts` free of registry imports. |

Checked and clean:
- **No parallel Bitbucket or GitHub UI.** `grep -i bitbucket src` finds the name only in the Bitbucket descriptor, the registry, `schemas.ts` (the rule), the API functions and the redirect-only `settings.integrations.tsx`. `settings.github.tsx`, `settings.integrations.tsx` and `_app/platform-connections.tsx` are redirect-only, each keeping its route declaration.
- **Pool forms.** `runner-pools.new.tsx` and `runner-pools.$poolId.settings.tsx` use one `TargetsRow` and one `PlatformOptionsSection` for every platform, with payloads from the shared `buildCreatePayload` and `buildUpdatePayload`. The only remaining direct `SearchableMultiSelect` uses are the form's non-target pickers (providers and priority).
- **Registry is the single source.** `runner-row.tsx` badge and details line and `RunnerActionsMenu`'s name read `getPlatform()`. The hard-coded `runner.platform === "bitbucket"` ternary was removed. No other `platform ===` comparison remains in `src`.
- **No duplicated shared pieces.** The label regex stays only in `schemas.ts`. `SecretDialog` has no remaining definition. The multi-select was extended with optional fields and does not import the registry.
- **Layering.** `registry` imports the descriptors, and descriptors import only `types.ts` plus functions and UI primitives, never each other or routes. `targets.ts` imports only types. The shared components (`targets-row`, `platform-options-section`, `connections-section`) hold no platform names, enforced by `tests/platform-neutral-source.test.ts`.
- **Coexistence.** No changes under `crates/`, `migrations/` or `docs/openapi/`. `schemas.ts` changed only by exporting an existing rule.

## Notes for F
None; no rework required.
