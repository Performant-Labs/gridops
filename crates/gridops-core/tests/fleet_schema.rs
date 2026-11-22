//! Exercises fleet constraints on real `SQLite` files with legacy rows and WAL.
//! Migration success alone is insufficient: identity, history, and charged
//! resources must survive upgrade and reject writes that break ownership.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use anyhow::{Context as _, Result};
use sqlx::{
    Row, SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
const HOST: &str = "10000000-0000-4000-8000-000000000001";
const OTHER_HOST: &str = "20000000-0000-4000-8000-000000000001";
const ROOT: &str = "11000000-0000-4000-8000-000000000001";
const BACKEND: &str = "13000000-0000-4000-8000-000000000001";
const TARGET: &str = "30000000-0000-4000-8000-000000000001";
const OTHER_TARGET: &str = "30000000-0000-4000-8000-000000000002";
const PLACEMENT: &str = "60000000-0000-4000-8000-000000000001";

struct Database {
    directory: PathBuf,
    pool: SqlitePool,
}

impl Database {
    async fn legacy() -> Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("gridops-fleet-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory)?;
        let options = SqliteConnectOptions::new()
            .filename(directory.join("test.sqlite"))
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        MIGRATOR.run_to(21, &pool).await?;
        sqlx::raw_sql(include_str!("fixtures/fleet_legacy.sql"))
            .execute(&pool)
            .await?;
        Ok(Self { directory, pool })
    }

    async fn ready() -> Result<Self> {
        let database = Self::legacy().await?;
        MIGRATOR.run(&database.pool).await?;
        sqlx::raw_sql(include_str!("fixtures/fleet_ready.sql"))
            .execute(&database.pool)
            .await?;
        Ok(database)
    }

    async fn close(self) -> Result<()> {
        self.pool.close().await;
        std::fs::remove_dir_all(self.directory)?;
        Ok(())
    }
}

async fn schema(pool: &SqlitePool) -> Result<BTreeMap<String, String>> {
    let rows = sqlx::query("SELECT name,sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' AND name != '_sqlx_migrations'")
        .fetch_all(pool).await?;
    Ok(rows
        .iter()
        .map(|row| (row.get("name"), row.get("sql")))
        .collect())
}

async fn records(pool: &SqlitePool, tables: &[String]) -> Result<BTreeMap<String, Vec<String>>> {
    let mut result = BTreeMap::new();
    for table in tables {
        // Names come exclusively from SQLite's schema, never request input.
        let columns = sqlx::query("SELECT name FROM pragma_table_info(?) ORDER BY cid")
            .bind(table)
            .fetch_all(pool)
            .await?;
        let columns = columns
            .iter()
            .map(|row| format!("\"{}\"", row.get::<String, _>("name").replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(",");
        let query = format!(
            "SELECT json_array({columns}) AS record FROM \"{}\" ORDER BY record",
            table.replace('"', "\"\"")
        );
        let rows = sqlx::query_scalar(sqlx::AssertSqlSafe(query))
            .fetch_all(pool)
            .await?;
        result.insert(table.clone(), rows);
    }
    Ok(result)
}

async fn rejected(pool: &SqlitePool, sql: &'static str) {
    let outcome = sqlx::query(sql).execute(pool).await;
    assert!(outcome.is_err(), "unsafe write was accepted: {sql}");
}

#[tokio::test]
async fn additive_upgrade_preserves_every_legacy_row_index_and_trigger() -> Result<()> {
    let db = Database::legacy().await?;
    let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != '_sqlx_migrations' ORDER BY name")
        .fetch_all(&db.pool).await?;
    let old_schema = schema(&db.pool).await?;
    let old_columns = table_columns(&db.pool, &tables).await?;
    let old_records = records(&db.pool, &tables).await?;
    MIGRATOR.run(&db.pool).await?;
    // Rows compare on every column the table had before the upgrade; the
    // columns 0047 adds are checked on their own below.
    let mut new_records = records_on(&db.pool, &old_columns).await?;
    // The one row an upgrade may add: whether the built-in runner host stays
    // on. This legacy fixture has a Docker pool with a runner on the host, so
    // it stays on. Every legacy row is otherwise untouched.
    let added = new_records
        .get_mut("settings")
        .context("settings table")?
        .extract_if(.., |record| record.contains("builtinRunnerHostEnabled"))
        .collect::<Vec<_>>();
    assert_eq!(added.len(), 1, "one new settings row");
    assert!(added[0].contains("\"true\""), "{}", added[0]);
    assert_eq!(new_records, old_records);
    assert_added_columns(&db.pool, &old_columns, LEGACY_ADDED_COLUMNS).await?;
    // 0047 rebuilds `runners` (its ci_platform CHECK) and adds a column to
    // `runner_pools`, so only those definitions may change; their shape is
    // asserted instead. Every other legacy definition is byte-for-byte equal.
    let new_schema = schema(&db.pool).await?;
    for (name, definition) in old_schema {
        if exempt_from_definition_check(&name, &[]) {
            continue;
        }
        assert_eq!(
            new_schema.get(&name),
            Some(&definition),
            "changed legacy schema: {name}"
        );
    }
    assert_runners_contract(&db.pool).await?;
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&db.pool)
            .await?
            .is_empty()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
            .fetch_one(&db.pool)
            .await?,
        "ok"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("PRAGMA journal_mode")
            .fetch_one(&db.pool)
            .await?,
        "wal"
    );
    assert_foreign_keys_on_every_connection(&db.pool).await?;
    // Reopening/rerunning the migration neither replays DDL nor loses history.
    MIGRATOR.run(&db.pool).await?;
    let mut rerun = records_on(&db.pool, &old_columns).await?;
    rerun
        .get_mut("settings")
        .context("settings table")?
        .retain(|record| !record.contains("builtinRunnerHostEnabled"));
    assert_eq!(rerun, old_records);
    db.close().await
}

/// The definitions 0047 may change, each asserted by shape instead: the
/// `runners` rebuild and its indexes, `runner_pools` (one added column), and
/// whatever else the caller names.
fn exempt_from_definition_check(name: &str, also: &[&str]) -> bool {
    name == "runners"
        || name == "runner_pools"
        || RUNNERS_INDEXES.iter().any(|(index, _, _)| *index == name)
        || also.contains(&name)
}

/// Every index on `runners` after 0047: name, unique, columns. The first eight
/// are the ones the table has after 0046; 0013's `runners_github_id_unique`
/// must not come back.
const RUNNERS_INDEXES: [(&str, bool, &str); 10] = [
    ("runners_container_id_unique", true, "container_id"),
    ("runners_pool_status_idx", false, "pool_id,status"),
    ("runners_last_job_idx", false, "last_job_id"),
    (
        "runners_pool_configuration_idx",
        false,
        "pool_id,configuration_version",
    ),
    (
        "runners_target_repository_idx",
        false,
        "pool_id,target_repository_id,status",
    ),
    ("runners_github_id_idx", false, "github_runner_id"),
    (
        "runners_bitbucket_connection_idx",
        false,
        "pool_id,bitbucket_connection_id,status",
    ),
    ("runners_name_idx", false, "name,created_at"),
    (
        "runners_gitlab_connection_idx",
        false,
        "pool_id,gitlab_connection_id,status",
    ),
    ("runners_gitlab_runner_id_idx", false, "gitlab_runner_id"),
];

/// Outbound foreign keys of `runners` after 0047: (from, table, to, on delete).
const RUNNERS_FOREIGN_KEYS: [(&str, &str, &str, &str); 5] = [
    (
        "bitbucket_connection_id",
        "bitbucket_connections",
        "id",
        "SET NULL",
    ),
    (
        "gitlab_connection_id",
        "gitlab_connections",
        "id",
        "SET NULL",
    ),
    ("last_job_id", "workflow_jobs", "id", "SET NULL"),
    ("pool_id", "runner_pools", "id", "CASCADE"),
    ("target_repository_id", "repositories", "id", "SET NULL"),
];

/// The columns 0047 and later migrations add to tables that already hold
/// rows, with the value every existing row must have afterwards (`None` =
/// NULL). #183: 0048 appends `fleet_ci_targets.retired_at`, NULL for every
/// target that exists when it runs.
type AddedColumns = &'static [(
    &'static str,
    &'static [(&'static str, &'static str, Option<i64>)],
)];

