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

