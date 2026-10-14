//! Agent command delivery. The agent long-polls for commands bound to its
//! current session and posts each result back; both run under host
//! credentials only.

use std::time::Duration;

use axum::{Json, extract::State};
use gridops_core::fleet::{
    ids::AuthoritySessionId,
    protocol::{
        agent::AgentCommandEnvelope, outcomes::AgentCommandResult, primitives::BoundedInteger,
    },
    registry::{RegistryService, ResultDisposition},
};
use serde::{Deserialize, Serialize};

use super::{
    auth::FleetAgent,
    boundary::FleetJson,
    error::{FleetApiError, RequestContext},
};
use crate::state::AppState;

const POLL_STEP: Duration = Duration::from_millis(500);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PollBody {
    authority_session_id: AuthoritySessionId,
    wait_ms: BoundedInteger<0, 25_000>,
    limit: BoundedInteger<1, 16>,
}

#[derive(Serialize)]
pub(super) struct PollResponse {
    commands: Vec<AgentCommandEnvelope>,
}

pub(super) async fn poll(
    State(state): State<AppState>,
    context: RequestContext,
    agent: FleetAgent,
    FleetJson(body): FleetJson<PollBody>,
) -> Result<Json<PollResponse>, FleetApiError> {
    let service = RegistryService::new(state.database, state.vault);
    let limit = u32::try_from(body.limit.get()).unwrap_or(16);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(body.wait_ms.get());
    loop {
        let commands = service
            .poll_commands(agent.authenticated(), body.authority_session_id, limit)
            .await
            .map_err(|error| context.registry_error(&error))?;
        if !commands.is_empty() || tokio::time::Instant::now() + POLL_STEP > deadline {
            return Ok(Json(PollResponse { commands }));
        }
        tokio::time::sleep(POLL_STEP).await;
    }
}

#[derive(Serialize)]
pub(super) struct ResultReceipt {
    disposition: &'static str,
}

pub(super) async fn result(
    State(state): State<AppState>,
    context: RequestContext,
    agent: FleetAgent,
    FleetJson(body): FleetJson<AgentCommandResult>,
) -> Result<Json<ResultReceipt>, FleetApiError> {
    let disposition = RegistryService::new(state.database, state.vault)
        .record_command_result(agent.authenticated(), &body)
        .await
        .map_err(|error| context.registry_error(&error))?;
    Ok(Json(ResultReceipt {
        disposition: match disposition {
            ResultDisposition::Applied => "applied",
            ResultDisposition::Stale => "stale",
        },
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BootstrapBody {
    reference: gridops_core::fleet::ids::BootstrapReference,
}

#[derive(Serialize)]
pub(super) struct BootstrapMaterial {
    material: String,
}

/// Registration material for a start command. The response is never cached
/// and the material is never logged.
pub(super) async fn bootstrap(
    State(state): State<AppState>,
    context: RequestContext,
    agent: FleetAgent,
    FleetJson(body): FleetJson<BootstrapBody>,
) -> Result<axum::response::Response, FleetApiError> {
    use axum::response::IntoResponse as _;
    let material = RegistryService::new(state.database, state.vault)
        .claim_bootstrap(agent.authenticated(), body.reference)
        .await
        .map_err(|error| context.registry_error(&error))?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(BootstrapMaterial { material }),
    )
        .into_response())
}

/// Log lines from the host's agent.
pub(super) async fn logs(
    State(state): State<AppState>,
    context: RequestContext,
    agent: FleetAgent,
    FleetJson(body): FleetJson<gridops_core::fleet::registry::logs::AgentLogBatch>,
) -> Result<axum::http::StatusCode, FleetApiError> {
    RegistryService::new(state.database, state.vault)
        .store_agent_logs(agent.authenticated(), body)
        .await
        .map_err(|error| context.registry_error(&error))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
