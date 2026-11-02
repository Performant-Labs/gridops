//! Deleting a machine through the API: who may, when it is refused and why,
//! what disappears, what an agent still talking as the machine sees, and that
//! enrolling the same computer again starts a clean record.

use super::{
    api_tokens::{member_cookie, member_fixture, mint, token_get},
    inventory::*,
    placement::{FakeDocker, active_machine, admitted_placement, seed_pool},
    *,
};
use gridops_core::api_tokens::ApiTokenScope;
use std::sync::Arc;

fn admin_delete(f: &Fixture, path: &str) -> reqwest::RequestBuilder {
    f.client
        .delete(f.url(path))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin)
}

fn token_delete(f: &Fixture, path: &str, token: &str) -> reqwest::RequestBuilder {
    f.client.delete(f.url(path)).bearer_auth(token)
}

/// Retires through the cookie-only route, as the machine page does.
async fn retire(f: &Fixture, host: &str) -> Result<()> {
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM fleet_hosts WHERE id=?")
        .bind(host)
        .fetch_one(&f.database)
        .await?;
    let response = f
        .admin_post(&format!("/hosts/{host}/retire"))
        .json(&json!({"expectedRevision": revision}))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

async fn listed(f: &Fixture) -> Result<Vec<String>> {
    let page: Value = f
        .client
        .get(f.url("/hosts"))
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?
        .json()
        .await?;
    Ok(page["items"]
        .as_array()
        .context("items")?
        .iter()
        .filter_map(|item| item["id"].as_str().map(str::to_owned))
        .collect())
}

async fn refusal_reasons(response: Response) -> Result<Vec<String>> {
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body: Value = response.json().await?;
    assert_eq!(body["code"], "host_not_deletable");
    assert_eq!(body["details"]["kind"], "delete_blocked");
    Ok(body["details"]["reasons"]
        .as_array()
        .context("reasons")?
        .iter()
        .filter_map(|reason| reason.as_str().map(str::to_owned))
        .collect())
}

#[tokio::test]
async fn a_retired_machine_is_deleted_by_token_and_gone_from_every_view() -> Result<()> {
    let f = Fixture::new().await?;
    let token = mint(
        &f,
        "cleanup-bot",
        &[ApiTokenScope::FleetRead, ApiTokenScope::FleetOperate],
    )
    .await?;
    let (host, secret) = enrolled(&f).await?;
    assert_eq!(listed(&f).await?, std::slice::from_ref(&host));

    retire(&f, &host).await?;
    let deleted = token_delete(&f, &format!("/hosts/{host}"), &token)
        .send()
        .await?;
    assert_eq!(deleted.status(), StatusCode::OK);
    let body: Value = deleted.json().await?;
    assert_eq!(body, json!({"hostId": host}));

    // Gone from the list and its counts, and from the machine page and events.
    assert_eq!(listed(&f).await?, Vec::<String>::new());
    for path in [format!("/hosts/{host}"), format!("/hosts/{host}/events")] {
        error(
            token_get(&f, &path, &token).send().await?,
            StatusCode::NOT_FOUND,
            "not_found",
        )
        .await?;
    }
    // A second delete finds nothing.
    error(
        token_delete(&f, &format!("/hosts/{host}"), &token)
            .send()
            .await?,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await?;

    // The audit row names the machine, its last heartbeat and the token.
    let (label, metadata): (String, String) = sqlx::query_as(
        "SELECT actor_label,metadata FROM audit_events WHERE action='fleet.host.deleted'",
    )
    .fetch_one(&f.database)
    .await?;
    assert!(label.contains("(token: cleanup-bot)"), "{label}");
    let metadata: Value = serde_json::from_str(&metadata)?;
    assert_eq!(metadata["name"], "ThinkPad");
    assert_eq!(metadata["hostId"], host.as_str());
    assert!(metadata.get("lastHeartbeatAt").is_some());
    assert_eq!(metadata["viaApiToken"]["name"], "cleanup-bot");
    let events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fleet_events WHERE kind='fleet.host.deleted' AND host_id=?",
    )
    .bind(&host)
    .fetch_one(&f.database)
    .await?;
    assert_eq!(events, 1);

    // An agent still talking as the deleted machine is turned away on every
    // agent route, and recreates nothing.
    let incarnation = control_plane(&f).await?;
    for action in ["handshake", "inventory", "heartbeat", "commands/poll"] {
        let body = if action == "handshake" {
            handshake_body()
        } else {
            json!({"incarnation": incarnation})
        };
        error(
            agent_post(&f, &host, &secret, action)
                .json(&body)
                .send()
                .await?,
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
        )
        .await?;
    }
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM fleet_hosts")
        .fetch_one(&f.database)
        .await?;
    assert_eq!(rows, 1, "the agent must not recreate the record");

    // Enrolling the same computer again is a new, clean, pending record.
    let issued = f.issue("enroll-again").await?;
    let again = f.consume(issued["code"].as_str().context("code")?).await?;
    assert_eq!(again.status(), StatusCode::CREATED);
    let again: Value = again.json().await?;
    let new_host = again["host_id"].as_str().context("host")?;
    assert_ne!(new_host, host);
    assert_eq!(listed(&f).await?, [new_host.to_owned()]);
    let read: Value = f
        .client
        .get(f.url(&format!("/hosts/{new_host}")))
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(read["item"]["enrollment"], "pending");
    assert_eq!(read["item"]["duplicateHardware"], false);
    Ok(())
}

#[tokio::test]
async fn a_machine_in_service_with_live_work_is_refused_with_every_reason() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let machine = active_machine(&f, &f.origin, Arc::new(FakeDocker::default())).await?;
    let host = machine.host.to_string();
    admitted_placement(&f, &machine, Uuid::new_v4()).await?;

    let reasons =
        refusal_reasons(admin_delete(&f, &format!("/hosts/{host}")).send().await?).await?;
    for expected in [
        "not_retired",
        "still_heartbeating",
        "active_credential",
        "live_placement",
    ] {
        assert!(
            reasons.iter().any(|reason| reason == expected),
            "{expected} in {reasons:?}"
        );
    }
    // Nothing changed.
    assert_eq!(listed(&f).await?, std::slice::from_ref(&host));
    let deleted_at: Option<i64> =
        sqlx::query_scalar("SELECT deleted_at FROM fleet_hosts WHERE id=?")
            .bind(&host)
            .fetch_one(&f.database)
            .await?;
    assert_eq!(deleted_at, None);
    let super::placement::Machine { stop, running, .. } = machine;
    let _ = stop.send(());
    running.await??;
    Ok(())
}

