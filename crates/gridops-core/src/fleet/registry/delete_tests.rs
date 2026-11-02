//! Deleting a machine: what is refused and why, what a delete keeps and
//! removes, and that a deleted machine stays dead.

use super::*;
use crate::fleet::protocol::browser::HostDeleteBlocker as Blocker;

const MEMBER: &str = "registry-member";

/// An approved, retired machine, as the Retire button leaves it.
async fn retired_machine(fixture: &Fixture) -> Result<(HostId, String)> {
    let issue = fixture.issue_empty("delete-host").await?;
    let enrolled = fixture.consume(&issue).await?;
    let id = enrolled.host_id.to_string();
    sqlx::query(
        "UPDATE fleet_hosts SET enrollment_state='approved',lifecycle_state='paused' WHERE id=?",
    )
    .bind(&id)
    .execute(&fixture.pool)
    .await?;
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM fleet_hosts WHERE id=?")
        .bind(&id)
        .fetch_one(&fixture.pool)
        .await?;
    fixture
        .service
        .retire_host(
            &Fixture::principal(),
            enrolled.host_id,
            Revision::new(u64::try_from(revision)?)?,
        )
        .await?;
    Ok((enrolled.host_id, enrolled.credential.expose().to_owned()))
}

async fn refusal(fixture: &Fixture, host: HostId) -> Result<Vec<Blocker>> {
    match fixture
        .service
        .delete_host(&Fixture::principal(), host)
        .await
    {
        Err(RegistryError::HostNotDeletable(reasons)) => Ok(reasons),
        other => anyhow::bail!("expected a refusal, got {other:?}"),
    }
}

async fn deleted_at(fixture: &Fixture, host: HostId) -> Result<Option<i64>> {
    Ok(
        sqlx::query_scalar("SELECT deleted_at FROM fleet_hosts WHERE id=?")
            .bind(host.to_string())
            .fetch_one(&fixture.pool)
            .await?,
    )
}

#[tokio::test]
async fn an_approved_machine_in_service_is_refused_until_it_is_retired() -> Result<()> {
    let fixture = Fixture::new(1_000_000).await?;
    let issue = fixture.issue_empty("in-service").await?;
    let enrolled = fixture.consume(&issue).await?;
    let id = enrolled.host_id.to_string();
    sqlx::query(
        "UPDATE fleet_hosts SET enrollment_state='approved',lifecycle_state='active' WHERE id=?",
    )
    .bind(&id)
    .execute(&fixture.pool)
    .await?;
    // Not retired, and its credential is live; both are named.
    assert_eq!(
        refusal(&fixture, enrolled.host_id).await?,
        [Blocker::NotRetired, Blocker::ActiveCredential]
    );
    assert_eq!(deleted_at(&fixture, enrolled.host_id).await?, None);
    // Offline does not change that: an approved machine is retired first.
    fixture.clock.set(10_000_000);
    assert_eq!(
        refusal(&fixture, enrolled.host_id).await?,
        [Blocker::NotRetired, Blocker::ActiveCredential]
    );
    fixture.close().await
}