const LEGACY_ADDED_COLUMNS: AddedColumns = &[
    (
        "runners",
        &[
            ("gitlab_connection_id", "TEXT", None),
            ("gitlab_runner_id", "INTEGER", None),
        ],
    ),
    ("runner_pools", &[("run_untagged", "INTEGER", Some(0))]),
];

const FLEET_ADDED_COLUMNS: AddedColumns = &[
    (
        "runners",
        &[
            ("gitlab_connection_id", "TEXT", None),
            ("gitlab_runner_id", "INTEGER", None),
        ],
    ),
    ("runner_pools", &[("run_untagged", "INTEGER", Some(0))]),
    (
        "fleet_ci_targets",
        &[
            ("gitlab_connection_id", "TEXT", None),
            ("gitlab_project_id", "INTEGER", None),
            ("gitlab_group_id", "INTEGER", None),
            ("retired_at", "INTEGER", None),
        ],
    ),
];

/// (name, upper-cased declared type, notnull, default, pk) per column, in order.
type Column = (String, String, i64, Option<String>, i64);

async fn columns(pool: &SqlitePool, table: &str) -> Result<Vec<Column>> {
    let rows = sqlx::query(
        "SELECT name,upper(type) AS type,\"notnull\",dflt_value,pk FROM pragma_table_info(?) ORDER BY cid",
    )
    .bind(table)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| {
            (
                row.get("name"),
                row.get("type"),
                row.get("notnull"),
                row.get("dflt_value"),
                row.get("pk"),
            )
        })
        .collect())
}

async fn table_columns(
    pool: &SqlitePool,
    tables: &[String],
) -> Result<BTreeMap<String, Vec<Column>>> {
    let mut result = BTreeMap::new();
    for table in tables {
        result.insert(table.clone(), columns(pool, table).await?);
    }
    Ok(result)
}

/// Like `records`, but over a fixed column list per table, so rows taken
/// before and after an upgrade that appends columns compare exactly.
async fn records_on(
    pool: &SqlitePool,
    tables: &BTreeMap<String, Vec<Column>>,
) -> Result<BTreeMap<String, Vec<String>>> {
    let mut result = BTreeMap::new();
    for (table, columns) in tables {
        // Names come exclusively from SQLite's schema, never request input.
        let columns = columns
            .iter()
            .map(|column| format!("\"{}\"", column.0.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(",");
        let query = format!(
            "SELECT json_array({columns}) AS record FROM \"{}\" ORDER BY record",
            table.replace('"', "\"\"")
        );
        let rows = sqlx::query_scalar(sqlx::AssertSqlSafe(query))
            .fetch_all(pool)
            .await?;
        result.insert(table.clone(), rows);
    }
    Ok(result)
}

/// Every pre-existing column keeps its definition and position; the only new
/// columns on existing tables are the ones listed, appended in order, nullable
/// unless a default is given, and every existing row holds the listed value.
async fn assert_added_columns(
    pool: &SqlitePool,
    old: &BTreeMap<String, Vec<Column>>,
    added: AddedColumns,
) -> Result<()> {
    for (table, before) in old {
        let after = columns(pool, table).await?;
        let expected = added
            .iter()
            .find(|(name, _)| name == table)
            .map_or(&[][..], |(_, columns)| *columns);
        assert_eq!(
            after.get(..before.len()),
            Some(&before[..]),
            "{table}: existing columns changed"
        );
        let appended: Vec<_> = after[before.len()..]
            .iter()
            .map(|column| (column.0.as_str(), column.1.as_str()))
            .collect();
        let wanted: Vec<_> = expected
            .iter()
            .map(|(name, kind, _)| (*name, *kind))
            .collect();
        assert_eq!(appended, wanted, "{table}: added columns");
        for (column, (name, _, value)) in after[before.len()..].iter().zip(expected) {
            // Names come from the constant tables above, never request input.
            let query = match value {
                None => {
                    assert_eq!(column.2, 0, "{table}.{name} must be nullable");
                    assert_eq!(column.3, None, "{table}.{name} has no default");
                    format!("SELECT COUNT(*) FROM \"{table}\" WHERE \"{name}\" IS NOT NULL")
                }
                Some(value) => {
                    assert_eq!(column.2, 1, "{table}.{name} is NOT NULL");
                    assert_eq!(
                        column.3.as_deref(),
                        Some(value.to_string().as_str()),
                        "{table}.{name} default"
                    );
                    format!("SELECT COUNT(*) FROM \"{table}\" WHERE \"{name}\" IS NOT {value}")
                }
            };
            let differing: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(query))
                .fetch_one(pool)
                .await?;
            assert_eq!(
                differing, 0,
                "existing {table} rows must hold {value:?} in {name}"
            );
        }
    }
    Ok(())
}

/// Indexes created by `CREATE INDEX` on a table: name -> (unique, columns).
async fn created_indexes(
    pool: &SqlitePool,
    table: &str,
) -> Result<BTreeMap<String, (bool, String)>> {
    let names: Vec<(String, bool)> = sqlx::query_as(
        "SELECT name,\"unique\" FROM pragma_index_list(?) WHERE origin='c' ORDER BY name",
    )
    .bind(table)
    .fetch_all(pool)
    .await?;
    let mut result = BTreeMap::new();
    for (name, unique) in names {
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_index_info(?) ORDER BY seqno")
                .bind(&name)
                .fetch_all(pool)
                .await?;
        result.insert(name, (unique, columns.join(",")));
    }
    Ok(result)
}

/// (from, table, to, on update, on delete), sorted.
async fn foreign_keys(
    pool: &SqlitePool,
    table: &str,
) -> Result<Vec<(String, String, String, String, String)>> {
    Ok(sqlx::query_as(
        "SELECT \"from\",\"table\",\"to\",on_update,on_delete FROM pragma_foreign_key_list(?) ORDER BY \"from\",\"table\"",
    )
    .bind(table)
    .fetch_all(pool)
    .await?)
}

/// The rebuilt `runners` keeps every index and outbound foreign key, gains the
/// two GitLab ones, and keeps its CHECKs, with `ci_platform` widened.
async fn assert_runners_contract(pool: &SqlitePool) -> Result<()> {
    let expected: BTreeMap<String, (bool, String)> = RUNNERS_INDEXES
        .iter()
        .map(|(name, unique, columns)| ((*name).to_owned(), (*unique, (*columns).to_owned())))
        .collect();
    assert_eq!(
        created_indexes(pool, "runners").await?,
        expected,
        "runners indexes"
    );
    let expected: Vec<_> = RUNNERS_FOREIGN_KEYS
        .iter()
        .map(|(from, table, to, on_delete)| {
            (
                (*from).to_owned(),
                (*table).to_owned(),
                (*to).to_owned(),
                "NO ACTION".to_owned(),
                (*on_delete).to_owned(),
            )
        })
        .collect();
    assert_eq!(
        foreign_keys(pool, "runners").await?,
        expected,
        "runners foreign keys"
    );
    // The CHECKs, probed in a transaction that is rolled back.
    let mut tx = pool.begin().await?;
    let insert = |id: &'static str, column: &'static str, value: &'static str| {
        // Column names come from the literals below, never request input.
        sqlx::AssertSqlSafe(format!(
            "INSERT INTO runners (id,pool_id,name,created_at,updated_at,{column}) VALUES ('{id}','legacy-pool','{id}',1,1,'{value}')"
        ))
    };
    sqlx::query(insert("probe-gitlab", "ci_platform", "gitlab"))
        .execute(&mut *tx)
        .await
        .context("ci_platform accepts 'gitlab'")?;
    for (id, column, value) in [
        ("probe-platform", "ci_platform", "circleci"),
        ("probe-provider", "provider", "qemu"),
        ("probe-runtime", "runtime", "container"),
    ] {
        assert!(
            sqlx::query(insert(id, column, value))
                .execute(&mut *tx)
                .await
                .is_err(),
            "runners.{column} accepted '{value}'"
        );
    }
    tx.rollback().await?;
    Ok(())
}

