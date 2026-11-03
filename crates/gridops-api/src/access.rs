//! Who may call the routes outside the fleet router.
//!
//! Three kinds of route, mounted as three router groups in `main.rs`:
//!
//! - **public** routes never authenticate a user (health, the published API
//!   document, the install script, GitHub OAuth and webhook endpoints); an
//!   `Authorization` header has no effect on them;
//! - **token** routes (runner pools) take a browser session exactly as
//!   before, or a scoped API token through the extractors below;
//! - **session** routes need a signed-in browser session and take no token:
//!   [`refuse_bearer`] answers any bearer token with `403 token_not_accepted`
//!   before the handler runs, so a token is never answered as if the caller
//!   were signed out.
//!
//! `docs/fleet-api.md` ("Who may call what") lists every route; the tests
//! below keep that table, these groups and the token scope table in step.

use std::marker::PhantomData;

use axum::{
    extract::{FromRequestParts, Request},
    http::request::Parts,
    middleware::Next,
    response::{IntoResponse, Response},
};
use gridops_core::{api_tokens::ApiTokenScope, fleet::protocol::browser::ErrorCode};
use sqlx::Row as _;

use crate::{
    auth::{AuthUser, OptionalAuth, TokenActor, assert_same_origin},
    fleet::boundary::{
        Refusal, authenticate_scoped_token, has_bearer_scheme, refusal, token_not_accepted,
    },
    state::AppState,
};

/// Refuses a bearer token on a route that does not accept one. Requests
/// without a bearer token (a cookie, nothing, or another `Authorization`
/// scheme such as a proxy's `Basic`) reach the handler exactly as before.
pub(crate) async fn refuse_bearer(request: Request, next: Next) -> Response {
    if has_bearer_scheme(request.headers()) {
        return token_not_accepted();
    }
    next.run(request).await
}

/// The token scope a pool route requires.
pub(crate) trait PoolScope {
    const SCOPE: ApiTokenScope;
}

/// Read pools, their events and capacity, and the pool form's options.
pub(crate) struct PoolsRead;
impl PoolScope for PoolsRead {
    const SCOPE: ApiTokenScope = ApiTokenScope::PoolsRead;
}

/// Create, edit and remove pools.
pub(crate) struct PoolsManage;
impl PoolScope for PoolsManage {
    const SCOPE: ApiTokenScope = ApiTokenScope::PoolsManage;
}

/// The token's owner as an [`AuthUser`], so the pool handlers apply exactly
/// the ownership and validation rules they apply to that user's browser
/// session. The owner must still be a system administrator: tokens are an
/// administrator's tool (only administrators can create them), and a
/// demoted owner's token stops working everywhere, as on the fleet routes,
/// even where the browser would still let that member act through their
/// installation permission.
async fn token_owner(
    parts: &Parts,
    state: &AppState,
    scope: ApiTokenScope,
) -> Result<AuthUser, Refusal> {
    let token = authenticate_scoped_token(parts, state, scope).await?;
    let row = sqlx::query("SELECT id,login,github_id,role FROM users WHERE id=?")
        .bind(&token.user_id)
        .fetch_optional(&state.database)
        .await
        .map_err(|error| {
            tracing::error!(kind = ?error, "api token owner lookup failed");
            refusal(ErrorCode::DependencyUnavailable)
        })?
        .ok_or_else(|| refusal(ErrorCode::Unauthenticated))?;
    let role: String = row.get("role");
    if role != "admin" {
        return Err(refusal(ErrorCode::Forbidden));
    }
    Ok(AuthUser {
        id: row.get("id"),
        login: row.get("login"),
        github_id: row.get("github_id"),
        role,
        via_token: Some(TokenActor {
            id: token.token_id,
            name: token.name,
        }),
    })
}

/// Replaces `AuthUser` on a pool read route: the session as before, or a
/// token holding `S::SCOPE`.
pub(crate) struct PoolUser<S: PoolScope>(pub(crate) AuthUser, pub(crate) PhantomData<S>);

impl<S: PoolScope> FromRequestParts<AppState> for PoolUser<S> {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = if has_bearer_scheme(&parts.headers) {
            token_owner(parts, state, S::SCOPE)
                .await
                .map_err(IntoResponse::into_response)?
        } else {
            AuthUser::from_request_parts(parts, state)
                .await
                .map_err(IntoResponse::into_response)?
        };
        Ok(Self(user, PhantomData))
    }
}

