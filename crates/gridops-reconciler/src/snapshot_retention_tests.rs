//! #152: the reconcile pass's retention cleanup prunes old inventory
//! snapshots, keeping `inventorySnapshotRetentionDays` (default 7, clamped
//! to 1..=3650) of them, and never an open session's current snapshot. A
//! failing prune is logged and does not stop the rest of the cleanup.

use super::*;
use gridops_core::connect_database_path;
use std::fs;

const DAY: i64 = 86_400_000;
const HOUR: i64 = 3_600_000;
const HOST: &str = "snapshot-retention-host";
const SESSION: &str = "snapshot-retention-session";
/// The open session's current inventory, received long ago: always kept.
const CURRENT: i64 = 100;

struct Harness {
    app: Reconciler,
    directory: std::path::PathBuf,
}

impl Harness {
    /// A reconciler over a fresh database with one host, its open session,
    /// and that session's current snapshot, received 400 days ago.
    async fn new() -> Result<Self> {
        let directory = std::env::temp_dir().join(format!(
            "gridops-snapshot-retention-test-{}",
            uuid::Uuid::new_v4()
        ));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        sqlx::query(
            "INSERT INTO fleet_hosts (id,name,host_os,architecture,created_at,updated_at)
             VALUES (?,'snapshot-retention','linux','x64',1,1)",
        )
        .bind(HOST)
        .execute(&database)
        .await?;
        sqlx::query(
            "INSERT INTO host_credentials (id,host_id,host_epoch,generation,verifier,created_at)
             VALUES ('snapshot-retention-credential',?,1,1,?,1)",
        )
        .bind(HOST)
        // A sha256 in base64url: 43 characters.
        .bind("V".repeat(43))
        .execute(&database)
        .await?;
        sqlx::query(
            "INSERT INTO fleet_observation_sessions
             (session_id,host_id,host_epoch,boot_id,control_plane_incarnation,negotiated_protocol,
              initial_credential_id,initial_credential_generation,current_credential_id,
              current_credential_generation,initial_request_id,initial_body_digest,state,
              created_at,last_seen_at,lease_expires_at,handshake_received_at,
              handshake_lease_expires_at,handshake_state,current_inventory_sequence)
             VALUES (?,?,1,'boot-1','inc-1',1,'snapshot-retention-credential',1,
                     'snapshot-retention-credential',1,'request-1',?,'observing',1,1,1,1,1,
                     'observing',?)",
        )
        .bind(SESSION)
        .bind(HOST)
        .bind("A".repeat(43))
        .bind(CURRENT)
        .execute(&database)
        .await?;
        let config = Config::from_env()?;
        let app = Reconciler {
            github: GitHubClient::new(config.clone())?,
            config,
            database,
            bitbucket: BitbucketClient::new()?,
            gitlab: Arc::new(gridops_core::gitlab::fake::FakeGitLab::default()),
            vault: Vault::new([0; 32], b"reconciler-test".to_vec()),
            http: reqwest::Client::new(),
            pass: Arc::default(),
            tart: Arc::new(tart_lane::TartLane::new(
                Arc::default(),
                tart_lane::SLOT_WATCH_INTERVAL,
            )),
        };
        let harness = Self { app, directory };
        harness.snapshot(CURRENT, now_millis() - 400 * DAY).await?;
        Ok(harness)
    }

    async fn finish(self) -> Result<()> {
        self.app.database.close().await;
        fs::remove_dir_all(self.directory)?;
        Ok(())
    }

    /// Snapshot `sequence` of the open session, received at `received_at`.
    async fn snapshot(&self, sequence: i64, received_at: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO fleet_inventory_snapshots
             (host_id,authority_session_id,host_epoch,control_plane_incarnation,inventory_sequence,
              digest,snapshot_json,received_at,receipt_lease_expires_at,receipt_state,
              inventory_revision,domain_mappings_json,backend_mappings_json,interactive_mappings_json)
             VALUES (?,?,1,'inc-1',?,?,'{}',?,?,'reconciling',1,'[]','[]','[]')",
        )
        .bind(HOST)
        .bind(SESSION)
        .bind(sequence)
        .bind("A".repeat(43))
        .bind(received_at)
        .bind(received_at + 60_000)
        .execute(&self.app.database)
        .await?;
        Ok(())
    }

    /// Stores the retention setting as the settings API does (a JSON number).
    async fn retain_days(&self, days: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO settings (key,value,updated_at) VALUES ('inventorySnapshotRetentionDays',?,1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        )
        .bind(days.to_string())
        .execute(&self.app.database)
        .await?;
        Ok(())
    }

    /// The sequences of the snapshots left, in order.
    async fn remaining(&self) -> Result<Vec<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT inventory_sequence FROM fleet_inventory_snapshots ORDER BY inventory_sequence",
        )
        .fetch_all(&self.app.database)
        .await?)
    }
}

#[tokio::test]
async fn retention_keeps_the_configured_days_of_snapshots_seven_by_default() -> Result<()> {
    let h = Harness::new().await?;
    let now = now_millis();

    // No setting: seven days are kept.
    h.snapshot(1, now - 7 * DAY - HOUR).await?;
    h.snapshot(2, now - 7 * DAY + HOUR).await?;
    cleanup_retention(&h.app).await?;
    assert_eq!(h.remaining().await?, [2, CURRENT], "the default is 7 days");

    // Thirty days.
    h.retain_days(30).await?;
    h.snapshot(3, now - 8 * DAY).await?;
    h.snapshot(4, now - 31 * DAY).await?;
    cleanup_retention(&h.app).await?;
    assert_eq!(h.remaining().await?, [2, 3, CURRENT], "30 days are kept");

    // Below the range: one day, not zero (zero would take snapshot 6 too).
    h.retain_days(0).await?;
    h.snapshot(5, now - DAY - HOUR).await?;
    h.snapshot(6, now - DAY + HOUR).await?;
    cleanup_retention(&h.app).await?;
    assert_eq!(h.remaining().await?, [6, CURRENT], "0 days is clamped to 1");

    // Above the range: 3,650 days, not 5,000.
    h.retain_days(5_000).await?;
    h.snapshot(7, now - 3_651 * DAY).await?;
    h.snapshot(8, now - 3_649 * DAY).await?;
    cleanup_retention(&h.app).await?;
    assert_eq!(
        h.remaining().await?,
        [6, 8, CURRENT],
        "5,000 days is clamped to 3,650"
    );
    h.finish().await
}

/// A prune that fails is logged, and the pass's other retention deletes
/// still run. (Guard: holds before the prune exists too.)
#[tokio::test]
async fn a_failing_snapshot_prune_does_not_stop_the_rest_of_the_cleanup() -> Result<()> {
    let h = Harness::new().await?;
    let now = now_millis();
    h.snapshot(1, now - 30 * DAY).await?;
    sqlx::raw_sql(
        "CREATE TRIGGER snapshot_retention_test_refuses_deletes
         BEFORE DELETE ON fleet_inventory_snapshots
         BEGIN SELECT RAISE(ABORT,'snapshot deletes refused by the test'); END;",
    )
    .execute(&h.app.database)
    .await?;
    sqlx::query("INSERT INTO reconciler_passes (started_at,duration_ms) VALUES (?,1)")
        .bind(now - 40 * DAY)
        .execute(&h.app.database)
        .await?;

    cleanup_retention(&h.app).await?;

    let passes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reconciler_passes")
        .fetch_one(&h.app.database)
        .await?;
    assert_eq!(passes, 0, "the reconciler-pass retention still ran");
    assert_eq!(h.remaining().await?, [1, CURRENT]);
    h.finish().await
}
