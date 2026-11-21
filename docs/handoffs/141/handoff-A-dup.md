# Handoff-A-dup: Phase 7 - GitLab in the shared connections and targets UI (#141)  (anti-duplication gate, re-check after S REWORK)

**Date:** 2026-11-20
**Branch:** claude/gitlab-141-ui-3ae7f5
**Diff base:** e5d685f (previous A-dup PASS; full-feature diff from 3571a74 passed there)   **Diff head:** efcbc82
**Reuse map:** docs/handoffs/141-brief.md (Reuse / extend-vs-new in handoff-F.md)
**Verdict:** PASS

## Summary
The only production change since the last A-dup PASS is F's rework for S's clipped-Edit defect: one class-string change plus a comment in `ConnectionRow` (`connections-section.tsx`). It edits the existing shared action cluster in place and adds no file, component, helper or GitLab string. The earlier finding (descriptor extends the registry, no parallel path) still holds because no other src, crates or tests file changed since c0e2d58.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `src/features/platform-connections/gitlab.tsx:46` | Carried over from the earlier pass: `OTHER_PLATFORM_KINDS` hardcodes the GitHub and Bitbucket kind ids for `excludes`, following the brief and the Bitbucket pattern. A fourth platform means editing every descriptor. | Track as follow-up; no action now. |

No duplication; extension is clean:
- The rework fixes the layout generically in the existing `ConnectionRow` cluster (`flex min-w-0 flex-wrap items-center justify-end gap-1`), where it helps any descriptor that contributes several actions. It does not add a GitLab-specific wrapper or breakpoint branch. `sm`+ layout is unchanged because the parent is `sm:w-auto sm:shrink-0`.
- The diff since e5d685f contains zero `gitlab` strings in `src/`, so the neutral-source rule holds.
- Re-confirmed from the earlier pass: GitLab is one descriptor (`gitlab.tsx`) plus one line in `registry.tsx`. Row actions, select field, platform option values, delivery platform lookup and pool wire fields extend `ConnectionBindings`, `ConnectionField`, `platformOptions`, `buildCreatePayload`, `getPlatform`'s neighbour `platformForDeliveryId`, and the Bitbucket-pattern schema fields. The reveal reuses `SecretRevealDialog`; the badge reuses `PlatformBadge`.
- The backend edit is limited to the one runners-list query and its test, as the brief allows. Nothing under `migrations/` or `docs/openapi/` changed.

## Notes for F
None (PASS).
