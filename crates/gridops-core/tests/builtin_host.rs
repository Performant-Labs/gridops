//! The built-in runner host is the machine `GridOps` itself runs on. Whether it
//! may start runners is one instance setting: how it is first decided for an
//! install that already has pools, how pools that depend on it are reported
//! when it is off, and what the setting change records.

use anyhow::Result;
use gridops_core::{builtin_host, connect_database_path};
use sqlx::SqlitePool;

async fn database() -> Result<(SqlitePool, std::path::PathBuf)> {
    let directory =
        std::env::temp_dir().join(format!("gridops-builtin-host-{}", uuid::Uuid::new_v4()));
    let pool = connect_database_path(&directory.join("db.sqlite")).await?;
    Ok((pool, directory))
}

async fn seed_installation(db: &SqlitePool) -> Result<()> {
    sqlx::query(
        "INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
         VALUES (1,1,'octo-org','Organization','Organization','selected',1,1)",
    )
    .execute(db)
    .await?;
    sqlx::query(
        "INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at)
         VALUES ('admin',1,'admin','admin','',1,1,1)",
    )
    .execute(db)
    .await?;
    Ok(())
}

async fn seed_pool(db: &SqlitePool, id: &str, providers: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO runner_pools (id,installation_id,name,scope,provider,providers,labels,image,docker_image,tart_image,created_at,updated_at)
         VALUES (?,1,?,'organization','docker',?,'[]','runner:fixture','runner:fixture','macos',1,1)",
    )
    .bind(id)
    .bind(id)
    .bind(providers)
    .execute(db)
    .await?;
    Ok(())
}

async fn seed_profile(
    db: &SqlitePool,
    pool_id: &str,
    enabled: bool,
    placement: &str,
    archived: bool,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO pool_execution_profiles
           (id,pool_id,name,enabled,runtime_kind,execution_os,architecture,mode,image,cpu_millis,memory_mib,disk_bytes,created_at,updated_at,placement,archived_at)
         VALUES (?,?,'p',?,'docker','linux','x64','ephemeral','runner:fixture',2000,2048,0,1,1,?,?)",
    )
    .bind(format!("profile-{pool_id}-{placement}-{enabled}-{archived}"))
    .bind(pool_id)
    .bind(enabled)
    .bind(placement)
    .bind(archived.then_some(5_i64))
    .execute(db)
    .await?;
    Ok(())
}

async fn seed_runner(
    db: &SqlitePool,
    pool_id: &str,
    container: Option<&str>,
    status: &str,
    deleted: bool,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO runners (id,pool_id,container_id,name,status,deleted_at,created_at,updated_at)
         VALUES (?,?,?,?,?,?,1,1)",
    )
    .bind(format!("runner-{}", uuid::Uuid::new_v4()))
    .bind(pool_id)
    .bind(container)
    .bind("r")
    .bind(status)
    .bind(deleted.then_some(9_i64))
    .execute(db)
    .await?;
    Ok(())
}

/// Runs the migration again after removing its row, as if it were applied to
/// the data seeded so far.
async fn decide_again(db: &SqlitePool) -> Result<bool> {
    sqlx::query("DELETE FROM settings WHERE key=?")
        .bind(builtin_host::SETTING_KEY)
        .execute(db)
        .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/0039_builtin_runner_host_setting.sql"
    ))
    .execute(db)
    .await?;
    Ok(builtin_host::is_enabled(db).await)
}

#[tokio::test]
async fn a_new_install_starts_with_the_built_in_host_off() -> Result<()> {
    let (db, directory) = database().await?;
    assert!(!builtin_host::is_enabled(&db).await);
    let stored: String = sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
        .bind(builtin_host::SETTING_KEY)
        .fetch_one(&db)
        .await?;
    assert_eq!(stored, "false");
    db.close().await;
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}

