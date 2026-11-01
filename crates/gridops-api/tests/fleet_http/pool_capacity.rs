//! A pool's runner counts can be read and set through the fleet API with a
//! scoped token, and every change leaves an audit record.

use super::placement::seed_pool;
use super::*;
use gridops_core::api_tokens::{ApiTokenScope, ApiTokenService};

const DAY: i64 = 86_400_000;

async fn mint(f: &Fixture, scopes: &[ApiTokenScope]) -> Result<String> {
    let receipt = ApiTokenService::new(f.database.clone())
        .create(
            "fleet-user",
            "capacity",
            scopes,
            gridops_core::now_millis() + DAY,
        )
        .await?;
    Ok(receipt.expose().to_owned())
}

fn put(f: &Fixture, pool: &str, token: &str, body: &Value) -> reqwest::RequestBuilder {
    f.client
        .put(f.url(&format!("/pools/{pool}/capacity")))
        .bearer_auth(token)
        .json(body)
}

async fn counts(f: &Fixture) -> Result<(i64, i64, i64)> {
    Ok(sqlx::query_as(
        "SELECT min_count,desired_count,max_count FROM runner_pools WHERE id='fleet-pool'",
    )
    .fetch_one(&f.database)
    .await?)
}

#[tokio::test]
async fn a_token_reads_and_sets_the_pool_counts_and_the_change_is_audited() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    sqlx::query(
        "UPDATE runner_pools SET min_count=0,desired_count=6,max_count=6 WHERE id='fleet-pool'",
    )
    .execute(&f.database)
    .await?;
    let token = mint(&f, &[ApiTokenScope::FleetRead, ApiTokenScope::FleetOperate]).await?;

    let read = f
        .client
        .get(f.url("/pools/fleet-pool/capacity"))
        .bearer_auth(&token)
        .send()
        .await?;
    assert_eq!(read.status(), StatusCode::OK);
    assert_eq!(
        read.json::<Value>().await?,
        json!({"minCount": 0, "desiredCount": 6, "maxCount": 6})
    );

    let saved = put(
        &f,
        "fleet-pool",
        &token,
        &json!({"minCount": 0, "desiredCount": 4, "maxCount": 4}),
    )
    .send()
    .await?;
    assert_eq!(saved.status(), StatusCode::OK);
    assert_eq!(
        saved.json::<Value>().await?,
        json!({"minCount": 0, "desiredCount": 4, "maxCount": 4})
    );
    assert_eq!(counts(&f).await?, (0, 4, 4));

    let metadata: String = sqlx::query_scalar(
        "SELECT metadata FROM audit_events WHERE action='fleet.pool.capacity_saved' AND target_id='fleet-pool'",
    )
    .fetch_one(&f.database)
    .await?;
    let metadata: Value = serde_json::from_str(&metadata)?;
    assert_eq!(metadata["from"]["max"], 6);
    assert_eq!(metadata["to"]["max"], 4);
    Ok(())
}

#[tokio::test]
async fn setting_the_counts_needs_the_operate_scope() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let before = counts(&f).await?;
    let reader = mint(&f, &[ApiTokenScope::FleetRead]).await?;
    let response = put(
        &f,
        "fleet-pool",
        &reader,
        &json!({"minCount": 0, "desiredCount": 1, "maxCount": 1}),
    )
    .send()
    .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: Value = response.json().await?;
    assert_eq!(body["details"]["requiredScope"], "fleet:operate");
    assert_eq!(counts(&f).await?, before);
    Ok(())
}

#[tokio::test]
async fn impossible_counts_and_unknown_pools_change_nothing() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let before = counts(&f).await?;
    let token = mint(&f, &[ApiTokenScope::FleetOperate]).await?;
    for body in [
        json!({"minCount": 3, "desiredCount": 2, "maxCount": 4}),
        json!({"minCount": 0, "desiredCount": 5, "maxCount": 4}),
        json!({"minCount": -1, "desiredCount": 1, "maxCount": 4}),
        json!({"minCount": 0, "desiredCount": 1, "maxCount": 100_000}),
        json!({"minCount": 0, "desiredCount": 0, "maxCount": 0}),
        json!({"minCount": 0, "desiredCount": 1, "maxCount": 4, "paused": true}),
        json!({"desiredCount": 1, "maxCount": 4}),
    ] {
        let response = put(&f, "fleet-pool", &token, &body).send().await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
    }
    assert_eq!(counts(&f).await?, before);

    let missing = put(
        &f,
        "no-such-pool",
        &token,
        &json!({"minCount": 0, "desiredCount": 1, "maxCount": 4}),
    )
    .send()
    .await?;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let audited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_events WHERE action='fleet.pool.capacity_saved'",
    )
    .fetch_one(&f.database)
    .await?;
    assert_eq!(audited, 0);
    Ok(())
}
