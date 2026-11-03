//! The images a runner pool clones (issues #97 and #103): `POST` and `PUT
//! /api/v1/runner-pools` store the Tart base VM the caller sent, never the
//! server default in its place, store no image for a provider the pool does not
//! use, and refuse a request whose `image` and primary provider image disagree
//! instead of silently keeping one of them.

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
        "Tart base VM (tartImage) is required for a pool that uses macOS · Tart virtual machines.",
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

fn admin_put(f: &Fixture, id: &str, body: &Value) -> reqwest::RequestBuilder {
    f.client
        .put(format!("{}/api/v1/runner-pools/{id}", f.origin))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin)
        .json(body)
}

async fn read_pool(f: &Fixture, id: &str) -> Result<Value> {
    let response = f
        .client
        .get(format!("{}/api/v1/runner-pools/{id}", f.origin))
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(response.json().await?)
}

/// The edit body a client builds by reading the pool and sending it back.
fn edit_of(read: &Value, delta: &Value) -> Result<Value> {
    let mut body = json!({});
    for key in [
        "name",
        "mode",
        "provider",
        "providers",
        "labels",
        "image",
        "dockerImage",
        "tartImage",
        "macosRuntime",
        "desiredCount",
        "minCount",
        "maxCount",
        "autoscalingEnabled",
        "queueScaleFactor",
        "idleTimeoutMinutes",
        "cpuLimit",
        "memoryLimitMb",
        "runnerGroupId",
    ] {
        body[key] = read[key].clone();
    }
    // The pool-name label is added by the server; sending it back is refused.
    body["labels"] = json!([]);
    for (key, value) in delta.as_object().context("delta")? {
        body[key] = value.clone();
    }
    Ok(body)
}

async fn edit(f: &Fixture, id: &str, body: &Value) -> Result<()> {
    let response = admin_put(f, id, body).send().await?;
    let status = response.status();
    let answer: Value = response.json().await?;
    assert_eq!(status, StatusCode::OK, "{answer}");
    Ok(())
}

async fn edit_refused(f: &Fixture, id: &str, body: &Value, message: &str) -> Result<()> {
    let before = stored_images(f, id).await?;
    let response = admin_put(f, id, body).send().await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.json::<Value>().await?, json!({ "error": message }));
    assert_eq!(
        stored_images(f, id).await?,
        before,
        "a refused edit changes nothing"
    );
    Ok(())
}

/// Issue #103: the form and a body built from `/options` `.defaults` send an
/// image for both providers. A pool stores none for a provider it does not
/// use, so adding that provider later cannot silently clone the default.
#[tokio::test]
async fn a_pool_stores_no_image_for_a_provider_it_does_not_use() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;

    let docker = create(
        &f,
        &pool(
            "linux-app",
            &json!({
                "provider": "docker", "providers": ["docker"], "image": "runner:fixture",
                "dockerImage": "runner:fixture", "tartImage": "gridops-macos-tahoe-base",
            }),
        )?,
    )
    .await?;
    assert_eq!(
        stored_images(&f, &docker).await?,
        (
            "runner:fixture".into(),
            "runner:fixture".into(),
            String::new()
        )
    );
    assert_eq!(read_pool(&f, &docker).await?["tartImage"], "");

    let tart = create(
        &f,
        &pool(
            "macos-app",
            &json!({
                "image": "acme-xcode-26", "tartImage": "acme-xcode-26",
                "dockerImage": "ghcr.io/actions/actions-runner:latest",
            }),
        )?,
    )
    .await?;
    assert_eq!(
        stored_images(&f, &tart).await?,
        (
            "acme-xcode-26".into(),
            String::new(),
            "acme-xcode-26".into()
        )
    );

    // Taking a provider out of a pool drops its image too.
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
    let read = read_pool(&f, &mixed).await?;
    edit(
        &f,
        &mixed,
        &edit_of(&read, &json!({"providers": ["docker"]}))?,
    )
    .await?;
    assert_eq!(
        stored_images(&f, &mixed).await?,
        (
            "runner:fixture".into(),
            "runner:fixture".into(),
            String::new()
        )
    );
    Ok(())
}

