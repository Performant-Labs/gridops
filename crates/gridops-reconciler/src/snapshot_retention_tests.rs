//! #152: the reconcile pass's retention cleanup prunes old inventory
//! snapshots, keeping `inventorySnapshotRetentionDays` (default 7, clamped
//! to 1..=3650) of them, and never an open session's current snapshot. A
//! failing prune is logged and does not stop the rest of the cleanup.
//!
//! #222: the cleanup prunes the other retained tables (webhook deliveries,
//! audit events, capacity samples, machine samples, reconciler passes, and
//! the three fleet health tables of #197) the same way: in bounded batches, so
//! one pass deletes at most `BATCH_SIZE * MAX_BATCHES` rows of each, and a
//! table whose prune fails is logged and does not stop the other tables'
//! prunes or the pass.

use super::*;
use gridops_core::connect_database_path;
use gridops_core::fleet::registry::snapshot_retention::{BATCH_SIZE, MAX_BATCHES};
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

    /// The parents capacity samples need: an installation and a runner pool.
    async fn capacity_parents(&self) -> Result<()> {
        sqlx::raw_sql(
            "INSERT INTO installations (id,account_id,account_login,account_type,target_type,
                                        repository_selection,created_at,updated_at)
               VALUES (1,1,'retention-org','Organization','Organization','selected',1,1);
             INSERT INTO runner_pools (id,installation_id,name,scope,labels,image,created_at,updated_at)
               VALUES ('retention-pool',1,'retention','organization','[]','runner:latest',1,1);",
        )
        .execute(&self.app.database)
        .await?;
        Ok(())
    }

    /// Inserts `count` rows of `table` in one statement, the first stamped
    /// `youngest` and each next one `table.step` older, so insertion (rowid)
    /// order is the reverse of age.
    async fn rows(&self, table: Table, count: i64, youngest: i64) -> Result<()> {
        let Table {
            name,
            columns,
            values,
            step,
            ..
        } = table;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {name} ({columns})
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?),
               s(at) AS (SELECT ? - (i - 1) * ? FROM n)
             SELECT {values} FROM s"
        )))
        .bind(count)
        .bind(youngest)
        .bind(step)
        .execute(&self.app.database)
        .await?;
        Ok(())
    }

    /// The retention-column values left in `table`, oldest first.
    async fn stamps(&self, table: Table) -> Result<Vec<i64>> {
        let Table { name, column, .. } = table;
        Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT {column} FROM {name} ORDER BY {column}"
        )))
        .fetch_all(&self.app.database)
        .await?)
    }

    /// Stores an integer setting as the settings API does (a JSON number).
    async fn setting(&self, key: &str, value: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO settings (key,value,updated_at) VALUES (?,?,1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        )
        .bind(key)
        .bind(value.to_string())
        .execute(&self.app.database)
        .await?;
        Ok(())
    }
}

/// A table the cleanup prunes in batches (#222): its name, the column its
/// retention cutoff applies to, its window in days when no setting is stored,
/// and how the tests insert rows into it (the columns, and their values over
/// `s(at)`, `at` being the retention column's value), each row `step`
/// milliseconds older than the one before. The two hourly fleet health tables
/// step by an hour: their `hour` must be a whole hour, and is in their
/// primary key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Table {
    name: &'static str,
    column: &'static str,
    default_days: i64,
    columns: &'static str,
    values: &'static str,
    step: i64,
}

const WEBHOOKS: Table = Table {
    name: "webhook_deliveries",
    column: "received_at",
    default_days: 90,
    columns: "id,event,signature_valid,received_at",
    values: "lower(hex(randomblob(16))),'push',1,at",
    step: 1,
};
const AUDIT: Table = Table {
    name: "audit_events",
    column: "created_at",
    default_days: 365,
    columns: "id,actor_label,action,target_type,created_at",
    values: "lower(hex(randomblob(16))),'retention-test','test','test',at",
    step: 1,
};
const CAPACITY: Table = Table {
    name: "capacity_samples",
    column: "recorded_at",
    default_days: 31,
    columns: "id,installation_id,pool_id,recorded_at",
    values: "lower(hex(randomblob(16))),1,'retention-pool',at",
    step: 1,
};
const MACHINES: Table = Table {
    name: "host_samples",
    column: "received_at",
    default_days: 31,
    columns: "host_id,host_epoch,observed_at,received_at,coverage",
    values: "'snapshot-retention-host',1,1,at,'unknown'",
    step: 1,
};
const PASSES: Table = Table {
    name: "reconciler_passes",
    column: "started_at",
    default_days: 31,
    columns: "started_at,duration_ms",
    values: "at,1",
    step: 1,
};
const HEALTH_HOURLY: Table = Table {
    name: "fleet_health_hourly",
    column: "hour",
    default_days: 90,
    columns: "hour,host_id,pool_id,category,job_count,completed_count,infra_failure_count,\
              duration_ms_sum,cpu_seconds_sum,queue_wait_histogram,duration_histogram,computed_at",
    values: "at,'unmanaged','unmanaged','other',1,1,0,10,0.5,\
             '[0,0,0,0,0,0,0,0,0]','[0,0,0,0,0,0,0,0,0]',at",
    step: HOUR,
};
const HOST_HOURLY: Table = Table {
    name: "fleet_host_hourly",
    column: "hour",
    default_days: 90,
    columns: "hour,host_id,online_minutes,start_attempts,start_failures_json,\
              uncertain_placements,computed_at",
    values: "at,'snapshot-retention-host',0,0,'{}',0,at",
    step: HOUR,
};
const HEALTH_SNAPSHOTS: Table = Table {
    name: "fleet_health_snapshots",
    column: "recorded_at",
    default_days: 90,
    columns: "recorded_at,\"window\",fleet_score,host_scores_json",
    values: "at,'24h',NULL,'{}'",
    step: 1,
};

