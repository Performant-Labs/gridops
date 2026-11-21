# Evidence: #138 GitLab platform-connection and pool-target API

Source facts in **unchanged** code that F's diff (and T's tests) rely on.

## F (Phase 5)

- **Fact:** Pool memberships in `runner_pool_gitlab_connections` cascade when their connection is deleted, so an unchecked connection delete would silently untarget a pool. That is why `delete_gitlab_connection` checks pool memberships first and answers 409.
  **Source:** `migrations/0047_gitlab.sql:49-50`
  **Verbatim excerpt:**
  > FOREIGN KEY (`pool_id`) REFERENCES `runner_pools`(`id`) ON UPDATE no action ON DELETE cascade,
  > FOREIGN KEY (`connection_id`) REFERENCES `gitlab_connections`(`id`) ON UPDATE no action ON DELETE cascade

- **Fact:** `fleet_ci_targets.gitlab_connection_id` has no ON DELETE action (NO ACTION), so deleting a connection that a fleet target references would fail with a foreign-key error (a 500) unless it is checked first and answered with 409.
  **Source:** `migrations/0047_gitlab.sql:97`
  **Verbatim excerpt:**
  > gitlab_connection_id TEXT REFERENCES gitlab_connections(id),

- **Fact:** `runners.gitlab_connection_id` is SET NULL when its connection is deleted. That is why the D17 GitLab arm treats a NULL connection id as "connection gone": it skips the GitLab call and still deletes the runner.
  **Source:** `migrations/0047_gitlab.sql:144-145`
  **Verbatim excerpt:**
  > CHECK (runtime IN ('vm','native')), `gitlab_connection_id` text
  > REFERENCES `gitlab_connections`(`id`) ON UPDATE no action ON DELETE set null, `gitlab_runner_id` integer,

- **Fact:** Inserting a `runner_pools` row maps it to its installation through a trigger. So a GitLab pool, which is created with an `installationId` and no repositories, passes the `runner_pool_installations` checks in `pool_access` and `ensure_pool_access`.
  **Source:** `migrations/0014_runner_pool_installations.sql:19-24`
  **Verbatim excerpt:**
  > CREATE TRIGGER `runner_pool_installation_after_pool_insert`
  > AFTER INSERT ON `runner_pools`
  > BEGIN
  > 	INSERT OR IGNORE INTO `runner_pool_installations` (`pool_id`,`installation_id`)
  > 	VALUES (NEW.`id`,NEW.`installation_id`);
  > END;

- **Fact:** `require_system_admin`, which every GitLab connection write and `selected_gitlab_connections` call, allows only the `admin` role and answers 403 to everyone else, including an installation administrator.
  **Source:** `crates/gridops-api/src/auth.rs:185-191`
  **Verbatim excerpt:**
  > pub fn require_system_admin(user: &AuthUser) -> ApiResult<()> {
  >     if user.role == "admin" {
  >         Ok(())
  >     } else {
  >         Err(ApiError::Forbidden)
  >     }
  > }

- **Fact:** The webhook secret comes from `random_token(32)`: 32 random bytes encoded as unpadded URL-safe base64, which is 43 characters. T's tests require at least 32.
  **Source:** `crates/gridops-api/src/auth.rs:410-415`
  **Verbatim excerpt:**
  > pub fn random_token(bytes: usize) -> String {
  >     use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
  >     let mut value = vec![0_u8; bytes];
  >     rand::rng().fill(&mut value[..]);
  >     URL_SAFE_NO_PAD.encode(value)
  > }

- **Fact:** `ApiError::BadRequest(message)` is a 400 whose body is `{"error": message}`, which matches the spec's `ApiErrorBody`. Every `GitLabError` is mapped to it.
  **Source:** `crates/gridops-api/src/error.rs:37`
  **Verbatim excerpt:**
  > Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message.clone()),

- **Fact:** `FakeGitLab` keys its state and its call log by the credentials' `base_url` `Url`. The handlers build that `Url` as the stored base URL plus a trailing `/`, which for a root instance equals `Url::parse("https://gitlab.example.com")`.
  **Source:** `crates/gridops-core/src/gitlab/fake.rs:156-163`
  **Verbatim excerpt:**
  > state.calls.push(FakeCall {
  >     base_url: credentials.base_url.clone(),
  >     method,
  > });
  > let instance = state
  >     .instances
  >     .entry(credentials.base_url.clone())
  >     .or_default();

- **Fact:** The fake's `delete_runner` answers `NotFound` for a runner it doesn't know. The D17 arm counts that as a successful delete.
  **Source:** `crates/gridops-core/src/gitlab/fake.rs:209-217`
  **Verbatim excerpt:**
  > self.call(credentials, FakeMethod::DeleteRunner, |instance| {
  >     let before = instance.runners.len();
  >     instance.runners.retain(|runner| runner.id != runner_id);
  >     if instance.runners.len() == before {
  >         Err(GitLabError::NotFound)
  >     } else {
  >         Ok(())
  >     }
  > })

- **Fact:** The API's existing rotate and scale-down filters exclude only Bitbucket runners, so GitLab runners also reach `delete_runner_resources` through them. This is safe because the GitLab arm never fails a delete.
  **Source:** `crates/gridops-api/src/resources.rs:4666` and `:4733`
  **Verbatim excerpt:**
  > runner.ci_platform != "bitbucket"
  >
  > runner.ci_platform != "bitbucket" && !runner.busy && !out_of_sight(runner)
