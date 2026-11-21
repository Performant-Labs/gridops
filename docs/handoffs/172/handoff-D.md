# Handoff-D: Phase 2 - Design: GitLab Docker pool default image (#172)

**Date:** 2026-11-21
**Branch:** claude/gitlab-172-pool-image-bb7d36
**Mode:** (a) generated low-fi
**Wireframe:** docs/handoffs/172/wireframe.html

## Screens & states covered
Only the Docker image row changes, on four existing screens. No runner-group control exists on any form, and none is added.
- **New pool form (1a-1f):**
  - No target, GitHub or Bitbucket: required, prefilled with the GitHub default, help text "OCI image for each Linux runner." (unchanged).
  - GitLab target: optional, empty, placeholder `gitlab/gitlab-runner:ubuntu-v19.6.0`, help "Leave empty to use gitlab/gitlab-runner:ubuntu-v19.6.0." An empty submit sends an empty image; the body keeps carrying `runnerGroupId: 1` as today (brief option A), and the server ignores it for a GitLab pool.
  - GitLab with a typed image: sent as typed, and kept if the selection switches back to GitHub or Bitbucket.
  - Switching GitLab back to GitHub with nothing typed: required and prefilled again.
  - Errors: GitLab with a bad non-empty image shows the server message in the existing "Could not create the pool" Callout. GitHub with the image cleared keeps today's required behaviour and message.
- **Settings form (2a-2d):** a GitLab pool's image is optional with the same help, and empty saves as empty. A GitHub or Bitbucket pool is unchanged. - **Fleet execution section (2d, 2e):** for a GitLab pool with an empty image, a new Docker platform starts from the descriptor's raw reference `gitlab/gitlab-runner:ubuntu-v19.6.0`, never the "Default (...)" label and never the GitHub image. GitHub and Bitbucket pools pass what they pass today.
- **Read-only settings and pool detail (3a-3d):** a GitLab pool with an empty image shows "Default (gitlab/gitlab-runner:ubuntu-v19.6.0)". Any stored image shows verbatim. The text comes from the descriptor.
- **State matrix (section 4):** required, initial value, help, empty-submit outcome and empty display, per platform class.
- **Not applicable:** the change touches single-value fields, so there are no empty/one/many list states.
- **Behaviour rule for the form state:** the field holds only what the user typed. The GitHub default is the fallback only while the user has typed nothing and the platform has no default image. This is how a typed image survives a platform switch (brief review W-3).

## Existing components/patterns reused
`SettingsSection` / `SettingsRow` / `Input` (`font-mono text-xs`), the existing optional-field help-text pattern, the danger `Callout`, and `PropertyRow` and `SettingsValue` for the read-only displays. The platform descriptor (`src/features/platform-connections/`) drives every platform-specific difference, so there are no new components, and no new platform comparisons in the route files.

## Open questions for approval
None blocking. The two amended-brief decisions are drawn as decided (revision after architecture review r1: runner group option A; Fleet section receives the raw default). Defaults I chose, which the operator can overrule:
1. **Placeholder and help text:** both name the default. This is the brief's "help/placeholder" and costs one line.
2. **Runner group:** no UI exists for it, so nothing is drawn. Per the brief (option A), the body keeps `runnerGroupId: 1`; whether GitLab's `toCreatePayload` keeps or drops its own `runnerGroupId: 1` is invisible on the wire and left to F.
3. **Wireframe not rendered:** the operator's global rule is headless-only, so I did not open the wireframe in a browser or preview. It is HTML and CSS only, with no SVG or icon geometry, and I checked headlessly that its tags are balanced. Layout and appearance are unverified visually. The structure and copy are what need sign-off.

## Approval
[To be filled by O: "Approved by operator <ISO timestamp>" — D does not self-approve.]
