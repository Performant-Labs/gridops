//! Fleet transport is isolated from legacy routes and host runtime execution.
//! Only implemented registry operations are exposed; enrollment grants no start authority.

mod api_tokens;
mod auth;
pub(crate) mod boundary;
mod commands;
mod credentials;
mod enrollment;
mod error;
mod inventory;
pub(crate) mod openapi;
mod profiles;
mod query;
mod query_boundary;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    response::{IntoResponse, Response},
    routing::{any, get, post, put},
};
use gridops_core::fleet::protocol::{browser::ErrorCode, primitives::DEFAULT_JSON_BYTES};

use self::error::RequestContext;
use crate::state::AppState;

/// The token scope map, as data: every route, fleet or not, that accepts a
/// bearer token, and the scope it requires. Every other route refuses tokens
/// (`token_not_accepted`) or never authenticates users. The extractors in
/// `boundary.rs` and `access.rs` name the same scope per handler; the
/// `OpenAPI` drift test, the `access.rs` tests (against `docs/fleet-api.md`)
/// and the HTTP tests cross-check all of them against this table.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const TOKEN_SCOPED_ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "/api/v1/fleet/hosts", "fleet:read"),
    ("GET", "/api/v1/fleet/hosts/{host_id}", "fleet:read"),
    ("DELETE", "/api/v1/fleet/hosts/{host_id}", "fleet:operate"),
    (
        "GET",
        "/api/v1/fleet/hosts/{host_id}/backends",
        "fleet:read",
    ),
    (
        "GET",
        "/api/v1/fleet/hosts/{host_id}/backends/{backend_id}",
        "fleet:read",
    ),
    ("GET", "/api/v1/fleet/hosts/{host_id}/domains", "fleet:read"),
    (
        "GET",
        "/api/v1/fleet/hosts/{host_id}/domains/{domain_id}",
        "fleet:read",
    ),
    ("GET", "/api/v1/fleet/hosts/{host_id}/samples", "fleet:read"),
    ("GET", "/api/v1/fleet/hosts/{host_id}/events", "fleet:read"),
    ("GET", "/api/v1/fleet/hosts/{host_id}/grants", "fleet:read"),
    (
        "GET",
        "/api/v1/fleet/hosts/{host_id}/unconfirmed",
        "fleet:read",
    ),
    ("GET", "/api/v1/fleet/targets", "fleet:read"),
    ("GET", "/api/v1/fleet/workload", "fleet:read"),
    ("GET", "/api/v1/fleet/pools/{pool_id}/profile", "fleet:read"),
    ("GET", "/api/v1/fleet/pools/{pool_id}/queue", "fleet:read"),
    (
        "GET",
        "/api/v1/fleet/pools/{pool_id}/capacity",
        "fleet:read",
    ),
    ("GET", "/api/v1/operations/{operation_id}", "fleet:read"),
    ("POST", "/api/v1/fleet/enrollments", "fleet:enroll"),
    (
        "GET",
        "/api/v1/fleet/enrollments/{enrollment_id}",
        "fleet:enroll",
    ),
    (
        "POST",
        "/api/v1/fleet/enrollments/{enrollment_id}/revoke",
        "fleet:enroll",
    ),
    (
        "POST",
        "/api/v1/fleet/hosts/{host_id}/approve",
        "fleet:approve",
    ),
    (
        "POST",
        "/api/v1/fleet/hosts/{host_id}/retire",
        "fleet:retire",
    ),
    (
        "POST",
        "/api/v1/fleet/hosts/{host_id}/backends/{backend_id}/enabled",
        "fleet:operate",
    ),
    (
        "POST",
        "/api/v1/fleet/hosts/{host_id}/lifecycle",
        "fleet:operate",
    ),
    (
        "PUT",
        "/api/v1/fleet/hosts/{host_id}/budget",
        "fleet:operate",
    ),
    (
        "POST",
        "/api/v1/fleet/hosts/{host_id}/grants/{target_id}",
        "fleet:operate",
    ),
    (
        "PUT",
        "/api/v1/fleet/pools/{pool_id}/profile",
        "fleet:operate",
    ),
    (
        "PUT",
        "/api/v1/fleet/pools/{pool_id}/capacity",
        "fleet:operate",
    ),
    // Runner pools, outside the fleet router (`access.rs`).
    ("GET", "/api/v1/runner-pools", "pools:read"),
    ("GET", "/api/v1/runner-pools/options", "pools:read"),
    ("GET", "/api/v1/runner-pools/repositories", "pools:read"),
    (
        "GET",
        "/api/v1/installations/{installation_id}/runner-groups",
        "pools:read",
    ),
    (
        "GET",
        "/api/v1/installations/{installation_id}/repositories",
        "pools:read",
    ),
    ("GET", "/api/v1/runner-pools/{pool_id}", "pools:read"),
    ("GET", "/api/v1/runner-pools/{pool_id}/events", "pools:read"),
    ("GET", "/api/v1/overview/capacity", "pools:read"),
    ("GET", "/api/v1/overview/tart-host", "pools:read"),
    ("POST", "/api/v1/runner-pools", "pools:manage"),
    ("PUT", "/api/v1/runner-pools/{pool_id}", "pools:manage"),
    ("DELETE", "/api/v1/runner-pools/{pool_id}", "pools:manage"),
];

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/fleet/targets", get(profiles::targets))
        .route(
            "/api/v1/fleet/pools/{pool_id}/profile",
            get(profiles::profile).put(profiles::save_profile),
        )
        .route("/api/v1/fleet/pools/{pool_id}/queue", get(profiles::queue))
        .route(
            "/api/v1/fleet/pools/{pool_id}/capacity",
            get(profiles::capacity).put(profiles::save_capacity),
        )
        .route("/api/v1/fleet/workload", get(profiles::workload))
        .route("/api/v1/fleet/hosts", get(query::hosts))
        .route(
            "/api/v1/fleet/hosts/{host_id}",
            get(query::host).delete(enrollment::delete_host),
        )
        .route(
            "/api/v1/fleet/hosts/{host_id}/backends",
            get(query::backends),
        )
        .route(
            "/api/v1/fleet/hosts/{host_id}/backends/{backend_id}",
            get(query::backend),
        )
        .route("/api/v1/fleet/hosts/{host_id}/domains", get(query::domains))
        .route(
            "/api/v1/fleet/hosts/{host_id}/domains/{domain_id}",
            get(query::domain),
        )
        .route(
            "/api/v1/fleet/hosts/{host_id}/unconfirmed",
            get(enrollment::unconfirmed),
        )
        .route(
            "/api/v1/fleet/hosts/{host_id}/unconfirmed/repair",
            post(enrollment::repair_unconfirmed),
        )
        .route("/api/v1/fleet/hosts/{host_id}/samples", get(query::samples))
        .route("/api/v1/fleet/hosts/{host_id}/events", get(query::events))
        .route("/api/v1/fleet/enrollments", post(enrollment::issue))
        .route(
            "/api/v1/fleet/enrollments/{enrollment_id}",
            get(enrollment::status),
        )
        .route(
            "/api/v1/fleet/enrollments/{enrollment_id}/revoke",
            post(enrollment::revoke),
        )
        .route("/api/v1/fleet/agent/enroll", post(enrollment::consume))
        .route(
            "/api/v1/fleet/agent/hosts/{host_id}/handshake",
            post(inventory::handshake),
        )
        .route(
            "/api/v1/fleet/agent/hosts/{host_id}/inventory",
            post(inventory::submit),
        )
        .route(
            "/api/v1/fleet/agent/hosts/{host_id}/logs",
            post(commands::logs),
        )
        .route(
            "/api/v1/fleet/agent/hosts/{host_id}/output",
            post(commands::output),
        )
        .route(
            "/api/v1/fleet/hosts/{host_id}/logs",
            get(enrollment::agent_logs),
        )
        .route(
            "/api/v1/fleet/agent/hosts/{host_id}/bootstrap",
            post(commands::bootstrap),
        )
        .route(
            "/api/v1/fleet/agent/hosts/{host_id}/commands/poll",
            post(commands::poll),
        )
        .route(
            "/api/v1/fleet/agent/hosts/{host_id}/commands/result",
            post(commands::result),
        )
        .route(
            "/api/v1/fleet/agent/hosts/{host_id}/heartbeat",
            post(inventory::heartbeat),
        )
        .route(
            "/api/v1/fleet/hosts/{host_id}/backends/{backend_id}/enabled",
            post(enrollment::backend_enabled),
        )
        .route(
            "/api/v1/fleet/hosts/{host_id}/grants",
            get(enrollment::grants),
        )
        .route(
            "/api/v1/fleet/hosts/{host_id}/grants/{target_id}",
            post(enrollment::set_grant),
        )
        .route(
            "/api/v1/fleet/hosts/{host_id}/budget",
            put(enrollment::set_budget),
        )
        .route(
            "/api/v1/fleet/hosts/{host_id}/retire",
            post(enrollment::retire),
        )
        .route(
            "/api/v1/fleet/hosts/{host_id}/lifecycle",
            post(enrollment::lifecycle),
        )
        .route(
            "/api/v1/fleet/hosts/{host_id}/approve",
            post(enrollment::approve),
        )
        .route(
            "/api/v1/fleet/hosts/{host_id}/recover",
            post(enrollment::recover),
        )
        .route(
            "/api/v1/fleet/hosts/{host_id}/credentials/rotate",
            post(credentials::request),
        )
        .route(
            "/api/v1/fleet/agent/hosts/{host_id}/credentials/rotate",
            post(credentials::exchange),
        )
        .route(
            "/api/v1/fleet/agent/hosts/{host_id}/credentials/ack",
            post(credentials::acknowledge),
        )
        .route(
            "/api/v1/fleet/api-tokens",
            post(api_tokens::create).get(api_tokens::list),
        )
        .route(
            "/api/v1/fleet/api-tokens/{token_id}/revoke",
            post(api_tokens::revoke),
        )
        .route("/api/v1/fleet", any(not_found))
        .route("/api/v1/fleet/", any(not_found))
        .route("/api/v1/fleet/{*path}", any(not_found))
        .layer(DefaultBodyLimit::max(DEFAULT_JSON_BYTES))
        .layer(middleware::from_fn(boundary::envelope))
}

pub(crate) fn operation_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/operations/{operation_id}",
            get(credentials::status),
        )
        .route("/api/v1/operations", any(not_found))
        .route("/api/v1/operations/", any(not_found))
        .route("/api/v1/operations/{operation_id}/", any(not_found))
        .route("/api/v1/operations/{operation_id}/{*path}", any(not_found))
        .layer(middleware::from_fn(boundary::envelope))
}

async fn not_found(context: RequestContext) -> Response {
    context.error(ErrorCode::NotFound).into_response()
}