/// 0047 switches `foreign_keys` off for its own run; every pooled connection,
/// the migration's included, must have it on again afterwards.
async fn assert_foreign_keys_on_every_connection(pool: &SqlitePool) -> Result<()> {
    let mut connections = Vec::new();
    for _ in 0..pool.options().get_max_connections() {
        connections.push(pool.acquire().await?);
    }
    for connection in &mut connections {
        assert_eq!(
            sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys")
                .fetch_one(&mut **connection)
                .await?,
            1
        );
    }
    Ok(())
}

#[tokio::test]
async fn domains_are_integer_bounded_and_cannot_cross_physical_hosts() -> Result<()> {
    let db = Database::ready().await?;
    for sql in [
        "UPDATE host_resource_domains SET cpu_millis=0.5",
        "UPDATE host_resource_domains SET memory_mib=-1",
        "UPDATE fleet_hosts SET epoch=0",
        "UPDATE fleet_hosts SET revision=1.5",
        "UPDATE host_resource_domains SET parent_domain_id=id",
        "INSERT INTO host_resource_domains (id,host_id,name,cpu_millis,memory_mib,disk_bytes) SELECT 'duplicate-root',host_id,'Duplicate',1,1,1 FROM host_resource_domains WHERE parent_domain_id IS NULL LIMIT 1",
    ] {
        rejected(&db.pool, sql).await;
    }
    let result = sqlx::query("INSERT INTO host_resource_domains (id,host_id,parent_domain_id,name,cpu_millis,memory_mib,disk_bytes) VALUES ('cross-host',?,?,'Invalid',1,1,1)")
        .bind(OTHER_HOST).bind(ROOT).execute(&db.pool).await;
    assert!(result.is_err());
    let result = sqlx::query("UPDATE host_backends SET host_id=? WHERE id=?")
        .bind(OTHER_HOST)
        .bind(BACKEND)
        .execute(&db.pool)
        .await;
    assert!(result.is_err());
    // The Mac owns the physical budget while its Docker execution is Linux.
    let row = sqlx::query("SELECT h.host_os,b.execution_os FROM host_backends b JOIN fleet_hosts h ON h.id=b.host_id WHERE b.id=?")
        .bind(BACKEND).fetch_one(&db.pool).await?;
    assert_eq!(row.get::<String, _>("host_os"), "macos");
    assert_eq!(row.get::<String, _>("execution_os"), "linux");
    db.close().await
}

#[tokio::test]
async fn unknown_capacity_cannot_be_deleted_reassigned_or_released_without_proof() -> Result<()> {
    let db = Database::ready().await?;
    sqlx::query("UPDATE capacity_allocations SET state='uncertain'")
        .execute(&db.pool)
        .await?;
    for sql in [
        "DELETE FROM capacity_allocations",
        "UPDATE capacity_allocations SET host_epoch=2",
        "UPDATE capacity_allocations SET cpu_millis=1",
        "UPDATE workload_placements SET host_epoch=2",
        "UPDATE workload_placements SET generation=2",
        "UPDATE capacity_allocations SET state='released',released_at=100",
    ] {
        rejected(&db.pool, sql).await;
    }
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT SUM(cpu_millis) FROM capacity_allocations WHERE domain_id=? AND state!='released'")
        .bind(ROOT).fetch_one(&db.pool).await?, 1000);
    sqlx::query("UPDATE workload_placements SET local_absence_proven_at=100,upstream_cleanup_state='pending'")
        .execute(&db.pool).await?;
    rejected(
        &db.pool,
        "UPDATE capacity_allocations SET state='released',released_at=100",
    )
    .await;
    sqlx::query("UPDATE workload_placements SET delayed_starts_excluded_at=101")
        .execute(&db.pool)
        .await?;
    sqlx::query("UPDATE capacity_allocations SET state='released',released_at=102")
        .execute(&db.pool)
        .await?;
    rejected(
        &db.pool,
        "UPDATE capacity_allocations SET state='reserved',released_at=NULL",
    )
    .await;
    rejected(
        &db.pool,
        "UPDATE workload_placements SET local_absence_proven_at=NULL",
    )
    .await;
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT upstream_cleanup_state FROM workload_placements WHERE id=?"
        )
        .bind(PLACEMENT)
        .fetch_one(&db.pool)
        .await?,
        "pending"
    );
    db.close().await
}

#[tokio::test]
async fn immutable_commands_keep_idempotency_and_distinguish_host_scope() -> Result<()> {
    let db = Database::ready().await?;
    for sql in [
        "UPDATE host_commands SET kind='start_runner'",
        "UPDATE host_commands SET payload_json='{\"changed\":true}'",
        "UPDATE host_commands SET deadline_at=9999999",
        "UPDATE fleet_operations SET request_hash='changed'",
        "UPDATE workload_intents SET target_id='30000000-0000-4000-8000-000000000002'",
        "INSERT INTO host_commands (id,operation_id,scope,host_id,host_epoch,kind,protocol_version,request_hash,payload_json,expected_revision,issued_at,deadline_at,idempotency_key) SELECT 'invalid-start',operation_id,'host',host_id,host_epoch,'start_runner',1,'hash','{}',1,1,2,'other-key' FROM host_commands LIMIT 1",
    ] {
        rejected(&db.pool, sql).await;
    }
    sqlx::query("INSERT INTO fleet_operations (id,kind,host_id,principal_kind,principal_scope,idempotency_key,request_hash,created_at,updated_at) VALUES ('80000000-0000-4000-8000-000000000001','upgrade_agent',?,'reconciler','controller:reconciler','upgrade-fixture','hash',1,1)")
        .bind(HOST).execute(&db.pool).await?;
    sqlx::query("INSERT INTO host_commands (id,operation_id,scope,host_id,host_epoch,kind,protocol_version,request_hash,payload_json,expected_revision,issued_at,deadline_at,idempotency_key) VALUES ('81000000-0000-4000-8000-000000000001','80000000-0000-4000-8000-000000000001','host',?,1,'upgrade_agent',1,'hash','{}',1,1,2,'upgrade-command')")
        .bind(HOST).execute(&db.pool).await?;
    db.close().await
}

