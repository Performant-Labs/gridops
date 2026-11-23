//! #197: the harness the fleet health HTTP tests share (#197's own wiring
//! tests and the #194, #199 and #201 contract tests).
//!
//! Every harness has its own migrated database, an administrator (`admin-1`)
//! and a member (`member-1`) with sessions, and the real routers driven with
//! `tower::ServiceExt::oneshot`. The health state gets the fake source and a
//! clock the test moves, through `AppState::with_fleet_health_source` and
//! `AppState::with_fleet_health_clock`, set at `fake::FIXTURE_NOW`.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

use anyhow::{Context as _, Result};
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use gridops_core::{
    BitbucketClient, Config, GitHubClient, Vault,
    fleet::registry::RegistryClock,
    fleet_health::fake::{self, FakeFleetHealthSource},
};
use serde_json::Value;
use sqlx::SqlitePool;
use tower::ServiceExt as _;
use url::Url;

use crate::state::AppState;

pub(super) const PUBLIC: &str = "https://gridops.example";
pub(super) const HEALTH: &str = "/api/v1/fleet/health";

/// A clock the test sets; the health cache reads it.
#[derive(Debug)]
pub(super) struct TestClock(AtomicI64);

impl TestClock {
    #[allow(
        dead_code,
        reason = "the #194 contract tests move the clock once unmarked"
    )]
    pub(super) fn set(&self, now: i64) {
        self.0.store(now, Ordering::SeqCst);
    }

    pub(super) fn advance(&self, millis: i64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }
}

impl RegistryClock for TestClock {
    fn now_millis(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub(super) struct Harness {
    pub(super) router: Router,
    pub(super) database: SqlitePool,
    pub(super) source: FakeFleetHealthSource,
    pub(super) clock: Arc<TestClock>,
    pub(super) admin: String,
    pub(super) member: String,
    vault: Vault,
    path: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        for suffix in ["", "-shm", "-wal"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}

impl Harness {
    pub(super) async fn new() -> Result<Self> {
        let path =
            std::env::temp_dir().join(format!("gridops-health-{}.sqlite", uuid::Uuid::new_v4()));
        let database = gridops_core::connect_database_path(&path).await?;
        let config = Config::from_env()?
            .with_base_url(Url::parse(PUBLIC)?)
            .with_manager(Url::parse("http://127.0.0.1:9")?, "test-manager-credential");
        let vault = Vault::new([9_u8; 32], b"health-test-session-secret".to_vec());
        let source = FakeFleetHealthSource::default();
        let clock = Arc::new(TestClock(AtomicI64::new(fake::FIXTURE_NOW)));
        let state = AppState::new(
            config.clone(),
            database.clone(),
            vault.clone(),
            GitHubClient::new(config)?,
            BitbucketClient::new()?,
        )?
        .with_fleet_health_source(Arc::new(source.clone()))
        .with_fleet_health_clock(clock.clone());
        let router = super::router()
            .merge(crate::session_routes())
            .merge(crate::token_routes())
            .with_state(state);
        let mut harness = Self {
            router,
            database,
            source,
            clock,
            admin: String::new(),
            member: String::new(),
            vault,
            path,
        };
        harness.admin = harness.sign_in("admin-1", 1, "admin").await?;
        harness.member = harness.sign_in("member-1", 2, "member").await?;
        Ok(harness)
    }

    /// Seeds a user with a session; returns the session cookie.
    pub(super) async fn sign_in(
        &self,
        user_id: &str,
        github_id: i64,
        role: &str,
    ) -> Result<String> {
        let now = gridops_core::now_millis();
        sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES (?,?,?,?,'',?,?,?)")
            .bind(user_id).bind(github_id).bind(user_id).bind(role).bind(now).bind(now).bind(now)
            .execute(&self.database).await?;
        let session = format!("health-session-{user_id}");
        sqlx::query("INSERT INTO sessions (id,token_hash,user_id,expires_at,last_seen_at,created_at) VALUES (?,?,?,?,?,?)")
            .bind(format!("session-row-{user_id}"))
            .bind(gridops_core::crypto::hash_token(&session))
            .bind(user_id)
            .bind(now + 3_600_000)
            .bind(now)
            .bind(now)
            .execute(&self.database)
            .await?;
        Ok(format!(
            "gridops_session={session}.{}",
            self.vault.sign(&session)?
        ))
    }

    /// Sends a same-origin request; returns the status and the JSON body
    /// (`Value::Null` when empty, a string when not JSON).
    pub(super) async fn send(
        &self,
        method: Method,
        uri: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> Result<(StatusCode, Value)> {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::ORIGIN, PUBLIC);
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let request = match body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))?,
            None => request.body(Body::empty())?,
        };
        let response = self.router.clone().oneshot(request).await?;
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024).await?;
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        Ok((status, body))
    }

    pub(super) async fn get_as(
        &self,
        uri: &str,
        cookie: Option<&str>,
    ) -> Result<(StatusCode, Value)> {
        self.send(Method::GET, uri, cookie, None).await
    }

    pub(super) async fn admin_get(&self, uri: &str) -> Result<(StatusCode, Value)> {
        let admin = self.admin.clone();
        self.get_as(uri, Some(&admin)).await
    }
}

/// `fixture.json`, parsed.
pub(super) fn fixture() -> Result<Value> {
    serde_json::from_str(include_str!(
        "../../../../src/features/fleet-health/fixture.json"
    ))
    .context("fixture.json")
}

/// `docs/openapi/gridops-v1.json`, parsed.
pub(super) fn openapi() -> Result<Value> {
    serde_json::from_str(include_str!("../../../../docs/openapi/gridops-v1.json"))
        .context("gridops-v1.json")
}

/// The fleet error envelope's `code`, if the body is one.
pub(super) fn error_code(body: &Value) -> Option<&str> {
    body.pointer("/error/code")
        .or_else(|| body.get("code"))
        .and_then(Value::as_str)
}
