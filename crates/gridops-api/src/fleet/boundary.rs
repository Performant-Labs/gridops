//! Narrow fleet HTTP parsing and cookie authentication boundaries.
//! Cookie writes require an exact Origin; agent bearer requests use a separate extractor.

use std::{marker::PhantomData, panic::AssertUnwindSafe, time::Duration};

use axum::{
    Json,
    extract::{FromRequest, FromRequestParts, Path, Request},
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use futures_util::FutureExt as _;
use gridops_core::api_tokens::{ApiTokenScope, ApiTokenService};
use gridops_core::fleet::{UpstreamText, authorization::Principal, protocol::browser::ErrorCode};
use serde::de::DeserializeOwned;

use super::error::{FleetApiError, RequestContext};
use crate::{
    auth::{AuthUser, authenticate_session},
    error::ApiError,
    state::AppState,
};

pub(super) fn context(parts: &Parts) -> RequestContext {
    parts
        .extensions
        .get::<RequestContext>()
        .copied()
        .unwrap_or_else(RequestContext::new)
}

/// Replaces untrusted request IDs and normalizes router-level failures.
pub(super) async fn envelope(request: Request, next: Next) -> Response {
    // Finish before the legacy 30-second outer timeout so fleet failures retain
    // their typed envelope. The 25-second agent poll fits inside this deadline.
    envelope_with_deadline(request, next, Duration::from_secs(29)).await
}

async fn envelope_with_deadline(mut request: Request, next: Next, deadline: Duration) -> Response {
    let context = RequestContext::new();
    request.extensions_mut().insert(context);
    let mut response =
        match tokio::time::timeout(deadline, AssertUnwindSafe(next.run(request)).catch_unwind())
            .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => {
                // Never format a panic payload: it may contain request data.
                tracing::error!(request_id = %context.id(), "fleet handler failed unexpectedly");
                context
                    .error(ErrorCode::DependencyUnavailable)
                    .into_response()
            }
            Err(_) => {
                tracing::warn!(request_id = %context.id(), "fleet request deadline exceeded");
                context
                    .error(ErrorCode::DependencyUnavailable)
                    .into_response()
            }
        };
    // Handler errors carry the typed body. Method/fallback rejection does not.
    if response.status() == StatusCode::METHOD_NOT_ALLOWED {
        response = context.error(ErrorCode::InvalidRequest).into_response();
    }
    if let Ok(header) = HeaderValue::from_str(&context.id().to_string()) {
        response.headers_mut().insert("x-request-id", header);
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

impl FromRequestParts<AppState> for RequestContext {
    type Rejection = FleetApiError;

    async fn from_request_parts(parts: &mut Parts, _: &AppState) -> Result<Self, Self::Rejection> {
        Ok(context(parts))
    }
}

/// Scope markers naming what a route's extractor demands of a bearer token.
/// The default, [`CookieOnly`], rejects tokens outright, so a handler opts
/// in by naming a scope type; that opt-in is the default-deny mechanism.
pub(super) trait RouteScope {
    const REQUIRED: Option<ApiTokenScope>;
}

pub(super) struct CookieOnly;
impl RouteScope for CookieOnly {
    const REQUIRED: Option<ApiTokenScope> = None;
}

pub(super) struct Read;
impl RouteScope for Read {
    const REQUIRED: Option<ApiTokenScope> = Some(ApiTokenScope::FleetRead);
}

pub(super) struct Enroll;
impl RouteScope for Enroll {
    const REQUIRED: Option<ApiTokenScope> = Some(ApiTokenScope::FleetEnroll);
}

pub(super) struct Approve;
impl RouteScope for Approve {
    const REQUIRED: Option<ApiTokenScope> = Some(ApiTokenScope::FleetApprove);
}

pub(super) struct Operate;
impl RouteScope for Operate {
    const REQUIRED: Option<ApiTokenScope> = Some(ApiTokenScope::FleetOperate);
}

/// Parse the `Authorization: Bearer gat_...` header of a token request.
/// Mirrors the agent `bearer()` rules: no cookie alongside, exactly one
/// header, case-insensitive scheme, exact charset and length.
pub(super) fn token_bearer(headers: &HeaderMap) -> Option<&str> {
    if headers.contains_key(header::COOKIE) {
        return None;
    }
    let mut authorizations = headers.get_all(header::AUTHORIZATION).iter();
    let value = authorizations.next()?.to_str().ok()?;
    if authorizations.next().is_some() {
        return None;
    }
    let (scheme, token) = value.split_once(' ')?;
    let body = token.strip_prefix("gat_")?;
    (scheme.eq_ignore_ascii_case("bearer")
        && body.len() == 43
        && body
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || b"_-".contains(&ch)))
    .then_some(token)
}

/// Resolve a browser request's credential and principal. Cookie-only routes
/// reject every `Authorization` header. Scoped routes accept either the
/// cookie path (unchanged) or a token that holds the route's scope; every
/// authentication failure is the same `unauthenticated`, and a scope miss is
/// `forbidden` naming the scope.
async fn resolve_user<S: RouteScope>(
    parts: &mut Parts,
    state: &AppState,
    refresh_activity: bool,
) -> Result<Principal, FleetApiError> {
    let context = context(parts);
    let authorizations = parts.headers.get_all(header::AUTHORIZATION).iter();
    let authorization_count = authorizations.count();
    if authorization_count > 1 {
        return Err(context.error(ErrorCode::Unauthenticated));
    }
    if authorization_count == 1 {
        let Some(required) = S::REQUIRED else {
            // Cookie-only route: any Authorization header is refused.
            return Err(context.error(ErrorCode::Unauthenticated));
        };
        let token = token_bearer(&parts.headers)
            .ok_or_else(|| context.error(ErrorCode::Unauthenticated))?;
        let authenticated = ApiTokenService::new(state.database.clone())
            .authenticate(token)
            .await
            .map_err(|error| {
                if matches!(error, gridops_core::api_tokens::ApiTokenError::Database(_)) {
                    context.error(ErrorCode::DependencyUnavailable)
                } else {
                    // Unknown, malformed, expired, revoked and ownerless all
                    // look alike.
                    context.error(ErrorCode::Unauthenticated)
                }
            })?;
        if !authenticated.scopes.contains(&required) {
            let required_scope = UpstreamText::new(required.as_str())
                .map_err(|_| context.error(ErrorCode::InvalidRequest))?;
            return Err(context.error_with_details(
                ErrorCode::Forbidden,
                gridops_core::fleet::protocol::browser::ErrorDetails::Scope { required_scope },
            ));
        }
        return Ok(Principal::AuthenticatedUser {
            user_id: authenticated.user_id,
        });
    }
    let session = if refresh_activity {
        AuthUser::from_request_parts(parts, state)
            .await
            .map(|user| user.id)
            .map_err(session_error(&context))
    } else {
        authenticate_session(parts, state)
            .await
            .map(|session| session.user.id)
            .map_err(session_error(&context))
    }?;
    Ok(Principal::AuthenticatedUser { user_id: session })
}

fn session_error(context: &RequestContext) -> impl Fn(ApiError) -> FleetApiError + '_ {
    move |error| {
        context.error(match error {
            ApiError::Unauthorized => ErrorCode::Unauthenticated,
            ApiError::Forbidden => ErrorCode::Forbidden,
            _ => ErrorCode::DependencyUnavailable,
        })
    }
}

