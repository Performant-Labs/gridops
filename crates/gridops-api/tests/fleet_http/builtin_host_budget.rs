//! The built-in runner host's budget (how much of the machine `GridOps` runs
//! on it may fill with runners) is a `GridOps` setting an administrator
//! changes, like a fleet machine's. It is checked against what the host
//! reports, pushed to the runner manager, and audited. While it is not set
//! the manager's own figures (its environment, or 95% of the host) apply.

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::State,
    routing::{get, put},
};

use super::{
    api_tokens::{member_cookie, member_fixture},
    *,
};

const KEY: &str = "builtinRunnerHostBudget";
const AUDIT: &str = "settings.builtin_runner_host_budget_changed";

/// A runner manager on a 16-core, 64,301 MB host that remembers each policy
/// it is sent and answers with the budget that policy puts in force.
struct FakeManager {
    url: String,
    policies: Arc<Mutex<Vec<Value>>>,
}

impl FakeManager {
    async fn start() -> Result<Self> {
        let policies = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route(
                "/v1/health",
                get(|| async {
                    Json(json!({
                        "status": "ok",
                        "hostName": "control-plane-host",
                        "availableCpus": 16.0,
                        "totalMemoryMb": 64_301,
                        "provisioningPaused": false,
                        "builtinHost": {"enabled": true, "dockerAvailable": true},
                        "capacity": {
                            "cpuBudget": 15.2, "memoryBudgetMb": 61_085, "maxRunners": 7,
                            "budgetSource": {"cpu": "default", "memory": "default"},
                        },
                    }))
                }),
            )
            .route(
                "/v1/policy",
                put(
                    |State(policies): State<Arc<Mutex<Vec<Value>>>>, Json(body): Json<Value>| async move {
                        let budget = body["builtinHostBudget"].clone();
                        policies
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(body);
                        let cpu = budget["cpuMillis"].as_f64().map_or(15.2, |millis| millis / 1_000.0);
                        let memory = budget["memoryMib"].as_i64().unwrap_or(61_085);
                        Json(json!({
                            "builtinHostBudget": {
                                "cpuBudget": cpu, "memoryBudgetMb": memory, "maxRunners": 2,
                                "source": {
                                    "cpu": if budget["cpuMillis"].is_null() { "default" } else { "setting" },
                                    "memory": if budget["memoryMib"].is_null() { "default" } else { "setting" },
                                },
                            },
                        }))
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

async fn put_budget(f: &Fixture, body: Value) -> Result<Response> {
    Ok(
        admin(f, reqwest::Method::PUT, "/settings/runner-host/budget")
            .json(&body)
            .send()
            .await?,
    )
}

async fn stored(f: &Fixture) -> Result<Option<Value>> {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
        .bind(KEY)
        .fetch_optional(&f.database)
        .await?;
    Ok(value
        .map(|value| serde_json::from_str(&value))
        .transpose()?)
}

async fn audited(f: &Fixture) -> Result<Vec<(String, Value)>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT actor_label,metadata FROM audit_events WHERE action=? ORDER BY created_at,rowid",
    )
    .bind(AUDIT)
    .fetch_all(&f.database)
    .await?;
    rows.into_iter()
        .map(|(who, metadata)| Ok((who, serde_json::from_str(&metadata)?)))
        .collect()
}

#[tokio::test]
async fn an_administrator_sets_the_budget_and_it_reaches_the_manager_and_the_audit_log()
-> Result<()> {
    let manager = FakeManager::start().await?;
    let f = Fixture::with_manager(&manager.url).await?;

    let settings: Value = admin(&f, reqwest::Method::GET, "/settings")
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        settings["data"]["settings"]["builtinRunnerHostBudget"],
        json!({"cpuMillis": null, "memoryMib": null}),
        "nothing set: the manager's own figures apply"
    );
    assert_eq!(
        settings["data"]["manager"]["capacity"]["budgetSource"],
        json!({"cpu": "default", "memory": "default"})
    );

    let response = put_budget(&f, json!({"cpuMillis": 12_000, "memoryMib": 49_152})).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await?;
    assert_eq!(
        body["budget"],
        json!({"cpuMillis": 12_000, "memoryMib": 49_152})
    );
    assert_eq!(body["changed"], true);
    assert_eq!(body["managerSynced"], true);
    assert_eq!(body["effective"]["cpuBudget"], 12.0);
    assert_eq!(body["effective"]["memoryBudgetMb"], 49_152);
    assert_eq!(
        stored(&f).await?,
        Some(json!({"cpuMillis": 12_000, "memoryMib": 49_152}))
    );
    let policy = manager
        .last_policy()
        .context("the manager was sent the policy")?;
    assert_eq!(
        policy["builtinHostBudget"],
        json!({"cpuMillis": 12_000, "memoryMib": 49_152})
    );
    // The rest of the policy goes with it unchanged.
    assert_eq!(policy["builtinHostEnabled"], false);
    assert_eq!(policy["provisioningPaused"], false);

    // A figure left out is kept; null returns it to the manager's own.
    let body: Value = put_budget(&f, json!({"memoryMib": null}))
        .await?
        .json()
        .await?;
    assert_eq!(
        body["budget"],
        json!({"cpuMillis": 12_000, "memoryMib": null})
    );
    assert_eq!(body["effective"]["source"]["memory"], "default");
    assert_eq!(
        manager.last_policy().context("policy")?["builtinHostBudget"],
        json!({"cpuMillis": 12_000, "memoryMib": null})
    );

    // Up to exactly what the host reports.
    let response = put_budget(&f, json!({"cpuMillis": 16_000, "memoryMib": 64_301})).await?;
    assert_eq!(response.status(), StatusCode::OK);

    // Saying the same again changes and records nothing.
    let again: Value = put_budget(&f, json!({"cpuMillis": 16_000}))
        .await?
        .json()
        .await?;
    assert_eq!(again["changed"], false);

    let audit = audited(&f).await?;
    assert_eq!(audit.len(), 3, "no-op changes are not recorded");
    assert!(audit.iter().all(|(who, _)| who == "fleet-admin"));
    assert_eq!(
        (audit[0].1["before"].clone(), audit[0].1["after"].clone()),
        (
            json!({"cpuMillis": null, "memoryMib": null}),
            json!({"cpuMillis": 12_000, "memoryMib": 49_152})
        )
    );
    assert_eq!(
        audit[1].1["after"],
        json!({"cpuMillis": 12_000, "memoryMib": null})
    );
    Ok(())
}

#[tokio::test]
async fn a_budget_out_of_range_or_malformed_is_refused_and_changes_nothing() -> Result<()> {
    let manager = FakeManager::start().await?;
    let f = Fixture::with_manager(&manager.url).await?;
    for body in [
        json!({}),
        json!({"cpuMillis": 0}),
        json!({"cpuMillis": -1_000}),
        json!({"memoryMib": 0}),
        json!({"cpuMillis": 16_001}),
        json!({"memoryMib": 64_302}),
        json!({"cpuMillis": "lots"}),
        json!({"cpuMillis": 1.5}),
        json!({"cpuMillis": 4_000, "diskBytes": 1}),
    ] {
        let response = put_budget(&f, body.clone()).await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
    }
    let message: Value = put_budget(&f, json!({"cpuMillis": 16_001}))
        .await?
        .json()
        .await?;
    assert!(
        message["error"]
            .as_str()
            .unwrap_or_default()
            .contains("16 cores"),
        "says what the host reports: {message}"
    );
    assert_eq!(stored(&f).await?, None);
    assert!(audited(&f).await?.is_empty());
    assert_eq!(manager.last_policy(), None, "nothing was pushed");
    Ok(())
}

#[tokio::test]
async fn without_the_hosts_figures_a_budget_cannot_be_set_but_can_be_cleared() -> Result<()> {
    // The manager is unreachable, so there is nothing to check a figure against.
    let f = Fixture::new().await?;
    let response = put_budget(&f, json!({"cpuMillis": 4_000})).await?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(stored(&f).await?, None);

    let response = put_budget(&f, json!({"cpuMillis": null, "memoryMib": null})).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await?;
    assert_eq!(body["managerSynced"], false);
    assert_eq!(body["effective"], Value::Null);
    assert_eq!(body["changed"], false, "it was not set to begin with");
    Ok(())
}

#[tokio::test]
async fn only_an_administrator_on_the_same_origin_can_change_the_budget() -> Result<()> {
    let f = member_fixture().await?;
    let cookie = member_cookie(&f).await?;
    let refused = f
        .client
        .put(format!("{}/api/v1/settings/runner-host/budget", f.origin))
        .header(header::COOKIE, &cookie)
        .header(header::ORIGIN, &f.origin)
        .json(&json!({"cpuMillis": null}))
        .send()
        .await?;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    let cross_origin = f
        .client
        .put(format!("{}/api/v1/settings/runner-host/budget", f.origin))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, "https://elsewhere.example")
        .json(&json!({"cpuMillis": null}))
        .send()
        .await?;
    assert!(cross_origin.status().is_client_error());

    let anonymous = f
        .client
        .put(format!("{}/api/v1/settings/runner-host/budget", f.origin))
        .header(header::ORIGIN, &f.origin)
        .json(&json!({"cpuMillis": null}))
        .send()
        .await?;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(stored(&f).await?, None);
    Ok(())
}
