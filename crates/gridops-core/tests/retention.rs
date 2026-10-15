//! Retention deletes hold `SQLite`'s single write lock while they run, and
//! every agent request waits behind them. Each must reach its rows through an
//! index; a full scan of the webhook payloads held the lock for seconds.

use anyhow::Result;
use gridops_core::connect_database_path;
use sqlx::Row as _;

#[tokio::test]
async fn retention_deletes_find_their_rows_through_an_index() -> Result<()> {
    let directory =
        std::env::temp_dir().join(format!("gridops-retention-{}", uuid::Uuid::new_v4()));
    let pool = connect_database_path(&directory.join("db.sqlite")).await?;
    for (table, statement) in [
        (
            "webhook_deliveries",
            "DELETE FROM webhook_deliveries WHERE received_at < 1",
        ),
        (
            "audit_events",
            "DELETE FROM audit_events WHERE created_at < 1",
        ),
        (
            "capacity_samples",
            "DELETE FROM capacity_samples WHERE recorded_at < 1",
        ),
        (
            "fleet_agent_logs",
            "DELETE FROM fleet_agent_logs WHERE received_at < 1",
        ),
        (
            "fleet_agent_logs",
            "DELETE FROM fleet_agent_logs WHERE host_id='h' AND received_at < 1",
        ),
        (
            "log_streams",
            "DELETE FROM log_streams WHERE expires_at IS NOT NULL AND expires_at < 1",
        ),
    ] {
        let plan: Vec<String> = sqlx::query(sqlx::AssertSqlSafe(format!(
            "EXPLAIN QUERY PLAN {statement}"
        )))
        .fetch_all(&pool)
        .await?
        .iter()
        .map(|row| row.get("detail"))
        .collect();
        assert!(
            plan.iter()
                .all(|step| !step.starts_with(&format!("SCAN {table}")))
                && plan.iter().any(|step| step.contains("INDEX")),
            "{statement} scans {table}: {plan:?}"
        );
    }
    pool.close().await;
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}
