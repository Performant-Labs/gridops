//! The windowed half of the fleet health response from the database (#204):
//! the machines and their budgets, the window's hourly rows (rolled and live,
//! [`hour::window_rows`]), the scores and windowed concerns (#192), the usage
//! by category ([`usage`]) and the trend from `fleet_health_snapshots`. The
//! API's database source serves it (with #193's speed indexes, cached there),
//! and the reconciler's snapshots record its scores, so the two always agree.

use anyhow::{Context as _, Result};
use sqlx::SqlitePool;

use super::{
    FleetHealthInputs, FleetLiveStats, FleetSummary, HealthTargets, HealthWindow, HostHealth,
    HostInfo, HostSpeed, HourlyJobRow, RetiredHost, ScoredFleet, SpeedResult, TrendPoint,
    WindowedHealth, concerns, hour, score, usage,
};

/// A machine's id, name, root CPU and memory budget, observed root disk,
/// `created_at`, and `retired_at` while it is retired.
type HostRow = (
    String,
    String,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    i64,
    Option<i64>,
);

/// The window's machines in id order (#262, [`super::machines`]): every
/// approved, non-deleted machine whose life overlaps the covered time
/// `[ceil_hour(now − W), now)`. The ones not retired are the page's
/// (`HostInfo`, counting from their `created_at`); the ones retired during
/// the window are never on the page but count for the fleet's components
/// until their `retired_at` (`RetiredHost`); one retired before the window
/// is neither. Each page machine has its root budget's CPU and memory and
/// its root disk's size as its agent observed it (`fleet_observed_domains`,
/// the size the disk alarm reads since #240, not the admin-set budget disk),
/// `None` before any observation. A machine with no root budget row has a
/// budget of 0.
///
/// # Errors
/// The read failed, or a budget does not fit its type.
pub async fn window_machines(
    db: &SqlitePool,
    window: HealthWindow,
    now: i64,
) -> Result<(Vec<HostInfo>, Vec<RetiredHost>)> {
    let rows: Vec<HostRow> = sqlx::query_as(concat!(
        "SELECT fh.id,fh.name,d.cpu_millis,d.memory_mib,
                (SELECT o.disk_bytes FROM fleet_observed_domains o
                  WHERE o.host_id=fh.id AND o.local_key='root') AS disk_bytes,
                fh.created_at,
                CASE WHEN fh.lifecycle_state='retired' THEN fh.retired_at END
           FROM fleet_hosts fh
           LEFT JOIN host_resource_domains d ON d.host_id=fh.id AND d.parent_domain_id IS NULL
          WHERE ",
        crate::fleet_machine_approved!("fh"),
        " AND ",
        crate::fleet_machine_overlaps!("fh", "?1", "?2"),
        " ORDER BY fh.id"
    ))
    .bind(hour::ceil_hour(now - window.millis()))
    .bind(now)
    .fetch_all(db)
    .await?;
    let mut hosts = Vec::new();
    let mut retired_hosts = Vec::new();
    for (id, name, cpu, memory, disk, created_at, retired_at) in rows {
        let cpu_budget_millis =
            u32::try_from(cpu.unwrap_or(0)).with_context(|| format!("{name}'s CPU budget"))?;
        if let Some(retired_at) = retired_at {
            retired_hosts.push(RetiredHost {
                id,
                cpu_budget_millis,
                created_at: Some(created_at),
                retired_at,
            });
            continue;
        }
        hosts.push(HostInfo {
            cpu_budget_millis,
            memory_budget_mib: u32::try_from(memory.unwrap_or(0))
                .with_context(|| format!("{name}'s memory budget"))?,
            disk_total_bytes: disk.and_then(|bytes| u64::try_from(bytes).ok()),
            created_at: Some(created_at),
            id,
            name,
        });
    }
    Ok((hosts, retired_hosts))
}

