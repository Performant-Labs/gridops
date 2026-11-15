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
    extract::Path,
    http::StatusCode as Status,
    routing::{delete, get},
};

use super::*;

const MAC_CONTAINER: &str = "tart-0123456789abcdef0123456789abcdef";

/// A runner manager whose Tart agent is out of reach. It remembers the
/// runner removals it is asked for.
struct MacAsleep {
    url: String,
    removed: Arc<Mutex<Vec<String>>>,
}

impl MacAsleep {
    async fn start() -> Result<Self> {
        let removed = Arc::new(Mutex::new(Vec::<String>::new()));
        let removals = Arc::clone(&removed);
        let app = Router::new()
            .route(
                "/v1/runners",
                get(|| async { Json(json!({ "runners": [], "unavailableProviders": ["tart"] })) }),
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
                        (
                            Status::SERVICE_UNAVAILABLE,
                            Json(json!({
                                "code": "tart_agent_unavailable",
                                "error": "Tart agent is unavailable",
                            })),
                        )
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(Self { url, removed })
    }

    fn removed(&self) -> Vec<String> {
        self.removed
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
