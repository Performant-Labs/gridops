//! Fleet targets and pool fleet settings. A pool runs on fleet hosts once it
//! has enabled platforms (one execution profile each). Only system administrators can read or change
//! these, because they decide which machines run which repositories' jobs.

use axum::{Json, extract::State};
use gridops_core::fleet::{
    FleetError, Principal,
    protocol::browser::ErrorCode,
    targets::{self, FleetSettings, FleetSettingsInput, TargetSummary},
};
use serde::{Deserialize, Serialize};

use super::{
    boundary::{FleetJson, FleetPath, FleetUser, FleetWriteUser},
    error::{FleetApiError, RequestContext},
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
    user: FleetUser,
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
    user: FleetUser,
    FleetPath(path): FleetPath<PoolPath>,
) -> Result<Json<ProfileResponse>, FleetApiError> {
    require_admin(&state, &context, user.principal()).await?;
    let item = targets::pool_fleet(&state.database, &path.pool_id)
        .await
        .map_err(|error| fleet_error(context, &error))?;
    Ok(Json(ProfileResponse { item }))
}

#[derive(Serialize)]
pub(super) struct QueueResponse {
    items: Vec<targets::QueuedRunner>,
}

pub(super) async fn queue(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetUser,
    FleetPath(path): FleetPath<PoolPath>,
) -> Result<Json<QueueResponse>, FleetApiError> {
    require_admin(&state, &context, user.principal()).await?;
    let items = targets::pool_queue(&state.database, &path.pool_id)
        .await
        .map_err(|error| fleet_error(context, &error))?;
    Ok(Json(QueueResponse { items }))
}

pub(super) async fn save_profile(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetWriteUser,
    FleetPath(path): FleetPath<PoolPath>,
    FleetJson(body): FleetJson<FleetSettingsInput>,
) -> Result<Json<ProfileResponse>, FleetApiError> {
    require_admin(&state, &context, user.principal()).await?;
    let item = targets::save_pool_fleet(&state.database, &path.pool_id, &body)
        .await
        .map_err(|error| fleet_error(context, &error))?;
    Ok(Json(ProfileResponse { item: Some(item) }))
}

async fn require_admin(
    state: &AppState,
    context: &RequestContext,
    principal: &Principal,
) -> Result<(), FleetApiError> {
    let Principal::AuthenticatedUser { user_id } = principal else {
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
