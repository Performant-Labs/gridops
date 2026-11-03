//! Who may call what, through the actual API binary.
//!
//! The runner-pool routes and machine retirement accept a scoped API token
//! (`pools:read`, `pools:manage`, `fleet:retire`) with the same validation and
//! ownership rules the browser applies to the token's owner. Every other
//! route that needs a signed-in user refuses a bearer token with
//! `403 token_not_accepted` instead of answering as a signed-out caller. The
//! browser (cookie) behaviour of every touched route is unchanged.
//!
//! The authoritative matrix lives in `docs/fleet-api.md`; one test below reads
//! it and checks every row against the running server.

use super::{
    api_tokens::{member_cookie, mint, token_get},
    placement::seed_pool,
    *,
};
use gridops_core::api_tokens::{ApiTokenScope, ApiTokenService};

const DAY: i64 = 86_400_000;
const ALL_SCOPES: [ApiTokenScope; 7] = [
    ApiTokenScope::FleetRead,
    ApiTokenScope::FleetEnroll,
    ApiTokenScope::FleetApprove,
    ApiTokenScope::FleetOperate,
    ApiTokenScope::FleetRetire,
    ApiTokenScope::PoolsRead,
    ApiTokenScope::PoolsManage,
];
/// Planted in every secret column a pool response could leak from.
const SECRET_SENTINEL: &str = "pool-secret-sentinel-must-not-leak";

fn api(f: &Fixture, path: &str) -> String {
    format!("{}{path}", f.origin)
}

/// Every scope except `missing`: shows that no other scope implies it.
fn all_but(missing: ApiTokenScope) -> Vec<ApiTokenScope> {
    ALL_SCOPES
        .iter()
        .copied()
        .filter(|scope| *scope != missing)
        .collect()
}

async fn mint_for(
    f: &Fixture,
    user: &str,
    name: &str,
    scopes: &[ApiTokenScope],
) -> Result<(String, String)> {
    let receipt = ApiTokenService::new(f.database.clone())
        .create(user, name, scopes, gridops_core::now_millis() + DAY)
        .await?;
    Ok((receipt.token_id().to_owned(), receipt.expose().to_owned()))
}

/// A second administrator who owns tokens, with admin access to the pool's
/// installation, so demoting them shows the token stops while their
/// installation access (which the browser would honour) stays.
async fn seed_second_owner(f: &Fixture) -> Result<()> {
    let now = gridops_core::now_millis();
    sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('pool-owner',3,'pool-owner','admin',?,?,?,?)")
        .bind(SECRET_SENTINEL).bind(now).bind(now).bind(now).execute(&f.database).await?;
    // Admin access to the pool installation, when the test has one.
    sqlx::query("INSERT INTO user_installations (user_id,installation_id,permission,created_at) SELECT 'pool-owner',id,'admin',1 FROM installations WHERE id=1")
        .execute(&f.database).await?;
    Ok(())
}

/// The installation, repository and repository pool the browser tests use,
/// plus secrets planted where a careless response could pick them up.
async fn seed(f: &Fixture) -> Result<()> {
    seed_pool(f).await?;
    sqlx::query("UPDATE users SET access_token=? WHERE id='fleet-user'")
        .bind(SECRET_SENTINEL)
        .execute(&f.database)
        .await?;
    sqlx::query("INSERT INTO bitbucket_connections (id,name,workspace,workspace_uuid,access_token_key,created_at,updated_at) VALUES ('6a1c1b2e-0000-4000-8000-000000000001','bb','bb-space','{bb}',?,1,1)")
        .bind(SECRET_SENTINEL)
        .execute(&f.database)
        .await?;
    seed_second_owner(f).await
}

fn create_body(name: &str) -> Value {
    json!({
        "installationId": 1, "name": name, "scope": "organization", "mode": "ephemeral",
        "provider": "docker", "labels": [], "image": "runner:fixture",
        "desiredCount": 0, "minCount": 0, "maxCount": 2, "autoscalingEnabled": false,
        "queueScaleFactor": 1, "idleTimeoutMinutes": 10, "cpuLimit": 1.0,
        "memoryLimitMb": 1024, "runnerGroupId": 1,
    })
}

fn update_body(name: &str, desired: i64) -> Value {
    json!({
        "name": name, "mode": "ephemeral", "provider": "docker", "labels": [],
        "image": "runner:fixture", "desiredCount": desired, "minCount": 0, "maxCount": 3,
        "autoscalingEnabled": false, "queueScaleFactor": 1, "idleTimeoutMinutes": 10,
        "cpuLimit": 1.0, "memoryLimitMb": 1024, "runnerGroupId": 1,
    })
}

/// A pool inserted directly, to give each delete attempt its own target.
async fn spare_pool(f: &Fixture, id: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,created_at,updated_at)
         VALUES (?,1,?,'organization','docker','[\"docker\"]','[]','runner:fixture','runner:fixture','macos',1,1)",
    )
    .bind(id)
    .bind(id)
    .execute(&f.database)
    .await?;
    Ok(())
}

async fn pool_exists(f: &Fixture, id: &str) -> Result<bool> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runner_pools WHERE id=?")
        .bind(id)
        .fetch_one(&f.database)
        .await?;
    Ok(count == 1)
}

/// A fleet error with the server's request id in the header, `no-store`,
/// and the given code; returns the body for further checks.
async fn fleet_error(response: Response, status: StatusCode, code: &str) -> Result<Value> {
    let url = response.url().to_string();
    assert_eq!(response.status(), status, "{url}");
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .with_context(|| format!("no cache policy on {url}"))?,
        "no-store"
    );
    let request_id = response
        .headers()
        .get("x-request-id")
        .context("request id")?
        .to_str()?
        .to_owned();
    let body: Value = response.json().await?;
    assert_eq!(body["code"], code, "{url}: {body}");
    assert_eq!(body["requestId"], request_id.as_str(), "{url}");
    // It parses as the shared contract type.
    serde_json::from_value::<FleetErrorResponse>(body.clone())?;
    Ok(body)
}

