//! Durable fleet-domain identifiers, lifecycle values, and authorization.

mod admission;
pub mod authorization;
pub mod capabilities;
pub mod cursor;
pub mod domain;
pub mod ids;
pub mod placement;
pub mod protocol;
pub mod registry;
pub mod service;
mod store;
pub mod targets;
pub mod workload;

pub use service::{
    FleetError, FleetService, OperationStatus, Submission, SubmissionState, SubmitIntent,
    WorkloadKind,
};

pub use authorization::{
    AuthorizationError, AuthorizationProof, Capability, DockerSocketProof, ExecuteSandboxProof,
    GrantSnapshot, InteractiveSessionProof, NativeExecutionProof, OwnedFixPlacement, Principal,
    SessionGrantSnapshot,
};
pub use domain::{
    AllocationReleaseError, AllocationState, Architecture, BackendReadiness, BitbucketConnectionId,
    CiPlatform, CiTarget, DelayedStartExclusion, EnrollmentState, ExecutionOs, ExternalId,
    ExternalIdError, Freshness, HostIntent, HostLifecycle, IntegrityState, LifecycleAxis,
    LifecycleTransitionError, LifecycleValidationError, NonNilTargetUuid, PlacementOwnership,
    PlacementState, PressureState, ProvenLocalAbsence, ProviderCleanupState, ResourceAmount,
    ResourceError, ResourceRequest, RuntimeKind,
};
pub use ids::{
    AllocationId, BackendId, CiTargetId, CommandId, CounterError, CounterKind, FleetIdParseError,
    FleetTextError, Generation, HostEpoch, HostId, MAX_COUNTER, NilFleetUuid, NonNilUuid,
    OperationId, PlacementId, ProfileId, ResourceDomainId, Revision, SessionId, UpstreamText,
    WorkloadId,
};

/// Creates the fleet control plane on first start. An existing row keeps its
/// incarnation and readiness, so restarts never invalidate agent sessions.
pub async fn ensure_control_plane(pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO fleet_control_plane
         (singleton,incarnation,readiness,schema_version,protocol_min,protocol_max,updated_at)
         VALUES (1,?,'active',1,1,1,?) ON CONFLICT(singleton) DO NOTHING",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(crate::now_millis())
    .execute(pool)
    .await?;
    Ok(())
}
