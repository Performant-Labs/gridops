//! Admin, cookie-only management of API tokens. A bearer token can never
//! mint, extend or list tokens, so a leaked token cannot replicate itself.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use gridops_core::api_tokens::{
    ApiTokenError, ApiTokenRow, ApiTokenScope, ApiTokenService, CreateOutcome, TokenReceipt,
};
use gridops_core::fleet::UpstreamText;
use gridops_core::fleet::authorization::Principal;
use gridops_core::fleet::protocol::browser::{ErrorCode, ErrorDetails};
use gridops_core::fleet::protocol::primitives::{BrowserCounter, BrowserUint53, WireTimestamp};
use serde::{Deserialize, Serialize};

use super::boundary::{FleetJson, FleetPath, FleetUser, FleetWriteUser};
use super::enrollment::RequestKey;
use super::error::{FleetApiError, RequestContext};
use crate::state::AppState;

const DEFAULT_EXPIRES_IN_DAYS: u64 = 90;
const MAX_EXPIRES_IN_DAYS: u64 = 365;
const MAX_TOKENS_PER_USER: u64 = 50;
const DAY_MILLIS: i64 = 86_400_000;

fn acting_user(principal: &Principal) -> Option<&str> {
    match principal {
        Principal::AuthenticatedUser { user_id } => Some(user_id),
        _ => None,
    }
}

fn invalid_field(context: RequestContext, field: &'static str) -> FleetApiError {
    match UpstreamText::new(field) {
        Ok(field) => context.error_with_details(
            ErrorCode::InvalidRequest,
            ErrorDetails::InvalidField { field },
        ),
        Err(_) => context.error(ErrorCode::InvalidRequest),
    }
}

fn service_error(context: &RequestContext, error: &ApiTokenError) -> FleetApiError {
    match error {
        ApiTokenError::NotAdmin => context.error(ErrorCode::Forbidden),
        ApiTokenError::NotFound => context.error(ErrorCode::NotFound),
        ApiTokenError::IdempotencyConflict => context.error(ErrorCode::IdempotencyConflict),
        ApiTokenError::LimitReached => match BrowserUint53::new(MAX_TOKENS_PER_USER) {
            Ok(maximum) => context
                .error_with_details(ErrorCode::InvalidRequest, ErrorDetails::Limit { maximum }),
            Err(_) => context.error(ErrorCode::InvalidRequest),
        },
        ApiTokenError::InvalidName | ApiTokenError::DuplicateName => {
            invalid_field(*context, "name")
        }
        ApiTokenError::InvalidExpiry => invalid_field(*context, "expiresInDays"),
        ApiTokenError::NoScopes | ApiTokenError::UnknownScope => invalid_field(*context, "scopes"),
        ApiTokenError::AuthenticationFailed => context.error(ErrorCode::Unauthenticated),
        ApiTokenError::Database(_) => context.error(ErrorCode::DependencyUnavailable),
    }
}

fn timestamp(context: RequestContext, millis: i64) -> Result<WireTimestamp, FleetApiError> {
    WireTimestamp::from_millis(millis).map_err(|_| context.error(ErrorCode::ContractRange))
}

/// Everything about a token except the secret; this is also the replay and
/// list shape, so the plaintext can appear in exactly one response.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenMetadata {
    id: String,
    name: String,
    display_prefix: String,
    scopes: Vec<ApiTokenScope>,
    created_at: WireTimestamp,
    expires_at: WireTimestamp,
    last_used_at: Option<WireTimestamp>,
    revoked_at: Option<WireTimestamp>,
}

impl TokenMetadata {
    fn from_row(context: RequestContext, row: &ApiTokenRow) -> Result<Self, FleetApiError> {
        Ok(Self {
            id: row.id.clone(),
            name: row.name.clone(),
            display_prefix: row.display_prefix.clone(),
            scopes: row.scopes.clone(),
            created_at: timestamp(context, row.created_at)?,
            expires_at: timestamp(context, row.expires_at)?,
            last_used_at: row
                .last_used_at
                .map(|value| timestamp(context, value))
                .transpose()?,
            revoked_at: row
                .revoked_at
                .map(|value| timestamp(context, value))
                .transpose()?,
        })
    }

