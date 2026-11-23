//! Contract tests for #194 (`GET /api/v1/fleet/health`), written by #197.
//!
//! #194 removes the `#[ignore = "contract: #194"]` markers and makes these
//! pass; it changes nothing else here (`Fleet health guard`). Against the
//! fake source and the health state's clock (`health_test_support.rs`):
//! - the response is the `OpenAPI` example (the fixture) for each window,
//!   with `window` set to the one asked for;
//! - a bad window is a 400 naming the allowed values; no session is refused
//!   exactly as `/fleet/hosts` refuses it; a non-administrator gets 403
//!   (D-4);
//! - the response is cached per window for 30 s; concurrent requests for an
//!   empty window share one computation; a failing source serves the last
//!   response with `"stale": true` while it is under 10 minutes old, and a
//!   503 after that.
//!
//! Mounted from `fleet/mod.rs` (#197-owned).

use std::time::Duration;

use anyhow::{Context as _, Result};
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::health_test_support::{HEALTH, Harness, error_code, fixture, openapi};

const SECOND: i64 = 1_000;
const MINUTE: i64 = 60 * SECOND;

/// The `OpenAPI` example for `window`.
fn expected(window: &str) -> Result<Value> {
    let spec = openapi()?;
    let mut example = spec
        .pointer(
            "/paths/~1api~1v1~1fleet~1health/get/responses/200/content/application~1json/example",
        )
        .cloned()
        .context("GET /api/v1/fleet/health has no 200 example")?;
    example["window"] = json!(window);
    Ok(example)
}

#[tokio::test]
async fn the_response_equals_the_openapi_example_for_each_window() -> Result<()> {
    let harness = Harness::new().await?;
    assert_eq!(expected("24h")?, fixture()?);
    for window in ["1h", "24h", "7d"] {
        let (status, body) = harness
            .admin_get(&format!("{HEALTH}?window={window}"))
            .await?;
        assert_eq!(status, StatusCode::OK, "{window}: {body}");
        assert_eq!(body, expected(window)?, "{window}");
    }
    Ok(())
}

#[tokio::test]
async fn a_bad_window_is_a_400_and_callers_without_access_are_refused() -> Result<()> {
    let harness = Harness::new().await?;
    for bad in ["30d", "1d", "24H", ""] {
        let (status, body) = harness.admin_get(&format!("{HEALTH}?window={bad}")).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "window={bad}: {body}");
        let text = body.to_string();
        for allowed in ["1h", "24h", "7d"] {
            assert!(text.contains(allowed), "the 400 names {allowed}: {body}");
        }
    }
    assert_eq!(
        harness.source.windowed_calls(),
        0,
        "a bad window reads nothing"
    );

    // No session: refused exactly the way /fleet/hosts refuses it.
    let (status, body) = harness
        .get_as(&format!("{HEALTH}?window=24h"), None)
        .await?;
    let (hosts_status, hosts_body) = harness.get_as("/api/v1/fleet/hosts", None).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(status, hosts_status);
    assert_eq!(error_code(&body), error_code(&hosts_body));
    assert_eq!(error_code(&body), Some("unauthenticated"));

    // A member is not an administrator.
    let member = harness.member.clone();
    let (status, body) = harness
        .get_as(&format!("{HEALTH}?window=7d"), Some(&member))
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(error_code(&body), Some("forbidden"));
    assert_eq!(
        harness.source.windowed_calls(),
        0,
        "a refused caller reads nothing"
    );
    Ok(())
}

#[tokio::test]
async fn a_second_call_within_30_seconds_does_not_call_the_source() -> Result<()> {
    let harness = Harness::new().await?;
    let uri = format!("{HEALTH}?window=24h");
    let (status, first) = harness.admin_get(&uri).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (harness.source.windowed_calls(), harness.source.live_calls()),
        (1, 1)
    );
    harness.clock.advance(29 * SECOND);
    let (status, second) = harness.admin_get(&uri).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second, first);
    assert_eq!(
        (harness.source.windowed_calls(), harness.source.live_calls()),
        (1, 1),
        "served from the cache"
    );
    // The cache is per window.
    harness.admin_get(&format!("{HEALTH}?window=7d")).await?;
    assert_eq!(harness.source.windowed_calls(), 2);
    // Past 30 s the window is computed again.
    harness.clock.advance(2 * SECOND);
    harness.admin_get(&uri).await?;
    assert_eq!(harness.source.windowed_calls(), 3);
    Ok(())
}

#[tokio::test]
async fn ten_concurrent_calls_on_an_empty_cache_call_the_source_once() -> Result<()> {
    let harness = std::sync::Arc::new(Harness::new().await?);
    harness.source.hold();
    let mut calls = Vec::new();
    for _ in 0..10 {
        let harness = std::sync::Arc::clone(&harness);
        calls.push(tokio::spawn(async move {
            harness.admin_get(&format!("{HEALTH}?window=1h")).await
        }));
    }
    // Let every request reach the handler while the first computation waits.
    tokio::time::sleep(Duration::from_millis(200)).await;
    harness.source.release();
    let mut bodies = Vec::new();
    for call in calls {
        let (status, body) = tokio::time::timeout(Duration::from_secs(10), call).await???;
        assert_eq!(status, StatusCode::OK, "{body}");
        bodies.push(body);
    }
    assert_eq!(
        harness.source.windowed_calls(),
        1,
        "one computation for all ten"
    );
    assert_eq!(harness.source.live_calls(), 1);
    assert!(bodies.windows(2).all(|pair| pair[0] == pair[1]));
    Ok(())
}

#[tokio::test]
async fn a_failing_source_serves_the_stale_response_then_a_503_after_10_minutes() -> Result<()> {
    let harness = Harness::new().await?;
    let uri = format!("{HEALTH}?window=24h");
    let (status, fresh) = harness.admin_get(&uri).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fresh.get("stale"), Some(&json!(false)));

    harness.source.set_failing(true);
    harness.clock.advance(31 * SECOND);
    let (status, stale) = harness.admin_get(&uri).await?;
    assert_eq!(status, StatusCode::OK, "{stale}");
    assert_eq!(stale.get("stale"), Some(&json!(true)));
    let mut unmarked = stale.clone();
    unmarked["stale"] = json!(false);
    assert_eq!(unmarked, fresh, "the last good response, marked stale");

    // Still under 10 minutes since that response was computed.
    harness.clock.advance(9 * MINUTE);
    let (status, body) = harness.admin_get(&uri).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body.get("stale"), Some(&json!(true)));

    harness.clock.advance(MINUTE);
    let (status, body) = harness.admin_get(&uri).await?;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(error_code(&body), Some("dependency_unavailable"));

    // Once the source recovers, fresh data comes back.
    harness.source.set_failing(false);
    let (status, body) = harness.admin_get(&uri).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("stale"), Some(&json!(false)));

    // No response yet and a failing source: 503 straight away.
    harness.source.set_failing(true);
    let (status, _) = harness.admin_get(&format!("{HEALTH}?window=1h")).await?;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    Ok(())
}
