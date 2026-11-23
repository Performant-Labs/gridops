//! `GET /api/v1/fleet/health` (#194): the fleet health page's one read.
//!
//! #197 wires it: the route, the admin check (D-4, the `GET
//! /api/v1/fleet/workload` pattern), the health state on `AppState` and its
//! default source. Until #194 lands the handler ignores `?window=`, has no
//! cache and always answers the 24-hour response; #194 adds the window
//! check (400), the 30-second per-window cache, the shared in-flight
//! computation and the stale-then-503 rules, all in this file. #204 owns
//! "the source selection" below ([`FleetHealthState::new`]): the fake source
//! is served until its database source (`health_source.rs`) is plugged in.

use std::{fmt, sync::Arc};

use axum::{Json, extract::State};
use gridops_core::{
    fleet::{
        protocol::browser::ErrorCode,
        registry::{RegistryClock, system_clock},
    },
    fleet_health::{FleetHealth, FleetHealthSource, HealthWindow, assemble, fake},
};

use super::{
    boundary::{FleetUser, Read},
    error::{FleetApiError, RequestContext},
    profiles::require_admin,
};
use crate::state::AppState;

/// The health endpoint's state on [`AppState`]: where the data comes from and
/// the trusted clock the cache reads (#194 adds the cache here).
#[derive(Clone)]
pub(crate) struct FleetHealthState {
    source: Arc<dyn FleetHealthSource>,
    clock: Arc<dyn RegistryClock>,
}

impl FleetHealthState {
    /// The production state: the source the page is served from, and the
    /// system clock.
    pub(crate) fn new() -> Self {
        Self {
            // #204: the database source replaces the fake here.
            source: Arc::new(fake::FakeFleetHealthSource::default()),
            clock: system_clock(),
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

/// The fleet's health over a window: scores, speed, usage, the live strip
/// and the areas of concern. Administrators only (D-4).
pub(super) async fn health(
    State(state): State<AppState>,
    context: RequestContext,
    user: FleetUser<Read>,
) -> Result<Json<FleetHealth>, FleetApiError> {
    require_admin(&state, &context, user.principal()).await?;
    let health = &state.fleet_health;
    let now = health.clock.now_millis();
    // #194 reads `?window=`, refuses anything else with a 400, and caches.
    let window = HealthWindow::Day;
    let unavailable = |error: anyhow::Error| {
        tracing::warn!(error = ?error, "fleet health source failed");
        context.error(ErrorCode::DependencyUnavailable)
    };
    let windowed = health
        .source
        .windowed(window, now)
        .await
        .map_err(unavailable)?;
    let live = health.source.live(now).await.map_err(unavailable)?;
    Ok(Json(assemble(window, now, windowed, live)))
}
