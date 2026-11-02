//! Focused shape tests for the inventory boundary.
//!
//! Database reconciliation tests live in `tests/fleet_inventory.rs`; these
//! unit tests ensure invalid graph and capability identity data is rejected
//! before a writer transaction is opened.

#[cfg(test)]
mod tests {
    use anyhow::{Result, anyhow};
    use sqlx::SqlitePool;
    use std::fmt::Display;
    use std::sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    };

    use crate::fleet::{
        Architecture, BackendReadiness, ExecutionOs, RuntimeKind,
        capabilities::{
            BackendCapabilities, CapabilityReport, CpuMemoryEnforcement, DockerSocketCapability,
            EnvironmentCapability, EnvironmentReadiness, InteractiveCapability,
            ResourceEnforcement,
        },
        ids::{AuthoritySessionId, ControlPlaneIncarnation, HostEpoch, UpstreamText},
        protocol::{
            inventory::{
                DomainKind, InventoryReport, ObservedBackend, ObservedCapacity, ObservedDomain,
                ObservedSample, SampleCoverage,
            },
            primitives::{BrowserCounter, ProtocolVersion, WireTimestamp},
        },
    };
    use crate::fleet::{
        authorization::Principal,
        ids::OperationId,
        protocol::inventory::{
            HandshakeRequest, HeartbeatRequest, InventoryRequest, ProtocolRange,
        },
        registry::{EnrollmentScope, HostRegistration, IssueEnrollment, RegistryClock},
    };
    use crate::{Vault, connect_database_path};

    fn must<T, E: Display>(result: Result<T, E>, context: &str) -> Result<T> {
        result.map_err(|error| anyhow!("{context}: {error}"))
    }

    fn text(value: &str) -> Result<UpstreamText> {
        must(UpstreamText::new(value), "valid test text")
    }

    fn report() -> Result<InventoryReport> {
        let report = CapabilityReport {
            schema_version: must(ProtocolVersion::new(1), "protocol")?,
            runtime_kind: RuntimeKind::NativeProcess,
            execution_os: ExecutionOs::Linux,
            architecture: Architecture::X64,
            supported_runners: Vec::new(),
            environments: vec![EnvironmentCapability {
                reference: must(
                    crate::fleet::protocol::primitives::ImageReference::try_from(
                        "runner:latest".to_owned(),
                    ),
                    "image",
                )?,
                readiness: EnvironmentReadiness::Unknown,
                version: None,
            }],
            any_image: false,
            resources: ResourceEnforcement {
                cpu: CpuMemoryEnforcement::Reservation,
                memory: CpuMemoryEnforcement::Reservation,
                disk: crate::fleet::capabilities::DiskEnforcement::Estimate,
            },
            interactive: InteractiveCapability::Unsupported,
            docker_socket: DockerSocketCapability::Unsupported,
        };
        Ok(InventoryReport {
            host_epoch: must(HostEpoch::new(1), "epoch")?,
            control_plane_incarnation: must(
                ControlPlaneIncarnation::try_new(uuid::Uuid::new_v4()),
                "incarnation",
            )?,
            authority_session_id: must(
                AuthoritySessionId::try_new(uuid::Uuid::new_v4()),
                "session",
            )?,
            boot_id: text("boot")?,
            inventory_sequence: must(BrowserCounter::new(1), "sequence")?,
            observed_at: must(WireTimestamp::from_millis(1_000), "time")?,
            domains: vec![ObservedDomain {
                local_key: text("root")?,
                known_id: None,
                parent_local_key: None,
                kind: DomainKind::Physical,
                name: text("physical")?,
                runtime_identity: None,
                capacity: ObservedCapacity {
                    cpu_millis: None,
                    memory_mib: None,
                    disk_bytes: None,
                },
            }],
            backends: vec![ObservedBackend {
                local_key: text("native")?,
                known_id: None,
                domain_local_key: text("root")?,
                runtime_kind: RuntimeKind::NativeProcess,
                execution_os: ExecutionOs::Linux,
                architecture: Architecture::X64,
                capabilities: must(BackendCapabilities::try_from(report), "capabilities")?,
                readiness: BackendReadiness::Unknown,
                reason: None,
                runtime_identity: None,
            }],
            interactive_sessions: Vec::new(),
            samples: vec![ObservedSample {
                domain_local_key: text("root")?,
                observed_at: must(WireTimestamp::from_millis(1_000), "time")?,
                coverage: SampleCoverage::Partial,
                cpu_used_millis: None,
                memory_used_mib: None,
                disk_free_bytes: None,
                uptime_seconds: None,
            }],
        })
    }

    #[test]
    fn complete_inventory_rejects_duplicate_or_missing_domain_keys() -> Result<()> {
        let mut value = report()?;
        value.domains.push(value.domains[0].clone());
        assert!(value.validate_shape().is_err());
        let mut value = report()?;
        value.backends[0].domain_local_key = text("missing")?;
        assert!(value.validate_shape().is_err());
        Ok(())
    }

    #[test]
    fn backend_identity_must_match_typed_capabilities() -> Result<()> {
        let mut value = report()?;
        value.backends[0].execution_os = ExecutionOs::Windows;
        assert!(value.validate_shape().is_err());
        Ok(())
    }

    #[test]
    fn wire_deserialization_applies_protocol_and_inventory_bounds() -> Result<()> {
        assert!(serde_json::from_str::<ProtocolRange>(r#"{"min":0,"max":1}"#).is_err());
        assert!(serde_json::from_str::<ProtocolRange>(r#"{"min":2,"max":1}"#).is_err());
        let encoded = serde_json::to_value(report()?)?;
        let mut unknown = encoded.clone();
        unknown["unexpected"] = true.into();
        assert!(serde_json::from_value::<InventoryReport>(unknown).is_err());
        let mut too_many = encoded;
        too_many["domains"] = (0..=64)
            .map(|index| {
                serde_json::json!({
                    "local_key": format!("domain-{index}"),
                    "known_id": null,
                    "parent_local_key": if index == 0 { serde_json::Value::Null } else { serde_json::json!("domain-0") },
                    "kind": if index == 0 { "physical" } else { "container" },
                    "name": "observed",
                    "runtime_identity": null,
                    "capacity": {"cpu_millis": null,"memory_mib": null,"disk_bytes": null}
                })
            })
            .collect();
        assert!(serde_json::from_value::<InventoryReport>(too_many).is_err());
        Ok(())
    }

    #[derive(Debug)]
    struct TestClock(AtomicI64);

    impl RegistryClock for TestClock {
        fn now_millis(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    struct Fixture {
        directory: std::path::PathBuf,
        pool: SqlitePool,
        clock: Arc<TestClock>,
        service: crate::fleet::registry::RegistryService,
    }

    impl Fixture {
        async fn new() -> Result<Self> {
            let directory =
                std::env::temp_dir().join(format!("gridops-inventory-{}", uuid::Uuid::new_v4()));
            let pool = connect_database_path(&directory.join("inventory.sqlite")).await?;
            sqlx::query("INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at) VALUES ('inventory-admin',99,'inventory-admin','admin','',?,?,?)")
                .bind(1_000_i64).bind(1_000_i64).bind(1_000_i64).execute(&pool).await?;
            sqlx::query("INSERT INTO fleet_control_plane (singleton,incarnation,readiness,schema_version,protocol_min,protocol_max,updated_at) VALUES (1,?,'paused',1,1,1,?)")
                .bind(uuid::Uuid::new_v4().to_string()).bind(1_000_i64).execute(&pool).await?;
            let clock = Arc::new(TestClock(AtomicI64::new(1_000)));
            let service = crate::fleet::registry::RegistryService::with_clock(
                pool.clone(),
                Vault::test_with_secret(b"inventory-test-secret"),
                clock.clone(),
            );
            Ok(Self {
                directory,
                pool,
                clock,
                service,
            })
        }

        async fn close(self) -> Result<()> {
            self.pool.close().await;
            std::fs::remove_dir_all(self.directory)?;
            Ok(())
        }

        async fn host(&self) -> Result<(crate::fleet::registry::AuthenticatedHost, String)> {
            let principal = Principal::AuthenticatedUser {
                user_id: "inventory-admin".to_owned(),
            };
            let issue = match self
                .service
                .issue_enrollment(
                    &principal,
                    IssueEnrollment {
                        idempotency_key: "inventory-issue".to_owned(),
                        scope: EnrollmentScope::new([])?,
                        intended_host_id: None,
                        ttl_millis: None,
                    },
                )
                .await?
            {
                crate::fleet::registry::EnrollmentIssue::Created(value) => value,
                crate::fleet::registry::EnrollmentIssue::Replay(_) => {
                    anyhow::bail!("unexpected enrollment replay")
                }
            };
            let receipt = self
                .service
                .consume_enrollment(
                    issue.code.expose(),
                    HostRegistration::new(
                        "inventory-host",
                        crate::fleet::ExecutionOs::Linux,
                        crate::fleet::Architecture::X64,
                        None,
                    )?,
                )
                .await?;
            let secret = receipt.credential.expose().to_owned();
            let authenticated = self.service.authenticate(receipt.host_id, &secret).await?;
            Ok((authenticated, secret))
        }

        async fn incarnation(&self) -> Result<crate::fleet::ids::ControlPlaneIncarnation> {
            Ok(sqlx::query_scalar::<_, String>(
                "SELECT incarnation FROM fleet_control_plane WHERE singleton=1",
            )
            .fetch_one(&self.pool)
            .await?
            .parse()?)
        }
    }

    fn request_id() -> Result<OperationId> {
        must(OperationId::try_new(uuid::Uuid::new_v4()), "request id")
    }

    #[tokio::test]
    async fn disk_budget_backfills_for_hosts_enrolled_before_disk_probing() -> Result<()> {
        let fixture = Fixture::new().await?;
        let (authenticated, _) = fixture.host().await?;
        let incarnation = fixture.incarnation().await?;
        let session_id = AuthoritySessionId::try_new(uuid::Uuid::new_v4())?;
        let boot_id = text("inventory-boot")?;
        let handshake = HandshakeRequest::new(
            request_id()?,
            session_id,
            boot_id.clone(),
            ProtocolRange::new(1, 1)?,
            text("agent-1")?,
            Some(HostEpoch::new(1)?),
        );
        fixture.service.handshake(&authenticated, handshake).await?;
        let host = authenticated.host_id.to_string();

        // The first report predates disk probing: cpu and memory only.
        let mut without_disk = report()?;
        without_disk.control_plane_incarnation = incarnation;
        without_disk.authority_session_id = session_id;
        without_disk.boot_id = boot_id.clone();
        without_disk.domains[0].capacity.cpu_millis = Some(must(
            crate::fleet::protocol::primitives::BrowserUint53::new(4_000),
            "cpu",
        )?);
        without_disk.domains[0].capacity.memory_mib = Some(must(
            crate::fleet::protocol::primitives::BrowserUint53::new(1_024),
            "memory",
        )?);
        fixture
            .service
            .submit_inventory(
                &authenticated,
                InventoryRequest::new(request_id()?, without_disk)?,
            )
            .await?;
        let initial: (i64, i64, i64) = sqlx::query_as(
            "SELECT cpu_millis,memory_mib,disk_bytes FROM host_resource_domains WHERE host_id=?",
        )
        .bind(&host)
        .fetch_one(&fixture.pool)
        .await?;
        assert_eq!(initial, (3_200, 816, 0));

        // The upgraded agent adds disk: the missing budget backfills at the
        // same 80% share without touching the budgets already set.
        let mut with_disk = report()?;
        with_disk.control_plane_incarnation = incarnation;
        with_disk.authority_session_id = session_id;
        with_disk.boot_id = boot_id;
        with_disk.inventory_sequence = must(BrowserCounter::new(2), "sequence")?;
        with_disk.observed_at = must(WireTimestamp::from_millis(2_000), "time")?;
        with_disk.domains[0].capacity.cpu_millis = Some(must(
            crate::fleet::protocol::primitives::BrowserUint53::new(4_000),
            "cpu",
        )?);
        with_disk.domains[0].capacity.memory_mib = Some(must(
            crate::fleet::protocol::primitives::BrowserUint53::new(1_024),
            "memory",
        )?);
        with_disk.domains[0].capacity.disk_bytes = Some(must(
            crate::fleet::protocol::primitives::BrowserUint53::new(1_000_000_000_000),
            "disk",
        )?);
        with_disk.samples[0].disk_free_bytes = Some(must(
            crate::fleet::protocol::primitives::BrowserUint53::new(400_000_000_000),
            "free",
        )?);
        with_disk.samples[0].observed_at = must(WireTimestamp::from_millis(2_000), "time")?;
        fixture
            .service
            .submit_inventory(
                &authenticated,
                InventoryRequest::new(request_id()?, with_disk)?,
            )
            .await?;

        let backfilled: (i64, i64, i64) = sqlx::query_as(
            "SELECT cpu_millis,memory_mib,disk_bytes FROM host_resource_domains WHERE host_id=?",
        )
        .bind(&host)
        .fetch_one(&fixture.pool)
        .await?;
        assert_eq!(backfilled, (3_200, 816, 800_000_000_000));
        let free: Option<i64> = sqlx::query_scalar(
            "SELECT disk_free_bytes FROM host_samples WHERE host_id=? ORDER BY observed_at DESC LIMIT 1",
        )
        .bind(&host)
        .fetch_one(&fixture.pool)
        .await?;
        assert_eq!(free, Some(400_000_000_000));
        fixture.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn service_reconciles_and_replays_complete_inventory_without_refreshing_receipt()
    -> Result<()> {
        let fixture = Fixture::new().await?;
        let (authenticated, _) = fixture.host().await?;
        let incarnation = fixture.incarnation().await?;
        let session_id = AuthoritySessionId::try_new(uuid::Uuid::new_v4())?;
        let boot_id = text("inventory-boot")?;
        let handshake = HandshakeRequest::new(
            request_id()?,
            session_id,
            boot_id.clone(),
            ProtocolRange::new(1, 1)?,
            text("agent-1")?,
            Some(HostEpoch::new(1)?),
        );
        let handshake_response = fixture.service.handshake(&authenticated, handshake).await?;
        assert!(handshake_response.lease.is_some());
        let mut inventory = report()?;
        inventory.control_plane_incarnation = incarnation;
        inventory.authority_session_id = session_id;
        inventory.boot_id = boot_id;
        let request = InventoryRequest::new(request_id()?, inventory)?;
        let first = fixture
            .service
            .submit_inventory(&authenticated, request.clone())
            .await?;
        assert_eq!(first.inventory_revision.get(), 1);
        assert_eq!(first.domain_ids.len(), 1);
        assert_eq!(first.backend_ids.len(), 1);
        let complete_at: i64 =
            sqlx::query_scalar("SELECT last_inventory_at FROM fleet_hosts WHERE id=?")
                .bind(authenticated.host_id().to_string())
                .fetch_one(&fixture.pool)
                .await?;
        let replay = fixture
            .service
            .submit_inventory(&authenticated, request)
            .await?;
        assert_eq!(replay.digest, first.digest);
        assert_eq!(replay.inventory_revision, first.inventory_revision);
        assert_eq!(replay.domain_ids, first.domain_ids);
        assert_eq!(replay.backend_ids, first.backend_ids);
        let complete_after: i64 =
            sqlx::query_scalar("SELECT last_inventory_at FROM fleet_hosts WHERE id=?")
                .bind(authenticated.host_id().to_string())
                .fetch_one(&fixture.pool)
                .await?;
        assert_eq!(complete_after, complete_at);
        let heartbeat = HeartbeatRequest::new(
            request_id()?,
            authenticated.epoch(),
            incarnation,
            session_id,
            text("inventory-boot")?,
            crate::fleet::protocol::primitives::BrowserCounter::new(1)?,
            WireTimestamp::from_millis(2_000)?,
            vec![inventory_sample("root")?],
        )?;
        fixture.service.heartbeat(&authenticated, heartbeat).await?;
        let (complete, last_inventory, last_heartbeat): (i64, i64, i64) = sqlx::query_as("SELECT inventory_complete,last_inventory_at,last_heartbeat_at FROM fleet_hosts WHERE id=?").bind(authenticated.host_id().to_string()).fetch_one(&fixture.pool).await?;
        assert_eq!(complete, 1);
        assert_eq!(last_inventory, complete_at);
        assert_eq!(last_heartbeat, fixture.clock.now_millis());
        fixture.close().await
    }

    #[tokio::test]
    async fn live_conflicting_session_is_fenced_and_quarantined() -> Result<()> {
        let fixture = Fixture::new().await?;
        let (authenticated, _) = fixture.host().await?;
        let first = AuthoritySessionId::try_new(uuid::Uuid::new_v4())?;
        fixture
            .service
            .handshake(
                &authenticated,
                HandshakeRequest::new(
                    request_id()?,
                    first,
                    text("boot-a")?,
                    ProtocolRange::new(1, 1)?,
                    text("agent")?,
                    Some(HostEpoch::new(1)?),
                ),
            )
            .await?;
        let second = AuthoritySessionId::try_new(uuid::Uuid::new_v4())?;
        let outcome = fixture
            .service
            .handshake(
                &authenticated,
                HandshakeRequest::new(
                    request_id()?,
                    second,
                    text("boot-b")?,
                    ProtocolRange::new(1, 1)?,
                    text("agent")?,
                    Some(HostEpoch::new(1)?),
                ),
            )
            .await;
        assert!(matches!(
            outcome,
            Err(crate::fleet::registry::InventoryError::SessionConflict)
        ));
        let (integrity, complete): (String, i64) =
            sqlx::query_as("SELECT integrity_state,inventory_complete FROM fleet_hosts WHERE id=?")
                .bind(authenticated.host_id().to_string())
                .fetch_one(&fixture.pool)
                .await?;
        assert_eq!(integrity, "quarantined");
        assert_eq!(complete, 0);
        fixture.close().await
    }

    fn inventory_sample(domain: &str) -> Result<ObservedSample> {
        Ok(ObservedSample {
            domain_local_key: text(domain)?,
            observed_at: must(WireTimestamp::from_millis(2_000), "sample time")?,
            coverage: SampleCoverage::Partial,
            cpu_used_millis: None,
            memory_used_mib: None,
            disk_free_bytes: None,
            uptime_seconds: None,
        })
    }

    mod inventory_regression_tests {
        include!("inventory_regression_tests.rs");
    }
}