#[tokio::test]
async fn target_identity_and_event_projection_cannot_be_rebound() -> Result<()> {
    let db = Database::ready().await?;
    for sql in [
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key) VALUES ('incomplete','github_repository','incomplete')",
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,organization_id,repository_id) VALUES ('mixed','github_organization','mixed',1,123,10)",
        "UPDATE fleet_ci_targets SET repository_id=11",
        "DELETE FROM profile_ci_targets",
    ] {
        rejected(&db.pool, sql).await;
    }
    // Revocation must preserve the historical mapping rather than require a delete.
    sqlx::query("UPDATE profile_ci_targets SET enabled=0,revoked_at=10")
        .execute(&db.pool)
        .await?;
    for (host, target) in [(OTHER_HOST, TARGET), (HOST, OTHER_TARGET)] {
        let result = sqlx::query("INSERT INTO fleet_events (id,host_id,placement_id,target_id,kind,created_at) VALUES (?, ?, ?, ?, 'test',1)")
            .bind(uuid::Uuid::new_v4().to_string()).bind(host).bind(PLACEMENT).bind(target).execute(&db.pool).await;
        assert!(result.is_err());
    }
    db.close().await
}

#[tokio::test]
async fn linux_backends_belong_to_windows_macos_and_linux_physical_hosts() -> Result<()> {
    let db = Database::ready().await?;
    let hosts: Vec<String> = sqlx::query_scalar(
        "SELECT h.host_os FROM host_backends b JOIN fleet_hosts h ON h.id=b.host_id WHERE b.execution_os='linux' ORDER BY h.host_os",
    ).fetch_all(&db.pool).await?;
    assert_eq!(hosts, ["linux", "macos", "windows"]);
    for host in [HOST, OTHER_HOST] {
        let roots: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM host_resource_domains WHERE host_id=? AND parent_domain_id IS NULL",
        ).bind(host).fetch_one(&db.pool).await?;
        assert_eq!(roots, 1);
        let backends: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM host_backends WHERE host_id=?")
                .bind(host)
                .fetch_one(&db.pool)
                .await?;
        assert_eq!(
            backends, 2,
            "native and Linux execution share one physical host"
        );
    }
    db.close().await
}

#[tokio::test]
async fn epoch_replacement_and_retirement_preserve_original_ownership() -> Result<()> {
    let db = Database::ready().await?;
    sqlx::query(
        "UPDATE fleet_hosts SET epoch=2,lifecycle_state='retired',retired_at=10 WHERE id=?",
    )
    .bind(HOST)
    .execute(&db.pool)
    .await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT host_epoch FROM workload_placements WHERE id=?")
            .bind(PLACEMENT)
            .fetch_one(&db.pool)
            .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM capacity_allocations WHERE host_epoch=1"
        )
        .fetch_one(&db.pool)
        .await?,
        2
    );
    rejected(&db.pool, "DELETE FROM fleet_hosts").await;
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&db.pool)
            .await?
            .is_empty()
    );
    db.close().await
}

#[tokio::test]
async fn inventory_is_invalidated_on_upgrade_and_every_authority_change() -> Result<()> {
    let db = Database::legacy().await?;
    MIGRATOR.run_to(22, &db.pool).await?;
    sqlx::raw_sql(include_str!("fixtures/fleet_ready.sql"))
        .execute(&db.pool)
        .await?;
    sqlx::query("UPDATE fleet_hosts SET inventory_complete=1,last_inventory_at=1 WHERE id=?")
        .bind(HOST)
        .execute(&db.pool)
        .await?;
    MIGRATOR.run(&db.pool).await?;
    let snapshot: (i64, Option<i64>) =
        sqlx::query_as("SELECT inventory_complete,last_inventory_at FROM fleet_hosts WHERE id=?")
            .bind(HOST)
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(snapshot, (0, None));
    rejected(&db.pool, "INSERT INTO fleet_hosts (id,name,host_os,architecture,inventory_complete,last_inventory_at,created_at,updated_at) VALUES ('a0000000-0000-4000-8000-000000000001','bad','linux','x64',1,1,1,1)").await;
    for change in [
        "UPDATE fleet_hosts SET epoch=epoch+1 WHERE id=?",
        "UPDATE fleet_hosts SET boot_id='next-boot' WHERE id=?",
        "UPDATE fleet_hosts SET agent_session_id='next-session' WHERE id=?",
        "UPDATE fleet_hosts SET authority_incarnation='next-incarnation' WHERE id=?",
    ] {
        sqlx::query("UPDATE fleet_hosts SET agent_session_id='current-session',boot_id='current-boot',authority_incarnation='current-incarnation' WHERE id=?")
            .bind(HOST).execute(&db.pool).await?;
        sqlx::query("UPDATE fleet_hosts SET inventory_complete=1,last_inventory_at=2,inventory_epoch=epoch,inventory_session_id=agent_session_id,inventory_boot_id=boot_id WHERE id=?")
            .bind(HOST).execute(&db.pool).await?;
        sqlx::query("UPDATE fleet_hosts SET epoch=epoch,boot_id=boot_id,agent_session_id=agent_session_id,authority_incarnation=authority_incarnation WHERE id=?")
            .bind(HOST).execute(&db.pool).await?;
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT inventory_complete FROM fleet_hosts WHERE id=?")
                .bind(HOST)
                .fetch_one(&db.pool)
                .await?,
            1
        );
        sqlx::query(change).bind(HOST).execute(&db.pool).await?;
        let snapshot: (i64, Option<i64>, Option<i64>, Option<String>, Option<String>) = sqlx::query_as("SELECT inventory_complete,last_inventory_at,inventory_epoch,inventory_session_id,inventory_boot_id FROM fleet_hosts WHERE id=?")
            .bind(HOST).fetch_one(&db.pool).await?;
        assert_eq!(snapshot, (0, None, None, None, None));
        assert!(
            sqlx::query("UPDATE fleet_hosts SET inventory_complete=1 WHERE id=?")
                .bind(HOST)
                .execute(&db.pool)
                .await
                .is_err()
        );
        sqlx::query("UPDATE fleet_hosts SET inventory_complete=1,last_inventory_at=3,inventory_epoch=epoch,inventory_session_id=agent_session_id,inventory_boot_id=boot_id WHERE id=?")
            .bind(HOST).execute(&db.pool).await?;
    }
    // Even one UPDATE that attempts to refresh inventory while replacing the
    // session must wait for a later authenticated inventory acknowledgment.
    sqlx::query("UPDATE fleet_hosts SET agent_session_id='replacement',inventory_session_id='replacement',inventory_complete=1 WHERE id=?")
        .bind(HOST).execute(&db.pool).await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT inventory_complete FROM fleet_hosts WHERE id=?")
            .bind(HOST)
            .fetch_one(&db.pool)
            .await?,
        0
    );
    sqlx::query("INSERT INTO fleet_hosts (id,name,host_os,architecture,inventory_complete,last_inventory_at,inventory_epoch,agent_session_id,inventory_session_id,boot_id,inventory_boot_id,authority_incarnation,created_at,updated_at) VALUES ('a0000000-0000-4000-8000-000000000002','valid snapshot','linux','x64',1,1,1,'session','session','boot','boot','incarnation',1,1)")
        .execute(&db.pool).await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM workload_placements")
            .fetch_one(&db.pool)
            .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM capacity_allocations")
            .fetch_one(&db.pool)
            .await?,
        2
    );
    db.close().await
}

