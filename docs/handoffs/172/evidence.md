# Evidence: #172 (unchanged source the diff relies on)

## F (Phase 6)

- **Fact:** A GitLab pool's empty stored Docker image resolves to the GitLab runner image at provisioning, so the validator may let it through empty.
  **Source:** `crates/gridops-core/src/fleet/targets.rs:503-509`
  **Verbatim excerpt:**
  > pub fn docker_runner_image(pool_image: &str, gitlab: bool) -> String {
  >     match pool_image.trim() {
  >         "" if gitlab => crate::provisioning::GITLAB_RUNNER_IMAGE.to_owned(),
  >         "" => STOCK_DOCKER_IMAGE.to_owned(),
  >         image => image.to_owned(),
  >     }
  > }

- **Fact:** The server's GitLab runner image constant is the value the GitLab descriptor's `defaultDockerImage` copies.
  **Source:** `crates/gridops-core/src/provisioning.rs:34`
  **Verbatim excerpt:**
  > pub const GITLAB_RUNNER_IMAGE: &str = "gitlab/gitlab-runner:ubuntu-v19.6.0";

- **Fact:** The create handler already stores runner group 1 for a `repository` or `gitlab` pool, whatever the body sends, so letting `runnerGroupId` default to 0 for a GitLab pool needs no new normalisation.
  **Source:** `crates/gridops-api/src/resources.rs:2674-2679`
  **Verbatim excerpt:**
  > // Only GitHub organization pools use a runner group.
  > let runner_group_id = if matches!(input.scope.as_str(), "repository" | "gitlab") {
  >     1
  > } else {
  >     input.runner_group_id
  > };

- **Fact:** The update handler does the same from the stored pool's scope.
  **Source:** `crates/gridops-api/src/resources.rs:1741-1746`
  **Verbatim excerpt:**
  > // Only GitHub organization pools use a runner group.
  > let runner_group_id = if matches!(pool.scope.as_str(), "repository" | "gitlab") {
  >     1
  > } else {
  >     input.runner_group_id
  > };

- **Fact:** `pool_access` returns a 404 for a missing or inaccessible pool, which is why moving `validate` after it turns an invalid body to a missing pool from 400 into 404 (the accepted precedence change).
  **Source:** `crates/gridops-api/src/resources.rs:5032-5034`
  **Verbatim excerpt:**
  > .fetch_optional(&state.database)
  > .await?
  > .ok_or_else(|| ApiError::NotFound("Runner pool does not exist or is not accessible.".into()))

- **Fact:** The Fleet execution editor starts a new Docker platform from the `dockerImage` prop it is given, and falls back to the stock (GitHub) image only when that prop is blank. The settings route therefore passes a GitLab pool's raw default image, and `fleet-profile-section.tsx` is unchanged.
  **Source:** `src/features/fleet/fleet-profile-section.tsx:39-41`
  **Verbatim excerpt:**
  > /** Each runtime's starting image. Docker starts from the pool's own image, so jobs keep the tools it has. */
  > const defaultImages = (poolDockerImage: string): Record<Runtime, string> =>
  >   ({ ...stockImage, docker: poolDockerImage.trim() || stockImage.docker });

- **Fact:** GitHub's create partial always puts `runnerGroupId: 1` in the body (runner-group option A). The form therefore keeps sending it for GitLab pools, and the server ignores it.
  **Source:** `src/features/platform-connections/github.tsx:277-282`
  **Verbatim excerpt:**
  > return {
  >   installationId: first ? Number(first.connectionId) : 0,
  >   scope: "repository",
  >   repositoryIds: repositories.map((option) => Number(option.nativeId)),
  >   runnerGroupId: 1,
  > };
