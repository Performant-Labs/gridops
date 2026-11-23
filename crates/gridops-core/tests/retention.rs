//! Retention deletes hold `SQLite`'s single write lock while they run, and
//! every agent request waits behind them. Each must reach its rows through an
//! index; a full scan of the webhook payloads held the lock for seconds
//! (0034), and a scan of the 628k machine samples held it for up to 13 s on
//! Mars, answering the API's writers with 503s (#222).
//!
//! #222: the reconcile pass prunes each retained table through
//! `gridops_core::retention`, one bounded batch at a time: at most
//! `batch_size` rows per autocommit statement, oldest first, found through the
//! table's cutoff-column index, with a pause between batches and at most
//! `max_batches` batches per call. The rest goes on the next call.

use std::time::{Duration, Instant};

use anyhow::Result;
use gridops_core::connect_database_path;
use gridops_core::fleet::registry::snapshot_retention;
use gridops_core::retention::{self, BATCH_PAUSE, BATCH_SIZE, MAX_BATCHES, RetentionTable};
use sqlx::{Row as _, SqlitePool};

const NOW: i64 = 1_800_000_000_000;
const DAY: i64 = 86_400_000;
const CUTOFF: i64 = NOW - 31 * DAY;
const RECENT: i64 = NOW - DAY;

/// What each retained table must look like to its prune: the table, the
/// index on its cutoff column, that column, and how the tests insert a row
/// into it (the columns, and their values with one `?` for the cutoff
/// column's value). An exhaustive `match`, so a new table cannot be added
/// without saying which index its batches must use.
struct Expected {
    table: &'static str,
    index: &'static str,
    column: &'static str,
    insert_columns: &'static str,
    insert_values: &'static str,
}

const fn expected(table: RetentionTable) -> Expected {
    match table {
        RetentionTable::WebhookDeliveries => Expected {
            table: "webhook_deliveries",
            index: "webhook_deliveries_received_idx",
            column: "received_at",
            insert_columns: "id,event,signature_valid,received_at",
            insert_values: "lower(hex(randomblob(16))),'push',1,?",
        },
        RetentionTable::AuditEvents => Expected {
            table: "audit_events",
            index: "audit_events_created_idx",
            column: "created_at",
            insert_columns: "id,actor_label,action,target_type,created_at",
            insert_values: "lower(hex(randomblob(16))),'retention-test','test','test',?",
        },
        RetentionTable::CapacitySamples => Expected {
            table: "capacity_samples",
            index: "capacity_samples_recorded_idx",
            column: "recorded_at",
            insert_columns: "id,installation_id,pool_id,recorded_at",
            insert_values: "lower(hex(randomblob(16))),1,'retention-pool',?",
        },
        RetentionTable::HostSamples => Expected {
            table: "host_samples",
            // Added by migration 0051: without it the batch scans the table.
            index: "host_samples_received_idx",
            column: "received_at",
            insert_columns: "host_id,host_epoch,observed_at,received_at,coverage",
            insert_values: "'retention-host',1,1,?,'unknown'",
        },
        RetentionTable::ReconcilerPasses => Expected {
            table: "reconciler_passes",
            index: "reconciler_passes_started_idx",
            column: "started_at",
            insert_columns: "started_at,duration_ms",
            insert_values: "?,1",
        },
        // The three fleet health tables (0050, #197) are `STRICT`, and their
        // primary key leads with the cutoff column, so their batches search
        // the primary key's automatic index. Each hour must be a whole hour
        // (`CHECK (hour % 3600000 = 0)`); every stamp these tests use is.
        RetentionTable::FleetHealthHourly => Expected {
            table: "fleet_health_hourly",
            index: "sqlite_autoindex_fleet_health_hourly_1",
            column: "hour",
            insert_columns: "hour,host_id,pool_id,category,job_count,completed_count,\
                             infra_failure_count,duration_ms_sum,cpu_seconds_sum,\
                             queue_wait_histogram,duration_histogram,computed_at",
            insert_values: "?,'unmanaged','unmanaged','other',1,1,0,10,0.5,\
                            '[0,0,0,0,0,0,0,0,0]','[0,0,0,0,0,0,0,0,0]',1",
        },
        RetentionTable::FleetHostHourly => Expected {
            table: "fleet_host_hourly",
            index: "sqlite_autoindex_fleet_host_hourly_1",
            column: "hour",
            insert_columns: "hour,host_id,online_minutes,start_attempts,start_failures_json,\
                             uncertain_placements,computed_at",
            insert_values: "?,'retention-host',0,0,'{}',0,1",
        },
        RetentionTable::FleetHealthSnapshots => Expected {
            table: "fleet_health_snapshots",
            index: "sqlite_autoindex_fleet_health_snapshots_1",
            column: "recorded_at",
            insert_columns: "recorded_at,\"window\",fleet_score,host_scores_json",
            insert_values: "?,'24h',NULL,'{}'",
        },
    }
}

