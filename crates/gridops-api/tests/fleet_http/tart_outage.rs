//! Reconciling a macOS pool from the API while the Tart agent is out of
//! reach (the Mac sleeps). The runner manager then lists no Tart runners and
//! names `tart` under `unavailableProviders`; the Mac's runners are not gone,
//! only out of sight. The API's own pool reconcile compares its runners
//! against that list exactly as the reconciler does, so it must not take
//! them for missing either: not mark them failed, not queue their GitHub
//! deregistration, not ask the manager to remove their VMs.

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{Path, RawQuery},
    http::StatusCode as Status,
    response::IntoResponse as _,
    routing::{delete, get},
};

use super::*;

const MAC_CONTAINER: &str = "tart-0123456789abcdef0123456789abcdef";

/// Whether a runner list request opts in to the partial list (review
/// finding 1 of PR #123): `GET /v1/runners?partial=true`.
fn asks_for_partial_list(query: Option<&str>) -> bool {
    query
        .unwrap_or_default()
        .split('&')
        .any(|pair| pair == "partial=true")
}

/// What the runner manager answers for a request that had to reach the
/// Tart agent while it is out of reach.
fn tart_agent_unavailable() -> (Status, Json<Value>) {
    (
        Status::SERVICE_UNAVAILABLE,
        Json(json!({
            "code": "tart_agent_unavailable",
            "error": "Tart agent is unavailable",
        })),
    )
}

/// A runner manager whose Tart agent is out of reach. As the real manager
/// does, it gives the partial list only to a caller that asks for it, and
/// fails the whole list for any other (an API from before #122 would take
/// every runner missing from a partial list for gone). It remembers the
/// runner lists and removals it is asked for.
struct MacAsleep {
    url: String,
    removed: Arc<Mutex<Vec<String>>>,
    list_queries: Arc<Mutex<Vec<String>>>,
}

