//! Contract tests for #200 (the hourly machine rollup), written by #197.
//!
//! #200 removes the `#[ignore = "contract: #200"]` markers and makes these
//! pass; it changes nothing else here (`Fleet health guard`). The rules are
//! #200's "Work" list as `docs/design/fleet-health.md` records them. For
//! each machine and finished hour `[hour, hour + 1 h)`, one
//! `fleet_host_hourly` row:
//! - `online_minutes`: minutes of the hour with at least one `host_samples`
//!   row (by `observed_at`); no samples is a row with 0 and NULL use;
//! - CPU used p50 and p95, memory used p95, and the minimum `disk_free_bytes`
//!   from those samples;
//! - `start_attempts`: placements of the machine that reached `starting` in
//!   the hour; `uncertain_placements`: those that ended `uncertain` in it;
//! - `start_failures_json`: `placement_failed` events of the hour that
//!   `start_health::is_start_failure` counts, by reason code.
//!
//! `rollup_hour(db, hour, now)` writes one hour idempotently;
//! `run_pass(db, now)` is the per-pass entry, as for #191.
//!
//! Mounted from `main.rs` (#197-owned). Samples, placements and failures
//! are seeded with #197's `testdata`.

use super::*;
use gridops_core::{
    connect_database_path,
    fleet::PlacementFailureReason,
    fleet_health::{
        speed_check,
        testdata::{self, PlacementEnd, SampleSeed, SeededFleet},
    },
};

const SECOND: i64 = 1_000;
const MINUTE: i64 = 60 * SECOND;
const HOUR: i64 = 60 * MINUTE;
const GIB: i64 = 1024 * 1024 * 1024;
/// A whole hour; every test reads as of this time.
const NOW: i64 = 1_800_000_000_000;
/// The hour the tests roll up.
const H: i64 = NOW - 2 * HOUR;

/// A row of `fleet_host_hourly` without `computed_at`.
#[derive(Debug, PartialEq, Eq)]
struct Row {
    host_id: String,
    online_minutes: i64,
    cpu_p50: Option<i64>,
    cpu_p95: Option<i64>,
    memory_p95: Option<i64>,
    min_disk_free: Option<i64>,
    start_attempts: i64,
    start_failures: Value,
    uncertain: i64,
}

async fn rows(db: &SqlitePool, hour: i64) -> Result<Vec<Row>> {
    let raw = sqlx::query(
        "SELECT host_id,online_minutes,cpu_used_p50_millis,cpu_used_p95_millis,memory_used_p95_mib,
                min_disk_free_bytes,start_attempts,start_failures_json,uncertain_placements
           FROM fleet_host_hourly WHERE hour=? ORDER BY host_id",
    )
    .bind(hour)
    .fetch_all(db)
    .await?;
    let mut rows = Vec::new();
    for row in raw {
        rows.push(Row {
            host_id: row.get("host_id"),
            online_minutes: row.get("online_minutes"),
            cpu_p50: row.get("cpu_used_p50_millis"),
            cpu_p95: row.get("cpu_used_p95_millis"),
            memory_p95: row.get("memory_used_p95_mib"),
            min_disk_free: row.get("min_disk_free_bytes"),
            start_attempts: row.get("start_attempts"),
            start_failures: serde_json::from_str(&row.get::<String, _>("start_failures_json"))?,
            uncertain: row.get("uncertain_placements"),
        });
    }
    Ok(rows)
}

fn row_for<'a>(rows: &'a [Row], host_id: &str) -> Result<&'a Row> {
    rows.iter()
        .find(|row| row.host_id == host_id)
        .with_context(|| format!("no row for {host_id}: {rows:?}"))
}

struct Harness {
    directory: std::path::PathBuf,
    db: SqlitePool,
    fleet: SeededFleet,
}

impl Harness {
    async fn new() -> Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("gridops-host-rollup-{}", uuid::Uuid::new_v4()));
        let db = connect_database_path(&directory.join("gridops.sqlite")).await?;
        let fleet = testdata::seed_fleet(&db).await?;
        Ok(Self {
            directory,
            db,
            fleet,
        })
    }

    fn id(&self, name: &str) -> Result<String> {
        Ok(self.fleet.host(name)?.id.clone())
    }

    /// One sample every 30 s from `from` for `count` samples, `shape`
    /// adjusting each (by its index).
    async fn samples(
        &self,
        name: &str,
        from: i64,
        count: i64,
        shape: impl Fn(i64, &mut SampleSeed),
    ) -> Result<()> {
        let samples: Vec<SampleSeed> = (0..count)
            .map(|index| {
                let mut sample = SampleSeed::at(from + index * 30 * SECOND);
                shape(index, &mut sample);
                sample
            })
            .collect();
        testdata::insert_samples(&self.db, self.fleet.host(name)?, &samples).await
    }

    async fn finish(self) -> Result<()> {
        self.db.close().await;
        std::fs::remove_dir_all(self.directory)?;
        Ok(())
    }
}

