# Handoff-A-dup: Phase 7 - GitLab in the shared connections and targets UI (#141)  (anti-duplication gate)

**Date:** 2026-11-20
**Branch:** claude/gitlab-141-ui-3ae7f5
**Diff base:** 3571a74   **Diff head:** c0e2d58
**Reuse map:** docs/handoffs/141-brief.md (Reuse / extend-vs-new in handoff-F.md)
**Verdict:** PASS

## Summary
F extended the existing objects the brief named and built no parallel path. GitLab is one more descriptor beside `bitbucket.tsx`, registered in the existing `platforms` array. Every shared-file change widens an existing contract generically.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `src/features/platform-connections/gitlab.tsx:46` | `OTHER_PLATFORM_KINDS` hardcodes the GitHub and Bitbucket kind ids for `excludes`. This follows the brief and the Bitbucket pattern, but a fourth platform means editing every descriptor. | Track as follow-up; no action now. |

No duplication; extension is clean:
- Row actions extend `ConnectionBindings` and `ConnectionRow` in `connections-section.tsx`, and use the one `SecretRevealDialog` (renamed type `SecretReveal`). No per-row dialog.
- The select field reuses `SearchableSelect`. `validate` and `defaultValue` extend `ConnectionField` and `FieldInput`.
- Platform option values extend `platformOptions`, `TargetUpdateValues`, `buildCreatePayload` and `PlatformOptionsSection`. `initialPlatformOptionValues` sits beside `initialSelection` in `targets.ts`.
- Delivery platform: `platformForDeliveryId` sits beside `getPlatform` in `registry.tsx`. `webhooks.tsx` reuses the exported `PlatformBadge`, with no third badge component.
- Pool fields in `schemas.ts` and `runner-pools.functions.ts` follow the Bitbucket pattern (create-only scope rule plus a shared bounds rule).
- The `github.tsx` change (gap 4) is generic: it branches on `organization` and contains no `gitlab` string.
- Backend: one narrow edit to the existing runners query in `resources.rs` (extra columns and a LEFT JOIN), as the brief's round-3 amendment allows. No `migrations/` or `docs/openapi/` changes.
- GitLab API functions follow the Bitbucket function shapes in `platform-connections.functions.ts`.

## Notes for F
None.
