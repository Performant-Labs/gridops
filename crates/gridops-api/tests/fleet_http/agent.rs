//! Runs the real host agent supervisor against the API binary: enrollment,
//! handshake, inventory, and the stops that retrying can't fix.

use gridops_host_agent::{
    client::Client,
    journal::Journal,
    supervisor::{Stop, Supervisor},
};
use tokio::sync::oneshot;

use super::{inventory::control_plane, *};

async fn agent(f: &Fixture) -> Result<(Supervisor, Journal)> {
    let journal =
        Journal::open(&f.directory.join(format!("agent-{}.sqlite", Uuid::new_v4()))).await?;
    let supervisor = Supervisor::new(
        Client::new(&f.origin)?,
        journal.clone(),
        "agent-host".into(),
    )
    .with_heartbeat(Duration::from_millis(200));
    Ok((supervisor, journal))
}

#[tokio::test]
async fn enrolled_agent_reports_inventory_and_stops_cleanly() -> Result<()> {
    let f = Fixture::new().await?;
    control_plane(&f).await?;
    let (supervisor, journal) = agent(&f).await?;
    let code = f.issue("agent-supervisor").await?;
    supervisor
        .enroll(code["code"].as_str().context("code")?, Some("agent-hw"))
        .await?;
    let host = journal
        .identity()
        .await?
        .context("identity")?
        .host_id
        .to_string();
    assert!(supervisor.enroll("reused", None).await.is_err());

    let (stop, stopped) = oneshot::channel::<()>();
    let running = tokio::spawn(async move {
        supervisor
            .run(async {
                let _ = stopped.await;
            })
            .await
    });
    let mut inventory = None;
    for _ in 0..100 {
        inventory = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT last_inventory_at FROM fleet_hosts WHERE id=?",
        )
        .bind(&host)
        .fetch_one(&f.database)
        .await?;
        if inventory.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(inventory.is_some(), "agent never submitted inventory");
    let (enrollment, domains): (String, i64) = sqlx::query_as(
        "SELECT enrollment_state,(SELECT COUNT(*) FROM fleet_observed_domains WHERE host_id=?) FROM fleet_hosts WHERE id=?",
    )
    .bind(&host)
    .bind(&host)
    .fetch_one(&f.database)
    .await?;
    assert_eq!((enrollment.as_str(), domains), ("pending", 1));
    let _ = stop.send(());
    assert_eq!(running.await??, Stop::Shutdown);

    // Restarting while the old session's lease is live replaces the session;
    // only a different boot would mean a copied credential.
    let (restarted, journal) = (
        Supervisor::new(
            Client::new(&f.origin)?,
            journal.clone(),
            "agent-host".into(),
        ),
        journal,
    );
    let (stop, stopped) = oneshot::channel::<()>();
    let running = tokio::spawn(async move {
        restarted
            .run(async {
                let _ = stopped.await;
            })
            .await
    });
    let mut sessions = 0;
    for _ in 0..100 {
        sessions = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM fleet_observation_sessions WHERE host_id=?",
        )
        .bind(&host)
        .fetch_one(&f.database)
        .await?;
        if sessions == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(sessions, 2);
    let integrity: String =
        sqlx::query_scalar("SELECT integrity_state FROM fleet_hosts WHERE id=?")
            .bind(&host)
            .fetch_one(&f.database)
            .await?;
    assert_ne!(integrity, "quarantined");
    let _ = stop.send(());
    assert_eq!(running.await??, Stop::Shutdown);
    drop(journal);
    Ok(())
}

#[tokio::test]
async fn incompatible_server_and_rejected_credentials_stop_the_agent() -> Result<()> {
    let f = Fixture::new().await?;
    control_plane(&f).await?;
    let (supervisor, journal) = agent(&f).await?;
    let code = f.issue("agent-incompatible").await?;
    supervisor
        .enroll(code["code"].as_str().context("code")?, None)
        .await?;

    sqlx::query("UPDATE fleet_control_plane SET protocol_min=2,protocol_max=2")
        .execute(&f.database)
        .await?;
    assert_eq!(
        supervisor.run(std::future::pending()).await?,
        Stop::Incompatible
    );

    sqlx::query("UPDATE fleet_control_plane SET protocol_min=1,protocol_max=1")
        .execute(&f.database)
        .await?;
    let host = journal
        .identity()
        .await?
        .context("identity")?
        .host_id
        .to_string();
    sqlx::query("UPDATE host_credentials SET revoked_at=? WHERE host_id=?")
        .bind(gridops_core::now_millis())
        .bind(&host)
        .execute(&f.database)
        .await?;
    assert_eq!(
        supervisor.run(std::future::pending()).await?,
        Stop::CredentialRejected
    );
    Ok(())
}