struct Db {
    pool: SqlitePool,
    directory: std::path::PathBuf,
}

impl Db {
    /// A fresh migrated database with the parents the sample tables need: an
    /// installation and a runner pool (capacity samples) and a host (machine
    /// samples).
    async fn new() -> Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("gridops-retention-{}", uuid::Uuid::new_v4()));
        let pool = connect_database_path(&directory.join("db.sqlite")).await?;
        sqlx::raw_sql(
            "INSERT INTO installations (id,account_id,account_login,account_type,target_type,
                                        repository_selection,created_at,updated_at)
               VALUES (1,1,'retention-org','Organization','Organization','selected',1,1);
             INSERT INTO runner_pools (id,installation_id,name,scope,labels,image,created_at,updated_at)
               VALUES ('retention-pool',1,'retention','organization','[]','runner:latest',1,1);
             INSERT INTO fleet_hosts (id,name,host_os,architecture,created_at,updated_at)
               VALUES ('retention-host','retention','linux','x64',1,1);",
        )
        .execute(&pool)
        .await?;
        Ok(Self { pool, directory })
    }

    async fn close(self) -> Result<()> {
        self.pool.close().await;
        std::fs::remove_dir_all(self.directory)?;
        Ok(())
    }

    /// Inserts one row of `table` whose cutoff column is `at`.
    async fn row(&self, table: RetentionTable, at: i64) -> Result<()> {
        let e = expected(table);
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {} ({}) VALUES ({})",
            e.table, e.insert_columns, e.insert_values
        )))
        .bind(at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The cutoff-column values left in `table`, oldest first.
    async fn remaining(&self, table: RetentionTable) -> Result<Vec<i64>> {
        let e = expected(table);
        Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT {column} FROM {table} ORDER BY {column}",
            column = e.column,
            table = e.table
        )))
        .fetch_all(&self.pool)
        .await?)
    }

    async fn plan(&self, statement: &str) -> Result<Vec<String>> {
        Ok(sqlx::query(sqlx::AssertSqlSafe(format!(
            "EXPLAIN QUERY PLAN {statement}"
        )))
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|row| row.get("detail"))
        .collect())
    }
}

/// The retention deletes that are not batched: each still finds its rows
/// through an index. (The batched ones, every `RetentionTable`, are pinned by
/// the next test, against the statements the prune runs.)
#[tokio::test]
async fn retention_deletes_find_their_rows_through_an_index() -> Result<()> {
    let db = Db::new().await?;
    for (table, statement) in [
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
        let plan = db.plan(statement).await?;
        assert!(
            plan.iter()
                .all(|step| !step.starts_with(&format!("SCAN {table}")))
                && plan.iter().any(|step| step.contains("INDEX")),
            "{statement} scans {table}: {plan:?}"
        );
    }
    db.close().await
}

