//! The overview's disk alarms, driven through a real enrolled host.

use super::inventory::*;
use super::*;

/// Enrolls a host whose root disk is nearly full, then asserts the
/// dashboard overview names it — and that a second, offline host never
/// raises a disk alarm.
#[tokio::test]
async fn overview_alarms_when_a_fleet_host_runs_out_of_disk() -> Result<()> {
    let f = Fixture::new().await?;

    // The nearly-full host: 100 GiB disk with 2 GiB free, well below both
    // the warning and the critical floor.
    let (host, secret) = enrolled(&f).await?;
    let incarnation = control_plane(&f).await?;
    let handshake = receipt(
        agent_post(&f, &host, &secret, "handshake")
            .json(&handshake_body())
            .send()
            .await?,
    )
    .await?;
    let mut body = inventory_body(&handshake, &incarnation);
    body["report"]["domains"][0]["capacity"]["disk_bytes"] = json!(100_i64 * 1024 * 1024 * 1024);
    body["report"]["samples"][0]["disk_free_bytes"] = json!(2_i64 * 1024 * 1024 * 1024);
    receipt(
        agent_post(&f, &host, &secret, "inventory")
            .json(&body)
            .send()
            .await?,
    )
    .await?;
    // The alert path only considers hosts whose heartbeat is fresh.
    receipt(
        agent_post(&f, &host, &secret, "heartbeat")
            .json(&heartbeat_body(&handshake, &incarnation))
            .send()
            .await?,
    )
    .await?;

    // An approved host that went quiet: its stale sample must not alarm.
    let issued = f.issue("disk-gone-enrollment").await?;
    let response = f.consume(issued["code"].as_str().context("code")?).await?;
    let gone_body: Value = response.json().await?;
    let gone = gone_body["host_id"].as_str().context("host id")?.to_owned();
    let gone_secret = gone_body["credential"]
        .as_str()
        .context("credential")?
        .to_owned();
    let gone_handshake = receipt(
        agent_post(&f, &gone, &gone_secret, "handshake")
            .json(&handshake_body())
            .send()
            .await?,
    )
    .await?;
    let mut gone_report = inventory_body(&gone_handshake, &incarnation);
    gone_report["report"]["domains"][0]["capacity"]["disk_bytes"] =
        json!(100_i64 * 1024 * 1024 * 1024);
    gone_report["report"]["samples"][0]["disk_free_bytes"] = json!(1024_i64 * 1024);
    receipt(
        agent_post(&f, &gone, &gone_secret, "inventory")
            .json(&gone_report)
            .send()
            .await?,
    )
    .await?;
    // Silence it: no heartbeat within the freshness window.
    sqlx::query("UPDATE fleet_hosts SET last_heartbeat_at=1 WHERE id=?")
        .bind(&gone)
        .execute(&f.database)
        .await?;

    // Only approved hosts raise disk alarms.
    sqlx::query("UPDATE fleet_hosts SET enrollment_state='approved' WHERE id IN (?,?)")
        .bind(&host)
        .bind(&gone)
        .execute(&f.database)
        .await?;
    let overview: Value = f
        .client
        .get(format!("{}/api/v1/overview", f.origin))
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?
        .json()
        .await?;
    let alerts = overview["slo"]["alerts"].as_array().context("alerts")?;
    let disk = alerts
        .iter()
        .find(|alert| {
            alert["title"]
                .as_str()
                .is_some_and(|title| title.contains("ThinkPad"))
        })
        .context("the fresh host raises a disk alarm")?;
    assert_eq!(disk["level"], "error");
    let detail = disk["detail"].as_str().context("detail")?;
    assert!(detail.contains("2.0 GiB free of 100 GiB"), "{detail}");
    assert!(detail.contains("2% free"), "{detail}");
    assert!(
        !alerts.iter().any(|alert| alert["title"]
            .as_str()
            .is_some_and(|title| title.contains("running out of disk")
                && !title.contains("ThinkPad")
                && title.contains("GridOps server"))),
        "the fixture server volume is healthy and must not alarm"
    );
    Ok(())
}
