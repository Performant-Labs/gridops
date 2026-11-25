//! `GET /api/v1/fleet/health` (#194): the fleet health page's one read.
//!
//! The handler reads the whole response from one [`FleetHealthSource`]: the
//! database source (`health_source::DbFleetHealthSource`, #204), which
//! [`FleetHealthState::new`] selects in production; tests inject
//! `fleet_health::fake`'s through `AppState::with_fleet_health_source`. It
//! caches the response per window for 30 seconds
//! (`CACHE_TTL_MILLIS`), and shares one computation among the concurrent
//! requests that find the window expired. When the source fails, the last
//! good response is served with `stale: true` while it is under 10 minutes
//! old (`STALE_MAX_AGE_MILLIS`), and a 503 otherwise. The route, the admin
//! check (D-4, the `GET /api/v1/fleet/workload` pattern) and the health
//! state on `AppState` are #197's.
//!
//! #240: each window has its own lock, held only by a computation of that
//! window, and a cache hit takes none of them, so a hit never waits on a
//! computation. A computation is given up on after 2 s
//! (`COMPUTE_TIMEOUT`), which counts as a failure, and after a failure the
//! window is not computed again for 30 s (`RETRY_AFTER_FAILURE_MILLIS`): it
//! serves stale or 503 meanwhile. The live part is read once per
//! computation, and misses on several windows at once share the one live
//! reading in flight; nothing of it is kept once it is read.