    fn from_receipt(
        context: RequestContext,
        receipt: &TokenReceipt,
    ) -> Result<Self, FleetApiError> {
        Ok(Self {
            id: receipt.token_id().to_owned(),
            name: receipt.name().to_owned(),
            // The first eight body characters, never a credential.
            display_prefix: receipt.expose()["gat_".len().."gat_".len() + 8].to_owned(),
            scopes: receipt.scopes().to_vec(),
            created_at: timestamp(context, receipt.created_at())?,
            expires_at: timestamp(context, receipt.expires_at())?,
            last_used_at: None,
            revoked_at: None,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CreateBody {
    name: String,
    scopes: Vec<String>,
    expires_in_days: Option<BrowserCounter>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatedResponse {
    #[serde(flatten)]
    token: TokenMetadata,
    /// The plaintext, exactly once, only in the 201.
    token_secret: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MetadataResponse {
    token: TokenMetadata,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListResponse {
    items: Vec<TokenMetadata>,
}

pub(super) async fn create(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetWriteUser,
    key: RequestKey,
    FleetJson(body): FleetJson<CreateBody>,
) -> Result<Response, FleetApiError> {
    let actor = acting_user(user.principal()).ok_or_else(|| context.error(ErrorCode::Forbidden))?;
    let mut scopes = Vec::new();
    for scope in &body.scopes {
        scopes.push(ApiTokenScope::parse(scope).ok_or_else(|| invalid_field(context, "scopes"))?);
    }
    let days = body.expires_in_days.unwrap_or(
        BrowserCounter::new(DEFAULT_EXPIRES_IN_DAYS)
            .map_err(|_| context.error(ErrorCode::ContractRange))?,
    );
    if days.get() > MAX_EXPIRES_IN_DAYS {
        return Err(invalid_field(context, "expiresInDays"));
    }
    let expires_at = gridops_core::now_millis() + days.get().cast_signed() * DAY_MILLIS;
    let outcome = ApiTokenService::new(state.database.clone())
        .create_admin(actor, &body.name, &scopes, expires_at, &key.into_string())
        .await
        .map_err(|error| service_error(&context, &error))?;
    match outcome {
        CreateOutcome::Created(receipt) => Ok((
            StatusCode::CREATED,
            Json(CreatedResponse {
                token: TokenMetadata::from_receipt(context, &receipt)?,
                token_secret: receipt.expose().to_owned(),
            }),
        )
            .into_response()),
        CreateOutcome::Replay(row) => Ok((
            StatusCode::OK,
            Json(MetadataResponse {
                token: TokenMetadata::from_row(context, &row)?,
            }),
        )
            .into_response()),
    }
}

pub(super) async fn list(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetUser,
) -> Result<Json<ListResponse>, FleetApiError> {
    let actor = acting_user(user.principal()).ok_or_else(|| context.error(ErrorCode::Forbidden))?;
    let rows = ApiTokenService::new(state.database.clone())
        .list_for_user(actor)
        .await
        .map_err(|error| service_error(&context, &error))?;
    let items = rows
        .iter()
        .map(|row| TokenMetadata::from_row(context, row))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(ListResponse { items }))
}

#[derive(Deserialize)]
pub(super) struct RevokePath {
    token_id: String,
}

pub(super) async fn revoke(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetWriteUser,
    FleetPath(path): FleetPath<RevokePath>,
) -> Result<Json<MetadataResponse>, FleetApiError> {
    let actor = acting_user(user.principal()).ok_or_else(|| context.error(ErrorCode::Forbidden))?;
    let row = ApiTokenService::new(state.database.clone())
        .revoke_admin(actor, &path.token_id)
        .await
        .map_err(|error| service_error(&context, &error))?;
    Ok(Json(MetadataResponse {
        token: TokenMetadata::from_row(context, &row)?,
    }))
}
