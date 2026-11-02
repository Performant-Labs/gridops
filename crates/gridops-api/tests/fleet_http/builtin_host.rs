//! The built-in runner host is the machine `GridOps` runs on. An administrator
//! turns it off with one setting, and with it off the API starts no runner
//! there, says why in a code, shows pools with nowhere else to run as unable
//! to, and offers no way to put a pool back on the host.

use super::{
    api_tokens::{member_cookie, member_fixture},
    placement::seed_pool,
    *,
};

const SETTING: &str = "builtinRunnerHostEnabled";

fn api(f: &Fixture, path: &str) -> String {
    format!("{}/api/v1{path}", f.origin)
}

fn admin(f: &Fixture, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
    f.client
        .request(method, api(f, path))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin)
}

async fn set(f: &Fixture, enabled: bool) -> Result<Response> {
    Ok(admin(f, reqwest::Method::PUT, "/settings/runner-host")
        .json(&json!({"builtinHostEnabled": enabled}))
        .send()
        .await?)
}

async fn stored(f: &Fixture) -> Result<String> {
    Ok(sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
        .bind(SETTING)
        .fetch_one(&f.database)
        .await?)
}

async fn seed_runner(f: &Fixture) -> Result<()> {
    sqlx::query(
        "INSERT INTO runners (id,pool_id,container_id,name,status,created_at,updated_at)
         VALUES ('host-runner','fleet-pool','0123456789abcdef0123','fleet-1234','online',1,1)",
    )
    .execute(&f.database)
    .await?;
    Ok(())
}

async fn code(response: Response) -> Result<(StatusCode, String)> {
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    Ok((status, body["code"].as_str().unwrap_or_default().to_owned()))
}

#[tokio::test]
async fn an_administrator_changes_the_setting_and_each_change_is_audited() -> Result<()> {
    let f = Fixture::new().await?;
    assert_eq!(stored(&f).await?, "false", "a new install starts off");
    let settings: Value = admin(&f, reqwest::Method::GET, "/settings")
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        settings["data"]["settings"]["builtinRunnerHostEnabled"],
        false
    );

    let on = set(&f, true).await?;
    assert_eq!(on.status(), StatusCode::OK);
    let body: Value = on.json().await?;
    assert_eq!(body["builtinRunnerHostEnabled"], true);
    assert_eq!(body["changed"], true);
    assert_eq!(stored(&f).await?, "true");

    // Saying the same thing again changes and records nothing.
    let again: Value = set(&f, true).await?.json().await?;
    assert_eq!(again["changed"], false);

    let off: Value = set(&f, false).await?.json().await?;
    assert_eq!(off["builtinRunnerHostEnabled"], false);
    assert_eq!(stored(&f).await?, "false");

    let audited: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT actor_label,target_id,metadata FROM audit_events
          WHERE action='settings.builtin_runner_host_changed' ORDER BY created_at,rowid",
    )
    .fetch_all(&f.database)
    .await?;
    assert_eq!(audited.len(), 2, "no-op changes are not recorded");
    assert!(
        audited
            .iter()
            .all(|(who, target, _)| who == "fleet-admin" && target == "gridops")
    );
    let first: Value = serde_json::from_str(&audited[0].2)?;
    assert_eq!(
        (first["before"].clone(), first["after"].clone()),
        (json!(false), json!(true))
    );
    let second: Value = serde_json::from_str(&audited[1].2)?;
    assert_eq!(
        (second["before"].clone(), second["after"].clone()),
        (json!(true), json!(false))
    );
    let by: String = sqlx::query_scalar("SELECT updated_by FROM settings WHERE key=?")
        .bind(SETTING)
        .fetch_one(&f.database)
        .await?;
    assert_eq!(by, "fleet-user");

    // Unknown fields and a missing flag are refused.
    for body in [
        json!({}),
        json!({"builtinHostEnabled": true, "x": 1}),
        json!({"builtinHostEnabled": "yes"}),
    ] {
        let response = admin(&f, reqwest::Method::PUT, "/settings/runner-host")
            .json(&body)
            .send()
            .await?;
        assert!(response.status().is_client_error(), "{body}");
    }
    assert_eq!(stored(&f).await?, "false");
    Ok(())
}