#[tokio::test]
async fn agent_completes_a_requested_credential_rotation() -> Result<()> {
    let f = Fixture::new().await?;
    control_plane(&f).await?;
    let (supervisor, journal) = agent(&f).await?;
    let code = f.issue("agent-rotation").await?;
    supervisor
        .enroll(code["code"].as_str().context("code")?, None)
        .await?;
    let host = journal
        .identity()
        .await?
        .context("identity")?
        .host_id
        .to_string();
    let (stop, stopped) = oneshot::channel::<()>();
    let running = tokio::spawn(async move {
        supervisor
            .run(async {
                let _ = stopped.await;
            })
            .await
    });

    let response = f
        .admin_post(&format!("/hosts/{host}/credentials/rotate"))
        .header("idempotency-key", "agent-rotate")
        .json(&json!({"expectedGeneration":1}))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let mut generation = 1;
    for _ in 0..100 {
        generation = journal
            .identity()
            .await?
            .context("identity")?
            .credential_generation
            .get();
        if generation == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(generation, 2, "agent never rotated its credential");
    let phase: String =
        sqlx::query_scalar("SELECT phase FROM host_credential_rotations WHERE host_id=?")
            .bind(&host)
            .fetch_one(&f.database)
            .await?;
    assert_eq!(phase, "acknowledged");
    // The session keeps heartbeating with the new credential.
    let before: i64 =
        sqlx::query_scalar("SELECT COALESCE(last_heartbeat_at,0) FROM fleet_hosts WHERE id=?")
            .bind(&host)
            .fetch_one(&f.database)
            .await?;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let after: i64 =
        sqlx::query_scalar("SELECT COALESCE(last_heartbeat_at,0) FROM fleet_hosts WHERE id=?")
            .bind(&host)
            .fetch_one(&f.database)
            .await?;
    assert!(after > before, "no heartbeats after rotation");
    let _ = stop.send(());
    assert_eq!(running.await??, Stop::Shutdown);
    Ok(())
}

#[tokio::test]
async fn agent_ships_its_log_lines() -> Result<()> {
    let f = Fixture::new().await?;
    control_plane(&f).await?;
    let (supervisor, journal) = agent(&f).await?;
    let code = f.issue("agent-logs").await?;
    supervisor
        .enroll(code["code"].as_str().context("code")?, None)
        .await?;
    let host = journal.identity().await?.context("identity")?.host_id;
    let logs = gridops_host_agent::logs::LogBuffer::default();
    logs.restore(
        vec![gridops_host_agent::logs::LogLine {
            logged_at: gridops_core::now_millis(),
            level: "warn",
            target: "gridops_host_agent::docker".into(),
            message: "Docker jobs unavailable: daemon not running".into(),
        }],
        3,
    );
    let supervisor = supervisor.with_logs(logs);
    let (stop, stopped) = oneshot::channel::<()>();
    let running = tokio::spawn(async move {
        supervisor
            .run(async {
                let _ = stopped.await;
            })
            .await
    });
    let mut items = Vec::new();
    for _ in 0..200 {
        let body: Value = f
            .client
            .get(f.url(&format!("/hosts/{host}/logs")))
            .header(header::COOKIE, &f.cookie)
            .send()
            .await?
            .json()
            .await?;
        items = body["items"].as_array().cloned().unwrap_or_default();
        if items.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let messages: Vec<&str> = items
        .iter()
        .filter_map(|item| item["message"].as_str())
        .collect();
    assert!(
        messages.contains(&"Docker jobs unavailable: daemon not running"),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.starts_with("3 agent log lines were dropped"))
    );
    let _ = stop.send(());
    assert_eq!(running.await??, Stop::Shutdown);
    Ok(())
}
