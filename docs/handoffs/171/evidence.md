# Evidence: #171 / #174 (unchanged source the diff relies on)

## F (Phase 5)

- **Fact:** The pool's Events list shows a row in the amber warning tone only when its level is exactly `warning`. Any other level except `error` shows as success, so the new event writes `"warning"`.
  **Source:** `src/features/runner-pools/pool-events.tsx:74`
  **Verbatim excerpt:**
  > const tone = event.level === "error" ? "danger" : event.level === "warning" ? "warning" : "success";

- **Fact (corrected in the Phase 6 rework for diff-gate B-1):** `GitLabError::Upstream` carries a free-form string, so the event message no longer uses `GitLabError`'s `Display` for it. `client_safe_reason` (in the diff) writes plain "GitLab request failed" for `Upstream` and the fixed `Display` text for every other variant. The detail stays in the returned error, which callers only log.
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

- **Fact:** The one variant whose text comes from a GitLab response, `MissingScope`, can only hold lowercase letters, `_` and spaces (at most 64 bytes); anything else is reported as the default scope. A response body never reaches the event message.
  **Source:** `crates/gridops-core/src/gitlab/http.rs:502-520`
  **Verbatim excerpt:**
  > fn missing_scope(body: &[u8]) -> Option<String> {
  >     let body: Value = serde_json::from_slice(body).ok()?;
  >     if body.get("error").and_then(Value::as_str) != Some("insufficient_scope") {
  >         return None;
  >     }
  >     let scope = body
  >         .get("scope")
  >         .and_then(Value::as_str)
  >         .map(str::trim)
  >         .filter(|scope| {
  >             !scope.is_empty()
  >                 && scope.len() <= 64
  >                 && scope
  >                     .bytes()
  >                     .all(|byte| byte.is_ascii_lowercase() || byte == b'_' || byte == b' ')
  >         })
  >         .unwrap_or(DEFAULT_MISSING_SCOPE);
  >     Some(scope.to_owned())
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

## T (Phase 7)

- **Fact:** The dedup's one-hour window compares `created_at` in milliseconds, the unit the event insert writes (so the sweep's read-back and the insert agree).
  **Source:** `crates/gridops-reconciler/src/main.rs:3988`
  **Verbatim excerpt:**
  > .bind(now_millis())

## F (Phase 6 rework, diff-gate round 1)

- **Fact (NV-2):** The fake does not model token scopes. Its `verify_access` checks only that the project or group was seeded, so `fake.rs` needs no change to stay consistent with the HTTP client's scope rule.
  **Source:** `crates/gridops-core/src/gitlab/fake.rs:316-332`
  **Verbatim excerpt:**
  >     async fn verify_access(
  >         &self,
  >         credentials: &GitLabCredentials,
  >         scope: &Scope,
  >     ) -> Result<(), GitLabError> {
  >         self.call(credentials, FakeMethod::VerifyAccess, |instance| {
  >             if instance
  >                 .scopes
  >                 .iter()
  >                 .any(|seeded| seeded.kind() == scope.kind() && seeded.id() == scope.id())
  >             {
  >                 Ok(())
  >             } else {
  >                 Err(GitLabError::NotFound)
  >             }
  >         })
  >     }

- **Fact (NV-3):** The table-driven create-refusal test runs all seven cases, including the new "missing manage_runner" case, and only then asserts that no connection row and no runtime secret were stored. The stored-nothing assertion therefore covers the seventh case.
  **Source:** `crates/gridops-api/src/resources/gitlab_tests.rs:768` and `:799-806`
  **Verbatim excerpt:**
  >     for (case, failure, scope_path, named) in cases {
  >
  >     let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gitlab_connections")
  >         .fetch_one(&h.database)
  >         .await?;
  >     let secrets: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runtime_secrets")
  >         .fetch_one(&h.database)
  >         .await?;
  >     assert_eq!((rows, secrets), (0, 0), "a refused create stores nothing");

- **Fact (NV-1):** The failing `release_preflight` test asserts on a "Linux" fixture machine and fails at the pre-branch commit `7164fda` too, with no #171 code present. Run on this Mac: `cargo test -p gridops-api --test fleet_http release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` in a detached worktree at `7164fda` -> `test result: FAILED. 0 passed; 1 failed` (panic at `release_preflight.rs:234:5`).
  **Source:** `crates/gridops-api/tests/fleet_http/release_preflight.rs:220-224`
  **Verbatim excerpt:**
  > async fn the_release_preflight_fails_early_without_a_macos_arm64_machine() -> Result<()> {
  >     let f = Fixture::new().await?;
  >     seed_pool(&f).await?;
  >     // A healthy fleet of one Linux machine: no Mac for the macOS agent build.
  >     active_machine(&f, &f.origin, Arc::new(FakeDocker::default())).await?;

- **Fact (NV-4):** Outside `docs/handoffs/` and the OpenAPI file, the only GitLab scope lists in `docs/`, `README.md` and `CHANGELOG.md` are the three-row table in `docs/gitlab-runners.md`, its runtime-warning paragraph and the new CHANGELOG entry. None still lists two scopes. Output of `grep -rn -E "create_runner|manage_runner|read_api" docs README.md CHANGELOG.md | grep -v docs/handoffs/ | grep -v openapi`:
  **Source:** `docs/gitlab-runners.md:23-25`
  **Verbatim excerpt:**
  > | `create_runner` | Creating each runner (`POST /user/runners`). |
  > | `manage_runner` | Deleting each runner when it leaves service, and the orphan sweep (`DELETE /runners/:id`). GitLab 17.1 or later. |
  > | `read_api` | Resolving the project or group, the pending-jobs poll, reading single jobs, listing the scope's runners for the sweep, and checking the token. |
  >
  > (other hits: `docs/gitlab-runners.md:158` "refuses it (for example, the token lacks `manage_runner`), the runner's pool"; `CHANGELOG.md:35` and `:37`, the Unreleased "Fixed" entry)
