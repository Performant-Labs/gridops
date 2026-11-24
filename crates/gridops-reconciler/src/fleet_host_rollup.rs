//! The fleet health per-machine hourly rollup (#200): one
//! `fleet_host_hourly` row per (hour, machine) from its resource samples
//! (online minutes, CPU and memory p50/p95, disk headroom), its start
//! attempts and failures, and its uncertain placements. A machine with no
//! samples in the hour still gets a row (0 minutes, NULL percentiles).
//!
//! [`run_pass`] runs every reconciler pass, log-and-continue (`reconcile()`),
//! rolling up every hour listed in `fleet_health_recompute` plus at most one
//! catch-up hour, oldest first; on first start the catch-up covers the last
//! 7 days. The reads happen outside the write transaction, each bounded by an
//! indexed time column (`host_samples_host_time_idx`,
//! `fleet_events_host_kind_time_idx`, `host_commands_host_kind_issued_idx`,
//! `workload_placements_state_idx`); the rows of an hour are written in one
//! short transaction (#179), one `INSERT … ON CONFLICT DO UPDATE` per row,
//! with the hour taken off `fleet_health_recompute` in it, so re-running an
//! hour gives identical rows.

use std::collections::BTreeMap;

use anyhow::Result;
use gridops_core::fleet::start_health::is_start_failure;
use gridops_core::fleet::{PlacementFailureReason, failure::PLACEMENT_FAILED_EVENT};
use serde_json::Value;
use sqlx::SqlitePool;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
/// How far back the catch-up reaches: on first start, and again after the
/// rollup has fallen further behind than this (a long pause), the catch-up
/// covers the last 7 days and nothing older.
const CATCH_UP_MILLIS: i64 = 7 * 24 * HOUR;
/// The settings key holding the last hour the catch-up rolled up, so the
/// walk continues where it left off instead of reading the table for it.
const LAST_ROLLED_KEY: &str = "lastHostRollupHour";
/// Rows per write transaction (#179: writes in batches of at most 500 rows).
const WRITE_BATCH: usize = 500;

/// One sample row: when it was observed, and the CPU, memory and disk free
/// it reported (`None` when the machine did not say).
type SampleRow = (i64, Option<i64>, Option<i64>, Option<i64>);

/// One machine's hour, read outside the write transaction and written whole.
struct HostHour {
    host_id: String,
    online_minutes: i64,
    cpu_used_p50_millis: Option<i64>,
    cpu_used_p95_millis: Option<i64>,
    memory_used_p95_mib: Option<i64>,
    min_disk_free_bytes: Option<i64>,
    start_attempts: i64,
    start_failures: BTreeMap<String, i64>,
    uncertain_placements: i64,
}

/// The whole hour `ms` falls in starts at.
fn floor_hour(ms: i64) -> i64 {
    ms - ms.rem_euclid(HOUR)
}

/// The first whole hour at or after `ms`.
fn ceil_hour(ms: i64) -> i64 {
    let remainder = ms.rem_euclid(HOUR);
    if remainder == 0 {
        ms
    } else {
        ms + HOUR - remainder
    }
}

/// The nearest-rank percentile `percent` (50, 95, …) of ascending `values`:
/// the value at rank ⌈percent × n / 100⌉. `None` without values.
fn nearest_rank(sorted: &[i64], percent: i64) -> Option<i64> {
    let count = i64::try_from(sorted.len()).ok()?;
    if count == 0 {
        return None;
    }
    let rank = (percent.saturating_mul(count).saturating_add(99) / 100).clamp(1, count);
    sorted.get(usize::try_from(rank - 1).ok()?).copied()
}

