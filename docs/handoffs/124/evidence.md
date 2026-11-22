# Evidence (#124)

## F (Phase 6)

- **Fact:** the API binary migrates its database before it binds its port, so a child that loses the port has still migrated the fixture's database, and a database the fixture migrated and seeded first is only re-checked at startup (the fixtures now seed before spawning).
  **Source:** `crates/gridops-api/src/main.rs:87`, `crates/gridops-api/src/main.rs:195`
  **Verbatim excerpt:**
  >     let database = connect_database(&config).await?;
  >     let listener = tokio::net::TcpListener::bind(config.api_bind()).await?;

- **Fact:** the test-side `connect_database_path` runs the same migrations, so seeding a fresh database before the API starts works.
  **Source:** `crates/gridops-core/src/db.rs:46-49`
  **Verbatim excerpt:**
  >     MIGRATOR
  >         .run(&pool)
  >         .await
  >         .context("failed to migrate GridOps database")?;

- **Fact:** `GET /api/v1/auth/me` is served by `auth::me`, which requires the `AuthUser` extractor (any role), so it is the identity probe the helper uses.
  **Source:** `crates/gridops-api/src/main.rs:273`, `crates/gridops-api/src/auth.rs:145`
  **Verbatim excerpt:**
  >         .route("/api/v1/auth/me", get(auth::me))
  > pub async fn me(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Viewer>> {

- **Fact:** a session cookie is split on its first `.`, and a token whose hash is not in the serving process's database is answered 401; so a per-fixture token (a simple UUID, no `.`) is unknown to any other API, which answers the probe 401.
  **Source:** `crates/gridops-api/src/auth.rs:87`, `crates/gridops-api/src/auth.rs:101-103`
  **Verbatim excerpt:**
  >     let (token, signature) = value.split_once('.').ok_or(ApiError::Unauthorized)?;
  >     let Some(row) = row else {
  >         return Err(ApiError::Unauthorized);
  >     };

- **Fact:** the probe's only side effect is the `last_seen_at` update the `AuthUser` extractor makes; it happens at fixture start (before any test body runs) and on `Fixture::restart`. The only tests that set and read `last_seen_at` (`tests/fleet_http/query.rs`) never restart.
  **Source:** `crates/gridops-api/src/auth.rs:64-68`
  **Verbatim excerpt:**
  >         let session = authenticate_session(parts, state).await?;
  >         sqlx::query("UPDATE sessions SET last_seen_at = ? WHERE id = ?")
  >             .bind(now_millis())
  >             .bind(&session.session_id)
  >             .execute(&state.database)


## T (follow-up round on PR #205)

- **Fact:** the API commits a command as `delivered` inside the poll transaction and only then returns it for the poll reply, so `host_commands.state='delivered'` does not prove the agent received the command (the race behind `an_api_restart_leaves_a_running_runner_alone`).
  **Source:** `crates/gridops-core/src/fleet/registry/commands.rs:145`, `crates/gridops-core/src/fleet/registry/commands.rs:161`
  **Verbatim excerpt:**
  >             sqlx::query("UPDATE host_commands SET state='delivered',attempt_count=? WHERE id=?")
  >         tx.commit().await?;

- **Fact:** the agent journals a command's result before `dispatch` returns it for posting, so a `/commands/result` post carrying the inspect's command id proves the answer is journaled and will be handed over after a reconnect.
  **Source:** `crates/gridops-host-agent/src/dispatch.rs:120`, `crates/gridops-host-agent/src/client.rs:225`
  **Verbatim excerpt:**
  >         self.journal.complete(&result).await?;
  >         self.agent::<_, serde_json::Value>(host, credential, "commands/result", result)

- **Fact:** an open command past its deadline is held `uncertain` only if it was delivered; one still pending is `cancelled` (so `an_administrator_repairs_a_placement_the_agent_cannot_confirm` must see the start run before its deadline passes).
  **Source:** `crates/gridops-core/src/fleet/registry/commands.rs:778-780`
  **Verbatim excerpt:**
  >         let delivered = row.get::<String, _>("state") == "delivered";
  >         sqlx::query("UPDATE host_commands SET state=? WHERE id=?")
  >             .bind(if delivered { "uncertain" } else { "cancelled" })
