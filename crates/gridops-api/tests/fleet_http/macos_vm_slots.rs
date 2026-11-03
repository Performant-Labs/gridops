//! The Tart host's macOS VM slots (#95). The limit is part of the host's
//! budget in `GridOps`: an administrator sets it with the other settings, it
//! is pushed to the runner manager, which enforces it at admission, and a
//! pool waiting for a slot shows the manager's reason: its place in the
//! queue and who holds the slots.

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::State,
    routing::{get, put},
};

use super::{placement::seed_pool, *};

/// A runner manager that remembers each policy it is sent.
struct FakeManager {
    url: String,
    policies: Arc<Mutex<Vec<Value>>>,
}

impl FakeManager {
    async fn start() -> Result<Self> {
        let policies = Arc::new(Mutex::new(Vec::<Value>::new()));
        let app =
            Router::new()
                .route(
                    "/v1/health",
                    get(|| async { Json(json!({"status": "ok", "providers": {}})) }),
                )
                .route(
                    "/v1/policy",
                    put(
                        |State(policies): State<Arc<Mutex<Vec<Value>>>>,
                         Json(body): Json<Value>| async move {
                            let slots = body["tartVmSlots"].clone();
                            policies
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .push(body);
                            Json(json!({"tartVmSlots": slots}))
                        },
                    ),
                )
                .with_state(Arc::clone(&policies));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(Self { url, policies })
    }

    fn last_policy(&self) -> Option<Value> {
        self.policies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last()
            .cloned()
    }
}

fn admin(f: &Fixture, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
    f.client
        .request(method, format!("{}/api/v1{path}", f.origin))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin)
}

fn settings_body(slots: Option<i64>) -> Value {
    let mut body = json!({
        "logRetentionDays": 30, "logStorageBudgetMb": 4096, "webhookRetentionDays": 90,
        "auditRetentionDays": 365, "reconcileIntervalSeconds": 30,
        "githubSyncIntervalSeconds": 60, "autoUpdateImages": false, "provisioningPaused": false,
    });
    if let Some(slots) = slots {
        body["macosVmSlots"] = json!(slots);
    }
    body
}

async fn shown(f: &Fixture) -> Result<Value> {
    let settings: Value = admin(f, reqwest::Method::GET, "/settings")
        .send()
        .await?
        .json()
        .await?;
    Ok(settings["data"]["settings"]["macosVmSlots"].clone())
}

#[tokio::test]
async fn the_vm_slot_limit_is_a_gridops_setting_pushed_to_the_manager() -> Result<()> {
    let manager = FakeManager::start().await?;
    let f = Fixture::with_manager(&manager.url).await?;
    assert_eq!(shown(&f).await?, json!(2), "a Mac allows two by default");

    let response = admin(&f, reqwest::Method::PUT, "/settings")
        .json(&settings_body(Some(1)))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(shown(&f).await?, json!(1));
    let policy = manager
        .last_policy()
        .context("the manager was sent the policy")?;
    assert_eq!(policy["tartVmSlots"], 1);

    // macOS allows no more than two, and at least one is needed.
    for refused in [0, 3] {
        let response = admin(&f, reqwest::Method::PUT, "/settings")
            .json(&settings_body(Some(refused)))
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{refused}");
    }
    assert_eq!(
        shown(&f).await?,
        json!(1),
        "a refused value changes nothing"
    );

    // A client that predates the setting leaves it as it is.
    let response = admin(&f, reqwest::Method::PUT, "/settings")
        .json(&settings_body(None))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(shown(&f).await?, json!(1));
    assert_eq!(manager.last_policy().context("policy")?["tartVmSlots"], 1);
    Ok(())
}

#[tokio::test]
async fn a_pool_waiting_for_a_vm_slot_shows_why() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let reason = "Waiting for a macOS VM slot: position 2 of 2 in the queue. 2 of 2 slots in use (GridOps runners: 1; other software: 1 — runner-warm-1 (ci)).";
    sqlx::query("UPDATE runner_pools SET state='waiting' WHERE id='fleet-pool'")
        .execute(&f.database)
        .await?;
    sqlx::query(
        "INSERT INTO runner_pool_waits (pool_id,reason,updated_at) VALUES ('fleet-pool',?,1)",
    )
    .bind(reason)
    .execute(&f.database)
    .await?;
    let detail: Value = admin(&f, reqwest::Method::GET, "/runner-pools/fleet-pool")
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(detail["provisionWaitReason"], reason);
    let list: Value = admin(&f, reqwest::Method::GET, "/runner-pools")
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(list["items"][0]["provisionWaitReason"], reason);

    // Shown only while the pool is waiting.
    sqlx::query("UPDATE runner_pools SET state='active' WHERE id='fleet-pool'")
        .execute(&f.database)
        .await?;
    let detail: Value = admin(&f, reqwest::Method::GET, "/runner-pools/fleet-pool")
        .send()
        .await?
        .json()
        .await?;
    assert!(detail["provisionWaitReason"].is_null(), "{detail}");
    Ok(())
}
