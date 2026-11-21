# Handoff-D: Phase 2 - Design: GitLab Docker pool default image (#172)

**Date:** 2026-11-21
**Branch:** claude/gitlab-172-pool-image-bb7d36
**Mode:** (a) generated low-fi
**Wireframe:** docs/handoffs/172/wireframe.html

## Screens & states covered
Only the Docker image row changes, on four existing screens. No runner-group control exists on any form, and none is added.
- **New pool form (1a-1f):**
  - No target, GitHub or Bitbucket: required, prefilled with the GitHub default, help text "OCI image for each Linux runner." (unchanged).
  - GitLab target: optional, empty, placeholder `gitlab/gitlab-runner:ubuntu-v19.6.0`, help "Leave empty to use gitlab/gitlab-runner:ubuntu-v19.6.0." An empty submit sends an empty image and no `runnerGroupId`.
  - GitLab with a typed image: sent as typed, and kept if the selection switches back to GitHub or Bitbucket.
  - Switching GitLab back to GitHub with nothing typed: required and prefilled again.
  - Errors: GitLab with a bad non-empty image shows the server message in the existing "Could not create the pool" Callout. GitHub with the image cleared keeps today's required behaviour and message.
- **Settings form (2a-2d):** a GitLab pool's image is optional with the same help, and empty saves as empty. A GitHub or Bitbucket pool is unchanged. The Fleet execution section must not show a blank image for a GitLab pool.
- **Read-only settings and pool detail (3a-3d):** a GitLab pool with an empty image shows "Default (gitlab/gitlab-runner:ubuntu-v19.6.0)". Any stored image shows verbatim. The text comes from the descriptor.
- **State matrix (section 4):** required, initial value, help, empty-submit outcome and empty display, per platform class.
- **Not applicable:** the change touches single-value fields, so there are no empty/one/many list states.
- **Behaviour rule for the form state:** the field holds only what the user typed. The GitHub default is the fallback only while the user has typed nothing and the platform has no default image. This is how a typed image survives a platform switch (brief review W-3).

## Existing components/patterns reused
`SettingsSection` / `SettingsRow` / `Input` (`font-mono text-xs`), the existing optional-field help-text pattern, the danger `Callout`, and `PropertyRow` and `SettingsValue` for the read-only displays. The platform descriptor (`src/features/platform-connections/`) drives every platform-specific difference, so there are no new components, and no new platform comparisons in the route files.

## Open questions for approval
None blocking. Defaults I chose, which the operator can overrule:
1. **Placeholder and help text:** both name the default. This is the brief's "help/placeholder" and costs one line.
2. **Fleet execution section:** on a GitLab pool with an empty image I assume it receives the effective default text, not blank. If that section needs the raw value, F must say so in handoff-F.
3. **`toCreatePayload` and `runnerGroupId: 1`:** the wireframe assumes GitLab stops sending it, since there is no UI for it. Brief review NIT-2 asks for a firm decision, and that belongs to F and the brief. I assume removal.
4. **Wireframe not rendered:** the operator's global rule is headless-only, so I did not open the wireframe in a browser or preview. It is HTML and CSS only, with no SVG or icon geometry, and it parses as well-formed HTML. Layout and appearance are unverified visually. The structure and copy are what need sign-off.

## Approval
[To be filled by O: "Approved by operator <ISO timestamp>" — D does not self-approve.]