#[tokio::test]
async fn deleting_a_pool_takes_its_finished_fleet_history_with_it() -> Result<()> {
    use gridops_core::fleet::targets::delete_pool;
    let db = Database::ready().await?;
    // The fixture's placement is still reserved, with capacity charged to the host.
    assert!(!delete_pool(&db.pool, "legacy-pool").await?);
    let pools: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runner_pools WHERE id='legacy-pool'")
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(pools, 1);
    sqlx::query("UPDATE workload_placements SET state='cleaned',local_absence_proven_at=2,delayed_starts_excluded_at=2 WHERE id=?")
        .bind(PLACEMENT)
        .execute(&db.pool)
        .await?;
    sqlx::query(
        "UPDATE capacity_allocations SET state='released',released_at=2 WHERE placement_id=?",
    )
    .bind(PLACEMENT)
    .execute(&db.pool)
    .await?;
    assert!(delete_pool(&db.pool, "legacy-pool").await?);
    for table in [
        "runner_pools",
        "pool_execution_profiles",
        "profile_ci_targets",
        "workload_intents",
        "workload_placements",
        "capacity_allocations",
        "host_commands",
    ] {
        // Names come from the list above, never request input.
        let rows: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
                .fetch_one(&db.pool)
                .await?;
        assert_eq!(rows, 0, "{table} kept rows");
    }
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&db.pool)
            .await?
            .is_empty()
    );
    db.close().await
}

/// Every table that references `fleet_ci_targets(id)` (0022). They are not
/// rebuilt, so their foreign keys must survive the `fleet_ci_targets` rebuild.
const TARGET_REFERRERS: [&str; 7] = [
    "host_target_grants",
    "host_session_target_grants",
    "profile_ci_targets",
    "workload_intents",
    "workload_placements",
    "fleet_events",
    "external_runner_observations",
];

#[tokio::test]
async fn gitlab_upgrade_from_0046_keeps_the_fleet_graph_and_every_reference() -> Result<()> {
    let db = Database::legacy().await?;
    MIGRATOR.run_to(46, &db.pool).await?;
    sqlx::raw_sql(include_str!("fixtures/fleet_ready.sql"))
        .execute(&db.pool)
        .await?;
    // Populate the third inbound foreign key of `runners`, so the rebuild is
    // tested with cascade, set-null and no-action references all present.
    sqlx::query("UPDATE workload_placements SET runner_id='legacy-runner' WHERE id=?")
        .bind(PLACEMENT)
        .execute(&db.pool)
        .await?;
    let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != '_sqlx_migrations' ORDER BY name")
        .fetch_all(&db.pool).await?;
    let old_schema = schema(&db.pool).await?;
    let old_columns = table_columns(&db.pool, &tables).await?;
    let old_records = records(&db.pool, &tables).await?;
    let mut old_foreign_keys = BTreeMap::new();
    for table in &tables {
        old_foreign_keys.insert(table.clone(), foreign_keys(&db.pool, table).await?);
    }
    for table in TARGET_REFERRERS {
        assert!(
            old_foreign_keys
                .get(table)
                .is_some_and(|keys| keys.iter().any(|key| key.1 == "fleet_ci_targets")),
            "{table} references fleet_ci_targets before 0047"
        );
    }

    MIGRATOR.run(&db.pool).await?;

    assert_eq!(records_on(&db.pool, &old_columns).await?, old_records);
    assert_added_columns(&db.pool, &old_columns, FLEET_ADDED_COLUMNS).await?;
    let new_schema = schema(&db.pool).await?;
    for (name, definition) in &old_schema {
        if exempt_from_definition_check(
            name,
            &["fleet_ci_targets", "fleet_ci_targets_immutable_identity"],
        ) {
            continue;
        }
        assert_eq!(
            new_schema.get(name),
            Some(definition),
            "changed schema: {name}"
        );
    }
    assert!(
        new_schema.contains_key("fleet_ci_targets_immutable_identity"),
        "the immutable-identity trigger is recreated"
    );
    for table in &tables {
        if table == "runners" || table == "fleet_ci_targets" {
            continue;
        }
        assert_eq!(
            Some(&foreign_keys(&db.pool, table).await?),
            old_foreign_keys.get(table),
            "{table} foreign keys"
        );
    }
    let no_action = |from: &str, table: &str| {
        (
            from.to_owned(),
            table.to_owned(),
            "id".to_owned(),
            "NO ACTION".to_owned(),
            "NO ACTION".to_owned(),
        )
    };
    assert_eq!(
        foreign_keys(&db.pool, "fleet_ci_targets").await?,
        vec![
            no_action("bitbucket_connection_id", "bitbucket_connections"),
            no_action("gitlab_connection_id", "gitlab_connections"),
            no_action("installation_id", "installations"),
            no_action("repository_id", "repositories"),
        ]
    );
    assert_runners_contract(&db.pool).await?;
    assert_eq!(
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT runner_id FROM workload_placements WHERE id=?"
        )
        .bind(PLACEMENT)
        .fetch_one(&db.pool)
        .await?
        .as_deref(),
        Some("legacy-runner")
    );
    for (target, kind) in [
        (TARGET, "github_repository"),
        (OTHER_TARGET, "bitbucket_workspace"),
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT kind FROM fleet_ci_targets WHERE id=?")
                .bind(target)
                .fetch_one(&db.pool)
                .await?,
            kind
        );
    }
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&db.pool)
            .await?
            .is_empty()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
            .fetch_one(&db.pool)
            .await?,
        "ok"
    );
    assert_foreign_keys_on_every_connection(&db.pool).await?;
    db.close().await
}

const GITLAB_CONNECTION: &str = "INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at) VALUES (?,'GitLab',?,?,?,'acme/tools',?,?,1,1)";

async fn gitlab_connection(
    pool: &SqlitePool,
    id: &str,
    base_url: &str,
    scope: (&str, i64),
) -> Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error> {
    sqlx::query(GITLAB_CONNECTION)
        .bind(id)
        .bind(base_url)
        .bind(scope.0)
        .bind(scope.1)
        .bind(format!("{id}-token-key"))
        .bind(format!("{id}-webhook-key"))
        .execute(pool)
        .await
}

