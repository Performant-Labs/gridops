//! Proves authorization and projections share one held `SQLite` read snapshot.
//! Independent WAL writers may commit revocation without mixing policy epochs within a read.

use super::*;
use crate::Vault;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use std::time::Duration;

#[tokio::test]
async fn held_snapshot_keeps_role_grant_and_projection_consistent() -> anyhow::Result<()> {
    let directory = std::env::temp_dir().join(format!("gridops-query-snapshot-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&directory)?;
    let options = SqliteConnectOptions::new()
        .filename(directory.join("proof.sqlite"))
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(3));
    let readers = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options.clone())
        .await?;
    sqlx::migrate!("../../migrations").run(&readers).await?;
    let writers = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await?;
    sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('snapshot-reader',9100,'snapshot-reader','member','',1,1,1)").execute(&readers).await?;
    sqlx::query("INSERT INTO fleet_control_plane (singleton,incarnation,readiness,schema_version,protocol_min,protocol_max,updated_at) VALUES (1,'10000000-0000-4000-8000-000000000001','paused',1,1,1,1)").execute(&readers).await?;
    sqlx::query("INSERT INTO fleet_hosts (id,name,host_os,architecture,enrollment_state,lifecycle_state,integrity_state,created_at,updated_at) VALUES ('20000000-0000-4000-8000-000000000001','before','windows','x64','approved','active','verified',1,1)").execute(&readers).await?;
    sqlx::query("INSERT INTO host_resource_domains (id,host_id,parent_domain_id,name,cpu_millis,memory_mib,disk_bytes) VALUES ('21000000-0000-4000-8000-000000000001','20000000-0000-4000-8000-000000000001',NULL,'root',1000,1000,1000)").execute(&readers).await?;
    sqlx::query("INSERT INTO host_user_grants(host_id,user_id,permission) VALUES ('20000000-0000-4000-8000-000000000001','snapshot-reader','read')").execute(&readers).await?;
    let service = RegistryService::new(
        readers.clone(),
        Vault::test_with_secret(b"snapshot-synthetic"),
    );
    let actor = Principal::AuthenticatedUser {
        user_id: "snapshot-reader".to_owned(),
    };
    let host: HostId = "20000000-0000-4000-8000-000000000001".parse()?;
    for role in ["member", "admin"] {
        sqlx::query("UPDATE users SET role=? WHERE id='snapshot-reader'")
            .bind(role)
            .execute(&writers)
            .await?;
        sqlx::query("UPDATE host_user_grants SET revoked_at=? WHERE user_id='snapshot-reader'")
            .bind(if role == "member" { None } else { Some(2_i64) })
            .execute(&writers)
            .await?;
        sqlx::query("UPDATE fleet_hosts SET name='before'")
            .execute(&writers)
            .await?;
        sqlx::query("UPDATE host_resource_domains SET cpu_millis=1000")
            .execute(&writers)
            .await?;
        // Uses the production snapshot opener: role SELECT establishes SQLite read snapshot.
        let (mut tx, snapshot) = service.open_snapshot(&actor).await?;
        assert_eq!(snapshot.system_admin, role == "admin");
        let mut write = writers.begin().await?;
        sqlx::query("UPDATE users SET role='member' WHERE id='snapshot-reader'")
            .execute(&mut *write)
            .await?;
        sqlx::query("UPDATE host_user_grants SET revoked_at=3 WHERE user_id='snapshot-reader'")
            .execute(&mut *write)
            .await?;
        sqlx::query("UPDATE fleet_hosts SET name='after'")
            .execute(&mut *write)
            .await?;
        sqlx::query("UPDATE host_resource_domains SET cpu_millis=2000")
            .execute(&mut *write)
            .await?;
        tokio::time::timeout(Duration::from_secs(2), write.commit()).await??;
        // Production authorization and projection helpers continue through the held transaction.
        let targets = RegistryService::readable_targets(&mut tx, &snapshot)?;
        let visibility = host_visibility(&mut tx, &snapshot, &targets, host, None).await?;
        assert_eq!(visibility, HostVisibility::Physical);
        let item = service
            .host_projection(&mut tx, &snapshot, &targets, host, visibility, None)
            .await?;
        let json = serde_json::to_value(item)?;
        assert_eq!(json["name"], "before");
        assert_eq!(json["physicalBudget"]["cpuMillis"], 1000);
        tx.commit().await?;
        assert!(matches!(
            service.get_host(&actor, host).await,
            Err(QueryError::NotFound)
        ));
        sqlx::query("UPDATE host_user_grants SET revoked_at=NULL WHERE user_id='snapshot-reader'")
            .execute(&writers)
            .await?;
        let fresh = serde_json::to_value(service.get_host(&actor, host).await?.item)?;
        assert_eq!(fresh["name"], "after");
        assert_eq!(fresh["physicalBudget"]["cpuMillis"], 2000);
    }
    readers.close().await;
    writers.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