/// The fleet score's trend for `window`: its snapshots recorded in
/// `[now − W, now]`, oldest first (`fleet_health_snapshots`' primary key
/// leads with `recorded_at`). Empty until the reconciler has recorded one;
/// past snapshots are never made up.
///
/// # Errors
/// The read failed, or a stored score is out of range.
pub async fn trend(db: &SqlitePool, window: HealthWindow, now: i64) -> Result<Vec<TrendPoint>> {
    let rows: Vec<(i64, Option<i64>)> = sqlx::query_as(
        r#"SELECT recorded_at,fleet_score FROM fleet_health_snapshots
            WHERE recorded_at>=? AND recorded_at<=? AND "window"=? ORDER BY recorded_at"#,
    )
    .bind(now - window.millis())
    .bind(now)
    .bind(window.as_str())
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(at, score)| {
            Ok(TrendPoint {
                at,
                score: score.map(u8::try_from).transpose()?,
            })
        })
        .collect()
}

/// What the scores of `window` at `now` are computed from: the machines,
/// the window's rows and `speed`, with the default targets. The live fields
/// stay empty: no score reads them.
///
/// # Errors
/// A read failed.
pub async fn inputs(
    db: &SqlitePool,
    window: HealthWindow,
    now: i64,
    speed: Vec<SpeedResult>,
) -> Result<FleetHealthInputs> {
    let (hosts, retired_hosts) = window_machines(db, window, now).await?;
    let (hourly, host_hourly) = hour::window_rows(db, window, now).await?;
    Ok(FleetHealthInputs {
        window,
        now,
        hosts,
        retired_hosts,
        hourly,
        host_hourly,
        speed,
        live: FleetLiveStats::default(),
        host_live: Vec::new(),
        targets: HealthTargets::default(),
    })
}

/// The windowed half of `window` at `now`, with `speed` (#193's indexes for
/// the window, which the caller computes or caches).
///
/// # Errors
/// A read failed.
pub async fn windowed_health(
    db: &SqlitePool,
    window: HealthWindow,
    now: i64,
    speed: Vec<SpeedResult>,
) -> Result<WindowedHealth> {
    let inputs = inputs(db, window, now, speed).await?;
    let trend = trend(db, window, now).await?;
    let scored = score::score_fleet(&inputs);
    Ok(assemble(&inputs, &scored, trend))
}

/// The windowed half from its scored inputs (pure): the fleet's score,
/// components, trend and usage; each machine's budgets, score, components,
/// speed (by runner size), usage and hourly usage, in id order; and the
/// windowed concerns.
#[must_use]
pub fn assemble(
    inputs: &FleetHealthInputs,
    scored: &ScoredFleet,
    trend: Vec<TrendPoint>,
) -> WindowedHealth {
    let start = inputs.now - inputs.window.millis();
    let rows: Vec<&HourlyJobRow> = inputs
        .hourly
        .iter()
        .filter(|row| row.hour >= start && row.hour < inputs.now)
        .collect();
    let hosts = inputs
        .hosts
        .iter()
        .map(|info| {
            let scored_host = scored.hosts.iter().find(|host| host.host_id == info.id);
            let own: Vec<&HourlyJobRow> = rows
                .iter()
                .copied()
                .filter(|row| row.host.as_str() == info.id && info.life().counts_hour(row.hour))
                .collect();
            let mut speed: Vec<HostSpeed> = inputs
                .speed
                .iter()
                .filter(|result| result.host_id == info.id)
                .cloned()
                .map(HostSpeed::from)
                .collect();
            speed.sort_by_key(|entry| entry.cpu_millis);
            HostHealth {
                id: info.id.clone(),
                name: info.name.clone(),
                cpu_budget_millis: info.cpu_budget_millis,
                memory_budget_mib: info.memory_budget_mib,
                disk_total_bytes: info.disk_total_bytes,
                score: scored_host.and_then(|host| host.score),
                components: scored_host
                    .map(|host| host.components.clone())
                    .unwrap_or_default(),
                speed,
                categories: usage::categories(own.iter().copied()),
                usage: usage::hourly(own.iter().copied()),
                live: None,
            }
        })
        .collect();
    WindowedHealth {
        fleet: FleetSummary {
            score: scored.score,
            components: scored.components.clone(),
            trend,
            categories: usage::categories(rows.iter().copied()),
        },
        hosts,
        concerns: concerns::windowed(inputs, scored),
    }
}