/// Reads one machine's hour. Every read is bounded by an indexed time column
/// (#179) and happens outside the write transaction; `uncertain_placements`
/// was counted for the whole hour in one query (`uncertain_counts`).
async fn read_host_hour(
    db: &SqlitePool,
    host_id: &str,
    hour: i64,
    uncertain_placements: i64,
) -> Result<HostHour> {
    // One bounded read per machine per hour on `host_samples_host_time_idx`
    // (host_id, observed_at): the online minutes, the CPU and memory series
    // for their percentiles, and the disk's least free.
    let samples: Vec<SampleRow> = sqlx::query_as(
        "SELECT observed_at,cpu_used_millis,memory_used_mib,disk_free_bytes FROM host_samples
          WHERE host_id=? AND observed_at>=? AND observed_at<?",
    )
    .bind(host_id)
    .bind(hour)
    .bind(hour + HOUR)
    .fetch_all(db)
    .await?;
    let mut minutes = 0_u64;
    let mut cpu_used = Vec::new();
    let mut memory_used = Vec::new();
    let mut min_disk_free: Option<i64> = None;
    for (observed_at, cpu_used_millis, memory_used_mib, disk_free_bytes) in samples {
        // The query keeps `observed_at` inside the hour, so this is a minute
        // of it; the clamp only keeps a stray row from shifting past the mask.
        let minute = ((observed_at - hour) / MINUTE).clamp(0, 59);
        minutes |= 1_u64 << u32::try_from(minute).unwrap_or(0);
        if let Some(cpu_used_millis) = cpu_used_millis {
            cpu_used.push(cpu_used_millis);
        }
        if let Some(memory_used_mib) = memory_used_mib {
            memory_used.push(memory_used_mib);
        }
        if let Some(disk_free_bytes) = disk_free_bytes {
            min_disk_free =
                Some(min_disk_free.map_or(disk_free_bytes, |least| least.min(disk_free_bytes)));
        }
    }
    cpu_used.sort_unstable();
    memory_used.sort_unstable();

    // The machine's placements that reached `starting` in the hour: a start
    // is issued exactly then, on `host_commands_host_kind_issued_idx`.
    let start_attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT placement_id) FROM host_commands
          WHERE host_id=? AND kind='start_runner' AND issued_at>=? AND issued_at<?",
    )
    .bind(host_id)
    .bind(hour)
    .bind(hour + HOUR)
    .fetch_one(db)
    .await?;

    // The hour's `placement_failed` events, read as `start_gate` reads them,
    // counted only when the machine was at fault, by reason. One bounded read
    // on `fleet_events_host_kind_time_idx`.
    let details: Vec<String> = sqlx::query_scalar(
        "SELECT detail_json FROM fleet_events
          WHERE host_id=? AND kind=? AND created_at>=? AND created_at<?",
    )
    .bind(host_id)
    .bind(PLACEMENT_FAILED_EVENT)
    .bind(hour)
    .bind(hour + HOUR)
    .fetch_all(db)
    .await?;
    let mut start_failures = BTreeMap::new();
    for detail_json in details {
        let detail: Value = serde_json::from_str(&detail_json).unwrap_or_default();
        let reason = PlacementFailureReason::from_code(
            detail.get("reason").and_then(Value::as_str).unwrap_or(""),
        );
        if is_start_failure(reason, detail.get("agent_code").and_then(Value::as_str)) {
            *start_failures
                .entry(reason.as_str().to_owned())
                .or_insert(0) += 1;
        }
    }

    Ok(HostHour {
        host_id: host_id.to_owned(),
        online_minutes: i64::from(minutes.count_ones()),
        cpu_used_p50_millis: nearest_rank(&cpu_used, 50),
        cpu_used_p95_millis: nearest_rank(&cpu_used, 95),
        memory_used_p95_mib: nearest_rank(&memory_used, 95),
        min_disk_free_bytes: min_disk_free,
        start_attempts,
        start_failures,
        uncertain_placements,
    })
}

/// The placements that ended `uncertain` in the hour, per machine. One
/// bounded read on `workload_placements_state_idx` (`state`, `updated_at`)
/// for the whole hour, instead of one unbounded read per machine.
async fn uncertain_counts(db: &SqlitePool, hour: i64) -> Result<BTreeMap<String, i64>> {
    let counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT host_id,COUNT(*) FROM workload_placements
          WHERE state='uncertain' AND updated_at>=? AND updated_at<? GROUP BY host_id",
    )
    .bind(hour)
    .bind(hour + HOUR)
    .fetch_all(db)
    .await?;
    Ok(counts.into_iter().collect())
}

