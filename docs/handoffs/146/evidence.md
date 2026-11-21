# Evidence: #146 GitLab HTTP client

## F (Phase 6)

- **Fact:** `ExternalId` deserializes through `TryFrom<u64>`, so a JSON `id` that is a string, 0, or above `i64::MAX` is a decode error. `http.rs` maps every decode error to a fixed `Upstream` message.
  **Source:** `crates/gridops-core/src/fleet/domain.rs:15-17`
  **Verbatim excerpt:**
  > #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
  > #[serde(try_from = "u64", into = "u64")]
  > pub struct ExternalId(NonZeroU64);

- **Fact:** `ExternalId`'s `TryFrom<u64>` rejects 0 and anything above `i64::MAX`.
  **Source:** `crates/gridops-core/src/fleet/domain.rs:40-46`
  **Verbatim excerpt:**
  > fn try_from(value: u64) -> Result<Self, Self::Error> {
  >     if value == 0 || value > i64::MAX as u64 {
  >         return Err(ExternalIdError {
  >             value,
  >             max: i64::MAX as u64,
  >         });
  >     }

- **Fact:** A job status GitLab adds later decodes as `JobStatus::Unknown` through `#[serde(other)]`. `http.rs` decodes `status` straight into `JobStatus` and relies on this.
  **Source:** `crates/gridops-core/src/gitlab/mod.rs:144-149`
  **Verbatim excerpt:**
  >     Skipped,
  >     Manual,
  >     Scheduled,
  >     #[serde(other)]
  >     Unknown,
  > }

- **Fact:** `GitLabCredentials`' `Debug` redacts the access token. `GitLabClient` holds no credentials, so its derived `Debug` prints no token.
  **Source:** `crates/gridops-core/src/gitlab/mod.rs:30-36`
  **Verbatim excerpt:**
  > impl fmt::Debug for GitLabCredentials {
  >     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
  >         f.debug_struct("GitLabCredentials")
  >             .field("base_url", &self.base_url.as_str())
  >             .field("access_token", &"[redacted]")
  >             .finish()
  >     }

- **Fact:** `CreatedRunner`'s `Debug` redacts the `glrt-` runner token that `create_runner` returns.
  **Source:** `crates/gridops-core/src/gitlab/mod.rs:111-117`
  **Verbatim excerpt:**
  > impl fmt::Debug for CreatedRunner {
  >     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
  >         f.debug_struct("CreatedRunner")
  >             .field("id", &self.id)
  >             .field("token", &"[redacted]")
  >             .finish()
  >     }

- **Fact:** `GitLabError::Upstream` displays as `GitLab request failed: <message>`, so its text is exactly the fixed message `http.rs` builds.
  **Source:** `crates/gridops-core/src/gitlab/mod.rs:220-221`
  **Verbatim excerpt:**
  >     #[error("GitLab request failed: {0}")]
  >     Upstream(String),
