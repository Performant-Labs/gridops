# Handoff-A-dup: Phase 7 - One Connections page and one Targets picker for every CI platform (#145)  (anti-duplication gate, round 5)

**Date:** 2026-11-20
**Branch:** claude/ui-145-platform-neutral-cf5832
**Diff base:** 14fc5b7 (round-4 gate head)   **Diff head:** 1124233
**Reuse map:** docs/handoffs/145-brief.md ("Reuse map" + round blocks), docs/handoffs/145/survey.md
**Verdict:** PASS

## Summary
Round 5 (F's fix for S's 360 px REWORK) edited two existing objects in place, `SettingsSection` and `ConnectionRow`, with layout classes only. No file, component, prop, export or dependency was added. No parallel path. The round-4 PASS and the two warns stand.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `src/components/settings-ui.tsx:28` | A shared component (used by about 14 settings and pool surfaces) was edited by a story that does not own it. This is spec'd (S's REWORK names `SettingsSection` as the place to fix) and F grepped the users. The header wraps only when actions plus 12rem do not fit, and F reports pixel-identical output at 768 and 1280 and for the fleet-profile Save at 360. Not drift. Of the `actions={...}` users, only `fleet-profile-section`, `connections-section` and `github-app-section` use `SettingsSection`; the rest use other components. | None. Track the other `SettingsSection` pages' 360 appearance in U's re-walk. |
| 2 | warn | `src/features/platform-connections/github-app-section.tsx:50` | Carried over: App section keeps its own Install link beside the descriptor's `connectLink`. Same URL, different surfaces. | Follow-up only. |
| 3 | warn | `src/routes/_app/runner-pools.index.tsx:81`, `src/routes/_app/runner-pools.$poolId.index.tsx:130` | Carried over: pool list and detail still branch on `pool.scope === "repository"`. | Track for #141. |

Checked and clean:
- **No new files.** `git diff --name-status 14fc5b7 HEAD` outside `docs/handoffs` is two `M` lines: `src/components/settings-ui.tsx` and `src/features/platform-connections/connections-section.tsx`.
- **Fix made in the shared object, not beside it.** F rejected a local header in `github-app-section.tsx` as a duplicate of `SettingsSection`'s markup; the diff confirms the header was fixed in `SettingsSection` itself.
- **`ConnectionRow` extended in place.** The aside and Edit/Remove actions were regrouped in one trailing wrapper inside the same row; the `showActions` condition is the same as before; the descriptor contract (`types.ts`) is untouched. No `platform ===` branch and no platform name was added to shared code.
- **Layering and dependency direction unchanged.** No import added or changed in either file.
- **Cross-cutting.** Interactions (`run`, confirm, success text, `reload`) are unchanged.
- **Coexistence.** `git diff --stat master...HEAD -- crates migrations docs/openapi` is empty.

## Notes for F
None; no rework required.
