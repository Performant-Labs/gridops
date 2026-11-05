mod access;
mod agent;
mod auth;
mod error;
mod fleet;
mod github_app;
mod install;
mod oauth;
mod resources;
mod state;
mod webhooks;

use std::time::Duration;

use anyhow::Result;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderName, HeaderValue, Method, Request, StatusCode, header},
    middleware,
    routing::{any, delete, get, post, put},
};
use gridops_core::{BitbucketClient, Config, GitHubClient, Vault, connect_database};
use tower::ServiceBuilder;
use tower_http::{
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    cors::CorsLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _};

use crate::state::AppState;

/// Clears this deployment's default images from the providers runner pools do
/// not use (issue #103; migration 0044 covers the built-in defaults only). A
/// failure is logged and the API starts anyway: the API already ignores such
/// an image and drops it on the pool's next edit.
async fn clear_unused_default_images(database: &sqlx::SqlitePool, config: &Config) {
    match gridops_core::db::clear_unused_default_images(
        database,
        config.runner_image(),
        &resources::default_tart_image(),
    )
    .await
    {
        Ok(0) => tracing::debug!(
            "no runner pool holds this deployment's default image for a provider it does not use"
        ),
        Ok(pools) => tracing::info!(
            pools,
            "cleared this deployment's default images from providers runner pools do not use"
        ),
        Err(error) => tracing::warn!(
            %error,
            "could not clear this deployment's default images from providers runner pools do not use"
        ),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "gridops=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env()?;
    config.validate_api()?;
    let database = connect_database(&config).await?;
    gridops_core::fleet::ensure_control_plane(&database).await?;
    clear_unused_default_images(&database, &config).await;
    let vault = Vault::from_config(&config)?;
    let github = GitHubClient::new(config.clone())?;
    let bitbucket = BitbucketClient::new()?;
    let state = AppState::new(config.clone(), database, vault, github, bitbucket)?;
    state.validate_api().await?;
    tokio::spawn({
        let state = state.clone();
        async move { state.follow_github_app_rename().await }
    });
    let request_id = HeaderName::from_static("x-request-id");
    let origin: HeaderValue = config.base_url().origin().ascii_serialization().parse()?;
    let web_root = config.web_root();
    let assets = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        ))
        .service(ServeDir::new(web_root.join("assets")));
    // The page names hashed asset files, so it must be revalidated or a
    // browser keeps running the previous release.
    let spa = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        .service(
            ServeDir::new(web_root)
                .append_index_html_on_directories(true)
                .fallback(ServeFile::new(web_root.join("index.html"))),
        );

    let app = Router::new()
        .merge(fleet::router())
        .merge(fleet::operation_router())
        .merge(public_routes())
        .merge(token_routes())
        .merge(session_routes())
        .route("/api/{*path}", any(not_found))
        .route("/auth/{*path}", any(not_found))
        .nest_service("/assets", assets)
        .fallback_service(spa)
        .with_state(state)
        .layer(DefaultBodyLimit::max(25 * 1024 * 1024))
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
        .layer(SetSensitiveRequestHeadersLayer::new([
            header::AUTHORIZATION,
            header::COOKIE,
        ]))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(CompressionLayer::new())
        .layer(
            ServiceBuilder::new()
                .layer(SetResponseHeaderLayer::overriding(
                    HeaderName::from_static("content-security-policy"),
                    HeaderValue::from_static(
                        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: https://avatars.githubusercontent.com https://github.com; connect-src 'self'; font-src 'self' data:; frame-ancestors 'none'; base-uri 'self'; form-action 'self' https://github.com",
                    ),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    HeaderName::from_static("cross-origin-opener-policy"),
                    HeaderValue::from_static("same-origin"),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    HeaderName::from_static("permissions-policy"),
                    HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    header::REFERRER_POLICY,
                    HeaderValue::from_static("strict-origin-when-cross-origin"),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    header::X_CONTENT_TYPE_OPTIONS,
                    HeaderValue::from_static("nosniff"),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    HeaderName::from_static("x-frame-options"),
                    HeaderValue::from_static("DENY"),
                )),
        )
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(origin)
                .allow_credentials(true)
                .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
                .allow_headers([
                    header::CONTENT_TYPE,
                    HeaderName::from_static("x-csrf-token"),
                    HeaderName::from_static("idempotency-key"),
                ])
                .expose_headers([HeaderName::from_static("x-request-id")]),
        );

    let listener = tokio::net::TcpListener::bind(config.api_bind()).await?;
    tracing::info!(
        address = %config.api_bind(),
        version = gridops_core::build_info::FULL_VERSION,
        commit = gridops_core::build_info::COMMIT,
        "GridOps Rust API listening"
    );
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

/// Routes that never authenticate a user; an `Authorization` header has no
/// effect on them. See "Who may call what" in `docs/fleet-api.md`.
fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/api/health", get(resources::health))
        .route("/api/v1/openapi.json", get(fleet::openapi::get))
        .route("/install.sh", get(install::shell))
        .route("/api/v1/auth/github", get(oauth::begin))
        .route("/api/v1/auth/github/callback", get(oauth::callback))
        .route("/api/v1/webhooks/github", post(webhooks::receive))
        .route("/auth/github", get(oauth::begin))
        .route("/auth/github/callback", get(oauth::callback))
        .route(
            "/auth/github-app/manifest/callback",
            get(github_app::manifest_callback),
        )
        .route("/api/webhooks/github", post(webhooks::receive))
}

