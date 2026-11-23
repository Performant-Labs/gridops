//! Contract tests for #199's API (the job category rules), written by #197.
//!
//! #199 removes the `#[ignore = "contract: #199"]` markers and makes these
//! pass; it changes nothing else here (`Fleet health guard`). The routes are
//! registered by #197 in `session_routes()` (session-only, admin-only):
//! - `GET /api/v1/settings/job-categories` → `{"rules": [{position,
//!   category, field, pattern}]}` in position order, positions `0..n`;
//! - `PUT` the same path with `{"rules": [{category, field, pattern}]}`
//!   replaces the whole list in that order, and answers with the new list;
//!   an invalid regex is a 422 whose `details` are `[{position, message}]`,
//!   one per failing rule (D-3(d)), and nothing is saved;
//! - `POST /api/v1/settings/job-categories/preview` with a proposed list
//!   answers `{"rules": [{position, matches}], "other": [{name, jobs}]}`:
//!   how many jobs completed in the last 7 days each rule would classify
//!   (first match wins), and the 20 most common job names left in `other`,
//!   most jobs first. Nothing is saved.
//!
//! Mounted from `fleet/mod.rs` (#197-owned). Jobs are seeded with #197's
//! `testdata`.

use anyhow::Result;
use axum::http::{Method, StatusCode};
use gridops_core::fleet_health::testdata::{self, JobSeed};
use serde_json::{Value, json};

use super::health_test_support::Harness;

const RULES: &str = "/api/v1/settings/job-categories";
const PREVIEW: &str = "/api/v1/settings/job-categories/preview";
const DAY: i64 = 86_400_000;

fn defaults() -> Value {
    json!({"rules": [
        {"position": 0, "category": "test", "field": "job_name",
         "pattern": "test|spec|e2e|playwright|vitest|jest|pytest|cargo test|shard"},
        {"position": 1, "category": "build", "field": "job_name",
         "pattern": "build|compile|bundle|image|docker|package"},
        {"position": 2, "category": "check", "field": "job_name",
         "pattern": "lint|clippy|fmt|format|typecheck|tsc|check|audit"},
        {"position": 3, "category": "deploy", "field": "job_name",
         "pattern": "deploy|release|publish|pages"},
    ]})
}

#[tokio::test]
#[ignore = "contract: #199"]
async fn the_rules_are_listed_in_order() -> Result<()> {
    let harness = Harness::new().await?;
    let (status, body) = harness.admin_get(RULES).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, defaults());
    Ok(())
}

#[tokio::test]
#[ignore = "contract: #199"]
async fn saving_replaces_the_whole_list_and_keeps_its_order() -> Result<()> {
    let harness = Harness::new().await?;
    let admin = harness.admin.clone();
    let proposed = json!({"rules": [
        {"category": "deploy", "field": "workflow_name", "pattern": "^release$"},
        {"category": "test", "field": "job_name", "pattern": "spec"},
        {"category": "other", "field": "labels", "pattern": "gpu"},
    ]});
    let (status, saved) = harness
        .send(Method::PUT, RULES, Some(&admin), Some(proposed))
        .await?;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let expected = json!({"rules": [
        {"position": 0, "category": "deploy", "field": "workflow_name", "pattern": "^release$"},
        {"position": 1, "category": "test", "field": "job_name", "pattern": "spec"},
        {"position": 2, "category": "other", "field": "labels", "pattern": "gpu"},
    ]});
    assert_eq!(saved, expected);
    let (_, listed) = harness.admin_get(RULES).await?;
    assert_eq!(listed, expected, "the saved order is what is listed");
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM job_category_rules")
        .fetch_one(&harness.database)
        .await?;
    assert_eq!(rows, 3, "the old rules are gone");

    // An empty list is allowed: everything is `other`.
    let (status, body) = harness
        .send(Method::PUT, RULES, Some(&admin), Some(json!({"rules": []})))
        .await?;
    assert_eq!((status, body), (StatusCode::OK, json!({"rules": []})));
    Ok(())
}