/// Replaces `OptionalAuth` on a pool read route that answers a signed-out
/// browser with an empty body. A bearer token is never answered that way: it
/// either authenticates with `S::SCOPE` or is refused.
pub(crate) struct OptionalPoolUser<S: PoolScope>(
    pub(crate) Option<AuthUser>,
    pub(crate) PhantomData<S>,
);

impl<S: PoolScope> FromRequestParts<AppState> for OptionalPoolUser<S> {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = if has_bearer_scheme(&parts.headers) {
            Some(
                token_owner(parts, state, S::SCOPE)
                    .await
                    .map_err(IntoResponse::into_response)?,
            )
        } else {
            OptionalAuth::from_request_parts(parts, state)
                .await
                .map_err(IntoResponse::into_response)?
                .0
        };
        Ok(Self(user, PhantomData))
    }
}

/// Replaces `SameOrigin` followed by `AuthUser` on a pool write route. A
/// browser request keeps both checks in their original order. A token
/// request carries no ambient credential, so the cross-site check does not
/// apply to it.
pub(crate) struct PoolWriteUser<S: PoolScope>(pub(crate) AuthUser, pub(crate) PhantomData<S>);

impl<S: PoolScope> FromRequestParts<AppState> for PoolWriteUser<S> {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = if has_bearer_scheme(&parts.headers) {
            token_owner(parts, state, S::SCOPE)
                .await
                .map_err(IntoResponse::into_response)?
        } else {
            assert_same_origin(state, &parts.headers).map_err(IntoResponse::into_response)?;
            AuthUser::from_request_parts(parts, state)
                .await
                .map_err(IntoResponse::into_response)?
        };
        Ok(Self(user, PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context as _;
    use std::collections::BTreeSet;

    const MAIN: &str = include_str!("main.rs");
    const FLEET: &str = include_str!("fleet/mod.rs");
    const DOC: &str = include_str!("../../../docs/fleet-api.md");

    /// `(method, path, outcome, scope)`: one row of the documented matrix.
    type Row = (String, String, String, Option<String>);

    fn documented() -> anyhow::Result<Vec<Row>> {
        let section = DOC
            .split("\n## Who may call what\n")
            .nth(1)
            .context("no 'Who may call what' section")?;
        let section = section.split("\n## ").next().unwrap_or_default();
        let mut rows = Vec::new();
        for line in section.lines() {
            let cells: Vec<String> = line
                .trim()
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().replace('`', ""))
                .collect();
            if cells.len() == 6 && matches!(cells[0].as_str(), "GET" | "POST" | "PUT" | "DELETE") {
                rows.push((
                    cells[0].clone(),
                    cells[1].clone(),
                    cells[5].clone(),
                    (cells[3] != "—").then(|| cells[3].clone()),
                ));
            }
        }
        Ok(rows)
    }

    /// The `(method, path)` pairs registered by `.route(...)` calls in
    /// `source`, skipping `any(...)` catch-alls.
    fn routes_in(source: &str) -> anyhow::Result<Vec<(String, String)>> {
        let mut routes = Vec::new();
        let mut rest = source;
        while let Some(at) = rest.find(".route(") {
            rest = &rest[at + ".route(".len()..];
            let mut depth = 1_usize;
            let mut end = None;
            for (index, ch) in rest.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(index);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let call = &rest[..end.context("unbalanced .route(")?];
            let path = call.split('"').nth(1).context("route without a path")?;
            let handlers = &call[call.find(',').context("route without a handler")?..];
            for method in ["get", "post", "put", "delete", "any"] {
                let needle = format!("{method}(");
                let mut search = handlers;
                while let Some(found) = search.find(&needle) {
                    let before = search[..found].chars().last();
                    if method != "any"
                        && before.is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_' || ch == ':'))
                    {
                        routes.push((method.to_ascii_uppercase(), path.to_owned()));
                    }
                    search = &search[found + needle.len()..];
                }
            }
        }
        Ok(routes)
    }

    /// The text of `fn name(...) { ... }` in `source`.
    fn function<'a>(source: &'a str, name: &str) -> anyhow::Result<&'a str> {
        let start = source
            .find(&format!("fn {name}("))
            .with_context(|| format!("main.rs has no fn {name}"))?;
        let body = &source[start..];
        let end = body.find("\n}\n").context("unterminated function")?;
        Ok(&body[..end])
    }

    #[test]
    fn the_documented_matrix_lists_exactly_the_registered_routes() -> anyhow::Result<()> {
        let mut registered: Vec<(String, String)> = routes_in(MAIN)?;
        registered.extend(routes_in(FLEET)?);
        let registered: BTreeSet<_> = registered.into_iter().collect();
        let documented: BTreeSet<_> = documented()?
            .into_iter()
            .map(|(method, path, _, _)| (method, path))
            .collect();
        let missing: Vec<_> = registered.difference(&documented).collect();
        let stale: Vec<_> = documented.difference(&registered).collect();
        assert!(
            missing.is_empty() && stale.is_empty(),
            "docs/fleet-api.md 'Who may call what' is out of date.\nnot documented: {missing:?}\ndocumented but not registered: {stale:?}"
        );
        Ok(())
    }

    #[test]
    fn the_documented_scopes_are_the_token_scope_table() -> anyhow::Result<()> {
        let documented: BTreeSet<(String, String, String)> = documented()?
            .into_iter()
            .filter_map(|(method, path, outcome, scope)| {
                assert_eq!(
                    scope.is_some(),
                    outcome == "accepted",
                    "{method} {path}: a scope is listed exactly when a token is accepted"
                );
                scope.map(|scope| (method, path, scope))
            })
            .collect();
        let table: BTreeSet<(String, String, String)> = crate::fleet::TOKEN_SCOPED_ROUTES
            .iter()
            .map(|(method, path, scope)| {
                (
                    (*method).to_owned(),
                    (*path).to_owned(),
                    (*scope).to_owned(),
                )
            })
            .collect();
        assert_eq!(documented, table);
        // Every scope named is one the token service knows.
        for (_, _, scope) in &table {
            assert!(ApiTokenScope::parse(scope).is_some(), "{scope}");
        }
        Ok(())
    }

    /// Each route outside the fleet router sits in the router group its
    /// documented outcome needs, and nothing is mounted outside the groups.
    #[test]
    fn every_route_sits_in_the_group_its_documented_outcome_needs() -> anyhow::Result<()> {
        let outcomes: std::collections::BTreeMap<(String, String), String> = documented()?
            .into_iter()
            .map(|(method, path, outcome, _)| ((method, path), outcome))
            .collect();
        let mut grouped = 0;
        for (group, expected) in [
            ("public_routes", "ignored"),
            ("token_routes", "accepted"),
            ("session_routes", "403 token_not_accepted"),
        ] {
            let routes = routes_in(function(MAIN, group)?)?;
            assert!(!routes.is_empty(), "{group} is empty");
            for route in routes {
                grouped += 1;
                assert_eq!(
                    outcomes.get(&route).map(String::as_str),
                    Some(expected),
                    "{route:?} is in {group}"
                );
            }
        }
        assert_eq!(
            grouped,
            routes_in(MAIN)?.len(),
            "every route in main.rs belongs to one of the three groups"
        );
        assert!(
            function(MAIN, "session_routes")?
                .contains("route_layer(middleware::from_fn(access::refuse_bearer))"),
            "the session group refuses bearer tokens"
        );
        Ok(())
    }

    #[tokio::test]
    async fn the_refusal_is_the_fleet_envelope_and_lets_everything_else_through()
    -> anyhow::Result<()> {
        use axum::{Router, body::Body, http::StatusCode, middleware, routing::get};
        use gridops_core::fleet::protocol::browser::FleetErrorResponse;
        use tower::ServiceExt as _;

        let router = Router::new()
            .route("/session", get(|| async { "handler ran" }))
            .route_layer(middleware::from_fn(refuse_bearer));
        for authorization in ["Bearer gat_whatever", "bearer x", "Bearer"] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/session")
                        .header("authorization", authorization)
                        .header("cookie", "gridops_session=present")
                        .body(Body::empty())?,
                )
                .await?;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(response.headers()["cache-control"], "no-store");
            let id = response.headers()["x-request-id"].to_str()?.to_owned();
            let body = axum::body::to_bytes(response.into_body(), 1024).await?;
            let error: FleetErrorResponse = serde_json::from_slice(&body)?;
            assert_eq!(error.code, ErrorCode::TokenNotAccepted);
            assert_eq!(error.request_id.to_string(), id);
            assert!(error.details.is_none());
        }
        for authorization in [None, Some("Basic cHJveHk6cGFzcw==")] {
            let mut request = Request::builder().uri("/session");
            if let Some(value) = authorization {
                request = request.header("authorization", value);
            }
            let response = router.clone().oneshot(request.body(Body::empty())?).await?;
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), 1024).await?;
            assert_eq!(body.as_ref(), b"handler ran");
        }
        Ok(())
    }
}
