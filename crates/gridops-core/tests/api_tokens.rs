//! Integration tests for API token storage against a real file-backed
//! `SQLite` database, following `fleet_registry.rs`.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::Result;
use gridops_core::api_tokens::{ApiTokenError, ApiTokenScope, ApiTokenService};
use gridops_core::connect_database_path;
use gridops_core::fleet::registry::RegistryClock;
use sqlx::Row as _;
use uuid::Uuid;

struct FakeClock(AtomicI64);

impl RegistryClock for FakeClock {
    fn now_millis(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct Fixture {
    pool: sqlx::SqlitePool,
    service: ApiTokenService,
    clock: Arc<FakeClock>,
    user_id: String,
}

impl Fixture {
    async fn new() -> Result<Self> {
        let directory = std::env::temp_dir().join(format!("gridops-tokens-{}", Uuid::new_v4()));
        let pool = connect_database_path(&directory.join("tokens.sqlite")).await?;
        let clock = Arc::new(FakeClock(AtomicI64::new(1_700_000_000_000)));
        let user_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO users (id,github_id,login,access_token,last_login_at,created_at,updated_at)
             VALUES (?, 1, 'admin', 'x', 0, 0, 0)",
        )
        .bind(&user_id)
        .execute(&pool)
        .await?;
        let service = ApiTokenService::with_clock(pool.clone(), clock.clone());
        Ok(Self {
            pool,
            service,
            clock,
            user_id,
        })
    }

    fn advance(&self, millis: i64) {
        self.clock.0.fetch_add(millis, Ordering::SeqCst);
    }
}

const DAY: i64 = 24 * 60 * 60 * 1000;

#[tokio::test]
async fn create_then_authenticate_round_trips() -> Result<()> {
    let fixture = Fixture::new().await?;
    let receipt = fixture
        .service
        .create(
            &fixture.user_id,
            "ci-bot",
            &[ApiTokenScope::FleetRead, ApiTokenScope::FleetApprove],
            fixture.clock.now_millis() + 90 * DAY,
        )
        .await?;
    assert_eq!(receipt.name(), "ci-bot");
    assert_eq!(
        receipt.scopes(),
        &[ApiTokenScope::FleetRead, ApiTokenScope::FleetApprove]
    );
    assert!(receipt.expose().starts_with("gat_"));

    let authenticated = fixture.service.authenticate(receipt.expose()).await?;
    assert_eq!(authenticated.user_id, fixture.user_id);
    assert_eq!(authenticated.token_id, receipt.token_id());
    assert_eq!(authenticated.name, "ci-bot");
    assert_eq!(
        authenticated.scopes,
        vec![ApiTokenScope::FleetRead, ApiTokenScope::FleetApprove]
    );

    // Only the SHA-256 verifier is stored; the plaintext itself appears
    // nowhere in the row.
    let verifier: String = sqlx::query_scalar("SELECT verifier FROM api_tokens WHERE id=?")
        .bind(receipt.token_id())
        .fetch_one(&fixture.pool)
        .await?;
    assert_ne!(verifier, receipt.expose());
    assert_eq!(verifier.len(), 43);
    Ok(())
}

#[tokio::test]
async fn malformed_and_foreign_credentials_fail_identically() -> Result<()> {
    let fixture = Fixture::new().await?;
    let receipt = fixture
        .service
        .create(
            &fixture.user_id,
            "bot",
            &[ApiTokenScope::FleetRead],
            fixture.clock.now_millis() + DAY,
        )
        .await?;

    let truncated = &receipt.expose()[..receipt.expose().len() - 1];
    let no_prefix = receipt.expose().trim_start_matches("gat_");
    let host_credential = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    for candidate in ["wrong", truncated, no_prefix, host_credential, "gat_", ""] {
        assert!(
            matches!(
                fixture.service.authenticate(candidate).await,
                Err(ApiTokenError::AuthenticationFailed)
            ),
            "{candidate} should not authenticate"
        );
    }
    Ok(())
}

#[tokio::test]
async fn expired_and_revoked_tokens_fail_and_double_revoke_is_a_noop() -> Result<()> {
    let fixture = Fixture::new().await?;
    let receipt = fixture
        .service
        .create(
            &fixture.user_id,
            "short-lived",
            &[ApiTokenScope::FleetEnroll],
            fixture.clock.now_millis() + DAY,
        )
        .await?;

    fixture
        .service
        .revoke(&fixture.user_id, receipt.token_id())
        .await?;
    assert!(matches!(
        fixture.service.authenticate(receipt.expose()).await,
        Err(ApiTokenError::AuthenticationFailed)
    ));
    // Revoking twice is a no-op, not an error.
    fixture
        .service
        .revoke(&fixture.user_id, receipt.token_id())
        .await?;
    assert!(matches!(
        fixture
            .service
            .revoke(&fixture.user_id, "00000000-0000-4000-8000-000000000000")
            .await,
        Err(ApiTokenError::NotFound)
    ));

    let expiring = fixture
        .service
        .create(
            &fixture.user_id,
            "expiring",
            &[ApiTokenScope::FleetRead],
            fixture.clock.now_millis() + DAY,
        )
        .await?;
    fixture.advance(DAY + 1);
    assert!(matches!(
        fixture.service.authenticate(expiring.expose()).await,
        Err(ApiTokenError::AuthenticationFailed)
    ));
    Ok(())
}

#[tokio::test]
async fn deleting_the_owner_removes_their_tokens() -> Result<()> {
    let fixture = Fixture::new().await?;
    let receipt = fixture
        .service
        .create(
            &fixture.user_id,
            "doomed",
            &[ApiTokenScope::FleetRead],
            fixture.clock.now_millis() + DAY,
        )
        .await?;
    sqlx::query("DELETE FROM users WHERE id=?")
        .bind(&fixture.user_id)
        .execute(&fixture.pool)
        .await?;
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_tokens WHERE user_id=?")
        .bind(&fixture.user_id)
        .fetch_one(&fixture.pool)
        .await?;
    assert_eq!(remaining, 0);
    assert!(matches!(
        fixture.service.authenticate(receipt.expose()).await,
        Err(ApiTokenError::AuthenticationFailed)
    ));
    Ok(())
}

#[tokio::test]
async fn creation_rules_are_enforced() -> Result<()> {
    let fixture = Fixture::new().await?;
    let now = fixture.clock.now_millis();

    assert!(matches!(
        fixture
            .service
            .create(&fixture.user_id, "", &[ApiTokenScope::FleetRead], now + DAY)
            .await,
        Err(ApiTokenError::InvalidName)
    ));
    assert!(matches!(
        fixture
            .service
            .create(&fixture.user_id, "ok", &[], now + DAY)
            .await,
        Err(ApiTokenError::NoScopes)
    ));
    assert!(matches!(
        fixture
            .service
            .create(&fixture.user_id, "ok", &[ApiTokenScope::FleetRead], now)
            .await,
        Err(ApiTokenError::InvalidExpiry)
    ));
    assert!(matches!(
        fixture
            .service
            .create(
                &fixture.user_id,
                "ok",
                &[ApiTokenScope::FleetRead],
                now + 366 * DAY
            )
            .await,
        Err(ApiTokenError::InvalidExpiry)
    ));

    // Duplicate name among non-revoked tokens...
    fixture
        .service
        .create(
            &fixture.user_id,
            "same",
            &[ApiTokenScope::FleetRead],
            now + DAY,
        )
        .await?;
    assert!(matches!(
        fixture
            .service
            .create(
                &fixture.user_id,
                "same",
                &[ApiTokenScope::FleetEnroll],
                now + DAY
            )
            .await,
        Err(ApiTokenError::DuplicateName)
    ));
    // ...but the name frees up once revoked, for the same or another user.
    let listed = fixture.service.list_for_user(&fixture.user_id).await?;
    let Some(first) = listed.iter().find(|row| row.name == "same") else {
        anyhow::bail!("token not listed");
    };
    fixture.service.revoke(&fixture.user_id, &first.id).await?;
    fixture
        .service
        .create(
            &fixture.user_id,
            "same",
            &[ApiTokenScope::FleetOperate],
            now + DAY,
        )
        .await?;
    Ok(())
}

#[tokio::test]
async fn non_revoked_token_cap_is_enforced() -> Result<()> {
    let fixture = Fixture::new().await?;
    let now = fixture.clock.now_millis();
    for index in 0..50 {
        fixture
            .service
            .create(
                &fixture.user_id,
                &format!("bot-{index}"),
                &[ApiTokenScope::FleetRead],
                now + DAY,
            )
            .await?;
    }
    assert!(matches!(
        fixture
            .service
            .create(
                &fixture.user_id,
                "one-too-many",
                &[ApiTokenScope::FleetRead],
                now + DAY
            )
            .await,
        Err(ApiTokenError::LimitReached)
    ));
    let listed = fixture.service.list_for_user(&fixture.user_id).await?;
    assert_eq!(listed.len(), 50);
    fixture
        .service
        .revoke(&fixture.user_id, &listed[0].id)
        .await?;
    fixture
        .service
        .create(
            &fixture.user_id,
            "one-more",
            &[ApiTokenScope::FleetRead],
            now + DAY,
        )
        .await?;
    Ok(())
}

#[tokio::test]
async fn list_never_returns_the_verifier_and_record_use_throttles() -> Result<()> {
    let fixture = Fixture::new().await?;
    let receipt = fixture
        .service
        .create(
            &fixture.user_id,
            "listed",
            &[ApiTokenScope::FleetRead, ApiTokenScope::FleetOperate],
            fixture.clock.now_millis() + DAY,
        )
        .await?;
    let listed = fixture.service.list_for_user(&fixture.user_id).await?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].display_prefix.len(), 8);
    assert_eq!(listed[0].last_used_at, None);
    let serialized = serde_json::to_string(&listed[0])?;
    assert!(!serialized.contains("verifier"));
    assert!(!serialized.contains(receipt.expose()));

    fixture.service.record_use(receipt.token_id()).await?;
    let used = fixture.service.list_for_user(&fixture.user_id).await?;
    assert_eq!(used[0].last_used_at, Some(fixture.clock.now_millis()));

    // A second use within a minute does not move `last_used_at`.
    fixture.advance(30_000);
    fixture.service.record_use(receipt.token_id()).await?;
    let again = fixture.service.list_for_user(&fixture.user_id).await?;
    assert_eq!(again[0].last_used_at, used[0].last_used_at);

    // A use after a minute does.
    fixture.advance(31_000);
    fixture.service.record_use(receipt.token_id()).await?;
    let later = fixture.service.list_for_user(&fixture.user_id).await?;
    assert_eq!(later[0].last_used_at, Some(fixture.clock.now_millis()));
    let row = sqlx::query("SELECT scopes_json FROM api_tokens WHERE id=?")
        .bind(receipt.token_id())
        .fetch_one(&fixture.pool)
        .await?;
    assert_eq!(
        row.get::<String, _>("scopes_json"),
        "[\"fleet:read\",\"fleet:operate\"]"
    );
    Ok(())
}

