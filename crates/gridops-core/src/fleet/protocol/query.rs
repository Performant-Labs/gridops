//! Version-one read-only fleet query contracts.
//!
//! These DTOs are deliberately separate from the legacy browser summaries:
//! host and target projections are distinct tagged shapes, values are checked
//! at construction/deserialization, and no raw fleet configuration crosses the
//! browser boundary.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize, Serializer, de::Deserializer};

use super::{
    browser::{BoundedItems, OpaqueCursor, ResourceAmountDto},
    primitives::{
        BrowserCounter, BrowserUint53, ContractError, ProtocolVersion, Sha256Digest, WireTimestamp,
    },
};
use crate::fleet::{
    Architecture, BackendReadiness, EnrollmentState, ExecutionOs, Freshness, HostIntent,
    IntegrityState, PressureState, RuntimeKind,
    ids::{
        BackendId, CiTargetId, EventId, HostId, PlacementId, ResourceDomainId, Revision,
        UpstreamText,
    },
};

pub const QUERY_SCHEMA_VERSION: u8 = 1;

/// A bounded, checked string map for administrator-owned host tags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CheckedTags(BTreeMap<String, String>);

impl CheckedTags {
    pub fn new(values: BTreeMap<String, String>) -> Result<Self, ContractError> {
        if values.len() > 64
            || values.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 128
                    || value.len() > 128
                    || key.chars().any(char::is_control)
                    || value.chars().any(char::is_control)
            })
        {
            return Err(ContractError::InvalidText);
        }
        Ok(Self(values))
    }

    #[must_use]
    pub fn as_map(&self) -> &BTreeMap<String, String> {
        &self.0
    }
}