/// How the tests fill each table the cleanup prunes in batches. An
/// exhaustive `match`, so a new `RetentionTable` does not compile until it
/// has test rows, and so the cap and isolation tests cover it.
const fn table(retained: RetentionTable) -> Table {
    match retained {
        RetentionTable::WebhookDeliveries => WEBHOOKS,
        RetentionTable::AuditEvents => AUDIT,
        RetentionTable::CapacitySamples => CAPACITY,
        RetentionTable::HostSamples => MACHINES,
        RetentionTable::ReconcilerPasses => PASSES,
        RetentionTable::FleetHealthHourly => HEALTH_HOURLY,
        RetentionTable::FleetHostHourly => HOST_HOURLY,
        RetentionTable::FleetHealthSnapshots => HEALTH_SNAPSHOTS,
    }
}

/// Every table the cleanup prunes in batches, besides the snapshots: one per
/// entry of `RetentionTable::ALL`, which `tests/retention.rs` pins to name
/// all eight retained tables once.
fn tables() -> Vec<Table> {
    RetentionTable::ALL.into_iter().map(table).collect()
}

/// `millis` rounded down to a whole hour, which every table accepts.
const fn whole_hour(millis: i64) -> i64 {
    millis - millis.rem_euclid(HOUR)
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

/// Each table keeps its own window: webhook deliveries
/// `webhookRetentionDays` (default 90), audit events `auditRetentionDays`
/// (default 365), and capacity samples, machine samples and reconciler passes
/// 31 days. (Guard: holds before #222 too; the batching must not move a
/// cutoff.) The fleet health tables' 90 days are guarded in one place,
/// `fleet_health_wiring_tests::retention_keeps_90_days_of_fleet_health_rows`.
#[tokio::test]
async fn each_retained_table_keeps_its_own_window() -> Result<()> {
    const WINDOWED: [Table; 5] = [WEBHOOKS, AUDIT, CAPACITY, MACHINES, PASSES];
    let h = Harness::new().await?;
    h.capacity_parents().await?;
    let now = now_millis();
    for table in WINDOWED {
        h.rows(table, 1, now - table.default_days * DAY - HOUR)
            .await?;
        h.rows(table, 1, now - table.default_days * DAY + HOUR)
            .await?;
    }
    cleanup_retention(&h.app).await?;
    for table in WINDOWED {
        assert_eq!(
            h.stamps(table).await?,
            [now - table.default_days * DAY + HOUR],
            "{} keeps {} days by default",
            table.name,
            table.default_days
        );
    }

    // The two configurable windows follow their settings.
    h.setting("webhookRetentionDays", 10).await?;
    h.setting("auditRetentionDays", 20).await?;
    for (table, days) in [(WEBHOOKS, 10), (AUDIT, 20)] {
        h.rows(table, 1, now - days * DAY + HOUR).await?;
    }
    cleanup_retention(&h.app).await?;
    assert_eq!(
        h.stamps(WEBHOOKS).await?,
        [now - 10 * DAY + HOUR],
        "webhookRetentionDays 10"
    );
    assert_eq!(
        h.stamps(AUDIT).await?,
        [now - 20 * DAY + HOUR],
        "auditRetentionDays 20"
    );
    h.finish().await
}

/// The incident (#222): one pass must not delete everything old in one write
/// lock. After a long gap a pass deletes at most `BATCH_SIZE * MAX_BATCHES`
/// rows of each table, oldest first; the rest goes on the next pass.
#[tokio::test]
async fn a_pass_deletes_at_most_batch_size_times_max_batches_rows_of_each_table() -> Result<()> {
    let h = Harness::new().await?;
    h.capacity_parents().await?;
    let cap = BATCH_SIZE * i64::from(MAX_BATCHES);
    let youngest = whole_hour(now_millis() - 400 * DAY);
    for table in tables() {
        h.rows(table, cap + 1, youngest).await?;
    }

    cleanup_retention(&h.app).await?;

    for table in tables() {
        assert_eq!(
            h.stamps(table).await?,
            [youngest],
            "{}: one pass deletes the {cap} oldest of {} old rows, no more",
            table.name,
            cap + 1
        );
    }
    h.finish().await
}

/// A table whose prune fails is logged, and the other tables are still
/// pruned and the cleanup (so the reconcile pass) still succeeds. Each table
/// in turn refuses its deletes through a test-only trigger.
#[tokio::test]
async fn a_failing_table_prune_does_not_stop_the_other_tables_or_the_pass() -> Result<()> {
    for refused in tables() {
        let h = Harness::new().await?;
        h.capacity_parents().await?;
        let old = whole_hour(now_millis() - 400 * DAY);
        for table in tables() {
            h.rows(table, 1, old).await?;
        }
        h.snapshot(1, old).await?;
        let name = refused.name;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE TRIGGER retention_test_refuses_deletes BEFORE DELETE ON {name}
             BEGIN SELECT RAISE(ABORT,'{name} deletes refused by the test'); END;"
        )))
        .execute(&h.app.database)
        .await?;

        let result = cleanup_retention(&h.app).await;

        let mut left = Vec::new();
        for table in tables() {
            left.push((table.name, h.stamps(table).await?.len()));
        }
        let expected: Vec<_> = tables()
            .iter()
            .map(|table| (table.name, usize::from(*table == refused)))
            .collect();
        assert!(
            result.is_ok() && left == expected,
            "with {name} refusing deletes: cleanup returned {result:?}, rows left {left:?}"
        );
        assert_eq!(
            h.remaining().await?,
            [CURRENT],
            "with {name} refusing deletes, the snapshot prune still ran"
        );
        h.finish().await?;
    }
    Ok(())
}