#[tokio::test]
async fn an_install_with_only_fleet_pools_and_no_built_in_runners_starts_with_the_host_off()
-> Result<()> {
    let (db, directory) = database().await?;
    seed_installation(&db).await?;
    seed_pool(&db, "fleet", r#"["docker"]"#).await?;
    seed_profile(&db, "fleet", true, "machines", false).await?;
    // Runners on machines have no container on this host, and finished ones don't count.
    seed_runner(&db, "fleet", None, "online", false).await?;
    seed_runner(&db, "fleet", Some("old"), "online", true).await?;
    seed_runner(&db, "fleet", Some("failed"), "failed", false).await?;
    assert!(!decide_again(&db).await?);
    db.close().await;
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}

#[tokio::test]
async fn an_install_keeps_the_built_in_host_on_while_something_could_still_use_it() -> Result<()> {
    // A pool that can start Docker runners and has no fleet execution.
    let (db, directory) = database().await?;
    seed_installation(&db).await?;
    seed_pool(&db, "legacy", r#"["docker"]"#).await?;
    assert!(decide_again(&db).await?, "pool without a fleet profile");
    // A disabled or archived profile is no fleet execution either.
    seed_profile(&db, "legacy", false, "machines", false).await?;
    seed_profile(&db, "legacy", true, "machines", true).await?;
    assert!(decide_again(&db).await?, "profile off or archived");
    db.close().await;
    let _ = std::fs::remove_dir_all(directory);

    // A fleet pool whose placement may fall back to, or start on, the host.
    for placement in ["machines_first", "host_first"] {
        let (db, directory) = database().await?;
        seed_installation(&db).await?;
        seed_pool(&db, "mixed", r#"["docker"]"#).await?;
        seed_profile(&db, "mixed", true, placement, false).await?;
        assert!(decide_again(&db).await?, "placement {placement}");
        db.close().await;
        let _ = std::fs::remove_dir_all(directory);
    }

    // A live runner with a container on this host, whatever the pool says now.
    let (db, directory) = database().await?;
    seed_installation(&db).await?;
    seed_pool(&db, "fleet", r#"["docker"]"#).await?;
    seed_profile(&db, "fleet", true, "machines", false).await?;
    seed_runner(&db, "fleet", Some("container-1"), "busy", false).await?;
    assert!(decide_again(&db).await?, "live built-in runner");
    db.close().await;
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}

#[tokio::test]
async fn pools_that_only_use_other_providers_do_not_keep_the_host_on() -> Result<()> {
    let (db, directory) = database().await?;
    seed_installation(&db).await?;
    seed_pool(&db, "mac", r#"["tart"]"#).await?;
    sqlx::query("UPDATE runner_pools SET provider='tart' WHERE id='mac'")
        .execute(&db)
        .await?;
    assert!(!decide_again(&db).await?);
    db.close().await;
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}

#[tokio::test]
async fn migrating_never_overwrites_a_choice_already_made() -> Result<()> {
    let (db, directory) = database().await?;
    seed_installation(&db).await?;
    seed_pool(&db, "legacy", r#"["docker"]"#).await?;
    sqlx::query("UPDATE settings SET value='false' WHERE key=?")
        .bind(builtin_host::SETTING_KEY)
        .execute(&db)
        .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/0039_builtin_runner_host_setting.sql"
    ))
    .execute(&db)
    .await?;
    assert!(!builtin_host::is_enabled(&db).await);
    db.close().await;
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}

#[tokio::test]
async fn a_missing_setting_counts_as_off() -> Result<()> {
    let (db, directory) = database().await?;
    sqlx::query("DELETE FROM settings WHERE key=?")
        .bind(builtin_host::SETTING_KEY)
        .execute(&db)
        .await?;
    assert!(!builtin_host::is_enabled(&db).await);
    db.close().await;
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}

#[tokio::test]
async fn changing_the_setting_reports_before_and_after_and_records_who() -> Result<()> {
    let (db, directory) = database().await?;
    seed_installation(&db).await?;
    let first = builtin_host::set_enabled(&db, "admin", true).await?;
    assert_eq!(
        (first.before, first.after, first.changed()),
        (false, true, true)
    );
    assert!(builtin_host::is_enabled(&db).await);
    let (by,): (Option<String>,) = sqlx::query_as("SELECT updated_by FROM settings WHERE key=?")
        .bind(builtin_host::SETTING_KEY)
        .fetch_one(&db)
        .await?;
    assert_eq!(by.as_deref(), Some("admin"));
    let again = builtin_host::set_enabled(&db, "admin", true).await?;
    assert!(!again.changed());
    db.close().await;
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}

#[tokio::test]
async fn pools_that_can_only_run_on_the_host_are_marked_while_it_is_off_and_released_when_it_is_on()
-> Result<()> {
    let (db, directory) = database().await?;
    seed_installation(&db).await?;
    seed_pool(&db, "legacy", r#"["docker"]"#).await?;
    seed_pool(&db, "fleet", r#"["docker"]"#).await?;
    seed_profile(&db, "fleet", true, "machines", false).await?;
    seed_pool(&db, "mac", r#"["tart"]"#).await?;
    seed_pool(&db, "mixed", r#"["docker","tart"]"#).await?;
    seed_pool(&db, "paused", r#"["docker"]"#).await?;
    sqlx::query("UPDATE runner_pools SET state='paused',paused=1 WHERE id='paused'")
        .execute(&db)
        .await?;

    assert!(builtin_host::pool_blocked(&db, "legacy").await?);
    assert!(!builtin_host::pool_blocked(&db, "fleet").await?);
    assert!(!builtin_host::pool_blocked(&db, "mac").await?);
    assert!(
        !builtin_host::pool_blocked(&db, "mixed").await?,
        "tart can still run it"
    );

    builtin_host::set_enabled(&db, "admin", false).await?;
    builtin_host::sync_pool_states(&db).await?;
    let state = |id: &'static str| {
        let db = db.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT state FROM runner_pools WHERE id=?")
                .bind(id)
                .fetch_one(&db)
                .await
        }
    };
    assert_eq!(state("legacy").await?, builtin_host::POOL_STATE);
    assert_eq!(state("fleet").await?, "active");
    assert_eq!(state("mac").await?, "active");
    assert_eq!(
        state("paused").await?,
        "paused",
        "a paused pool stays paused"
    );

    // Giving the pool a fleet profile releases it.
    seed_profile(&db, "legacy", true, "machines", false).await?;
    builtin_host::sync_pool_states(&db).await?;
    assert_eq!(state("legacy").await?, "active");

    // Turning the host back on releases the rest.
    seed_pool(&db, "again", r#"["docker"]"#).await?;
    builtin_host::sync_pool_states(&db).await?;
    assert_eq!(state("again").await?, builtin_host::POOL_STATE);
    builtin_host::set_enabled(&db, "admin", true).await?;
    builtin_host::sync_pool_states(&db).await?;
    assert_eq!(state("again").await?, "active");
    db.close().await;
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}

// The built-in host's budget: how much of it GridOps may fill with runners.
// Stored with the other settings; while none is stored the runner manager's
// own figures (its environment, or 95% of the host) apply.

#[tokio::test]
async fn with_no_budget_stored_the_managers_own_figures_apply() -> Result<()> {
    let (db, _directory) = database().await?;
    assert_eq!(
        builtin_host::budget(&db).await,
        builtin_host::Budget::default()
    );
    assert_eq!(builtin_host::budget(&db).await.cpu_millis, None);
    assert_eq!(builtin_host::budget(&db).await.memory_mib, None);
    Ok(())
}

#[tokio::test]
async fn a_budget_is_stored_read_back_and_cleared_and_each_change_reports_before_and_after()
-> Result<()> {
    let (db, _directory) = database().await?;
    seed_installation(&db).await?;
    let set = builtin_host::Budget {
        cpu_millis: Some(12_000),
        memory_mib: Some(49_152),
    };
    let change = builtin_host::set_budget(&db, "admin", set).await?;
    assert_eq!(change.before, builtin_host::Budget::default());
    assert_eq!(change.after, set);
    assert!(change.changed());
    assert_eq!(builtin_host::budget(&db).await, set);
    let by: String = sqlx::query_scalar("SELECT updated_by FROM settings WHERE key=?")
        .bind(builtin_host::BUDGET_KEY)
        .fetch_one(&db)
        .await?;
    assert_eq!(by, "admin");

    // Only memory goes back to the manager's own figure.
    let partly = builtin_host::Budget {
        cpu_millis: Some(12_000),
        memory_mib: None,
    };
    let change = builtin_host::set_budget(&db, "admin", partly).await?;
    assert_eq!((change.before, change.after), (set, partly));
    assert_eq!(builtin_host::budget(&db).await, partly);

    let same = builtin_host::set_budget(&db, "admin", partly).await?;
    assert!(!same.changed());
    Ok(())
}

#[tokio::test]
async fn an_unreadable_stored_budget_counts_as_none() -> Result<()> {
    let (db, _directory) = database().await?;
    sqlx::query("INSERT INTO settings (key,value,updated_at) VALUES (?,?,1)")
        .bind(builtin_host::BUDGET_KEY)
        .bind("\"not a budget\"")
        .execute(&db)
        .await?;
    assert_eq!(
        builtin_host::budget(&db).await,
        builtin_host::Budget::default()
    );
    Ok(())
}

#[tokio::test]
async fn the_manager_policy_carries_the_setting_and_the_budget() -> Result<()> {
    let (db, _directory) = database().await?;
    seed_installation(&db).await?;
    // Nothing stored: the budget is sent as nulls, which tells the manager to
    // use its own figures (and undoes an earlier setting).
    let policy = builtin_host::manager_policy(&db, false).await;
    assert_eq!(
        policy,
        serde_json::json!({
            "provisioningPaused": false,
            "builtinHostEnabled": false,
            "builtinHostBudget": {"cpuMillis": null, "memoryMib": null},
        })
    );
    builtin_host::set_enabled(&db, "admin", true).await?;
    builtin_host::set_budget(
        &db,
        "admin",
        builtin_host::Budget {
            cpu_millis: Some(8_000),
            memory_mib: None,
        },
    )
    .await?;
    let policy = builtin_host::manager_policy(&db, true).await;
    assert_eq!(
        policy,
        serde_json::json!({
            "provisioningPaused": true,
            "builtinHostEnabled": true,
            "builtinHostBudget": {"cpuMillis": 8000, "memoryMib": null},
        })
    );
    Ok(())
}
