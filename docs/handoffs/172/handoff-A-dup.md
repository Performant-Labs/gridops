# Handoff-A-dup: Phase 7 - GitLab Docker pool default image and no runner group (#172)  (anti-duplication gate)

**Date:** 2026-11-21
**Branch:** claude/gitlab-172-pool-image-bb7d36
**Diff base:** 7afb7c5   **Diff head:** 80ba004
**Reuse map:** none (no survey.md; the amended brief's Scope section stands in, as in handoff-A.md)
**Verdict:** PASS

## Summary
F extended the objects the brief named and built no parallel path. The Rust side adds a `gitlab: bool` parameter to the single `validate_pool_configuration` (the `macos_runtime` precedent) and reuses the existing runner-group normalisation. The TS side adds one optional `PlatformDescriptor` key, pure helpers in `targets.ts` in the `labelRules(descriptors, selection)` shape, and a neutral `dockerImageOptional` parameter on the existing `validateProvider`. No new file, module, validator or schema was added under `src/` or `crates/`.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | info | `src/features/runner-pools/schemas.ts:101` | `updateRunnerPoolSchemaFor(bool)` is a factory, not a second schema: the refinements exist once, and the exported `updateRunnerPoolSchema` is its `false` instance. `parseUpdateRunnerPoolInput` builds the relaxed instance per call only when asked. Not duplication. | None. |

All three of A's round-2 plan warns are honoured: the `targets.ts` helpers take the descriptors argument and import no registry; `input.validate(pool.scope == "gitlab")` runs after `pool_access` and before `assert_pool_admin` (`resources.rs:1603-1609`); `dockerImageOptional` is destructured beside `poolId` in `updateRunnerPoolAction` and never sent in the PUT body.

Drift checks:
- Layering and dependency direction: `schemas.ts` imports nothing from `platform-connections/`; `targets.ts` is unchanged in imports; only routes import `platformForPool` (the #169 pattern the detail route already uses).
- No platform id comparison or "gitlab" literal was added in `schemas.ts`'s `validateProvider`, the two routes, the detail route, `targets.ts` or `fleet-profile-section.tsx` (grep clean; the only `=== "gitlab"` is the create schema's existing scope branch and the Rust scope checks the brief decided).
- `gitlab.tsx` changed only by the pool-level `defaultDockerImage` key, so #171/#174 and #173 territory is untouched.
- `FleetExecutionSection` and its `defaultImages` fallback are reused unchanged; the route only widens the expression it passes.
- Warn (no action needed for this gate): `shownDockerImage` and `dockerImageHelp` are small display helpers in `targets.ts`; the brief asked for one composing helper, and F's folding of the "stored or default" choice into it is a reasonable reading.

## Notes for F
None required (PASS).