/// The value at sorted position `index` of the known CPU series: 40 samples
/// at 500, 60 at 1,500, 18 at 4,000 and 2 at 9,000. Any usual percentile
/// method gives p50 = 1,500 and p95 = 4,000 on it.
fn cpu_series(sorted_index: i64) -> u32 {
    match sorted_index {
        0..=39 => 500,
        40..=99 => 1_500,
        100..=117 => 4_000,
        _ => 9_000,
    }
}

#[tokio::test]
async fn an_hour_of_samples_starts_and_failures_gives_one_row_per_machine() -> Result<()> {
    let harness = Harness::new().await?;
    let io = harness.id("io")?;
    let jupiter = harness.id("Jupiter")?;
    let callisto = harness.id("callisto")?;
    let europa = harness.id("europa")?;
    let laptop = harness.id("MacBookPro")?;

    // io: a full hour of samples, one with 20 GiB free.
    harness
        .samples("io", H, 120, |index, sample| {
            if index == 77 {
                sample.disk_free_bytes = Some(20 * GIB);
            }
        })
        .await?;
    // Jupiter: a full hour of the known CPU series (out of time order) and
    // memory at 2,048 MiB but for 10 samples at 6,000 (p95 = 6,000).
    harness
        .samples("Jupiter", H, 120, |index, sample| {
            let scrambled = (index * 7) % 120;
            sample.cpu_used_millis = cpu_series(scrambled);
            sample.memory_used_mib = if scrambled >= 110 { 6_000 } else { 2_048 };
        })
        .await?;
    // callisto: offline for the second half of the hour.
    harness.samples("callisto", H, 60, |_, _| {}).await?;
    // Samples just outside the hour count for neither.
    harness
        .samples("europa", H - 30 * SECOND, 1, |_, _| {})
        .await?;
    harness.samples("europa", H + HOUR, 1, |_, _| {}).await?;

    // io's starts: three in the hour (one ends uncertain), one before it.
    let io_host = harness.fleet.host("io")?.clone();
    for (at, end) in [
        (H + 5 * MINUTE, PlacementEnd::Stopped),
        (H + 15 * MINUTE, PlacementEnd::Failed),
        (H + 25 * MINUTE, PlacementEnd::Uncertain),
        (H - MINUTE, PlacementEnd::Stopped),
    ] {
        testdata::insert_start_attempt(&harness.db, &harness.fleet, &io_host, at, end).await?;
    }
    // io's failures: the machine's fault three times; not its fault twice
    // (an agent-reported cancellation, an operator release); one next hour.
    for (at, reason, code) in [
        (H + 6 * MINUTE, PlacementFailureReason::AgentReported, None),
        (
            H + 16 * MINUTE,
            PlacementFailureReason::StartDeadlineExpired,
            None,
        ),
        (
            H + 26 * MINUTE,
            PlacementFailureReason::StartDeadlineExpired,
            None,
        ),
        (
            H + 36 * MINUTE,
            PlacementFailureReason::AgentReported,
            Some("cancelled"),
        ),
        (
            H + 46 * MINUTE,
            PlacementFailureReason::OperatorReleased,
            None,
        ),
        (H + HOUR, PlacementFailureReason::StartDeadlineExpired, None),
    ] {
        testdata::insert_placement_failure(&harness.db, &io_host, at, reason, code).await?;
    }

    fleet_host_rollup::rollup_hour(&harness.db, H, NOW).await?;
    let rows = rows(&harness.db, H).await?;
    assert_eq!(
        rows.len(),
        5,
        "one row per machine, samples or not: {rows:?}"
    );
    let default_disk = testdata::DISK_BYTES / 2;
    let quiet = |host_id: &str, online_minutes: i64, used: Option<(i64, i64, i64, i64)>| Row {
        host_id: host_id.to_owned(),
        online_minutes,
        cpu_p50: used.map(|used| used.0),
        cpu_p95: used.map(|used| used.1),
        memory_p95: used.map(|used| used.2),
        min_disk_free: used.map(|used| used.3),
        start_attempts: 0,
        start_failures: json!({}),
        uncertain: 0,
    };
    // `SampleSeed::at` reports 1,000 CPU millis, 1,024 MiB and half the
    // 100 GiB disk free.
    assert_eq!(
        row_for(&rows, &io)?,
        &Row {
            start_attempts: 3,
            start_failures: json!({"agent_reported": 1, "start_deadline_expired": 2}),
            uncertain: 1,
            ..quiet(&io, 60, Some((1_000, 1_000, 1_024, 20 * GIB)))
        },
        "a full hour"
    );
    assert_eq!(
        row_for(&rows, &jupiter)?,
        &quiet(&jupiter, 60, Some((1_500, 4_000, 6_000, default_disk))),
        "percentiles on the known series"
    );
    assert_eq!(
        row_for(&rows, &callisto)?,
        &quiet(&callisto, 30, Some((1_000, 1_000, 1_024, default_disk))),
        "offline for half the hour"
    );
    assert_eq!(
        row_for(&rows, &europa)?,
        &quiet(&europa, 0, None),
        "no samples in the hour"
    );
    assert_eq!(row_for(&rows, &laptop)?, &quiet(&laptop, 0, None));
    harness.finish().await
}

