//! Approval evidence matrix and public approval parity tests.
//!
//! This module owns only read-to-write approval coverage. Each case creates a
//! fresh database fixture so inventory, observation, authorization, and
//! immutable receipt predicates cannot be masked by an earlier mutation.

use std::sync::Arc;

use super::*;
use crate::{
    Vault,
    fleet::{
        Architecture, ExecutionOs,
        authorization::Principal,
        ids::{AuthoritySessionId, OperationId, Revision, UpstreamText},
        protocol::{
            inventory::{
                DomainKind, HandshakeRequest, InventoryReport, InventoryRequest, ObservedCapacity,
                ObservedDomain, ObservedSample, ProtocolRange, SampleCoverage,
            },
            primitives::{BrowserCounter, WireTimestamp},
            query::HostReadV1,
        },
        registry::{
            ApproveHost, EnrollmentIssue, EnrollmentScope, HostRegistration, IssueEnrollment,
        },
    },
};
use anyhow::Result;
use serde_json::Value;

const APPROVAL_HOST: &str = "20000000-0000-4000-8000-000000000001";
const APPROVAL_TARGET: &str = "70000000-0000-4000-8000-000000000001";

async fn approval_fixture() -> Result<(Fixture, Arc<QueryClock>, i64)> {
    let mut fixture = Fixture::new().await?;
    let now = 100_000_i64;
    let clock = QueryClock::new(now);
    fixture.service = RegistryService::with_clock(
        fixture.pool.clone(),
        Vault::test_with_secret(b"query-test-secret"),
        clock.clone(),
    );
    seed_current_observation(&fixture, now).await?;
    Ok((fixture, clock, now))
}

fn operation_id() -> Result<OperationId> {
    Ok(OperationId::try_new(uuid::Uuid::new_v4())?)
}

async fn local_inventory_fixture() -> Result<(
    Fixture,
    crate::fleet::registry::AuthenticatedHost,
    i64,
    crate::fleet::protocol::inventory::InventoryReport,
)> {
    let mut fixture = Fixture::new().await?;
    let now = 100_000_i64;
    let clock = QueryClock::new(now);
    fixture.service = RegistryService::with_clock(
        fixture.pool.clone(),
        Vault::test_with_secret(b"query-test-secret"),
        clock,
    );
    sqlx::query("INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at) VALUES (70,70,'approval-org','Organization','Organization','selected',1,1)")
        .execute(&fixture.pool).await?;
    sqlx::query("INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at) VALUES (70,70,'approval','repo','approval/repo',0,'main','https://example.test/approval',1,1,1)")
        .execute(&fixture.pool).await?;
    sqlx::query("INSERT INTO user_installations (user_id,installation_id,permission,created_at) VALUES ('query-admin',70,'admin',1)")
        .execute(&fixture.pool).await?;
    sqlx::query("INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,repository_id) VALUES (?, 'github_repository','approval-target',70,70)")
        .bind(APPROVAL_TARGET).execute(&fixture.pool).await?;
    let issue = match fixture
        .service
        .issue_enrollment(
            &Fixture::principal(),
            IssueEnrollment {
                idempotency_key: "query-approval-parity".to_owned(),
                scope: EnrollmentScope::new([APPROVAL_TARGET.parse()?])?,
                intended_host_id: None,
                ttl_millis: None,
            },
        )
        .await?
    {
        EnrollmentIssue::Created(issue) => issue,
        EnrollmentIssue::Replay(_) => anyhow::bail!("unexpected enrollment replay"),
    };
    let receipt = fixture
        .service
        .consume_enrollment(
            issue.code.expose(),
            HostRegistration::new(
                "local approval host",
                ExecutionOs::Linux,
                Architecture::X64,
                None,
            )?,
        )
        .await?;
    let authenticated = fixture
        .service
        .authenticate(receipt.host_id, receipt.credential.expose())
        .await?;
    let incarnation: crate::fleet::ids::ControlPlaneIncarnation = sqlx::query_scalar::<_, String>(
        "SELECT incarnation FROM fleet_control_plane WHERE singleton=1",
    )
    .fetch_one(&fixture.pool)
    .await?
    .parse()?;
    let session = AuthoritySessionId::try_new(uuid::Uuid::new_v4())?;
    let boot = UpstreamText::new("query-parity-boot")?;
    fixture
        .service
        .handshake(
            &authenticated,
            HandshakeRequest::new(
                operation_id()?,
                session,
                boot.clone(),
                ProtocolRange::new(1, 1)?,
                UpstreamText::new("query-test-agent")?,
                Some(authenticated.epoch()),
            ),
        )
        .await?;
    let report = InventoryReport {
        host_epoch: authenticated.epoch(),
        control_plane_incarnation: incarnation,
        authority_session_id: session,
        boot_id: boot,
        inventory_sequence: BrowserCounter::new(1)?,
        observed_at: WireTimestamp::from_millis(now)?,
        domains: vec![ObservedDomain {
            local_key: UpstreamText::new("root")?,
            known_id: None,
            parent_local_key: None,
            kind: DomainKind::Physical,
            name: UpstreamText::new("physical")?,
            runtime_identity: None,
            capacity: ObservedCapacity {
                cpu_millis: None,
                memory_mib: None,
                disk_bytes: None,
            },
        }],
        backends: Vec::new(),
        interactive_sessions: Vec::new(),
        samples: vec![ObservedSample {
            domain_local_key: UpstreamText::new("root")?,
            observed_at: WireTimestamp::from_millis(now)?,
            coverage: SampleCoverage::Partial,
            cpu_used_millis: None,
            memory_used_mib: None,
            disk_free_bytes: None,
            uptime_seconds: None,
        }],
    };
    fixture
        .service
        .submit_inventory(
            &authenticated,
            InventoryRequest::new(operation_id()?, report.clone())?,
        )
        .await?;
    Ok((fixture, authenticated, now, report))
}