impl<'de> Deserialize<'de> for CheckedTags {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(BTreeMap::<String, String>::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryResult<T> {
    schema_version: ProtocolVersion,
    pub read_at: WireTimestamp,
    pub item: T,
}

impl<T> QueryResult<T> {
    pub fn new(read_at: WireTimestamp, item: T) -> Self {
        Self {
            schema_version: query_schema_version(),
            read_at,
            item,
        }
    }

    #[must_use]
    pub fn schema_version(&self) -> u8 {
        QUERY_SCHEMA_VERSION
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryPage<T> {
    schema_version: ProtocolVersion,
    pub read_at: WireTimestamp,
    pub items: BoundedItems<T, 100>,
    pub next_cursor: Option<OpaqueCursor>,
}

impl<T> QueryPage<T> {
    pub fn new(
        read_at: WireTimestamp,
        items: Vec<T>,
        next_cursor: Option<OpaqueCursor>,
    ) -> Result<Self, ContractError> {
        Ok(Self {
            schema_version: query_schema_version(),
            read_at,
            items: BoundedItems::new(items)?,
            next_cursor,
        })
    }

    #[must_use]
    pub fn schema_version(&self) -> u8 {
        QUERY_SCHEMA_VERSION
    }
}

fn query_schema_version() -> ProtocolVersion {
    match ProtocolVersion::new(u64::from(QUERY_SCHEMA_VERSION)) {
        Ok(version) => version,
        Err(_) => unreachable!("query schema version is fixed at one"),
    }
}

#[allow(clippy::large_enum_variant)] // Tagged projections intentionally keep restricted fields absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "projection",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HostReadV1 {
    Host {
        id: HostId,
        name: UpstreamText,
        host_os: ExecutionOs,
        architecture: Architecture,
        enrollment: EnrollmentState,
        lifecycle: HostIntent,
        integrity: IntegrityState,
        heartbeat_freshness: Freshness,
        pressure: PressureState,
        policy_revision: BrowserCounter,
        agent_version: Option<UpstreamText>,
        last_heartbeat_at: Option<WireTimestamp>,
        last_inventory_at: Option<WireTimestamp>,
        backend_count: BrowserUint53,
        counts: FleetRecordCounts,
        tags: CheckedTags,
        physical_budget: ResourceAmountDto,
        physical_allocation: AllocationReadV1,
        latest_root_measurement: Option<MeasurementReadV1>,
        inventory_evidence: InventoryEvidenceV1,
        approval_evidence: ApprovalEvidenceV1,
        /// Another approved, unretired host reports the same hardware
        /// fingerprint. This is a hint for the approver, never proof of
        /// identity or ownership.
        duplicate_hardware: bool,
    },
    Target {
        id: HostId,
        name: UpstreamText,
        host_os: ExecutionOs,
        architecture: Architecture,
        counts: TargetFleetRecordCounts,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "projection",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BackendReadV1 {
    Host {
        id: BackendId,
        host_id: HostId,
        domain_id: ResourceDomainId,
        runtime_kind: RuntimeKind,
        execution_os: ExecutionOs,
        architecture: Architecture,
        enabled: bool,
        policy_revision: BrowserCounter,
        observed_readiness: BackendReadiness,
        probe_freshness: Freshness,
        safe_reason: Option<BackendSafeReasonV1>,
        counts: FleetRecordCounts,
        allocation: AllocationReadV1,
    },
    Target {
        id: BackendId,
        host_id: HostId,
        runtime_kind: RuntimeKind,
        execution_os: ExecutionOs,
        architecture: Architecture,
        counts: TargetFleetRecordCounts,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendSafeReasonV1 {
    CapabilityUnverified,
    IncompatibleRuntime,
    IncompatibleOs,
    IncompatibleArchitecture,
    Offline,
    PressureUnknown,
    ResourcePressure,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountScope {
    Host,
    VisibleTargets,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FleetRecordCounts {
    pub scope: CountScope,
    pub known_managed_runner_count: BrowserUint53,
    pub charged_ci_placement_count: BrowserUint53,
    pub charged_fix_placement_count: BrowserUint53,
    pub running_ci_placement_count: BrowserUint53,
    pub running_fix_placement_count: BrowserUint53,
    pub uncertain_placement_count: BrowserUint53,
}

/// Counts exposed by a target-scoped projection must describe only targets
/// visible through that projection. The wrapped shape remains wire-compatible
/// with `FleetRecordCounts` while making the scope invariant explicit in Rust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetFleetRecordCounts(FleetRecordCounts);

impl TargetFleetRecordCounts {
    #[must_use]
    pub fn as_counts(&self) -> &FleetRecordCounts {
        &self.0
    }
}

impl AsRef<FleetRecordCounts> for TargetFleetRecordCounts {
    fn as_ref(&self) -> &FleetRecordCounts {
        &self.0
    }
}

impl TryFrom<FleetRecordCounts> for TargetFleetRecordCounts {
    type Error = ContractError;

    fn try_from(value: FleetRecordCounts) -> Result<Self, Self::Error> {
        if value.scope == CountScope::VisibleTargets {
            Ok(Self(value))
        } else {
            Err(ContractError::InvalidJson)
        }
    }
}

impl Serialize for TargetFleetRecordCounts {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TargetFleetRecordCounts {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_from(FleetRecordCounts::deserialize(deserializer)?)
            .map_err(|error| serde::de::Error::custom(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocationReadV1 {
    pub charged: ResourceAmountDto,
    pub uncertain: ResourceAmountDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeasurementReadV1 {
    pub observed_at: WireTimestamp,
    pub received_at: WireTimestamp,
    pub coverage: CoverageV1,
    pub provenance: SampleProvenanceV1,
    pub cpu_used_millis: Option<BrowserUint53>,
    pub memory_used_mib: Option<BrowserUint53>,
    pub disk_free_bytes: Option<BrowserUint53>,
    pub uptime_seconds: Option<BrowserUint53>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageV1 {
    Complete,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum InventoryEvidenceV1 {
    Current {
        inventory_revision: BrowserCounter,
        digest: Sha256Digest,
        received_at: WireTimestamp,
    },
    Stale {
        inventory_revision: Option<BrowserCounter>,
        digest: Option<Sha256Digest>,
        received_at: Option<WireTimestamp>,
    },
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ApprovalEvidenceV1 {
    Available {
        expected_revision: BrowserCounter,
        expected_inventory_revision: BrowserCounter,
        expected_inventory_digest: Sha256Digest,
        inventory_received_at: WireTimestamp,
        observation_lease_expires_at: WireTimestamp,
    },
    Unavailable {
        reason: ApprovalUnavailableReasonV1,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalUnavailableReasonV1 {
    RequiresSystemAdmin,
    PendingHost,
    RetiredHost,
    MissingControlPlane,
    MissingObservation,
    ObservationExpired,
    ObservationConflicted,
    StaleInventory,
    InvalidInventoryReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DomainReadV1 {
    pub id: ResourceDomainId,
    pub host_id: HostId,
    pub parent_id: Option<ResourceDomainId>,
    pub name: UpstreamText,
    pub kind: Option<ObservedDomainKindV1>,
    pub policy_revision: BrowserCounter,
    pub budget: ResourceAmountDto,
    pub allocation: AllocationReadV1,
    pub observed_capacity: ObservedCapacityV1,
    pub latest_measurement: Option<MeasurementReadV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedDomainKindV1 {
    Physical,
    VirtualMachine,
    Container,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ObservedCapacityV1 {
    Current {
        received_at: WireTimestamp,
        cpu_millis: Option<BrowserUint53>,
        memory_mib: Option<BrowserUint53>,
        disk_bytes: Option<BrowserUint53>,
    },
    Historical {
        received_at: WireTimestamp,
        cpu_millis: Option<BrowserUint53>,
        memory_mib: Option<BrowserUint53>,
        disk_bytes: Option<BrowserUint53>,
    },
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SampleReadV1 {
    pub domain_id: Option<ResourceDomainId>,
    pub observed_at: WireTimestamp,
    pub received_at: WireTimestamp,
    pub coverage: CoverageV1,
    pub provenance: SampleProvenanceV1,
    pub cpu_used_millis: Option<BrowserUint53>,
    pub memory_used_mib: Option<BrowserUint53>,
    pub disk_free_bytes: Option<BrowserUint53>,
    pub uptime_seconds: Option<BrowserUint53>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleProvenanceV1 {
    Current,
    Historical,
    LegacyUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostEventReadV1 {
    pub id: EventId,
    pub host_id: HostId,
    pub target_id: Option<CiTargetId>,
    pub placement_id: Option<PlacementId>,
    pub kind: HostEventKindV1,
    pub occurred_at: WireTimestamp,
    pub cursor: OpaqueCursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostEventKindV1 {
    EnrollmentIssued,
    EnrollmentConsumed,
    EnrollmentRevoked,
    HostApproved,
    HostRecovered,
    HostCredentialRotated,
    HostCredentialRevoked,
    InventoryReceived,
    InventoryConflict,
    BackendChanged,
    PlacementQueued,
    PlacementAdmitted,
    PlacementChanged,
    CommandChanged,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevisionValue(pub Revision);

/// Core request values are already parsed by the API boundary. They remain
/// public so the transport layer can pass the exact normalized filters into a
/// single snapshot transaction without reinterpreting raw query text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogQuery {
    pub limit: usize,
    pub cursor: Option<OpaqueCursor>,
    pub target_id: Option<CiTargetId>,
}

impl Default for CatalogQuery {
    fn default() -> Self {
        Self {
            limit: 25,
            cursor: None,
            target_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleQuery {
    pub limit: usize,
    pub cursor: Option<OpaqueCursor>,
    pub domain_id: Option<ResourceDomainId>,
    pub received_from: WireTimestamp,
    pub received_to: WireTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventQuery {
    pub limit: usize,
    pub cursor: Option<OpaqueCursor>,
    pub occurred_from: Option<WireTimestamp>,
    pub occurred_to: Option<WireTimestamp>,
    pub target_id: Option<CiTargetId>,
}

impl Default for EventQuery {
    fn default() -> Self {
        Self {
            limit: 25,
            cursor: None,
            occurred_from: None,
            occurred_to: None,
            target_id: None,
        }
    }
}
