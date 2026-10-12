//! File-backed WAL checks for the read-only query boundary.
//!
//! These tests exercise real migrations and the service methods so cursor
//! privacy, authorization, projection shape, and page continuation are not
//! inferred from DTO-only unit tests.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use uuid::Uuid;

use super::{super::authorization::Principal, RegistryClock, RegistryService};
use crate::{
    Vault,
    fleet::protocol::{
        browser::OpaqueCursor,
        browser::ResourceAmountDto,
        primitives::{BrowserCounter, BrowserUint53, Sha256Digest, WireTimestamp},
        query::{
            AllocationReadV1, ApprovalEvidenceV1, BackendReadV1, CatalogQuery, CountScope,
            EventQuery, FleetRecordCounts, HostReadV1, InventoryEvidenceV1, ObservedCapacityV1,
            QueryPage, QueryResult, SampleProvenanceV1, SampleQuery, TargetFleetRecordCounts,
        },
    },
};

#[path = "query_approval_tests.rs"]
mod approval_matrix;

struct Fixture {
    directory: PathBuf,
    pool: SqlitePool,
    service: RegistryService,
}

#[derive(Debug)]
struct QueryClock(AtomicI64);

impl QueryClock {
    fn new(now: i64) -> Arc<Self> {
        Arc::new(Self(AtomicI64::new(now)))
    }

    fn set(&self, now: i64) {
        self.0.store(now, Ordering::SeqCst);
    }
}

