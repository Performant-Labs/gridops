//! The Tart host card (#95 phase 2): the Mac's macOS VM slots, who holds
//! each, and the pools waiting for one, in order.
//!
//! The runner manager arbitrates the slots and reports them in its health
//! (`providers.tart.vmSlots`): the limit and the count in use, the runner ids
//! of `GridOps` VMs that are running (`runnerIds`), the starts in flight with
//! their runner and pool (`startingVms`), the VMs other software runs, and the
//! queue by pool id. This adds the names `GridOps` holds (pool, runner, when
//! the runner was started), and only those the caller may see: a pool of an
//! installation the caller does not belong to is reported as hidden, without
//! its id or name or its runner's.

use std::collections::HashMap;

use axum::{Json, extract::State, http::Method};
use serde_json::{Value, json};
use sqlx::Row as _;

use super::{iso, manager_json};
use crate::{
    access::{OptionalPoolUser, PoolsRead},
    auth::AuthUser,
    error::ApiResult,
    state::AppState,
};

/// A pool as the caller may know it.
struct PoolName {
    name: String,
    visible: bool,
}

/// A runner `GridOps` has a record of.
struct RunnerRecord {
    pool_id: String,
    name: String,
    created_at: i64,
}

pub async fn tart_host(
    State(state): State<AppState>,
    OptionalPoolUser(user, ..): OptionalPoolUser<PoolsRead>,
) -> ApiResult<Json<Value>> {
    let Some(user) = user else {
        return Ok(Json(json!({
            "authenticated": false, "tartAgent": "unknown", "message": null,
            "slotSetting": null, "slots": null,
        })));
    };
    let slot_setting = gridops_core::tart_host::vm_slots(&state.database).await;
    let health = match manager_json(&state, Method::GET, "v1/health", None).await {
        Ok(health) => health,
        // Without the manager there is no telling whether a Tart agent is
        // configured, let alone what it holds.
        Err(error) => {
            return Ok(Json(json!({
                "tartAgent": "unknown", "message": error.to_string(),
                "slotSetting": slot_setting, "slots": null,
            })));
        }
    };
    let tart = health.pointer("/providers/tart").unwrap_or(&Value::Null);
    if tart.get("available").and_then(Value::as_bool) != Some(true) {
        // The manager says `configured: false` when it has no Tart agent; a
        // manager that says nothing about Tart has none either.
        let configured =
            !tart.is_null() && tart.get("configured").and_then(Value::as_bool) != Some(false);
        return Ok(Json(if configured {
            json!({
                "tartAgent": "unavailable",
                "message": tart.get("error").and_then(Value::as_str).unwrap_or("The Tart agent is unavailable."),
                "slotSetting": slot_setting, "slots": null,
            })
        } else {
            json!({
                "tartAgent": "not_configured", "message": null,
                "slotSetting": slot_setting, "slots": null,
            })
        }));
    }
    let slots = match tart.get("vmSlots").filter(|slots| slots.is_object()) {
        Some(report) => Some(slots(&state, &user, report).await?),
        None => None,
    };
    Ok(Json(json!({
        "tartAgent": "available", "message": null,
        "slotSetting": slot_setting, "slots": slots,
    })))
}

async fn slots(state: &AppState, user: &AuthUser, report: &Value) -> ApiResult<Value> {
    let count = |key: &str| {
        report
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0)
    };
    let strings = |value: Option<&Value>, key: Option<&str>| -> Vec<String> {
        value
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| key.map_or(Some(item), |key| item.get(key)))
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect()
    };
    let running = strings(report.get("runnerIds"), None);
    let starting = report
        .get("startingVms")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|start| {
            (
                start
                    .get("runnerId")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                start
                    .get("poolId")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            )
        })
        .collect::<Vec<_>>();
    let queue = report
        .get("queue")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let runner_ids = running
        .iter()
        .cloned()
        .chain(
            starting
                .iter()
                .filter_map(|(runner_id, _)| runner_id.clone()),
        )
        .collect::<Vec<_>>();
    let runners = runner_records(state, &runner_ids).await?;
    let pool_ids = runners
        .values()
        .map(|runner| runner.pool_id.clone())
        .chain(starting.iter().filter_map(|(_, pool_id)| pool_id.clone()))
        .chain(strings(Some(&Value::Array(queue.clone())), Some("poolId")))
        .collect::<Vec<_>>();
    let pools = pool_names(state, user, &pool_ids).await?;

    let mut holders = Vec::new();
    // GridOps runners whose VMs are running, oldest first. An agent that
    // does not name them, or names fewer than it counts, leaves the rest
    // unidentified.
    let mut named = running
        .iter()
        .map(|runner_id| runner_holder("runner", Some(runner_id), None, &runners, &pools))
        .collect::<Vec<_>>();
    named.sort_by_key(|holder| {
        let since = holder["since"].as_str().map(ToOwned::to_owned);
        (since.is_none(), since)
    });
    holders.extend(named);
    for _ in running.len()..count("gridops") {
        holders.push(runner_holder("runner", None, None, &runners, &pools));
    }
    for external in report
        .get("external")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        holders.push(json!({
            "kind": "external",
            "name": external.get("name"),
            "user": external.get("user"),
            "source": external.get("source"),
        }));
    }
    // Starts in flight, after what already runs.
    for (runner_id, pool_id) in &starting {
        holders.push(runner_holder(
            "starting",
            runner_id.as_deref(),
            pool_id.as_deref(),
            &runners,
            &pools,
        ));
    }
    for _ in starting.len()..count("starting") {
        holders.push(runner_holder("starting", None, None, &runners, &pools));
    }

    let queue = queue
        .iter()
        .map(|entry| {
            let pool = pool_reference(entry.get("poolId").and_then(Value::as_str), &pools);
            json!({
                "position": entry.get("position"),
                "poolId": pool.id,
                "poolName": pool.name,
                "poolHidden": pool.hidden,
                "waitingSince": entry.get("waitingSince"),
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "limit": report.get("limit"),
        "inUse": report.get("inUse"),
        "free": report.get("free"),
        "detection": report.get("detection"),
        "agentMaxRunners": report.get("agentMaxRunners"),
        "warning": report.get("warning"),
        "holders": holders,
        "queue": queue,
    }))
}

