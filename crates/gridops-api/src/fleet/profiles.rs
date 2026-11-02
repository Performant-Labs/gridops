//! Fleet targets and pool fleet settings. A pool runs on fleet hosts once it
//! has enabled platforms (one execution profile each). Only system administrators can read or change
//! these, because they decide which machines run which repositories' jobs.

use axum::{Json, extract::State};
use gridops_core::fleet::{
    FleetError, Principal,
    protocol::browser::ErrorCode,
    targets::{self, FleetSettings, FleetSettingsInput, TargetSummary},
    workload::{self, FleetWorkload},
};
use serde::{Deserialize, Serialize};

use super::{
    boundary::{FleetJson, FleetPath, FleetUser, FleetWriteUser, Operate, Read},
    error::{FleetApiError, RequestContext},
    query_boundary::FleetQuery,
};
use crate::state::AppState;

#[derive(Serialize)]
pub(super) struct Targets {
    items: Vec<TargetSummary>,
}

#[derive(Serialize)]
pub(super) struct ProfileResponse {
    item: Option<FleetSettings>,
}

#[derive(Deserialize)]
pub(super) struct PoolPath {
    pool_id: String,
}

pub(super) async fn targets(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetUser<Read>,
) -> Result<Json<Targets>, FleetApiError> {
    require_admin(&state, &context, user.principal()).await?;
    let items = targets::list_targets(&state.database)
        .await
        .map_err(|error| fleet_error(context, &error))?;
    Ok(Json(Targets { items }))
}