#[tokio::test]
async fn gitlab_fleet_targets_satisfy_the_shape_check_and_stay_immutable() -> Result<()> {
    let db = Database::ready().await?;
    gitlab_connection(&db.pool, "gl-a", "https://gitlab.example.com", ("group", 7)).await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_project_id) VALUES ('gl-project','gitlab_project','gitlab:project:gl-a:42','gl-a',42)")
        .execute(&db.pool).await.context("a gitlab_project target is accepted")?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_group_id) VALUES ('gl-group','gitlab_group','gitlab:group:gl-a:7','gl-a',7)")
        .execute(&db.pool).await.context("a gitlab_group target is accepted")?;
    for sql in [
        // A GitHub column on a GitLab row.
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_project_id,installation_id) VALUES ('x1','gitlab_project','x1','gl-a',43,1)",
        // A Bitbucket column on a GitLab row.
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_group_id,bitbucket_connection_id) VALUES ('x2','gitlab_group','x2','gl-a',8,'legacy-bb')",
        // A project row without its project id, or with a zero one.
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id) VALUES ('x3','gitlab_project','x3','gl-a')",
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_project_id) VALUES ('x4','gitlab_project','x4','gl-a',0)",
        // A group row that also names a project, or has no connection.
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_group_id,gitlab_project_id) VALUES ('x5','gitlab_group','x5','gl-a',9,44)",
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_group_id) VALUES ('x6','gitlab_group','x6',10)",
        // A GitHub or Bitbucket row carrying a GitLab column.
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,repository_id,gitlab_project_id) VALUES ('x7','github_repository','x7',1,10,45)",
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,bitbucket_connection_id,workspace_uuid,gitlab_connection_id) VALUES ('x8','bitbucket_workspace','x8','legacy-bb','22222222-2222-4222-8222-222222222222','gl-a')",
        // An unknown kind, a reused canonical key, a non-integer id (STRICT).
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_project_id) VALUES ('x9','gitlab_pipeline','x9','gl-a',46)",
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_project_id) VALUES ('x10','gitlab_project','gitlab:project:gl-a:42','gl-a',47)",
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_project_id) VALUES ('x11','gitlab_project','x11','gl-a','forty-eight')",
        // A connection that does not exist.
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_project_id) VALUES ('x12','gitlab_project','x12','gl-missing',49)",
    ] {
        rejected(&db.pool, sql).await;
    }
    let refused =
        sqlx::query("UPDATE fleet_ci_targets SET gitlab_project_id=50 WHERE id='gl-project'")
            .execute(&db.pool)
            .await;
    assert!(
        refused.as_ref().err().is_some_and(|error| error
            .to_string()
            .contains("CI target identity is immutable")),
        "{refused:?}"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT strict FROM pragma_table_list WHERE name='fleet_ci_targets'"
        )
        .fetch_one(&db.pool)
        .await?,
        1,
        "fleet_ci_targets stays STRICT"
    );
    db.close().await
}

#[tokio::test]
async fn gitlab_tables_enforce_the_pinned_constraints() -> Result<()> {
    let db = Database::ready().await?;
    let pool = &db.pool;
    let base = "https://gitlab.example.com";
    gitlab_connection(pool, "gl-a", base, ("group", 7)).await?;
    // One connection per (instance, scope); the same id under the other kind
    // or on another instance is a different scope.
    assert!(
        gitlab_connection(pool, "gl-dup", base, ("group", 7))
            .await
            .is_err()
    );
    gitlab_connection(pool, "gl-b", base, ("project", 7)).await?;
    gitlab_connection(pool, "gl-c", "https://gitlab.other.example", ("group", 7)).await?;
    for (id, scope) in [("gl-kind", ("user", 8)), ("gl-zero", ("group", 0))] {
        assert!(
            gitlab_connection(pool, id, base, scope).await.is_err(),
            "{id} was accepted"
        );
    }
    // Each stored secret key belongs to exactly one connection.
    for sql in [
        "INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at) VALUES ('gl-token','GitLab','https://gitlab.example.com','group',98,'p','gl-a-token-key','fresh-webhook-key',1,1)",
        "INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at) VALUES ('gl-hook','GitLab','https://gitlab.example.com','group',99,'p','fresh-token-key','gl-a-webhook-key',1,1)",
    ] {
        rejected(pool, sql).await;
    }
    assert_eq!(
        foreign_keys(pool, "gitlab_connections").await?,
        vec![(
            "created_by".to_owned(),
            "users".to_owned(),
            "id".to_owned(),
            "NO ACTION".to_owned(),
            "SET NULL".to_owned()
        )]
    );

    // Pool membership mirrors runner_pool_bitbucket_connections.
    sqlx::query("INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES ('legacy-pool','gl-b',1)")
        .execute(pool).await?;
    rejected(pool, "INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES ('legacy-pool','gl-b',2)").await;
    rejected(pool, "INSERT INTO runner_pool_gitlab_connections (pool_id,connection_id,created_at) VALUES ('legacy-pool','gl-missing',1)").await;
    assert_eq!(
        created_indexes(pool, "runner_pool_gitlab_connections").await?,
        BTreeMap::from([(
            "runner_pool_gitlab_connections_connection_idx".to_owned(),
            (false, "connection_id,pool_id".to_owned())
        )])
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT run_untagged FROM runner_pools WHERE id='legacy-pool'"
        )
        .fetch_one(pool)
        .await?,
        0
    );

    // Jobs: one row per (connection, GitLab job id), with the pinned defaults.
    let job = "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,status_changed_at,created_at,updated_at) VALUES (?,?,42,1001,500,'pending',1,1,1)";
    sqlx::query(job)
        .bind("job-a")
        .bind("gl-b")
        .execute(pool)
        .await?;
    assert!(
        sqlx::query(job)
            .bind("job-dup")
            .bind("gl-b")
            .execute(pool)
            .await
            .is_err(),
        "(connection_id, job_id) is unique"
    );
    sqlx::query(job)
        .bind("job-c")
        .bind("gl-c")
        .execute(pool)
        .await?;
    let defaults: (String, i64, Option<i64>, Option<String>) = sqlx::query_as(
        "SELECT tag_list,retries_count,gitlab_runner_id,pool_id FROM gitlab_jobs WHERE id='job-a'",
    )
    .fetch_one(pool)
    .await?;
    assert_eq!(defaults, ("[]".to_owned(), 0, None, None));
    assert_eq!(
        created_indexes(pool, "gitlab_jobs").await?,
        BTreeMap::from([
            (
                "gitlab_jobs_active_created_idx".to_owned(),
                (false, "created_at,id".to_owned())
            ),
            (
                "gitlab_jobs_connection_job_unique".to_owned(),
                (true, "connection_id,job_id".to_owned())
            ),
        ])
    );
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT partial FROM pragma_index_list('gitlab_jobs') WHERE name='gitlab_jobs_active_created_idx'")
            .fetch_one(pool)
            .await?,
        "the active-jobs index is partial"
    );
    assert_eq!(
        foreign_keys(pool, "gitlab_jobs").await?,
        vec![
            (
                "connection_id".to_owned(),
                "gitlab_connections".to_owned(),
                "id".to_owned(),
                "NO ACTION".to_owned(),
                "CASCADE".to_owned()
            ),
            (
                "pool_id".to_owned(),
                "runner_pools".to_owned(),
                "id".to_owned(),
                "NO ACTION".to_owned(),
                "SET NULL".to_owned()
            ),
        ]
    );

    // A runner keeps its GitLab runner id when its connection goes, and the
    // connection's memberships and jobs go with it.
    sqlx::query("INSERT INTO runners (id,pool_id,name,ci_platform,gitlab_connection_id,gitlab_runner_id,created_at,updated_at) VALUES ('gl-runner','legacy-pool','gl-runner','gitlab','gl-b',77,1,1)")
        .execute(pool).await?;
    sqlx::query("DELETE FROM gitlab_connections WHERE id='gl-b'")
        .execute(pool)
        .await?;
    let runner: (Option<String>, Option<i64>) = sqlx::query_as(
        "SELECT gitlab_connection_id,gitlab_runner_id FROM runners WHERE id='gl-runner'",
    )
    .fetch_one(pool)
    .await?;
    assert_eq!(runner, (None, Some(77)));
    for table in ["runner_pool_gitlab_connections", "gitlab_jobs"] {
        // Names come from the list above, never request input.
        let rows: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT COUNT(*) FROM {table} WHERE connection_id='gl-b'"
        )))
        .fetch_one(pool)
        .await?;
        assert_eq!(rows, 0, "{table} kept rows of a deleted connection");
    }
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(pool)
            .await?
            .is_empty()
    );
    db.close().await
}

