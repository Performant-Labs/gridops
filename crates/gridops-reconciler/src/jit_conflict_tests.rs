//! A runner name GitHub already holds (an earlier attempt registered it and
//! never started) must not block the runner's registration for good.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use gridops_core::{
    config::Config,
    github::{GitHubClient, JitRequest, RunnerTarget},
};
use serde_json::{Value, json};

/// A registration GitHub holds: (id, status, busy).
type Held = (i64, &'static str, bool);

#[derive(Clone)]
struct Fake {
    /// The runner GitHub still holds under the name, if any: (id, status, busy).
    leftover: Arc<Mutex<Option<Held>>>,
}

async fn fake_github(leftover: Option<Held>) -> Result<(String, Fake)> {
    let fake = Fake {
        leftover: Arc::new(Mutex::new(leftover)),
    };
    let app = Router::new()
        .route(
            "/orgs/{org}/actions/runners/generate-jitconfig",
            post(|State(fake): State<Fake>, Json(body): Json<Value>| async move {
                if fake.leftover.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_some() {
                    return (
                        StatusCode::CONFLICT,
                        Json(json!({"message": "Already exists - A runner with the name x already exists."})),
                    );
                }
                (
                    StatusCode::CREATED,
                    Json(json!({
                        "runner": {"id": 9, "name": body["name"], "status": "offline", "busy": false},
                        "encoded_jit_config": "jit",
                    })),
                )
            }),
        )
        .route(
            "/orgs/{org}/actions/runners",
            get(|State(fake): State<Fake>| async move {
                let runners: Vec<Value> = fake
                    .leftover
                    .lock().unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .map(|(id, status, busy)| {
                        json!({"id": id, "name": "light-1", "status": status, "busy": busy})
                    })
                    .collect();
                Json(json!({ "total_count": runners.len(), "runners": runners }))
            }),
        )
        .route(
            "/orgs/{org}/actions/runners/{id}",
            delete(|State(fake): State<Fake>, Path((_, _)): Path<(String, i64)>| async move {
                *fake.leftover.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                StatusCode::NO_CONTENT
            }),
        )
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let url = format!("http://{}", listener.local_addr()?);
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok((url, fake))
}

fn request() -> JitRequest {
    JitRequest {
        name: "light-1".into(),
        runner_group_id: 1,
        labels: vec![],
        work_folder: "_work".into(),
    }
}

async fn register(url: &str) -> Result<gridops_core::github::JitResponse> {
    let config = Config::from_env()?.with_github_api_url(reqwest::Url::parse(url)?);
    let github = GitHubClient::new(config)?;
    github
        .generate_jit_config(
            RunnerTarget::Organization {
                organization: "octo-org",
            },
            "token",
            &request(),
        )
        .await
}

#[tokio::test]
async fn an_offline_leftover_registration_is_replaced() -> Result<()> {
    let (url, fake) = fake_github(Some((5, "offline", false))).await?;
    let response = register(&url).await?;
    assert_eq!(response.encoded_jit_config, "jit");
    assert!(
        fake.leftover
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none(),
        "the leftover was removed"
    );
    Ok(())
}

#[tokio::test]
async fn a_live_runner_under_the_name_is_never_removed() -> Result<()> {
    for (status, busy) in [("online", false), ("offline", true)] {
        let (url, fake) = fake_github(Some((5, status, busy))).await?;
        let error = register(&url)
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("the conflict is reported"))?;
        assert!(error.to_string().contains("409"), "{error}");
        assert!(
            fake.leftover
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some(),
            "{status}/{busy} was kept"
        );
    }
    Ok(())
}