#[tokio::test]
async fn rolling_a_machine_hour_up_again_gives_the_same_row() -> Result<()> {
    let harness = Harness::new().await?;
    harness
        .samples("io", H, 90, |index, sample| {
            sample.cpu_used_millis = 500 + u32::try_from(index).unwrap_or(0) * 10;
        })
        .await?;
    let io_host = harness.fleet.host("io")?.clone();
    testdata::insert_start_attempt(
        &harness.db,
        &harness.fleet,
        &io_host,
        H + MINUTE,
        PlacementEnd::Uncertain,
    )
    .await?;
    testdata::insert_placement_failure(
        &harness.db,
        &io_host,
        H + 2 * MINUTE,
        PlacementFailureReason::AgentSessionLost,
        None,
    )
    .await?;
    let snapshot = || {
        sqlx::query_scalar::<_, String>(
            "SELECT json_group_array(json_array(hour,host_id,online_minutes,cpu_used_p50_millis,
                    cpu_used_p95_millis,memory_used_p95_mib,min_disk_free_bytes,start_attempts,
                    start_failures_json,uncertain_placements,computed_at))
               FROM (SELECT * FROM fleet_host_hourly ORDER BY hour,host_id)",
        )
        .fetch_one(&harness.db)
    };
    fleet_host_rollup::rollup_hour(&harness.db, H, NOW).await?;
    let first = snapshot().await?;
    fleet_host_rollup::rollup_hour(&harness.db, H, NOW).await?;
    assert_eq!(snapshot().await?, first, "the same rows, none added");
    let io_row = rows(&harness.db, H).await?;
    let io_row = row_for(&io_row, &harness.id("io")?)?;
    assert_eq!(io_row.online_minutes, 45);
    assert_eq!(io_row.start_failures, json!({"agent_session_lost": 1}));
    harness.finish().await
}

#[tokio::test]
async fn a_pass_rolls_up_a_listed_hour_and_takes_it_off_the_list() -> Result<()> {
    let harness = Harness::new().await?;
    harness.samples("io", H, 120, |_, _| {}).await?;
    sqlx::query("INSERT INTO fleet_health_recompute (hour,requested_at) VALUES (?,?)")
        .bind(H)
        .bind(NOW - MINUTE)
        .execute(&harness.db)
        .await?;
    fleet_host_rollup::run_pass(&harness.db, NOW).await?;
    let rows = rows(&harness.db, H).await?;
    assert_eq!(row_for(&rows, &harness.id("io")?)?.online_minutes, 60);
    harness.finish().await
}

#[tokio::test]
async fn seven_days_of_samples_for_5_machines_roll_up_with_no_long_write_transaction() -> Result<()>
{
    let directory =
        std::env::temp_dir().join(format!("gridops-host-rollup-week-{}", uuid::Uuid::new_v4()));
    let db = connect_database_path(&directory.join("gridops.sqlite")).await?;
    let week = testdata::build_week(&db, NOW).await?;
    assert_eq!(week.samples, 5 * 7 * 24 * 120, "one sample every 30 s");
    speed_check::check(&db, async {
        for hour in (0..7 * 24).map(|index| NOW - 7 * 24 * HOUR + index * HOUR) {
            fleet_host_rollup::rollup_hour(&db, hour, NOW).await?;
        }
        Ok(())
    })
    .await?;
    let (rows, online): (i64, i64) =
        sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(online_minutes),0) FROM fleet_host_hourly")
            .fetch_one(&db)
            .await?;
    assert_eq!(rows, 5 * 7 * 24, "a row per machine per hour");
    assert_eq!(online, 5 * 7 * 24 * 60, "every minute has a sample");
    db.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