#[tokio::test]
async fn only_an_administrator_can_change_it() -> Result<()> {
    let f = member_fixture().await?;
    let cookie = member_cookie(&f).await?;
    let refused = f
        .client
        .put(api(&f, "/settings/runner-host"))
        .header(header::COOKIE, &cookie)
        .header(header::ORIGIN, &f.origin)
        .json(&json!({"builtinHostEnabled": true}))
        .send()
        .await?;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let anonymous = f
        .client
        .put(api(&f, "/settings/runner-host"))
        .header(header::ORIGIN, &f.origin)
        .json(&json!({"builtinHostEnabled": true}))
        .send()
        .await?;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let foreign = f
        .client
        .put(api(&f, "/settings/runner-host"))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, "http://evil.example")
        .json(&json!({"builtinHostEnabled": true}))
        .send()
        .await?;
    assert!(foreign.status().is_client_error());
    assert_eq!(stored(&f).await?, "false");
    let audited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_events WHERE action='settings.builtin_runner_host_changed'",
    )
    .fetch_one(&f.database)
    .await?;
    assert_eq!(audited, 0);
    Ok(())
}

#[tokio::test]
async fn with_the_host_off_requests_that_would_start_a_runner_there_are_refused_with_a_code()
-> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    seed_runner(&f).await?;
    for action in ["rebuild", "restart", "start", "resume"] {
        let response = admin(&f, reqwest::Method::POST, "/runners/host-runner/action")
            .json(&json!({"action": action}))
            .send()
            .await?;
        assert_eq!(
            code(response).await?,
            (
                StatusCode::CONFLICT,
                "builtin_runner_host_disabled".to_owned()
            ),
            "{action}"
        );
    }
    // The refused rebuild deleted nothing.
    let deleted: Option<i64> =
        sqlx::query_scalar("SELECT deleted_at FROM runners WHERE id='host-runner'")
            .fetch_one(&f.database)
            .await?;
    assert_eq!(deleted, None);

    Ok(())
}

#[tokio::test]
async fn scaling_a_pool_with_the_host_off_starts_nothing_and_marks_it_unable_to_run() -> Result<()>
{
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let scaled = admin(&f, reqwest::Method::POST, "/runner-pools/fleet-pool/action")
        .json(&json!({"action": "scale", "desiredCount": 3}))
        .send()
        .await?;
    assert_eq!(scaled.status(), StatusCode::OK);
    let body: Value = scaled.json().await?;
    assert_eq!(body["provisioned"], 0);
    assert_eq!(body["builtinRunnerHostOff"], true);
    let (state, desired): (String, i64) =
        sqlx::query_as("SELECT state,desired_count FROM runner_pools WHERE id='fleet-pool'")
            .fetch_one(&f.database)
            .await?;
    assert_eq!(state, "builtin-host-off");
    assert_eq!(desired, 3, "the target is kept for when it can run");
    let runners: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runners")
        .fetch_one(&f.database)
        .await?;
    assert_eq!(runners, 0);
    Ok(())
}

#[tokio::test]
async fn a_pool_with_nowhere_to_run_says_so_and_a_fleet_pool_does_not() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let detail: Value = admin(&f, reqwest::Method::GET, "/runner-pools/fleet-pool")
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(detail["builtinRunnerHostEnabled"], false);
    assert_eq!(detail["cannotRun"]["code"], "builtin_runner_host_disabled");
    assert!(
        detail["cannotRun"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("built-in runner host is off")
    );
    assert!(detail["runnerHost"].is_null(), "no host budget is shown");
    let list: Value = admin(&f, reqwest::Method::GET, "/runner-pools")
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        list["items"][0]["cannotRun"]["code"],
        "builtin_runner_host_disabled"
    );
    let options: Value = admin(&f, reqwest::Method::GET, "/runner-pools/options")
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(options["builtinRunnerHostEnabled"], false);

    // Turning the host on lifts it; so does running the pool on machines.
    set(&f, true).await?;
    let detail: Value = admin(&f, reqwest::Method::GET, "/runner-pools/fleet-pool")
        .send()
        .await?
        .json()
        .await?;
    assert!(detail["cannotRun"].is_null());
    set(&f, false).await?;
    let saved = admin(&f, reqwest::Method::PUT, "/fleet/pools/fleet-pool/profile")
        .json(&json!({
            "enabled": true, "mode": "ephemeral", "cpuMillis": 1000, "memoryMib": 1024, "diskBytes": 0,
            "platforms": [{"runtimeKind": "docker", "executionOs": "linux", "architecture": "arm64"}],
        }))
        .send()
        .await?;
    assert_eq!(saved.status(), StatusCode::OK);
    let detail: Value = admin(&f, reqwest::Method::GET, "/runner-pools/fleet-pool")
        .send()
        .await?
        .json()
        .await?;
    assert!(detail["cannotRun"].is_null());
    Ok(())
}

