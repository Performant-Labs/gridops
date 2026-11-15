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
    routing::{delete, get, post, put},
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

// ---------------------------------------------------------------------------
// Round 3, GREEN review B2: a runner manager from before round 3 (0.2.0).
// ---------------------------------------------------------------------------

/// The Tart agent's address, as a 0.2.0 runner manager's errors name it.
const AGENT_HOST: &str = "192.0.2.10";
const AGENT_PORT: &str = "8787";

/// Whether `text` names the Tart agent's URL, host or port.
fn names_the_agent(text: &str) -> bool {
    text.contains("http://") || text.contains(AGENT_HOST) || text.contains(AGENT_PORT)
}

/// What a 0.2.0 runner manager answers to any call it could not pass on to
/// the sleeping Mac's agent: reqwest's error, which names the agent's URL.
fn old_tart_agent_unavailable(path: &str) -> axum::response::Response {
    (
        Status::SERVICE_UNAVAILABLE,
        Json(json!({
            "code": "tart_agent_unavailable",
            "error": format!(
                "Tart agent is unavailable: error sending request for url (http://{AGENT_HOST}:{AGENT_PORT}/{path})"
            ),
        })),
    )
        .into_response()
}

/// A 0.2.0 runner manager while the Mac sleeps. It ignores `partial=true`
/// and fails the whole runner list, and fails every call that reaches the
/// agent, naming the agent's URL as that manager does. Admission and a
/// runner start fail the same way when `start_fails` / `admission_fails`
/// say so: the API sends those only for Docker today, so they guard the
/// one path all manager errors take (`manager_json`). It remembers each
/// call, so a test knows its request reached the manager.
struct OldManagerMacAsleep {
    url: String,
    calls: Arc<Mutex<Vec<String>>>,
}