async fn approval_json(fixture: &Fixture) -> Result<Value> {
    let result = fixture
        .service
        .get_host(&Fixture::principal(), APPROVAL_HOST.parse()?)
        .await?;
    Ok(serde_json::to_value(result.item)?)
}

fn assert_stale_approval(value: &Value) {
    assert_eq!(value["inventoryEvidence"]["state"], "stale");
    assert_eq!(value["approvalEvidence"]["state"], "unavailable");
    assert_eq!(value["approvalEvidence"]["reason"], "stale_inventory");
    assert_unavailable_fields(value);
}

fn assert_unavailable_fields(value: &Value) {
    let approval = value["approvalEvidence"].as_object();
    assert!(approval.is_some(), "approval evidence must be an object");
    for field in [
        "expectedRevision",
        "expectedInventoryRevision",
        "expectedInventoryDigest",
        "inventoryReceivedAt",
        "observationLeaseExpiresAt",
    ] {
        assert!(
            !approval.is_some_and(|approval| approval.contains_key(field)),
            "unavailable field leaked: {field}"
        );
    }
}

async fn run_stale_case(case: &str) -> Result<()> {
    let (fixture, _clock, now) = approval_fixture().await?;
    match case {
        "missing" => {
            sqlx::query("UPDATE fleet_hosts SET inventory_complete=0,inventory_revision=0,inventory_digest=NULL,last_inventory_at=NULL WHERE id=?")
                .bind(APPROVAL_HOST).execute(&fixture.pool).await?;
        }
        "observation-ended" => {
            sqlx::query("UPDATE fleet_observation_sessions SET ended_at=? WHERE session_id='query-observation-session'")
                .bind(now).execute(&fixture.pool).await?;
        }
        "lease-expired" => {
            sqlx::query("UPDATE fleet_observation_sessions SET lease_expires_at=? WHERE session_id='query-observation-session'")
                .bind(now).execute(&fixture.pool).await?;
        }
        "credential-revoked" => {
            sqlx::query(
                "UPDATE host_credentials SET revoked_at=? WHERE id='query-observation-credential'",
            )
            .bind(now)
            .execute(&fixture.pool)
            .await?;
        }
        "credential-overlap" => {
            sqlx::query("UPDATE host_credentials SET overlap_expires_at=? WHERE id='query-observation-credential'")
                .bind(now).execute(&fixture.pool).await?;
        }
        "session-mismatch" => {
            sqlx::query("UPDATE fleet_hosts SET agent_session_id='wrong-session' WHERE id=?")
                .bind(APPROVAL_HOST)
                .execute(&fixture.pool)
                .await?;
        }
        "boot-mismatch" => {
            sqlx::query("UPDATE fleet_hosts SET agent_session_id='wrong-boot-session' WHERE id=?")
                .bind(APPROVAL_HOST)
                .execute(&fixture.pool)
                .await?;
        }
        "epoch-mismatch" => {
            sqlx::query("UPDATE fleet_hosts SET inventory_revision=8 WHERE id=?")
                .bind(APPROVAL_HOST)
                .execute(&fixture.pool)
                .await?;
        }
        "incarnation-mismatch" => {
            sqlx::query("UPDATE fleet_hosts SET inventory_revision=8 WHERE id=?")
                .bind(APPROVAL_HOST)
                .execute(&fixture.pool)
                .await?;
        }
        "snapshot-mismatch" => {
            sqlx::query("UPDATE fleet_hosts SET inventory_revision=8 WHERE id=?")
                .bind(APPROVAL_HOST)
                .execute(&fixture.pool)
                .await?;
        }
        "incomplete" => {
            sqlx::query("UPDATE fleet_hosts SET inventory_complete=0 WHERE id=?")
                .bind(APPROVAL_HOST)
                .execute(&fixture.pool)
                .await?;
        }
        "receipt-future" => {
            sqlx::query("UPDATE fleet_hosts SET last_inventory_at=? WHERE id=?")
                .bind(now + 1)
                .bind(APPROVAL_HOST)
                .execute(&fixture.pool)
                .await?;
        }
        "receipt-at-age-limit" => {
            sqlx::query("UPDATE fleet_hosts SET last_inventory_at=? WHERE id=?")
                .bind(now - 45_000)
                .bind(APPROVAL_HOST)
                .execute(&fixture.pool)
                .await?;
        }
        _ => anyhow::bail!("unknown approval case: {case}"),
    }
    let value = approval_json(&fixture).await?;
    if case == "missing" {
        assert_eq!(value["inventoryEvidence"]["state"], "missing");
        assert_eq!(value["approvalEvidence"]["state"], "unavailable");
        assert_eq!(value["approvalEvidence"]["reason"], "stale_inventory");
        assert_unavailable_fields(&value);
    } else {
        assert_stale_approval(&value);
    }
    fixture.close().await
}