#[tokio::test]
async fn a_retired_machine_is_deleted_hidden_and_audited_with_its_history_intact() -> Result<()> {
    let fixture = Fixture::new(1_000_000).await?;
    fixture.install_target().await?;
    let (host, credential) = retired_machine(&fixture).await?;
    let id = host.to_string();
    sqlx::query("UPDATE fleet_hosts SET last_heartbeat_at=123 WHERE id=?")
        .bind(&id)
        .execute(&fixture.pool)
        .await?;
    fixture.seed_history(host).await?;
    // The history is finished work: cleaned, released, answered.
    sqlx::query(
        "UPDATE workload_placements SET local_absence_proven_at=1,delayed_starts_excluded_at=1 WHERE id='registry-placement'",
    )
    .execute(&fixture.pool)
    .await?;
    sqlx::query("UPDATE workload_placements SET state='cleaned' WHERE id='registry-placement'")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("UPDATE capacity_allocations SET state='released',released_at=1 WHERE id='registry-allocation'")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("UPDATE host_commands SET state='succeeded' WHERE id='registry-command'")
        .execute(&fixture.pool)
        .await?;
    let revision_before: i64 = sqlx::query_scalar("SELECT revision FROM fleet_hosts WHERE id=?")
        .bind(&id)
        .fetch_one(&fixture.pool)
        .await?;

    let deleted = fixture
        .service
        .delete_host(&Fixture::principal(), host)
        .await?;
    assert_eq!(deleted.host_id, host);
    assert_eq!(deleted.name, "registry-host");

    let row: (Option<i64>, String, String, i64) = sqlx::query_as(
        "SELECT deleted_at,enrollment_state,lifecycle_state,revision FROM fleet_hosts WHERE id=?",
    )
    .bind(&id)
    .fetch_one(&fixture.pool)
    .await?;
    assert_eq!(row.0, Some(1_000_000));
    assert_eq!((row.1.as_str(), row.2.as_str()), ("revoked", "retired"));
    assert_eq!(row.3, revision_before + 1);

    // History stays: the row, its resource domain and budget, placements,
    // commands, samples and the events that already named it.
    for (table, minimum) in [
        ("host_resource_domains", 1),
        ("workload_placements", 1),
        ("capacity_allocations", 1),
        ("host_commands", 1),
        ("host_samples", 1),
        ("fleet_events", 2),
    ] {
        let rows: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT COUNT(*) FROM {table} WHERE host_id=?"
        )))
        .bind(&id)
        .fetch_one(&fixture.pool)
        .await?;
        assert!(rows >= minimum, "{table} kept {rows} rows");
    }
    let live_credentials: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM host_credentials WHERE host_id=? AND revoked_at IS NULL",
    )
    .bind(&id)
    .fetch_one(&fixture.pool)
    .await?;
    assert_eq!(live_credentials, 0);
    let enabled_backends: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM host_backends WHERE host_id=? AND enabled=1")
            .bind(&id)
            .fetch_one(&fixture.pool)
            .await?;
    assert_eq!(enabled_backends, 0);

    // The audit row and the fleet event name the actor, the machine and its
    // last heartbeat.
    let audit: (Option<String>, String, String, String) = sqlx::query_as(
        "SELECT actor_user_id,actor_label,target_id,metadata FROM audit_events WHERE action='fleet.host.deleted'",
    )
    .fetch_one(&fixture.pool)
    .await?;
    assert_eq!(audit.0.as_deref(), Some(ADMIN));
    assert_eq!(audit.1, ADMIN);
    assert_eq!(audit.2, id);
    let metadata: serde_json::Value = serde_json::from_str(&audit.3)?;
    assert_eq!(metadata["name"], "registry-host");
    assert_eq!(metadata["hostId"], id);
    assert_eq!(metadata["lastHeartbeatAt"], 123);
    let event: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT host_id,actor_user_id FROM fleet_events WHERE kind='fleet.host.deleted'",
    )
    .fetch_one(&fixture.pool)
    .await?;
    assert_eq!(event, (Some(id.clone()), Some(ADMIN.to_owned())));

    // A deleted machine is gone for every later operation.
    assert!(matches!(
        fixture
            .service
            .delete_host(&Fixture::principal(), host)
            .await,
        Err(RegistryError::NotFound)
    ));
    assert!(matches!(
        fixture
            .service
            .retire_host(&Fixture::principal(), host, Revision::new(1)?)
            .await,
        Err(RegistryError::NotFound)
    ));
    assert!(matches!(
        fixture.service.authenticate(host, &credential).await,
        Err(RegistryError::InvalidCredential)
    ));
    fixture.close().await
}