#[tokio::test]
async fn a_gitlab_connection_a_fleet_target_references_cannot_be_deleted() -> Result<()> {
    let db = Database::ready().await?;
    let pool = &db.pool;
    for (connection, scope) in [
        ("gl-project-conn", ("project", 42)),
        ("gl-group-conn", ("group", 7)),
        ("gl-unused", ("group", 8)),
    ] {
        gitlab_connection(pool, connection, "https://gitlab.example.com", scope).await?;
    }
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_project_id) VALUES ('gl-project','gitlab_project','gitlab:project:gl-project-conn:42','gl-project-conn',42)")
        .execute(pool).await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_group_id) VALUES ('gl-group','gitlab_group','gitlab:group:gl-group-conn:7','gl-group-conn',7)")
        .execute(pool).await?;
    let tables = [
        "gitlab_connections".to_owned(),
        "fleet_ci_targets".to_owned(),
    ];
    let before = records(pool, &tables).await?;
    for connection in ["gl-project-conn", "gl-group-conn"] {
        let refused = sqlx::query("DELETE FROM gitlab_connections WHERE id=?")
            .bind(connection)
            .execute(pool)
            .await;
        assert!(
            refused
                .as_ref()
                .err()
                .is_some_and(|error| error.to_string().contains("FOREIGN KEY constraint failed")),
            "deleting {connection}: {refused:?}"
        );
        assert_eq!(
            records(pool, &tables).await?,
            before,
            "{connection} delete changed rows"
        );
    }
    let deleted = sqlx::query("DELETE FROM gitlab_connections WHERE id='gl-unused'")
        .execute(pool)
        .await?;
    assert_eq!(
        deleted.rows_affected(),
        1,
        "an unreferenced connection deletes"
    );
    db.close().await
}

#[tokio::test]
async fn the_0047_foreign_key_guard_fails_on_a_dangling_reference() -> Result<()> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/0047_gitlab.sql"
    );
    let migration = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    assert_foreign_key_guard(&migration, "_gitlab_0047_fk_guard", "0047").await
}

/// #183: 0048 rebuilds `fleet_ci_targets` with foreign keys off, as 0047 did,
/// so it carries the same guard: a temp table whose CHECK refuses to commit
/// while `pragma_foreign_key_check` reports any dangling reference.
#[tokio::test]
async fn the_0048_foreign_key_guard_fails_on_a_dangling_reference() -> Result<()> {
    let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        let numbered = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("0048_"));
        if numbered && path.extension().is_some_and(|extension| extension == "sql") {
            found.push(path);
        }
    }
    let [path] = found.as_slice() else {
        anyhow::bail!("expected exactly one migration 0048 in {directory}, found {found:?}");
    };
    let migration =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    assert_foreign_key_guard(&migration, "_fk_guard", "0048").await
}

/// The statements of `migration` that mention `marker` are its foreign-key
/// guard (create, fill, drop), and run on their own they refuse a database
/// with a dangling reference and accept a clean one.
async fn assert_foreign_key_guard(migration: &str, marker: &str, name: &str) -> Result<()> {
    // Comments are dropped first, so a `;` inside one cannot split a statement.
    let code: String = migration
        .lines()
        .map(|line| line.split("--").next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    let guard: Vec<String> = code
        .split(';')
        .filter(|statement| statement.contains(marker))
        .map(|statement| statement.trim().to_owned())
        .collect();
    let starts: Vec<_> = guard
        .iter()
        .map(|statement| statement.split_whitespace().next().unwrap_or_default())
        .collect();
    assert_eq!(
        starts,
        ["CREATE", "INSERT", "DROP"],
        "{name}'s guard: {guard:?}"
    );
    let guard = format!("{};", guard.join(";\n"));
    for dangling in [false, true] {
        let directory =
            std::env::temp_dir().join(format!("gridops-fk-guard-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory)?;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(directory.join("guard.sqlite"))
                    .create_if_missing(true)
                    .foreign_keys(false),
            )
            .await?;
        sqlx::raw_sql(
            "CREATE TABLE parent (id TEXT PRIMARY KEY);
             CREATE TABLE child (id TEXT PRIMARY KEY, parent_id TEXT REFERENCES parent(id));
             INSERT INTO parent (id) VALUES ('present');
             INSERT INTO child (id,parent_id) VALUES ('ok','present');",
        )
        .execute(&pool)
        .await?;
        if dangling {
            sqlx::query("INSERT INTO child (id,parent_id) VALUES ('orphan','missing')")
                .execute(&pool)
                .await?;
        }
        let outcome = sqlx::raw_sql(sqlx::AssertSqlSafe(guard.clone()))
            .execute(&pool)
            .await;
        if dangling {
            assert!(
                outcome
                    .as_ref()
                    .err()
                    .is_some_and(|error| error.to_string().contains("CHECK constraint failed")),
                "the guard let a dangling reference through: {outcome:?}"
            );
        } else {
            assert!(
                outcome.is_ok(),
                "the guard refused a clean database: {outcome:?}"
            );
        }
        pool.close().await;
        std::fs::remove_dir_all(directory)?;
    }
    Ok(())
}

/// A GitLab project target of `gl-a`, beside the fixture's GitHub and
/// Bitbucket targets.
const GITLAB_TARGET: &str = "30000000-0000-4000-8000-000000000003";

/// #183, 0048: the `fleet_ci_targets` rebuild that adds `retired_at`, run on
/// a 0047 database that already holds a GitLab target with a machine grant,
/// a profile link and a fleet event. Every row and every foreign key survives
/// it, `retired_at` is appended last and NULL on every existing target, the
/// identity trigger is back under its name, and the table stays STRICT.
#[tokio::test]
async fn the_0048_rebuild_keeps_every_target_and_reference() -> Result<()> {
    let db = Database::legacy().await?;
    MIGRATOR.run_to(47, &db.pool).await?;
    sqlx::raw_sql(include_str!("fixtures/fleet_ready.sql"))
        .execute(&db.pool)
        .await?;
    gitlab_connection(
        &db.pool,
        "gl-a",
        "https://gitlab.example.com",
        ("project", 42),
    )
    .await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_project_id) VALUES (?,'gitlab_project','gitlab:project:gl-a:42','gl-a',42)")
        .bind(GITLAB_TARGET).execute(&db.pool).await?;
    sqlx::query("INSERT INTO host_target_grants (host_id,target_id,allow_schedule) VALUES (?,?,1)")
        .bind(HOST)
        .bind(GITLAB_TARGET)
        .execute(&db.pool)
        .await?;
    sqlx::query("INSERT INTO profile_ci_targets (profile_id,target_id) VALUES ('40000000-0000-4000-8000-000000000001',?)")
        .bind(GITLAB_TARGET).execute(&db.pool).await?;
    sqlx::query(
        "INSERT INTO fleet_events (id,target_id,kind,created_at) VALUES ('gl-event',?,'test',1)",
    )
    .bind(GITLAB_TARGET)
    .execute(&db.pool)
    .await?;
    let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != '_sqlx_migrations' ORDER BY name")
        .fetch_all(&db.pool).await?;
    let old_schema = schema(&db.pool).await?;
    let old_columns = table_columns(&db.pool, &tables).await?;
    let old_records = records(&db.pool, &tables).await?;
    let mut old_foreign_keys = BTreeMap::new();
    for table in &tables {
        old_foreign_keys.insert(table.clone(), foreign_keys(&db.pool, table).await?);
    }

    MIGRATOR.run(&db.pool).await?;

    assert_eq!(records_on(&db.pool, &old_columns).await?, old_records);
    assert_added_columns(
        &db.pool,
        &old_columns,
        &[("fleet_ci_targets", &[("retired_at", "INTEGER", None)])],
    )
    .await?;
    let new_schema = schema(&db.pool).await?;
    for (name, definition) in &old_schema {
        if name == "fleet_ci_targets" || name == "fleet_ci_targets_immutable_identity" {
            continue;
        }
        assert_eq!(
            new_schema.get(name),
            Some(definition),
            "changed schema: {name}"
        );
    }
    assert!(
        new_schema.contains_key("fleet_ci_targets_immutable_identity"),
        "the immutable-identity trigger is recreated under its name"
    );
    for table in &tables {
        assert_eq!(
            Some(&foreign_keys(&db.pool, table).await?),
            old_foreign_keys.get(table),
            "{table} foreign keys"
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT strict FROM pragma_table_list WHERE name='fleet_ci_targets'"
        )
        .fetch_one(&db.pool)
        .await?,
        1,
        "fleet_ci_targets stays STRICT"
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&db.pool)
            .await?
            .is_empty()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
            .fetch_one(&db.pool)
            .await?,
        "ok"
    );
    assert_foreign_keys_on_every_connection(&db.pool).await?;
    db.close().await
}