#[tokio::test]
async fn approval_negative_matrix_is_fresh_and_omits_unavailable_fields() -> Result<()> {
    for case in [
        "missing",
        "observation-ended",
        "lease-expired",
        "credential-revoked",
        "credential-overlap",
        "session-mismatch",
        "boot-mismatch",
        "epoch-mismatch",
        "incarnation-mismatch",
        "snapshot-mismatch",
        "incomplete",
        "receipt-future",
        "receipt-at-age-limit",
    ] {
        run_stale_case(case).await?;
    }
    Ok(())
}

#[tokio::test]
async fn approval_receipt_boundaries_and_terminal_states_are_explicit() -> Result<()> {
    let (fixture, clock, now) = approval_fixture().await?;
    let current = approval_json(&fixture).await?;
    assert_eq!(current["inventoryEvidence"]["state"], "current");
    assert_eq!(current["approvalEvidence"]["state"], "available");
    assert_eq!(current["approvalEvidence"]["expectedInventoryRevision"], 9);
    assert_eq!(
        current["approvalEvidence"]["expectedInventoryDigest"],
        "A".repeat(43)
    );

    sqlx::query(
        "UPDATE host_credentials SET overlap_expires_at=? WHERE id='query-observation-credential'",
    )
    .bind(now + 1)
    .execute(&fixture.pool)
    .await?;
    let overlap = approval_json(&fixture).await?;
    assert_eq!(overlap["approvalEvidence"]["state"], "available");

    clock.set(now + 45_000);
    let age_boundary = approval_json(&fixture).await?;
    assert_stale_approval(&age_boundary);

    let (fixture, _clock, _now) = approval_fixture().await?;
    sqlx::query("UPDATE fleet_hosts SET enrollment_state='approved' WHERE id=?")
        .bind(APPROVAL_HOST)
        .execute(&fixture.pool)
        .await?;
    let approved = approval_json(&fixture).await?;
    assert_eq!(approved["inventoryEvidence"]["state"], "current");
    assert_eq!(approved["approvalEvidence"]["reason"], "pending_host");
    assert_unavailable_fields(&approved);
    fixture.close().await?;

    let (fixture, _clock, _now) = approval_fixture().await?;
    sqlx::query("UPDATE fleet_hosts SET lifecycle_state='retired' WHERE id=?")
        .bind(APPROVAL_HOST)
        .execute(&fixture.pool)
        .await?;
    let retired = approval_json(&fixture).await?;
    assert_eq!(retired["approvalEvidence"]["reason"], "retired_host");
    assert_unavailable_fields(&retired);
    fixture.close().await?;

    let (fixture, _clock, _now) = approval_fixture().await?;
    sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('approval-reader',777,'approval-reader','member','',1,1,1)")
        .execute(&fixture.pool).await?;
    sqlx::query("INSERT INTO host_user_grants(host_id,user_id,permission) VALUES (?,?,'read')")
        .bind(APPROVAL_HOST)
        .bind("approval-reader")
        .execute(&fixture.pool)
        .await?;
    let response = fixture
        .service
        .get_host(
            &Principal::AuthenticatedUser {
                user_id: "approval-reader".to_owned(),
            },
            APPROVAL_HOST.parse()?,
        )
        .await?;
    let is_host = matches!(response.item, HostReadV1::Host { .. });
    let value = serde_json::to_value(response.item)?;
    assert!(is_host);
    assert_eq!(value["inventoryEvidence"]["state"], "current");
    assert_eq!(value["approvalEvidence"]["reason"], "requires_system_admin");
    assert_unavailable_fields(&value);
    fixture.close().await
}

