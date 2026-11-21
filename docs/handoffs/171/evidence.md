# Evidence: #171 / #174 (unchanged source the diff relies on)

## F (Phase 5)

- **Fact:** The pool's Events list shows a row in the amber warning tone only when its level is exactly `warning`. Any other level except `error` shows as success, so the new event writes `"warning"`.
  **Source:** `src/features/runner-pools/pool-events.tsx:74`
  **Verbatim excerpt:**
  > const tone = event.level === "error" ? "danger" : event.level === "warning" ? "warning" : "success";

- **Fact:** The reason text in the event message is `GitLabError`'s `Display`. That text carries only a fixed description, a scope name or a status, never a response body or a token. The message for `Upstream` starts "GitLab request failed", and the message for `MissingScope` names the scope.
  **Source:** `crates/gridops-core/src/gitlab/mod.rs:223-238`
  **Verbatim excerpt:**
  > pub enum GitLabError {
  >     #[error("GitLab rejected the access token")]
  >     Unauthorized,
  >     /// The access token lacks this scope, for example `create_runner`.
  >     #[error("the GitLab access token is missing the {0} scope")]
  >     MissingScope(String),
  >     #[error("the GitLab access token's user lacks the required role")]
  >     MissingRole,
  >     #[error("GitLab could not find the resource")]
  >     NotFound,
  >     /// `retry_after` is taken from a `Retry-After` header in seconds.
  >     #[error("GitLab rate limited the request")]
  >     RateLimited { retry_after: Option<Duration> },
  >     #[error("GitLab request failed: {0}")]
  >     Upstream(String),
  > }

- **Fact:** Create, token replacement and Verify all call `verify_access`. Tightening its scope loop therefore makes all three refuse a token without `manage_runner`.
  **Source:** `crates/gridops-api/src/resources/gitlab.rs:139`, `:254`, `:317`
  **Verbatim excerpt:**
  > .verify_access(&credentials, &scope)
  >
  > .verify_access(&credentials, &connection.scope)
  >
  > .verify_access(&credentials, &connection.scope)

- **Fact:** The API already turns `MissingScope` into "The GitLab access token is missing the {scope} scope." No change to it is needed.
  **Source:** `crates/gridops-api/src/resources/gitlab.rs:800-802`
  **Verbatim excerpt:**
  > GitLabError::MissingScope(scope) => {
  >     format!("The GitLab access token is missing the {scope} scope.")
  > }

- **Fact:** `runner_events.runner_id` is a foreign key to `runners(id)`. The orphan sweep's rows leave it NULL because the row the marker names may be gone. Rows also cascade away with their pool.
  **Source:** `migrations/0001_initial.sql:92-93`
  **Verbatim excerpt:**
  > FOREIGN KEY (`runner_id`) REFERENCES `runners`(`id`) ON UPDATE no action ON DELETE cascade,
  > FOREIGN KEY (`pool_id`) REFERENCES `runner_pools`(`id`) ON UPDATE no action ON DELETE cascade

- **Fact:** `runner_pool_gitlab_connections` has an index on `(connection_id, pool_id)`. The sweep fans out to linked pools with `WHERE connection_id=?`, which uses that index.
  **Source:** `migrations/0047_gitlab.sql:52-53`
  **Verbatim excerpt:**
  > CREATE INDEX `runner_pool_gitlab_connections_connection_idx`
  >   ON `runner_pool_gitlab_connections` (`connection_id`,`pool_id`);