/// Writes the hour's rows, replacing what is there, and takes the hour off
/// `fleet_health_recompute` in the same transaction, so a re-run gives
/// identical rows. At most [`WRITE_BATCH`] rows per transaction (#179); the
/// recompute row goes with the last of them.
async fn write_host_hours(db: &SqlitePool, hour: i64, now: i64, hours: &[HostHour]) -> Result<()> {
    let mut tx = db.begin().await?;
    for (index, host_hour) in hours.iter().enumerate() {
        sqlx::query(
            "INSERT INTO fleet_host_hourly (hour,host_id,online_minutes,cpu_used_p50_millis,
                cpu_used_p95_millis,memory_used_p95_mib,min_disk_free_bytes,start_attempts,
                start_failures_json,uncertain_placements,computed_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?)
             ON CONFLICT(hour,host_id) DO UPDATE SET
               online_minutes=excluded.online_minutes,
               cpu_used_p50_millis=excluded.cpu_used_p50_millis,
               cpu_used_p95_millis=excluded.cpu_used_p95_millis,
               memory_used_p95_mib=excluded.memory_used_p95_mib,
               min_disk_free_bytes=excluded.min_disk_free_bytes,
               start_attempts=excluded.start_attempts,
               start_failures_json=excluded.start_failures_json,
               uncertain_placements=excluded.uncertain_placements,
               computed_at=excluded.computed_at",
        )
        .bind(hour)
        .bind(&host_hour.host_id)
        .bind(host_hour.online_minutes)
        .bind(host_hour.cpu_used_p50_millis)
        .bind(host_hour.cpu_used_p95_millis)
        .bind(host_hour.memory_used_p95_mib)
        .bind(host_hour.min_disk_free_bytes)
        .bind(host_hour.start_attempts)
        .bind(serde_json::to_string(&host_hour.start_failures)?)
        .bind(host_hour.uncertain_placements)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if (index + 1) % WRITE_BATCH == 0 && index + 1 < hours.len() {
            tx.commit().await?;
            tx = db.begin().await?;
        }
    }
    sqlx::query("DELETE FROM fleet_health_recompute WHERE hour=?")
        .bind(hour)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Writes the rows of the hour starting at `hour`, replacing what is there.
///
/// # Errors
/// A database read or write failed.
pub(crate) async fn rollup_hour(db: &SqlitePool, hour: i64, now: i64) -> Result<()> {
    let hosts: Vec<String> = sqlx::query_scalar("SELECT id FROM fleet_hosts ORDER BY id")
        .fetch_all(db)
        .await?;
    let uncertain = uncertain_counts(db, hour).await?;
    let mut hours = Vec::with_capacity(hosts.len());
    for host_id in &hosts {
        let uncertain_placements = uncertain.get(host_id).copied().unwrap_or(0);
        hours.push(read_host_hour(db, host_id, hour, uncertain_placements).await?);
    }
    write_host_hours(db, hour, now, &hours).await
}