#[tokio::test]
async fn disabling_names_the_pools_that_can_no_longer_run() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    set(&f, true).await?;
    seed_runner(&f).await?;
    let off: Value = set(&f, false).await?.json().await?;
    assert_eq!(off["cannotRunPools"][0]["name"], "fleet");
    assert_eq!(off["runnersOnBuiltinHost"], 1);
    let state: String = sqlx::query_scalar("SELECT state FROM runner_pools WHERE id='fleet-pool'")
        .fetch_one(&f.database)
        .await?;
    assert_eq!(state, "builtin-host-off");
    // Turning it on again releases the pool at once.
    set(&f, true).await?;
    let state: String = sqlx::query_scalar("SELECT state FROM runner_pools WHERE id='fleet-pool'")
        .fetch_one(&f.database)
        .await?;
    assert_eq!(state, "active");
    Ok(())
}

#[tokio::test]
async fn creating_a_pool_with_the_host_off_starts_no_runner_and_says_it_cannot_run() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let created = admin(&f, reqwest::Method::POST, "/runner-pools")
        .json(&json!({
            "installationId": 1, "name": "new-pool", "scope": "organization", "mode": "ephemeral",
            "provider": "docker", "providers": ["docker"], "labels": ["gridops"],
            "image": "runner:fixture", "dockerImage": "runner:fixture",
            "desiredCount": 2, "minCount": 0, "maxCount": 3, "autoscalingEnabled": false,
            "queueScaleFactor": 1, "idleTimeoutMinutes": 5, "cpuLimit": 2, "memoryLimitMb": 2048,
            "runnerGroupId": 1,
        }))
        .send()
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let body: Value = created.json().await?;
    assert_eq!(body["provisioned"], json!([]));
    assert_eq!(body["cannotRun"]["code"], "builtin_runner_host_disabled");
    let id = body["id"].as_str().context("pool id")?;
    let (state, runners): (String, i64) = sqlx::query_as(
        "SELECT state,(SELECT COUNT(*) FROM runners WHERE pool_id=runner_pools.id) FROM runner_pools WHERE id=?",
    )
    .bind(id)
    .fetch_one(&f.database)
    .await?;
    assert_eq!((state.as_str(), runners), ("builtin-host-off", 0));
    Ok(())
}

#[tokio::test]
async fn with_the_host_off_a_pool_must_run_on_machines_only() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let linux = json!({"runtimeKind": "docker", "executionOs": "linux", "architecture": "arm64"});
    for (enabled, placement, field) in [
        (true, "host_first", "placement"),
        (true, "machines_first", "placement"),
        (false, "machines", "enabled"),
    ] {
        let response = f
            .client
            .put(f.url("/pools/fleet-pool/profile"))
            .header(header::COOKIE, &f.cookie)
            .header(header::ORIGIN, &f.origin)
            .json(&json!({
                "enabled": enabled, "placement": placement, "mode": "ephemeral",
                "cpuMillis": 1000, "memoryMib": 1024, "diskBytes": 0, "platforms": [linux.clone()],
            }))
            .send()
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{placement} {enabled}"
        );
        let body: Value = response.json().await?;
        assert_eq!(body["code"], "invalid_request");
        assert_eq!(body["details"]["field"], field);
    }
    let profiles: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_execution_profiles")
        .fetch_one(&f.database)
        .await?;
    assert_eq!(profiles, 0, "a refused save changes nothing");

    let accepted = f
        .client
        .put(f.url("/pools/fleet-pool/profile"))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin)
        .json(&json!({
            "enabled": true, "placement": "machines", "mode": "ephemeral",
            "cpuMillis": 1000, "memoryMib": 1024, "diskBytes": 0, "platforms": [linux.clone()],
        }))
        .send()
        .await?;
    assert_eq!(accepted.status(), StatusCode::OK);

    // With the host on, the earlier placements are offered again.
    set(&f, true).await?;
    let host_first = f
        .client
        .put(f.url("/pools/fleet-pool/profile"))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin)
        .json(&json!({
            "enabled": true, "placement": "host_first", "mode": "ephemeral",
            "cpuMillis": 1000, "memoryMib": 1024, "diskBytes": 0, "platforms": [linux],
        }))
        .send()
        .await?;
    assert_eq!(host_first.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn with_the_host_on_requests_reach_the_manager_as_before() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    seed_runner(&f).await?;
    set(&f, true).await?;
    let response = admin(&f, reqwest::Method::POST, "/runners/host-runner/action")
        .json(&json!({"action": "restart"}))
        .send()
        .await?;
    // Ephemeral runners cannot be restarted: past the gate, the old rule answers.
    let (status, code) = code(response).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_ne!(code, "builtin_runner_host_disabled");
    Ok(())
}