#[tokio::test]
async fn each_kind_of_live_work_is_refused_by_name_and_allowed_once_cleaned_up() -> Result<()> {
    let fixture = Fixture::new(1_000_000).await?;
    fixture.install_target().await?;
    let (host, _credential) = retired_machine(&fixture).await?;
    fixture.seed_history(host).await?;

    // A running placement holding capacity, and a command still waiting.
    assert_eq!(
        refusal(&fixture, host).await?,
        [Blocker::LivePlacement, Blocker::PendingCommand]
    );

    // An unconfirmed start is its own reason.
    sqlx::query("UPDATE workload_placements SET state='uncertain' WHERE id='registry-placement'")
        .execute(&fixture.pool)
        .await?;
    assert_eq!(
        refusal(&fixture, host).await?,
        [
            Blocker::LivePlacement,
            Blocker::UnconfirmedStart,
            Blocker::PendingCommand
        ]
    );

    // A placement that stopped, whose runner has not finished and whose
    // capacity is not released.
    sqlx::query("UPDATE workload_placements SET state='stopped' WHERE id='registry-placement'")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("UPDATE host_commands SET state='succeeded' WHERE id='registry-command'")
        .execute(&fixture.pool)
        .await?;
    assert_eq!(refusal(&fixture, host).await?, [Blocker::LivePlacement]);

    // Prove it gone and release it: the placement is cleaned, but its runner
    // row has not finished.
    sqlx::query(
        "UPDATE workload_placements SET local_absence_proven_at=1,delayed_starts_excluded_at=1 WHERE id='registry-placement'",
    )
    .execute(&fixture.pool)
    .await?;
    sqlx::query("UPDATE capacity_allocations SET state='released',released_at=1 WHERE id='registry-allocation'")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("UPDATE workload_placements SET state='cleaned' WHERE id='registry-placement'")
        .execute(&fixture.pool)
        .await?;
    sqlx::query(
        "INSERT INTO runners (id,pool_id,name,provider,ci_platform,os,architecture,status,ephemeral,configuration_version,created_at,updated_at)
         VALUES ('registry-workload','registry-pool','r','docker','github','linux','X64','online',1,1,1,1)",
    )
    .execute(&fixture.pool)
    .await?;
    assert_eq!(refusal(&fixture, host).await?, [Blocker::LiveRunner]);

    // A command past its deadline can no longer be honoured and does not
    // block; one still within its deadline does.
    sqlx::query("UPDATE runners SET status='deleted' WHERE id='registry-workload'")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("UPDATE host_commands SET state='pending' WHERE id='registry-command'")
        .execute(&fixture.pool)
        .await?;
    assert_eq!(refusal(&fixture, host).await?, [Blocker::PendingCommand]);
    fixture.clock.set(1_000_000 + 2_000);
    fixture
        .service
        .delete_host(&Fixture::principal(), host)
        .await?;
    assert!(deleted_at(&fixture, host).await?.is_some());
    fixture.close().await
}

#[tokio::test]
async fn a_pending_machine_that_is_still_reporting_is_refused_until_it_goes_quiet() -> Result<()> {
    let fixture = Fixture::new(1_000_000).await?;
    let issue = fixture.issue_empty("pending-duplicate").await?;
    let enrolled = fixture.consume(&issue).await?;
    let id = enrolled.host_id.to_string();
    sqlx::query("UPDATE fleet_hosts SET last_heartbeat_at=? WHERE id=?")
        .bind(1_000_000 - 30_000)
        .bind(&id)
        .execute(&fixture.pool)
        .await?;
    assert_eq!(
        refusal(&fixture, enrolled.host_id).await?,
        [Blocker::StillHeartbeating]
    );
    assert_eq!(deleted_at(&fixture, enrolled.host_id).await?, None);

    // Quiet for longer than the offline threshold: the delete revokes the
    // credential it had, so the agent is turned away.
    fixture.clock.set(1_000_000 + 61_000);
    fixture
        .service
        .delete_host(&Fixture::principal(), enrolled.host_id)
        .await?;
    assert!(matches!(
        fixture
            .service
            .authenticate(enrolled.host_id, enrolled.credential.expose())
            .await,
        Err(RegistryError::InvalidCredential)
    ));
    fixture.close().await
}

#[tokio::test]
async fn a_member_cannot_delete_and_nothing_changes() -> Result<()> {
    let fixture = Fixture::new(1_000_000).await?;
    sqlx::query(
        "INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at)
         VALUES (?,100,'member','member','',1,1,1)",
    )
    .bind(MEMBER)
    .execute(&fixture.pool)
    .await?;
    let (host, _credential) = retired_machine(&fixture).await?;
    let before = fixture.audit_event_count().await?;
    let member = Principal::AuthenticatedUser {
        user_id: MEMBER.to_owned(),
    };
    assert!(matches!(
        fixture.service.delete_host(&member, host).await,
        Err(RegistryError::Forbidden)
    ));
    assert!(matches!(
        fixture
            .service
            .delete_host(&Principal::Reconciler, host)
            .await,
        Err(RegistryError::Forbidden)
    ));
    assert_eq!(deleted_at(&fixture, host).await?, None);
    assert_eq!(fixture.audit_event_count().await?, before);
    fixture.close().await
}