/// One reconciler pass of the per-machine rollup: every hour listed in
/// `fleet_health_recompute`, then at most one catch-up hour, oldest first.
/// The catch-up walk remembers the last hour it rolled up in
/// `settings` ([`LAST_ROLLED_KEY`]); on first start, and again after falling
/// more than [`CATCH_UP_MILLIS`] behind, it covers the last 7 days. It never
/// rolls an hour that has not finished.
///
/// # Errors
/// A database read or write failed; `reconcile()` logs it and goes on.
pub(crate) async fn run_pass(db: &SqlitePool, now: i64) -> Result<()> {
    let listed: Vec<i64> =
        sqlx::query_scalar("SELECT hour FROM fleet_health_recompute ORDER BY hour")
            .fetch_all(db)
            .await?;
    for hour in listed {
        rollup_hour(db, hour, now).await?;
    }

    let now_hour = floor_hour(now);
    let oldest = ceil_hour(now) - CATCH_UP_MILLIS;
    let rolled = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
        .bind(LAST_ROLLED_KEY)
        .fetch_optional(db)
        .await?
        .and_then(|value| serde_json::from_str::<i64>(&value).ok());
    // The hour after the last one rolled up, never older than the 7-day reach.
    let next = match rolled {
        Some(rolled) if rolled + HOUR >= oldest => rolled + HOUR,
        _ => oldest,
    };
    if next < now_hour {
        rollup_hour(db, next, now).await?;
        sqlx::query(
            "INSERT INTO settings (key,value,updated_at) VALUES (?,?,?)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",
        )
        .bind(LAST_ROLLED_KEY)
        .bind(next.to_string())
        .bind(now)
        .execute(db)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The known CPU series of the contract tests: 40 samples at 500, 60 at
    /// 1,500, 18 at 4,000 and 2 at 9,000. p50 = 1,500, p95 = 4,000.
    fn cpu_series() -> Vec<i64> {
        let mut series = Vec::new();
        series.extend(std::iter::repeat_n(500, 40));
        series.extend(std::iter::repeat_n(1_500, 60));
        series.extend(std::iter::repeat_n(4_000, 18));
        series.extend(std::iter::repeat_n(9_000, 2));
        series
    }

    #[test]
    fn nearest_rank_takes_the_ceil_rank() {
        assert_eq!(nearest_rank(&[], 50), None);
        assert_eq!(nearest_rank(&[7], 50), Some(7));
        assert_eq!(nearest_rank(&[7], 95), Some(7));
        assert_eq!(nearest_rank(&[1, 2, 3, 4], 50), Some(2));
        assert_eq!(nearest_rank(&[1, 2, 3, 4], 95), Some(4));
        let series = cpu_series();
        assert_eq!(nearest_rank(&series, 50), Some(1_500));
        assert_eq!(nearest_rank(&series, 95), Some(4_000));
    }

    #[test]
    fn hours_floor_and_ceil_to_whole_hours() {
        assert_eq!(floor_hour(HOUR + MINUTE), HOUR);
        assert_eq!(floor_hour(2 * HOUR), 2 * HOUR);
        assert_eq!(ceil_hour(HOUR + MINUTE), 2 * HOUR);
        assert_eq!(ceil_hour(2 * HOUR), 2 * HOUR);
    }

    #[test]
    fn the_catch_up_reach_is_seven_whole_days() {
        assert_eq!(CATCH_UP_MILLIS, 7 * 24 * HOUR);
    }
    /// Item 9: the catch-up rolls an ended hour only once `hour_end + 15 min
    /// <= now`, as the job rollup does, so samples of the hour's last
    /// minutes that land after it ends still count.
    #[tokio::test]
    async fn the_catch_up_waits_15_minutes_so_late_samples_still_count() -> Result<()> {
        use gridops_core::{
            connect_database_path,
            fleet_health::testdata::{self, SampleSeed},
        };
        const NOW: i64 = 1_800_000_000_000;
        const GRACE: i64 = 15 * MINUTE;
        let directory =
            std::env::temp_dir().join(format!("gridops-host-rollup-own-{}", uuid::Uuid::new_v4()));
        let db = connect_database_path(&directory.join("gridops.sqlite")).await?;
        let fleet = testdata::seed_fleet(&db).await?;
        let io = fleet.host("io")?;
        let hour = NOW - HOUR;
        // The catch-up's next hour is the one that just ended.
        sqlx::query("INSERT INTO settings (key,value,updated_at) VALUES (?,?,?)")
            .bind(LAST_ROLLED_KEY)
            .bind((hour - HOUR).to_string())
            .bind(hour)
            .execute(&db)
            .await?;
        let every_30_s = |from: i64, count: i64| -> Vec<SampleSeed> {
            (0..count)
                .map(|index| SampleSeed::at(from + index * 30_000))
                .collect()
        };
        let online = || {
            sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(SUM(online_minutes),0) FROM fleet_host_hourly
                  WHERE hour=? AND host_id=?",
            )
            .bind(hour)
            .bind(&io.id)
            .fetch_one(&db)
        };
        let rows = || {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM fleet_host_hourly WHERE hour=?")
                .bind(hour)
                .fetch_one(&db)
        };

        // The first 50 minutes arrive on time.
        testdata::insert_samples(&db, io, &every_30_s(hour, 100)).await?;
        run_pass(&db, NOW + GRACE - 1_000).await?;
        assert_eq!(rows().await?, 0, "rolled inside the grace");

        // The last 10 minutes arrive late, after the hour ended.
        testdata::insert_samples(&db, io, &every_30_s(hour + 50 * MINUTE, 20)).await?;
        run_pass(&db, NOW + GRACE).await?;
        assert_eq!(
            online().await?,
            60,
            "rolled at hour end + 15 min with every minute"
        );

        db.close().await;
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }
}