#[tokio::test]
#[ignore = "contract: #199"]
async fn an_invalid_regex_is_a_422_naming_the_rule_and_nothing_is_saved() -> Result<()> {
    let harness = Harness::new().await?;
    let admin = harness.admin.clone();
    let (status, body) = harness
        .send(
            Method::PUT,
            RULES,
            Some(&admin),
            Some(json!({"rules": [
                {"category": "test", "field": "job_name", "pattern": "ok"},
                {"category": "build", "field": "job_name", "pattern": "(unclosed"},
                {"category": "check", "field": "job_name", "pattern": "fine"},
                {"category": "deploy", "field": "job_name", "pattern": "[z-a]"},
            ]})),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let details = body
        .get("details")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let positions: Vec<Option<i64>> = details
        .iter()
        .map(|detail| detail.get("position").and_then(Value::as_i64))
        .collect();
    assert_eq!(positions, [Some(1), Some(3)], "{body}");
    for detail in &details {
        let message = detail.get("message").and_then(Value::as_str).unwrap_or("");
        assert!(!message.trim().is_empty(), "{detail}");
    }
    assert!(body.get("error").and_then(Value::as_str).is_some());
    let (_, listed) = harness.admin_get(RULES).await?;
    assert_eq!(listed, defaults(), "nothing was saved");

    // A value outside the contract is a 422 too (or the extractor's 400),
    // never saved.
    let (status, _) = harness
        .send(
            Method::PUT,
            RULES,
            Some(&admin),
            Some(
                json!({"rules": [{"category": "fix-agent", "field": "job_name", "pattern": "x"}]}),
            ),
        )
        .await?;
    assert!(
        status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
        "{status}"
    );
    let (_, listed) = harness.admin_get(RULES).await?;
    assert_eq!(listed, defaults());
    Ok(())
}

#[tokio::test]
#[ignore = "contract: #199"]
async fn the_preview_counts_each_rule_and_lists_the_commonest_names_left_in_other() -> Result<()> {
    let harness = Harness::new().await?;
    let admin = harness.admin.clone();
    let fleet = testdata::seed_fleet(&harness.database).await?;
    let io = fleet.host("io")?.id.clone();
    let now = gridops_core::now_millis();
    let mut jobs = Vec::new();
    let mut id = 1;
    let mut job = |name: &str, completed: i64| {
        id += 1;
        JobSeed::completed(
            id,
            name,
            Some(&io),
            completed - 60_000,
            completed - 50_000,
            completed,
        )
    };
    for _ in 0..3 {
        jobs.push(job("unit tests", now - DAY));
    }
    for _ in 0..2 {
        jobs.push(job("docker image", now - 2 * DAY));
    }
    for _ in 0..4 {
        jobs.push(job("notify slack", now - 3 * DAY));
    }
    jobs.push(job("notify email", now - 6 * DAY));
    // Outside the 7 days, and not completed: not counted.
    jobs.push(job("unit tests", now - 8 * DAY));
    jobs.push(job("notify slack", now - 9 * DAY));
    let mut running = job("unit tests", now);
    running.status = "in_progress".to_owned();
    running.completed_at = None;
    running.conclusion = None;
    jobs.push(running);
    testdata::insert_jobs(&harness.database, &fleet, &jobs).await?;

    let (status, body) = harness
        .send(
            Method::POST,
            PREVIEW,
            Some(&admin),
            Some(json!({"rules": [
                {"category": "test", "field": "job_name", "pattern": "test"},
                {"category": "build", "field": "job_name", "pattern": "docker"},
                {"category": "deploy", "field": "job_name", "pattern": "pages"},
            ]})),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({
            "rules": [
                {"position": 0, "matches": 3},
                {"position": 1, "matches": 2},
                {"position": 2, "matches": 0},
            ],
            "other": [
                {"name": "notify slack", "jobs": 4},
                {"name": "notify email", "jobs": 1},
            ]
        })
    );
    // First match wins: a later rule that also matches counts nothing.
    let (_, body) = harness
        .send(
            Method::POST,
            PREVIEW,
            Some(&admin),
            Some(json!({"rules": [
                {"category": "check", "field": "job_name", "pattern": "notify"},
                {"category": "deploy", "field": "job_name", "pattern": "slack"},
            ]})),
        )
        .await?;
    assert_eq!(
        body.get("rules"),
        Some(&json!([{"position": 0, "matches": 5}, {"position": 1, "matches": 0}]))
    );
    // A preview saves nothing.
    let (_, listed) = harness.admin_get(RULES).await?;
    assert_eq!(listed, defaults());
    // An invalid proposed rule is named the same way as on save.
    let (status, body) = harness
        .send(
            Method::POST,
            PREVIEW,
            Some(&admin),
            Some(json!({"rules": [
                {"category": "test", "field": "job_name", "pattern": "("},
            ]})),
        )
        .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body.pointer("/details/0/position"), Some(&json!(0)));
    Ok(())
}

#[tokio::test]
#[ignore = "contract: #199"]
async fn only_administrators_may_read_save_or_preview() -> Result<()> {
    let harness = Harness::new().await?;
    let member = harness.member.clone();
    let body = json!({"rules": []});
    for (method, uri, payload) in [
        (Method::GET, RULES, None),
        (Method::PUT, RULES, Some(body.clone())),
        (Method::POST, PREVIEW, Some(body.clone())),
    ] {
        let (status, response) = harness
            .send(method.clone(), uri, Some(&member), payload.clone())
            .await?;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "member {method} {uri}: {response}"
        );
        let (status, _) = harness.send(method.clone(), uri, None, payload).await?;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "anonymous {method} {uri}");
    }
    let (_, listed) = harness.admin_get(RULES).await?;
    assert_eq!(listed, defaults(), "a refused save changed nothing");
    Ok(())
}
