//! Scoped API tokens for the bot-friendly API.
//!
//! A token is another way to authenticate an existing user, not a new
//! principal: `authenticate` resolves the stored verifier to its owner and
//! scopes, and every registry transaction re-checks the owner's live role.
//! Only a SHA-256 verifier is stored; the plaintext exists once, in the
//! receipt returned by [`ApiTokenService::create`].

use std::fmt;
use std::sync::Arc;

use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use sqlx::Row as _;
use thiserror::Error;
use uuid::Uuid;

use crate::fleet::registry::{OneTimeSecret, RegistryClock, Verifier};

/// The plaintext of a freshly created token, shown to the caller exactly once.
///
/// Deliberately neither `Debug`-transparent nor `Serialize`: the management
/// route serializes the receipt's metadata itself and copies the secret into
/// exactly one response.
#[derive(Clone)]
pub struct TokenReceipt {
    token_id: String,
    name: String,
    scopes: Vec<ApiTokenScope>,
    expires_at: i64,
    created_at: i64,
    secret: SecretString,
}

impl TokenReceipt {
    #[must_use]
    pub fn token_id(&self) -> &str {
        &self.token_id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn scopes(&self) -> &[ApiTokenScope] {
        &self.scopes
    }

    #[must_use]
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }

    #[must_use]
    pub fn created_at(&self) -> i64 {
        self.created_at
    }

    /// Expose the plaintext only at the dedicated receipt boundary.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.secret.expose_secret()
    }
}

impl fmt::Debug for TokenReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokenReceipt(REDACTED)")
    }
}

/// A non-secret row as listed for its owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApiTokenRow {
    pub id: String,
    pub name: String,
    pub display_prefix: String,
    pub scopes: Vec<ApiTokenScope>,
    pub created_at: i64,
    pub expires_at: i64,
    pub last_used_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

/// What a successful authentication resolves to: the owning user and the
/// token's scopes, plus the identity the audit issue needs for `last_used_at`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedApiToken {
    pub token_id: String,
    pub user_id: String,
    pub name: String,
    pub scopes: Vec<ApiTokenScope>,
}

/// The token scopes. A route accepts a token only when it declares one of
/// these; anything else is cookie-only. No scope implies another. The
/// declaration order is the stored and returned order, so new scopes go last
/// and tokens created earlier keep exactly the order they were stored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApiTokenScope {
    #[serde(rename = "fleet:read")]
    FleetRead,
    #[serde(rename = "fleet:enroll")]
    FleetEnroll,
    #[serde(rename = "fleet:approve")]
    FleetApprove,
    #[serde(rename = "fleet:operate")]
    FleetOperate,
    /// Retire a machine, only when it cannot hurt a healthy fleet (see
    /// `RegistryService::retire_host_with_policy`). Not implied by
    /// `fleet:operate`.
    #[serde(rename = "fleet:retire")]
    FleetRetire,
    /// Read runner pools, their events and capacity, and the pool form's
    /// options, as the token's owner may.
    #[serde(rename = "pools:read")]
    PoolsRead,
    /// Create, edit and remove runner pools with the form's validation, as
    /// the token's owner may. Does not imply `pools:read`.
    #[serde(rename = "pools:manage")]
    PoolsManage,
}

impl ApiTokenScope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FleetRead => "fleet:read",
            Self::FleetEnroll => "fleet:enroll",
            Self::FleetApprove => "fleet:approve",
            Self::FleetOperate => "fleet:operate",
            Self::FleetRetire => "fleet:retire",
            Self::PoolsRead => "pools:read",
            Self::PoolsManage => "pools:manage",
        }
    }

    /// Parse the wire name, `fleet:read` and friends.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "fleet:read" => Some(Self::FleetRead),
            "fleet:enroll" => Some(Self::FleetEnroll),
            "fleet:approve" => Some(Self::FleetApprove),
            "fleet:operate" => Some(Self::FleetOperate),
            "fleet:retire" => Some(Self::FleetRetire),
            "pools:read" => Some(Self::PoolsRead),
            "pools:manage" => Some(Self::PoolsManage),
            _ => None,
        }
    }
}

impl fmt::Display for ApiTokenScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The outcome of an idempotent create: the fresh receipt carries the
/// plaintext exactly once; a replay carries only metadata.
#[derive(Debug)]
pub enum CreateOutcome {
    Created(TokenReceipt),
    Replay(ApiTokenRow),
}