pub(super) async fn profile(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetUser<Read>,
    FleetPath(path): FleetPath<PoolPath>,
) -> Result<Json<ProfileResponse>, FleetApiError> {
    require_admin(&state, &context, user.principal()).await?;
    let item = targets::pool_fleet(&state.database, &path.pool_id)
        .await
        .map_err(|error| fleet_error(context, &error))?;
    Ok(Json(ProfileResponse { item }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct QueueResponse {
    items: Vec<targets::QueuedRunner>,
    blocked_hosts: Vec<targets::BlockedHost>,
}

pub(super) async fn queue(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetUser<Read>,
    FleetPath(path): FleetPath<PoolPath>,
) -> Result<Json<QueueResponse>, FleetApiError> {
    require_admin(&state, &context, user.principal()).await?;
    let items = targets::pool_queue(&state.database, &path.pool_id)
        .await
        .map_err(|error| fleet_error(context, &error))?;
    let blocked_hosts = targets::pool_blocked_hosts(&state.database, &path.pool_id)
        .await
        .map_err(|error| fleet_error(context, &error))?;
    Ok(Json(QueueResponse {
        items,
        blocked_hosts,
    }))
}

fn invalid_field(context: RequestContext, field: &'static str) -> FleetApiError {
    match gridops_core::fleet::UpstreamText::new(field) {
        Ok(field) => context.error_with_details(
            ErrorCode::InvalidRequest,
            gridops_core::fleet::protocol::browser::ErrorDetails::InvalidField { field },
        ),
        Err(_) => context.error(ErrorCode::InvalidRequest),
    }
}

pub(super) async fn save_profile(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetWriteUser<Operate>,
    FleetPath(path): FleetPath<PoolPath>,
    FleetJson(body): FleetJson<FleetSettingsInput>,
) -> Result<Json<ProfileResponse>, FleetApiError> {
    require_admin(&state, &context, user.principal()).await?;
    // With the built-in runner host off, fleet machines are the only place a
    // pool can run: it must run on them, and cannot be left to the host.
    if !gridops_core::builtin_host::is_enabled(&state.database).await {
        if body.placement != targets::Placement::Machines {
            return Err(invalid_field(context, "placement"));
        }
        if !body.enabled {
            return Err(invalid_field(context, "enabled"));
        }
    }
    let item = targets::save_pool_fleet(&state.database, &path.pool_id, &body)
        .await
        .map_err(|error| fleet_error(context, &error))?;
    record_profile_saved(&state, context, user.principal(), &path.pool_id, &body).await?;
    Ok(Json(ProfileResponse { item: Some(item) }))
}

/// The runner counts a pool may scale between.
#[derive(Serialize)]
pub(super) struct Capacity {
    #[serde(rename = "minCount")]
    min: i64,
    #[serde(rename = "desiredCount")]
    desired: i64,
    #[serde(rename = "maxCount")]
    max: i64,
}

/// The most runners one pool can be set to run at once.
const MAX_POOL_RUNNERS: i64 = 256;

pub(super) async fn capacity(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetUser<Read>,
    FleetPath(path): FleetPath<PoolPath>,
) -> Result<Json<Capacity>, FleetApiError> {
    require_admin(&state, &context, user.principal()).await?;
    read_capacity(&state, &context, &path.pool_id)
        .await?
        .map(Json)
        .ok_or_else(|| context.error(ErrorCode::NotFound))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapacityInput {
    #[serde(rename = "minCount")]
    min: i64,
    #[serde(rename = "desiredCount")]
    desired: i64,
    #[serde(rename = "maxCount")]
    max: i64,
}

/// Set a pool's runner counts. Runners above the new count are not stopped:
/// ephemeral runners finish their job and are not replaced, so the pool
/// drains down to the new count on its own.
pub(super) async fn save_capacity(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetWriteUser<Operate>,
    FleetPath(path): FleetPath<PoolPath>,
    FleetJson(body): FleetJson<CapacityInput>,
) -> Result<Json<Capacity>, FleetApiError> {
    require_admin(&state, &context, user.principal()).await?;
    let database_down = |_| context.error(ErrorCode::DependencyUnavailable);
    if !(0 <= body.min
        && body.min <= body.desired
        && body.desired <= body.max
        && body.max <= MAX_POOL_RUNNERS)
    {
        return Err(context.error(ErrorCode::InvalidRequest));
    }
    let Some(before) = read_capacity(&state, &context, &path.pool_id).await? else {
        return Err(context.error(ErrorCode::NotFound));
    };
    // A repository pool keeps a runner for each of its repositories, as the
    // browser route requires.
    let repositories: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM runner_pool_repositories WHERE pool_id=?")
            .bind(&path.pool_id)
            .fetch_one(&state.database)
            .await
            .map_err(database_down)?;
    let scope: String = sqlx::query_scalar("SELECT scope FROM runner_pools WHERE id=?")
        .bind(&path.pool_id)
        .fetch_one(&state.database)
        .await
        .map_err(database_down)?;
    if scope == "repository" && repositories > body.max {
        return Err(context.error(ErrorCode::InvalidRequest));
    }
    sqlx::query(
        "UPDATE runner_pools SET min_count=?,desired_count=?,max_count=?,updated_at=? WHERE id=?",
    )
    .bind(body.min)
    .bind(body.desired)
    .bind(body.max)
    .bind(gridops_core::now_millis())
    .bind(&path.pool_id)
    .execute(&state.database)
    .await
    .map_err(database_down)?;
    record_capacity_saved(
        &state,
        context,
        user.principal(),
        &path.pool_id,
        &before,
        &body,
    )
    .await?;
    Ok(Json(Capacity {
        min: body.min,
        desired: body.desired,
        max: body.max,
    }))
}

async fn read_capacity(
    state: &AppState,
    context: &RequestContext,
    pool_id: &str,
) -> Result<Option<Capacity>, FleetApiError> {
    let row: Option<(i64, i64, i64)> =
        sqlx::query_as("SELECT min_count,desired_count,max_count FROM runner_pools WHERE id=?")
            .bind(pool_id)
            .fetch_optional(&state.database)
            .await
            .map_err(|_| context.error(ErrorCode::DependencyUnavailable))?;
    Ok(row.map(|(min, desired, max)| Capacity { min, desired, max }))
}

async fn record_capacity_saved(
    state: &AppState,
    context: RequestContext,
    principal: &Principal,
    pool_id: &str,
    before: &Capacity,
    body: &CapacityInput,
) -> Result<(), FleetApiError> {
    let Some(user_id) = principal.user_id() else {
        return Ok(());
    };
    let login: Option<String> = sqlx::query_scalar("SELECT login FROM users WHERE id=?")
        .bind(user_id)
        .fetch_optional(&state.database)
        .await
        .map_err(|_| context.error(ErrorCode::DependencyUnavailable))?;
    sqlx::query(
        r#"INSERT INTO audit_events (id,actor_user_id,actor_label,action,target_type,target_id,metadata,created_at)
           VALUES (?,?,?,'fleet.pool.capacity_saved','runner_pool',?,?,?)"#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(login.unwrap_or_else(|| user_id.to_owned()))
    .bind(pool_id)
    .bind(
        serde_json::json!({
            "from": {"min": before.min, "desired": before.desired, "max": before.max},
            "to": {"min": body.min, "desired": body.desired, "max": body.max},
        })
        .to_string(),
    )
    .bind(gridops_core::now_millis())
    .execute(&state.database)
    .await
    .map_err(|_| context.error(ErrorCode::DependencyUnavailable))?;
    Ok(())
}

/// Every other admin change to pool/runner/installation state is logged in
/// `audit_events` (pool settings, host enrollment/approval, GitHub app events);
/// a fleet execution profile save is exactly the kind of change you'd most
/// want in that trail, so it must be too.
async fn record_profile_saved(
    state: &AppState,
    context: RequestContext,
    principal: &Principal,
    pool_id: &str,
    body: &FleetSettingsInput,
) -> Result<(), FleetApiError> {
    let Some(user_id) = principal.user_id() else {
        return Ok(());
    };
    let login: Option<String> = sqlx::query_scalar("SELECT login FROM users WHERE id=?")
        .bind(user_id)
        .fetch_optional(&state.database)
        .await
        .map_err(|_| context.error(ErrorCode::DependencyUnavailable))?;
    sqlx::query(
        r#"INSERT INTO audit_events (id,actor_user_id,actor_label,action,target_type,target_id,metadata,created_at)
           VALUES (?,?,?,'fleet.pool.profile_saved','runner_pool',?,?,?)"#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(login.unwrap_or_else(|| user_id.to_owned()))
    .bind(pool_id)
    .bind(
        serde_json::json!({
            "enabled": body.enabled,
            "placement": body.placement,
            "mode": body.mode,
            "platformCount": body.platforms.len(),
        })
        .to_string(),
    )
    .bind(gridops_core::now_millis())
    .execute(&state.database)
    .await
    .map_err(|_| context.error(ErrorCode::DependencyUnavailable))?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkloadParams {
    window: Option<String>,
}

/// Each machine's runners, CPU load and memory over the last day, week or
/// month, for the overview.
pub(super) async fn workload(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetUser<Read>,
    FleetQuery(params): FleetQuery<WorkloadParams>,
) -> Result<Json<FleetWorkload>, FleetApiError> {
    require_admin(&state, &context, user.principal()).await?;
    let (window, bucket) = match params.window.as_deref().unwrap_or("24h") {
        "24h" => (86_400_000, 10 * 60_000),
        "7d" => (7 * 86_400_000, 60 * 60_000),
        "30d" => (30 * 86_400_000, 6 * 60 * 60_000),
        _ => return Err(context.error(ErrorCode::InvalidRequest)),
    };
    workload::fleet_workload(&state.database, gridops_core::now_millis(), window, bucket)
        .await
        .map(Json)
        .map_err(|error| fleet_error(context, &error))
}

async fn require_admin(
    state: &AppState,
    context: &RequestContext,
    principal: &Principal,
) -> Result<(), FleetApiError> {
    let Some(user_id) = principal.user_id() else {
        return Err(context.error(ErrorCode::Forbidden));
    };
    let role: Option<String> = sqlx::query_scalar("SELECT role FROM users WHERE id=?")
        .bind(user_id)
        .fetch_optional(&state.database)
        .await
        .map_err(|_| context.error(ErrorCode::DependencyUnavailable))?;
    if role.as_deref() == Some("admin") {
        Ok(())
    } else {
        Err(context.error(ErrorCode::Forbidden))
    }
}

fn fleet_error(context: RequestContext, error: &FleetError) -> FleetApiError {
    context.error(match error {
        FleetError::NotFound => ErrorCode::NotFound,
        FleetError::Forbidden => ErrorCode::Forbidden,
        FleetError::InvalidRequest(_) | FleetError::InvalidIdentifier(_) => {
            ErrorCode::InvalidRequest
        }
        FleetError::IdempotencyConflict | FleetError::StaleProfileRevision => {
            ErrorCode::RevisionConflict
        }
        _ => {
            tracing::error!(request_id = %context.id(), "fleet profile dependency failed");
            ErrorCode::DependencyUnavailable
        }
    })
}
