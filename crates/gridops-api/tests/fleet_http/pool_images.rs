//! The images a new runner pool clones (issue #97): `POST /api/v1/runner-pools`
//! stores the Tart base VM the caller sent, never the server default in its
//! place, and refuses a request whose `image` and primary provider image
//! disagree instead of silently keeping one of them.

use super::{placement::seed_pool, *};

fn admin_post(f: &Fixture, body: &Value) -> reqwest::RequestBuilder {
    f.client
        .post(format!("{}/api/v1/runner-pools", f.origin))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin)
        .json(body)
}

/// An organization pool with no runners to start, so creating it touches
/// nothing but the database.
fn pool(name: &str, delta: &Value) -> Result<Value> {
    let mut body = json!({
        "installationId": 1, "name": name, "scope": "organization", "mode": "ephemeral",
        "provider": "tart", "providers": ["tart"], "labels": [],
        "desiredCount": 0, "minCount": 0, "maxCount": 2, "autoscalingEnabled": false,
        "queueScaleFactor": 1, "idleTimeoutMinutes": 5, "cpuLimit": 4, "memoryLimitMb": 8192,
        "runnerGroupId": 1,
    });
    for (key, value) in delta.as_object().context("delta")? {
        body[key] = value.clone();
    }
    Ok(body)
}

async fn stored_images(f: &Fixture, id: &str) -> Result<(String, String, String)> {
    Ok(
        sqlx::query_as("SELECT image,docker_image,tart_image FROM runner_pools WHERE id=?")
            .bind(id)
            .fetch_one(&f.database)
            .await?,
    )
}

async fn create(f: &Fixture, body: &Value) -> Result<String> {
    let response = admin_post(f, body).send().await?;
    let status = response.status();
    let answer: Value = response.json().await?;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    Ok(answer["id"].as_str().context("pool id")?.to_owned())
}

async fn refused(f: &Fixture, body: &Value, message: &str) -> Result<()> {
    let response = admin_post(f, body).send().await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.json::<Value>().await?, json!({ "error": message }));
    Ok(())
}

#[tokio::test]
async fn a_new_pool_keeps_the_tart_base_vm_it_was_given() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;

    // The form's request for a Tart pool: the typed VM in both fields.
    let tart = create(
        &f,
        &pool(
            "macos-app",
            &json!({"image": "acme-xcode-26", "tartImage": "acme-xcode-26", "macosRuntime": "vm"}),
        )?,
    )
    .await?;
    let (image, _, tart_image) = stored_images(&f, &tart).await?;
    assert_eq!(
        (image.as_str(), tart_image.as_str()),
        ("acme-xcode-26", "acme-xcode-26")
    );
    let read: Value = f
        .client
        .get(format!("{}/api/v1/runner-pools/{tart}", f.origin))
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(read["tartImage"], "acme-xcode-26");

    // Without tartImage, `image` is the Tart base VM of a Tart-first pool.
    let image_only = create(
        &f,
        &pool("macos-image-only", &json!({"image": "acme-xcode-26"}))?,
    )
    .await?;
    assert_eq!(stored_images(&f, &image_only).await?.2, "acme-xcode-26");

    // A pool with Docker first keeps its Tart base VM separately.
    let mixed = create(
        &f,
        &pool(
            "cross-platform",
            &json!({
                "provider": "docker", "providers": ["docker", "tart"],
                "image": "runner:fixture", "dockerImage": "runner:fixture", "tartImage": "acme-xcode-26",
            }),
        )?,
    )
    .await?;
    assert_eq!(
        stored_images(&f, &mixed).await?,
        (
            "runner:fixture".into(),
            "runner:fixture".into(),
            "acme-xcode-26".into()
        )
    );

    // There, leaving tartImage out is an error, not the server default.
    refused(
        &f,
        &pool(
            "cross-no-vm",
            &json!({"provider": "docker", "providers": ["docker", "tart"], "image": "runner:fixture"}),
        )?,
        "Tart base VM must contain 1-300 non-padding characters.",
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn a_new_pool_whose_image_and_tart_base_vm_disagree_is_refused() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let pools_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runner_pools")
        .fetch_one(&f.database)
        .await?;

    // A client that merged the form's defaults and set only `image` used to
    // get the default VM stored and `image` dropped.
    refused(
        &f,
        &pool(
            "macos-app",
            &json!({"image": "acme-xcode-26", "tartImage": "gridops-macos-tahoe-base"}),
        )?,
        "image and tartImage name different images for the primary provider; send the same value in both, or leave tartImage out.",
    )
    .await?;
    refused(
        &f,
        &pool(
            "linux-app",
            &json!({"provider": "docker", "providers": ["docker"], "image": "ghcr.io/acme/runner:2", "dockerImage": "runner:fixture"}),
        )?,
        "image and dockerImage name different images for the primary provider; send the same value in both, or leave dockerImage out.",
    )
    .await?;

    let pools_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runner_pools")
        .fetch_one(&f.database)
        .await?;
    assert_eq!(pools_after, pools_before, "no refused pool was stored");
    Ok(())
}