/// Stable failures for the token service.
#[derive(Debug, Error)]
pub enum ApiTokenError {
    /// Authentication did not resolve to a live, unrevoked, unexpired token
    /// of an existing user. Deliberately covers every reason alike.
    #[error("authentication failed")]
    AuthenticationFailed,
    #[error("name must be 1-64 characters without control characters")]
    InvalidName,
    #[error("at least one scope is required")]
    NoScopes,
    #[error("unknown scope")]
    UnknownScope,
    #[error("expiry must be in the future and at most 365 days out")]
    InvalidExpiry,
    #[error("a non-revoked token with this name already exists for the user")]
    DuplicateName,
    #[error("the user already holds the maximum number of non-revoked tokens")]
    LimitReached,
    #[error("token not found")]
    NotFound,
    #[error("the acting user must be a system admin")]
    NotAdmin,
    #[error("the same idempotency key was used with a different request")]
    IdempotencyConflict,
    #[error("database error")]
    Database(#[from] sqlx::Error),
}

const TOKEN_PREFIX: &str = "gat_";
const MAX_NON_REVOKED_PER_USER: i64 = 50;
const MAX_NAME_CHARS: usize = 64;
const MAX_LIFETIME_MILLIS: i64 = 365 * 24 * 60 * 60 * 1000;

/// Generate a fresh `gat_`-prefixed token text: the 43-character
/// base64url body is the same shape as a host credential, so the two can
/// never collide, and the prefix keeps secret scanners happy.
fn generate_token_text() -> String {
    format!("{TOKEN_PREFIX}{}", OneTimeSecret::generate().expose())
}

fn parse_token_text(value: &str) -> Result<OneTimeSecret, ApiTokenError> {
    let body = value
        .strip_prefix(TOKEN_PREFIX)
        .ok_or(ApiTokenError::AuthenticationFailed)?;
    OneTimeSecret::parse(body).map_err(|_| ApiTokenError::AuthenticationFailed)
}

fn validate_name(name: &str) -> Result<(), ApiTokenError> {
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(ApiTokenError::InvalidName);
    }
    Ok(())
}

fn normalize_scopes(scopes: &[ApiTokenScope]) -> Result<Vec<ApiTokenScope>, ApiTokenError> {
    if scopes.is_empty() {
        return Err(ApiTokenError::NoScopes);
    }
    let mut sorted = scopes.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    Ok(sorted)
}

/// Storage and lookup for API tokens. No HTTP here; the API crate's
/// extractors and management routes call this service.
pub struct ApiTokenService {
    pool: sqlx::SqlitePool,
    clock: Arc<dyn RegistryClock>,
}