impl OldManagerMacAsleep {
    async fn start(admission_fails: bool) -> Result<Self> {
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = Arc::clone(&calls);
        let record = move |call: String| {
            seen.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(call);
        };
        let (r1, r2, r3, r4, r5, r6) = (
            record.clone(),
            record.clone(),
            record.clone(),
            record.clone(),
            record.clone(),
            record,
        );
        let app = Router::new()
            .route(
                "/v1/health",
                get(|| async {
                    Json(json!({
                        "status": "degraded",
                        "providers": {"tart": {"available": false, "error": "Tart agent is unavailable."}},
                    }))
                }),
            )
            .route("/v1/policy", put(|| async { Json(json!({})) }))
            .route(
                "/v1/runners",
                get(move || {
                    r1("list".into());
                    async { old_tart_agent_unavailable("v1/runners") }
                })
                .post(move || {
                    r2("start".into());
                    async { old_tart_agent_unavailable("v1/runners") }
                }),
            )
            .route(
                "/v1/runners/{id}",
                delete(move |Path(id): Path<String>| {
                    r3(format!("removal {id}"));
                    async move { old_tart_agent_unavailable(&format!("v1/runners/{id}")) }
                }),
            )
            .route(
                "/v1/runners/{id}/logs",
                get(move |Path(id): Path<String>| {
                    r4(format!("logs {id}"));
                    async move { old_tart_agent_unavailable(&format!("v1/runners/{id}/logs")) }
                }),
            )
            .route(
                "/v1/runners/{id}/{action}",
                post(move |Path((id, action)): Path<(String, String)>| {
                    r5(format!("{action} {id}"));
                    async move {
                        old_tart_agent_unavailable(&format!("v1/runners/{id}/{action}"))
                    }
                }),
            )
            .route(
                "/v1/admissions",
                post(move || {
                    r6("admission".into());
                    async move {
                        if admission_fails {
                            old_tart_agent_unavailable("v1/capacity")
                        } else {
                            Json(json!({ "leaseId": "lease-1" })).into_response()
                        }
                    }
                }),
            )
            .route(
                "/v1/admissions/{lease_id}",
                delete(|| async { Json(json!({})) }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(Self { url, calls })
    }

    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// `method path` as the fixture's admin, with a JSON body: the status and
/// the body as text.
async fn send(
    f: &Fixture,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<(reqwest::StatusCode, String)> {
    let mut request = f
        .client
        .request(method, format!("{}{path}", f.origin))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await?;
    let status = response.status();
    Ok((status, response.text().await.unwrap_or_default()))
}

/// Checks that `what` reached the manager (`call`), that the API answered
/// it with the manager's 503, and that neither that answer nor anything the
/// API keeps and serves (each runner's failure reason, each event's message
/// and metadata) names the agent's URL, host or port.
async fn assert_the_agent_is_not_named(
    f: &Fixture,
    manager: &OldManagerMacAsleep,
    what: &str,
    call: &str,
    (status, body): (reqwest::StatusCode, String),
) -> Result<()> {
    assert!(
        manager.calls().iter().any(|made| made.starts_with(call)),
        "the setup: {what} did not reach the manager ({call}); it made {:?} and the API answered {status}: {body}",
        manager.calls()
    );
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "the setup: {what} did not fail with the manager's 503: {body}"
    );
    assert!(
        !names_the_agent(&body),
        "the API's answer to {what} names the agent's address: {body}"
    );
    let kept: Vec<String> = sqlx::query_scalar(
        "SELECT COALESCE(failure_reason,'') FROM runners
         UNION ALL SELECT event||' '||COALESCE(message,'')||' '||COALESCE(metadata,'') FROM runner_events",
    )
    .fetch_all(&f.database)
    .await?;
    let leaks = kept
        .iter()
        .filter(|text| names_the_agent(text))
        .collect::<Vec<_>>();
    assert!(
        leaks.is_empty(),
        "after {what}, a failure reason or event names the agent's address: {leaks:?}"
    );
    Ok(())
}

/// B2: with a 0.2.0 manager the API passes the manager's error text on as
/// its own 503 (`manager_json`), so a removal, a stop or another control
/// action on a Mac runner while the Mac sleeps must not name the agent.
#[tokio::test]
async fn with_an_older_manager_removing_or_controlling_a_mac_runner_does_not_name_the_agent()
-> Result<()> {
    for action in ["delete", "stop", "pause", "resume", "start", "restart"] {
        let manager = OldManagerMacAsleep::start(false).await?;
        let f = Fixture::with_manager(&manager.url).await?;
        seed_mac_pool(&f).await?;
        // A persistent runner, so `start` and `restart` are not refused
        // before they reach the manager.
        sqlx::raw_sql(
            "INSERT OR REPLACE INTO settings (key,value,updated_at) VALUES ('builtinRunnerHostEnabled','true',1);
             UPDATE runners SET ephemeral=0 WHERE id='runner-mac-1';",
        )
        .execute(&f.database)
        .await?;
        let answer = send(
            &f,
            reqwest::Method::POST,
            "/api/v1/runners/runner-mac-1/action",
            Some(json!({ "action": action })),
        )
        .await?;
        let call = if action == "delete" {
            "removal"
        } else {
            action
        };
        assert_the_agent_is_not_named(&f, &manager, &format!("a runner {action}"), call, answer)
            .await?;
    }
    Ok(())
}

/// B2, the same defect in the other path a manager error takes
/// (`manager_get`, which passes the manager's whole body on): reading a Mac
/// runner's logs, or following them, while the Mac sleeps.
#[tokio::test]
async fn with_an_older_manager_reading_a_mac_runners_logs_does_not_name_the_agent() -> Result<()> {
    for path in [
        "/api/v1/runners/runner-mac-1/logs",
        "/api/v1/runners/runner-mac-1/logs/stream?tail=10",
    ] {
        let manager = OldManagerMacAsleep::start(false).await?;
        let f = Fixture::with_manager(&manager.url).await?;
        seed_mac_pool(&f).await?;
        let answer = send(&f, reqwest::Method::GET, path, None).await?;
        assert_the_agent_is_not_named(&f, &manager, &format!("GET {path}"), "logs", answer).await?;
    }
    Ok(())
}

/// B2: a 0.2.0 manager fails the whole runner list while the Mac sleeps, and
/// the API's pool reconcile passes that failure on: it must not name the
/// agent.
#[tokio::test]
async fn with_an_older_manager_reconciling_a_mac_pool_does_not_name_the_agent() -> Result<()> {
    let manager = OldManagerMacAsleep::start(false).await?;
    let f = Fixture::with_manager(&manager.url).await?;
    seed_mac_pool(&f).await?;
    let answer = reconcile(&f).await?;
    assert_the_agent_is_not_named(&f, &manager, "a pool reconcile", "list", answer).await
}

/// A Docker pool on the built-in host that wants one runner more than it
/// has, so a reconcile starts one in the request.
async fn seed_docker_pool(f: &Fixture) -> Result<()> {
    sqlx::raw_sql(
        "INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
           VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
         INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,desired_count,ephemeral,runner_group_id,created_at,updated_at)
           VALUES ('pool-docker',1,'docker','organization','docker','[\"docker\"]','[\"self-hosted\"]','ubuntu','ghcr.io/actions/actions-runner:latest','',1,0,1,1,1);
         INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('fleet-user',1,'admin',1);
         INSERT OR REPLACE INTO settings (key,value,updated_at) VALUES ('builtinRunnerHostEnabled','true',1);",
    )
    .execute(&f.database)
    .await?;
    Ok(())
}

/// B2, admission: the API's own start asks the manager for capacity first
/// (`reserve_runner_capacity`), and passes its refusal on. Only Docker
/// starts go this way today, so this guards the shared path.
#[tokio::test]
async fn with_an_older_manager_a_refused_admission_does_not_name_the_agent() -> Result<()> {
    let manager = OldManagerMacAsleep::start(true).await?;
    let f = Fixture::with_manager(&manager.url).await?;
    seed_docker_pool(&f).await?;
    let answer = send(
        &f,
        reqwest::Method::POST,
        "/api/v1/runner-pools/pool-docker/action",
        Some(json!({ "action": "reconcile" })),
    )
    .await?;
    assert_the_agent_is_not_named(&f, &manager, "an admission", "admission", answer).await
}

fn app_private_key() -> Result<String> {
    use rsa::pkcs1::{EncodeRsaPrivateKey as _, LineEnding};
    let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048)?;
    Ok(key.to_pkcs1_pem(LineEnding::LF)?.to_string())
}

/// Stands in for api.github.com as far as a non-ephemeral start needs it:
/// an installation token and a registration token.
async fn fake_github() -> Result<String> {
    let expires = || (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    let app = Router::new()
        .route(
            "/app",
            get(|| async {
                Json(json!({ "slug": "gridops", "owner": { "login": "octo-org", "type": "Organization" } }))
            }),
        )
        .route(
            "/app/installations/{id}/access_tokens",
            post(move || async move { Json(json!({ "token": "installation-token", "expires_at": expires() })) }),
        )
        .route(
            "/orgs/{org}/actions/runners/registration-token",
            post(move || async move { Json(json!({ "token": "registration-token", "expires_at": expires() })) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(url)
}

/// B2, manual start: when the manager fails the start itself, the API keeps
/// its error as the runner's failure reason and in a `Runner provisioning
/// failed` event, both served later, and answers with it. None may name the
/// agent. Only Docker starts go this way today, so this guards the shared
/// path.
#[tokio::test]
async fn with_an_older_manager_a_failed_start_does_not_name_the_agent_in_its_answer_reason_or_event()
-> Result<()> {
    let manager = OldManagerMacAsleep::start(false).await?;
    let github = fake_github().await?;
    let f = Fixture::start(
        Some(manager.url.clone()),
        vec![
            ("GRIDOPS_GITHUB_API_URL".into(), github),
            ("GITHUB_APP_ID".into(), "1".into()),
            ("GITHUB_APP_PRIVATE_KEY".into(), app_private_key()?),
        ],
    )
    .await?;
    seed_docker_pool(&f).await?;
    let answer = send(
        &f,
        reqwest::Method::POST,
        "/api/v1/runner-pools/pool-docker/action",
        Some(json!({ "action": "reconcile" })),
    )
    .await?;
    let (failed, events): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM runners WHERE pool_id='pool-docker' AND status='failed' AND failure_reason IS NOT NULL),
                (SELECT COUNT(*) FROM runner_events WHERE pool_id='pool-docker' AND event='Runner provisioning failed')",
    )
    .fetch_one(&f.database)
    .await?;
    assert_eq!(
        (failed, events),
        (1, 1),
        "the setup: the failed start was not recorded ({:?}: {})",
        answer.0,
        answer.1
    );
    assert_the_agent_is_not_named(&f, &manager, "a runner start", "start", answer).await
}
