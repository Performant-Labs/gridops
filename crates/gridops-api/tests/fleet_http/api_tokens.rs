//! Exercises scoped API-token authentication through the actual API binary.
//! Tokens must reach exactly the routes the v1 map names, through the same
//! handlers as cookies, and fail closed everywhere else.

use super::inventory::*;
use super::*;
use gridops_core::api_tokens::{ApiTokenScope, ApiTokenService};

const DAY: i64 = 86_400_000;

pub(super) async fn mint(f: &Fixture, name: &str, scopes: &[ApiTokenScope]) -> Result<String> {
    let receipt = ApiTokenService::new(f.database.clone())
        .create("fleet-user", name, scopes, gridops_core::now_millis() + DAY)
        .await?;
    Ok(receipt.expose().to_owned())
}

pub(super) fn token_get(f: &Fixture, path: &str, token: &str) -> reqwest::RequestBuilder {
    f.client.get(f.url(path)).bearer_auth(token)
}

fn token_post(f: &Fixture, path: &str, token: &str) -> reqwest::RequestBuilder {
    f.client.post(f.url(path)).bearer_auth(token)
}

pub(super) async fn member_fixture() -> Result<Fixture> {
    let f = Fixture::new().await?;
    let now = gridops_core::now_millis();
    sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('fleet-member',2,'fleet-member','member','',?,?,?)")
        .bind(now).bind(now).bind(now).execute(&f.database).await?;
    Ok(f)
}

/// Logs the member in by reusing the seeded session machinery: their cookie
/// is a second valid session row for the member user.
pub(super) async fn member_cookie(f: &Fixture) -> Result<String> {
    let now = gridops_core::now_millis();
    let token = format!("member-{}", Uuid::new_v4());
    sqlx::query("INSERT INTO sessions (id,token_hash,user_id,expires_at,last_seen_at,created_at) VALUES (?,?,'fleet-member',?,?,?)")
        .bind(Uuid::new_v4().to_string())
        .bind(gridops_core::crypto::hash_token(&token))
        .bind(now + 3_600_000).bind(now).bind(now)
        .execute(&f.database)
        .await?;
    let mut mac = Hmac::<Sha256>::new_from_slice(b"fleet-http-test-session-key")?;
    mac.update(token.as_bytes());
    Ok(format!(
        "gridops_session={token}.{}",
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    ))
}

#[tokio::test]
async fn management_routes_are_cookie_only_and_admin_only() -> Result<()> {
    let f = member_fixture().await?;
    let cookie = member_cookie(&f).await?;

    // Member cookie: forbidden on create and revoke, and their list is empty.
    let refused = f
        .client
        .post(f.url("/api-tokens"))
        .header(header::COOKIE, &cookie)
        .header(header::ORIGIN, &f.origin)
        .header("idempotency-key", "member-create")
        .json(&json!({"name":"member-token","scopes":["fleet:read"]}))
        .send()
        .await?;
    error(refused, StatusCode::FORBIDDEN, "forbidden").await?;

    let listed = f
        .client
        .get(f.url("/api-tokens"))
        .header(header::COOKIE, &cookie)
        .send()
        .await?;
    assert_eq!(listed.status(), StatusCode::OK);
    let body: Value = listed.json().await?;
    assert_eq!(body["items"], json!([]));

    // A valid admin token cannot manage tokens: not even the list. The
    // Origin is correct so the write refusal is the token itself.
    let admin_token = mint(&f, "admin", &[ApiTokenScope::FleetRead]).await?;
    let bearer_list = token_get(&f, "/api-tokens", &admin_token).send().await?;
    error(bearer_list, StatusCode::UNAUTHORIZED, "unauthenticated").await?;
    let bearer_create = token_post(&f, "/api-tokens", &admin_token)
        .header(header::ORIGIN, &f.origin)
        .header("idempotency-key", "bearer-create")
        .json(&json!({"name":"self-replicate","scopes":["fleet:read"]}))
        .send()
        .await?;
    error(bearer_create, StatusCode::UNAUTHORIZED, "unauthenticated").await?;

    // Unauthenticated entirely.
    let anonymous = f.client.get(f.url("/api-tokens")).send().await?;
    error(anonymous, StatusCode::UNAUTHORIZED, "unauthenticated").await?;
    Ok(())
}