impl ApiTokenService {
    #[must_use]
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self {
            pool,
            clock: crate::fleet::registry::system_clock(),
        }
    }

    #[must_use]
    pub fn with_clock(pool: sqlx::SqlitePool, clock: Arc<dyn RegistryClock>) -> Self {
        Self { pool, clock }
    }

    fn validate_expiry(now: i64, expires_at: i64) -> Result<(), ApiTokenError> {
        if expires_at <= now || expires_at - now > MAX_LIFETIME_MILLIS {
            return Err(ApiTokenError::InvalidExpiry);
        }
        Ok(())
    }

    async fn enforce_capacity(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        user_id: &str,
    ) -> Result<(), ApiTokenError> {
        let live: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM api_tokens WHERE user_id=? AND revoked_at IS NULL",
        )
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await?;
        if live >= MAX_NON_REVOKED_PER_USER {
            return Err(ApiTokenError::LimitReached);
        }
        Ok(())
    }

    async fn enforce_unique_name(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        user_id: &str,
        name: &str,
    ) -> Result<(), ApiTokenError> {
        let duplicate: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM api_tokens
              WHERE user_id=? AND revoked_at IS NULL AND name=?",
        )
        .bind(user_id)
        .bind(name)
        .fetch_one(&mut **tx)
        .await?;
        if duplicate > 0 {
            return Err(ApiTokenError::DuplicateName);
        }
        Ok(())
    }

    /// Create a token for `user_id`. The plaintext is returned once, in the
    /// receipt; only its verifier is stored. `expires_at` is epoch
    /// milliseconds. No admin check: tests and internal tooling use this.
    pub async fn create(
        &self,
        user_id: &str,
        name: &str,
        scopes: &[ApiTokenScope],
        expires_at: i64,
    ) -> Result<TokenReceipt, ApiTokenError> {
        validate_name(name)?;
        let scopes = normalize_scopes(scopes)?;
        let now = self.clock.now_millis();
        Self::validate_expiry(now, expires_at)?;
        let token_id = Uuid::new_v4().to_string();
        let secret_text = generate_token_text();
        let verifier = Verifier::from_secret(&secret_text);
        let display_prefix = secret_text[TOKEN_PREFIX.len()..]
            .chars()
            .take(8)
            .collect::<String>();

        let mut tx = self.pool.begin().await?;
        Self::enforce_capacity(&mut tx, user_id).await?;
        Self::enforce_unique_name(&mut tx, user_id, name).await?;
        sqlx::query(
            "INSERT INTO api_tokens
               (id,user_id,name,verifier,display_prefix,scopes_json,created_at,expires_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&token_id)
        .bind(user_id)
        .bind(name)
        .bind(verifier.as_str())
        .bind(&display_prefix)
        .bind(&scopes_json(&scopes)?)
        .bind(now)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(TokenReceipt {
            token_id,
            name: name.to_owned(),
            scopes,
            expires_at,
            created_at: now,
            secret: SecretString::from(secret_text),
        })
    }

    /// The management-API create: requires the acting user to be a system
    /// admin, honours an idempotency key, and writes the `api_token.created`
    /// audit event in the same transaction as the insert. A replay of the
    /// same key with the same request returns the row's metadata only; the
    /// same key with a different request is a conflict.
    pub async fn create_admin(
        &self,
        actor_user_id: &str,
        name: &str,
        scopes: &[ApiTokenScope],
        expires_at: i64,
        idempotency_key: &str,
    ) -> Result<CreateOutcome, ApiTokenError> {
        validate_name(name)?;
        let scopes = normalize_scopes(scopes)?;
        let now = self.clock.now_millis();
        Self::validate_expiry(now, expires_at)?;

        let mut tx = self.pool.begin().await?;
        let role: Option<String> = sqlx::query_scalar("SELECT role FROM users WHERE id=?")
            .bind(actor_user_id)
            .fetch_optional(&mut *tx)
            .await?;
        if role.as_deref() != Some("admin") {
            return Err(ApiTokenError::NotAdmin);
        }
        if let Some(row) = sqlx::query(
            "SELECT id,name,display_prefix,scopes_json,created_at,expires_at,
                    last_used_at,revoked_at
               FROM api_tokens WHERE user_id=? AND idempotency_key=?",
        )
        .bind(actor_user_id)
        .bind(idempotency_key)
        .fetch_optional(&mut *tx)
        .await?
        {
            let replayed = row_from(&row)?;
            if replayed.name != name || replayed.scopes != scopes {
                return Err(ApiTokenError::IdempotencyConflict);
            }
            return Ok(CreateOutcome::Replay(replayed));
        }
        Self::enforce_capacity(&mut tx, actor_user_id).await?;
        Self::enforce_unique_name(&mut tx, actor_user_id, name).await?;

        let token_id = Uuid::new_v4().to_string();
        let secret_text = generate_token_text();
        let verifier = Verifier::from_secret(&secret_text);
        let display_prefix = secret_text[TOKEN_PREFIX.len()..]
            .chars()
            .take(8)
            .collect::<String>();
        sqlx::query(
            "INSERT INTO api_tokens
               (id,user_id,name,verifier,display_prefix,scopes_json,created_at,
                expires_at,idempotency_key)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&token_id)
        .bind(actor_user_id)
        .bind(name)
        .bind(verifier.as_str())
        .bind(&display_prefix)
        .bind(&scopes_json(&scopes)?)
        .bind(now)
        .bind(expires_at)
        .bind(idempotency_key)
        .execute(&mut *tx)
        .await?;
        audit_event(
            &mut tx,
            Some(actor_user_id),
            "api_token.created",
            &token_id,
            serde_json::json!({
                "name": name,
                "scopes": scopes.iter().map(|scope| scope.as_str()).collect::<Vec<_>>(),
                "expiresAt": expires_at,
            }),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(CreateOutcome::Created(TokenReceipt {
            token_id,
            name: name.to_owned(),
            scopes,
            expires_at,
            created_at: now,
            secret: SecretString::from(secret_text),
        }))
    }

    /// The management-API revoke: a user may revoke their own token, a
    /// system admin may revoke anyone's, and the `api_token.revoked` audit
    /// event lands in the same transaction. Revoking twice is a no-op.
    pub async fn revoke_admin(
        &self,
        actor_user_id: &str,
        token_id: &str,
    ) -> Result<ApiTokenRow, ApiTokenError> {
        let now = self.clock.now_millis();
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT id,user_id,name,display_prefix,scopes_json,created_at,expires_at,
                    last_used_at,revoked_at
               FROM api_tokens WHERE id=?",
        )
        .bind(token_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ApiTokenError::NotFound)?;
        let owner: String = row.get("user_id");
        if owner != actor_user_id {
            let role: Option<String> = sqlx::query_scalar("SELECT role FROM users WHERE id=?")
                .bind(actor_user_id)
                .fetch_optional(&mut *tx)
                .await?;
            if role.as_deref() != Some("admin") {
                return Err(ApiTokenError::NotAdmin);
            }
        }
        let revoked: Option<i64> = row.get("revoked_at");
        if revoked.is_none() {
            sqlx::query("UPDATE api_tokens SET revoked_at=? WHERE id=?")
                .bind(now)
                .bind(token_id)
                .execute(&mut *tx)
                .await?;
            audit_event(
                &mut tx,
                Some(actor_user_id),
                "api_token.revoked",
                token_id,
                serde_json::json!({"name": row.get::<String, _>("name")}),
                now,
            )
            .await?;
        }
        tx.commit().await?;
        let row = sqlx::query(
            "SELECT id,name,display_prefix,scopes_json,created_at,expires_at,
                    last_used_at,revoked_at
               FROM api_tokens WHERE id=?",
        )
        .bind(token_id)
        .fetch_one(&self.pool)
        .await?;
        row_from(&row)
    }

    /// The owner's tokens, newest first, never including a verifier.
    pub async fn list_for_user(&self, user_id: &str) -> Result<Vec<ApiTokenRow>, ApiTokenError> {
        let rows = sqlx::query(
            "SELECT id,name,display_prefix,scopes_json,created_at,expires_at,
                    last_used_at,revoked_at
               FROM api_tokens WHERE user_id=? ORDER BY created_at DESC,id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_from).collect()
    }

    /// Revoke a token. Revoking twice is a no-op; an unknown token id is an
    /// error. The next `authenticate` with its plaintext fails.
    pub async fn revoke(&self, user_id: &str, token_id: &str) -> Result<(), ApiTokenError> {
        let now = self.clock.now_millis();
        let result = sqlx::query(
            "UPDATE api_tokens SET revoked_at=?
              WHERE id=? AND user_id=? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(token_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            let known: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_tokens WHERE id=?")
                .bind(token_id)
                .fetch_one(&self.pool)
                .await?;
            if known == 0 {
                return Err(ApiTokenError::NotFound);
            }
        }
        Ok(())
    }

    /// Resolve a bearer token to its owner and scopes. A parse failure, an
    /// unknown, expired or revoked token, and a deleted owner all return the
    /// same error, so a caller cannot tell them apart. This does not write;
    /// `last_used_at` is the audit issue's job.
    pub async fn authenticate(
        &self,
        token_text: &str,
    ) -> Result<AuthenticatedApiToken, ApiTokenError> {
        parse_token_text(token_text)?;
        let verifier = Verifier::from_secret(token_text);
        let now = self.clock.now_millis();
        let row = sqlx::query(
            "SELECT t.id,t.user_id,t.name,t.scopes_json,t.expires_at,t.revoked_at
               FROM api_tokens t
               JOIN users u ON u.id=t.user_id
              WHERE t.verifier=?",
        )
        .bind(verifier.as_str())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(ApiTokenError::AuthenticationFailed)?;
        let revoked_at: Option<i64> = row.get("revoked_at");
        let expires_at: i64 = row.get("expires_at");
        if revoked_at.is_some() || expires_at <= now {
            return Err(ApiTokenError::AuthenticationFailed);
        }
        let scopes_json: String = row.get("scopes_json");
        let scopes = serde_json::from_str::<Vec<&str>>(&scopes_json)
            .map_err(|_| ApiTokenError::AuthenticationFailed)?
            .iter()
            .map(|scope| ApiTokenScope::parse(scope).ok_or(ApiTokenError::AuthenticationFailed))
            .collect::<Result<Vec<_>, _>>()?;
        if scopes.is_empty() {
            return Err(ApiTokenError::AuthenticationFailed);
        }
        Ok(AuthenticatedApiToken {
            token_id: row.get("id"),
            user_id: row.get("user_id"),
            name: row.get("name"),
            scopes,
        })
    }

    /// Record a successful use. Throttled: a write only when the stored
    /// `last_used_at` is missing or older than a minute.
    pub async fn record_use(&self, token_id: &str) -> Result<(), ApiTokenError> {
        let now = self.clock.now_millis();
        sqlx::query(
            "UPDATE api_tokens SET last_used_at=?
              WHERE id=? AND revoked_at IS NULL
                AND (last_used_at IS NULL OR last_used_at <= ?)",
        )
        .bind(now)
        .bind(token_id)
        .bind(now - 60_000)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Record a scope denial, at most one row per token per minute so a
    /// misbehaving bot cannot flood the audit table. Never blocks the
    /// refusal: a write failure is swallowed after logging.
    pub async fn record_denial(
        &self,
        token_id: &str,
        actor_user_id: Option<&str>,
        method: &str,
        route: &str,
        required_scope: &str,
    ) -> Result<(), ApiTokenError> {
        let now = self.clock.now_millis();
        let mut tx = self.pool.begin().await?;
        let recent: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM audit_events
              WHERE action='api_token.denied' AND target_id=? AND created_at > ?",
        )
        .bind(token_id)
        .bind(now - 60_000)
        .fetch_one(&mut *tx)
        .await?;
        if recent == 0 {
            sqlx::query(
                "INSERT INTO audit_events
                 (id,actor_user_id,actor_label,action,target_type,target_id,metadata,created_at)
                 VALUES (?,?,?,?,?,?,?,?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(actor_user_id)
            .bind(actor_user_id.unwrap_or("fleet-agent"))
            .bind("api_token.denied")
            .bind("api_token")
            .bind(token_id)
            .bind(serde_json::to_string(&serde_json::json!({
                "method": method,
                "route": route,
                "requiredScope": required_scope,
            }))?)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
        }
        Ok(())
    }
}

impl From<serde_json::Error> for ApiTokenError {
    fn from(_: serde_json::Error) -> Self {
        Self::Database(sqlx::Error::Decode("json encoding failed".into()))
    }
}

fn scopes_json(scopes: &[ApiTokenScope]) -> Result<String, ApiTokenError> {
    Ok(serde_json::to_string(
        &scopes
            .iter()
            .map(|scope| scope.as_str())
            .collect::<Vec<_>>(),
    )?)
}

fn row_from(row: &sqlx::sqlite::SqliteRow) -> Result<ApiTokenRow, ApiTokenError> {
    let scopes_json: String = row.get("scopes_json");
    let scopes = serde_json::from_str::<Vec<&str>>(&scopes_json)
        .map_err(|_| {
            ApiTokenError::Database(sqlx::Error::Decode("scopes_json is malformed".into()))
        })?
        .iter()
        .map(|scope| ApiTokenScope::parse(scope).ok_or(ApiTokenError::UnknownScope))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ApiTokenRow {
        id: row.get("id"),
        name: row.get("name"),
        display_prefix: row.get("display_prefix"),
        scopes,
        created_at: row.get("created_at"),
        expires_at: row.get("expires_at"),
        last_used_at: row.get("last_used_at"),
        revoked_at: row.get("revoked_at"),
    })
}

/// The audit insert, in the caller's transaction, mirroring the registry's
/// `audit_and_event` without the host-scoped fleet event. Metadata never
/// carries the token or its verifier.
async fn audit_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor_user_id: Option<&str>,
    action: &str,
    token_id: &str,
    metadata: serde_json::Value,
    now: i64,
) -> Result<(), ApiTokenError> {
    sqlx::query(
        "INSERT INTO audit_events
         (id,actor_user_id,actor_label,action,target_type,target_id,metadata,created_at)
         VALUES (?,?,?,?,?,?,?,?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(actor_user_id)
    .bind(actor_user_id.unwrap_or("fleet-agent"))
    .bind(action)
    .bind("api_token")
    .bind(token_id)
    .bind(serde_json::to_string(&metadata)?)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_strings_round_trip() -> anyhow::Result<()> {
        for scope in [
            ApiTokenScope::FleetRead,
            ApiTokenScope::FleetEnroll,
            ApiTokenScope::FleetApprove,
            ApiTokenScope::FleetOperate,
            ApiTokenScope::FleetRetire,
            ApiTokenScope::PoolsRead,
            ApiTokenScope::PoolsManage,
        ] {
            assert_eq!(
                serde_json::to_string(&scope)?,
                format!("\"{}\"", scope.as_str())
            );
            assert_eq!(
                ApiTokenScope::parse(scope.as_str()),
                Some(scope),
                "{}",
                scope.as_str()
            );
        }
        assert_eq!(ApiTokenScope::FleetRetire.as_str(), "fleet:retire");
        assert_eq!(ApiTokenScope::PoolsRead.as_str(), "pools:read");
        assert_eq!(ApiTokenScope::PoolsManage.as_str(), "pools:manage");
        assert!(ApiTokenScope::parse("fleet:admin").is_none());
        assert!(ApiTokenScope::parse("pools:write").is_none());
        assert!(ApiTokenScope::parse("pools").is_none());
        assert!(serde_json::from_str::<ApiTokenScope>("\"tokens\"").is_err());
        Ok(())
    }

    /// Stored and returned scopes are sorted by this order, so existing
    /// tokens keep exactly the order they were stored in, and the web form
    /// lists the scopes the same way.
    #[test]
    fn new_scopes_sort_after_the_v1_scopes_and_imply_nothing() -> anyhow::Result<()> {
        let normalized = normalize_scopes(&[
            ApiTokenScope::PoolsManage,
            ApiTokenScope::FleetOperate,
            ApiTokenScope::PoolsRead,
            ApiTokenScope::FleetRetire,
            ApiTokenScope::FleetRead,
        ])?;
        assert_eq!(
            normalized,
            [
                ApiTokenScope::FleetRead,
                ApiTokenScope::FleetOperate,
                ApiTokenScope::FleetRetire,
                ApiTokenScope::PoolsRead,
                ApiTokenScope::PoolsManage,
            ]
        );
        // No scope widens into another: a v1 token holds only what it lists.
        let v1 = normalize_scopes(&[ApiTokenScope::FleetOperate])?;
        for scope in [
            ApiTokenScope::FleetRetire,
            ApiTokenScope::PoolsRead,
            ApiTokenScope::PoolsManage,
        ] {
            assert!(!v1.contains(&scope));
        }
        Ok(())
    }

    #[test]
    fn names_are_validated() {
        assert!(validate_name("ci-bot").is_ok());
        assert!(validate_name(&"x".repeat(64)).is_ok());
        assert!(matches!(validate_name(""), Err(ApiTokenError::InvalidName)));
        assert!(matches!(
            validate_name(&"x".repeat(65)),
            Err(ApiTokenError::InvalidName)
        ));
        assert!(matches!(
            validate_name("bad\u{7}name"),
            Err(ApiTokenError::InvalidName)
        ));
    }

    #[test]
    fn generated_token_has_the_documented_shape() -> anyhow::Result<()> {
        let text = generate_token_text();
        assert!(text.starts_with("gat_"));
        assert_eq!(text.len(), TOKEN_PREFIX.len() + 43);
        parse_token_text(&text)?;
        assert!(matches!(
            parse_token_text(&text.replace("gat_", "")),
            Err(ApiTokenError::AuthenticationFailed)
        ));
        assert!(matches!(
            parse_token_text("gat_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            Err(ApiTokenError::AuthenticationFailed)
        ));
        Ok(())
    }

    #[test]
    fn receipt_is_redacted_in_debug() {
        let text = generate_token_text();
        assert_eq!(
            format!(
                "{:?}",
                TokenReceipt {
                    token_id: Uuid::new_v4().to_string(),
                    name: "bot".into(),
                    scopes: vec![ApiTokenScope::FleetRead],
                    expires_at: 1,
                    created_at: 0,
                    secret: SecretString::from(text),
                }
            ),
            "TokenReceipt(REDACTED)"
        );
    }
}