#[tokio::test]
async fn adding_tart_to_a_docker_only_pool_needs_its_base_vm() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let id = create(
        &f,
        &pool(
            "linux-app",
            &json!({
                "provider": "docker", "providers": ["docker"], "image": "runner:fixture",
                "dockerImage": "runner:fixture", "tartImage": "gridops-macos-tahoe-base",
            }),
        )?,
    )
    .await?;
    let read = read_pool(&f, &id).await?;

    // Read, add Tart, send back: the pool has no Tart base VM to send.
    edit_refused(
        &f,
        &id,
        &edit_of(&read, &json!({"providers": ["docker", "tart"]}))?,
        "Tart base VM (tartImage) is required for a pool that uses macOS · Tart virtual machines.",
    )
    .await?;

    edit(
        &f,
        &id,
        &edit_of(
            &read,
            &json!({"providers": ["docker", "tart"], "tartImage": "acme-xcode-26"}),
        )?,
    )
    .await?;
    assert_eq!(
        stored_images(&f, &id).await?,
        (
            "runner:fixture".into(),
            "runner:fixture".into(),
            "acme-xcode-26".into()
        )
    );
    assert_eq!(read_pool(&f, &id).await?["tartImage"], "acme-xcode-26");
    Ok(())
}

/// #99's agreement rule on edits (#104): a read-modify-write that changes
/// only one of `image` and the primary provider's image is refused rather
/// than silently keeping the other.
#[tokio::test]
async fn an_edit_whose_image_and_tart_base_vm_disagree_is_refused() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let id = create(
        &f,
        &pool(
            "macos-app",
            &json!({"image": "acme-xcode-26", "tartImage": "acme-xcode-26"}),
        )?,
    )
    .await?;
    let read = read_pool(&f, &id).await?;

    edit_refused(
        &f,
        &id,
        &edit_of(&read, &json!({"image": "acme-xcode-27"}))?,
        "image and tartImage name different images for the primary provider; send the same value in both, or leave tartImage out.",
    )
    .await?;
    edit_refused(
        &f,
        &id,
        &edit_of(&read, &json!({"tartImage": "acme-xcode-27"}))?,
        "image and tartImage name different images for the primary provider; send the same value in both, or leave tartImage out.",
    )
    .await?;

    edit(
        &f,
        &id,
        &edit_of(
            &read,
            &json!({"image": "acme-xcode-27", "tartImage": "acme-xcode-27"}),
        )?,
    )
    .await?;
    assert_eq!(stored_images(&f, &id).await?.2, "acme-xcode-27");

    let docker = create(
        &f,
        &pool(
            "linux-app",
            &json!({"provider": "docker", "providers": ["docker"], "image": "runner:fixture", "dockerImage": "runner:fixture"}),
        )?,
    )
    .await?;
    let read = read_pool(&f, &docker).await?;
    edit_refused(
        &f,
        &docker,
        &edit_of(&read, &json!({"image": "ghcr.io/acme/runner:2"}))?,
        "image and dockerImage name different images for the primary provider; send the same value in both, or leave dockerImage out.",
    )
    .await?;
    Ok(())
}

/// An image left over for a provider the pool does not use (one an operator
/// typed, which the upgrade keeps) is dropped by the next edit. No runner ever
/// used it, so dropping it alone does not replace the pool's runners.
#[tokio::test]
async fn dropping_an_unused_image_does_not_replace_the_runners() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let id = create(
        &f,
        &pool(
            "linux-app",
            &json!({"provider": "docker", "providers": ["docker"], "image": "runner:fixture", "dockerImage": "runner:fixture"}),
        )?,
    )
    .await?;
    sqlx::query("UPDATE runner_pools SET tart_image='acme-xcode-26' WHERE id=?")
        .bind(&id)
        .execute(&f.database)
        .await?;
    let read = read_pool(&f, &id).await?;
    let version = read["configurationVersion"].as_i64().context("version")?;

    let response = admin_put(&f, &id, &edit_of(&read, &json!({}))?)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let answer: Value = response.json().await?;
    assert_eq!(answer["rollingReplacement"], false, "{answer}");
    assert_eq!(answer["configurationVersion"], version, "{answer}");
    assert_eq!(stored_images(&f, &id).await?.2, "");
    Ok(())
}

