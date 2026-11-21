
## Phase 2 Design (D) — #172

- **Decided:** Wireframe (`wireframe.html`) is limited to the Docker image row on new-pool, settings, read-only settings and pool detail. GitLab: optional, empty, placeholder and help name `gitlab/gitlab-runner:ubuntu-v19.6.0`. Empty displays as "Default (gitlab/gitlab-runner:ubuntu-v19.6.0)". GitHub and Bitbucket are unchanged.
- **Decided:** Form image state holds only user-typed text, so a typed image survives GitLab to GitHub switching and the GitHub default reappears only when nothing was typed.
- **Assumed:** No runner-group control exists on any form (confirmed in the route sources), so nothing is drawn for it. Fleet execution section gets the effective default text for an empty GitLab image. `toCreatePayload` drops `runnerGroupId: 1`.
- **Hedged:** The wireframe was not rendered visually, because of the headless-only rule. It is text, HTML and CSS only (no icon geometry), and it parses as well-formed HTML.
- **Evidence:** `src/routes/_app/runner-pools.new.tsx` (docker-image row, `useState(options.defaults.dockerImage)`), `src/routes/_app/runner-pools.$poolId.settings.tsx` (editable row, `ReadOnlyPool`), `src/routes/_app/runner-pools.$poolId.index.tsx:119` (detail Image property). No survey.md exists for #172, so the survey was these sources plus the brief and brief review r1.