impl RegistryClock for QueryClock {
    fn now_millis(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

impl Fixture {
    async fn new() -> Result<Self> {
        let directory = std::env::temp_dir().join(format!("gridops-query-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory)?;
        let options = SqliteConnectOptions::new()
            .filename(directory.join("query.sqlite"))
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        sqlx::migrate!("../../migrations").run(&pool).await?;
        sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('query-admin',999,'query-admin','admin','',1,1,1)").execute(&pool).await?;
        sqlx::query("INSERT INTO fleet_control_plane (singleton,incarnation,readiness,schema_version,protocol_min,protocol_max,updated_at) VALUES (1,'10000000-0000-4000-8000-000000000001','paused',1,1,1,1)").execute(&pool).await?;
        sqlx::query("INSERT INTO fleet_hosts (id,name,host_os,architecture,enrollment_state,lifecycle_state,integrity_state,created_at,updated_at) VALUES ('20000000-0000-4000-8000-000000000001','Windows host','windows','x64','approved','active','verified',1,1),('30000000-0000-4000-8000-000000000001','Mac host','macos','arm64','approved','active','verified',1,1)").execute(&pool).await?;
        sqlx::query("INSERT INTO host_resource_domains (id,host_id,parent_domain_id,name,cpu_millis,memory_mib,disk_bytes) VALUES ('21000000-0000-4000-8000-000000000001','20000000-0000-4000-8000-000000000001',NULL,'Windows root',4000,8192,100000),('31000000-0000-4000-8000-000000000001','30000000-0000-4000-8000-000000000001',NULL,'Mac root',4000,8192,100000)").execute(&pool).await?;
        let service =
            RegistryService::new(pool.clone(), Vault::test_with_secret(b"query-test-secret"));
        Ok(Self {
            directory,
            pool,
            service,
        })
    }

    fn principal() -> Principal {
        Principal::AuthenticatedUser {
            user_id: "query-admin".to_owned(),
        }
    }

    async fn close(self) -> Result<()> {
        self.pool.close().await;
        std::fs::remove_dir_all(self.directory)?;
        Ok(())
    }

    async fn reopen(&self) -> Result<SqlitePool> {
        Ok(SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(self.directory.join("query.sqlite"))
                    .foreign_keys(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .busy_timeout(Duration::from_secs(5)),
            )
            .await?)
    }
}

async fn seed_current_observation(fixture: &Fixture, now: i64) -> Result<String> {
    let digest = "A".repeat(43);
    sqlx::query("UPDATE fleet_hosts SET enrollment_state='pending',lifecycle_state='paused',agent_session_id='query-observation-session',boot_id='query-observation-boot',authority_incarnation='10000000-0000-4000-8000-000000000001',current_credential_generation=1 WHERE id='20000000-0000-4000-8000-000000000001'")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_credentials (id,host_id,host_epoch,generation,verifier,created_at) VALUES ('query-observation-credential','20000000-0000-4000-8000-000000000001',1,1,?,?)")
        .bind(&digest)
        .bind(now - 1_000)
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_observation_sessions (session_id,host_id,host_epoch,boot_id,control_plane_incarnation,negotiated_protocol,initial_credential_id,initial_credential_generation,current_credential_id,current_credential_generation,initial_request_id,initial_body_digest,state,created_at,last_seen_at,lease_expires_at,handshake_received_at,handshake_lease_expires_at,handshake_state,current_inventory_sequence,current_inventory_digest,current_inventory_received_at) VALUES ('query-observation-session','20000000-0000-4000-8000-000000000001',1,'query-observation-boot','10000000-0000-4000-8000-000000000001',1,'query-observation-credential',1,'query-observation-credential',1,'query-observation-request',?,'ready',?,?,?, ?,?,'ready',7,?,?)")
        .bind(&digest)
        .bind(now - 1_000)
        .bind(now - 500)
        .bind(now + 60_000)
        .bind(now - 500)
        .bind(now + 60_000)
        .bind(&digest)
        .bind(now)
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_inventory_snapshots (host_id,authority_session_id,host_epoch,control_plane_incarnation,inventory_sequence,digest,snapshot_json,received_at,receipt_lease_expires_at,receipt_state,inventory_revision,domain_mappings_json,backend_mappings_json,interactive_mappings_json) VALUES ('20000000-0000-4000-8000-000000000001','query-observation-session',1,'10000000-0000-4000-8000-000000000001',7,?,'{}',?,?,'ready',9,'[]','[]','[]')")
        .bind(&digest)
        .bind(now)
        .bind(now + 60_000)
        .execute(&fixture.pool)
        .await?;
    sqlx::query("UPDATE fleet_hosts SET inventory_complete=1,inventory_revision=9,inventory_digest=?,inventory_epoch=1,inventory_session_id='query-observation-session',inventory_boot_id='query-observation-boot',last_inventory_at=? WHERE id='20000000-0000-4000-8000-000000000001'")
        .bind(&digest)
        .bind(now)
        .execute(&fixture.pool)
        .await?;
    Ok(digest)
}

#[tokio::test]
async fn host_page_uses_fixed_length_sealed_cursor_and_rechecks_scope() -> Result<()> {
    let fixture = Fixture::new().await?;
    let first = fixture
        .service
        .list_hosts(
            &Fixture::principal(),
            CatalogQuery {
                limit: 1,
                cursor: None,
                target_id: None,
            },
        )
        .await?;
    assert_eq!(first.items.as_slice().len(), 1);
    let cursor = first
        .next_cursor
        .clone()
        .ok_or_else(|| anyhow::anyhow!("expected continuation"))?;
    let encoded: String = cursor.clone().into();
    assert!(encoded.starts_with("q1."));
    assert!((2500..=4096).contains(&encoded.len()));
    assert!(!encoded.contains("after"));
    let second = fixture
        .service
        .list_hosts(
            &Fixture::principal(),
            CatalogQuery {
                limit: 1,
                cursor: Some(cursor),
                target_id: None,
            },
        )
        .await?;
    assert_eq!(second.items.as_slice().len(), 1);
    assert!(second.next_cursor.is_none());
    let tampered = OpaqueCursor::try_from(format!("q1.{}", "A".repeat(encoded.len() - 3)))?;
    let result = fixture
        .service
        .list_hosts(
            &Fixture::principal(),
            CatalogQuery {
                limit: 1,
                cursor: Some(tampered),
                target_id: None,
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(super::queries::QueryError::InvalidCursor)
    ));
    fixture.close().await
}

#[tokio::test]
async fn restricted_projection_is_a_distinct_shape_and_does_not_serialize_host_fields() -> Result<()>
{
    let fixture = Fixture::new().await?;
    sqlx::query("INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (1,1,'query','Organization','Organization','selected',1,1)").execute(&fixture.pool).await?;
    sqlx::query("INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at) VALUES (1,1,'query','repo','query/repo',0,'main','https://example.test/repo',1,1,1)").execute(&fixture.pool).await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,repository_id) VALUES ('40000000-0000-4000-8000-000000000001','github_repository','query:1:1',1,1)").execute(&fixture.pool).await?;
    sqlx::query("INSERT INTO host_target_grants (host_id,target_id,allow_schedule,revision) VALUES ('20000000-0000-4000-8000-000000000001','40000000-0000-4000-8000-000000000001',1,1)").execute(&fixture.pool).await?;
    // A non-admin user sees the target shape even though the host has a Linux
    // compatible backend in other fixtures; physical OS never filters it.
    sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('query-target',998,'query-target','member','',1,1,1)").execute(&fixture.pool).await?;
    sqlx::query("INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('query-target',1,'read',1)").execute(&fixture.pool).await?;
    let service = RegistryService::new(
        fixture.pool.clone(),
        Vault::test_with_secret(b"query-test-secret"),
    );
    let response = service
        .get_host(
            &Principal::AuthenticatedUser {
                user_id: "query-target".to_owned(),
            },
            "20000000-0000-4000-8000-000000000001".parse()?,
        )
        .await?;
    let value = serde_json::to_value(&response.item)?;
    assert_eq!(
        value.get("projection").and_then(serde_json::Value::as_str),
        Some("target")
    );
    assert!(value.get("physicalBudget").is_none());
    assert!(value.get("approvalEvidence").is_none());
    assert_eq!(
        value.get("hostOs").and_then(serde_json::Value::as_str),
        Some("windows")
    );
    assert!(value.get("host_os").is_none());
    let round_trip: HostReadV1 = serde_json::from_value(value.clone())?;
    assert_eq!(round_trip, response.item);
    let mut unknown = value
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("expected object projection"))?;
    unknown.insert("host_os".to_owned(), serde_json::json!("windows"));
    assert!(serde_json::from_value::<HostReadV1>(serde_json::Value::Object(unknown)).is_err());
    assert!(matches!(response.item, HostReadV1::Target { .. }));
    fixture.close().await
}

#[test]
fn query_projection_fields_use_camel_case_and_strict_round_trip() -> Result<()> {
    let counter = BrowserCounter::new(1)?;
    let uint = BrowserUint53::new(1)?;
    let timestamp = WireTimestamp::from_millis(1)?;
    let digest = Sha256Digest::of(b"query");

    let inventory = InventoryEvidenceV1::Current {
        inventory_revision: counter,
        digest,
        received_at: timestamp,
    };
    let inventory_json = serde_json::to_value(&inventory)?;
    assert_eq!(
        inventory_json.get("inventoryRevision"),
        Some(&serde_json::json!(1))
    );
    assert!(inventory_json.get("inventory_revision").is_none());
    assert_eq!(
        serde_json::from_value::<InventoryEvidenceV1>(inventory_json.clone())?,
        inventory
    );
    let mut inventory_unknown = inventory_json
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("expected inventory object"))?;
    inventory_unknown.insert("inventory_revision".to_owned(), serde_json::json!(1));
    assert!(
        serde_json::from_value::<InventoryEvidenceV1>(serde_json::Value::Object(inventory_unknown))
            .is_err()
    );

    let approval = ApprovalEvidenceV1::Available {
        expected_revision: counter,
        expected_inventory_revision: counter,
        expected_inventory_digest: digest,
        inventory_received_at: timestamp,
        observation_lease_expires_at: timestamp,
    };
    let approval_json = serde_json::to_value(&approval)?;
    assert_eq!(
        approval_json.get("expectedInventoryRevision"),
        Some(&serde_json::json!(1))
    );
    assert!(approval_json.get("expected_inventory_revision").is_none());
    assert_eq!(
        serde_json::from_value::<ApprovalEvidenceV1>(approval_json)?,
        approval
    );

    let counts = FleetRecordCounts {
        scope: CountScope::Host,
        known_managed_runner_count: uint,
        charged_ci_placement_count: uint,
        charged_fix_placement_count: uint,
        running_ci_placement_count: uint,
        running_fix_placement_count: uint,
        uncertain_placement_count: uint,
    };
    let amount = ResourceAmountDto {
        cpu_millis: uint,
        memory_mib: uint,
        disk_bytes: uint,
    };
    let backend = BackendReadV1::Host {
        id: "50000000-0000-4000-8000-000000000001".parse()?,
        host_id: "20000000-0000-4000-8000-000000000001".parse()?,
        domain_id: "21000000-0000-4000-8000-000000000001".parse()?,
        runtime_kind: crate::fleet::RuntimeKind::Docker,
        execution_os: crate::fleet::ExecutionOs::Windows,
        architecture: crate::fleet::Architecture::X64,
        enabled: true,
        policy_revision: counter,
        observed_readiness: crate::fleet::BackendReadiness::Ready,
        probe_freshness: crate::fleet::Freshness::Fresh,
        safe_reason: None,
        counts: counts.clone(),
        allocation: AllocationReadV1 {
            charged: amount.clone(),
            uncertain: amount,
        },
    };
    let backend_json = serde_json::to_value(&backend)?;
    assert_eq!(
        backend_json
            .get("hostId")
            .and_then(serde_json::Value::as_str),
        Some("20000000-0000-4000-8000-000000000001")
    );
    assert_eq!(
        backend_json
            .get("observedReadiness")
            .and_then(serde_json::Value::as_str),
        Some("ready")
    );
    assert!(backend_json.get("host_id").is_none());
    assert_eq!(
        serde_json::from_value::<BackendReadV1>(backend_json)?,
        backend
    );

    let target_counts = FleetRecordCounts {
        scope: CountScope::VisibleTargets,
        ..counts.clone()
    };
    let target_counts: TargetFleetRecordCounts = target_counts.try_into()?;
    assert_eq!(target_counts.as_counts().scope, CountScope::VisibleTargets);
    assert!(TargetFleetRecordCounts::try_from(counts.clone()).is_err());
    let target_host = HostReadV1::Target {
        id: "20000000-0000-4000-8000-000000000001".parse()?,
        name: crate::fleet::ids::UpstreamText::new("query-target")?,
        host_os: crate::fleet::ExecutionOs::Windows,
        architecture: crate::fleet::Architecture::X64,
        counts: target_counts.clone(),
    };
    let target_host_json = serde_json::to_value(&target_host)?;
    assert_eq!(
        serde_json::from_value::<HostReadV1>(target_host_json.clone())?,
        target_host
    );
    let mut invalid_target_host = target_host_json;
    invalid_target_host["counts"]["scope"] = serde_json::json!("host");
    assert!(serde_json::from_value::<HostReadV1>(invalid_target_host).is_err());
    let target_backend = BackendReadV1::Target {
        id: "50000000-0000-4000-8000-000000000001".parse()?,
        host_id: "20000000-0000-4000-8000-000000000001".parse()?,
        runtime_kind: crate::fleet::RuntimeKind::Docker,
        execution_os: crate::fleet::ExecutionOs::Windows,
        architecture: crate::fleet::Architecture::X64,
        counts: target_counts,
    };
    let target_backend_json = serde_json::to_value(&target_backend)?;
    assert_eq!(
        serde_json::from_value::<BackendReadV1>(target_backend_json.clone())?,
        target_backend
    );
    let mut invalid_target_backend = target_backend_json;
    invalid_target_backend["counts"]["scope"] = serde_json::json!("host");
    assert!(serde_json::from_value::<BackendReadV1>(invalid_target_backend).is_err());

    Ok(())
}

#[test]
fn query_envelopes_reject_unknown_schema_versions() -> Result<()> {
    let read_at = WireTimestamp::from_millis(1)?;
    let result = QueryResult::new(read_at, serde_json::json!({}));
    let mut encoded = serde_json::to_value(&result)?;
    encoded["schemaVersion"] = serde_json::json!(2);
    assert!(serde_json::from_value::<QueryResult<serde_json::Value>>(encoded).is_err());

    let page = QueryPage::new(read_at, vec![serde_json::json!({})], None)?;
    let mut encoded = serde_json::to_value(&page)?;
    encoded["schemaVersion"] = serde_json::json!(0);
    assert!(serde_json::from_value::<QueryPage<serde_json::Value>>(encoded).is_err());
    Ok(())
}

#[tokio::test]
async fn zero_limits_wrong_host_backend_and_measurement_provenance_are_checked() -> Result<()> {
    let fixture = Fixture::new().await?;
    let error = fixture
        .service
        .list_hosts(
            &Fixture::principal(),
            CatalogQuery {
                limit: 0,
                cursor: None,
                target_id: None,
            },
        )
        .await;
    assert!(matches!(
        error,
        Err(super::queries::QueryError::InvalidRequest(_))
    ));

    sqlx::query("INSERT INTO host_backends (id,host_id,domain_id,runtime_kind,execution_os,architecture,readiness,enabled,revision,created_at,updated_at) VALUES ('22000000-0000-4000-8000-000000000001','20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001','docker','linux','x64','ready',1,1,1,1),('22000000-0000-4000-8000-000000000002','20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001','tart_vm','macos','arm64','ready',1,1,1,1)")
        .execute(&fixture.pool)
        .await?;
    let wrong_host = fixture
        .service
        .get_backend(
            &Fixture::principal(),
            "30000000-0000-4000-8000-000000000001".parse()?,
            "22000000-0000-4000-8000-000000000001".parse()?,
        )
        .await;
    assert!(matches!(
        wrong_host,
        Err(super::queries::QueryError::NotFound)
    ));

    let now = crate::now_millis();
    sqlx::query("INSERT INTO host_samples (host_id,domain_id,host_epoch,observed_at,received_at,cpu_used_millis,memory_used_mib,disk_free_bytes,uptime_seconds,coverage) VALUES ('20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001',1,?, ?,NULL,0,NULL,10,'partial')")
        .bind(now - 1_000)
        .bind(now - 500)
        .execute(&fixture.pool)
        .await?;
    let detail = fixture
        .service
        .get_host(
            &Fixture::principal(),
            "20000000-0000-4000-8000-000000000001".parse()?,
        )
        .await?;
    let json = serde_json::to_value(&detail.item)?;
    assert_eq!(
        json["latestRootMeasurement"]["provenance"],
        "legacy_unknown"
    );
    assert_eq!(json["latestRootMeasurement"]["coverage"], "partial");
    assert_eq!(json["latestRootMeasurement"]["memoryUsedMib"], 0);
    assert!(json["latestRootMeasurement"]["cpuUsedMillis"].is_null());
    assert!(matches!(
        detail.item,
        HostReadV1::Host {
            latest_root_measurement: Some(_),
            ..
        }
    ));
    fixture.close().await
}

#[tokio::test]
async fn provider_identity_and_grant_revocation_are_rechecked_in_wal_snapshot() -> Result<()> {
    let fixture = Fixture::new().await?;
    sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('provider-user',1001,'provider-user','member','',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (10,10,'good-org','Organization','Organization','selected',1,1),(11,11,'other-org','Organization','Organization','selected',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at) VALUES (10,10,'good','repo','good/repo',0,'main','https://example.test/good',1,1,1),(11,11,'other','repo','other/repo',0,'main','https://example.test/other',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('provider-user',10,'read',1),('provider-user',11,'read',1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO bitbucket_connections (id,name,workspace,workspace_uuid,access_token_key,created_at,updated_at) VALUES ('bb-identity','workspace','good-workspace','{AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA}','sealed',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO bitbucket_user_grants (connection_id,user_id,permission) VALUES ('bb-identity','provider-user','read')")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,repository_id) VALUES ('41000000-0000-4000-8000-000000000001','github_repository','github-good',10,10),('41000000-0000-4000-8000-000000000002','github_repository','github-repository-mismatch',10,11)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,organization_id) VALUES ('41000000-0000-4000-8000-000000000003','github_organization','github-org-mismatch',10,11)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,bitbucket_connection_id,workspace_uuid) VALUES ('41000000-0000-4000-8000-000000000004','bitbucket_workspace','bitbucket-good','bb-identity','{aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa}'),('41000000-0000-4000-8000-000000000005','bitbucket_workspace','bitbucket-workspace-mismatch','bb-identity','BBBBBBBB-BBBB-BBBB-BBBB-BBBBBBBBBBBB')")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_target_grants (host_id,target_id,allow_schedule,revision) VALUES ('20000000-0000-4000-8000-000000000001','41000000-0000-4000-8000-000000000001',1,1),('20000000-0000-4000-8000-000000000001','41000000-0000-4000-8000-000000000004',1,1),('20000000-0000-4000-8000-000000000001','41000000-0000-4000-8000-000000000005',1,1)")
        .execute(&fixture.pool)
        .await?;
    let service = RegistryService::new(
        fixture.pool.clone(),
        Vault::test_with_secret(b"query-test-secret"),
    );
    let principal = Principal::AuthenticatedUser {
        user_id: "provider-user".to_owned(),
    };
    let hosts = service
        .list_hosts(
            &principal,
            CatalogQuery {
                limit: 25,
                cursor: None,
                target_id: None,
            },
        )
        .await?;
    assert_eq!(hosts.items.as_slice().len(), 1);
    assert!(matches!(
        hosts.items.as_slice()[0],
        HostReadV1::Target { .. }
    ));
    assert!(
        service
            .list_hosts(
                &principal,
                CatalogQuery {
                    limit: 25,
                    cursor: None,
                    target_id: Some("41000000-0000-4000-8000-000000000001".parse()?),
                },
            )
            .await
            .is_ok()
    );
    for target in [
        "41000000-0000-4000-8000-000000000002",
        "41000000-0000-4000-8000-000000000003",
        "41000000-0000-4000-8000-000000000005",
    ] {
        assert!(matches!(
            service
                .list_hosts(
                    &principal,
                    CatalogQuery {
                        limit: 25,
                        cursor: None,
                        target_id: Some(target.parse()?),
                    },
                )
                .await,
            Err(super::queries::QueryError::NotFound)
        ));
    }
    sqlx::query("UPDATE installations SET suspended_at=2 WHERE id=10")
        .execute(&fixture.pool)
        .await?;
    assert!(matches!(
        service
            .list_hosts(
                &principal,
                CatalogQuery {
                    limit: 25,
                    cursor: None,
                    target_id: Some("41000000-0000-4000-8000-000000000001".parse()?),
                },
            )
            .await,
        Err(super::queries::QueryError::NotFound)
    ));
    let bb_hosts = service
        .list_hosts(
            &principal,
            CatalogQuery {
                limit: 25,
                cursor: None,
                target_id: Some("41000000-0000-4000-8000-000000000004".parse()?),
            },
        )
        .await?;
    assert_eq!(bb_hosts.items.as_slice().len(), 1);
    sqlx::query("UPDATE bitbucket_user_grants SET revoked_at=2 WHERE connection_id='bb-identity' AND user_id='provider-user'")
        .execute(&fixture.pool)
        .await?;
    assert!(matches!(
        service
            .list_hosts(
                &principal,
                CatalogQuery {
                    limit: 25,
                    cursor: None,
                    target_id: Some("41000000-0000-4000-8000-000000000004".parse()?),
                },
            )
            .await,
        Err(super::queries::QueryError::NotFound)
    ));
    fixture.close().await
}

#[tokio::test]
async fn authorization_is_applied_before_limit_and_rechecked_between_pages() -> Result<()> {
    let fixture = Fixture::new().await?;
    sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('page-user',1002,'page-user','member','',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    for (index, granted) in [
        (1, false),
        (2, true),
        (3, false),
        (4, true),
        (5, false),
        (6, true),
    ] {
        let host_id = format!("40000000-0000-4000-8000-{index:012}");
        let domain_id = format!("40100000-0000-4000-8000-{index:012}");
        sqlx::query("INSERT INTO fleet_hosts (id,name,host_os,architecture,enrollment_state,lifecycle_state,integrity_state,created_at,updated_at) VALUES (?,?,'linux','x64','approved','active','verified',1,1)")
            .bind(&host_id)
            .bind(format!("page-{index}"))
            .execute(&fixture.pool)
            .await?;
        sqlx::query("INSERT INTO host_resource_domains (id,host_id,parent_domain_id,name,cpu_millis,memory_mib,disk_bytes) VALUES (?,?,NULL,?,100,100,100)")
            .bind(&domain_id)
            .bind(&host_id)
            .bind(format!("root-{index}"))
            .execute(&fixture.pool)
            .await?;
        if granted {
            sqlx::query("INSERT INTO host_user_grants (host_id,user_id,permission) VALUES (?,'page-user','read')")
                .bind(&host_id)
                .execute(&fixture.pool)
                .await?;
        }
    }
    let service = RegistryService::new(
        fixture.pool.clone(),
        Vault::test_with_secret(b"query-test-secret"),
    );
    let principal = Principal::AuthenticatedUser {
        user_id: "page-user".to_owned(),
    };
    let first = service
        .list_hosts(
            &principal,
            CatalogQuery {
                limit: 2,
                cursor: None,
                target_id: None,
            },
        )
        .await?;
    let first_ids = first
        .items
        .as_slice()
        .iter()
        .map(|item| match item {
            HostReadV1::Host { id, .. } | HostReadV1::Target { id, .. } => id.to_string(),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        first_ids,
        vec![
            "40000000-0000-4000-8000-000000000002",
            "40000000-0000-4000-8000-000000000004"
        ]
    );
    let cursor = first
        .next_cursor
        .clone()
        .ok_or_else(|| anyhow::anyhow!("page cursor"))?;
    let second = service
        .list_hosts(
            &principal,
            CatalogQuery {
                limit: 2,
                cursor: Some(cursor),
                target_id: None,
            },
        )
        .await?;
    assert_eq!(second.items.as_slice().len(), 1);
    assert!(second.next_cursor.is_none());
    let next_cursor = service
        .list_hosts(
            &principal,
            CatalogQuery {
                limit: 1,
                cursor: None,
                target_id: None,
            },
        )
        .await?
        .next_cursor
        .ok_or_else(|| anyhow::anyhow!("revocation page cursor"))?;
    let independent = fixture.reopen().await?;
    sqlx::query("UPDATE host_user_grants SET revoked_at=2 WHERE host_id='40000000-0000-4000-8000-000000000004' AND user_id='page-user'")
        .execute(&independent)
        .await?;
    independent.close().await;
    let after_revoke = service
        .list_hosts(
            &principal,
            CatalogQuery {
                limit: 1,
                cursor: Some(next_cursor),
                target_id: None,
            },
        )
        .await?;
    assert_eq!(after_revoke.items.as_slice().len(), 1);
    let only_id = match &after_revoke.items.as_slice()[0] {
        HostReadV1::Host { id, .. } | HostReadV1::Target { id, .. } => id.to_string(),
    };
    assert_eq!(only_id, "40000000-0000-4000-8000-000000000006");
    fixture.close().await
}

#[tokio::test]
async fn samples_use_received_windows_history_labels_and_events_hide_secrets() -> Result<()> {
    let fixture = Fixture::new().await?;
    let now = crate::now_millis();
    let from = WireTimestamp::from_millis(now - 5_000)?;
    let to = WireTimestamp::from_millis(now + 5_000)?;
    sqlx::query("INSERT INTO host_samples (host_id,domain_id,host_epoch,observed_at,received_at,cpu_used_millis,memory_used_mib,disk_free_bytes,uptime_seconds,coverage) VALUES ('20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001',1,?, ?,NULL,0,NULL,10,'partial'),('20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001',1,?, ?,4,NULL,8,NULL,'complete')")
        .bind(now + 4_000)
        .bind(now - 2_000)
        .bind(now - 4_000)
        .bind(now - 1_000)
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_credentials (id,host_id,host_epoch,generation,verifier,created_at) VALUES ('cred-history','20000000-0000-4000-8000-000000000001',1,1,?,1)")
        .bind("A".repeat(43))
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_observation_sessions (session_id,host_id,host_epoch,boot_id,control_plane_incarnation,negotiated_protocol,initial_credential_id,initial_credential_generation,current_credential_id,current_credential_generation,initial_request_id,initial_body_digest,state,created_at,last_seen_at,lease_expires_at,handshake_received_at,handshake_lease_expires_at,handshake_state) VALUES ('history-session','20000000-0000-4000-8000-000000000001',1,'boot-history','10000000-0000-4000-8000-000000000001',1,'cred-history',1,'cred-history',1,'history-request',?, 'observing',?,?,?, ?,?,'observing')")
        .bind("A".repeat(43))
        .bind(now - 4_000)
        .bind(now - 4_000)
        .bind(now + 60_000)
        .bind(now - 4_000)
        .bind(now + 60_000)
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_samples (host_id,domain_id,host_epoch,observed_at,received_at,cpu_used_millis,memory_used_mib,disk_free_bytes,uptime_seconds,coverage,authority_session_id,boot_id,control_plane_incarnation) VALUES ('20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001',1,?, ?,1,1,1,1,'complete','history-session','boot-history','10000000-0000-4000-8000-000000000001')")
        .bind(now - 3_000)
        .bind(now - 500)
        .execute(&fixture.pool)
        .await?;
    let samples = fixture
        .service
        .list_samples(
            &Fixture::principal(),
            "20000000-0000-4000-8000-000000000001".parse()?,
            SampleQuery {
                limit: 25,
                cursor: None,
                domain_id: None,
                received_from: from,
                received_to: to,
            },
        )
        .await?;
    assert_eq!(samples.items.as_slice().len(), 3);
    assert_eq!(
        samples.items.as_slice()[0].provenance,
        SampleProvenanceV1::LegacyUnknown
    );
    assert_eq!(
        samples.items.as_slice()[1].provenance,
        SampleProvenanceV1::LegacyUnknown
    );
    assert_eq!(
        samples.items.as_slice()[2].provenance,
        SampleProvenanceV1::Historical
    );
    assert_eq!(
        samples.items.as_slice()[0].observed_at.millis(),
        now + 4_000
    );
    assert!(matches!(
        fixture
            .service
            .list_samples(
                &Fixture::principal(),
                "20000000-0000-4000-8000-000000000001".parse()?,
                SampleQuery {
                    limit: 25,
                    cursor: None,
                    domain_id: None,
                    received_from: WireTimestamp::from_millis(now - 86_400_001)?,
                    received_to: to,
                },
            )
            .await,
        Err(super::queries::QueryError::InvalidRequest(_))
    ));
    assert!(matches!(
        fixture
            .service
            .list_samples(
                &Fixture::principal(),
                "20000000-0000-4000-8000-000000000001".parse()?,
                SampleQuery {
                    limit: 0,
                    cursor: None,
                    domain_id: None,
                    received_from: from,
                    received_to: to,
                },
            )
            .await,
        Err(super::queries::QueryError::InvalidRequest(_))
    ));

    sqlx::query("INSERT INTO fleet_events (id,host_id,target_id,actor_user_id,kind,detail_json,created_at) VALUES ('43000000-0000-4000-8000-000000000001','20000000-0000-4000-8000-000000000001',NULL,'query-admin','fleet.inventory.received','{\"secret\":\"must-not-cross-boundary\"}',1),('43000000-0000-4000-8000-000000000002','20000000-0000-4000-8000-000000000001',NULL,'query-admin','fleet.unknown.secret','{\"actor\":\"hidden\"}',2)")
        .execute(&fixture.pool)
        .await?;
    let service_clock = QueryClock::new(now);
    let service = RegistryService::with_clock(
        fixture.pool.clone(),
        Vault::test_with_secret(b"query-test-secret"),
        service_clock.clone(),
    );
    let first = service
        .list_host_events(
            &Fixture::principal(),
            "20000000-0000-4000-8000-000000000001".parse()?,
            EventQuery {
                limit: 1,
                cursor: None,
                occurred_from: None,
                occurred_to: None,
                target_id: None,
            },
        )
        .await?;
    assert_eq!(first.items.as_slice().len(), 1);
    let first_json = serde_json::to_value(&first.items.as_slice()[0])?;
    assert_eq!(first_json["kind"], "inventory_received");
    assert!(first_json.get("detailJson").is_none());
    assert!(first_json.get("actorUserId").is_none());
    let cursor = first
        .next_cursor
        .ok_or_else(|| anyhow::anyhow!("event cursor"))?;
    let expiry_cursor = cursor.clone();
    let second = service
        .list_host_events(
            &Fixture::principal(),
            "20000000-0000-4000-8000-000000000001".parse()?,
            EventQuery {
                limit: 1,
                cursor: Some(cursor.clone()),
                occurred_from: None,
                occurred_to: None,
                target_id: None,
            },
        )
        .await?;
    assert_eq!(
        serde_json::to_value(&second.items.as_slice()[0])?["kind"],
        "other"
    );
    sqlx::query(
        "UPDATE fleet_query_retention_floors SET first_retained=3 WHERE stream='fleet_events'",
    )
    .execute(&fixture.pool)
    .await?;
    let gap_result = service
        .list_host_events(
            &Fixture::principal(),
            "20000000-0000-4000-8000-000000000001".parse()?,
            EventQuery {
                limit: 1,
                cursor: Some(cursor),
                occurred_from: None,
                occurred_to: None,
                target_id: None,
            },
        )
        .await;
    assert!(matches!(
        gap_result,
        Err(super::queries::QueryError::CursorGap)
    ));
    service_clock.set(now + 15 * 60 * 1000 + 1);
    assert!(matches!(
        service
            .list_host_events(
                &Fixture::principal(),
                "20000000-0000-4000-8000-000000000001".parse()?,
                EventQuery {
                    limit: 1,
                    cursor: Some(expiry_cursor),
                    occurred_from: None,
                    occurred_to: None,
                    target_id: None,
                },
            )
            .await,
        Err(super::queries::QueryError::CursorExpired)
    ));
    fixture.close().await
}

#[tokio::test]
async fn current_sample_provenance_matches_latest_and_list_paths() -> Result<()> {
    let fixture = Fixture::new().await?;
    let now = 1_000_000_i64;
    let clock = QueryClock::new(now);
    let service = RegistryService::with_clock(
        fixture.pool.clone(),
        Vault::test_with_secret(b"query-test-secret"),
        clock.clone(),
    );
    seed_current_observation(&fixture, now).await?;
    sqlx::query("INSERT INTO host_samples (host_id,domain_id,host_epoch,observed_at,received_at,cpu_used_millis,memory_used_mib,disk_free_bytes,uptime_seconds,coverage,authority_session_id,boot_id,control_plane_incarnation) VALUES ('20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001',1,?, ?,1,2,3,4,'complete','query-observation-session','query-observation-boot','10000000-0000-4000-8000-000000000001')")
        .bind(now - 2_000)
        .bind(now)
        .execute(&fixture.pool)
        .await?;
    let host: crate::fleet::ids::HostId = "20000000-0000-4000-8000-000000000001".parse()?;
    let domain: crate::fleet::ids::ResourceDomainId =
        "21000000-0000-4000-8000-000000000001".parse()?;
    let latest = service
        .get_domain(&Fixture::principal(), host, domain)
        .await?;
    assert_eq!(
        latest
            .item
            .latest_measurement
            .as_ref()
            .map(|measurement| measurement.provenance),
        Some(SampleProvenanceV1::Current)
    );
    let listed = service
        .list_samples(
            &Fixture::principal(),
            host,
            SampleQuery {
                limit: 25,
                cursor: None,
                domain_id: Some(domain),
                received_from: WireTimestamp::from_millis(now)?,
                received_to: WireTimestamp::from_millis(now + 1)?,
            },
        )
        .await?;
    assert_eq!(listed.items.as_slice().len(), 1);
    assert_eq!(
        listed.items.as_slice()[0].provenance,
        SampleProvenanceV1::Current
    );
    for (offset, expected) in [
        (44_999_i64, SampleProvenanceV1::Current),
        (45_000, SampleProvenanceV1::Historical),
        (-1, SampleProvenanceV1::Historical),
    ] {
        clock.set(now + offset);
        let latest = service
            .get_domain(&Fixture::principal(), host, domain)
            .await?;
        assert_eq!(
            latest
                .item
                .latest_measurement
                .as_ref()
                .map(|measurement| measurement.provenance),
            Some(expected),
            "offset={offset}"
        );
        let listed = service
            .list_samples(
                &Fixture::principal(),
                host,
                SampleQuery {
                    limit: 25,
                    cursor: None,
                    domain_id: Some(domain),
                    received_from: WireTimestamp::from_millis(now)?,
                    received_to: WireTimestamp::from_millis(now + 1)?,
                },
            )
            .await?;
        assert_eq!(
            listed.items.as_slice()[0].provenance,
            expected,
            "offset={offset}"
        );
    }
    clock.set(now);
    sqlx::query("UPDATE fleet_hosts SET agent_session_id='wrong-session' WHERE id='20000000-0000-4000-8000-000000000001'")
        .execute(&fixture.pool)
        .await?;
    let latest = service
        .get_domain(&Fixture::principal(), host, domain)
        .await?;
    assert_eq!(
        latest
            .item
            .latest_measurement
            .as_ref()
            .map(|measurement| measurement.provenance),
        Some(SampleProvenanceV1::Historical)
    );
    let listed = service
        .list_samples(
            &Fixture::principal(),
            host,
            SampleQuery {
                limit: 25,
                cursor: None,
                domain_id: Some(domain),
                received_from: WireTimestamp::from_millis(now)?,
                received_to: WireTimestamp::from_millis(now + 1)?,
            },
        )
        .await?;
    assert_eq!(
        listed.items.as_slice()[0].provenance,
        SampleProvenanceV1::Historical
    );
    fixture.close().await
}

#[tokio::test]
async fn malformed_persisted_projection_fails_closed_and_gets_do_not_mutate_wal_state() -> Result<()>
{
    let fixture = Fixture::new().await?;
    let before: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM fleet_hosts),(SELECT COUNT(*) FROM fleet_events),(SELECT COUNT(*) FROM host_samples),(SELECT COUNT(*) FROM audit_events)",
    )
    .fetch_one(&fixture.pool)
    .await?;
    let host = fixture
        .service
        .get_host(
            &Fixture::principal(),
            "20000000-0000-4000-8000-000000000001".parse()?,
        )
        .await?;
    assert!(matches!(host.item, HostReadV1::Host { .. }));
    let after_reads: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM fleet_hosts),(SELECT COUNT(*) FROM fleet_events),(SELECT COUNT(*) FROM host_samples),(SELECT COUNT(*) FROM audit_events)",
    )
    .fetch_one(&fixture.pool)
    .await?;
    assert_eq!(before, after_reads);

    let oversized_tags = serde_json::to_string(
        &(0..65)
            .map(|index| (format!("tag-{index}"), "value".to_owned()))
            .collect::<std::collections::BTreeMap<_, _>>(),
    )?;
    sqlx::query(
        "UPDATE fleet_hosts SET tags_json=? WHERE id='20000000-0000-4000-8000-000000000001'",
    )
    .bind(oversized_tags)
    .execute(&fixture.pool)
    .await?;
    assert!(matches!(
        fixture
            .service
            .get_host(
                &Fixture::principal(),
                "20000000-0000-4000-8000-000000000001".parse()?,
            )
            .await,
        Err(super::queries::QueryError::MalformedData)
    ));
    fixture.close().await
}

#[tokio::test]
async fn approval_evidence_matches_current_observation_without_work_authority() -> Result<()> {
    let fixture = Fixture::new().await?;
    let now = crate::now_millis();
    let digest = "A".repeat(43);
    sqlx::query("UPDATE fleet_hosts SET enrollment_state='pending',lifecycle_state='paused',agent_session_id='approval-session',boot_id='approval-boot',authority_incarnation='10000000-0000-4000-8000-000000000001',current_credential_generation=1 WHERE id='20000000-0000-4000-8000-000000000001'")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("UPDATE fleet_hosts SET inventory_complete=1,inventory_revision=9,inventory_digest=?,inventory_epoch=1,inventory_session_id='approval-session',inventory_boot_id='approval-boot',last_inventory_at=? WHERE id='20000000-0000-4000-8000-000000000001'")
        .bind(&digest)
        .bind(now)
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_credentials (id,host_id,host_epoch,generation,verifier,created_at) VALUES ('approval-credential','20000000-0000-4000-8000-000000000001',1,1,?,?)")
        .bind(&digest)
        .bind(now - 1_000)
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_observation_sessions (session_id,host_id,host_epoch,boot_id,control_plane_incarnation,negotiated_protocol,initial_credential_id,initial_credential_generation,current_credential_id,current_credential_generation,initial_request_id,initial_body_digest,state,created_at,last_seen_at,lease_expires_at,handshake_received_at,handshake_lease_expires_at,handshake_state,current_inventory_sequence,current_inventory_digest,current_inventory_received_at) VALUES ('approval-session','20000000-0000-4000-8000-000000000001',1,'approval-boot','10000000-0000-4000-8000-000000000001',1,'approval-credential',1,'approval-credential',1,'approval-request',?, 'ready',?,?,?, ?,?,'ready',7,?,?)")
        .bind(&digest)
        .bind(now - 1_000)
        .bind(now - 500)
        .bind(now + 60_000)
        .bind(now - 500)
        .bind(now + 60_000)
        .bind(&digest)
        .bind(now)
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_inventory_snapshots (host_id,authority_session_id,host_epoch,control_plane_incarnation,inventory_sequence,digest,snapshot_json,received_at,receipt_lease_expires_at,receipt_state,inventory_revision,domain_mappings_json,backend_mappings_json,interactive_mappings_json) VALUES ('20000000-0000-4000-8000-000000000001','approval-session',1,'10000000-0000-4000-8000-000000000001',7,?,'{}',?,?,'ready',9,'[]','[]','[]')")
        .bind(&digest)
        .bind(now)
        .bind(now + 60_000)
        .execute(&fixture.pool)
        .await?;
    let available = fixture
        .service
        .get_host(
            &Fixture::principal(),
            "20000000-0000-4000-8000-000000000001".parse()?,
        )
        .await?;
    let value = serde_json::to_value(&available.item)?;
    assert_eq!(value["inventoryEvidence"]["state"], "current");
    assert_eq!(value["inventoryEvidence"]["inventoryRevision"], 9);
    assert_eq!(value["approvalEvidence"]["state"], "available");
    assert_eq!(value["approvalEvidence"]["expectedInventoryRevision"], 9);
    assert_eq!(value["approvalEvidence"]["expectedInventoryDigest"], digest);
    assert!(
        value["approvalEvidence"]
            .get("workAuthorityExpiresAt")
            .is_none()
    );

    sqlx::query(
        "UPDATE fleet_observation_sessions SET ended_at=? WHERE session_id='approval-session'",
    )
    .bind(now)
    .execute(&fixture.pool)
    .await?;
    let stale = fixture
        .service
        .get_host(
            &Fixture::principal(),
            "20000000-0000-4000-8000-000000000001".parse()?,
        )
        .await?;
    let stale_json = serde_json::to_value(&stale.item)?;
    assert_eq!(stale_json["inventoryEvidence"]["state"], "stale");
    assert_eq!(stale_json["approvalEvidence"]["state"], "unavailable");
    assert_eq!(stale_json["approvalEvidence"]["reason"], "stale_inventory");

    sqlx::query("UPDATE fleet_hosts SET lifecycle_state='retired' WHERE id='20000000-0000-4000-8000-000000000001'")
        .execute(&fixture.pool)
        .await?;
    let retired = fixture
        .service
        .get_host(
            &Fixture::principal(),
            "20000000-0000-4000-8000-000000000001".parse()?,
        )
        .await?;
    assert_eq!(
        serde_json::to_value(&retired.item)?["approvalEvidence"]["reason"],
        "retired_host"
    );
    fixture.close().await
}

#[tokio::test]
async fn counts_and_allocations_are_distinct_scoped_and_root_bounded() -> Result<()> {
    let fixture = Fixture::new().await?;
    sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('count-gh-a',2010,'count-gh-a','member','',1,1,1),('count-bb-b',2011,'count-bb-b','member','',1,1,1),('count-physical-read',2012,'count-physical-read','member','',1,1,1),('count-physical-operator',2013,'count-physical-operator','member','',1,1,1),('count-physical-admin',2014,'count-physical-admin','member','',1,1,1),('count-unrelated',2015,'count-unrelated','member','',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (20,20,'count-org','Organization','Organization','selected',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at) VALUES (20,20,'count','repo','count/repo',0,'main','https://example.test/count',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO bitbucket_connections (id,name,workspace,workspace_uuid,access_token_key,created_at,updated_at) VALUES ('count-bb','count-bb','count-workspace','{AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA}','count-bb-token',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('count-gh-a',20,'read',1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO bitbucket_user_grants (connection_id,user_id,permission) VALUES ('count-bb','count-bb-b','read')")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,repository_id) VALUES ('44000000-0000-4000-8000-000000000001','github_repository','count-target-a',20,20)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,bitbucket_connection_id,workspace_uuid) VALUES ('44000000-0000-4000-8000-000000000002','bitbucket_workspace','count-target-b','count-bb','{aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa}')")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_target_grants (host_id,target_id,allow_schedule,revision) VALUES ('20000000-0000-4000-8000-000000000001','44000000-0000-4000-8000-000000000001',1,1),('20000000-0000-4000-8000-000000000001','44000000-0000-4000-8000-000000000002',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_user_grants (host_id,user_id,permission) VALUES ('20000000-0000-4000-8000-000000000001','count-physical-read','read'),('20000000-0000-4000-8000-000000000001','count-physical-operator','operator'),('20000000-0000-4000-8000-000000000001','count-physical-admin','admin')")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_resource_domains (id,host_id,parent_domain_id,name,cpu_millis,memory_mib,disk_bytes) VALUES ('21100000-0000-4000-8000-000000000001','20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001','child',200,200,200)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_hosts (id,name,host_os,architecture,enrollment_state,lifecycle_state,integrity_state,created_at,updated_at) VALUES ('70000000-0000-4000-8000-000000000001','Hidden host','linux','x64','pending','paused','unverified',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_resource_domains (id,host_id,parent_domain_id,name,cpu_millis,memory_mib,disk_bytes) VALUES ('71000000-0000-4000-8000-000000000001','70000000-0000-4000-8000-000000000001',NULL,'Hidden root',100,200,300)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_backends (id,host_id,domain_id,runtime_kind,execution_os,architecture,readiness,enabled,revision,created_at,updated_at) VALUES ('72000000-0000-4000-8000-000000000001','70000000-0000-4000-8000-000000000001','71000000-0000-4000-8000-000000000001','docker','linux','x64','unknown',0,1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_backends (id,host_id,domain_id,runtime_kind,execution_os,architecture,readiness,enabled,revision,created_at,updated_at) VALUES ('22000000-0000-4000-8000-000000000001','20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001','docker','linux','x64','ready',1,1,1,1),('22000000-0000-4000-8000-000000000002','20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001','tart_vm','macos','arm64','ready',1,1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,image,created_at,updated_at) VALUES ('pool-count',20,20,'count-pool','repository','count-image',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO pool_execution_profiles (id,pool_id,name,enabled,runtime_kind,execution_os,architecture,mode,image,cpu_millis,memory_mib,disk_bytes,created_at,updated_at,all_authorized_hosts) VALUES ('profile-count','pool-count','count-profile',1,'docker','linux','x64','ephemeral','count-image',100,100,100,1,1,1),('profile-count-b','pool-count','count-profile-b',1,'tart_vm','macos','arm64','ephemeral','count-image',100,100,100,1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO profile_ci_targets (profile_id,target_id,enabled) VALUES ('profile-count','44000000-0000-4000-8000-000000000001',1),('profile-count-b','44000000-0000-4000-8000-000000000002',1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO runners (id,pool_id,name,created_at,updated_at) VALUES ('runner-count-1','pool-count','runner-1',1,1),('runner-count-2','pool-count','runner-2',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at) VALUES (920,20,'fix',1,'workflow_dispatch','completed','sha','https://example.test/run',1,1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at) VALUES (921,920,'fix','completed','[]','https://example.test/job',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO agent_runs (id,job_id,run_id,repository_id,trigger,status,created_at,updated_at) VALUES ('agent-count',921,1,20,'manual','succeeded',1,1)")
        .execute(&fixture.pool)
        .await?;
    for (operation, intent, kind, target, agent) in [
        (
            "op-count-1",
            "intent-count-1",
            "ci_runner",
            "44000000-0000-4000-8000-000000000001",
            None,
        ),
        (
            "op-count-2",
            "intent-count-2",
            "ci_runner",
            "44000000-0000-4000-8000-000000000001",
            None,
        ),
        (
            "op-count-3",
            "intent-count-3",
            "fix_sandbox",
            "44000000-0000-4000-8000-000000000001",
            Some("agent-count"),
        ),
        (
            "op-count-4",
            "intent-count-4",
            "ci_runner",
            "44000000-0000-4000-8000-000000000002",
            None,
        ),
        (
            "op-count-5",
            "intent-count-5",
            "ci_runner",
            "44000000-0000-4000-8000-000000000001",
            None,
        ),
        (
            "op-count-6",
            "intent-count-6",
            "ci_runner",
            "44000000-0000-4000-8000-000000000002",
            None,
        ),
    ] {
        let profile = if target.ends_with("0002") {
            "profile-count-b"
        } else {
            "profile-count"
        };
        sqlx::query("INSERT INTO fleet_operations (id,kind,principal_kind,principal_scope,idempotency_key,request_hash,status,created_at,updated_at) VALUES (?,'submit_workload','autoscaler','count',?,'hash','succeeded',1,1)")
            .bind(intent)
            .bind(operation)
            .execute(&fixture.pool)
            .await?;
        sqlx::query("INSERT INTO workload_intents (id,workload_id,generation,kind,pool_id,profile_id,target_id,agent_run_id,expected_profile_revision,status,created_at,updated_at) VALUES (?,?,1,?,'pool-count',?,?,?,1,'admitted',1,1)")
            .bind(intent)
            .bind(format!("workload-{intent}"))
            .bind(kind)
            .bind(profile)
            .bind(target)
            .bind(agent)
            .execute(&fixture.pool)
            .await?;
    }
    for (
        placement,
        intent,
        target,
        state,
        runner,
        agent,
        cpu,
        domain,
        allocation_state,
        released_at,
    ) in [
        (
            "45000000-0000-4000-8000-000000000001",
            "intent-count-1",
            "44000000-0000-4000-8000-000000000001",
            "running",
            Some("runner-count-1"),
            None,
            10_i64,
            "21000000-0000-4000-8000-000000000001",
            "reserved",
            None,
        ),
        (
            "45000000-0000-4000-8000-000000000002",
            "intent-count-2",
            "44000000-0000-4000-8000-000000000001",
            "stopped",
            Some("runner-count-1"),
            None,
            20,
            "21000000-0000-4000-8000-000000000001",
            "uncertain",
            None,
        ),
        (
            "45000000-0000-4000-8000-000000000003",
            "intent-count-3",
            "44000000-0000-4000-8000-000000000001",
            "running",
            None,
            Some("agent-count"),
            30,
            "21100000-0000-4000-8000-000000000001",
            "committed",
            None,
        ),
        (
            "45000000-0000-4000-8000-000000000004",
            "intent-count-4",
            "44000000-0000-4000-8000-000000000002",
            "running",
            Some("runner-count-2"),
            None,
            40,
            "21000000-0000-4000-8000-000000000001",
            "released",
            Some(2_i64),
        ),
        (
            "45000000-0000-4000-8000-000000000005",
            "intent-count-5",
            "44000000-0000-4000-8000-000000000001",
            "stopped",
            Some("runner-count-1"),
            None,
            15,
            "21000000-0000-4000-8000-000000000001",
            "releasing",
            None,
        ),
        (
            "45000000-0000-4000-8000-000000000006",
            "intent-count-6",
            "44000000-0000-4000-8000-000000000002",
            "running",
            Some("runner-count-2"),
            None,
            7,
            "21000000-0000-4000-8000-000000000001",
            "committed",
            None,
        ),
    ] {
        let backend = if target.ends_with("0002") {
            "22000000-0000-4000-8000-000000000002"
        } else {
            "22000000-0000-4000-8000-000000000001"
        };
        let host_epoch = if placement.ends_with("0005") { 2 } else { 1 };
        sqlx::query("INSERT INTO workload_placements (id,intent_id,workload_id,generation,host_id,backend_id,host_epoch,profile_revision,target_id,config_snapshot_json,cpu_millis,memory_mib,disk_bytes,state,runner_id,agent_run_id,created_at,updated_at) VALUES (?,?,?,1,'20000000-0000-4000-8000-000000000001',?,?,1,?,'{}',?,?,?, ?,?,?,1,1)")
            .bind(placement)
            .bind(intent)
            .bind(format!("workload-{intent}"))
            .bind(backend)
            .bind(host_epoch)
            .bind(target)
            .bind(cpu)
            .bind(cpu * 2)
            .bind(cpu * 3)
            .bind(state)
            .bind(runner)
            .bind(agent)
            .execute(&fixture.pool)
            .await?;
        if allocation_state == "released" {
            sqlx::query("UPDATE workload_placements SET local_absence_proven_at=1,delayed_starts_excluded_at=1 WHERE id=?")
                .bind(placement)
                .execute(&fixture.pool)
                .await?;
        }
        sqlx::query("INSERT INTO capacity_allocations (id,placement_id,host_id,backend_id,host_epoch,domain_id,policy_revision,cpu_millis,memory_mib,disk_bytes,state,created_at,updated_at,released_at) VALUES (?,?, '20000000-0000-4000-8000-000000000001',?,?,?,1,?,?,?, ?,1,1,?)")
            .bind(format!("allocation-{placement}"))
            .bind(placement)
            .bind(backend)
            .bind(host_epoch)
            .bind(domain)
            .bind(cpu)
            .bind(cpu * 2)
            .bind(cpu * 3)
            .bind(allocation_state)
            .bind(released_at)
            .execute(&fixture.pool)
            .await?;
    }
    // A second reservation on the first placement belongs to the child
    // domain and must not inflate physical-root or backend allocation.
    sqlx::query("INSERT INTO capacity_allocations (id,placement_id,host_id,backend_id,host_epoch,domain_id,policy_revision,cpu_millis,memory_mib,disk_bytes,state,created_at,updated_at) VALUES ('allocation-child','45000000-0000-4000-8000-000000000001','20000000-0000-4000-8000-000000000001','22000000-0000-4000-8000-000000000001',1,'21100000-0000-4000-8000-000000000001',1,100,200,300,'reserved',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO capacity_allocations (id,placement_id,host_id,backend_id,host_epoch,domain_id,policy_revision,cpu_millis,memory_mib,disk_bytes,state,created_at,updated_at) VALUES ('allocation-child-p6','45000000-0000-4000-8000-000000000006','20000000-0000-4000-8000-000000000001','22000000-0000-4000-8000-000000000002',1,'21100000-0000-4000-8000-000000000001',1,70,140,210,'committed',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO runners (id,pool_id,name,deleted_at,created_at,updated_at) VALUES ('runner-count-deleted','pool-count','deleted-runner',2,1,1),('runner-count-legacy','pool-count','legacy-runner',NULL,1,1)")
        .execute(&fixture.pool)
        .await
        .map_err(|error| anyhow::anyhow!("deleted/legacy runners: {error}"))?;
    sqlx::query("INSERT INTO fleet_operations (id,kind,principal_kind,principal_scope,idempotency_key,request_hash,status,created_at,updated_at) VALUES ('intent-count-7','submit_workload','autoscaler','count','op-count-7','hash','succeeded',1,1)")
        .execute(&fixture.pool)
        .await
        .map_err(|error| anyhow::anyhow!("noalloc operation: {error}"))?;
    sqlx::query("INSERT INTO workload_intents (id,workload_id,generation,kind,pool_id,profile_id,target_id,agent_run_id,expected_profile_revision,status,created_at,updated_at) VALUES ('intent-count-7','workload-intent-count-7',1,'ci_runner','pool-count','profile-count','44000000-0000-4000-8000-000000000001',NULL,1,'admitted',1,1)")
        .execute(&fixture.pool)
        .await
        .map_err(|error| anyhow::anyhow!("noalloc intent: {error}"))?;
    sqlx::query("INSERT INTO workload_placements (id,intent_id,workload_id,generation,host_id,backend_id,host_epoch,profile_revision,target_id,config_snapshot_json,cpu_millis,memory_mib,disk_bytes,state,runner_id,created_at,updated_at) VALUES ('45000000-0000-4000-8000-000000000007','intent-count-7','workload-intent-count-7',1,'20000000-0000-4000-8000-000000000001','22000000-0000-4000-8000-000000000001',1,1,'44000000-0000-4000-8000-000000000001','{}',5,10,15,'stopped','runner-count-deleted',1,1)")
        .execute(&fixture.pool)
        .await
        .map_err(|error| anyhow::anyhow!("deleted runner placement: {error}"))?;
    sqlx::query("INSERT INTO external_runner_observations (id,target_id,host_id,upstream_id,source,confidence,freshness,owner_kind,evidence_json,observed_at,updated_at) VALUES ('external-count-1','44000000-0000-4000-8000-000000000001',NULL,'external-runner-1','upstream','unknown','fresh','external','{}',1,1)")
        .execute(&fixture.pool)
        .await
        .map_err(|error| anyhow::anyhow!("external observation: {error}"))?;
    sqlx::query("INSERT INTO fleet_events (id,host_id,target_id,kind,detail_json,created_at) VALUES ('46000000-0000-4000-8000-000000000001','20000000-0000-4000-8000-000000000001',NULL,'fleet.inventory.received','{}',1),('46000000-0000-4000-8000-000000000002','20000000-0000-4000-8000-000000000001','44000000-0000-4000-8000-000000000001','fleet.placement.started','{\"secret\":\"a\"}',2),('46000000-0000-4000-8000-000000000003','20000000-0000-4000-8000-000000000001','44000000-0000-4000-8000-000000000002','fleet.placement.started','{\"secret\":\"b\"}',3)")
        .execute(&fixture.pool)
        .await
        .map_err(|error| anyhow::anyhow!("events: {error}"))?;

    let physical = fixture
        .service
        .get_host(
            &Fixture::principal(),
            "20000000-0000-4000-8000-000000000001".parse()?,
        )
        .await?;
    let physical_json = serde_json::to_value(&physical.item)?;
    assert_eq!(physical_json["counts"]["knownManagedRunnerCount"], 2);
    assert_eq!(physical_json["counts"]["chargedCiPlacementCount"], 4);
    assert_eq!(physical_json["counts"]["chargedFixPlacementCount"], 1);
    assert_eq!(physical_json["counts"]["runningCiPlacementCount"], 3);
    assert_eq!(physical_json["counts"]["runningFixPlacementCount"], 1);
    assert_eq!(physical_json["counts"]["uncertainPlacementCount"], 1);
    assert_eq!(
        physical_json["physicalAllocation"]["charged"]["cpuMillis"],
        52
    );
    assert_eq!(
        physical_json["physicalAllocation"]["charged"]["memoryMib"],
        104
    );
    assert_eq!(
        physical_json["physicalAllocation"]["charged"]["diskBytes"],
        156
    );
    assert_eq!(
        physical_json["physicalAllocation"]["uncertain"]["cpuMillis"],
        20
    );
    assert_eq!(
        physical_json["physicalAllocation"]["uncertain"]["memoryMib"],
        40
    );
    assert_eq!(
        physical_json["physicalAllocation"]["uncertain"]["diskBytes"],
        60
    );
    let backend = fixture
        .service
        .get_backend(
            &Fixture::principal(),
            "20000000-0000-4000-8000-000000000001".parse()?,
            "22000000-0000-4000-8000-000000000001".parse()?,
        )
        .await?;
    assert_eq!(
        serde_json::to_value(&backend.item)?["allocation"]["charged"]["cpuMillis"],
        45
    );
    let backend_json = serde_json::to_value(&backend.item)?;
    assert_eq!(backend_json["allocation"]["charged"]["memoryMib"], 90);
    assert_eq!(backend_json["allocation"]["charged"]["diskBytes"], 135);
    let backend_b_json = serde_json::to_value(
        &fixture
            .service
            .get_backend(
                &Fixture::principal(),
                "20000000-0000-4000-8000-000000000001".parse()?,
                "22000000-0000-4000-8000-000000000002".parse()?,
            )
            .await?
            .item,
    )?;
    assert_eq!(backend_b_json["allocation"]["charged"]["cpuMillis"], 7);
    assert_eq!(backend_b_json["allocation"]["charged"]["memoryMib"], 14);
    assert_eq!(backend_b_json["allocation"]["charged"]["diskBytes"], 21);
    let child = fixture
        .service
        .get_domain(
            &Fixture::principal(),
            "20000000-0000-4000-8000-000000000001".parse()?,
            "21100000-0000-4000-8000-000000000001".parse()?,
        )
        .await?;
    assert_eq!(
        serde_json::to_value(&child.item)?["allocation"]["charged"]["cpuMillis"],
        200
    );
    let child_json = serde_json::to_value(&child.item)?;
    assert_eq!(child_json["allocation"]["charged"]["memoryMib"], 400);
    assert_eq!(child_json["allocation"]["charged"]["diskBytes"], 600);

    for (user_id, backend_id, expected_counts) in [
        (
            "count-gh-a",
            "22000000-0000-4000-8000-000000000001",
            [1, 3, 1, 1, 1, 1],
        ),
        (
            "count-bb-b",
            "22000000-0000-4000-8000-000000000002",
            [1, 1, 0, 2, 0, 0],
        ),
    ] {
        let principal = Principal::AuthenticatedUser {
            user_id: user_id.to_owned(),
        };
        let host = fixture
            .service
            .get_host(&principal, "20000000-0000-4000-8000-000000000001".parse()?)
            .await?;
        let host_json = serde_json::to_value(&host.item)?;
        assert_eq!(host_json["projection"], "target");
        let host_keys = host_json
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("target host object"))?
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            host_keys,
            [
                "projection",
                "id",
                "name",
                "hostOs",
                "architecture",
                "counts"
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        );
        assert_eq!(host_json["counts"]["scope"], "visible_targets");
        for (field, expected) in [
            ("knownManagedRunnerCount", expected_counts[0]),
            ("chargedCiPlacementCount", expected_counts[1]),
            ("chargedFixPlacementCount", expected_counts[2]),
            ("runningCiPlacementCount", expected_counts[3]),
            ("runningFixPlacementCount", expected_counts[4]),
            ("uncertainPlacementCount", expected_counts[5]),
        ] {
            assert_eq!(host_json["counts"][field], expected, "{user_id} {field}");
        }
        let backends = fixture
            .service
            .list_backends(
                &principal,
                "20000000-0000-4000-8000-000000000001".parse()?,
                CatalogQuery::default(),
            )
            .await?;
        assert_eq!(backends.items.as_slice().len(), 1, "{user_id}");
        let backend_json = serde_json::to_value(&backends.items.as_slice()[0])?;
        assert_eq!(backend_json["id"], backend_id, "{user_id}");
        let backend_keys = backend_json
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("target backend object"))?
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            backend_keys,
            [
                "projection",
                "id",
                "hostId",
                "runtimeKind",
                "executionOs",
                "architecture",
                "counts"
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        );
        assert_eq!(backend_json["counts"]["scope"], "visible_targets");
        for (field, expected) in [
            ("knownManagedRunnerCount", expected_counts[0]),
            ("chargedCiPlacementCount", expected_counts[1]),
            ("chargedFixPlacementCount", expected_counts[2]),
            ("runningCiPlacementCount", expected_counts[3]),
            ("runningFixPlacementCount", expected_counts[4]),
            ("uncertainPlacementCount", expected_counts[5]),
        ] {
            assert_eq!(backend_json["counts"][field], expected, "{user_id} {field}");
        }
        assert!(matches!(
            fixture
                .service
                .get_backend(
                    &principal,
                    "20000000-0000-4000-8000-000000000001".parse()?,
                    backend_id.parse()?,
                )
                .await?
                .item,
            BackendReadV1::Target { .. }
        ));
        let other_backend = if backend_id.ends_with("0001") {
            "22000000-0000-4000-8000-000000000002"
        } else {
            "22000000-0000-4000-8000-000000000001"
        };
        assert!(matches!(
            fixture
                .service
                .get_backend(
                    &principal,
                    "20000000-0000-4000-8000-000000000001".parse()?,
                    other_backend.parse()?,
                )
                .await,
            Err(super::queries::QueryError::NotFound)
        ));
        assert!(matches!(
            fixture
                .service
                .list_domains(
                    &principal,
                    "20000000-0000-4000-8000-000000000001".parse()?,
                    CatalogQuery::default(),
                )
                .await,
            Err(super::queries::QueryError::NotFound)
        ));
        let now = crate::now_millis();
        assert!(matches!(
            fixture
                .service
                .list_samples(
                    &principal,
                    "20000000-0000-4000-8000-000000000001".parse()?,
                    SampleQuery {
                        limit: 1,
                        cursor: None,
                        domain_id: None,
                        received_from: WireTimestamp::from_millis(now - 1_000)?,
                        received_to: WireTimestamp::from_millis(now + 1_000)?,
                    },
                )
                .await,
            Err(super::queries::QueryError::NotFound)
        ));
    }
    for user_id in [
        "count-physical-read",
        "count-physical-operator",
        "count-physical-admin",
    ] {
        let principal = Principal::AuthenticatedUser {
            user_id: user_id.to_owned(),
        };
        let host = fixture
            .service
            .get_host(&principal, "20000000-0000-4000-8000-000000000001".parse()?)
            .await?;
        let host_json = serde_json::to_value(&host.item)?;
        assert_eq!(host_json["projection"], "host", "{user_id}");
        assert_eq!(host_json["counts"]["scope"], "host", "{user_id}");
        assert_eq!(
            host_json["counts"]["chargedCiPlacementCount"], 4,
            "{user_id}"
        );
        assert_eq!(
            host_json["physicalAllocation"]["charged"]["cpuMillis"], 52,
            "{user_id}"
        );
        for (target_id, backend_id, expected_counts) in [
            (
                "44000000-0000-4000-8000-000000000001",
                "22000000-0000-4000-8000-000000000001",
                [1, 3, 1, 1, 1, 1],
            ),
            (
                "44000000-0000-4000-8000-000000000002",
                "22000000-0000-4000-8000-000000000002",
                [1, 1, 0, 2, 0, 0],
            ),
        ] {
            let page = fixture
                .service
                .list_backends(
                    &Fixture::principal(),
                    "20000000-0000-4000-8000-000000000001".parse()?,
                    CatalogQuery {
                        limit: 5,
                        cursor: None,
                        target_id: Some(target_id.parse()?),
                    },
                )
                .await?;
            assert_eq!(page.items.as_slice().len(), 1, "{user_id} {target_id}");
            let backend_json = serde_json::to_value(&page.items.as_slice()[0])?;
            assert_eq!(backend_json["projection"], "host");
            assert_eq!(backend_json["id"], backend_id);
            assert_eq!(backend_json["counts"]["scope"], "visible_targets");
            for (field, expected) in [
                ("knownManagedRunnerCount", expected_counts[0]),
                ("chargedCiPlacementCount", expected_counts[1]),
                ("chargedFixPlacementCount", expected_counts[2]),
                ("runningCiPlacementCount", expected_counts[3]),
                ("runningFixPlacementCount", expected_counts[4]),
                ("uncertainPlacementCount", expected_counts[5]),
            ] {
                assert_eq!(backend_json["counts"][field], expected, "{user_id} {field}");
            }
        }
    }
    for (user_id, expected_events) in [
        ("count-physical-read", 1_usize),
        ("count-physical-operator", 1),
        ("count-physical-admin", 1),
        ("count-gh-a", 1),
        ("count-bb-b", 1),
        ("query-admin", 3),
    ] {
        let events = fixture
            .service
            .list_host_events(
                &Principal::AuthenticatedUser {
                    user_id: user_id.to_owned(),
                },
                "20000000-0000-4000-8000-000000000001".parse()?,
                EventQuery {
                    limit: 10,
                    cursor: None,
                    occurred_from: None,
                    occurred_to: None,
                    target_id: None,
                },
            )
            .await?;
        assert_eq!(events.items.as_slice().len(), expected_events, "{user_id}");
        for event in events.items.as_slice() {
            let value = serde_json::to_value(event)?;
            assert!(value.get("detailJson").is_none());
            assert!(value.get("actorUserId").is_none());
            if user_id == "count-gh-a" {
                assert_eq!(value["targetId"], "44000000-0000-4000-8000-000000000001");
            } else if user_id == "count-bb-b" {
                assert_eq!(value["targetId"], "44000000-0000-4000-8000-000000000002");
            }
        }
    }
    assert!(matches!(
        fixture
            .service
            .get_host(
                &Principal::AuthenticatedUser {
                    user_id: "count-gh-a".to_owned(),
                },
                "70000000-0000-4000-8000-000000000001".parse()?,
            )
            .await,
        Err(super::queries::QueryError::NotFound)
    ));
    assert!(matches!(
        fixture
            .service
            .get_backend(
                &Principal::AuthenticatedUser {
                    user_id: "count-bb-b".to_owned(),
                },
                "70000000-0000-4000-8000-000000000001".parse()?,
                "72000000-0000-4000-8000-000000000001".parse()?,
            )
            .await,
        Err(super::queries::QueryError::NotFound)
    ));
    let unrelated = Principal::AuthenticatedUser {
        user_id: "count-unrelated".to_owned(),
    };
    assert!(matches!(
        fixture
            .service
            .list_backends(
                &Principal::AuthenticatedUser {
                    user_id: "count-physical-read".to_owned(),
                },
                "20000000-0000-4000-8000-000000000001".parse()?,
                CatalogQuery {
                    limit: 5,
                    cursor: None,
                    target_id: Some("44000000-0000-4000-8000-000000000002".parse()?),
                },
            )
            .await,
        Err(super::queries::QueryError::NotFound)
    ));
    assert!(matches!(
        fixture
            .service
            .get_host(&unrelated, "20000000-0000-4000-8000-000000000001".parse()?,)
            .await,
        Err(super::queries::QueryError::NotFound)
    ));
    assert!(matches!(
        fixture
            .service
            .list_backends(
                &Fixture::principal(),
                "20000000-0000-4000-8000-000000000001".parse()?,
                CatalogQuery {
                    limit: 5,
                    cursor: None,
                    target_id: Some("44000000-0000-4000-8000-000000000099".parse()?),
                },
            )
            .await,
        Err(super::queries::QueryError::NotFound)
    ));
    fixture.close().await
}

#[tokio::test]
async fn target_paging_rechecks_provider_scope_and_opaque_row_cursors() -> Result<()> {
    let fixture = Fixture::new().await?;
    sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('page-gh','2100','page-gh','member','',1,1,1),('page-gh-suspend','2101','page-gh-suspend','member','',1,1,1),('page-gh-events','2102','page-gh-events','member','',1,1,1),('page-bb','2103','page-bb','member','',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (70,70,'page-org','Organization','Organization','selected',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at) VALUES (70,70,'page','repo','page/repo',0,'main','https://example.test/page',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('page-gh',70,'read',1),('page-gh-suspend',70,'read',1),('page-gh-events',70,'read',1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO bitbucket_connections (id,name,workspace,workspace_uuid,access_token_key,created_at,updated_at) VALUES ('page-bb','page-bb','page-workspace','{CCCCCCCC-CCCC-CCCC-CCCC-CCCCCCCCCCCC}','page-bb-token',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO bitbucket_user_grants (connection_id,user_id,permission) VALUES ('page-bb','page-bb','read')")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,repository_id) VALUES ('80000000-0000-4000-8000-000000000001','github_repository','page-target-a',70,70)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,bitbucket_connection_id,workspace_uuid) VALUES ('80000000-0000-4000-8000-000000000002','bitbucket_workspace','page-target-b','page-bb','{cccccccc-cccc-cccc-cccc-cccccccccccc}')")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_target_grants (host_id,target_id,allow_schedule,revision) VALUES ('20000000-0000-4000-8000-000000000001','80000000-0000-4000-8000-000000000001',1,1),('30000000-0000-4000-8000-000000000001','80000000-0000-4000-8000-000000000001',1,1),('20000000-0000-4000-8000-000000000001','80000000-0000-4000-8000-000000000002',1,1),('30000000-0000-4000-8000-000000000001','80000000-0000-4000-8000-000000000002',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_backends (id,host_id,domain_id,runtime_kind,execution_os,architecture,readiness,enabled,revision,created_at,updated_at) VALUES ('81000000-0000-4000-8000-000000000001','20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001','docker','linux','x64','ready',1,1,1,1),('81000000-0000-4000-8000-000000000002','20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001','tart_vm','macos','arm64','ready',1,1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,image,created_at,updated_at) VALUES ('page-pool',70,70,'page-pool','repository','page-image',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO pool_execution_profiles (id,pool_id,name,enabled,runtime_kind,execution_os,architecture,mode,image,cpu_millis,memory_mib,disk_bytes,created_at,updated_at,all_authorized_hosts) VALUES ('page-profile-a','page-pool','page-a',1,'docker','linux','x64','ephemeral','page-image',100,100,100,1,1,1),('page-profile-b','page-pool','page-b',1,'tart_vm','macos','arm64','ephemeral','page-image',100,100,100,1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO profile_ci_targets (profile_id,target_id,enabled) VALUES ('page-profile-a','80000000-0000-4000-8000-000000000001',1),('page-profile-b','80000000-0000-4000-8000-000000000002',1)")
        .execute(&fixture.pool)
        .await?;
    for index in 1..=11_i64 {
        let event_id = format!("86000000-0000-4000-8000-{index:012}");
        sqlx::query("INSERT INTO fleet_events (id,host_id,target_id,kind,detail_json,created_at) VALUES (?,'20000000-0000-4000-8000-000000000001','80000000-0000-4000-8000-000000000001','fleet.placement.started','{}',?)")
            .bind(event_id)
            .bind(index)
            .execute(&fixture.pool)
            .await?;
        if index == 1 {
            sqlx::query("INSERT INTO fleet_events (id,host_id,target_id,kind,detail_json,created_at) VALUES ('86000000-0000-4000-8000-000000000099','20000000-0000-4000-8000-000000000001','80000000-0000-4000-8000-000000000002','fleet.placement.started','{}',1)")
                .execute(&fixture.pool)
                .await?;
        }
    }

    let now = 1_000_000_i64;
    let clock = QueryClock::new(now);
    let service = RegistryService::with_clock(
        fixture.pool.clone(),
        Vault::test_with_secret(b"query-test-secret"),
        clock.clone(),
    );
    let host = "20000000-0000-4000-8000-000000000001".parse()?;
    let target_a = "80000000-0000-4000-8000-000000000001".parse()?;
    let target_b = "80000000-0000-4000-8000-000000000002".parse()?;
    let gh = Principal::AuthenticatedUser {
        user_id: "page-gh".to_owned(),
    };
    let first = service
        .list_hosts(
            &gh,
            CatalogQuery {
                limit: 1,
                cursor: None,
                target_id: Some(target_a),
            },
        )
        .await
        .context("target page first")?;
    assert_eq!(first.items.as_slice().len(), 1);
    let cursor = first
        .next_cursor
        .clone()
        .ok_or_else(|| anyhow::anyhow!("page cursor"))?;
    let second = service
        .list_hosts(
            &gh,
            CatalogQuery {
                limit: 1,
                cursor: Some(cursor.clone()),
                target_id: Some(target_a),
            },
        )
        .await
        .context("target page second")?;
    assert_eq!(second.items.as_slice().len(), 1);
    let revoke_page = service
        .list_hosts(
            &gh,
            CatalogQuery {
                limit: 1,
                cursor: None,
                target_id: None,
            },
        )
        .await?;
    let revoke_cursor = revoke_page
        .next_cursor
        .ok_or_else(|| anyhow::anyhow!("revoke cursor"))?;
    let independent = fixture.reopen().await?;
    sqlx::query("DELETE FROM user_installations WHERE user_id='page-gh' AND installation_id=70")
        .execute(&independent)
        .await?;
    independent.close().await;
    assert!(matches!(
        service
            .list_hosts(
                &gh,
                CatalogQuery {
                    limit: 1,
                    cursor: Some(revoke_cursor),
                    target_id: None,
                },
            )
            .await,
        Ok(page) if page.items.as_slice().is_empty()
    ));
    assert!(matches!(
        service
            .get_backend(&gh, host, "81000000-0000-4000-8000-000000000001".parse()?)
            .await,
        Err(super::queries::QueryError::NotFound)
    ));

    // The target actor sees only A events even when B rows are interleaved.
    let sparse = service
        .list_host_events(
            &Principal::AuthenticatedUser {
                user_id: "page-gh-events".to_owned(),
            },
            host,
            EventQuery {
                limit: 1,
                cursor: None,
                occurred_from: None,
                occurred_to: None,
                target_id: None,
            },
        )
        .await?;
    assert_eq!(sparse.items.as_slice().len(), 1);
    assert_eq!(sparse.items.as_slice()[0].target_id, Some(target_a));

    let gh_suspend = Principal::AuthenticatedUser {
        user_id: "page-gh-suspend".to_owned(),
    };
    let suspend_page = service
        .list_hosts(
            &gh_suspend,
            CatalogQuery {
                limit: 1,
                cursor: None,
                target_id: None,
            },
        )
        .await?;
    let suspend_cursor = suspend_page
        .next_cursor
        .ok_or_else(|| anyhow::anyhow!("suspend cursor"))?;
    let independent = fixture.reopen().await?;
    sqlx::query("UPDATE installations SET suspended_at=2 WHERE id=70")
        .execute(&independent)
        .await?;
    independent.close().await;
    assert!(matches!(
        service
            .list_hosts(
                &gh_suspend,
                CatalogQuery {
                    limit: 1,
                    cursor: Some(suspend_cursor),
                    target_id: None,
                },
            )
            .await,
        Ok(page) if page.items.as_slice().is_empty()
    ));
    assert!(matches!(
        service
            .get_backend(
                &gh_suspend,
                host,
                "81000000-0000-4000-8000-000000000001".parse()?
            )
            .await,
        Err(super::queries::QueryError::NotFound)
    ));

    let bb = Principal::AuthenticatedUser {
        user_id: "page-bb".to_owned(),
    };
    let bb_page = service
        .list_hosts(
            &bb,
            CatalogQuery {
                limit: 1,
                cursor: None,
                target_id: None,
            },
        )
        .await?;
    let bb_cursor = bb_page
        .next_cursor
        .ok_or_else(|| anyhow::anyhow!("bitbucket cursor"))?;
    let independent = fixture.reopen().await?;
    sqlx::query("UPDATE bitbucket_user_grants SET revoked_at=2 WHERE connection_id='page-bb' AND user_id='page-bb'")
        .execute(&independent)
        .await?;
    independent.close().await;
    assert!(matches!(
        service
            .list_hosts(
                &bb,
                CatalogQuery {
                    limit: 1,
                    cursor: Some(bb_cursor),
                    target_id: None,
                },
            )
            .await,
        Ok(page) if page.items.as_slice().is_empty()
    ));
    assert!(matches!(
        service
            .get_backend(&bb, host, "81000000-0000-4000-8000-000000000002".parse()?)
            .await,
        Err(super::queries::QueryError::NotFound)
    ));

    let admin = Fixture::principal();
    let row_first = service
        .list_host_events(
            &admin,
            host,
            EventQuery {
                limit: 1,
                cursor: None,
                occurred_from: None,
                occurred_to: None,
                target_id: None,
            },
        )
        .await?;
    let row_first_cursor = row_first
        .next_cursor
        .ok_or_else(|| anyhow::anyhow!("row cursor"))?;
    let row_tenth = service
        .list_host_events(
            &admin,
            host,
            EventQuery {
                limit: 10,
                cursor: None,
                occurred_from: None,
                occurred_to: None,
                target_id: None,
            },
        )
        .await?;
    let row_tenth_cursor = row_tenth
        .next_cursor
        .ok_or_else(|| anyhow::anyhow!("row tenth cursor"))?;
    let first_encoded: String = row_first_cursor.clone().into();
    let tenth_encoded: String = row_tenth_cursor.clone().into();
    assert_eq!(first_encoded.len(), tenth_encoded.len());
    for encoded in [first_encoded, tenth_encoded] {
        assert!(encoded.starts_with("q1."));
        assert!(!encoded.contains("after"));
        let sealed = URL_SAFE_NO_PAD.decode(encoded.trim_start_matches("q1."))?;
        let sealed = String::from_utf8(sealed)?;
        assert!(sealed.contains('.'));
        assert!(!sealed.contains("after"));
        assert!(!sealed.contains("900000"));
    }

    let expiry_page = service
        .list_hosts(
            &admin,
            CatalogQuery {
                limit: 1,
                cursor: None,
                target_id: None,
            },
        )
        .await?;
    let expiry_cursor = expiry_page
        .next_cursor
        .ok_or_else(|| anyhow::anyhow!("expiry cursor"))?;
    clock.set(now + 900_000);
    assert!(matches!(
        service
            .list_hosts(
                &admin,
                CatalogQuery {
                    limit: 1,
                    cursor: Some(expiry_cursor),
                    target_id: None,
                },
            )
            .await,
        Err(super::queries::QueryError::CursorExpired)
    ));

    let scoped = service
        .list_hosts(
            &admin,
            CatalogQuery {
                limit: 1,
                cursor: None,
                target_id: Some(target_a),
            },
        )
        .await?;
    let scoped_cursor = scoped
        .next_cursor
        .ok_or_else(|| anyhow::anyhow!("scoped cursor"))?;
    clock.set(now);
    assert!(matches!(
        service
            .list_hosts(
                &admin,
                CatalogQuery {
                    limit: 1,
                    cursor: Some(scoped_cursor.clone()),
                    target_id: Some(target_b),
                },
            )
            .await,
        Err(super::queries::QueryError::InvalidCursor)
    ));
    assert!(matches!(
        service
            .list_backends(
                &admin,
                host,
                CatalogQuery {
                    limit: 1,
                    cursor: Some(scoped_cursor.clone()),
                    target_id: Some(target_a),
                },
            )
            .await,
        Err(super::queries::QueryError::InvalidCursor)
    ));
    let independent = fixture.reopen().await?;
    sqlx::query("UPDATE fleet_control_plane SET incarnation='10000000-0000-4000-8000-000000000002' WHERE singleton=1")
        .execute(&independent)
        .await?;
    independent.close().await;
    assert!(matches!(
        service
            .list_hosts(
                &admin,
                CatalogQuery {
                    limit: 1,
                    cursor: Some(scoped_cursor),
                    target_id: Some(target_a),
                },
            )
            .await,
        Err(super::queries::QueryError::InvalidCursor)
    ));
    fixture.close().await
}

#[tokio::test]
async fn target_filters_scope_physical_counts_and_backend_selection() -> Result<()> {
    let fixture = Fixture::new().await?;
    sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('physical-reader',2001,'physical-reader','member','',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (50,50,'filter-org','Organization','Organization','selected',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at) VALUES (50,50,'filter','repo','filter/repo',0,'main','https://example.test/filter',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('physical-reader',50,'read',1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,repository_id) VALUES ('60000000-0000-4000-8000-000000000001','github_repository','filter-target',50,50)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_target_grants (host_id,target_id,allow_schedule,revision) VALUES ('20000000-0000-4000-8000-000000000001','60000000-0000-4000-8000-000000000001',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_user_grants (host_id,user_id,permission) VALUES ('20000000-0000-4000-8000-000000000001','physical-reader','read')")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_backends (id,host_id,domain_id,runtime_kind,execution_os,architecture,readiness,enabled,revision,created_at,updated_at) VALUES ('62000000-0000-4000-8000-000000000001','20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001','docker','linux','x64','ready',1,1,1,1),('62000000-0000-4000-8000-000000000002','20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001','tart_vm','macos','arm64','ready',1,1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,image,created_at,updated_at) VALUES ('filter-pool',50,50,'filter-pool','repository','filter-image',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO pool_execution_profiles (id,pool_id,name,enabled,runtime_kind,execution_os,architecture,mode,image,cpu_millis,memory_mib,disk_bytes,created_at,updated_at,all_authorized_hosts) VALUES ('filter-profile','filter-pool','filter-profile',1,'docker','linux','x64','ephemeral','filter-image',100,100,100,1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO profile_ci_targets (profile_id,target_id,enabled) VALUES ('filter-profile','60000000-0000-4000-8000-000000000001',1)")
        .execute(&fixture.pool)
        .await?;
    let target = "60000000-0000-4000-8000-000000000001".parse()?;
    let admin_all = fixture
        .service
        .list_backends(
            &Fixture::principal(),
            "20000000-0000-4000-8000-000000000001".parse()?,
            CatalogQuery {
                limit: 25,
                cursor: None,
                target_id: None,
            },
        )
        .await?;
    assert_eq!(admin_all.items.as_slice().len(), 2);
    let admin_filtered = fixture
        .service
        .list_backends(
            &Fixture::principal(),
            "20000000-0000-4000-8000-000000000001".parse()?,
            CatalogQuery {
                limit: 25,
                cursor: None,
                target_id: Some(target),
            },
        )
        .await?;
    assert_eq!(admin_filtered.items.as_slice().len(), 1);
    let admin_json = serde_json::to_value(&admin_filtered.items.as_slice()[0])?;
    assert_eq!(admin_json["id"], "62000000-0000-4000-8000-000000000001");
    assert_eq!(admin_json["counts"]["scope"], "visible_targets");
    let member = Principal::AuthenticatedUser {
        user_id: "physical-reader".to_owned(),
    };
    let member_filtered = fixture
        .service
        .list_backends(
            &member,
            "20000000-0000-4000-8000-000000000001".parse()?,
            CatalogQuery {
                limit: 25,
                cursor: None,
                target_id: Some(target),
            },
        )
        .await?;
    assert_eq!(member_filtered.items.as_slice().len(), 1);
    assert!(matches!(
        fixture
            .service
            .list_backends(
                &Fixture::principal(),
                "20000000-0000-4000-8000-000000000001".parse()?,
                CatalogQuery {
                    limit: 25,
                    cursor: None,
                    target_id: Some("60000000-0000-4000-8000-000000000099".parse()?),
                },
            )
            .await,
        Err(super::queries::QueryError::NotFound)
    ));
    fixture.close().await
}

#[tokio::test]
async fn tag_only_and_host_plus_tag_selectors_match_list_and_detail() -> Result<()> {
    let fixture = Fixture::new().await?;
    sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('selector-reader',2002,'selector-reader','member','',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (51,51,'selector-org','Organization','Organization','selected',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at) VALUES (51,51,'selector','repo','selector/repo',0,'main','https://example.test/selector',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('selector-reader',51,'read',1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,repository_id) VALUES ('61000000-0000-4000-8000-000000000001','github_repository','selector-target',51,51)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_target_grants (host_id,target_id,allow_schedule,revision) VALUES ('20000000-0000-4000-8000-000000000001','61000000-0000-4000-8000-000000000001',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("UPDATE fleet_hosts SET tags_json='{\"region\":\"za\"}' WHERE id='20000000-0000-4000-8000-000000000001'")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_backends (id,host_id,domain_id,runtime_kind,execution_os,architecture,readiness,enabled,revision,created_at,updated_at) VALUES ('62000000-0000-4000-8000-000000000010','20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001','docker','linux','x64','ready',1,1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,image,created_at,updated_at) VALUES ('selector-pool',51,51,'selector-pool','repository','selector-image',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO pool_execution_profiles (id,pool_id,name,enabled,runtime_kind,execution_os,architecture,mode,image,cpu_millis,memory_mib,disk_bytes,created_at,updated_at,all_authorized_hosts) VALUES ('selector-profile','selector-pool','selector-profile',1,'docker','linux','x64','ephemeral','selector-image',100,100,100,1,1,0)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO profile_ci_targets (profile_id,target_id,enabled) VALUES ('selector-profile','61000000-0000-4000-8000-000000000001',1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO profile_host_selectors (profile_id,kind,tag_key,tag_value) VALUES ('selector-profile','tag','region','za')")
        .execute(&fixture.pool)
        .await?;
    let principal = Principal::AuthenticatedUser {
        user_id: "selector-reader".to_owned(),
    };
    let host = "20000000-0000-4000-8000-000000000001".parse()?;
    let backend = "62000000-0000-4000-8000-000000000010".parse()?;
    let list = fixture
        .service
        .list_backends(
            &principal,
            host,
            CatalogQuery {
                limit: 25,
                cursor: None,
                target_id: None,
            },
        )
        .await?;
    assert_eq!(list.items.as_slice().len(), 1);
    assert!(matches!(
        list.items.as_slice()[0],
        BackendReadV1::Target { .. }
    ));
    assert!(matches!(
        fixture
            .service
            .get_backend(&principal, host, backend)
            .await?
            .item,
        BackendReadV1::Target { .. }
    ));
    sqlx::query("INSERT INTO profile_host_selectors (profile_id,kind,host_id) VALUES ('selector-profile','host','20000000-0000-4000-8000-000000000001')")
        .execute(&fixture.pool)
        .await?;
    assert_eq!(
        fixture
            .service
            .list_backends(&principal, host, CatalogQuery::default())
            .await?
            .items
            .as_slice()
            .len(),
        1
    );
    sqlx::query("UPDATE profile_host_selectors SET tag_value='us' WHERE profile_id='selector-profile' AND kind='tag'")
        .execute(&fixture.pool)
        .await?;
    assert!(matches!(
        fixture.service.get_backend(&principal, host, backend).await,
        Err(super::queries::QueryError::NotFound)
    ));
    fixture.close().await
}

#[tokio::test]
async fn approval_receipt_boundaries_and_current_host_identity_are_rechecked() -> Result<()> {
    let fixture = Fixture::new().await?;
    let now = 1_000_000_i64;
    let clock = QueryClock::new(now);
    let service = RegistryService::with_clock(
        fixture.pool.clone(),
        Vault::test_with_secret(b"query-test-secret"),
        clock.clone(),
    );
    let digest = seed_current_observation(&fixture, now).await?;
    let host = "20000000-0000-4000-8000-000000000001".parse()?;
    let current = || async {
        Ok::<serde_json::Value, anyhow::Error>(serde_json::to_value(
            &service.get_host(&Fixture::principal(), host).await?.item,
        )?)
    };
    assert_eq!(current().await?["inventoryEvidence"]["state"], "current");
    assert_eq!(current().await?["approvalEvidence"]["state"], "available");
    for (label, clock_now, expected) in [
        ("fresh", now + 44_999, "current"),
        ("boundary", now + 45_000, "stale"),
        ("future", now - 1, "stale"),
    ] {
        clock.set(clock_now);
        let value = current().await?;
        assert_eq!(value["inventoryEvidence"]["state"], expected, "{label}");
        if expected == "current" {
            assert_eq!(value["approvalEvidence"]["state"], "available");
        }
    }
    clock.set(now);
    sqlx::query("UPDATE fleet_hosts SET agent_session_id='wrong-session' WHERE id=?")
        .bind(host.to_string())
        .execute(&fixture.pool)
        .await?;
    assert_eq!(current().await?["inventoryEvidence"]["state"], "stale");
    sqlx::query("UPDATE fleet_hosts SET agent_session_id='query-observation-session',boot_id='wrong-boot' WHERE id=?")
        .bind(host.to_string())
        .execute(&fixture.pool)
        .await?;
    assert_eq!(current().await?["inventoryEvidence"]["state"], "stale");
    sqlx::query("UPDATE fleet_hosts SET boot_id='query-observation-boot' WHERE id=?")
        .bind(host.to_string())
        .execute(&fixture.pool)
        .await?;
    sqlx::query("UPDATE host_credentials SET revoked_at=? WHERE id='query-observation-credential'")
        .bind(now)
        .execute(&fixture.pool)
        .await?;
    assert_eq!(current().await?["inventoryEvidence"]["state"], "stale");
    sqlx::query(
        "UPDATE host_credentials SET revoked_at=NULL WHERE id='query-observation-credential'",
    )
    .execute(&fixture.pool)
    .await?;
    sqlx::query("UPDATE fleet_hosts SET inventory_digest=? WHERE id=?")
        .bind(String::from(Sha256Digest::of(b"other")))
        .bind(host.to_string())
        .execute(&fixture.pool)
        .await?;
    assert_eq!(current().await?["inventoryEvidence"]["state"], "stale");
    // Keep the digest variable used to document the positive counter/session
    // divergence fixture and to avoid accidentally testing only timestamps.
    assert_eq!(digest.len(), 43);
    fixture.close().await
}

#[tokio::test]
async fn observed_capacity_and_backend_probe_provenance_have_explicit_states() -> Result<()> {
    let fixture = Fixture::new().await?;
    let now = 1_000_000_i64;
    let clock = QueryClock::new(now);
    let service = RegistryService::with_clock(
        fixture.pool.clone(),
        Vault::test_with_secret(b"query-test-secret"),
        clock.clone(),
    );
    let _digest = seed_current_observation(&fixture, now).await?;
    sqlx::query("UPDATE fleet_observation_sessions SET lease_expires_at=? WHERE session_id='query-observation-session'")
        .bind(now + 200_000)
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_backends (id,host_id,domain_id,runtime_kind,execution_os,architecture,readiness,enabled,revision,created_at,updated_at,probe_epoch,probe_session_id,probe_boot_id,probe_incarnation,last_probed_at) VALUES ('63000000-0000-4000-8000-000000000001','20000000-0000-4000-8000-000000000001','21000000-0000-4000-8000-000000000001','docker','linux','x64','ready',1,1,1,1,1,'query-observation-session','query-observation-boot','10000000-0000-4000-8000-000000000001',?)")
        .bind(now)
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_observed_domains (host_id,local_key,domain_id,parent_local_key,kind,observed_name,runtime_identity,cpu_millis,memory_mib,disk_bytes,session_id,host_epoch,control_plane_incarnation,inventory_sequence,received_at) VALUES ('20000000-0000-4000-8000-000000000001','root','21000000-0000-4000-8000-000000000001',NULL,'physical','root','root-runtime',1000,2000,3000,'query-observation-session',1,'10000000-0000-4000-8000-000000000001',7,?)")
        .bind(now)
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_observed_backends (host_id,local_key,backend_id,domain_local_key,runtime_kind,execution_os,architecture,capabilities_json,observed_readiness,reason,runtime_identity,session_id,host_epoch,control_plane_incarnation,inventory_sequence,received_at) VALUES ('20000000-0000-4000-8000-000000000001','docker','63000000-0000-4000-8000-000000000001','root','docker','linux','x64','{}','ready',NULL,'docker-runtime','query-observation-session',1,'10000000-0000-4000-8000-000000000001',7,?)")
        .bind(now)
        .execute(&fixture.pool)
        .await?;
    let host = "20000000-0000-4000-8000-000000000001".parse()?;
    let domain = service
        .get_domain(
            &Fixture::principal(),
            host,
            "21000000-0000-4000-8000-000000000001".parse()?,
        )
        .await?;
    assert!(matches!(
        domain.item.observed_capacity,
        ObservedCapacityV1::Current { .. }
    ));
    sqlx::query("UPDATE fleet_hosts SET inventory_revision=10 WHERE id=?")
        .bind(host.to_string())
        .execute(&fixture.pool)
        .await?;
    let mismatched_domain = service
        .get_domain(
            &Fixture::principal(),
            host,
            "21000000-0000-4000-8000-000000000001".parse()?,
        )
        .await?;
    assert!(matches!(
        mismatched_domain.item.observed_capacity,
        ObservedCapacityV1::Historical { .. }
    ));
    let mismatched_backend = serde_json::to_value(
        &service
            .get_backend(
                &Fixture::principal(),
                host,
                "63000000-0000-4000-8000-000000000001".parse()?,
            )
            .await?
            .item,
    )?;
    assert_eq!(mismatched_backend["probeFreshness"], "unknown");
    sqlx::query("UPDATE fleet_hosts SET inventory_revision=9 WHERE id=?")
        .bind(host.to_string())
        .execute(&fixture.pool)
        .await?;
    sqlx::query("UPDATE fleet_observation_sessions SET current_inventory_sequence=8 WHERE session_id='query-observation-session'")
        .execute(&fixture.pool)
        .await?;
    let historical = service
        .get_domain(
            &Fixture::principal(),
            host,
            "21000000-0000-4000-8000-000000000001".parse()?,
        )
        .await?;
    assert!(matches!(
        historical.item.observed_capacity,
        ObservedCapacityV1::Historical { .. }
    ));
    let backend: crate::fleet::ids::BackendId = "63000000-0000-4000-8000-000000000001".parse()?;
    for (offset, expected) in [
        (44_999_i64, "fresh"),
        (45_000, "stale"),
        (90_000, "offline"),
        (-1, "unknown"),
    ] {
        sqlx::query("UPDATE fleet_observation_sessions SET current_inventory_sequence=7 WHERE session_id='query-observation-session'")
            .execute(&fixture.pool)
            .await?;
        clock.set(now + offset);
        let value = serde_json::to_value(
            &service
                .get_backend(&Fixture::principal(), host, backend)
                .await?
                .item,
        )?;
        assert_eq!(value["probeFreshness"], expected, "offset={offset}");
    }
    clock.set(now);
    sqlx::query("UPDATE host_backends SET last_probed_at=? WHERE id=?")
        .bind(now - 1)
        .bind(backend.to_string())
        .execute(&fixture.pool)
        .await?;
    let detached_timestamp = serde_json::to_value(
        &service
            .get_backend(&Fixture::principal(), host, backend)
            .await?
            .item,
    )?;
    assert_eq!(detached_timestamp["probeFreshness"], "unknown");
    sqlx::query("UPDATE host_backends SET last_probed_at=? WHERE id=?")
        .bind(now)
        .bind(backend.to_string())
        .execute(&fixture.pool)
        .await?;
    sqlx::query("UPDATE host_backends SET probe_session_id='wrong-session' WHERE id=?")
        .bind(backend.to_string())
        .execute(&fixture.pool)
        .await?;
    let unknown = serde_json::to_value(
        &service
            .get_backend(&Fixture::principal(), host, backend)
            .await?
            .item,
    )?;
    assert_eq!(unknown["probeFreshness"], "unknown");
    fixture.close().await
}

#[tokio::test]
async fn historical_target_events_survive_schedule_grant_revocation() -> Result<()> {
    let fixture = Fixture::new().await?;
    sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('event-reader',2003,'event-reader','member','',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (52,52,'event-org','Organization','Organization','selected',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at) VALUES (52,52,'event','repo','event/repo',0,'main','https://example.test/event',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('event-reader',52,'read',1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,repository_id) VALUES ('64000000-0000-4000-8000-000000000001','github_repository','event-target',52,52)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_target_grants (host_id,target_id,allow_schedule,revision) VALUES ('20000000-0000-4000-8000-000000000001','64000000-0000-4000-8000-000000000001',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO host_user_grants (host_id,user_id,permission) VALUES ('20000000-0000-4000-8000-000000000001','event-reader','read')")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO fleet_events (id,host_id,target_id,actor_user_id,kind,detail_json,created_at) VALUES ('65000000-0000-4000-8000-000000000001','20000000-0000-4000-8000-000000000001','64000000-0000-4000-8000-000000000001','event-reader','fleet.inventory.received','{}',1)")
        .execute(&fixture.pool)
        .await?;
    let principal = Principal::AuthenticatedUser {
        user_id: "event-reader".to_owned(),
    };
    let host: crate::fleet::ids::HostId = "20000000-0000-4000-8000-000000000001".parse()?;
    sqlx::query("UPDATE host_target_grants SET revoked_at=2 WHERE host_id=? AND target_id=?")
        .bind(host.to_string())
        .bind("64000000-0000-4000-8000-000000000001")
        .execute(&fixture.pool)
        .await?;
    let events = fixture
        .service
        .list_host_events(
            &principal,
            host,
            EventQuery {
                limit: 25,
                cursor: None,
                target_id: None,
                occurred_from: None,
                occurred_to: None,
            },
        )
        .await?;
    assert_eq!(events.items.as_slice().len(), 1);
    assert_eq!(
        serde_json::to_value(&events.items.as_slice()[0])?["targetId"],
        "64000000-0000-4000-8000-000000000001"
    );
    fixture.close().await
}

#[tokio::test]
async fn readable_target_cte_handles_catalogs_larger_than_a_page() -> Result<()> {
    let fixture = Fixture::new().await?;
    sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('catalog-reader',2004,'catalog-reader','member','',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (53,53,'catalog-org','Organization','Organization','selected',1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at) VALUES (53,53,'catalog','repo','catalog/repo',0,'main','https://example.test/catalog',1,1,1)")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('catalog-reader',53,'read',1)")
        .execute(&fixture.pool)
        .await?;
    for index in 1..=1_100_u32 {
        let target_id = format!("66000000-0000-4000-8000-{index:012}");
        sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,repository_id) VALUES (?,'github_repository',?,53,53)")
            .bind(&target_id)
            .bind(format!("catalog-{index}"))
            .execute(&fixture.pool)
            .await?;
        sqlx::query("INSERT INTO host_target_grants (host_id,target_id,allow_schedule,revision) VALUES ('20000000-0000-4000-8000-000000000001',?,0,1)")
            .bind(&target_id)
            .execute(&fixture.pool)
            .await?;
    }
    let principal = Principal::AuthenticatedUser {
        user_id: "catalog-reader".to_owned(),
    };
    let hosts = fixture
        .service
        .list_hosts(
            &principal,
            CatalogQuery {
                limit: 1,
                cursor: None,
                target_id: None,
            },
        )
        .await?;
    assert_eq!(hosts.items.as_slice().len(), 1);
    assert!(matches!(
        hosts.items.as_slice()[0],
        HostReadV1::Target { .. }
    ));
    fixture.close().await
}
