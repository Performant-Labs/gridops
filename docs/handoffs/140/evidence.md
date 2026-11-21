# Evidence: #140 (unchanged source the diff relies on)

## F (Phase 6)

- **Fact:** `retry` refuses a row recorded with `signature_valid` false before any dispatch, so a `gitlab:unmatched:<key>` row never reaches the new GitLab arm.
  **Source:** `crates/gridops-api/src/webhooks.rs:197-201`
  **Verbatim excerpt:**
  > if !delivery.get::<bool, _>("signature_valid") {
  >     return Err(ApiError::BadRequest(
  >         "Only verified webhook payloads can be retried.".into(),
  >     ));
  > }

- **Fact:** a GitLab delivery has a NULL `installation_id`, so `retry` requires a system administrator for it.
  **Source:** `crates/gridops-api/src/webhooks.rs:204`
  **Verbatim excerpt:**
  > None => require_system_admin(&user)?,

- **Fact:** the credentials helper the webhook now reuses (made `pub(crate)` in this diff) appends `/` to the stored base URL and maps a bad URL to an internal error.
  **Source:** `crates/gridops-api/src/resources/gitlab.rs:711-724`
  **Verbatim excerpt:**
  > /// The credentials for one call. The stored base URL has no trailing slash;
  > /// the client gets one, so API paths join beneath an instance served under a
  > /// path prefix.
  > pub(crate) fn credentials(base_url: &str, access_token: String) -> ApiResult<GitLabCredentials> {
  >     let base_url = Url::parse(&format!("{base_url}/")).map_err(|error| {

- **Fact:** `runtime_secret` answers `None` for a key with no stored row and an error only when a stored value fails to open; the matcher treats `None` as "this candidate does not match".
  **Source:** `crates/gridops-api/src/state.rs:153-166`
  **Verbatim excerpt:**
  > pub async fn runtime_secret(&self, key: &str) -> Result<Option<String>> {
  >     let sealed =
  >         sqlx::query_scalar::<_, String>("SELECT value FROM runtime_secrets WHERE key=?")
  >             .bind(key)
  >             .fetch_optional(&self.database)
  >             .await?;

- **Fact:** `ApiError::Unauthorized` is answered as a 401 with the shared error body (the shape the spec's 401 declares).
  **Source:** `crates/gridops-api/src/error.rs:35`
  **Verbatim excerpt:**
  > Self::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),

- **Fact:** `build_status_changed_at` is parsed by the existing RFC 3339 helper into epoch milliseconds; anything else is `None` (the handler then uses the receive time).
  **Source:** `crates/gridops-api/src/webhooks.rs:986-990`
  **Verbatim excerpt:**
  > fn date_value(object: &Map<String, Value>, key: &str) -> Option<i64> {
  >     string(object, key)
  >         .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
  >         .map(|value| value.timestamp_millis())
  > }

- **Fact:** a `build_status` this build does not know deserializes as `JobStatus::Unknown`, which the handler skips (writers never store `unknown`).
  **Source:** `crates/gridops-core/src/gitlab/mod.rs:154-155`
  **Verbatim excerpt:**
  > #[serde(other)]
  > Unknown,

- **Fact:** `scope_kind` is only `project` or `group` and `scope_path` is never NULL, so the candidate query's two branches cover every connection.
  **Source:** `migrations/0047_gitlab.sql:29-31`
  **Verbatim excerpt:**
  > `scope_kind` text NOT NULL CHECK (`scope_kind` IN ('project','group')),
  > `scope_id` integer NOT NULL CHECK (`scope_id` > 0),
  > `scope_path` text NOT NULL,
