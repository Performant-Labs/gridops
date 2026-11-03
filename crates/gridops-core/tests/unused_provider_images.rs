//! Upgrading a database whose runner pools carry the default image of a
//! provider they do not use (issue #103). Migration 0017 gave every existing
//! pool both providers' default images, and the web form sent both, so a
//! Docker-only pool usually holds `tart_image = gridops-macos-tahoe-base`.
//! Adding Tart to it later would clone that VM without anyone choosing it.
//!
//! The upgrade clears such a default for a provider the pool does not use, and
//! nothing else: not a pool that uses the provider, and not a value an
//! operator typed.

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use sqlx::{
    SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// The last migration before the one that clears unused default images.
const BEFORE: i64 = 43;

const DEFAULT_DOCKER: &str = "ghcr.io/actions/actions-runner:latest";
const DEFAULT_TART: &str = "gridops-macos-tahoe-base";

async fn database() -> Result<(SqlitePool, PathBuf)> {
    let directory =
        std::env::temp_dir().join(format!("gridops-pool-images-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory)?;
    let options = SqliteConnectOptions::new()
        .filename(directory.join("test.sqlite"))
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await?;
    MIGRATOR.run_to(BEFORE, &pool).await?;
    sqlx::query(
        "INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
         VALUES (1,1,'octo-org','Organization','Organization','selected',1,1)",
    )
    .execute(&pool)
    .await?;
    Ok((pool, directory))
}

async fn seed(
    db: &SqlitePool,
    id: &str,
    provider: &str,
    providers: &str,
    docker_image: &str,
    tart_image: &str,
) -> Result<()> {
    let image = if provider == "tart" {
        tart_image
    } else {
        docker_image
    };
    sqlx::query(
        "INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,created_at,updated_at)
         VALUES (?,1,?,'organization',?,?,'[]',?,?,?,1,7)",
    )
    .bind(id)
    .bind(id)
    .bind(provider)
    .bind(providers)
    .bind(image)
    .bind(docker_image)
    .bind(tart_image)
    .execute(db)
    .await?;
    Ok(())
}

async fn images(db: &SqlitePool, id: &str) -> Result<(String, String, String, i64)> {
    Ok(sqlx::query_as(
        "SELECT image,docker_image,tart_image,updated_at FROM runner_pools WHERE id=?",
    )
    .bind(id)
    .fetch_one(db)
    .await?)
}

#[tokio::test]
async fn upgrading_clears_only_the_default_image_of_a_provider_a_pool_does_not_use() -> Result<()> {
    let (db, directory) = database().await?;
    seed(
        &db,
        "docker-only",
        "docker",
        r#"["docker"]"#,
        "runner:acme",
        DEFAULT_TART,
    )
    .await?;
    seed(
        &db,
        "docker-own-vm",
        "docker",
        r#"["docker"]"#,
        "runner:acme",
        "acme-xcode-26",
    )
    .await?;
    seed(
        &db,
        "tart-only",
        "tart",
        r#"["tart"]"#,
        DEFAULT_DOCKER,
        "acme-xcode-26",
    )
    .await?;
    seed(
        &db,
        "tart-own-docker",
        "tart",
        r#"["tart"]"#,
        "runner:acme",
        "acme-xcode-26",
    )
    .await?;
    seed(
        &db,
        "tart-default",
        "tart",
        r#"["tart"]"#,
        DEFAULT_DOCKER,
        DEFAULT_TART,
    )
    .await?;
    seed(
        &db,
        "mixed",
        "docker",
        r#"["docker","tart"]"#,
        DEFAULT_DOCKER,
        DEFAULT_TART,
    )
    .await?;
    // A Docker pool still on both of 0017's column defaults.
    seed(
        &db,
        "docker-defaults",
        "docker",
        r#"["docker"]"#,
        DEFAULT_DOCKER,
        DEFAULT_TART,
    )
    .await?;

    MIGRATOR.run(&db).await?;

    // The default Tart base VM of a Docker-only pool is gone.
    assert_eq!(
        images(&db, "docker-only").await?,
        ("runner:acme".into(), "runner:acme".into(), String::new(), 7)
    );
    assert_eq!(
        images(&db, "docker-defaults").await?,
        (
            DEFAULT_DOCKER.into(),
            DEFAULT_DOCKER.into(),
            String::new(),
            7
        )
    );
    // The default Docker image of a Tart-only pool too.
    assert_eq!(
        images(&db, "tart-only").await?,
        (
            "acme-xcode-26".into(),
            String::new(),
            "acme-xcode-26".into(),
            7
        )
    );
    assert_eq!(
        images(&db, "tart-default").await?,
        (DEFAULT_TART.into(), String::new(), DEFAULT_TART.into(), 7)
    );
    // A value that is not the server default may have been typed: kept.
    assert_eq!(
        images(&db, "docker-own-vm").await?,
        (
            "runner:acme".into(),
            "runner:acme".into(),
            "acme-xcode-26".into(),
            7
        )
    );
    assert_eq!(
        images(&db, "tart-own-docker").await?,
        (
            "acme-xcode-26".into(),
            "runner:acme".into(),
            "acme-xcode-26".into(),
            7
        )
    );
    // A pool that uses both providers keeps both, defaults included.
    assert_eq!(
        images(&db, "mixed").await?,
        (
            DEFAULT_DOCKER.into(),
            DEFAULT_DOCKER.into(),
            DEFAULT_TART.into(),
            7
        )
    );

    // Running it again changes nothing.
    let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&db)
        .await?;
    MIGRATOR.run(&db).await?;
    let reapplied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&db)
        .await?;
    assert_eq!(applied, reapplied);
    db.close().await;
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}

/// A deployment's own defaults (`GRIDOPS_RUNNER_IMAGE`,
/// `GRIDOPS_TART_RUNNER_IMAGE`), which migration 0044 cannot know. The web form
/// sent them for every new pool, so a Docker-only pool there holds the custom
/// Tart default and a Tart-only pool the custom Docker default.
const CUSTOM_DOCKER: &str = "ghcr.io/acme/actions-runner:2";
const CUSTOM_TART: &str = "acme-tahoe-base";

async fn migrated() -> Result<(SqlitePool, PathBuf)> {
    let (db, directory) = database().await?;
    MIGRATOR.run(&db).await?;
    Ok((db, directory))
}

#[tokio::test]
async fn startup_clears_a_custom_default_only_from_a_provider_a_pool_does_not_use() -> Result<()> {
    let (db, directory) = migrated().await?;
    // Carries the custom defaults on the provider it does not use.
    seed(
        &db,
        "docker-only",
        "docker",
        r#"["docker"]"#,
        "runner:acme",
        CUSTOM_TART,
    )
    .await?;
    seed(
        &db,
        "tart-only",
        "tart",
        r#"["tart"]"#,
        CUSTOM_DOCKER,
        "acme-xcode-26",
    )
    .await?;
    // Uses the provider: keeps the default, VM or native.
    seed(&db, "tart-default", "tart", r#"["tart"]"#, "", CUSTOM_TART).await?;
    seed(&db, "tart-native", "tart", r#"["tart"]"#, "", CUSTOM_TART).await?;
    sqlx::query("UPDATE runner_pools SET macos_runtime='native' WHERE id='tart-native'")
        .execute(&db)
        .await?;
    seed(
        &db,
        "mixed",
        "docker",
        r#"["docker","tart"]"#,
        CUSTOM_DOCKER,
        CUSTOM_TART,
    )
    .await?;
    // Tart second in `providers`, and a legacy pool with only `provider`.
    seed(
        &db,
        "tart-second",
        "docker",
        r#"["docker","tart"]"#,
        "runner:acme",
        CUSTOM_TART,
    )
    .await?;
    seed(&db, "legacy-tart", "tart", "[]", CUSTOM_DOCKER, CUSTOM_TART).await?;
    // Another value on an unused provider is not the default: kept.
    seed(
        &db,
        "docker-own-vm",
        "docker",
        r#"["docker"]"#,
        "runner:acme",
        "acme-xcode-26",
    )
    .await?;
    seed(
        &db,
        "tart-own-docker",
        "tart",
        r#"["tart"]"#,
        "runner:acme",
        "acme-xcode-26",
    )
    .await?;
    // So is the built-in default when this deployment configured another one.
    seed(
        &db,
        "docker-builtin-vm",
        "docker",
        r#"["docker"]"#,
        "runner:acme",
        DEFAULT_TART,
    )
    .await?;

    let changed =
        gridops_core::db::clear_unused_default_images(&db, CUSTOM_DOCKER, CUSTOM_TART).await?;
    assert_eq!(changed, 3, "docker-only, tart-only and legacy-tart");

    let expect = |image: &str, docker: &str, tart: &str| {
        (image.to_owned(), docker.to_owned(), tart.to_owned(), 7)
    };
    assert_eq!(
        images(&db, "docker-only").await?,
        expect("runner:acme", "runner:acme", "")
    );
    assert_eq!(
        images(&db, "tart-only").await?,
        expect("acme-xcode-26", "", "acme-xcode-26")
    );
    assert_eq!(
        images(&db, "legacy-tart").await?,
        expect(CUSTOM_TART, "", CUSTOM_TART)
    );
    assert_eq!(
        images(&db, "tart-default").await?,
        expect(CUSTOM_TART, "", CUSTOM_TART)
    );
    assert_eq!(
        images(&db, "tart-native").await?,
        expect(CUSTOM_TART, "", CUSTOM_TART)
    );
    assert_eq!(
        images(&db, "mixed").await?,
        expect(CUSTOM_DOCKER, CUSTOM_DOCKER, CUSTOM_TART)
    );
    assert_eq!(
        images(&db, "tart-second").await?,
        expect("runner:acme", "runner:acme", CUSTOM_TART)
    );
    assert_eq!(
        images(&db, "docker-own-vm").await?,
        expect("runner:acme", "runner:acme", "acme-xcode-26")
    );
    assert_eq!(
        images(&db, "tart-own-docker").await?,
        expect("acme-xcode-26", "runner:acme", "acme-xcode-26")
    );
    assert_eq!(
        images(&db, "docker-builtin-vm").await?,
        expect("runner:acme", "runner:acme", DEFAULT_TART)
    );
    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT DISTINCT configuration_version FROM runner_pools")
            .fetch_all(&db)
            .await?;
    assert_eq!(versions, vec![1], "no pool's runners are replaced");

    // Every later start finds nothing to do.
    assert_eq!(
        gridops_core::db::clear_unused_default_images(&db, CUSTOM_DOCKER, CUSTOM_TART).await?,
        0
    );
    assert_eq!(
        images(&db, "docker-only").await?,
        expect("runner:acme", "runner:acme", "")
    );
    db.close().await;
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}

#[tokio::test]
async fn startup_clears_nothing_when_a_default_is_set_empty() -> Result<()> {
    let (db, directory) = migrated().await?;
    seed(
        &db,
        "docker-only",
        "docker",
        r#"["docker"]"#,
        "runner:acme",
        "",
    )
    .await?;
    seed(&db, "tart-only", "tart", r#"["tart"]"#, "", "acme-xcode-26").await?;
    assert_eq!(
        gridops_core::db::clear_unused_default_images(&db, "", "").await?,
        0,
        "an empty default matches the pools that already store none"
    );
    db.close().await;
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}