/// #183 decision 1: retiring a GitLab target is one UPDATE that sets
/// `retired_at` and drops its connection. After it the connection can be
/// deleted while every reference to the target stays, and the retired row
/// never changes again: no un-retiring, no re-retiring, no new connection, no
/// identity edit. Dropping the connection without retiring, or retiring while
/// changing an identity column, is refused; an unretired target still blocks
/// its connection's delete.
#[tokio::test]
async fn retiring_a_gitlab_target_frees_its_connection_and_is_final() -> Result<()> {
    let db = Database::ready().await?;
    let pool = &db.pool;
    let base = "https://gitlab.example.com";
    gitlab_connection(pool, "gl-a", base, ("project", 42)).await?;
    gitlab_connection(pool, "gl-b", base, ("group", 7)).await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_project_id) VALUES ('gl-project','gitlab_project','gitlab:project:gl-a:42','gl-a',42)")
        .execute(pool).await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_group_id) VALUES ('gl-group','gitlab_group','gitlab:group:gl-b:7','gl-b',7)")
        .execute(pool).await?;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "INSERT INTO host_target_grants (host_id,target_id,allow_schedule) VALUES ('{HOST}','gl-project',1);
         INSERT INTO profile_ci_targets (profile_id,target_id) VALUES ('40000000-0000-4000-8000-000000000001','gl-project');
         INSERT INTO fleet_events (id,target_id,kind,created_at) VALUES ('gl-event','gl-project','test',1);"
    )))
    .execute(pool)
    .await?;

    for sql in [
        // The connection cannot be dropped from a target that isn't retired.
        "UPDATE fleet_ci_targets SET gitlab_connection_id=NULL WHERE id='gl-project'",
        // A retire cannot carry an identity change with it.
        "UPDATE fleet_ci_targets SET retired_at=10,gitlab_connection_id=NULL,gitlab_project_id=43 WHERE id='gl-project'",
        "UPDATE fleet_ci_targets SET retired_at=10,gitlab_connection_id=NULL,canonical_key='renamed' WHERE id='gl-project'",
        "UPDATE fleet_ci_targets SET retired_at=10,gitlab_connection_id=NULL,kind='gitlab_group',gitlab_project_id=NULL,gitlab_group_id=42 WHERE id='gl-project'",
    ] {
        rejected(pool, sql).await;
    }

    let retired = sqlx::query(
        "UPDATE fleet_ci_targets SET retired_at=10,gitlab_connection_id=NULL WHERE id='gl-project'",
    )
    .execute(pool)
    .await
    .context("0048 allows the retire: retired_at set and the connection dropped in one UPDATE")?;
    assert_eq!(retired.rows_affected(), 1);
    let row: (String, String, Option<String>, Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT kind,canonical_key,gitlab_connection_id,gitlab_project_id,retired_at FROM fleet_ci_targets WHERE id='gl-project'",
    )
    .fetch_one(pool)
    .await?;
    assert_eq!(
        row,
        (
            "gitlab_project".to_owned(),
            "gitlab:project:gl-a:42".to_owned(),
            None,
            Some(42),
            Some(10)
        ),
        "the retired row keeps its identity"
    );

    let deleted = sqlx::query("DELETE FROM gitlab_connections WHERE id='gl-a'")
        .execute(pool)
        .await
        .context("a connection whose only target is retired deletes")?;
    assert_eq!(deleted.rows_affected(), 1);
    for table in ["host_target_grants", "profile_ci_targets", "fleet_events"] {
        // Names come from the list above, never request input.
        let rows: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT COUNT(*) FROM {table} WHERE target_id='gl-project'"
        )))
        .fetch_one(pool)
        .await?;
        assert_eq!(rows, 1, "{table} keeps its reference to the retired target");
    }
    let refused = sqlx::query("DELETE FROM gitlab_connections WHERE id='gl-b'")
        .execute(pool)
        .await;
    assert!(
        refused
            .as_ref()
            .err()
            .is_some_and(|error| error.to_string().contains("FOREIGN KEY constraint failed")),
        "an unretired target still blocks its connection's delete: {refused:?}"
    );

    let before = records(pool, &["fleet_ci_targets".to_owned()]).await?;
    for sql in [
        "UPDATE fleet_ci_targets SET retired_at=NULL WHERE id='gl-project'",
        "UPDATE fleet_ci_targets SET retired_at=11 WHERE id='gl-project'",
        "UPDATE fleet_ci_targets SET gitlab_connection_id='gl-b' WHERE id='gl-project'",
        "UPDATE fleet_ci_targets SET retired_at=NULL,gitlab_connection_id='gl-b' WHERE id='gl-project'",
        "UPDATE fleet_ci_targets SET canonical_key='gitlab:project:gl-b:42' WHERE id='gl-project'",
        "UPDATE fleet_ci_targets SET gitlab_project_id=43 WHERE id='gl-project'",
    ] {
        rejected(pool, sql).await;
    }
    assert_eq!(
        records(pool, &["fleet_ci_targets".to_owned()]).await?,
        before,
        "a retired target never changes again"
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(pool)
            .await?
            .is_empty()
    );
    db.close().await
}