#[tokio::test]
async fn delete_is_admin_and_operate_scope_only() -> Result<()> {
    let f = member_fixture().await?;
    let (host, _secret) = enrolled(&f).await?;
    retire(&f, &host).await?;
    let path = format!("/hosts/{host}");
    let member = member_cookie(&f).await?;

    // A member is forbidden; so is a cookie request without the exact Origin.
    error(
        f.client
            .delete(f.url(&path))
            .header(header::COOKIE, &member)
            .header(header::ORIGIN, &f.origin)
            .send()
            .await?,
        StatusCode::FORBIDDEN,
        "forbidden",
    )
    .await?;
    error(
        f.client
            .delete(f.url(&path))
            .header(header::COOKIE, &f.cookie)
            .header(header::ORIGIN, "https://untrusted.example")
            .send()
            .await?,
        StatusCode::FORBIDDEN,
        "forbidden",
    )
    .await?;
    // No credentials at all.
    error(
        f.client
            .delete(f.url(&path))
            .header(header::ORIGIN, &f.origin)
            .send()
            .await?,
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
    )
    .await?;
    // Tokens without fleet:operate are told which scope is missing.
    for scopes in [
        vec![ApiTokenScope::FleetRead],
        vec![ApiTokenScope::FleetEnroll, ApiTokenScope::FleetApprove],
    ] {
        let token = mint(&f, &format!("under-scoped-{}", scopes.len()), &scopes).await?;
        let response = token_delete(&f, &path, &token).send().await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body: Value = response.json().await?;
        assert_eq!(body["details"]["kind"], "scope");
        assert_eq!(body["details"]["requiredScope"], "fleet:operate");
    }
    // A token owned by a user who is no longer an administrator.
    let token = mint(&f, "demoted", &[ApiTokenScope::FleetOperate]).await?;
    sqlx::query("UPDATE users SET role='member' WHERE id='fleet-user'")
        .execute(&f.database)
        .await?;
    error(
        token_delete(&f, &path, &token).send().await?,
        StatusCode::FORBIDDEN,
        "forbidden",
    )
    .await?;
    let deleted_at: Option<i64> =
        sqlx::query_scalar("SELECT deleted_at FROM fleet_hosts WHERE id=?")
            .bind(&host)
            .fetch_one(&f.database)
            .await?;
    assert_eq!(deleted_at, None, "no refused request deleted anything");

    // A malformed id is a bad request, not a lookup.
    sqlx::query("UPDATE users SET role='admin' WHERE id='fleet-user'")
        .execute(&f.database)
        .await?;
    error(
        admin_delete(&f, "/hosts/not-a-uuid").send().await?,
        StatusCode::BAD_REQUEST,
        "invalid_request",
    )
    .await?;
    Ok(())
}
