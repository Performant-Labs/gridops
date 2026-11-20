# Handoff-A-dup: Phase 7 - One Connections page and one Targets picker for every CI platform (#145)  (anti-duplication gate, round 4)

**Date:** 2026-11-20
**Branch:** claude/ui-145-platform-neutral-cf5832
**Diff base:** 932879f (the cycle-1 gate's head; cycle 1 passed at c2e91ac)   **Diff head:** 14fc5b7
**Reuse map:** docs/handoffs/145-brief.md ("Reuse map" + round-3 and round-4 blocks), docs/handoffs/145/survey.md
**Verdict:** PASS

## Summary
Round 4 extended the existing objects in place and built no parallel path. The nine non-handoff files changed are all modifications (no added files): `ConnectionsSection` and its Add connection dialog, the descriptor contract in `types.ts`, the GitHub and Bitbucket descriptors, the registry fallback, and `runner-row.tsx`. The earlier cycle-1 PASS still holds; nothing from it regressed.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `src/features/platform-connections/github-app-section.tsx:50` and `src/features/platform-connections/github.tsx:195` | The GitHub App section still renders its own "Install" link from `githubApp.installUrl`, while the GitHub descriptor's new `connectLink` offers the same URL through Add connection. Two entry points to one destination, but they are different surfaces (App management vs Add connection chooser) and the brief's round-4 decisions require the chooser link. Not a parallel implementation: both read the same page data. | Optional follow-up: leave as is, or have the App section reuse the descriptor's `connectLink` if the two ever drift. |
| 2 | warn | `src/routes/_app/runner-pools.index.tsx:81`, `src/routes/_app/runner-pools.$poolId.index.tsx:130` | Carried over from cycle 1: pool list and detail still branch on `pool.scope === "repository"`. Out of scope of the brief; #141 will revisit. | Track as follow-up: render through `readOnlySummary`. |

Checked and clean:
- **No new files, no new module.** `git diff --name-status 932879f HEAD` outside `docs/handoffs` is nine `M` lines: five platform-connections files, `runner-row.tsx` and three test files.
- **One dialog, extended.** The Add connection dialog was lifted into `ConnectionsSection` and now steps platform chooser, then the `create` form or the `connectLink` step. The old per-platform add flow is gone, not kept beside it. The empty state reuses `EmptyState` from `~/components/page`, the unavailable text reuses `Callout`, the link button reuses `buttonVariants`, and the reveal reuses `SecretRevealDialog`.
- **Contract extended, not forked.** `types.ts` gains `chooserDescription`, optional `connectLink` and a nullable `emptyText`, and loses `externalLinks`. No remaining reference to `externalLinks` in `src`, `tests` or the survey.
- **Registry is still the single source.** `runner-row.tsx` only adds `title={link.label}` from `getPlatform().runnerLink`. No `platform ===` comparison reintroduced.
- **Shared components stay platform-neutral.** `grep -niE "github|bitbucket"` on `targets-row.tsx`, `platform-options-section.tsx` and `connections-section.tsx` returns nothing. Dependency direction unchanged: `connections-section.tsx` imports `registry` and `types` from the platform side only.
- **Cross-cutting.** `postCreate` failures go through the existing `toast` from `sonner`, matching `save` and `useAction` neighbours; `submit` catches only `create` errors. Query invalidation reuses `connectionsQueryKey`.
- **Coexistence.** `git diff --stat master...HEAD -- crates migrations docs/openapi` is empty. No API, DB or OpenAPI change.

## Notes for F
None; no rework required.