/// A token stored before the pool and retire scopes existed keeps exactly
/// its scopes: nothing is added on read, and `fleet:operate` never implies
/// `fleet:retire` or a pool scope.
#[tokio::test]
async fn existing_tokens_keep_their_scopes_and_new_scopes_round_trip() -> Result<()> {
    let fixture = Fixture::new().await?;
    let legacy = fixture
        .service
        .create(
            &fixture.user_id,
            "release-operator",
            &[ApiTokenScope::FleetRead, ApiTokenScope::FleetOperate],
            fixture.clock.now_millis() + DAY,
        )
        .await?;
    let authenticated = fixture.service.authenticate(legacy.expose()).await?;
    assert_eq!(
        authenticated.scopes,
        vec![ApiTokenScope::FleetRead, ApiTokenScope::FleetOperate]
    );
    for scope in [
        ApiTokenScope::FleetRetire,
        ApiTokenScope::PoolsRead,
        ApiTokenScope::PoolsManage,
    ] {
        assert!(!authenticated.scopes.contains(&scope), "{scope}");
    }

    let receipt = fixture
        .service
        .create(
            &fixture.user_id,
            "pool-bot",
            &[
                ApiTokenScope::PoolsManage,
                ApiTokenScope::FleetRetire,
                ApiTokenScope::PoolsRead,
            ],
            fixture.clock.now_millis() + DAY,
        )
        .await?;
    let row = sqlx::query("SELECT scopes_json FROM api_tokens WHERE id=?")
        .bind(receipt.token_id())
        .fetch_one(&fixture.pool)
        .await?;
    assert_eq!(
        row.get::<String, _>("scopes_json"),
        "[\"fleet:retire\",\"pools:read\",\"pools:manage\"]"
    );
    let authenticated = fixture.service.authenticate(receipt.expose()).await?;
    assert_eq!(
        authenticated.scopes,
        vec![
            ApiTokenScope::FleetRetire,
            ApiTokenScope::PoolsRead,
            ApiTokenScope::PoolsManage,
        ]
    );
    Ok(())
}

/// A row naming a scope this build does not know fails closed: the token
/// does not authenticate at all rather than authenticating with less.
#[tokio::test]
async fn a_stored_scope_this_build_does_not_know_fails_closed() -> Result<()> {
    let fixture = Fixture::new().await?;
    let receipt = fixture
        .service
        .create(
            &fixture.user_id,
            "future",
            &[ApiTokenScope::PoolsRead],
            fixture.clock.now_millis() + DAY,
        )
        .await?;
    sqlx::query("UPDATE api_tokens SET scopes_json='[\"pools:read\",\"pools:admin\"]' WHERE id=?")
        .bind(receipt.token_id())
        .execute(&fixture.pool)
        .await?;
    assert!(matches!(
        fixture.service.authenticate(receipt.expose()).await,
        Err(ApiTokenError::AuthenticationFailed)
    ));
    Ok(())
}