/// #110's open question 2: the create form sends no base VM for native Tart.
/// The API accepts that, stores none, and asks for one when the pool is later
/// switched to Virtual machine.
#[tokio::test]
async fn a_native_tart_pool_stores_no_base_vm() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;

    // The create form's request for a native Tart pool.
    let id = create(
        &f,
        &pool(
            "macos-native",
            &json!({"macosRuntime": "native", "image": "", "dockerImage": "", "tartImage": ""}),
        )?,
    )
    .await?;
    assert_eq!(
        stored_images(&f, &id).await?,
        (String::new(), String::new(), String::new())
    );
    let read = read_pool(&f, &id).await?;
    assert_eq!(
        (&read["macosRuntime"], &read["tartImage"]),
        (&json!("native"), &json!(""))
    );

    edit_refused(
        &f,
        &id,
        &edit_of(&read, &json!({"macosRuntime": "vm"}))?,
        "Tart base VM (tartImage) is required for a pool that uses macOS · Tart virtual machines.",
    )
    .await?;
    edit(
        &f,
        &id,
        &edit_of(
            &read,
            &json!({"macosRuntime": "vm", "image": "acme-xcode-26", "tartImage": "acme-xcode-26"}),
        )?,
    )
    .await?;
    assert_eq!(
        stored_images(&f, &id).await?,
        (
            "acme-xcode-26".into(),
            String::new(),
            "acme-xcode-26".into()
        )
    );
    Ok(())
}

/// #110's open question 1: migration 0044 clears only the built-in defaults.
/// A deployment with its own `GRIDOPS_RUNNER_IMAGE` and
/// `GRIDOPS_TART_RUNNER_IMAGE` has those on providers its pools do not use;
/// the API clears exactly those when it starts.
#[tokio::test]
async fn startup_clears_this_deployments_default_images_from_unused_providers() -> Result<()> {
    const DOCKER: &str = "ghcr.io/acme/actions-runner:2";
    const TART: &str = "acme-tahoe-base";
    let mut f = Fixture::with_env(&[
        ("GRIDOPS_RUNNER_IMAGE", DOCKER),
        ("GRIDOPS_TART_RUNNER_IMAGE", TART),
    ])
    .await?;
    // `fleet-pool` is Docker-only with `tart_image = 'macos'`: not the default.
    seed_pool(&f).await?;
    // Pools as an earlier version's web form stored them.
    for (id, provider, image, docker_image, tart_image) in [
        ("docker-only", "docker", "runner:acme", "runner:acme", TART),
        (
            "tart-only",
            "tart",
            "acme-xcode-26",
            DOCKER,
            "acme-xcode-26",
        ),
        ("tart-default", "tart", TART, "", TART),
    ] {
        sqlx::query(
            "INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,created_at,updated_at)
             VALUES (?,1,?,'organization',?,json_array(?),'[]',?,?,?,1,7)",
        )
        .bind(id)
        .bind(id)
        .bind(provider)
        .bind(provider)
        .bind(image)
        .bind(docker_image)
        .bind(tart_image)
        .execute(&f.database)
        .await?;
    }

    f.restart(Duration::ZERO).await?;
    assert_eq!(
        stored_images(&f, "docker-only").await?,
        ("runner:acme".into(), "runner:acme".into(), String::new())
    );
    assert_eq!(
        stored_images(&f, "tart-only").await?,
        (
            "acme-xcode-26".into(),
            String::new(),
            "acme-xcode-26".into()
        )
    );
    assert_eq!(
        stored_images(&f, "tart-default").await?,
        (TART.into(), String::new(), TART.into())
    );
    assert_eq!(
        stored_images(&f, "fleet-pool").await?,
        (
            "runner:fixture".into(),
            "runner:fixture".into(),
            "macos".into()
        )
    );
    let log_path = f.directory.join("api.log");
    let log = || std::fs::read_to_string(&log_path);
    // The log is coloured: drop the escape sequences before reading it.
    let plain = |line: &str| {
        let mut text = String::new();
        let mut escape = false;
        for character in line.chars() {
            match (escape, character) {
                (false, '\u{1b}') => escape = true,
                (false, character) => text.push(character),
                (true, 'm') => escape = false,
                (true, _) => {}
            }
        }
        text
    };
    let cleared = |log: &str| {
        log.lines()
            .map(plain)
            .filter(|line| line.contains("cleared this deployment's default images"))
            .collect::<Vec<_>>()
    };
    let lines = cleared(&log()?);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("pools=2"), "{lines:?}");

    // The next start finds nothing more to clear.
    f.restart(Duration::ZERO).await?;
    assert_eq!(cleared(&log()?).len(), 1);
    assert_eq!(
        stored_images(&f, "tart-default").await?,
        (TART.into(), String::new(), TART.into())
    );
    Ok(())
}