/// #222's regression test. Each batch's inner select must search its table
/// through the cutoff-column index (`USING INDEX` or `USING COVERING INDEX`),
/// and no step may scan a table. This covers all eight retained tables,
/// including the three fleet health ones (#197), whose unbatched deletes used
/// to be checked in `fleet_health_schema.rs`. Without migration 0051, `host_samples` plans
/// as `SCAN host_samples USING COVERING INDEX host_samples_host_received_idx`
/// plus a temp B-tree: a read of every row inside the write lock.
#[tokio::test]
async fn every_retention_batch_finds_its_rows_through_its_cutoff_index() -> Result<()> {
    let mut names: Vec<&str> = RetentionTable::ALL.iter().map(|t| t.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "audit_events",
            "capacity_samples",
            "fleet_health_hourly",
            "fleet_health_snapshots",
            "fleet_host_hourly",
            "host_samples",
            "reconciler_passes",
            "webhook_deliveries",
        ],
        "ALL names each retained table once"
    );

    let db = Db::new().await?;
    for table in RetentionTable::ALL {
        let e = expected(table);
        assert_eq!(table.as_str(), e.table, "{table:?} is named for its table");
        let plan = db.plan(table.prune_batch()).await?;
        let search = format!("SEARCH {} USING ", e.table);
        let index = format!("INDEX {} ({}<?)", e.index, e.column);
        assert!(
            plan.iter()
                .any(|step| step.starts_with(&search) && step.ends_with(&index)),
            "the {} batch must search it through {}: {plan:?}",
            e.table,
            e.index
        );
        assert!(
            plan.iter().all(|step| !step.starts_with("SCAN ")),
            "the {} batch scans a table: {plan:?}",
            e.table
        );
    }
    db.close().await
}

/// Each table's prune deletes only its own rows older than the cutoff (a row
/// exactly at the cutoff stays), oldest first, at most `batch_size *
/// max_batches` of them per call, and returns how many went.
#[tokio::test]
async fn a_tables_prune_takes_only_its_rows_older_than_the_cutoff_oldest_first() -> Result<()> {
    let db = Db::new().await?;
    // Five old rows per table, inserted youngest first, so insertion (rowid)
    // order is the reverse of age; one at the cutoff; one recent.
    for table in RetentionTable::ALL {
        for days in 1..=5 {
            db.row(table, CUTOFF - days * DAY).await?;
        }
        db.row(table, CUTOFF).await?;
        db.row(table, RECENT).await?;
    }
    let untouched = vec![
        CUTOFF - 5 * DAY,
        CUTOFF - 4 * DAY,
        CUTOFF - 3 * DAY,
        CUTOFF - 2 * DAY,
        CUTOFF - DAY,
        CUTOFF,
        RECENT,
    ];
    let pruned = vec![CUTOFF, RECENT];

    for (done, table) in RetentionTable::ALL.into_iter().enumerate() {
        // Two full batches of two: the call stops at max_batches.
        assert_eq!(
            retention::prune(&db.pool, table, CUTOFF, 2, 2).await?,
            4,
            "{table:?}: two batches of two"
        );
        assert_eq!(
            db.remaining(table).await?,
            [CUTOFF - DAY, CUTOFF, RECENT],
            "{table:?}: the four oldest went first"
        );
        // The last old row, then nothing left to take.
        assert_eq!(
            retention::prune(&db.pool, table, CUTOFF, 2, 2).await?,
            1,
            "{table:?}: the rest"
        );
        assert_eq!(
            retention::prune(&db.pool, table, CUTOFF, 2, 2).await?,
            0,
            "{table:?}: nothing left"
        );
        // This table and the ones pruned before it hold only the rows at or
        // after the cutoff; the rest are untouched.
        for (index, other) in RetentionTable::ALL.into_iter().enumerate() {
            let want = if index <= done { &pruned } else { &untouched };
            assert_eq!(
                &db.remaining(other).await?,
                want,
                "{other:?} after pruning {table:?}"
            );
        }
    }
    db.close().await
}