use std::{
    fmt,
    str::FromStr,
    sync::{
        Arc, Mutex as SyncMutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::{Json, extract::State};
use futures_util::{
    FutureExt as _,
    future::{BoxFuture, Shared},
};
use gridops_core::{
    fleet::{
        UpstreamText,
        protocol::browser::{ErrorCode, ErrorDetails},
        registry::{RegistryClock, system_clock},
    },
    fleet_health::{
        FleetHealth, FleetHealthSource, HealthWindow, LiveHealth, UnknownValue, assemble,
    },
};
use serde::Deserialize;
use sqlx::SqlitePool;
use tokio::sync::Mutex;

use super::{
    boundary::{FleetUser, Read},
    error::{FleetApiError, RequestContext},
    health_source::DbFleetHealthSource,
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

/// How long one computation may take before it is given up on and counted
/// as a failure (a source that hangs, as SQLite does in a #179 lock storm).
const COMPUTE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long after a failure (or a timeout) a window waits before it calls
/// the source again, serving stale or 503 meanwhile.
const RETRY_AFTER_FAILURE_MILLIS: i64 = 30 * 1_000;

/// A cached response and when it was computed (the trusted clock's time).
struct CachedHealth {
    response: FleetHealth,
    computed_at: i64,
}

/// What a window remembers: its last good response and its last failure.
#[derive(Default)]
struct WindowEntry {
    last_good: Option<CachedHealth>,
    failed_at: Option<i64>,
}

impl WindowEntry {
    /// The answer without calling the source: the fresh response, or within
    /// 30 s of a failure what a failure serves. `None`: compute.
    fn answer(&self, now: i64) -> Option<Result<FleetHealth, ()>> {
        if let Some(entry) = &self.last_good {
            if now - entry.computed_at < CACHE_TTL_MILLIS {
                return Some(Ok(entry.response.clone()));
            }
        }
        match self.failed_at {
            Some(failed_at) if now - failed_at < RETRY_AFTER_FAILURE_MILLIS => {
                Some(self.after_failure(now))
            }
            _ => None,
        }
    }

    /// What a failure serves: the last good response marked stale while it
    /// is under 10 minutes old, otherwise nothing (a 503).
    fn after_failure(&self, now: i64) -> Result<FleetHealth, ()> {
        match &self.last_good {
            Some(entry) if now - entry.computed_at < STALE_MAX_AGE_MILLIS => {
                let mut stale = entry.response.clone();
                stale.stale = true;
                Ok(stale)
            }
            _ => Err(()),
        }
    }
}

/// One window's cache. `entry` is only locked to read or write it, never
/// across an await, so a hit never waits; `computing` is held across one
/// computation of this window, so concurrent misses share it, and a miss on
/// one window never holds up another.
#[derive(Default)]
struct WindowCache {
    entry: SyncMutex<WindowEntry>,
    computing: Mutex<()>,
}

impl WindowCache {
    fn entry(&self) -> std::sync::MutexGuard<'_, WindowEntry> {
        self.entry.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A live reading the source is taking, shared by every computation that
/// asks for one meanwhile.
type LiveReading = Shared<BoxFuture<'static, Result<Arc<LiveHealth>, Arc<anyhow::Error>>>>;

/// The live reading in flight, if any. The slot is cleared as soon as a
/// reader finishes (or gives up), so a reading is never served to a
/// computation that starts after it: the live part is not cached.
#[derive(Default)]
struct LiveFlight {
    current: SyncMutex<Option<(u64, LiveReading)>>,
    next: AtomicU64,
}

/// Clears the flight's slot when its reader is done with reading `id`,
/// whether it finished or was dropped by the timeout.
struct ClearFlight<'a> {
    flight: &'a LiveFlight,
    id: u64,
}

impl Drop for ClearFlight<'_> {
    fn drop(&mut self) {
        let mut current = self
            .flight
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if current.as_ref().is_some_and(|(id, _)| *id == self.id) {
            *current = None;
        }
    }
}

impl LiveFlight {
    /// The live part at `now`: joins the reading in flight, or starts one.
    async fn read(
        &self,
        source: &Arc<dyn FleetHealthSource>,
        now: i64,
    ) -> anyhow::Result<LiveHealth> {
        let (id, reading) = {
            let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((id, reading)) = current.as_ref() {
                (*id, reading.clone())
            } else {
                let id = self.next.fetch_add(1, Ordering::Relaxed);
                let source = Arc::clone(source);
                let reading = async move { source.live(now).await.map(Arc::new).map_err(Arc::new) }
                    .boxed()
                    .shared();
                *current = Some((id, reading.clone()));
                (id, reading)
            }
        };
        let _clear = ClearFlight { flight: self, id };
        match reading.await {
            Ok(live) => Ok(LiveHealth::clone(&live)),
            Err(error) => Err(anyhow::anyhow!("{error:#}")),
        }
    }
}

/// The per-window caches and the shared live reading.
#[derive(Default)]
struct HealthCache {
    hour: WindowCache,
    day: WindowCache,
    week: WindowCache,
    live: LiveFlight,
}

impl HealthCache {
    fn window(&self, window: HealthWindow) -> &WindowCache {
        match window {
            HealthWindow::Hour => &self.hour,
            HealthWindow::Day => &self.day,
            HealthWindow::Week => &self.week,
        }
    }
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
    /// The production state: the database source over `db` (#204), the
    /// system clock, and an empty cache.
    pub(crate) fn new(db: SqlitePool) -> Self {
        Self {
            source: Arc::new(DbFleetHealthSource::new(db)),
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
    let cache = health.cache.window(window);
    let unavailable = || context.error(ErrorCode::DependencyUnavailable);
    if let Some(answer) = cache.entry().answer(health.clock.now_millis()) {
        return answer.map(Json).map_err(|()| unavailable());
    }
    // A miss: one computation per window at a time. Whoever waited here
    // looks again first, since the one before may have just answered.
    let _computing = cache.computing.lock().await;
    let now = health.clock.now_millis();
    if let Some(answer) = cache.entry().answer(now) {
        return answer.map(Json).map_err(|()| unavailable());
    }
    let computed = match tokio::time::timeout(COMPUTE_TIMEOUT, compute(health, window, now)).await {
        Ok(computed) => computed,
        Err(_) => Err(anyhow::anyhow!(
            "the fleet health source took over {COMPUTE_TIMEOUT:?}"
        )),
    };
    let mut entry = cache.entry();
    match computed {
        Ok(response) => {
            entry.last_good = Some(CachedHealth {
                response: response.clone(),
                computed_at: response.generated_at,
            });
            entry.failed_at = None;
            Ok(Json(response))
        }
        Err(error) => {
            tracing::warn!(error = ?error, "fleet health source failed");
            // When it failed (up to 2 s after it started): the 30 s, and the
            // stale response's age, count from now.
            let failed_at = health.clock.now_millis();
            entry.failed_at = Some(failed_at);
            entry
                .after_failure(failed_at)
                .map(Json)
                .map_err(|()| unavailable())
        }
    }
}

/// The one computation a cache miss needs: the windowed half from the
/// source and the live half read once (shared with the other windows'
/// computations in flight), joined by [`assemble`].
async fn compute(
    health: &FleetHealthState,
    window: HealthWindow,
    now: i64,
) -> anyhow::Result<FleetHealth> {
    let (windowed, live) = tokio::try_join!(
        health.source.windowed(window, now),
        health.cache.live.read(&health.source, now),
    )?;
    Ok(assemble(window, now, windowed, live))
}

#[cfg(test)]
mod tests {
    //! #240: the cache's locking, timeout, failure memory and live sharing,
    //! against the fake source (`hold` is a source that hangs, `set_failing`
    //! one that fails) and the health state's clock.

    use std::{sync::Arc, time::Duration};

    use anyhow::Result;
    use axum::http::StatusCode;
    use serde_json::{Value, json};

    use super::super::health_test_support::{HEALTH, Harness, error_code, fixture};

    const SECOND: i64 = 1_000;

    /// #204: production's state serves the database, not the fixture: the
    /// seeded machines, by id and name, at the clock's time. Nothing of the
    /// response comes from `fleet_health::fake`, which release builds no
    /// longer compile.
    #[tokio::test]
    async fn production_serves_the_database_not_the_fixture() -> Result<()> {
        let harness = Harness::with_database_source().await?;
        let fleet = gridops_core::fleet_health::testdata::seed_fleet(&harness.database).await?;
        let (status, body) = harness.admin_get(&uri("24h")).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_ne!(body, fixture()?, "not the fake's fixture");
        let hosts: Vec<(String, String)> = body["hosts"]
            .as_array()
            .map(|hosts| {
                hosts
                    .iter()
                    .map(|host| {
                        (
                            host["id"].as_str().unwrap_or_default().to_owned(),
                            host["name"].as_str().unwrap_or_default().to_owned(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut seeded: Vec<(String, String)> = fleet
            .hosts
            .iter()
            .map(|host| (host.id.clone(), host.name.clone()))
            .collect();
        seeded.sort();
        assert_eq!(hosts, seeded, "the seeded machines, in id order");
        assert_eq!(
            body.get("generatedAt").and_then(Value::as_i64),
            Some(gridops_core::fleet_health::fake::FIXTURE_NOW)
        );
        assert_eq!(body["fleet"]["trend"], json!([]), "no snapshot yet");
        assert_eq!(harness.source.windowed_calls(), 0, "the fake is not called");
        Ok(())
    }

    fn uri(window: &str) -> String {
        format!("{HEALTH}?window={window}")
    }

    /// `harness.admin_get(uri)` on its own task, so a test can hold it.
    fn spawn_get(
        harness: &Arc<Harness>,
        window: &str,
    ) -> tokio::task::JoinHandle<Result<(StatusCode, Value)>> {
        let harness = Arc::clone(harness);
        let uri = uri(window);
        tokio::spawn(async move { harness.admin_get(&uri).await })
    }

    /// Item 1: a hit on a cached window answers while another window's
    /// computation (its windowed and its live read) hangs.
    #[tokio::test]
    async fn a_cache_hit_does_not_wait_on_another_windows_computation() -> Result<()> {
        let harness = Arc::new(Harness::new().await?);
        let (status, _) = harness.admin_get(&uri("1h")).await?;
        assert_eq!(status, StatusCode::OK);
        harness.source.hold();
        let week = spawn_get(&harness, "7d");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let (status, body) =
            tokio::time::timeout(Duration::from_millis(500), harness.admin_get(&uri("1h")))
                .await
                .map_err(|_| anyhow::anyhow!("the 1h hit waited on the 7d computation"))??;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body.get("stale"), Some(&json!(false)));
        harness.source.release();
        let (status, body) = tokio::time::timeout(Duration::from_secs(10), week).await???;
        assert_eq!(status, StatusCode::OK, "{body}");
        Ok(())
    }

    /// Item 1, for a miss (#245): a miss on one window starts its own
    /// computation while another window's computation hangs, instead of
    /// queueing behind it. With one computation lock for every window the
    /// 1h miss could only call the source once the 7d computation had given
    /// up, 2 s after it started.
    #[tokio::test]
    async fn a_miss_does_not_wait_on_another_windows_computation() -> Result<()> {
        let harness = Arc::new(Harness::new().await?);
        harness.source.hold();
        let started = std::time::Instant::now();
        let week = spawn_get(&harness, "7d");
        tokio::time::sleep(Duration::from_millis(100)).await;
        let hour = spawn_get(&harness, "1h");
        while harness.source.windowed_calls() < 2 && started.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let waited = started.elapsed();
        assert_eq!(harness.source.windowed_calls(), 2);
        assert!(
            waited < Duration::from_millis(1_500),
            "the 1h miss called the source {waited:?} after the 7d one: it waited on it"
        );
        harness.source.release();
        for call in [week, hour] {
            let (status, body) = tokio::time::timeout(Duration::from_secs(10), call).await???;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        Ok(())
    }

    /// Item 2: a source that hangs is given up on after 2 s and counts as a
    /// failure: the last good response is served stale (or a 503 when there
    /// is none), and the source is not called again for 30 s.
    #[tokio::test]
    async fn a_hanging_source_times_out_and_is_not_called_again_for_30_seconds() -> Result<()> {
        let harness = Arc::new(Harness::new().await?);
        let (status, fresh) = harness.admin_get(&uri("24h")).await?;
        assert_eq!(status, StatusCode::OK);
        harness.clock.advance(31 * SECOND);
        harness.source.hold();

        let started = std::time::Instant::now();
        let (status, stale) =
            tokio::time::timeout(Duration::from_secs(5), harness.admin_get(&uri("24h")))
                .await
                .map_err(|_| anyhow::anyhow!("the request hung with the source"))??;
        let waited = started.elapsed();
        assert_eq!(status, StatusCode::OK, "{stale}");
        assert_eq!(stale.get("stale"), Some(&json!(true)));
        let mut unmarked = stale.clone();
        unmarked["stale"] = json!(false);
        assert_eq!(unmarked, fresh, "the last good response, marked stale");
        assert!(
            waited >= Duration::from_millis(1_900) && waited < Duration::from_secs(4),
            "given up on after 2 s, not {waited:?}"
        );
        assert_eq!(harness.source.windowed_calls(), 2);

        // Within 30 s of the timeout the source is not called again.
        harness.clock.advance(29 * SECOND);
        let (status, body) =
            tokio::time::timeout(Duration::from_millis(500), harness.admin_get(&uri("24h")))
                .await
                .map_err(|_| anyhow::anyhow!("the source was called again within 30 s"))??;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body.get("stale"), Some(&json!(true)));
        assert_eq!(harness.source.windowed_calls(), 2);

        // No earlier response and a hanging source: a 503 after 2 s.
        let (status, body) =
            tokio::time::timeout(Duration::from_secs(5), harness.admin_get(&uri("1h")))
                .await
                .map_err(|_| anyhow::anyhow!("the request hung with the source"))??;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert_eq!(error_code(&body), Some("dependency_unavailable"));

        // 30 s after the timeout the source is tried again.
        harness.source.release();
        harness.clock.advance(SECOND);
        let (status, body) = harness.admin_get(&uri("24h")).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body.get("stale"), Some(&json!(false)));
        assert_eq!(harness.source.windowed_calls(), 4);
        Ok(())
    }

    /// Item 2 (PR-Agent c16c0a3c7aaa): the 30 s are counted from when the
    /// computation failed, not from when it started: a computation that
    /// hangs for its 2 s is not tried again 28 s later.
    #[tokio::test]
    async fn the_30_seconds_count_from_the_failure_not_the_start() -> Result<()> {
        let harness = Arc::new(Harness::new().await?);
        let (status, _) = harness.admin_get(&uri("24h")).await?;
        assert_eq!(status, StatusCode::OK);
        harness.clock.advance(31 * SECOND);
        harness.source.hold();
        let hung = spawn_get(&harness, "24h");
        // The trusted clock moves on while the source hangs.
        tokio::time::sleep(Duration::from_millis(300)).await;
        harness.clock.advance(2 * SECOND);
        let (status, body) = tokio::time::timeout(Duration::from_secs(5), hung).await???;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body.get("stale"), Some(&json!(true)));
        assert_eq!(harness.source.windowed_calls(), 2);
        harness.source.release();

        // 29 s after the timeout (31 s after the computation started).
        harness.clock.advance(29 * SECOND);
        let (status, body) = harness.admin_get(&uri("24h")).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body.get("stale"), Some(&json!(true)));
        assert_eq!(harness.source.windowed_calls(), 2, "not tried again yet");
        harness.clock.advance(SECOND);
        let (status, body) = harness.admin_get(&uri("24h")).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body.get("stale"), Some(&json!(false)));
        assert_eq!(harness.source.windowed_calls(), 3);
        Ok(())
    }

    /// Item 2: during an outage the failing source is called once per 30 s
    /// per window, not once per request; meanwhile the window serves stale
    /// or 503, as on the failing call itself.
    #[tokio::test]
    async fn a_failing_source_is_not_called_again_for_30_seconds() -> Result<()> {
        let harness = Harness::new().await?;
        let (status, _) = harness.admin_get(&uri("24h")).await?;
        assert_eq!(status, StatusCode::OK);
        harness.clock.advance(31 * SECOND);
        harness.source.set_failing(true);
        let (status, body) = harness.admin_get(&uri("24h")).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body.get("stale"), Some(&json!(true)));
        assert_eq!(harness.source.windowed_calls(), 2);

        for _ in 0..3 {
            harness.clock.advance(9 * SECOND);
            let (status, body) = harness.admin_get(&uri("24h")).await?;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body.get("stale"), Some(&json!(true)));
        }
        assert_eq!(
            harness.source.windowed_calls(),
            2,
            "27 s after the failure: not called again"
        );
        harness.clock.advance(3 * SECOND);
        let (status, body) = harness.admin_get(&uri("24h")).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body.get("stale"), Some(&json!(true)));
        assert_eq!(harness.source.windowed_calls(), 3, "30 s on: tried again");

        // A window with no response: a 503, and the same 503 without a call
        // for the next 30 s.
        let (status, body) = harness.admin_get(&uri("1h")).await?;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert_eq!(harness.source.windowed_calls(), 4);
        harness.clock.advance(10 * SECOND);
        let (status, body) = harness.admin_get(&uri("1h")).await?;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert_eq!(error_code(&body), Some("dependency_unavailable"));
        assert_eq!(harness.source.windowed_calls(), 4);

        // Once the source recovers, the next try after the 30 s is fresh.
        harness.source.set_failing(false);
        harness.clock.advance(20 * SECOND);
        let (status, body) = harness.admin_get(&uri("1h")).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body.get("stale"), Some(&json!(false)));
        Ok(())
    }

    /// Item 3: misses on the three windows at once share one live reading;
    /// one request that misses reads it exactly once; and nothing of it is
    /// kept for a later request.
    #[tokio::test]
    async fn concurrent_misses_on_every_window_share_one_live_reading() -> Result<()> {
        let harness = Arc::new(Harness::new().await?);
        harness.source.hold();
        let calls: Vec<_> = ["1h", "24h", "7d"]
            .into_iter()
            .map(|window| spawn_get(&harness, window))
            .collect();
        tokio::time::sleep(Duration::from_millis(200)).await;
        harness.source.release();
        for call in calls {
            let (status, body) = tokio::time::timeout(Duration::from_secs(10), call).await???;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        assert_eq!(harness.source.windowed_calls(), 3, "one per window");
        assert_eq!(
            harness.source.live_calls(),
            1,
            "one live reading for all three"
        );

        // Later misses read the live part again, once each.
        harness.clock.advance(31 * SECOND);
        harness.admin_get(&uri("24h")).await?;
        assert_eq!(harness.source.live_calls(), 2);
        harness.admin_get(&uri("7d")).await?;
        assert_eq!(harness.source.live_calls(), 3, "not cached across requests");
        Ok(())
    }
}