/// The browser's legacy error body, unchanged.
async fn legacy_error(response: Response, status: StatusCode, message: &str) -> Result<()> {
    let url = response.url().to_string();
    assert_eq!(response.status(), status, "{url}");
    let body: Value = response.json().await?;
    assert_eq!(body, json!({ "error": message }), "{url}");
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Caller {
    Anonymous,
    MalformedBearer,
    ExpiredToken,
    RevokedToken,
    DemotedOwner,
    WrongScope,
    RightScope,
    Cookie,
    CookieAndBearer,
}

const CALLERS: [Caller; 9] = [
    Caller::Anonymous,
    Caller::MalformedBearer,
    Caller::ExpiredToken,
    Caller::RevokedToken,
    Caller::DemotedOwner,
    Caller::WrongScope,
    Caller::RightScope,
    Caller::Cookie,
    Caller::CookieAndBearer,
];

/// The tokens one route's matrix needs, minted for its required scope.
struct Credentials {
    expired: String,
    revoked: String,
    demoted: String,
    wrong: String,
    right: String,
}

impl Credentials {
    async fn new(f: &Fixture, scope: ApiTokenScope, label: &str) -> Result<Self> {
        let service = ApiTokenService::new(f.database.clone());
        let (expired_id, expired) =
            mint_for(f, "fleet-user", &format!("{label}-expired"), &ALL_SCOPES).await?;
        sqlx::query("UPDATE api_tokens SET expires_at=1 WHERE id=?")
            .bind(&expired_id)
            .execute(&f.database)
            .await?;
        let (revoked_id, revoked) =
            mint_for(f, "fleet-user", &format!("{label}-revoked"), &ALL_SCOPES).await?;
        service.revoke("fleet-user", &revoked_id).await?;
        let (_, demoted) =
            mint_for(f, "pool-owner", &format!("{label}-demoted"), &ALL_SCOPES).await?;
        let (_, wrong) =
            mint_for(f, "fleet-user", &format!("{label}-wrong"), &all_but(scope)).await?;
        let (_, right) = mint_for(f, "fleet-user", &format!("{label}-right"), &[scope]).await?;
        Ok(Self {
            expired,
            revoked,
            demoted,
            wrong,
            right,
        })
    }
}

fn with_caller(
    f: &Fixture,
    request: reqwest::RequestBuilder,
    caller: Caller,
    credentials: &Credentials,
) -> reqwest::RequestBuilder {
    // Every request carries the correct Origin, so a refusal is about the
    // credential, never about the cross-site check.
    let request = request.header(header::ORIGIN, &f.origin);
    match caller {
        Caller::Anonymous => request,
        Caller::MalformedBearer => request.bearer_auth("not-a-token"),
        Caller::ExpiredToken => request.bearer_auth(&credentials.expired),
        Caller::RevokedToken => request.bearer_auth(&credentials.revoked),
        Caller::DemotedOwner => request.bearer_auth(&credentials.demoted),
        Caller::WrongScope => request.bearer_auth(&credentials.wrong),
        Caller::RightScope => request.bearer_auth(&credentials.right),
        Caller::Cookie => request.header(header::COOKIE, &f.cookie),
        Caller::CookieAndBearer => request
            .header(header::COOKIE, &f.cookie)
            .bearer_auth(&credentials.right),
    }
}

/// How a credential refusal looks on a route: the browser's own shapes for
/// anonymous callers (unchanged), the fleet envelope for every token failure.
async fn assert_refusal(
    response: Response,
    caller: Caller,
    scope: ApiTokenScope,
    anonymous: Anonymous,
) -> Result<()> {
    match caller {
        Caller::Anonymous => match anonymous {
            Anonymous::LegacyUnauthorized => {
                legacy_error(
                    response,
                    StatusCode::UNAUTHORIZED,
                    "authentication required",
                )
                .await
            }
            Anonymous::FleetUnauthenticated => {
                fleet_error(response, StatusCode::UNAUTHORIZED, "unauthenticated")
                    .await
                    .map(drop)
            }
            Anonymous::SignedOut(expected) => {
                assert_eq!(response.status(), StatusCode::OK);
                assert_eq!(response.json::<Value>().await?, expected);
                Ok(())
            }
        },
        Caller::MalformedBearer
        | Caller::ExpiredToken
        | Caller::RevokedToken
        | Caller::CookieAndBearer => {
            let body = fleet_error(response, StatusCode::UNAUTHORIZED, "unauthenticated").await?;
            // Every authentication failure looks the same.
            assert_eq!(body["details"], Value::Null);
            assert_eq!(body["message"], "Authentication required");
            Ok(())
        }
        Caller::DemotedOwner => {
            let body = fleet_error(response, StatusCode::FORBIDDEN, "forbidden").await?;
            assert_eq!(
                body["details"],
                Value::Null,
                "a role refusal names no scope"
            );
            Ok(())
        }
        Caller::WrongScope => {
            let body = fleet_error(response, StatusCode::FORBIDDEN, "forbidden").await?;
            assert_eq!(body["details"]["kind"], "scope");
            assert_eq!(body["details"]["requiredScope"], scope.as_str());
            Ok(())
        }
        Caller::RightScope | Caller::Cookie => unreachable!("not a refusal"),
    }
}

#[derive(Clone)]
enum Anonymous {
    LegacyUnauthorized,
    FleetUnauthenticated,
    SignedOut(Value),
}

/// A read route: refusals per caller, and token and cookie answers that are
/// identical for the right scope.
async fn read_matrix(
    f: &Fixture,
    path: &str,
    scope: ApiTokenScope,
    anonymous: Anonymous,
) -> Result<()> {
    let label = path.replace(['/', '{', '}', '?', '='], "-");
    let credentials = Credentials::new(f, scope, &label).await?;
    let mut cookie_answer = None;
    let mut token_answer = None;
    for caller in CALLERS {
        let response = with_caller(f, f.client.get(api(f, path)), caller, &credentials)
            .send()
            .await?;
        match caller {
            Caller::RightScope => {
                token_answer = Some((response.status(), response.json::<Value>().await?));
            }
            Caller::Cookie => {
                cookie_answer = Some((response.status(), response.json::<Value>().await?));
            }
            _ => assert_refusal(response, caller, scope, anonymous.clone()).await?,
        }
    }
    let (cookie_status, cookie_body) = cookie_answer.context("cookie")?;
    let (token_status, token_body) = token_answer.context("token")?;
    // The token reaches the same handler with the same owner: same answer.
    assert_eq!(token_status, cookie_status, "{path}");
    assert_eq!(token_body, cookie_body, "{path}");
    let serialized = serde_json::to_string(&token_body)?;
    assert!(
        !serialized.contains(SECRET_SENTINEL),
        "{path} leaked a secret"
    );
    assert_ne!(token_body["authenticated"], false, "{path}");
    Ok(())
}

#[tokio::test]
async fn pool_read_routes_negative_matrix() -> Result<()> {
    let f = Fixture::new().await?;
    seed(&f).await?;
    sqlx::query("UPDATE users SET role='member' WHERE id='pool-owner'")
        .execute(&f.database)
        .await?;
    let empty_page = |per_page: i64| json!({"authenticated": false, "items": [], "total": 0, "page": 1, "perPage": per_page});
    read_matrix(
        &f,
        "/api/v1/runner-pools",
        ApiTokenScope::PoolsRead,
        Anonymous::SignedOut(empty_page(25)),
    )
    .await?;
    read_matrix(
        &f,
        "/api/v1/runner-pools/options",
        ApiTokenScope::PoolsRead,
        Anonymous::SignedOut(json!({
            "authenticated": false, "installations": [], "repositories": [], "runnerGroups": [],
            "bitbucketConnections": [], "installUrl": null, "defaults": null,
        })),
    )
    .await?;
    read_matrix(
        &f,
        "/api/v1/overview/capacity",
        ApiTokenScope::PoolsRead,
        Anonymous::SignedOut(json!({"window": "24h", "points": []})),
    )
    .await?;
    read_matrix(
        &f,
        "/api/v1/overview/tart-host",
        ApiTokenScope::PoolsRead,
        Anonymous::SignedOut(json!({
            "authenticated": false, "tartAgent": "unknown", "message": null,
            "slotSetting": null, "slots": null,
        })),
    )
    .await?;
    for path in [
        "/api/v1/runner-pools/fleet-pool",
        "/api/v1/runner-pools/fleet-pool/events",
        // These ask GitHub, which is unreachable here: the token must get
        // exactly the answer the cookie gets, whatever it is.
        "/api/v1/runner-pools/repositories",
        "/api/v1/installations/1/repositories",
        "/api/v1/installations/1/runner-groups",
    ] {
        read_matrix(
            &f,
            path,
            ApiTokenScope::PoolsRead,
            Anonymous::LegacyUnauthorized,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test]
async fn pool_write_routes_negative_matrix() -> Result<()> {
    let f = Fixture::new().await?;
    seed(&f).await?;
    sqlx::query("UPDATE users SET role='member' WHERE id='pool-owner'")
        .execute(&f.database)
        .await?;
    let credentials = Credentials::new(&f, ApiTokenScope::PoolsManage, "write").await?;
    let scope = ApiTokenScope::PoolsManage;
    let pools_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runner_pools")
        .fetch_one(&f.database)
        .await?;

    for (index, caller) in CALLERS.into_iter().enumerate() {
        // Create.
        let name = format!("matrix-{index}");
        let created = with_caller(
            &f,
            f.client.post(api(&f, "/api/v1/runner-pools")),
            caller,
            &credentials,
        )
        .json(&create_body(&name))
        .send()
        .await?;
        let created_id = match caller {
            Caller::RightScope | Caller::Cookie => {
                assert_eq!(created.status(), StatusCode::CREATED, "{caller:?}");
                let body: Value = created.json().await?;
                body["id"].as_str().context("pool id")?.to_owned()
            }
            _ => {
                assert_refusal(created, caller, scope, Anonymous::LegacyUnauthorized).await?;
                continue;
            }
        };

        // Edit the pool that was just created.
        let updated = with_caller(
            &f,
            f.client
                .put(api(&f, &format!("/api/v1/runner-pools/{created_id}"))),
            caller,
            &credentials,
        )
        .json(&update_body(&name, 1))
        .send()
        .await?;
        assert_eq!(updated.status(), StatusCode::OK, "{caller:?}");
        let desired: i64 = sqlx::query_scalar("SELECT desired_count FROM runner_pools WHERE id=?")
            .bind(&created_id)
            .fetch_one(&f.database)
            .await?;
        assert_eq!(desired, 1);

        // Remove it.
        let deleted = with_caller(
            &f,
            f.client
                .delete(api(&f, &format!("/api/v1/runner-pools/{created_id}"))),
            caller,
            &credentials,
        )
        .send()
        .await?;
        assert_eq!(deleted.status(), StatusCode::OK, "{caller:?}");
        assert!(!pool_exists(&f, &created_id).await?);
    }

    // Every refused caller is refused on edit and remove as well, against a
    // pool that exists, and changes nothing.
    for (index, caller) in CALLERS.into_iter().enumerate() {
        if matches!(caller, Caller::RightScope | Caller::Cookie) {
            continue;
        }
        let pool = format!("spare-{index}");
        spare_pool(&f, &pool).await?;
        let updated = with_caller(
            &f,
            f.client
                .put(api(&f, &format!("/api/v1/runner-pools/{pool}"))),
            caller,
            &credentials,
        )
        .json(&update_body(&pool, 2))
        .send()
        .await?;
        assert_refusal(updated, caller, scope, Anonymous::LegacyUnauthorized).await?;
        let deleted = with_caller(
            &f,
            f.client
                .delete(api(&f, &format!("/api/v1/runner-pools/{pool}"))),
            caller,
            &credentials,
        )
        .send()
        .await?;
        assert_refusal(deleted, caller, scope, Anonymous::LegacyUnauthorized).await?;
        assert!(pool_exists(&f, &pool).await?, "{caller:?} removed a pool");
        let desired: i64 = sqlx::query_scalar("SELECT desired_count FROM runner_pools WHERE id=?")
            .bind(&pool)
            .fetch_one(&f.database)
            .await?;
        assert_eq!(desired, 0, "{caller:?} edited a pool");
    }
    let spare: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM runner_pools WHERE id LIKE 'spare-%'")
            .fetch_one(&f.database)
            .await?;
    let pools_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runner_pools")
        .fetch_one(&f.database)
        .await?;
    assert_eq!(
        pools_after,
        pools_before + spare,
        "no refused create landed"
    );
    Ok(())
}

#[tokio::test]
async fn a_pools_token_creates_lists_edits_and_removes_a_pool_like_the_form() -> Result<()> {
    let f = Fixture::new().await?;
    seed(&f).await?;
    let (token_id, token) = mint_for(
        &f,
        "fleet-user",
        "pool-bot",
        &[ApiTokenScope::PoolsRead, ApiTokenScope::PoolsManage],
    )
    .await?;
    let bearer = |request: reqwest::RequestBuilder| request.bearer_auth(&token);

    // The form's validation errors, byte for byte, for a token and a cookie.
    for invalid in [
        json!({"name": "X"}),
        json!({"maxCount": 101}),
        json!({"scope": "galaxy"}),
        json!({"provider": "podman"}),
        json!({"labels": ["self-hosted"]}),
    ] {
        let mut body = create_body("invalid-pool");
        for (key, value) in invalid.as_object().context("delta")? {
            body[key] = value.clone();
        }
        let token_answer = bearer(f.client.post(api(&f, "/api/v1/runner-pools")))
            .json(&body)
            .send()
            .await?;
        let cookie_answer = f
            .client
            .post(api(&f, "/api/v1/runner-pools"))
            .header(header::COOKIE, &f.cookie)
            .header(header::ORIGIN, &f.origin)
            .json(&body)
            .send()
            .await?;
        assert_eq!(token_answer.status(), cookie_answer.status(), "{invalid}");
        assert_eq!(token_answer.status(), StatusCode::BAD_REQUEST, "{invalid}");
        assert_eq!(
            token_answer.json::<Value>().await?,
            cookie_answer.json::<Value>().await?,
            "{invalid}"
        );
    }

    // Create, with no Origin at all and with a foreign one: a token carries
    // no ambient credential, so the cross-site check does not apply to it.
    let created = bearer(f.client.post(api(&f, "/api/v1/runner-pools")))
        .header(header::ORIGIN, "https://untrusted.example")
        .json(&create_body("bot-pool"))
        .send()
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created: Value = created.json().await?;
    let pool = created["id"].as_str().context("pool id")?.to_owned();
    let created_by: String = sqlx::query_scalar("SELECT created_by FROM runner_pools WHERE id=?")
        .bind(&pool)
        .fetch_one(&f.database)
        .await?;
    assert_eq!(created_by, "fleet-user", "the pool belongs to the owner");

    // A duplicate name is the form's conflict.
    let duplicate = bearer(f.client.post(api(&f, "/api/v1/runner-pools")))
        .json(&create_body("bot-pool"))
        .send()
        .await?;
    legacy_error(
        duplicate,
        StatusCode::CONFLICT,
        "A runner pool with this name already exists for the installation.",
    )
    .await?;

    // List and read.
    let listed: Value = bearer(f.client.get(api(&f, "/api/v1/runner-pools")))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(listed["authenticated"], true);
    let names: Vec<&str> = listed["items"]
        .as_array()
        .context("items")?
        .iter()
        .filter_map(|item| item["name"].as_str())
        .collect();
    assert!(
        names.contains(&"bot-pool") && names.contains(&"fleet"),
        "{names:?}"
    );
    let read: Value = bearer(
        f.client
            .get(api(&f, &format!("/api/v1/runner-pools/{pool}"))),
    )
    .send()
    .await?
    .json()
    .await?;
    assert_eq!(read["name"], "bot-pool");
    assert_eq!(read["canManage"], true);

    // Edit, then an invalid edit is the form's error.
    let edited = bearer(
        f.client
            .put(api(&f, &format!("/api/v1/runner-pools/{pool}"))),
    )
    .json(&update_body("bot-pool", 2))
    .send()
    .await?;
    assert_eq!(edited.status(), StatusCode::OK);
    let mut invalid = update_body("bot-pool", 2);
    invalid["desiredCount"] = json!(9);
    let refused = bearer(
        f.client
            .put(api(&f, &format!("/api/v1/runner-pools/{pool}"))),
    )
    .json(&invalid)
    .send()
    .await?;
    legacy_error(
        refused,
        StatusCode::BAD_REQUEST,
        "Capacity must satisfy 0 <= minimum <= desired <= maximum <= 100.",
    )
    .await?;

    // Events read for the pool.
    let events = bearer(
        f.client
            .get(api(&f, &format!("/api/v1/runner-pools/{pool}/events"))),
    )
    .send()
    .await?;
    assert_eq!(events.status(), StatusCode::OK);

    // Remove.
    let removed = bearer(
        f.client
            .delete(api(&f, &format!("/api/v1/runner-pools/{pool}"))),
    )
    .send()
    .await?;
    assert_eq!(removed.status(), StatusCode::OK);
    assert!(!pool_exists(&f, &pool).await?);

    // Every mutation is audited with the token's name and id, never its secret.
    for action in [
        "runner_pool.created",
        "runner_pool.updated",
        "runner_pool.deleted",
    ] {
        let (label, metadata): (String, String) = sqlx::query_as(
            "SELECT actor_label,metadata FROM audit_events WHERE action=? AND target_id=?",
        )
        .bind(action)
        .bind(&pool)
        .fetch_one(&f.database)
        .await
        .with_context(|| format!("audit {action}"))?;
        assert_eq!(label, "fleet-admin (token: pool-bot)", "{action}");
        assert!(!metadata.contains(&token), "{action} carries the secret");
        let metadata: Value = serde_json::from_str(&metadata)?;
        assert_eq!(metadata["viaApiToken"]["name"], "pool-bot", "{action}");
        assert_eq!(metadata["viaApiToken"]["id"], token_id.as_str(), "{action}");
    }
    // The token's use is recorded, as for fleet calls.
    let last_used: Option<i64> =
        sqlx::query_scalar("SELECT last_used_at FROM api_tokens WHERE id=?")
            .bind(&token_id)
            .fetch_one(&f.database)
            .await?;
    assert!(last_used.is_some());
    Ok(())
}

/// A pool on an installation the owner cannot reach is exactly as invisible
/// to the owner's token as to the owner's browser.
#[tokio::test]
async fn a_pools_token_sees_and_changes_only_what_its_owner_may() -> Result<()> {
    let f = Fixture::new().await?;
    seed(&f).await?;
    sqlx::raw_sql(
        "INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
           VALUES (2,2,'other-org','Organization','Organization','selected',1,1);
         INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,created_at,updated_at)
           VALUES ('other-pool',2,'other','organization','docker','[\"docker\"]','[]','runner:fixture','runner:fixture','macos',1,1);",
    )
    .execute(&f.database)
    .await?;
    let (_, token) = mint_for(
        &f,
        "fleet-user",
        "nosy-bot",
        &[ApiTokenScope::PoolsRead, ApiTokenScope::PoolsManage],
    )
    .await?;

    let listed: Value = f
        .client
        .get(api(&f, "/api/v1/runner-pools"))
        .bearer_auth(&token)
        .send()
        .await?
        .json()
        .await?;
    assert!(
        !serde_json::to_string(&listed)?.contains("other-pool"),
        "{listed}"
    );
    let requests = |client: &Client, cookie: bool| {
        let with = |request: reqwest::RequestBuilder| {
            if cookie {
                request
                    .header(header::COOKIE, &f.cookie)
                    .header(header::ORIGIN, &f.origin)
            } else {
                request.bearer_auth(&token)
            }
        };
        vec![
            with(client.get(api(&f, "/api/v1/runner-pools/other-pool"))),
            with(client.get(api(&f, "/api/v1/runner-pools/other-pool/events"))),
            with(client.put(api(&f, "/api/v1/runner-pools/other-pool")))
                .json(&update_body("other", 2)),
            with(client.delete(api(&f, "/api/v1/runner-pools/other-pool"))),
            with(client.get(api(&f, "/api/v1/installations/2/runner-groups"))),
            with(client.post(api(&f, "/api/v1/runner-pools"))).json(&{
                let mut body = create_body("sneaky");
                body["installationId"] = json!(2);
                body
            }),
        ]
    };
    for (token_request, cookie_request) in requests(&f.client, false)
        .into_iter()
        .zip(requests(&f.client, true))
    {
        let token_answer = token_request.send().await?;
        let cookie_answer = cookie_request.send().await?;
        let url = token_answer.url().to_string();
        assert!(
            token_answer.status().is_client_error(),
            "{url} answered {}",
            token_answer.status()
        );
        assert_eq!(token_answer.status(), cookie_answer.status(), "{url}");
        assert_eq!(
            token_answer.json::<Value>().await?,
            cookie_answer.json::<Value>().await?,
            "{url}"
        );
    }
    assert!(pool_exists(&f, "other-pool").await?);
    let sneaky: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runner_pools WHERE name='sneaky'")
        .fetch_one(&f.database)
        .await?;
    assert_eq!(sneaky, 0);
    Ok(())
}

/// A machine as the retire tests need it: approved, with the given lifecycle,
/// last seen `quiet_millis` ago.
async fn machine(f: &Fixture, key: &str, lifecycle: &str, quiet_millis: i64) -> Result<String> {
    let issued = f.issue(key).await?;
    let response = f.consume(issued["code"].as_str().context("code")?).await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = response.json().await?;
    let host = body["host_id"].as_str().context("host ID")?.to_owned();
    let seen = gridops_core::now_millis() - quiet_millis;
    sqlx::query(
        "UPDATE fleet_hosts SET enrollment_state='approved',lifecycle_state=?,
           last_heartbeat_at=?,last_inventory_at=? WHERE id=?",
    )
    .bind(lifecycle)
    .bind(seen)
    .bind(seen)
    .bind(&host)
    .execute(&f.database)
    .await?;
    Ok(host)
}

async fn host_state(f: &Fixture, host: &str) -> Result<(String, i64)> {
    Ok(
        sqlx::query_as("SELECT lifecycle_state,revision FROM fleet_hosts WHERE id=?")
            .bind(host)
            .fetch_one(&f.database)
            .await?,
    )
}

const LONG_QUIET: i64 = 60 * 60_000;

#[tokio::test]
async fn retire_route_negative_matrix() -> Result<()> {
    let f = Fixture::new().await?;
    seed_second_owner(&f).await?;
    sqlx::query("UPDATE users SET role='member' WHERE id='pool-owner'")
        .execute(&f.database)
        .await?;
    let scope = ApiTokenScope::FleetRetire;
    let credentials = Credentials::new(&f, scope, "retire").await?;
    for caller in CALLERS {
        let host = machine(&f, &format!("{caller:?}"), "paused", LONG_QUIET).await?;
        let (_, revision) = host_state(&f, &host).await?;
        let response = with_caller(
            &f,
            f.client.post(f.url(&format!("/hosts/{host}/retire"))),
            caller,
            &credentials,
        )
        .json(&json!({"expectedRevision": revision}))
        .send()
        .await?;
        match caller {
            Caller::RightScope | Caller::Cookie => {
                assert_eq!(response.status(), StatusCode::OK, "{caller:?}");
                assert_eq!(host_state(&f, &host).await?.0, "retired", "{caller:?}");
            }
            _ => {
                assert_refusal(response, caller, scope, Anonymous::FleetUnauthenticated).await?;
                assert_eq!(host_state(&f, &host).await?.0, "paused", "{caller:?}");
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn a_retire_token_retires_only_a_machine_that_cannot_hurt_the_fleet() -> Result<()> {
    let f = Fixture::new().await?;
    let (token_id, token) = mint_for(
        &f,
        "fleet-user",
        "retire-bot",
        &[ApiTokenScope::FleetRetire],
    )
    .await?;
    let retire = |host: &str, revision: i64, token: &str| {
        f.client
            .post(f.url(&format!("/hosts/{host}/retire")))
            .bearer_auth(token)
            .json(&json!({"expectedRevision": revision}))
            .send()
    };

    // A healthy machine in service: refused with every reason, nothing moves.
    let healthy = machine(&f, "healthy", "active", 0).await?;
    let (_, revision) = host_state(&f, &healthy).await?;
    let body = fleet_error(
        retire(&healthy, revision, &token).await?,
        StatusCode::CONFLICT,
        "host_not_retirable",
    )
    .await?;
    assert_eq!(body["details"]["kind"], "retire_blocked");
    assert_eq!(
        body["details"]["reasons"],
        json!(["lifecycle_active", "recently_seen"])
    );
    assert_eq!(host_state(&f, &healthy).await?, ("active".into(), revision));

    // Drained but still reporting in within the default fifteen minutes.
    let recent = machine(&f, "recent", "draining", 5 * 60_000).await?;
    let (_, revision) = host_state(&f, &recent).await?;
    let body = fleet_error(
        retire(&recent, revision, &token).await?,
        StatusCode::CONFLICT,
        "host_not_retirable",
    )
    .await?;
    assert_eq!(body["details"]["reasons"], json!(["recently_seen"]));

    // fleet:operate (and every other scope) does not imply fleet:retire.
    let (_, operator) = mint_for(
        &f,
        "fleet-user",
        "operator",
        &all_but(ApiTokenScope::FleetRetire),
    )
    .await?;
    let offline = machine(&f, "offline", "paused", LONG_QUIET).await?;
    let (_, revision) = host_state(&f, &offline).await?;
    let body = fleet_error(
        retire(&offline, revision, &operator).await?,
        StatusCode::FORBIDDEN,
        "forbidden",
    )
    .await?;
    assert_eq!(body["details"]["requiredScope"], "fleet:retire");
    assert_eq!(host_state(&f, &offline).await?.0, "paused");

    // A stale revision is the usual conflict.
    let body = fleet_error(
        retire(&offline, revision + 7, &token).await?,
        StatusCode::CONFLICT,
        "revision_conflict",
    )
    .await?;
    assert_eq!(body["details"], Value::Null);

    // An offline, paused, idle machine is retired, and the audit row names
    // the token.
    let retired = retire(&offline, revision, &token).await?;
    assert_eq!(retired.status(), StatusCode::OK);
    assert_eq!(host_state(&f, &offline).await?.0, "retired");
    let (label, metadata): (String, String) = sqlx::query_as(
        "SELECT actor_label,metadata FROM audit_events WHERE action='fleet.host.retired' AND target_id=?",
    )
    .bind(&offline)
    .fetch_one(&f.database)
    .await?;
    assert_eq!(label, "fleet-admin (token: retire-bot)");
    let metadata: Value = serde_json::from_str(&metadata)?;
    assert_eq!(metadata["viaApiToken"]["id"], token_id.as_str());
    // Retiring again is a no-op.
    let again = retire(&offline, revision, &token).await?;
    assert_eq!(again.status(), StatusCode::OK);

    // The browser retire keeps its rules: a healthy machine is retired.
    let (_, revision) = host_state(&f, &healthy).await?;
    let browser = f
        .admin_post(&format!("/hosts/{healthy}/retire"))
        .json(&json!({"expectedRevision": revision}))
        .send()
        .await?;
    assert_eq!(browser.status(), StatusCode::OK);
    assert_eq!(host_state(&f, &healthy).await?.0, "retired");
    Ok(())
}

#[tokio::test]
async fn the_quiet_period_for_a_token_retire_is_configurable() -> Result<()> {
    let f = Fixture::with_env(&[("GRIDOPS_TOKEN_RETIRE_MIN_OFFLINE_MINUTES", "120")]).await?;
    let (_, token) = mint_for(
        &f,
        "fleet-user",
        "retire-bot",
        &[ApiTokenScope::FleetRetire],
    )
    .await?;
    // Silent for an hour: enough by default, not under a two-hour policy.
    let host = machine(&f, "configured", "paused", LONG_QUIET).await?;
    let (_, revision) = host_state(&f, &host).await?;
    let body = fleet_error(
        f.client
            .post(f.url(&format!("/hosts/{host}/retire")))
            .bearer_auth(&token)
            .json(&json!({"expectedRevision": revision}))
            .send()
            .await?,
        StatusCode::CONFLICT,
        "host_not_retirable",
    )
    .await?;
    assert_eq!(body["details"]["reasons"], json!(["recently_seen"]));
    Ok(())
}

/// The browser keeps every existing behaviour on the routes this change
/// touched: signed-out bodies for anonymous readers, a foreign Origin
/// refused on cookie writes, and an `Authorization` header that is not a
/// bearer token (for example from a proxy's Basic authentication) ignored
/// where it always was.
#[tokio::test]
async fn browser_behaviour_of_the_touched_routes_is_unchanged() -> Result<()> {
    let f = Fixture::new().await?;
    seed(&f).await?;

    let foreign = f
        .client
        .post(api(&f, "/api/v1/runner-pools"))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, "https://untrusted.example")
        .json(&create_body("cross-site"))
        .send()
        .await?;
    legacy_error(foreign, StatusCode::FORBIDDEN, "forbidden").await?;
    let foreign = f
        .client
        .delete(api(&f, "/api/v1/runner-pools/fleet-pool"))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, "https://untrusted.example")
        .send()
        .await?;
    legacy_error(foreign, StatusCode::FORBIDDEN, "forbidden").await?;
    assert!(pool_exists(&f, "fleet-pool").await?);

    // A missing Origin still passes the legacy check for a cookie.
    let listed = f
        .client
        .get(api(&f, "/api/v1/runner-pools"))
        .header(header::COOKIE, &f.cookie)
        .header(header::AUTHORIZATION, "Basic cHJveHk6cGFzcw==")
        .send()
        .await?;
    assert_eq!(listed.status(), StatusCode::OK);
    assert_eq!(listed.json::<Value>().await?["authenticated"], true);
    let settings = f
        .client
        .get(api(&f, "/api/v1/settings"))
        .header(header::COOKIE, &f.cookie)
        .header(header::AUTHORIZATION, "Basic cHJveHk6cGFzcw==")
        .send()
        .await?;
    assert_eq!(settings.status(), StatusCode::OK);
    assert_eq!(settings.json::<Value>().await?["authenticated"], true);

    // Signed out, the settings page still answers with its signed-out body.
    let signed_out = f.client.get(api(&f, "/api/v1/settings")).send().await?;
    assert_eq!(signed_out.status(), StatusCode::OK);
    assert_eq!(
        signed_out.json::<Value>().await?,
        json!({"authenticated": false, "data": null})
    );
    Ok(())
}

/// Routes that need a signed-in user but do not take tokens refuse every
/// bearer token, valid or not, alone or next to a cookie, before doing
/// anything, instead of answering as a signed-out caller.
#[tokio::test]
async fn cookie_only_routes_refuse_tokens_instead_of_answering_signed_out() -> Result<()> {
    let f = Fixture::new().await?;
    seed(&f).await?;
    let (_, token) = mint_for(&f, "fleet-user", "everything", &ALL_SCOPES).await?;
    let member = {
        let now = gridops_core::now_millis();
        sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('fleet-member',4,'fleet-member','member','',?,?,?)")
            .bind(now).bind(now).bind(now).execute(&f.database).await?;
        member_cookie(&f).await?
    };
    for path in [
        "/api/v1/settings",
        "/api/v1/webhooks",
        "/api/v1/overview",
        "/api/v1/auth/session",
        "/api/v1/platform-connections/bitbucket",
        "/api/v1/runners",
    ] {
        for request in [
            f.client.get(api(&f, path)).bearer_auth(&token),
            f.client.get(api(&f, path)).bearer_auth("not-a-token"),
            f.client
                .get(api(&f, path))
                .header(header::COOKIE, &f.cookie)
                .bearer_auth(&token),
            f.client
                .get(api(&f, path))
                .header(header::COOKIE, &member)
                .bearer_auth(&token),
        ] {
            let body = fleet_error(
                request.send().await?,
                StatusCode::FORBIDDEN,
                "token_not_accepted",
            )
            .await?;
            assert_eq!(body["details"], Value::Null);
        }
    }
    // A refused write changes nothing: the setting is not saved.
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_events")
        .fetch_one(&f.database)
        .await?;
    let refused = f
        .client
        .put(api(&f, "/api/v1/settings/runner-host"))
        .bearer_auth(&token)
        .json(&json!({"enabled": false}))
        .send()
        .await?;
    fleet_error(refused, StatusCode::FORBIDDEN, "token_not_accepted").await?;
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_events")
        .fetch_one(&f.database)
        .await?;
    assert_eq!(before, after);
    // Token management, recovery and rotation stay cookie-only, and say so.
    for (path, body) in [
        (
            "/api-tokens",
            json!({"name": "self-replicate", "scopes": ["fleet:read"]}),
        ),
        (
            "/hosts/00000000-0000-4000-8000-000000000001/recover",
            json!({"expectedRevision": 1}),
        ),
        (
            "/hosts/00000000-0000-4000-8000-000000000001/credentials/rotate",
            json!({}),
        ),
    ] {
        let response = f
            .client
            .post(f.url(path))
            .bearer_auth(&token)
            .header("idempotency-key", "refused")
            .json(&body)
            .send()
            .await?;
        fleet_error(response, StatusCode::FORBIDDEN, "token_not_accepted").await?;
    }
    let tokens: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_tokens")
        .fetch_one(&f.database)
        .await?;
    assert_eq!(tokens, 1, "a token cannot mint a token");
    Ok(())
}

/// One row of the "Who may call what" table in `docs/fleet-api.md`.
#[derive(Debug)]
struct MatrixRow {
    method: String,
    path: String,
    session: String,
    scope: Option<String>,
    origin: String,
    bearer: String,
}

fn matrix_rows() -> Result<Vec<MatrixRow>> {
    let doc = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/fleet-api.md"),
    )?;
    let section = doc
        .split("\n## Who may call what\n")
        .nth(1)
        .context("docs/fleet-api.md has no 'Who may call what' section")?;
    let section = section.split("\n## ").next().unwrap_or_default();
    let mut rows = Vec::new();
    for line in section.lines() {
        let cells: Vec<String> = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(|cell| cell.trim().replace('`', ""))
            .collect();
        if cells.len() != 6 || !matches!(cells[0].as_str(), "GET" | "POST" | "PUT" | "DELETE") {
            continue;
        }
        rows.push(MatrixRow {
            method: cells[0].clone(),
            path: cells[1].clone(),
            session: cells[2].clone(),
            scope: (cells[3] != "—").then(|| cells[3].clone()),
            origin: cells[4].clone(),
            bearer: cells[5].clone(),
        });
    }
    anyhow::ensure!(rows.len() > 100, "matrix parsed only {} rows", rows.len());
    Ok(rows)
}

fn concrete(path: &str) -> String {
    path.split('/')
        .map(|segment| match segment {
            "{installation_id}" | "{run_id}" | "{job_id}" => "1",
            placeholder if placeholder.starts_with('{') => "00000000-0000-4000-8000-0000000000aa",
            literal => literal,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn request(client: &Client, f: &Fixture, method: &str, path: &str) -> reqwest::RequestBuilder {
    let url = api(f, &concrete(path));
    let builder = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "DELETE" => client.delete(url),
        other => unreachable!("method {other}"),
    };
    if path == "/api/v1/users/{user_id}/role" {
        // The handler checks the Origin after its body parses.
        builder.json(&json!({"role": "member"}))
    } else if matches!(method, "POST" | "PUT") {
        builder.json(&json!({}))
    } else {
        builder
    }
}

/// Every row of the documented matrix, against the running server: what a
/// bearer token gets on each route, that a signed-out reader of an
/// "optional" route still gets its signed-out body, and that every route
/// documented with an Origin check refuses a cookie request from another site.
#[tokio::test]
async fn every_route_in_the_documented_matrix_behaves_as_documented() -> Result<()> {
    let f = Fixture::new().await?;
    seed(&f).await?;
    let (_, token) = mint_for(&f, "fleet-user", "matrix", &ALL_SCOPES).await?;
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let rows = matrix_rows()?;
    for row in &rows {
        let what = format!("{} {}", row.method, row.path);
        let response = request(&client, &f, &row.method, &row.path)
            .bearer_auth(&token)
            .header(header::ORIGIN, &f.origin)
            .send()
            .await?;
        let status = response.status();
        let body: Value = response
            .bytes()
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null);
        match row.bearer.as_str() {
            "403 token_not_accepted" => {
                assert!(row.scope.is_none(), "{what}: a scoped route cannot refuse");
                assert_eq!(status, StatusCode::FORBIDDEN, "{what}: {body}");
                assert_eq!(body["code"], "token_not_accepted", "{what}");
                // A malformed bearer is refused the same way.
                let malformed = request(&client, &f, &row.method, &row.path)
                    .bearer_auth("x")
                    .send()
                    .await?;
                assert_eq!(malformed.status(), StatusCode::FORBIDDEN, "{what}");
            }
            "accepted" => {
                assert!(row.scope.is_some(), "{what}: accepted needs a scope");
                assert_ne!(status, StatusCode::UNAUTHORIZED, "{what}: {body}");
                assert_ne!(body["code"], "token_not_accepted", "{what}");
                assert_ne!(body["details"]["kind"], "scope", "{what}: {body}");
                assert_ne!(body["authenticated"], false, "{what}");
            }
            "ignored" => {
                assert!(row.scope.is_none(), "{what}");
                assert_ne!(body["code"], "token_not_accepted", "{what}");
                assert_ne!(body["authenticated"], false, "{what}");
            }
            "401 unauthenticated" => {
                assert!(row.scope.is_none(), "{what}");
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{what}: {body}");
                assert_eq!(body["code"], "unauthenticated", "{what}");
            }
            other => anyhow::bail!("{what}: unknown bearer outcome {other:?}"),
        }
        if row.session == "optional" {
            let anonymous = request(&client, &f, &row.method, &row.path).send().await?;
            assert_eq!(anonymous.status(), StatusCode::OK, "{what}");
        }
    }
    // Cross-site cookie requests last: a route wrongly documented as checked
    // would run with a real session.
    for row in rows.iter().filter(|row| row.origin != "—") {
        let what = format!("{} {}", row.method, row.path);
        let response = request(&client, &f, &row.method, &row.path)
            .header(header::COOKIE, &f.cookie)
            .header(header::ORIGIN, "https://untrusted.example")
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{what}");
    }
    Ok(())
}

/// A valid token used where only a session works can never be read as
/// "there is nothing here": the audit and token lists are cookie-only and
/// say so.
#[tokio::test]
async fn a_read_only_fleet_token_is_told_it_is_not_accepted_on_the_audit_log() -> Result<()> {
    let f = Fixture::new().await?;
    let token = mint(&f, "reader", &[ApiTokenScope::FleetRead]).await?;
    let response = f
        .client
        .get(api(&f, "/api/v1/audit"))
        .bearer_auth(&token)
        .send()
        .await?;
    fleet_error(response, StatusCode::FORBIDDEN, "token_not_accepted").await?;
    // The same token still reads the fleet.
    assert_eq!(
        token_get(&f, "/hosts", &token).send().await?.status(),
        StatusCode::OK
    );
    Ok(())
}

/// An administrator can create a token with the new scopes in the web app's
/// management route, and gets them back in the server's stored order.
#[tokio::test]
async fn the_management_route_creates_tokens_with_the_new_scopes() -> Result<()> {
    let f = Fixture::new().await?;
    let created = f
        .admin_post("/api-tokens")
        .header("idempotency-key", "new-scopes")
        .json(&json!({"name": "pool-bot", "scopes": ["pools:manage", "fleet:retire", "pools:read"], "expiresInDays": 2}))
        .send()
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let body: Value = created.json().await?;
    assert_eq!(
        body["scopes"],
        json!(["fleet:retire", "pools:read", "pools:manage"])
    );
    let secret = body["tokenSecret"].as_str().context("secret")?;
    // It reaches a pool route and nothing a fleet:read token would.
    let pools = f
        .client
        .get(api(&f, "/api/v1/runner-pools"))
        .bearer_auth(secret)
        .send()
        .await?;
    assert_eq!(pools.status(), StatusCode::OK);
    let hosts = fleet_error(
        token_get(&f, "/hosts", secret).send().await?,
        StatusCode::FORBIDDEN,
        "forbidden",
    )
    .await?;
    assert_eq!(hosts["details"]["requiredScope"], "fleet:read");
    // An unknown scope is still the form's field error.
    let refused = f
        .admin_post("/api-tokens")
        .header("idempotency-key", "bad-scope")
        .json(&json!({"name": "bad", "scopes": ["pools:write"]}))
        .send()
        .await?;
    let body = fleet_error(refused, StatusCode::BAD_REQUEST, "invalid_request").await?;
    assert_eq!(body["details"]["field"], "scopes");
    Ok(())
}
