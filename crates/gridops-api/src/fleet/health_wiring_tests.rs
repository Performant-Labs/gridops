//! #197: the fleet health wiring in `gridops-api` that is live before any
//! wave-1 issue lands:
//! - `GET /api/v1/fleet/health` returns the fixture (the fake source) to an
//!   administrator and is refused to anyone else (D-4: admin-only, like
//!   `GET /api/v1/fleet/workload`);
//! - it takes an administrator's `fleet:read` token, like `workload`
//!   (`TOKEN_SCOPED_ROUTES`, `docs/fleet-api.md` and `OpenAPI` agree; the
//!   drift tests in `openapi.rs` and `access.rs` cross-check them);
//! - `OpenAPI` describes the health route and the three job-category routes
//!   (#199), which are session-only;
//! - `ApiError`'s 422 carries per-rule `details` for #199, and no existing
//!   error body changes (D-3(d), refined).
//!
//! Not live yet: `?window=` is not read until #194, so today any value, a
//! bad one included, gets 200 and the 24 h fixture. The 400 for a bad window
//! that `OpenAPI` already publishes is #194's contract test
//! (`health_contract_tests.rs`), not behaviour this file pins.
//!
//! Mounted from `fleet/mod.rs`.

use anyhow::{Context as _, Result};
use axum::{http::StatusCode, response::IntoResponse as _};
use serde_json::{Value, json};

use super::health_test_support::{HEALTH, Harness, error_code, fixture, openapi};
use crate::error::ApiError;

#[tokio::test]
async fn an_admin_gets_the_fixture_and_a_member_is_refused() -> Result<()> {
    let harness = Harness::new().await?;
    let (status, body) = harness.admin_get(&format!("{HEALTH}?window=24h")).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        fixture()?,
        "the endpoint serves the fake source's fixture"
    );

    let member = harness.member.clone();
    let (status, body) = harness
        .get_as(&format!("{HEALTH}?window=24h"), Some(&member))
        .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(error_code(&body), Some("forbidden"), "{body}");
    Ok(())
}

#[test]
fn the_health_route_takes_fleet_read_tokens_like_workload() {
    let scope = |path: &str| {
        super::TOKEN_SCOPED_ROUTES
            .iter()
            .find(|(method, route, _)| *method == "GET" && *route == path)
            .map(|(_, _, scope)| *scope)
    };
    assert_eq!(scope(HEALTH), Some("fleet:read"));
    assert_eq!(scope(HEALTH), scope("/api/v1/fleet/workload"));
    for (method, route, _) in super::TOKEN_SCOPED_ROUTES {
        assert!(
            !route.starts_with("/api/v1/settings/job-categories"),
            "{method} {route}: the job-category routes are session-only"
        );
    }
}

#[test]
fn openapi_describes_the_health_and_job_category_routes() -> Result<()> {
    let spec = openapi()?;
    let operation = |path: &str, method: &str| -> Result<Value> {
        spec.get("paths")
            .and_then(|paths| paths.get(path))
            .and_then(|item| item.get(method))
            .cloned()
            .with_context(|| format!("OpenAPI has no {method} {path}"))
    };

    let health = operation(HEALTH, "get")?;
    assert_eq!(
        health.get("security"),
        operation("/api/v1/fleet/workload", "get")?.get("security"),
        "the health route is authenticated like workload"
    );
    let window = health
        .get("parameters")
        .and_then(Value::as_array)
        .and_then(|parameters| {
            parameters
                .iter()
                .find(|parameter| parameter.get("name") == Some(&json!("window")))
        })
        .context("no window parameter")?;
    assert_eq!(window.get("in"), Some(&json!("query")));
    assert_eq!(
        window.pointer("/schema/enum"),
        Some(&json!(["1h", "24h", "7d"]))
    );
    for status in ["200", "400", "401", "403", "503"] {
        assert!(
            health.pointer(&format!("/responses/{status}")).is_some(),
            "GET {HEALTH} documents {status}"
        );
    }

    let rules = "/api/v1/settings/job-categories";
    for (path, method, statuses) in [
        (rules, "get", &["200", "401", "403"][..]),
        (rules, "put", &["200", "401", "403", "422"][..]),
        (
            "/api/v1/settings/job-categories/preview",
            "post",
            &["200", "401", "403", "422"][..],
        ),
    ] {
        let op = operation(path, method)?;
        assert_eq!(
            op.get("security"),
            Some(&json!([{"cookieAuth": []}])),
            "{method} {path} is session-only"
        );
        for status in statuses {
            assert!(
                op.pointer(&format!("/responses/{status}")).is_some(),
                "{method} {path} documents {status}"
            );
        }
    }
    Ok(())
}

async fn body_of(error: ApiError) -> Result<(StatusCode, Value)> {
    let response = error.into_response();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

#[tokio::test]
async fn a_422_names_each_failing_rule_and_no_other_error_body_changes() -> Result<()> {
    let (status, body) = body_of(ApiError::Invalid {
        code: "invalid_job_category_rules",
        message: "2 rules have an invalid pattern.".to_owned(),
        details: json!([
            {"position": 1, "message": "regex parse error: unclosed group"},
            {"position": 3, "message": "regex parse error: invalid range"}
        ]),
    })
    .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        json!({
            "error": "2 rules have an invalid pattern.",
            "code": "invalid_job_category_rules",
            "details": [
                {"position": 1, "message": "regex parse error: unclosed group"},
                {"position": 3, "message": "regex parse error: invalid range"}
            ]
        })
    );

    // Unchanged: no `details` (nor `code`) where there was none.
    assert_eq!(
        body_of(ApiError::BadRequest("bad".to_owned())).await?,
        (StatusCode::BAD_REQUEST, json!({"error": "bad"}))
    );
    assert_eq!(
        body_of(ApiError::Forbidden).await?,
        (StatusCode::FORBIDDEN, json!({"error": "forbidden"}))
    );
    assert_eq!(
        body_of(ApiError::Refused {
            code: "in_use",
            message: "in use".to_owned()
        })
        .await?,
        (
            StatusCode::CONFLICT,
            json!({"error": "in use", "code": "in_use"})
        )
    );
    Ok(())
}
