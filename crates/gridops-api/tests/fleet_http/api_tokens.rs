//! Exercises scoped API-token authentication through the actual API binary.
//! Tokens must reach exactly the routes the v1 map names, through the same
//! handlers as cookies, and fail closed everywhere else.

use super::inventory::*;
use super::*;
use gridops_core::api_tokens::{ApiTokenScope, ApiTokenService};

const DAY: i64 = 86_400_000;

async fn mint(f: &Fixture, name: &str, scopes: &[ApiTokenScope]) -> Result<String> {
    let receipt = ApiTokenService::new(f.database.clone())
        .create("fleet-user", name, scopes, gridops_core::now_millis() + DAY)
        .await?;
    Ok(receipt.expose().to_owned())
}

fn token_get(f: &Fixture, path: &str, token: &str) -> reqwest::RequestBuilder {
    f.client.get(f.url(path)).bearer_auth(token)
}

fn token_post(f: &Fixture, path: &str, token: &str) -> reqwest::RequestBuilder {
    f.client.post(f.url(path)).bearer_auth(token)
}

#[tokio::test]
async fn reads_with_a_fleet_read_token_match_cookie_responses() -> Result<()> {
    let f = Fixture::new().await?;
    let (host, _secret) = enrolled(&f).await?;
    let token = mint(&f, "reader", &[ApiTokenScope::FleetRead]).await?;

    let cookie_hosts: Value = f
        .client
        .get(f.url("/hosts"))
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?
        .json()
        .await?;
    let token_hosts: Value = token_get(&f, "/hosts", &token).send().await?.json().await?;
    assert_eq!(cookie_hosts["items"], token_hosts["items"]);
    assert_eq!(cookie_hosts["schemaVersion"], token_hosts["schemaVersion"]);
    assert_eq!(token_hosts["items"][0]["id"], host);

    let targets_cookie: Value = f
        .client
        .get(f.url("/targets"))
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?
        .json()
        .await?;
    let targets_token: Value = token_get(&f, "/targets", &token)
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(targets_cookie, targets_token);
    Ok(())
}

#[tokio::test]
async fn enrollment_issue_with_a_token_keeps_idempotency() -> Result<()> {
    let f = Fixture::new().await?;
    let token = mint(&f, "issuer", &[ApiTokenScope::FleetEnroll]).await?;

    let first = token_post(&f, "/enrollments", &token)
        .header("idempotency-key", "token-issue-1")
        .json(&json!({"targetIds":[]}))
        .send()
        .await?;
    assert_eq!(first.status(), StatusCode::CREATED);
    assert_eq!(first.headers()[header::CACHE_CONTROL], "no-store");
    let created: Value = first.json().await?;
    assert!(
        created["code"]
            .as_str()
            .is_some_and(|code| !code.is_empty())
    );

    // A replay with the same key returns the enrollment metadata only.
    let replay = token_post(&f, "/enrollments", &token)
        .header("idempotency-key", "token-issue-1")
        .json(&json!({"targetIds":[]}))
        .send()
        .await?;
    assert_eq!(replay.status(), StatusCode::OK);
    let replayed: Value = replay.json().await?;
    assert_eq!(replayed["enrollment"]["id"], created["enrollment_id"]);
    assert!(replayed.get("code").is_none());
    Ok(())
}

#[tokio::test]
async fn scope_miss_is_forbidden_and_names_the_required_scope() -> Result<()> {
    let f = Fixture::new().await?;
    let reader = mint(&f, "reader", &[ApiTokenScope::FleetRead]).await?;
    let response = token_post(&f, "/enrollments", &reader)
        .header("idempotency-key", "scope-miss")
        .json(&json!({"targetIds":[]}))
        .send()
        .await?;
    let body = expect_error(response, StatusCode::FORBIDDEN).await?;
    assert_eq!(body["code"], "forbidden");
    assert_eq!(body["details"]["kind"], "scope");
    assert_eq!(body["details"]["requiredScope"], "fleet:enroll");
    Ok(())
}

async fn expect_error(response: Response, status: StatusCode) -> Result<Value> {
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    Ok(response.json().await?)
}

