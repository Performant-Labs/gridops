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
        Self::with_health(json!({"status": "ok", "providers": {}})).await
    }

    /// A manager whose `GET /v1/health` answers `health`.
    async fn with_health(health: Value) -> Result<Self> {
        let policies = Arc::new(Mutex::new(Vec::<Value>::new()));
        let app =
            Router::new()
                .route(
                    "/v1/health",
                    get(move || {
                        let health = health.clone();
                        async move { Json(health) }
                    }),
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

/// The manager's view of the Tart host's slots, which it reports when the
/// Tart agent's own runner cap is below the slot limit.
#[tokio::test]
async fn settings_show_the_tart_agents_cap_next_to_the_slot_limit() -> Result<()> {
    let warning = "The Tart agent allows 1 runner (GRIDOPS_TART_MAX_RUNNERS); macOS VM slots are set to 2, so GridOps uses at most 1 of them. Raise GRIDOPS_TART_MAX_RUNNERS to 2 and run the Tart agent installer again, or set the slots to 1.";
    let manager = FakeManager::with_health(json!({
        "status": "ok",
        "providers": {"tart": {"available": true, "vmSlots": {
            "limit": 2, "inUse": 0, "free": 2, "gridops": 0, "starting": 0,
            "external": [], "detection": "processes", "queue": [],
            "agentMaxRunners": 1, "warning": warning,
        }}},
    }))
    .await?;
    let f = Fixture::with_manager(&manager.url).await?;
    let settings: Value = admin(&f, reqwest::Method::GET, "/settings")
        .send()
        .await?
        .json()
        .await?;
    let slots = &settings["data"]["manager"]["tartVmSlots"];
    assert_eq!(slots["agentMaxRunners"], 1, "{settings}");
    assert_eq!(slots["limit"], 2);
    assert_eq!(slots["warning"], warning);
    assert_eq!(settings["data"]["settings"]["macosVmSlots"], 2);

    // No Tart agent: nothing to compare with.
    let manager = FakeManager::start().await?;
    let f = Fixture::with_manager(&manager.url).await?;
    let settings: Value = admin(&f, reqwest::Method::GET, "/settings")
        .send()
        .await?
        .json()
        .await?;
    assert!(
        settings["data"]["manager"]["tartVmSlots"].is_null(),
        "{settings}"
    );
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