pub(super) struct FleetUser<S: RouteScope = CookieOnly> {
    principal: Principal,
    scope: PhantomData<S>,
}

impl<S: RouteScope> FleetUser<S> {
    pub(super) fn principal(&self) -> &Principal {
        &self.principal
    }
}

impl<S: RouteScope> FromRequestParts<AppState> for FleetUser<S> {
    type Rejection = FleetApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let principal = resolve_user::<S>(parts, state, true).await?;
        Ok(Self {
            principal,
            scope: PhantomData,
        })
    }
}

/// Proves cookie authentication for read-only queries without refreshing session activity.
pub(super) struct FleetReadUser<S: RouteScope = CookieOnly> {
    principal: Principal,
    scope: PhantomData<S>,
}

impl<S: RouteScope> FleetReadUser<S> {
    pub(super) fn principal(&self) -> &Principal {
        &self.principal
    }
}

impl<S: RouteScope> FromRequestParts<AppState> for FleetReadUser<S> {
    type Rejection = FleetApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let principal = resolve_user::<S>(parts, state, false).await?;
        Ok(Self {
            principal,
            scope: PhantomData,
        })
    }
}

/// Proves cookie authentication plus Origin validation, not administration.
/// The core write transaction still reloads the current user's permissions.
/// A bearer token carries no ambient credential, so it skips the `Origin`
/// check; a cookie request is unchanged.
pub(super) struct FleetWriteUser<S: RouteScope = CookieOnly>(FleetUser<S>);

impl<S: RouteScope> FleetWriteUser<S> {
    pub(super) fn principal(&self) -> &Principal {
        self.0.principal()
    }
}