impl MacAsleep {
    async fn start() -> Result<Self> {
        let removed = Arc::new(Mutex::new(Vec::<String>::new()));
        let removals = Arc::clone(&removed);
        let list_queries = Arc::new(Mutex::new(Vec::<String>::new()));
        let lists = Arc::clone(&list_queries);
        let app = Router::new()
            .route(
                "/v1/runners",
                get(move |RawQuery(query): RawQuery| {
                    let lists = Arc::clone(&lists);
                    async move {
                        lists
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(query.clone().unwrap_or_default());
                        if !asks_for_partial_list(query.as_deref()) {
                            return tart_agent_unavailable().into_response();
                        }
                        Json(json!({
                            "runners": [],
                            "unavailableProviders": ["tart"],
                            "providerErrors": { "tart": {
                                "kind": "unreachable",
                                "message": "Tart agent is unavailable: connection timed out",
                            }},
                        }))
                        .into_response()
                    }
                }),
            )
            .route(
                "/v1/runners/{id}",
                delete(move |Path(id): Path<String>| {
                    let removals = Arc::clone(&removals);
                    async move {
                        removals
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(id);
                        tart_agent_unavailable()
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(Self {
            url,
            removed,
            list_queries,
        })
    }

    fn removed(&self) -> Vec<String> {
        self.removed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn list_queries(&self) -> Vec<String> {
        self.list_queries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// A macOS pool with one online runner on the Mac, registered with GitHub.
async fn seed_mac_pool(f: &Fixture) -> Result<()> {
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
           VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
         INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,desired_count,created_at,updated_at)
           VALUES ('pool-mac',1,'mac','organization','tart','[\"tart\"]','[\"self-hosted\"]','base','','base',1,1,1);
         INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('fleet-user',1,'admin',1);
         INSERT INTO runners (id,pool_id,name,provider,os,architecture,status,ephemeral,container_id,github_runner_id,last_heartbeat_at,created_at,updated_at)
           VALUES ('runner-mac-1','pool-mac','mac-abcd1234','tart','macOS','ARM64','online',1,'{MAC_CONTAINER}',4242,1,1,1);"
    )))
    .execute(&f.database)
    .await?;
    Ok(())
}

#[tokio::test]
async fn reconciling_a_mac_pool_while_the_mac_cannot_be_asked_leaves_its_runners_alone()
-> Result<()> {
    let manager = MacAsleep::start().await?;
    let f = Fixture::with_manager(&manager.url).await?;
    seed_mac_pool(&f).await?;

    let response = f
        .client
        .post(format!("{}/api/v1/runner-pools/pool-mac/action", f.origin))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin)
        .json(&json!({ "action": "reconcile" }))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    // A reconcile that failed outright would leave the runner alone too, and
    // prove nothing: the partial list must have been asked for and used.
    assert!(status.is_success(), "reconcile answered {status}: {body}");

    let (runner_status, deleted_at): (String, Option<i64>) =
        sqlx::query_as("SELECT status,deleted_at FROM runners WHERE id='runner-mac-1'")
            .fetch_one(&f.database)
            .await?;
    assert_eq!(
        (runner_status.as_str(), deleted_at),
        ("online", None),
        "the Mac's runner was taken for missing (reconcile answered {status}: {body})"
    );
    let cleanups: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM github_runner_cleanup WHERE id='runner-mac-1'")
            .fetch_one(&f.database)
            .await?;
    assert_eq!(cleanups, 0, "a GitHub deregistration was queued");
    assert_eq!(manager.removed(), Vec::<String>::new());
    Ok(())
}

// ---------------------------------------------------------------------------
// Round 2 (review of PR #123).
// ---------------------------------------------------------------------------

/// Review finding 1, the API's side: its pool reconcile asks the manager for
/// the partial list, so it goes on for the pool's Docker runners while the
/// Mac sleeps; a manager fails the list whole for a caller that does not ask.
#[tokio::test]
async fn reconciling_a_pool_asks_the_manager_for_the_partial_runner_list() -> Result<()> {
    let manager = MacAsleep::start().await?;
    let f = Fixture::with_manager(&manager.url).await?;
    seed_mac_pool(&f).await?;

    let (status, body) = reconcile(&f).await?;

    let queries = manager.list_queries();
    assert!(!queries.is_empty(), "the runner list was not asked for");
    assert!(
        queries
            .iter()
            .all(|query| asks_for_partial_list(Some(query))),
        "a runner list did not ask for the partial list: {queries:?} (reconcile answered {status}: {body})"
    );
    assert!(status.is_success(), "reconcile answered {status}: {body}");
    Ok(())
}

async fn reconcile(f: &Fixture) -> Result<(reqwest::StatusCode, String)> {
    let response = f
        .client
        .post(format!("{}/api/v1/runner-pools/pool-mac/action", f.origin))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin)
        .json(&json!({ "action": "reconcile" }))
        .send()
        .await?;
    let status = response.status();
    Ok((status, response.text().await.unwrap_or_default()))
}

/// Reconciles `pool-mac` while the Mac sleeps, after `change` (SQL) to the
/// seeded pool and runner, and checks the runner was left exactly as it was:
/// with `status`, not deleted, no GitHub deregistration queued, its VM not
/// asked to be removed. The reconcile itself must succeed, or it would leave
/// the runner alone without testing anything.
async fn assert_reconcile_leaves_the_mac_runner_alone(change: &str, status: &str) -> Result<()> {
    let manager = MacAsleep::start().await?;
    let f = Fixture::with_manager(&manager.url).await?;
    seed_mac_pool(&f).await?;
    sqlx::raw_sql(sqlx::AssertSqlSafe(change.to_owned()))
        .execute(&f.database)
        .await?;

    let (answered, body) = reconcile(&f).await?;
    assert!(
        answered.is_success(),
        "reconcile answered {answered}: {body}"
    );

    let (runner_status, deleted_at): (String, Option<i64>) =
        sqlx::query_as("SELECT status,deleted_at FROM runners WHERE id='runner-mac-1'")
            .fetch_one(&f.database)
            .await?;
    assert_eq!(
        (runner_status.as_str(), deleted_at),
        (status, None),
        "the Mac's runner was changed or removed ({body})"
    );
    let cleanups: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM github_runner_cleanup WHERE id='runner-mac-1'")
            .fetch_one(&f.database)
            .await?;
    assert_eq!(cleanups, 0, "a GitHub deregistration was queued ({body})");
    assert_eq!(manager.removed(), Vec::<String>::new(), "{body}");
    Ok(())
}

/// Review finding 2: a runner already marked failed when the Mac went to
/// sleep is out of sight like any other; its cleanup waits for the Mac.
#[tokio::test]
async fn reconcile_leaves_a_failed_mac_runner_for_later_while_the_mac_cannot_be_asked() -> Result<()>
{
    assert_reconcile_leaves_the_mac_runner_alone(
        "UPDATE runners SET status='failed' WHERE id='runner-mac-1';",
        "failed",
    )
    .await
}

/// Review finding 2: an outdated runner on the sleeping Mac is not rotated.
#[tokio::test]
async fn reconcile_does_not_rotate_an_outdated_mac_runner_while_the_mac_cannot_be_asked()
-> Result<()> {
    assert_reconcile_leaves_the_mac_runner_alone(
        "UPDATE runner_pools SET configuration_version=2 WHERE id='pool-mac';
         UPDATE runners SET configuration_version=1 WHERE id='runner-mac-1';",
        "online",
    )
    .await
}

/// Review finding 2: a paused pool drains its idle runners, but not one on
/// the sleeping Mac.
#[tokio::test]
async fn reconcile_does_not_drain_a_paused_pools_mac_runner_while_the_mac_cannot_be_asked()
-> Result<()> {
    assert_reconcile_leaves_the_mac_runner_alone(
        "UPDATE runner_pools SET paused=1 WHERE id='pool-mac';",
        "online",
    )
    .await
}

/// Review finding 2: a pool with more runners than it wants trims the
/// excess, but not a runner on the sleeping Mac.
#[tokio::test]
async fn reconcile_does_not_trim_an_excess_mac_runner_while_the_mac_cannot_be_asked() -> Result<()>
{
    assert_reconcile_leaves_the_mac_runner_alone(
        "UPDATE runner_pools SET desired_count=0 WHERE id='pool-mac';",
        "online",
    )
    .await
}

// ---------------------------------------------------------------------------
// Round 3 (second review of PR #123).
// ---------------------------------------------------------------------------

/// Makes the fixture's user a member who may only read the seeded
/// installation's pools, as a `pools:read` user.
async fn as_pools_reader(f: &Fixture) -> Result<()> {
    sqlx::raw_sql(
        "UPDATE users SET role='member' WHERE id='fleet-user';
         UPDATE user_installations SET permission='read' WHERE user_id='fleet-user';",
    )
    .execute(&f.database)
    .await?;
    Ok(())
}

/// `GET path` as the fixture's user: the status and the JSON body.
async fn read(f: &Fixture, path: &str) -> Result<(reqwest::StatusCode, Value)> {
    let response = f
        .client
        .get(format!("{}{path}", f.origin))
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// Round 3, finding 1a, the API's side of the chosen design: the reconciler
/// writes the Mac going away into the events of each pool that needs it,
/// with the pool's `pool_id` (the reconciler's
/// `every_pool_that_needs_the_mac_records_it_going_away_and_coming_back`).
/// A `pools:read` user then sees it in that pool's events tab and in the
/// overview's activity.
#[tokio::test]
async fn a_pools_reader_sees_the_mac_going_away_in_the_pools_events_and_the_overview() -> Result<()>
{
    let f = Fixture::new().await?;
    seed_mac_pool(&f).await?;
    as_pools_reader(&f).await?;
    sqlx::query(
        "INSERT INTO runner_events (id,pool_id,level,event,message,metadata,created_at)
         VALUES ('event-mac-away','pool-mac','warning','Runner provider unavailable',
                 'macOS (Tart) provider unavailable: its agent cannot be reached.',
                 '{\"provider\":\"tart\",\"kind\":\"unreachable\"}',?)",
    )
    .bind(chrono::Utc::now().timestamp_millis())
    .execute(&f.database)
    .await?;

    let (status, events) = read(&f, "/api/v1/runner-pools/pool-mac/events").await?;
    assert!(
        status.is_success(),
        "pool events answered {status}: {events}"
    );
    assert!(
        events["items"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|item| item["event"] == "Runner provider unavailable"),
        "the pool's events tab does not show the Mac going away: {events}"
    );
    let (status, overview) = read(&f, "/api/v1/overview").await?;
    assert!(
        status.is_success(),
        "overview answered {status}: {overview}"
    );
    assert!(
        overview["activity"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|item| item["event"] == "Runner provider unavailable"),
        "the overview's activity does not show the Mac going away: {overview}"
    );
    Ok(())
}

/// Round 3, finding 1b: a pool the reconciler skipped as
/// `provider-unavailable` keeps its reason where the API finds a pool's
/// reason (`runner_pool_waits`, the reconciler's
/// `a_skipped_mac_pool_says_which_provider_is_unavailable_and_how`). A
/// `pools:read` user gets it as `provisionWaitReason`, on the pool page and
/// in the pool list, as for a waiting pool. Today the API gives it only in
/// the state `waiting`.
#[tokio::test]
async fn a_pools_reader_gets_the_reason_a_pools_provider_is_unavailable() -> Result<()> {
    const REASON: &str = "macOS (Tart) provider unavailable: its agent cannot be reached.";
    let f = Fixture::new().await?;
    seed_mac_pool(&f).await?;
    as_pools_reader(&f).await?;
    sqlx::query("UPDATE runner_pools SET state='provider-unavailable' WHERE id='pool-mac'")
        .execute(&f.database)
        .await?;
    sqlx::query(
        "INSERT INTO runner_pool_waits (pool_id,reason,updated_at) VALUES ('pool-mac',?,1)",
    )
    .bind(REASON)
    .execute(&f.database)
    .await?;

    let (status, pool) = read(&f, "/api/v1/runner-pools/pool-mac").await?;
    assert!(status.is_success(), "the pool answered {status}: {pool}");
    assert_eq!(pool["state"], "provider-unavailable", "{pool}");
    assert_eq!(
        pool["provisionWaitReason"], REASON,
        "the pool page does not say why its provider is unavailable: {pool}"
    );
    let (status, pools) = read(&f, "/api/v1/runner-pools").await?;
    assert!(
        status.is_success(),
        "the pool list answered {status}: {pools}"
    );
    let listed = pools["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|item| item["id"] == "pool-mac")
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(
        listed["provisionWaitReason"], REASON,
        "the pool list does not say why its provider is unavailable: {pools}"
    );
    Ok(())
}