#[tokio::test]
async fn unknown_malformed_expired_and_revoked_tokens_are_identical() -> Result<()> {
    let f = Fixture::new().await?;
    let mut bodies: Vec<Value> = Vec::new();

    let unknown = token_get(
        &f,
        "/hosts",
        "gat_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    )
    .send()
    .await?;
    bodies.push(expect_error(unknown, StatusCode::UNAUTHORIZED).await?);

    let malformed = token_get(&f, "/hosts", "not-a-token").send().await?;
    bodies.push(expect_error(malformed, StatusCode::UNAUTHORIZED).await?);

    let host_credential = token_get(&f, "/hosts", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        .send()
        .await?;
    bodies.push(expect_error(host_credential, StatusCode::UNAUTHORIZED).await?);

    let expired = gridops_core::api_tokens::ApiTokenService::new(f.database.clone())
        .create("fleet-user", "expired", &[ApiTokenScope::FleetRead], 1)
        .await;
    assert!(expired.is_err(), "creation must reject a past expiry");

    let receipt = ApiTokenService::new(f.database.clone())
        .create(
            "fleet-user",
            "revoked",
            &[ApiTokenScope::FleetRead],
            gridops_core::now_millis() + DAY,
        )
        .await?;
    ApiTokenService::new(f.database.clone())
        .revoke("fleet-user", receipt.token_id())
        .await?;
    let revoked = token_get(&f, "/hosts", receipt.expose()).send().await?;
    bodies.push(expect_error(revoked, StatusCode::UNAUTHORIZED).await?);

    for body in &bodies[1..] {
        assert_eq!(body["code"], bodies[0]["code"]);
        assert_eq!(body["message"], bodies[0]["message"]);
        assert_eq!(body["details"], bodies[0]["details"]);
    }
    // None of them names a reason.
    assert_eq!(bodies[0]["code"], "unauthenticated");
    assert_eq!(bodies[0]["details"], Value::Null);
    Ok(())
}

#[tokio::test]
async fn cookie_and_bearer_together_is_rejected() -> Result<()> {
    let f = Fixture::new().await?;
    let token = mint(&f, "either", &[ApiTokenScope::FleetRead]).await?;
    let response = f
        .client
        .get(f.url("/hosts"))
        .bearer_auth(&token)
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?;
    error(response, StatusCode::UNAUTHORIZED, "unauthenticated").await?;
    Ok(())
}

#[tokio::test]
async fn bearer_needs_no_origin_but_cookie_writes_still_do() -> Result<()> {
    let f = Fixture::new().await?;
    let token = mint(&f, "issuer", &[ApiTokenScope::FleetEnroll]).await?;

    // A bearer request carries no ambient credential: a foreign Origin is fine.
    let issued = token_post(&f, "/enrollments", &token)
        .header("idempotency-key", "bearer-origin")
        .header(header::ORIGIN, "https://untrusted.example")
        .json(&json!({"targetIds":[]}))
        .send()
        .await?;
    assert_eq!(issued.status(), StatusCode::CREATED);

    // The cookie path still demands the exact Origin.
    let refused = f
        .admin_post("/enrollments")
        .header("idempotency-key", "cookie-origin")
        .header(header::ORIGIN, "https://untrusted.example")
        .json(&json!({"targetIds":[]}))
        .send()
        .await?;
    error(refused, StatusCode::FORBIDDEN, "forbidden").await?;
    Ok(())
}

#[tokio::test]
async fn cookie_only_routes_and_agent_routes_reject_tokens() -> Result<()> {
    let f = Fixture::new().await?;
    let (host, secret) = enrolled(&f).await?;
    let all_scopes = mint(
        &f,
        "overprivileged",
        &[
            ApiTokenScope::FleetRead,
            ApiTokenScope::FleetEnroll,
            ApiTokenScope::FleetApprove,
            ApiTokenScope::FleetOperate,
        ],
    )
    .await?;

    // Even every scope cannot reach cookie-only write routes. The Origin is
    // correct, so the failure is the token itself, not the CSRF check.
    let recover = token_post(&f, &format!("/hosts/{host}/recover"), &all_scopes)
        .header(header::ORIGIN, &f.origin)
        .json(&json!({"expectedRevision":1}))
        .send()
        .await?;
    error(recover, StatusCode::UNAUTHORIZED, "unauthenticated").await?;
    let retire = token_post(&f, &format!("/hosts/{host}/retire"), &all_scopes)
        .header(header::ORIGIN, &f.origin)
        .json(&json!({"expectedRevision":1}))
        .send()
        .await?;
    error(retire, StatusCode::UNAUTHORIZED, "unauthenticated").await?;

    // A token is not a host credential.
    let poll = token_post(
        &f,
        &format!("/agent/hosts/{host}/commands/poll"),
        &all_scopes,
    )
    .json(&json!({}))
    .send()
    .await?;
    error(poll, StatusCode::UNAUTHORIZED, "unauthenticated").await?;

    // A host credential is not a browser credential.
    let browser = token_get(&f, "/hosts", &secret).send().await?;
    error(browser, StatusCode::UNAUTHORIZED, "unauthenticated").await?;
    Ok(())
}

#[tokio::test]
async fn demoting_the_owner_neuters_their_token_without_touching_it() -> Result<()> {
    let f = Fixture::new().await?;
    let token = mint(&f, "issuer", &[ApiTokenScope::FleetEnroll]).await?;
    sqlx::query("UPDATE users SET role='member' WHERE id='fleet-user'")
        .execute(&f.database)
        .await?;

    let response = token_post(&f, "/enrollments", &token)
        .header("idempotency-key", "demoted")
        .json(&json!({"targetIds":[]}))
        .send()
        .await?;
    error(response, StatusCode::FORBIDDEN, "forbidden").await?;
    // The token row survives; promoting the owner restores access.
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM api_tokens WHERE user_id='fleet-user' AND revoked_at IS NULL",
    )
    .fetch_one(&f.database)
    .await?;
    assert_eq!(rows, 1);
    sqlx::query("UPDATE users SET role='admin' WHERE id='fleet-user'")
        .execute(&f.database)
        .await?;
    let restored = token_post(&f, "/enrollments", &token)
        .header("idempotency-key", "restored")
        .json(&json!({"targetIds":[]}))
        .send()
        .await?;
    assert_eq!(restored.status(), StatusCode::CREATED);
    Ok(())
}

/// The full onboarding a bot performs with the four v1 scopes: read the
/// host's approval evidence, approve it, enable a runtime and activate it.
#[tokio::test]
async fn a_token_drives_a_host_from_enrolled_to_active() -> Result<()> {
    let f = Fixture::new().await?;
    let token = mint(
        &f,
        "fleet-bot",
        &[
            ApiTokenScope::FleetRead,
            ApiTokenScope::FleetApprove,
            ApiTokenScope::FleetOperate,
        ],
    )
    .await?;

    let (host, secret) = enrolled(&f).await?;
    let incarnation = control_plane(&f).await?;
    let handshake = receipt(
        agent_post(&f, &host, &secret, "handshake")
            .json(&handshake_body())
            .send()
            .await?,
    )
    .await?;
    receipt(
        agent_post(&f, &host, &secret, "inventory")
            .json(&inventory_body(&handshake, &incarnation))
            .send()
            .await?,
    )
    .await?;

    // Read the approval evidence with fleet:read...
    let read: Value = token_get(&f, &format!("/hosts/{host}"), &token)
        .send()
        .await?
        .json()
        .await?;
    let evidence = &read["item"]["approvalEvidence"];
    assert_eq!(evidence["state"], "available");

    // ...approve with fleet:approve...
    let approved = token_post(&f, &format!("/hosts/{host}/approve"), &token)
        .json(&json!({
            "expectedRevision": evidence["expectedRevision"],
            "expectedInventoryRevision": evidence["expectedInventoryRevision"],
            "expectedInventoryDigest": evidence["expectedInventoryDigest"],
            "targetIds": [],
        }))
        .send()
        .await?;
    assert_eq!(
        approved.status(),
        StatusCode::OK,
        "{:?}",
        approved.text().await?
    );

    // ...turn on the runtime and activate with fleet:operate.
    let backend: String = sqlx::query_scalar("SELECT id FROM host_backends WHERE host_id=?")
        .bind(&host)
        .fetch_one(&f.database)
        .await?;
    let enabled = token_post(
        &f,
        &format!("/hosts/{host}/backends/{backend}/enabled"),
        &token,
    )
    .json(&json!({"enabled": true}))
    .send()
    .await?;
    assert_eq!(enabled.status(), StatusCode::OK);
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM fleet_hosts WHERE id=?")
        .bind(&host)
        .fetch_one(&f.database)
        .await?;
    let activated = token_post(&f, &format!("/hosts/{host}/lifecycle"), &token)
        .json(&json!({"lifecycle": "active", "expectedRevision": revision}))
        .send()
        .await?;
    assert_eq!(activated.status(), StatusCode::OK);

    let state: (String, String) =
        sqlx::query_as("SELECT enrollment_state,lifecycle_state FROM fleet_hosts WHERE id=?")
            .bind(&host)
            .fetch_one(&f.database)
            .await?;
    assert_eq!(state, ("approved".into(), "active".into()));
    Ok(())
}