/// A pool as the answer names it: its id and name only if the caller may see
/// it. A pool `GridOps` does not have (deleted, or never known) is neither
/// named nor hidden.
struct PoolReference {
    id: Option<String>,
    name: Option<String>,
    hidden: bool,
}

fn pool_reference(pool_id: Option<&str>, pools: &HashMap<String, PoolName>) -> PoolReference {
    match pool_id.and_then(|id| pools.get(id).map(|pool| (id, pool))) {
        Some((id, pool)) if pool.visible => PoolReference {
            id: Some(id.to_owned()),
            name: Some(pool.name.clone()),
            hidden: false,
        },
        Some(_) => PoolReference {
            id: None,
            name: None,
            hidden: true,
        },
        None => PoolReference {
            id: None,
            name: None,
            hidden: false,
        },
    }
}

/// A slot held by a `GridOps` runner: `runner` once its VM runs, `starting`
/// while it is being started. The runner is named when `GridOps` has its
/// record and the caller may see its pool; a start whose runner has no
/// record yet still names the pool the manager gave for it.
fn runner_holder(
    kind: &str,
    runner_id: Option<&str>,
    lease_pool_id: Option<&str>,
    runners: &HashMap<String, RunnerRecord>,
    pools: &HashMap<String, PoolName>,
) -> Value {
    let runner = runner_id.and_then(|id| runners.get(id).map(|runner| (id, runner)));
    let pool = pool_reference(
        runner
            .map(|(_, runner)| runner.pool_id.as_str())
            .or(lease_pool_id),
        pools,
    );
    let shown = runner.filter(|_| !pool.hidden);
    json!({
        "kind": kind,
        "runnerId": shown.map(|(id, _)| id),
        "runnerName": shown.map(|(_, runner)| runner.name.as_str()),
        "poolId": pool.id,
        "poolName": pool.name,
        "poolHidden": pool.hidden,
        // When GridOps began starting the runner: the slot is held from its
        // VM's boot, seconds later.
        "since": runner.map(|(_, runner)| iso(runner.created_at)),
    })
}

async fn runner_records(
    state: &AppState,
    runner_ids: &[String],
) -> ApiResult<HashMap<String, RunnerRecord>> {
    if runner_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        "SELECT id,pool_id,name,created_at FROM runners WHERE id IN (SELECT value FROM json_each(?))",
    )
    .bind(serde_json::to_string(runner_ids).unwrap_or_else(|_| "[]".into()))
    .fetch_all(&state.database)
    .await?;
    Ok(rows
        .iter()
        .map(|row| {
            (
                row.get::<String, _>("id"),
                RunnerRecord {
                    pool_id: row.get("pool_id"),
                    name: row.get("name"),
                    created_at: row.get("created_at"),
                },
            )
        })
        .collect())
}

/// The pools among `pool_ids` that exist, and whether `user` may see each:
/// the rule every pool read uses (all of the pool's installations are the
/// user's).
async fn pool_names(
    state: &AppState,
    user: &AuthUser,
    pool_ids: &[String],
) -> ApiResult<HashMap<String, PoolName>> {
    if pool_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        r#"SELECT p.id,p.name,
          CASE WHEN EXISTS (SELECT 1 FROM runner_pool_installations mapped WHERE mapped.pool_id=p.id)
            AND NOT EXISTS (
              SELECT 1 FROM runner_pool_installations mapped WHERE mapped.pool_id=p.id
                AND NOT EXISTS (SELECT 1 FROM user_installations access
                  WHERE access.user_id=? AND access.installation_id=mapped.installation_id)
            ) THEN 1 ELSE 0 END AS visible
        FROM runner_pools p WHERE p.id IN (SELECT value FROM json_each(?))"#,
    )
    .bind(&user.id)
    .bind(serde_json::to_string(pool_ids).unwrap_or_else(|_| "[]".into()))
    .fetch_all(&state.database)
    .await?;
    Ok(rows
        .iter()
        .map(|row| {
            (
                row.get::<String, _>("id"),
                PoolName {
                    name: row.get("name"),
                    visible: row.get::<i64, _>("visible") == 1,
                },
            )
        })
        .collect())
}
