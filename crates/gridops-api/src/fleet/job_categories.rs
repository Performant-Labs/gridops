//! The job category rules in Settings (#199): list, save (the whole list,
//! in order) and preview. Session-only and administrators only: the routes
//! are registered in `session_routes()` (`main.rs`), under `refuse_bearer`
//! (W-5).
//!
//! The bodies (`job_categories_contract_tests.rs`):
//! - `GET` → `{"rules": [{position, category, field, pattern}]}`;
//! - `PUT` `{"rules": [{category, field, pattern}]}` → the saved list, or a
//!   422 `ApiError::Invalid` whose `details` are `[{position, message}]`;
//! - `POST …/preview` with a proposed list → `{"rules": [{position,
//!   matches}], "other": [{name, jobs}]}`.
//!
//! #197 registers the routes and checks the caller. #199 fills the handlers
//! in (from `gridops_core::fleet_health::category`); until then each
//! answers 501.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse as _, Response},
};
use serde_json::{Value, json};

use crate::{
    auth::{AuthUser, require_system_admin},
    error::ApiResult,
    resources::SameOrigin,
    state::AppState,
};

fn not_yet() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({ "error": "Job category rules arrive with #199." })),
    )
        .into_response()
}

/// `GET /api/v1/settings/job-categories`.
#[allow(clippy::unused_async, reason = "#199 reads the rules here")]
pub(crate) async fn rules(State(state): State<AppState>, user: AuthUser) -> ApiResult<Response> {
    require_system_admin(&user)?;
    let _ = state;
    Ok(not_yet())
}

/// `PUT /api/v1/settings/job-categories`.
#[allow(clippy::unused_async, reason = "#199 saves the rules here")]
pub(crate) async fn save_rules(
    State(state): State<AppState>,
    _same_origin: SameOrigin,
    user: AuthUser,
    Json(input): Json<Value>,
) -> ApiResult<Response> {
    require_system_admin(&user)?;
    let _ = (state, input);
    Ok(not_yet())
}

/// `POST /api/v1/settings/job-categories/preview`.
#[allow(clippy::unused_async, reason = "#199 counts the matches here")]
pub(crate) async fn preview(
    State(state): State<AppState>,
    _same_origin: SameOrigin,
    user: AuthUser,
    Json(input): Json<Value>,
) -> ApiResult<Response> {
    require_system_admin(&user)?;
    let _ = (state, input);
    Ok(not_yet())
}