#[tokio::test]
async fn public_read_receipt_approves_unchanged_inventory_and_rejects_new_revision() -> Result<()> {
    let (fixture, authenticated, _now, _report) = local_inventory_fixture().await?;
    let principal = Fixture::principal();
    let host = fixture
        .service
        .get_host(&principal, authenticated.host_id())
        .await?;
    let value = serde_json::to_value(host.item)?;
    assert_eq!(value["inventoryEvidence"]["state"], "current");
    assert_eq!(value["approvalEvidence"]["state"], "available");
    let request = ApproveHost {
        host_id: authenticated.host_id(),
        expected_revision: Revision::new(
            value["approvalEvidence"]["expectedRevision"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("missing approval revision"))?,
        )?,
        expected_inventory_revision: value["approvalEvidence"]["expectedInventoryRevision"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing inventory revision"))?,
        expected_inventory_digest: value["approvalEvidence"]["expectedInventoryDigest"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing inventory digest"))?
            .to_owned(),
        scope: EnrollmentScope::new([APPROVAL_TARGET.parse()?])?,
    };
    fixture
        .service
        .approve_host(&principal, request.clone())
        .await?;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT enrollment_state FROM fleet_hosts WHERE id=?")
            .bind(authenticated.host_id().to_string())
            .fetch_one(&fixture.pool)
            .await?,
        "approved"
    );
    fixture.close().await?;

    let (fixture, authenticated, now, mut newer) = local_inventory_fixture().await?;
    let host = fixture
        .service
        .get_host(&principal, authenticated.host_id())
        .await?;
    let value = serde_json::to_value(host.item)?;
    let request = ApproveHost {
        host_id: authenticated.host_id(),
        expected_revision: Revision::new(
            value["approvalEvidence"]["expectedRevision"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("missing approval revision"))?,
        )?,
        expected_inventory_revision: value["approvalEvidence"]["expectedInventoryRevision"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing inventory revision"))?,
        expected_inventory_digest: value["approvalEvidence"]["expectedInventoryDigest"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing inventory digest"))?
            .to_owned(),
        scope: EnrollmentScope::new([APPROVAL_TARGET.parse()?])?,
    };
    newer.inventory_sequence = crate::fleet::protocol::primitives::BrowserCounter::new(2)?;
    newer.observed_at = WireTimestamp::from_millis(now)?;
    fixture
        .service
        .submit_inventory(
            &authenticated,
            InventoryRequest::new(operation_id()?, newer)?,
        )
        .await?;
    let before: (String, i64) =
        sqlx::query_as("SELECT enrollment_state,revision FROM fleet_hosts WHERE id=?")
            .bind(authenticated.host_id().to_string())
            .fetch_one(&fixture.pool)
            .await?;
    assert!(matches!(
        fixture.service.approve_host(&principal, request).await,
        Err(crate::fleet::registry::RegistryError::StaleRevision)
    ));
    let after: (String, i64) =
        sqlx::query_as("SELECT enrollment_state,revision FROM fleet_hosts WHERE id=?")
            .bind(authenticated.host_id().to_string())
            .fetch_one(&fixture.pool)
            .await?;
    assert_eq!(after, before);
    fixture.close().await?;

    let (fixture, authenticated, _now, _report) = local_inventory_fixture().await?;
    let host = fixture
        .service
        .get_host(&principal, authenticated.host_id())
        .await?;
    let value = serde_json::to_value(host.item)?;
    let request = ApproveHost {
        host_id: authenticated.host_id(),
        expected_revision: Revision::new(
            value["approvalEvidence"]["expectedRevision"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("missing approval revision"))?,
        )?,
        expected_inventory_revision: value["approvalEvidence"]["expectedInventoryRevision"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing inventory revision"))?,
        expected_inventory_digest: value["approvalEvidence"]["expectedInventoryDigest"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing inventory digest"))?
            .to_owned(),
        scope: EnrollmentScope::new([APPROVAL_TARGET.parse()?])?,
    };
    sqlx::query("UPDATE user_installations SET permission='read' WHERE user_id='query-admin' AND installation_id=70")
        .execute(&fixture.pool)
        .await?;
    let before: (String, i64, i64) = sqlx::query_as(
        "SELECT enrollment_state,revision,(SELECT COUNT(*) FROM host_target_grants WHERE host_id=?) FROM fleet_hosts WHERE id=?",
    )
    .bind(authenticated.host_id().to_string())
    .bind(authenticated.host_id().to_string())
    .fetch_one(&fixture.pool)
    .await?;
    let approval = fixture.service.approve_host(&principal, request).await;
    assert!(matches!(
        approval,
        Err(crate::fleet::registry::RegistryError::TargetScopeForbidden)
    ));
    let after: (String, i64, i64) = sqlx::query_as(
        "SELECT enrollment_state,revision,(SELECT COUNT(*) FROM host_target_grants WHERE host_id=?) FROM fleet_hosts WHERE id=?",
    )
    .bind(authenticated.host_id().to_string())
    .bind(authenticated.host_id().to_string())
    .fetch_one(&fixture.pool)
    .await?;
    assert_eq!(after, before);
    fixture.close().await
}
