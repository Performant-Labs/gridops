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

/// #102: a pool whose circuit is still open from earlier failures, waiting
/// in the slot queue, is `waiting` (the reconciler puts the wait first). The
/// API returns its reason, and still says the circuit is open.
#[tokio::test]
async fn a_pool_waiting_for_a_vm_slot_with_an_open_circuit_shows_why() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let reason = "Waiting for a macOS VM slot: position 1 of 1 in the queue. 2 of 2 slots in use (GridOps runners: 1; other software: 1 — runner-warm-1 (ci)).";
    sqlx::query(
        "UPDATE runner_pools SET state='waiting',provision_failure_count=3,provision_circuit_open=1,provision_retry_at=NULL WHERE id='fleet-pool'",
    )
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
    assert_eq!(detail["state"], "waiting", "{detail}");
    assert_eq!(detail["provisionWaitReason"], reason);
    assert_eq!(detail["provisionCircuitOpen"], true);
    assert_eq!(detail["provisionFailureCount"], 3);
    let list: Value = admin(&f, reqwest::Method::GET, "/runner-pools")
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(list["items"][0]["provisionWaitReason"], reason);
    assert_eq!(list["items"][0]["provisionCircuitOpen"], true);
    Ok(())
}

// ---- The Tart host card (#95 phase 2): GET /api/v1/overview/tart-host ----

/// Each example in the shared contract fixture: what the runner manager's
/// health says under `providers.tart`, and what the API answers. The web
/// app's tests parse and render the same bodies.
const TART_HOST_CONTRACTS: &str =
    include_str!("../../../../tests/fixtures/tart-host-contracts.json");

/// Two installations: the signed-in user belongs to the first only, so
/// `other-pool` (and its runner) is a pool they cannot see.
async fn seed_tart_pools(f: &Fixture) -> Result<()> {
    let at = |iso: &str| -> Result<i64> {
        Ok(chrono::DateTime::parse_from_rfc3339(iso)?.timestamp_millis())
    };
    sqlx::raw_sql(
        "INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
           VALUES (1,1,'octo-org','Organization','Organization','all',1,1),
                  (2,2,'other-org','Organization','Organization','all',1,1);
         INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,created_at,updated_at)
           VALUES ('mac-pool',1,'macos-release','organization','tart','[\"tart\"]','[\"self-hosted\"]','macos','runner:fixture','macos',1,1),
                  ('other-pool',2,'other-org-macos','organization','tart','[\"tart\"]','[\"self-hosted\"]','macos','runner:fixture','macos',1,1);
         INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('fleet-user',1,'read',1);",
    )
    .execute(&f.database)
    .await?;
    for (id, pool, name, created) in [
        (
            "runner-mac-1",
            "mac-pool",
            "macos-release-1a2b3c4d",
            "2026-11-03T16:00:00Z",
        ),
        (
            "runner-mac-2",
            "mac-pool",
            "macos-release-5e6f7a8b",
            "2026-11-03T16:04:00Z",
        ),
        (
            "runner-other-1",
            "other-pool",
            "other-org-macos-9c8d7e6f",
            "2026-11-03T15:58:00Z",
        ),
    ] {
        sqlx::query("INSERT INTO runners (id,pool_id,name,provider,status,created_at,updated_at) VALUES (?,?,?,'tart','starting',?,?)")
            .bind(id)
            .bind(pool)
            .bind(name)
            .bind(at(created)?)
            .bind(at(created)?)
            .execute(&f.database)
            .await?;
    }
    Ok(())
}

async fn tart_host(f: &Fixture) -> Result<Value> {
    let response = admin(f, reqwest::Method::GET, "/overview/tart-host")
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(response.json().await?)
}

/// Who holds each slot (a runner with its pool and name, a VM another
/// program runs, a start in progress), the queue with the waiting pools in
/// order, the agent's cap warning, and the states without slots: every
/// example of the shared contract fixture.
#[tokio::test]
async fn the_tart_host_card_shows_who_holds_the_slots_and_who_waits() -> Result<()> {
    let examples: Vec<Value> = serde_json::from_str(TART_HOST_CONTRACTS)?;
    assert!(examples.len() >= 5);
    for example in examples {
        let manager = FakeManager::with_health(json!({
            "status": "ok",
            "providers": {"docker": {"available": false}, "tart": example["managerTart"]},
        }))
        .await?;
        let f = Fixture::with_manager(&manager.url).await?;
        seed_tart_pools(&f).await?;
        assert_eq!(tart_host(&f).await?, example["body"], "{}", example["name"]);
    }
    Ok(())
}