impl<S: RouteScope> FromRequestParts<AppState> for FleetWriteUser<S> {
    type Rejection = FleetApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let context = context(parts);
        // The ambient cookie credential needs the exact-`Origin` check before
        // anything else, exactly as before. A scoped route's bearer request
        // carries no ambient credential and skips it; a cookie-only route
        // checks `Origin` first even when an `Authorization` header is
        // present, preserving the original ordering.
        let bearer_candidate =
            S::REQUIRED.is_some() && parts.headers.contains_key(header::AUTHORIZATION);
        if !bearer_candidate {
            let expected = state.config.base_url().origin().ascii_serialization();
            if !same_origin(&parts.headers, &expected) {
                return Err(context.error(ErrorCode::Forbidden));
            }
        }
        let principal = resolve_user::<S>(parts, state, true).await?;
        Ok(Self(FleetUser {
            principal,
            scope: PhantomData,
        }))
    }
}

fn same_origin(headers: &HeaderMap, expected: &str) -> bool {
    let mut values = headers.get_all(header::ORIGIN).iter();
    let origin = values.next();
    values.next().is_none() && origin.is_some_and(|value| value.as_bytes() == expected.as_bytes())
}

pub(super) struct FleetJson<T>(pub T);

impl<T: DeserializeOwned + Send> FromRequest<AppState> for FleetJson<T> {
    type Rejection = FleetApiError;

    async fn from_request(request: Request, state: &AppState) -> Result<Self, Self::Rejection> {
        let context = request
            .extensions()
            .get::<RequestContext>()
            .copied()
            .unwrap_or_else(RequestContext::new);
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|error| {
                context.error(if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ErrorCode::PayloadTooLarge
                } else {
                    ErrorCode::InvalidRequest
                })
            })
    }
}

pub(super) struct FleetPath<T>(pub T);

impl<T: DeserializeOwned + Send> FromRequestParts<AppState> for FleetPath<T> {
    type Rejection = FleetApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let context = context(parts);
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| Self(value))
            .map_err(|_| context.error(ErrorCode::InvalidRequest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn panic_keeps_the_fleet_error_envelope_without_exposing_its_payload()
    -> anyhow::Result<()> {
        use axum::{Router, body::Body, middleware, routing::get};
        use gridops_core::fleet::protocol::browser::FleetErrorResponse;
        use tower::ServiceExt as _;

        async fn fault() -> Response {
            std::panic::resume_unwind(Box::new("private-panic-sentinel"));
        }
        let router = Router::new()
            .route("/fault", get(fault))
            .layer(middleware::from_fn(envelope));
        let response = router
            .oneshot(Request::builder().uri("/fault").body(Body::empty())?)
            .await?;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let id = response.headers()["x-request-id"].to_str()?.to_owned();
        let body = axum::body::to_bytes(response.into_body(), 1024).await?;
        assert!(!String::from_utf8_lossy(&body).contains("private-panic-sentinel"));
        let error: FleetErrorResponse = serde_json::from_slice(&body)?;
        assert_eq!(error.code, ErrorCode::DependencyUnavailable);
        assert_eq!(error.request_id.to_string(), id);
        Ok(())
    }

    #[tokio::test]
    async fn deadline_keeps_the_fleet_error_envelope_and_server_request_id() -> anyhow::Result<()> {
        use axum::{Router, body::Body, middleware, routing::get};
        use gridops_core::fleet::protocol::browser::FleetErrorResponse;
        use tower::ServiceExt as _;

        let router = Router::new()
            .route("/slow", get(std::future::pending::<Response>))
            .layer(middleware::from_fn(|request, next| {
                envelope_with_deadline(request, next, Duration::from_millis(1))
            }));
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/slow")
                    .header("x-request-id", "untrusted-request-id")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let id = response.headers()["x-request-id"].to_str()?.to_owned();
        assert_ne!(id, "untrusted-request-id");
        let body = axum::body::to_bytes(response.into_body(), 1024).await?;
        let error: FleetErrorResponse = serde_json::from_slice(&body)?;
        assert_eq!(error.code, ErrorCode::DependencyUnavailable);
        assert_eq!(error.request_id.to_string(), id);
        Ok(())
    }

    #[test]
    fn cookie_origin_requires_one_exact_configured_origin() -> anyhow::Result<()> {
        let expected = "https://gridops.example";
        let mut headers = HeaderMap::new();
        assert!(!same_origin(&headers, expected));
        for value in [
            "null",
            "http://gridops.example",
            "https://other.example",
            "https://gridops.example/",
            "https://gridops.example https://other.example",
        ] {
            headers.insert(header::ORIGIN, value.parse()?);
            assert!(!same_origin(&headers, expected));
        }
        headers.insert(header::ORIGIN, expected.parse()?);
        assert!(same_origin(&headers, expected));
        headers.append(header::ORIGIN, expected.parse()?);
        assert!(!same_origin(&headers, expected));
        Ok(())
    }
}
