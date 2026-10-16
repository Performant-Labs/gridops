//! How busy each fleet machine has been: runners placed on it, CPU load and
//! memory in use, averaged over fixed buckets for charting.

use serde::Serialize;
use sqlx::{Row as _, SqlitePool};

use super::FleetError;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetWorkload {
    pub machines: Vec<WorkloadMachine>,
    pub points: Vec<WorkloadPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadMachine {
    pub id: String,
    pub name: String,
    pub cpu_millis: Option<i64>,
    pub memory_mib: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadPoint {
    pub recorded_at: i64,
    /// One entry per machine, in the order of `machines`.
    pub machines: Vec<MachineLoad>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineLoad {
    /// Runners on the machine, averaged over the bucket. The latest point,
    /// still in progress, has the runners on the machine now.
    pub runners: f64,
    /// Load as a percentage of the machine's CPU capacity; above 100 when
    /// work is waiting for a CPU.
    pub cpu_percent: Option<f64>,
    pub memory_percent: Option<f64>,
}

/// The workload of every machine that isn't retired over `[now - window, now)`,
/// in buckets of `bucket` milliseconds.
pub async fn fleet_workload(
    pool: &SqlitePool,
    now: i64,
    window: i64,
    bucket: i64,
) -> Result<FleetWorkload, FleetError> {
    if window <= 0 || bucket <= 0 {
        return Err(FleetError::InvalidRequest("workload window"));
    }
    let first = (now - window).div_euclid(bucket) * bucket;
    let count = usize::try_from((now - first).div_euclid(bucket) + 1).unwrap_or(0);
    let machines = sqlx::query(
        "SELECT h.id,h.name,d.cpu_millis,d.memory_mib FROM fleet_hosts h
           LEFT JOIN host_resource_domains d ON d.host_id=h.id AND d.parent_domain_id IS NULL
          WHERE h.retired_at IS NULL AND h.enrollment_state='approved'
          ORDER BY h.name,h.id",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| WorkloadMachine {
        id: row.get("id"),
        name: row.get("name"),
        cpu_millis: row.get("cpu_millis"),
        memory_mib: row.get("memory_mib"),
    })
    .collect::<Vec<_>>();
    let mut points = (0..count)
        .map(|index| WorkloadPoint {
            recorded_at: first + bucket * i64::try_from(index).unwrap_or(0),
            machines: vec![
                MachineLoad {
                    runners: 0.0,
                    cpu_percent: None,
                    memory_percent: None,
                };
                machines.len()
            ],
        })
        .collect::<Vec<_>>();
    #[allow(clippy::cast_precision_loss)]
    let span = bucket as f64;
    for (slot, machine) in machines.iter().enumerate() {
        // A placement holds its machine from admission until it stops.
        let placements = sqlx::query(
            "SELECT created_at,
                    CASE WHEN state IN ('stopped','cleaned','failed') THEN updated_at END AS ended_at
               FROM workload_placements
              WHERE host_id=?1 AND created_at<?2
                AND (state NOT IN ('stopped','cleaned','failed') OR updated_at>=?3)",
        )
        .bind(&machine.id)
        .bind(now)
        .bind(first)
        .fetch_all(pool)
        .await?;
        let mut current = 0.0;
        for placement in placements {
            let start = placement.get::<i64, _>("created_at").max(first);
            let ended_at = placement.get::<Option<i64>, _>("ended_at");
            if ended_at.is_none() {
                current += 1.0;
            }
            let end = ended_at.unwrap_or(now).min(now);
            let mut at = start;
            while at < end {
                let index = usize::try_from((at - first).div_euclid(bucket)).unwrap_or(0);
                let bucket_end = first + bucket * (i64::try_from(index).unwrap_or(0) + 1);
                let until = end.min(bucket_end);
                if let Some(point) = points.get_mut(index) {
                    #[allow(clippy::cast_precision_loss)]
                    let share = (until - at) as f64 / span;
                    point.machines[slot].runners += share;
                }
                at = until;
            }
        }
        if let Some(point) = points.last_mut() {
            point.machines[slot].runners = current;
        }
        let samples = sqlx::query(
            "SELECT (received_at/?2)*?2 AS bucket,AVG(cpu_used_millis) AS cpu,
                    AVG(memory_used_mib) AS memory
               FROM host_samples WHERE host_id=?1 AND received_at>=?3 AND received_at<=?4
              GROUP BY bucket",
        )
        .bind(&machine.id)
        .bind(bucket)
        .bind(first)
        .bind(now)
        .fetch_all(pool)
        .await?;
        for sample in samples {
            let index =
                usize::try_from((sample.get::<i64, _>("bucket") - first).div_euclid(bucket))
                    .unwrap_or(usize::MAX);
            let Some(point) = points.get_mut(index) else {
                continue;
            };
            let share = |used: Option<f64>, capacity: Option<i64>| {
                #[allow(clippy::cast_precision_loss)]
                let capacity = capacity.filter(|value| *value > 0)? as f64;
                Some(round(used? / capacity * 100.0))
            };
            let load = &mut point.machines[slot];
            load.cpu_percent = share(sample.get("cpu"), machine.cpu_millis);
            load.memory_percent = share(sample.get("memory"), machine.memory_mib);
        }
    }
    for point in &mut points {
        for load in &mut point.machines {
            load.runners = round(load.runners);
        }
    }
    Ok(FleetWorkload { machines, points })
}

fn round(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}
