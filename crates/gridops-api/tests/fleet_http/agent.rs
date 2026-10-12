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
    );
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
