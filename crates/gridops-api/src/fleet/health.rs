//! `GET /api/v1/fleet/health` (#194): the fleet health page's one read.
//!
//! The handler reads the whole response from one [`FleetHealthSource`]
//! ([`fake::FakeFleetHealthSource`] until #204 plugs in its database source
//! through [`FleetHealthState::new`]), caches it per window for 30 seconds
//! (`CACHE_TTL_MILLIS`), and shares one computation among the concurrent
//! requests that find the window expired. When the source fails, the last
//! good response is served with `stale: true` while it is under 10 minutes
//! old (`STALE_MAX_AGE_MILLIS`), and a 503 otherwise. The route, the admin
//! check (D-4, the `GET /api/v1/fleet/workload` pattern) and the health
//! state on `AppState` are #197's.

use std::{collections::HashMap, fmt, str::FromStr, sync::Arc};

use axum::{Json, extract::State};
use gridops_core::{
    fleet::{
        UpstreamText,
        protocol::browser::{ErrorCode, ErrorDetails},
        registry::{RegistryClock, system_clock},
    },
    fleet_health::{FleetHealth, FleetHealthSource, HealthWindow, UnknownValue, assemble, fake},
};
use serde::Deserialize;
use tokio::sync::Mutex;

use super::{
    boundary::{FleetUser, Read},
    error::{FleetApiError, RequestContext},
    profiles::require_admin,
    query_boundary::FleetQuery,
};
use crate::state::AppState;

/// How long a cached response is served without reading the source again.
const CACHE_TTL_MILLIS: i64 = 30 * 1_000;

/// How long the last good response may be served stale after a source
/// failure. Measured from when that response was computed, not from the last
/// stale serve, so a source that stays down degrades to a 503.
const STALE_MAX_AGE_MILLIS: i64 = 10 * 60 * 1_000;

/// A cached response and when it was computed (the trusted clock's time).
struct CachedHealth {
    response: FleetHealth,
    computed_at: i64,
}

/// The per-window cache: a small map of window to (response, time). The
/// entries sit behind one mutex that is held across a recomputation, so
/// concurrent requests for an expired window wait for one computation
/// instead of each starting one.
#[derive(Default)]
struct HealthCache {
    entries: Mutex<HashMap<HealthWindow, CachedHealth>>,
}

/// The health endpoint's state on [`AppState`]: where the data comes from,
/// the trusted clock the cache reads, and the per-window cache.
#[derive(Clone)]
pub(crate) struct FleetHealthState {
    source: Arc<dyn FleetHealthSource>,
    clock: Arc<dyn RegistryClock>,
    cache: Arc<HealthCache>,
}

impl FleetHealthState {
    /// The production state: the source the page is served from, the system
    /// clock, and an empty cache.
    pub(crate) fn new() -> Self {
        Self {
            // #204: the database source replaces the fake here.
            source: Arc::new(fake::FakeFleetHealthSource::default()),
            clock: system_clock(),
            cache: Arc::new(HealthCache::default()),
        }
    }

    /// Replaces the source; tests inject `FakeFleetHealthSource`.
    #[must_use]
    pub(crate) fn with_source(mut self, source: Arc<dyn FleetHealthSource>) -> Self {
        self.source = source;
        self
    }

    /// Replaces the clock; tests move a `RegistryClock` of their own.
    #[must_use]
    pub(crate) fn with_clock(mut self, clock: Arc<dyn RegistryClock>) -> Self {
        self.clock = clock;
        self
    }
}

impl fmt::Debug for FleetHealthState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FleetHealthState")
            .finish_non_exhaustive()
    }
}

/// The health endpoint's query: only `window`, which defaults to 24 h.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct HealthParams {
    window: Option<String>,
}

/// The 400 for anything but the three allowed `window` spellings: the
/// bounded error contract carries the allowed values in the `InvalidField`
/// details (the pattern `profiles` and `api_tokens` use).
fn bad_window(context: RequestContext) -> FleetApiError {
    match UpstreamText::new("window: 1h, 24h, 7d") {
        Ok(field) => context.error_with_details(
            ErrorCode::InvalidRequest,
            ErrorDetails::InvalidField { field },
        ),
        Err(_) => context.error(ErrorCode::InvalidRequest),
    }
}

/// The window the request asks for: absent is the default (24 h), anything
/// but `1h`, `24h` or `7d` is a 400 naming the allowed values.
fn requested_window(
    context: RequestContext,
    window: Option<&str>,
) -> Result<HealthWindow, FleetApiError> {
    match window {
        None => Ok(HealthWindow::Day),
        Some(window) => {
            HealthWindow::from_str(window).map_err(|UnknownValue(_)| bad_window(context))
        }
    }
}

/// The fleet's health over a window: scores, speed, usage, the live strip
/// and the areas of concern. Administrators only (D-4).
pub(super) async fn health(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetUser<Read>,
    FleetQuery(params): FleetQuery<HealthParams>,
) -> Result<Json<FleetHealth>, FleetApiError> {
    require_admin(&state, &context, user.principal()).await?;
    let window = requested_window(context, params.window.as_deref())?;
    let health = &state.fleet_health;
    let mut entries = health.cache.entries.lock().await;
    let now = health.clock.now_millis();
    if let Some(entry) = entries.get(&window) {
        if now - entry.computed_at < CACHE_TTL_MILLIS {
            return Ok(Json(entry.response.clone()));
        }
    }
    match compute(health, window, now).await {
        Ok(response) => {
            let computed_at = response.generated_at;
            entries.insert(
                window,
                CachedHealth {
                    response: response.clone(),
                    computed_at,
                },
            );
            Ok(Json(response))
        }
        Err(error) => {
            tracing::warn!(error = ?error, "fleet health source failed");
            if let Some(entry) = entries.get(&window) {
                if now - entry.computed_at < STALE_MAX_AGE_MILLIS {
                    let mut stale = entry.response.clone();
                    stale.stale = true;
                    return Ok(Json(stale));
                }
            }
            Err(context.error(ErrorCode::DependencyUnavailable))
        }
    }
}

/// The one computation a cache miss needs: the windowed and live halves from
/// the source, joined by [`assemble`].
async fn compute(
    health: &FleetHealthState,
    window: HealthWindow,
    now: i64,
) -> anyhow::Result<FleetHealth> {
    let windowed = health.source.windowed(window, now).await?;
    let live = health.source.live(now).await?;
    Ok(assemble(window, now, windowed, live))
}
