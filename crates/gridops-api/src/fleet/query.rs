//! Cookie-authenticated fleet reads expose only the core's scoped projections.
//! GET handlers never grant authority, refresh observations, or mutate fleet policy.

use axum::{Json, extract::State};
use gridops_core::fleet::{
    ids::{BackendId, CiTargetId, HostId, ResourceDomainId},
    protocol::{
        browser::{ErrorCode, OpaqueCursor},
        primitives::WireTimestamp,
        query::{
            BackendReadV1, CatalogQuery, DomainReadV1, EventQuery, HostEventReadV1, HostReadV1,
            QueryPage, QueryResult, SampleQuery, SampleReadV1,
        },
    },
    registry::{QueryError, RegistryService},
};
use serde::Deserialize;

use super::{
    boundary::{FleetPath, FleetReadUser},
    error::{FleetApiError, RequestContext},
    query_boundary::FleetQuery,
};
use crate::state::AppState;

/// The transport refines a supplied page size before the service can use it.
#[derive(Clone, Copy, Deserialize)]
#[serde(try_from = "u64")]
struct PageLimit(u8);

impl Default for PageLimit {
    fn default() -> Self {
        Self(25)
    }
}

impl TryFrom<u64> for PageLimit {
    type Error = &'static str;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if (1..=100).contains(&value) {
            Ok(Self(u8::try_from(value).map_err(|_| "invalid page size")?))
        } else {
            Err("invalid page size")
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CatalogParams {
    #[serde(default)]
    limit: PageLimit,
    cursor: Option<OpaqueCursor>,
    target_id: Option<CiTargetId>,
}

impl From<CatalogParams> for CatalogQuery {
    fn from(value: CatalogParams) -> Self {
        Self {
            limit: usize::from(value.limit.0),
            cursor: value.cursor,
            target_id: value.target_id,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SampleParams {
    #[serde(default)]
    limit: PageLimit,
    cursor: Option<OpaqueCursor>,
    domain_id: Option<ResourceDomainId>,
    received_from: WireTimestamp,
    received_to: WireTimestamp,
}

impl From<SampleParams> for SampleQuery {
    fn from(value: SampleParams) -> Self {
        Self {
            limit: usize::from(value.limit.0),
            cursor: value.cursor,
            domain_id: value.domain_id,
            received_from: value.received_from,
            received_to: value.received_to,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct EventParams {
    #[serde(default)]
    limit: PageLimit,
    cursor: Option<OpaqueCursor>,
    target_id: Option<CiTargetId>,
    occurred_from: Option<WireTimestamp>,
    occurred_to: Option<WireTimestamp>,
}

impl From<EventParams> for EventQuery {
    fn from(value: EventParams) -> Self {
        Self {
            limit: usize::from(value.limit.0),
            cursor: value.cursor,
            target_id: value.target_id,
            occurred_from: value.occurred_from,
            occurred_to: value.occurred_to,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NoParams {}

fn registry(state: &AppState) -> RegistryService {
    RegistryService::new(state.database.clone(), state.vault.clone())
}

fn query_error(context: RequestContext, error: &QueryError) -> FleetApiError {
    context.error(match error {
        QueryError::Forbidden => ErrorCode::Forbidden,
        QueryError::NotFound => ErrorCode::NotFound,
        QueryError::InvalidRequest(_) | QueryError::InvalidCursor => ErrorCode::InvalidRequest,
        QueryError::CursorExpired => ErrorCode::CursorExpired,
        QueryError::CursorGap => ErrorCode::CursorGap,
        QueryError::Database(_)
        | QueryError::DependencyUnavailable
        | QueryError::MalformedData
        | QueryError::Contract => {
            tracing::error!(request_id = %context.id(), "fleet query dependency failed");
            ErrorCode::DependencyUnavailable
        }
    })
}

pub(super) async fn hosts(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetReadUser,
    FleetQuery(params): FleetQuery<CatalogParams>,
) -> Result<Json<QueryPage<HostReadV1>>, FleetApiError> {
    registry(&state)
        .list_hosts(user.principal(), params.into())
        .await
        .map(Json)
        .map_err(|error| query_error(context, &error))
}

pub(super) async fn host(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetReadUser,
    FleetPath(host_id): FleetPath<HostId>,
    FleetQuery(_): FleetQuery<NoParams>,
) -> Result<Json<QueryResult<HostReadV1>>, FleetApiError> {
    registry(&state)
        .get_host(user.principal(), host_id)
        .await
        .map(Json)
        .map_err(|error| query_error(context, &error))
}

pub(super) async fn backends(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetReadUser,
    FleetPath(host_id): FleetPath<HostId>,
    FleetQuery(params): FleetQuery<CatalogParams>,
) -> Result<Json<QueryPage<BackendReadV1>>, FleetApiError> {
    registry(&state)
        .list_backends(user.principal(), host_id, params.into())
        .await
        .map(Json)
        .map_err(|error| query_error(context, &error))
}

pub(super) async fn backend(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetReadUser,
    FleetPath((host_id, backend_id)): FleetPath<(HostId, BackendId)>,
    FleetQuery(_): FleetQuery<NoParams>,
) -> Result<Json<QueryResult<BackendReadV1>>, FleetApiError> {
    registry(&state)
        .get_backend(user.principal(), host_id, backend_id)
        .await
        .map(Json)
        .map_err(|error| query_error(context, &error))
}

pub(super) async fn domains(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetReadUser,
    FleetPath(host_id): FleetPath<HostId>,
    FleetQuery(params): FleetQuery<CatalogParams>,
) -> Result<Json<QueryPage<DomainReadV1>>, FleetApiError> {
    registry(&state)
        .list_domains(user.principal(), host_id, params.into())
        .await
        .map(Json)
        .map_err(|error| query_error(context, &error))
}

pub(super) async fn domain(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetReadUser,
    FleetPath((host_id, domain_id)): FleetPath<(HostId, ResourceDomainId)>,
    FleetQuery(_): FleetQuery<NoParams>,
) -> Result<Json<QueryResult<DomainReadV1>>, FleetApiError> {
    registry(&state)
        .get_domain(user.principal(), host_id, domain_id)
        .await
        .map(Json)
        .map_err(|error| query_error(context, &error))
}

pub(super) async fn samples(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetReadUser,
    FleetPath(host_id): FleetPath<HostId>,
    FleetQuery(params): FleetQuery<SampleParams>,
) -> Result<Json<QueryPage<SampleReadV1>>, FleetApiError> {
    registry(&state)
        .list_samples(user.principal(), host_id, params.into())
        .await
        .map(Json)
        .map_err(|error| query_error(context, &error))
}

pub(super) async fn events(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetReadUser,
    FleetPath(host_id): FleetPath<HostId>,
    FleetQuery(params): FleetQuery<EventParams>,
) -> Result<Json<QueryPage<HostEventReadV1>>, FleetApiError> {
    registry(&state)
        .list_host_events(user.principal(), host_id, params.into())
        .await
        .map(Json)
        .map_err(|error| query_error(context, &error))
}
