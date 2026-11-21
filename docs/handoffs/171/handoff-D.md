# Handoff-D: Phase 2 - Design (#171 / #174)

**Date:** 2026-11-21
**Branch:** claude/gitlab-171-scopes-cbe8b5
**Mode:** (a) generated low-fi
**Wireframe:** docs/handoffs/171/wireframe.html

## Screens & states covered
- **GitLab connection form, Access token help (1a add, 1b token replacement):** one short paragraph naming `create_runner`, `manage_runner`, `read_api` (or `api`), "Maintainer role or higher on a project, or the Owner role on a group", and that GridOps checks both before saving. "Owner" is no longer stated for a project. The same field definition serves both modes.
- **Refusal states (1c):** create, token replacement and Verify each show "The GitLab access token is missing the manage_runner scope." (existing message format, existing error surface). Create and replacement leave nothing saved or changed. Verify success copy unchanged.
- **Pool Events list (2a many, 2b one expanded, 2c narrow, 2d empty/loading/error):** a warning row named "GitLab runner not deleted" with the brief's message. Runner-linked deletions show the runner id; orphan-sweep rows show an em dash and are deduplicated per pool, connection and reason within an hour. Empty/loading/error are the existing states. A successful or NotFound deletion writes nothing.
- No actions are added or disabled anywhere.

## Existing components/patterns reused
- `src/features/platform-connections/gitlab.tsx` field `help` slot (text edit only).
- `PoolEventTable`/`PoolEventRow` in `src/features/runner-pools/pool-events.tsx`: warning tone dot, Runner column, expandable metadata, mobile layout. No new component, column or control.
- Existing connection-form and Verify toast/error surface.

## Open questions for approval
None. Two choices made by D for the implementor: metadata keys in 2b are illustrative (tests assert message text, not metadata shape), and scope names render as inline code only if the help slot already supports it; plain text otherwise. The wireframe was written as static HTML with no icons or SVG (Unicode only, amber/red/green CSS dots); it was not rendered or viewed, per the session's headless-only rule, so visual layout is unverified.

## Approval
[To be filled by O.]