/// A pool name the caller may not see is never sent, in the holders or the
/// queue; nor is the name of a runner in such a pool.
#[tokio::test]
async fn the_tart_host_card_does_not_name_pools_the_caller_cannot_see() -> Result<()> {
    let examples: Vec<Value> = serde_json::from_str(TART_HOST_CONTRACTS)?;
    for example in examples.iter().take(2) {
        let manager = FakeManager::with_health(json!({
            "status": "ok", "providers": {"tart": example["managerTart"]},
        }))
        .await?;
        let f = Fixture::with_manager(&manager.url).await?;
        seed_tart_pools(&f).await?;
        let body = serde_json::to_string(&tart_host(&f).await?)?;
        for secret in ["other-pool", "other-org-macos", "runner-other-1"] {
            assert!(!body.contains(secret), "{secret} leaked: {body}");
        }

        // Once the user may see the other installation, its pool is named.
        sqlx::query("INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('fleet-user',2,'read',1)")
            .execute(&f.database)
            .await?;
        let body = serde_json::to_string(&tart_host(&f).await?)?;
        assert!(body.contains("other-org-macos"), "{body}");
    }
    Ok(())
}

/// A start the manager names but whose runner `GridOps` has no record of yet
/// still shows its pool; the counts the manager reports but does not name
/// (an older manager) are listed as unidentified holders, so the card never
/// shows fewer holders than slots in use.
#[tokio::test]
async fn the_tart_host_card_lists_every_slot_in_use_even_unnamed() -> Result<()> {
    let manager = FakeManager::with_health(json!({
        "status": "ok",
        "providers": {"tart": {"available": true, "vmSlots": {
            "limit": 2, "inUse": 2, "free": 0, "gridops": 0, "runnerIds": [],
            "starting": 2, "startingVms": [{"runnerId": "not-yet-recorded", "poolId": "mac-pool"}],
            "external": [], "detection": "processes", "queue": [],
            "agentMaxRunners": 2, "warning": null,
        }}},
    }))
    .await?;
    let f = Fixture::with_manager(&manager.url).await?;
    seed_tart_pools(&f).await?;
    let body = tart_host(&f).await?;
    assert_eq!(
        body["slots"]["holders"],
        json!([
            {"kind": "starting", "runnerId": null, "runnerName": null, "poolId": "mac-pool",
             "poolName": "macos-release", "poolHidden": false, "since": null},
            {"kind": "starting", "runnerId": null, "runnerName": null, "poolId": null,
             "poolName": null, "poolHidden": false, "since": null},
        ]),
        "{body}"
    );
    Ok(())
}

/// With the runner manager down `GridOps` cannot tell whether there is a Tart
/// agent; the answer says so, with the reason, and the slot setting.
#[tokio::test]
async fn the_tart_host_card_says_when_the_runner_manager_cannot_be_asked() -> Result<()> {
    // A manager that does not answer (nothing listens on port 9). Any
    // signed-in user and any `pools:read` token reads this message, so it
    // must not carry the request error, which names the runner manager's
    // internal address.
    let unreachable = Fixture::with_manager("http://127.0.0.1:9").await?;
    let body = tart_host(&unreachable).await?;
    assert_eq!(body["tartAgent"], "unknown", "{body}");
    assert_eq!(
        body["message"], "The runner manager could not be reached.",
        "{body}"
    );
    assert!(!body.to_string().contains("127.0.0.1"), "{body}");
    assert_eq!(body["slotSetting"], 2);
    assert!(body["slots"].is_null());

    // A manager GridOps has no token for: the same answer.
    let f = Fixture::new().await?;
    let body = tart_host(&f).await?;
    assert_eq!(body["tartAgent"], "unknown", "{body}");
    assert_eq!(
        body["message"], "The runner manager could not be reached.",
        "{body}"
    );
    assert_eq!(body["slotSetting"], 2);
    assert!(body["slots"].is_null());

    // Signed out: nothing about the host at all.
    let response = f
        .client
        .get(format!("{}/api/v1/overview/tart-host", f.origin))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<Value>().await?,
        json!({"authenticated": false, "tartAgent": "unknown", "message": null, "slotSetting": null, "slots": null})
    );
    Ok(())
}

/// Running runners are listed oldest first, whatever order the agent names
/// them in (it sorts runner ids, which says nothing about age).
#[tokio::test]
async fn the_tart_host_card_lists_running_runners_oldest_first() -> Result<()> {
    let manager = FakeManager::with_health(json!({
        "status": "ok",
        "providers": {"tart": {"available": true, "vmSlots": {
            "limit": 2, "inUse": 2, "free": 0, "gridops": 2,
            "runnerIds": ["runner-mac-2", "runner-mac-1"],
            "starting": 0, "startingVms": [], "external": [], "detection": "processes",
            "queue": [], "agentMaxRunners": 2, "warning": null,
        }}},
    }))
    .await?;
    let f = Fixture::with_manager(&manager.url).await?;
    seed_tart_pools(&f).await?;
    let body = tart_host(&f).await?;
    let names = body["slots"]["holders"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|holder| holder["runnerName"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            json!("macos-release-1a2b3c4d"),
            json!("macos-release-5e6f7a8b")
        ],
        "{body}"
    );
    Ok(())
}