/// A call stops at the first batch that comes back short instead of running
/// out its `max_batches`: with no cap that matters, a call that kept going
/// would sleep `BATCH_PAUSE` per remaining batch, forever.
#[tokio::test]
async fn a_prune_stops_at_the_first_short_batch() -> Result<()> {
    let db = Db::new().await?;
    for days in 1..=3 {
        db.row(RetentionTable::HostSamples, CUTOFF - days * DAY)
            .await?;
    }

    let deleted = tokio::time::timeout(
        Duration::from_secs(10),
        retention::prune(&db.pool, RetentionTable::HostSamples, CUTOFF, 2, u32::MAX),
    )
    .await
    .map_err(|_| anyhow::anyhow!("the prune did not stop after its short second batch"))??;

    assert_eq!(deleted, 3);
    assert_eq!(db.remaining(RetentionTable::HostSamples).await?, [0_i64; 0]);
    db.close().await
}

/// The write lock is let go between batches for long enough that the API's
/// writers, in another process, get it: each batch after the first waits
/// `BATCH_PAUSE` first. `tokio::time::sleep` never returns early, so only a
/// lower bound is asserted, which keeps the test deterministic (#152's
/// pattern).
#[tokio::test]
async fn a_prune_pauses_between_its_batches() -> Result<()> {
    let db = Db::new().await?;
    // Seven old rows: three full batches of two, so the call runs all three
    // batches (two pauses) and stops at max_batches, not early.
    for days in 1..=7 {
        db.row(RetentionTable::HostSamples, CUTOFF - days * DAY)
            .await?;
    }

    let started = Instant::now();
    let deleted = retention::prune(&db.pool, RetentionTable::HostSamples, CUTOFF, 2, 3).await?;
    let elapsed = started.elapsed();

    assert_eq!(deleted, 6, "three full batches of two ran");
    assert!(
        elapsed >= 2 * BATCH_PAUSE,
        "three batches take at least two pauses: {elapsed:?} < 2 * {BATCH_PAUSE:?}"
    );
    db.close().await
}

/// A batch size below 1 deletes nothing: a negative `LIMIT` is no limit in
/// `SQLite`, so passing one through would be the unbounded delete the
/// batching exists to prevent.
#[tokio::test]
async fn a_prune_with_a_batch_size_below_one_deletes_nothing() -> Result<()> {
    let db = Db::new().await?;
    for days in 1..=3 {
        db.row(RetentionTable::HostSamples, CUTOFF - days * DAY)
            .await?;
    }
    let all = [CUTOFF - 3 * DAY, CUTOFF - 2 * DAY, CUTOFF - DAY];

    for batch_size in [0, -1] {
        assert_eq!(
            retention::prune(
                &db.pool,
                RetentionTable::HostSamples,
                CUTOFF,
                batch_size,
                MAX_BATCHES
            )
            .await?,
            0,
            "batch_size {batch_size}"
        );
        assert_eq!(
            db.remaining(RetentionTable::HostSamples).await?,
            all,
            "batch_size {batch_size}"
        );
    }
    db.close().await
}

/// The decided bounds (#152, reused by #222): 500-row batches, at most 40 a
/// call (20,000 rows), and a pause of at least 25 ms between them. The
/// snapshot prune and the table prunes share them.
#[test]
fn every_prune_is_bounded_to_forty_batches_of_five_hundred_with_a_pause_between() {
    assert_eq!(BATCH_SIZE, 500);
    assert_eq!(MAX_BATCHES, 40);
    assert!(BATCH_PAUSE >= Duration::from_millis(25), "{BATCH_PAUSE:?}");
    assert_eq!(snapshot_retention::BATCH_SIZE, BATCH_SIZE);
    assert_eq!(snapshot_retention::MAX_BATCHES, MAX_BATCHES);
    assert_eq!(snapshot_retention::BATCH_PAUSE, BATCH_PAUSE);
}