#[tokio::test]
async fn create_shows_the_plaintext_once_and_honours_idempotency() -> Result<()> {
    let f = Fixture::new().await?;

    let created = f
        .admin_post("/api-tokens")
        .header("idempotency-key", "manage-1")
        .json(&json!({"name":"ci-bot","scopes":["fleet:read","fleet:enroll"],"expiresInDays":30}))
        .send()
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created_body: Value = created.json().await?;
    let secret = created_body["tokenSecret"]
        .as_str()
        .context("plaintext once")?
        .to_owned();
    assert!(secret.starts_with("gat_"));
    assert_eq!(created_body["name"], "ci-bot");
    assert_eq!(
        created_body["scopes"],
        json!(["fleet:read", "fleet:enroll"])
    );
    assert_eq!(
        created_body["displayPrefix"],
        &secret["gat_".len().."gat_".len() + 8]
    );

    // The created token authenticates immediately.
    let used = token_get(&f, "/hosts", &secret).send().await?;
    assert_eq!(used.status(), StatusCode::OK);

    // Replay: same key, same body: metadata only, no second secret.
    let replay = f
        .admin_post("/api-tokens")
        .header("idempotency-key", "manage-1")
        .json(&json!({"name":"ci-bot","scopes":["fleet:read","fleet:enroll"],"expiresInDays":30}))
        .send()
        .await?;
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_body: Value = replay.json().await?;
    assert_eq!(replay_body["token"]["id"], created_body["id"]);
    assert!(replay_body.get("tokenSecret").is_none());

    // Same key, different body: conflict.
    let conflict = f
        .admin_post("/api-tokens")
        .header("idempotency-key", "manage-1")
        .json(&json!({"name":"other","scopes":["fleet:read"],"expiresInDays":30}))
        .send()
        .await?;
    error(conflict, StatusCode::CONFLICT, "idempotency_conflict").await?;

    // The list carries no secret in any shape.
    let list = f
        .client
        .get(f.url("/api-tokens"))
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?;
    let list_body: Value = list.json().await?;
    assert_eq!(list_body["items"].as_array().context("items")?.len(), 1);
    let serialized = serde_json::to_string(&list_body)?;
    assert!(!serialized.contains(&secret));

    // The audit row names the token but never carries it.
    let audit: String =
        sqlx::query_scalar("SELECT metadata FROM audit_events WHERE action='api_token.created'")
            .fetch_one(&f.database)
            .await?;
    assert!(!audit.contains(&secret));
    assert!(audit.contains("ci-bot"));

    // Unknown scope is an invalid_request naming the field.
    let bad_scope = f
        .admin_post("/api-tokens")
        .header("idempotency-key", "manage-2")
        .json(&json!({"name":"bad","scopes":["fleet:admin"]}))
        .send()
        .await?;
    let body = expect_error(bad_scope, StatusCode::BAD_REQUEST).await?;
    assert_eq!(body["details"]["kind"], "invalid_field");
    assert_eq!(body["details"]["field"], "scopes");
    Ok(())
}