#[tokio::test]
async fn a_token_delete_names_the_token_in_the_audit_row() -> Result<()> {
    let fixture = Fixture::new(1_000_000).await?;
    let (host, _credential) = retired_machine(&fixture).await?;
    let principal = Principal::ApiToken {
        user_id: ADMIN.to_owned(),
        token_id: "token-1".to_owned(),
        token_name: "cleanup-bot".to_owned(),
    };
    fixture.service.delete_host(&principal, host).await?;
    let (label, metadata): (String, String) = sqlx::query_as(
        "SELECT actor_label,metadata FROM audit_events WHERE action='fleet.host.deleted'",
    )
    .fetch_one(&fixture.pool)
    .await?;
    assert!(label.contains("(token: cleanup-bot)"), "{label}");
    let metadata: serde_json::Value = serde_json::from_str(&metadata)?;
    assert_eq!(metadata["viaApiToken"]["name"], "cleanup-bot");
    fixture.close().await
}

#[tokio::test]
async fn a_deleted_machine_cannot_be_recovered_rotated_or_brought_back() -> Result<()> {
    let fixture = Fixture::new(1_000_000).await?;
    let issue = fixture.issue_empty("deleted-recovery").await?;
    let enrolled = fixture.consume(&issue).await?;
    let id = enrolled.host_id.to_string();
    // A pending machine with an open recovery code: deleting revokes the code.
    let recovery = fixture
        .service
        .recover_host(
            &Fixture::principal(),
            RecoverHost {
                host_id: enrolled.host_id,
                expected_revision: Revision::new(1)?,
                idempotency_key: "open-recovery".to_owned(),
            },
        )
        .await?;
    let RecoveryIssue::Created(recovery) = recovery else {
        anyhow::bail!("expected a new recovery code")
    };
    fixture
        .service
        .delete_host(&Fixture::principal(), enrolled.host_id)
        .await?;

    // The open code died with the machine, and cannot recreate it.
    assert!(matches!(
        fixture.consume_recovery(&recovery).await,
        Err(RegistryError::EnrollmentGone)
    ));
    let hosts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM fleet_hosts")
        .fetch_one(&fixture.pool)
        .await?;
    assert_eq!(hosts, 1, "no record was recreated");
    assert!(matches!(
        fixture
            .service
            .recover_host(
                &Fixture::principal(),
                RecoverHost {
                    host_id: enrolled.host_id,
                    expected_revision: Revision::new(2)?,
                    idempotency_key: "late-recovery".to_owned(),
                },
            )
            .await,
        Err(RegistryError::NotFound)
    ));
    assert!(matches!(
        fixture
            .service
            .request_rotation(
                &Fixture::principal(),
                RequestRotation {
                    host_id: enrolled.host_id,
                    expected_generation: Generation::new(1)?,
                    idempotency_key: "late-rotation".to_owned(),
                },
            )
            .await,
        Err(RegistryError::Forbidden)
    ));

    // The schema itself refuses to revive it or to change it back.
    for statement in [
        "UPDATE fleet_hosts SET deleted_at=NULL WHERE id=?",
        "UPDATE fleet_hosts SET enrollment_state='approved' WHERE id=?",
        "UPDATE fleet_hosts SET lifecycle_state='paused' WHERE id=?",
    ] {
        assert!(
            sqlx::query(statement)
                .bind(&id)
                .execute(&fixture.pool)
                .await
                .is_err(),
            "{statement} must be refused"
        );
    }
    fixture.close().await
}

#[tokio::test]
async fn the_schema_refuses_to_mark_a_busy_or_credentialed_machine_deleted() -> Result<()> {
    let fixture = Fixture::new(1_000_000).await?;
    fixture.install_target().await?;
    let issue = fixture.issue_empty("direct-sql").await?;
    let enrolled = fixture.consume(&issue).await?;
    let id = enrolled.host_id.to_string();
    let mark = "UPDATE fleet_hosts SET deleted_at=1,enrollment_state='revoked',lifecycle_state='retired' WHERE id=?";
    // Live credential.
    assert!(
        sqlx::query(mark)
            .bind(&id)
            .execute(&fixture.pool)
            .await
            .is_err()
    );
    sqlx::query("UPDATE host_credentials SET revoked_at=1 WHERE host_id=?")
        .bind(&id)
        .execute(&fixture.pool)
        .await?;
    // Live placement.
    fixture.seed_history(enrolled.host_id).await?;
    assert!(
        sqlx::query(mark)
            .bind(&id)
            .execute(&fixture.pool)
            .await
            .is_err()
    );
    // Not retired and revoked.
    assert!(
        sqlx::query("UPDATE fleet_hosts SET deleted_at=1 WHERE id=?")
            .bind(&id)
            .execute(&fixture.pool)
            .await
            .is_err()
    );
    fixture.close().await
}