/// Runner-pool routes: a browser session exactly as before, or a scoped API
/// token (`pools:read`, `pools:manage`) through the `access` extractors.
fn token_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/overview/capacity",
            get(resources::capacity_history),
        )
        .route("/api/v1/overview/tart-host", get(resources::tart_host))
        .route(
            "/api/v1/runner-pools",
            get(resources::runner_pools).post(resources::create_runner_pool),
        )
        .route(
            "/api/v1/runner-pools/options",
            get(resources::runner_pool_options),
        )
        .route(
            "/api/v1/runner-pools/repositories",
            get(resources::runner_pool_repository_options),
        )
        .route(
            "/api/v1/installations/{installation_id}/runner-groups",
            get(resources::installation_runner_groups),
        )
        .route(
            "/api/v1/installations/{installation_id}/repositories",
            get(resources::installation_repositories),
        )
        .route(
            "/api/v1/runner-pools/{pool_id}",
            get(resources::runner_pool)
                .put(resources::update_runner_pool)
                .delete(resources::delete_runner_pool),
        )
        .route(
            "/api/v1/runner-pools/{pool_id}/events",
            get(resources::runner_pool_events),
        )
}

/// Routes that need a signed-in browser session and take no API token. A
/// bearer token is refused with `403 token_not_accepted` before the handler
/// runs, so it is never answered as if the caller were signed out.
fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/session", get(auth::session))
        .route("/api/v1/auth/me", get(auth::me))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/users/{user_id}/role", put(auth::update_user_role))
        .route("/auth/logout", post(auth::logout))
        .route("/api/v1/overview", get(resources::overview))
        .route("/api/v1/overview/queued-jobs", get(resources::queued_jobs))
        .route("/api/v1/search", get(resources::search))
        .route("/api/v1/repositories", get(resources::repositories))
        .route("/api/v1/repositories/sync", post(oauth::sync))
        .route(
            "/api/v1/runner-pools/{pool_id}/action",
            post(resources::runner_pool_action),
        )
        .route("/api/v1/runners", get(resources::runners))
        .route(
            "/api/v1/runners/{runner_id}/action",
            post(resources::runner_action),
        )
        .route(
            "/api/v1/runners/{runner_id}/logs",
            get(resources::runner_logs),
        )
        .route(
            "/api/v1/runners/{runner_id}/logs/stream",
            get(resources::runner_log_stream),
        )
        .route("/api/v1/workflow-runs", get(resources::workflow_runs))
        .route(
            "/api/v1/workflow-runs/{run_id}",
            get(resources::workflow_run),
        )
        .route(
            "/api/v1/workflow-runs/{run_id}/logs",
            get(resources::workflow_run_logs),
        )
        .route(
            "/api/v1/workflow-runs/{run_id}/action",
            post(resources::workflow_run_action),
        )
        .route(
            "/api/v1/workflow-jobs/{job_id}/logs",
            get(resources::workflow_job_log_view),
        )
        .route(
            "/api/v1/workflow-jobs/{job_id}/agent-runs",
            post(agent::start_agent_run),
        )
        .route("/api/v1/agent-runs", get(agent::agent_runs))
        .route("/api/v1/agent-runs/{run_id}", get(agent::agent_run))
        .route(
            "/api/v1/agent-runs/{run_id}/cancel",
            post(agent::cancel_agent_run),
        )
        .route("/api/v1/settings/ai", get(agent::ai_settings))
        .route("/api/v1/settings/ai/api-key", post(agent::save_api_key))
        .route("/api/v1/settings/ai/oauth/start", post(agent::start_oauth))
        .route(
            "/api/v1/settings/ai/oauth/complete",
            post(agent::complete_oauth),
        )
        .route(
            "/api/v1/settings/ai/connections/{connection_id}",
            delete(agent::delete_connection),
        )
        .route(
            "/api/v1/settings/ai/connections/{connection_id}/models",
            get(agent::connection_models),
        )
        .route("/api/v1/settings/ai/agent", put(agent::save_agent_settings))
        .route(
            "/api/workflow-runs/{run_id}/logs",
            get(resources::workflow_run_logs),
        )
        .route("/api/v1/webhooks", get(resources::webhook_deliveries))
        .route(
            "/api/v1/webhooks/{delivery_id}",
            get(resources::webhook_delivery),
        )
        .route(
            "/api/v1/webhooks/{delivery_id}/retry",
            post(webhooks::retry),
        )
        .route(
            "/api/v1/webhooks/dismiss-failed",
            post(webhooks::dismiss_failed),
        )
        .route("/api/v1/audit", get(resources::audit_events))
        .route("/api/v1/log-streams", get(resources::log_targets))
        .route(
            "/api/v1/log-streams/{stream_id}/logs",
            get(resources::archived_logs),
        )
        .route(
            "/api/v1/settings",
            get(resources::settings).put(resources::save_settings),
        )
        .route(
            "/api/v1/settings/runner-host",
            put(resources::save_builtin_runner_host),
        )
        .route(
            "/api/v1/settings/runner-host/budget",
            put(resources::save_builtin_runner_host_budget),
        )
        .route(
            "/api/v1/platform-connections/bitbucket",
            get(resources::bitbucket_connections).post(resources::create_bitbucket_connection),
        )
        .route(
            "/api/v1/platform-connections/bitbucket/{connection_id}",
            put(resources::update_bitbucket_connection)
                .delete(resources::delete_bitbucket_connection),
        )
        .route(
            "/api/v1/github-app/manifest",
            post(github_app::create_manifest),
        )
        .route("/api/v1/backups/database", get(resources::database_backup))
        .route("/api/backups/database", get(resources::database_backup))
        .route_layer(middleware::from_fn(access::refuse_bearer))
}

async fn not_found(_: Request<axum::body::Body>) -> StatusCode {
    StatusCode::NOT_FOUND
}