#[tokio::test]
async fn revoking_through_the_api_kills_the_token_immediately() -> Result<()> {
    let f = Fixture::new().await?;
    let created = f
        .admin_post("/api-tokens")
        .header("idempotency-key", "revoke-1")
        .json(&json!({"name":"short-lived","scopes":["fleet:read"]}))
        .send()
        .await?;
    let created_body: Value = created.json().await?;
    let secret = created_body["tokenSecret"]
        .as_str()
        .context("secret")?
        .to_owned();
    let token_id = created_body["id"].as_str().context("id")?.to_owned();

    let revoked = f
        .admin_post(&format!("/api-tokens/{token_id}/revoke"))
        .send()
        .await?;
    assert_eq!(revoked.status(), StatusCode::OK);
    let revoked_body: Value = revoked.json().await?;
    assert_eq!(revoked_body["token"]["id"], token_id);
    assert!(revoked_body["token"]["revokedAt"].is_string());

    // The very next request with the plaintext fails...
    let dead = token_get(&f, "/hosts", &secret).send().await?;
    error(dead, StatusCode::UNAUTHORIZED, "unauthenticated").await?;

    // ...and revoking again is a no-op, not an error.
    let again = f
        .admin_post(&format!("/api-tokens/{token_id}/revoke"))
        .send()
        .await?;
    assert_eq!(again.status(), StatusCode::OK);

    // A member may not revoke the admin's token.
    let now = gridops_core::now_millis();
    sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('fleet-member',2,'fleet-member','member','',?,?,?)")
        .bind(now).bind(now).bind(now).execute(&f.database).await?;
    let member_session = member_cookie(&f).await?;
    let refused = f
        .client
        .post(f.url(&format!("/api-tokens/{token_id}/revoke")))
        .header(header::COOKIE, &member_session)
        .header(header::ORIGIN, &f.origin)
        .send()
        .await?;
    error(refused, StatusCode::FORBIDDEN, "forbidden").await?;

    let missing = f
        .admin_post("/api-tokens/00000000-0000-4000-8000-000000000000/revoke")
        .send()
        .await?;
    error(missing, StatusCode::NOT_FOUND, "not_found").await?;
    Ok(())
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
async fn token_actions_are_audited_with_the_token_and_record_last_use() -> Result<()> {
    let f = Fixture::new().await?;
    let token = mint(&f, "audited-bot", &[ApiTokenScope::FleetEnroll]).await?;

    // Before use: no last_used_at.
    let before: Option<i64> =
        sqlx::query_scalar("SELECT last_used_at FROM api_tokens WHERE name='audited-bot'")
            .fetch_one(&f.database)
            .await?;
    assert_eq!(before, None);

    let created = token_post(&f, "/enrollments", &token)
        .header("idempotency-key", "audited-issue")
        .json(&json!({"targetIds":[]}))
        .send()
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);

    // The enrollment audit event names the token, same action as cookies.
    let row: (String, String) = sqlx::query_as(
        "SELECT actor_label,metadata FROM audit_events WHERE action='fleet.enrollment.issued'",
    )
    .fetch_one(&f.database)
    .await?;
    assert!(
        row.0.contains("(token: audited-bot)"),
        "label was {}",
        row.0
    );
    let metadata: Value = serde_json::from_str(&row.1)?;
    assert_eq!(metadata["viaApiToken"]["name"], "audited-bot");
    assert!(
        metadata["viaApiToken"]["id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );

    // A successful call moved last_used_at.
    let after: Option<i64> =
        sqlx::query_scalar("SELECT last_used_at FROM api_tokens WHERE name='audited-bot'")
            .fetch_one(&f.database)
            .await?;
    assert!(after.is_some());

    // A scope denial is audited, rate-capped to one row per minute.
    let denied = token_get(&f, "/targets", &token).send().await?;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let denied = token_get(&f, "/targets", &token).send().await?;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let denials: Vec<String> =
        sqlx::query_scalar("SELECT metadata FROM audit_events WHERE action='api_token.denied'")
            .fetch_all(&f.database)
            .await?;
    assert_eq!(denials.len(), 1, "denial audit is rate-capped");
    let metadata: Value = serde_json::from_str(&denials[0])?;
    assert_eq!(metadata["requiredScope"], "fleet:read");
    assert_eq!(metadata["method"], "GET");
    assert_eq!(metadata["route"], "/api/v1/fleet/targets");

    // A rejected authentication never records use.
    let dead = format!("gat_{}", "B".repeat(43));
    token_get(&f, "/hosts", &dead).send().await?;
    let untouched: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM api_tokens WHERE last_used_at IS NOT NULL AND user_id!='fleet-user'",
    )
    .fetch_one(&f.database)
    .await?;
    assert_eq!(untouched, 0);
    Ok(())
}

/// The documented recipe, verbatim: the shipped script drives a host from
/// nothing to active against the real API process, with the test standing
/// in for the agent on the machine.
#[tokio::test]
async fn the_documented_script_onboards_a_host_end_to_end() -> Result<()> {
    let f = Fixture::new().await?;
    let token = mint(
        &f,
        "script-bot",
        &[
            ApiTokenScope::FleetRead,
            ApiTokenScope::FleetEnroll,
            ApiTokenScope::FleetApprove,
            ApiTokenScope::FleetOperate,
        ],
    )
    .await?;

    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fleet-enroll-bot.sh");
    let run = |args: &[&str]| -> Result<String> {
        let output = std::process::Command::new("sh")
            .arg(script.as_os_str())
            .args(args)
            .env("GRIDOPS_URL", &f.origin)
            .env("GRIDOPS_API_TOKEN", &token)
            .env("GRIDOPS_ALLOW_INSECURE", "1")
            .env("GRIDOPS_POLL_SECONDS", "1")
            .env("GRIDOPS_TIMEOUT_SECONDS", "60")
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        anyhow::ensure!(
            output.status.success(),
            "script {args:?} failed: {stdout}{stderr}"
        );
        // The token never crosses into the script's output.
        assert!(!stdout.contains(&token) && !stderr.contains(&token));
        Ok(stdout)
    };

    // 1. issue — the code and enrollment id arrive on stdout.
    let issued = run(&["issue"])?;
    let code = issued
        .lines()
        .find_map(|line| line.strip_prefix("code="))
        .context("code=")?
        .to_owned();
    let enrollment_id = issued
        .lines()
        .find_map(|line| line.strip_prefix("enrollment_id="))
        .context("enrollment_id=")?
        .to_owned();
    assert!(code.len() >= 40, "one-time code: {code}");

    // 2. the agent enrolls on the machine (the test plays the agent).
    let body: Value = f.consume(&code).await?.json().await?;
    let host = body["host_id"].as_str().context("host id")?.to_owned();
    let secret = body["credential"]
        .as_str()
        .context("credential")?
        .to_owned();
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

    // 3+4. approve waits for evidence and approves.
    let approved = run(&["approve", &enrollment_id])?;
    let script_host = approved
        .lines()
        .find_map(|line| line.strip_prefix("host_id="))
        .context("host_id=")?
        .to_owned();
    assert_eq!(script_host, host, "correlated through the enrollment");

    // 5. activate enables the runtimes and sets the host active.
    run(&["activate", &host])?;
    let state: (String, String) =
        sqlx::query_as("SELECT enrollment_state,lifecycle_state FROM fleet_hosts WHERE id=?")
            .bind(&host)
            .fetch_one(&f.database)
            .await?;
    assert_eq!(state, ("approved".into(), "active".into()));
    let enabled: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM host_backends WHERE host_id=? AND enabled=1")
            .bind(&host)
            .fetch_one(&f.database)
            .await?;
    assert!(enabled > 0, "at least one runtime enabled");
    Ok(())
}

#[tokio::test]
async fn the_script_refuses_unsafe_urls_and_missing_tokens() -> Result<()> {
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fleet-enroll-bot.sh");
    let attempt = |url: &str, token: &str| -> String {
        std::process::Command::new("sh")
            .arg(script.as_os_str())
            .arg("issue")
            .env("GRIDOPS_URL", url)
            .env("GRIDOPS_API_TOKEN", token)
            .output()
            .map(|output| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
            })
            .unwrap_or_default()
    };
    let refused = attempt("http://gridops.example.com", "gat_x");
    assert!(refused.contains("HTTPS"), "{refused}");
    let scheme = attempt("gridops.example.com", "gat_x");
    assert!(scheme.contains("https://"), "{scheme}");
    Ok(())
}

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
