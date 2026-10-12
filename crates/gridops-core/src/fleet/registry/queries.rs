//! Snapshot-consistent, read-only fleet queries.
//!
//! Every method opens one ordinary `SQLite` read transaction, loads the current
//! user and control-plane incarnation first, applies authorization in SQL
//! before ordering/limiting, and closes the transaction before returning. The
//! module exposes only checked projections; raw config, inventory JSON,
//! credentials, actor data, and host runtime identities never leave it.

use std::{collections::BTreeMap, fmt};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sqlx::{Row, Sqlite, Transaction};
use thiserror::Error;
use uuid::Uuid;

use super::RegistryService;
use crate::fleet::{
    Architecture, BackendReadiness, EnrollmentState, ExecutionOs, Freshness, HostIntent,
    IntegrityState, PressureState, RuntimeKind,
    authorization::Principal,
    cursor::{CursorCodec, CursorError, CursorPosition, CursorScope, CursorStream, RetainedFrom},
    ids::{BackendId, CiTargetId, ControlPlaneIncarnation, EventId, HostId, ResourceDomainId},
    protocol::{
        browser::{OpaqueCursor, ResourceAmountDto},
        primitives::{BrowserCounter, BrowserUint53, ContractError, Sha256Digest, WireTimestamp},
        query::{
            AllocationReadV1, ApprovalEvidenceV1, ApprovalUnavailableReasonV1, BackendReadV1,
            BackendSafeReasonV1, CatalogQuery, CheckedTags, CountScope, CoverageV1, DomainReadV1,
            EventQuery, FleetRecordCounts, HostEventKindV1, HostEventReadV1, HostReadV1,
            InventoryEvidenceV1, MeasurementReadV1, ObservedCapacityV1, ObservedDomainKindV1,
            QueryPage, QueryResult, SampleProvenanceV1, SampleQuery, SampleReadV1,
        },
    },
};

const MAX_PAGE_SIZE: usize = 100;
const CURSOR_TTL_MILLIS: i64 = 15 * 60 * 1000;
const CURSOR_PADDED_BYTES: usize = 2048;
const CURSOR_PREFIX: &str = "q1.";
const FRESH_HEARTBEAT_MILLIS: i64 = 90_000;
const FRESH_INVENTORY_MILLIS: i64 = 45_000;
const FRESH_PROBE_MILLIS: i64 = 45_000;

/// A stable error boundary for browser read handlers. Details intentionally do
/// not contain SQL, raw cursor text, stored JSON, or resource existence hints.
#[derive(Debug, Error)]
pub enum QueryError {
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("query is not authorized")]
    Forbidden,
    #[error("query resource was not found")]
    NotFound,
    #[error("query request is invalid: {0}")]
    InvalidRequest(&'static str),
    #[error("query cursor is invalid")]
    InvalidCursor,
    #[error("query cursor expired")]
    CursorExpired,
    #[error("query cursor history is unavailable")]
    CursorGap,
    #[error("query dependency is unavailable")]
    DependencyUnavailable,
    #[error("stored fleet data violates the read contract")]
    MalformedData,
    #[error("query response contract is invalid")]
    Contract,
}

impl From<ContractError> for QueryError {
    fn from(_: ContractError) -> Self {
        Self::Contract
    }
}

#[derive(Debug, Clone)]
struct Snapshot {
    user_id: String,
    system_admin: bool,
    incarnation: ControlPlaneIncarnation,
    now: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostVisibility {
    Physical,
    TargetOnly,
}

#[derive(Debug, Clone, Copy, Default)]
struct ReadableTargets;

impl RegistryService {
    /// List hosts visible to the current browser user, ordered by UUID.
    pub async fn list_hosts(
        &self,
        principal: &Principal,
        request: CatalogQuery,
    ) -> Result<QueryPage<HostReadV1>, QueryError> {
        let request = checked_catalog(request)?;
        let (mut tx, snapshot) = self.open_snapshot(principal).await?;
        let targets = Self::readable_targets(&mut tx, &snapshot)?;
        let target_filter = self
            .target_filter(&mut tx, &snapshot, request.target_id)
            .await?;
        let resource = "fleet:hosts";
        let normalized = catalog_scope("hosts", &request, target_filter.as_deref());
        let scope = browser_scope(principal, resource, &normalized)?;
        let after = match request.cursor.as_ref() {
            Some(cursor) => Some(self.decode_entity_cursor(
                cursor,
                &scope,
                snapshot.incarnation,
                CursorStream::Hosts,
                RetainedFrom::Catalog,
                snapshot.now,
            )?),
            None => None,
        };
        let ids = visible_host_ids(
            &mut tx,
            &snapshot,
            &targets,
            target_filter.as_deref(),
            after,
            request.limit,
        )
        .await?;
        let has_more = ids.len() > request.limit;
        let page_ids = ids.into_iter().take(request.limit).collect::<Vec<_>>();
        let mut items = Vec::with_capacity(page_ids.len());
        for host_id in &page_ids {
            let host_id = parse_host_id(host_id)?;
            let visibility = host_visibility(
                &mut tx,
                &snapshot,
                &targets,
                host_id,
                target_filter
                    .as_deref()
                    .and_then(|value| value.parse().ok()),
            )
            .await?;
            items.push(
                self.host_projection(
                    &mut tx,
                    &snapshot,
                    &targets,
                    host_id,
                    visibility,
                    target_filter.as_deref(),
                )
                .await?,
            );
        }
        let next_cursor = if has_more {
            page_ids
                .last()
                .map(|id| {
                    self.issue_entity_cursor(
                        &scope,
                        snapshot.incarnation,
                        CursorStream::Hosts,
                        id,
                        snapshot.now,
                    )
                })
                .transpose()?
        } else {
            None
        };
        let read_at = wire_timestamp(snapshot.now)?;
        tx.commit().await?;
        QueryPage::new(read_at, items, next_cursor).map_err(Into::into)
    }

    /// Read one host using a full physical or restricted target projection.
    pub async fn get_host(
        &self,
        principal: &Principal,
        host_id: HostId,
    ) -> Result<QueryResult<HostReadV1>, QueryError> {
        let (mut tx, snapshot) = self.open_snapshot(principal).await?;
        let targets = Self::readable_targets(&mut tx, &snapshot)?;
        let visibility = host_visibility(&mut tx, &snapshot, &targets, host_id, None).await?;
        let item = self
            .host_projection(&mut tx, &snapshot, &targets, host_id, visibility, None)
            .await?;
        let read_at = wire_timestamp(snapshot.now)?;
        tx.commit().await?;
        Ok(QueryResult::new(read_at, item))
    }

    /// List backends visible through physical host access or target use/config.
    pub async fn list_backends(
        &self,
        principal: &Principal,
        host_id: HostId,
        request: CatalogQuery,
    ) -> Result<QueryPage<BackendReadV1>, QueryError> {
        let request = checked_catalog(request)?;
        let (mut tx, snapshot) = self.open_snapshot(principal).await?;
        let targets = Self::readable_targets(&mut tx, &snapshot)?;
        let target_filter = self
            .target_filter(&mut tx, &snapshot, request.target_id)
            .await?
            .map(|value| value.parse())
            .transpose()
            .map_err(|_| QueryError::MalformedData)?;
        let visibility =
            host_visibility(&mut tx, &snapshot, &targets, host_id, target_filter).await?;
        let resource = format!("fleet:hosts/{host_id}/backends");
        let normalized = catalog_scope(
            "backends",
            &request,
            target_filter.as_ref().map(ToString::to_string).as_deref(),
        );
        let scope = browser_scope(principal, &resource, &normalized)?;
        let after = match request.cursor.as_ref() {
            Some(cursor) => Some(self.decode_entity_cursor(
                cursor,
                &scope,
                snapshot.incarnation,
                CursorStream::Backends,
                RetainedFrom::Catalog,
                snapshot.now,
            )?),
            None => None,
        };
        let ids = visible_backend_ids(
            &mut tx,
            &snapshot,
            &targets,
            host_id,
            visibility,
            target_filter,
            after,
            request.limit,
        )
        .await?;
        let has_more = ids.len() > request.limit;
        let page_ids = ids.into_iter().take(request.limit).collect::<Vec<_>>();
        let mut items = Vec::with_capacity(page_ids.len());
        for id in &page_ids {
            items.push(
                self.backend_projection(
                    &mut tx,
                    &snapshot,
                    &targets,
                    host_id,
                    parse_backend_id(id)?,
                    visibility,
                    target_filter,
                )
                .await?,
            );
        }
        let next_cursor = if has_more {
            page_ids
                .last()
                .map(|id| {
                    self.issue_entity_cursor(
                        &scope,
                        snapshot.incarnation,
                        CursorStream::Backends,
                        id,
                        snapshot.now,
                    )
                })
                .transpose()?
        } else {
            None
        };
        let read_at = wire_timestamp(snapshot.now)?;
        tx.commit().await?;
        QueryPage::new(read_at, items, next_cursor).map_err(Into::into)
    }

    /// Read one backend; inaccessible detail is deliberately indistinguishable
    /// from a missing backend to the transport layer.
    pub async fn get_backend(
        &self,
        principal: &Principal,
        host_id: HostId,
        backend_id: BackendId,
    ) -> Result<QueryResult<BackendReadV1>, QueryError> {
        let (mut tx, snapshot) = self.open_snapshot(principal).await?;
        let targets = Self::readable_targets(&mut tx, &snapshot)?;
        let visibility = host_visibility(&mut tx, &snapshot, &targets, host_id, None).await?;
        let item = self
            .backend_projection(
                &mut tx, &snapshot, &targets, host_id, backend_id, visibility, None,
            )
            .await?;
        let read_at = wire_timestamp(snapshot.now)?;
        tx.commit().await?;
        Ok(QueryResult::new(read_at, item))
    }

    /// List resource domains. Domains always require full physical host access.
    pub async fn list_domains(
        &self,
        principal: &Principal,
        host_id: HostId,
        request: CatalogQuery,
    ) -> Result<QueryPage<DomainReadV1>, QueryError> {
        let request = checked_catalog(request)?;
        if request.target_id.is_some() {
            return Err(QueryError::InvalidRequest(
                "target filter is not valid for domains",
            ));
        }
        let (mut tx, snapshot) = self.open_snapshot(principal).await?;
        ensure_physical_host(&mut tx, &snapshot, host_id).await?;
        let resource = format!("fleet:hosts/{host_id}/domains");
        let normalized = catalog_scope("domains", &request, None);
        let scope = browser_scope(principal, &resource, &normalized)?;
        let after = match request.cursor.as_ref() {
            Some(cursor) => Some(self.decode_entity_cursor(
                cursor,
                &scope,
                snapshot.incarnation,
                CursorStream::Domains,
                RetainedFrom::Catalog,
                snapshot.now,
            )?),
            None => None,
        };
        let ids = domain_ids(&mut tx, host_id, after, request.limit).await?;
        let has_more = ids.len() > request.limit;
        let page_ids = ids.into_iter().take(request.limit).collect::<Vec<_>>();
        let mut items = Vec::with_capacity(page_ids.len());
        for id in &page_ids {
            items.push(
                self.domain_projection(&mut tx, &snapshot, parse_domain_id(id)?, host_id)
                    .await?,
            );
        }
        let next_cursor = if has_more {
            page_ids
                .last()
                .map(|id| {
                    self.issue_entity_cursor(
                        &scope,
                        snapshot.incarnation,
                        CursorStream::Domains,
                        id,
                        snapshot.now,
                    )
                })
                .transpose()?
        } else {
            None
        };
        let read_at = wire_timestamp(snapshot.now)?;
        tx.commit().await?;
        QueryPage::new(read_at, items, next_cursor).map_err(Into::into)
    }

    /// Read one resource domain under physical host authorization.
    pub async fn get_domain(
        &self,
        principal: &Principal,
        host_id: HostId,
        domain_id: ResourceDomainId,
    ) -> Result<QueryResult<DomainReadV1>, QueryError> {
        let (mut tx, snapshot) = self.open_snapshot(principal).await?;
        ensure_physical_host(&mut tx, &snapshot, host_id).await?;
        let item = self
            .domain_projection(&mut tx, &snapshot, domain_id, host_id)
            .await?;
        let read_at = wire_timestamp(snapshot.now)?;
        tx.commit().await?;
        Ok(QueryResult::new(read_at, item))
    }

    /// List server-received samples in an explicit, bounded half-open window.
    pub async fn list_samples(
        &self,
        principal: &Principal,
        host_id: HostId,
        request: SampleQuery,
    ) -> Result<QueryPage<SampleReadV1>, QueryError> {
        let request = checked_sample(request)?;
        let (mut tx, snapshot) = self.open_snapshot(principal).await?;
        ensure_physical_host(&mut tx, &snapshot, host_id).await?;
        if let Some(domain_id) = request.domain_id {
            ensure_domain(&mut tx, host_id, domain_id).await?;
        }
        let resource = format!("fleet:hosts/{host_id}/samples");
        let normalized = sample_scope(&request);
        let scope = browser_scope(principal, &resource, &normalized)?;
        let floor = retention_floor(&mut tx, "samples").await?;
        let after = match request.cursor.as_ref() {
            Some(cursor) => Some(self.decode_row_cursor(
                cursor,
                &scope,
                snapshot.incarnation,
                CursorStream::Samples,
                floor,
                snapshot.now,
            )?),
            None => None,
        };
        let rows = sample_rows(&mut tx, host_id, &request, after, request.limit).await?;
        let has_more = rows.len() > request.limit;
        let page_rows = rows.into_iter().take(request.limit).collect::<Vec<_>>();
        let mut items = Vec::with_capacity(page_rows.len());
        for row in &page_rows {
            items.push(sample_projection(&mut tx, &snapshot, row).await?);
        }
        let next_cursor = if has_more {
            page_rows
                .last()
                .map(|row| {
                    self.issue_row_cursor(
                        &scope,
                        snapshot.incarnation,
                        CursorStream::Samples,
                        row.id,
                        snapshot.now,
                    )
                })
                .transpose()?
        } else {
            None
        };
        let read_at = wire_timestamp(snapshot.now)?;
        tx.commit().await?;
        QueryPage::new(read_at, items, next_cursor).map_err(Into::into)
    }

    /// List metadata-only host events; target rows remain target-authorized.
    pub async fn list_host_events(
        &self,
        principal: &Principal,
        host_id: HostId,
        request: EventQuery,
    ) -> Result<QueryPage<HostEventReadV1>, QueryError> {
        let request = checked_events(request)?;
        let (mut tx, snapshot) = self.open_snapshot(principal).await?;
        let targets = Self::readable_targets(&mut tx, &snapshot)?;
        let visibility =
            host_visibility(&mut tx, &snapshot, &targets, host_id, request.target_id).await?;
        let resource = format!("fleet:hosts/{host_id}/events");
        let normalized = event_scope(&request);
        let scope = browser_scope(principal, &resource, &normalized)?;
        let floor = retention_floor(&mut tx, "fleet_events").await?;
        let after = match request.cursor.as_ref() {
            Some(cursor) => Some(self.decode_row_cursor(
                cursor,
                &scope,
                snapshot.incarnation,
                CursorStream::HostEvents,
                floor,
                snapshot.now,
            )?),
            None => None,
        };
        let rows = event_rows(
            &mut tx, &snapshot, &targets, host_id, visibility, &request, after,
        )
        .await?;
        let has_more = rows.len() > request.limit;
        let page_rows = rows.into_iter().take(request.limit).collect::<Vec<_>>();
        let mut items = Vec::with_capacity(page_rows.len());
        for row in &page_rows {
            items.push(event_projection(self, &mut tx, &snapshot, &scope, row)?);
        }
        let next_cursor = if has_more {
            page_rows
                .last()
                .map(|row| {
                    self.issue_row_cursor(
                        &scope,
                        snapshot.incarnation,
                        CursorStream::HostEvents,
                        row.cursor,
                        snapshot.now,
                    )
                })
                .transpose()?
        } else {
            None
        };
        let read_at = wire_timestamp(snapshot.now)?;
        tx.commit().await?;
        QueryPage::new(read_at, items, next_cursor).map_err(Into::into)
    }

    async fn open_snapshot(
        &self,
        principal: &Principal,
    ) -> Result<(Transaction<'_, Sqlite>, Snapshot), QueryError> {
        let Principal::AuthenticatedUser { user_id } = principal else {
            return Err(QueryError::Forbidden);
        };
        let mut tx = self.pool.begin().await?;
        let role: Option<String> = sqlx::query_scalar("SELECT role FROM users WHERE id=?")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(role) = role else {
            return Err(QueryError::Forbidden);
        };
        let incarnation: String =
            sqlx::query_scalar("SELECT incarnation FROM fleet_control_plane WHERE singleton=1")
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(QueryError::DependencyUnavailable)?;
        let incarnation = incarnation
            .parse()
            .map_err(|_| QueryError::DependencyUnavailable)?;
        let now = self.clock.now_millis();
        Ok((
            tx,
            Snapshot {
                user_id: user_id.clone(),
                system_admin: role == "admin",
                incarnation,
                now,
            },
        ))
    }

    fn readable_targets(
        _tx: &mut Transaction<'_, Sqlite>,
        _snapshot: &Snapshot,
    ) -> Result<ReadableTargets, QueryError> {
        // Authorization is intentionally evaluated by the reusable SQL CTE
        // below.  In particular, do not materialize the entire readable
        // target catalog into Rust and then expand it into SQLite bind lists.
        Ok(ReadableTargets)
    }

    async fn target_filter(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        snapshot: &Snapshot,
        target_id: Option<CiTargetId>,
    ) -> Result<Option<String>, QueryError> {
        let Some(target_id) = target_id else {
            return Ok(None);
        };
        if !target_is_readable(tx, snapshot, target_id).await? {
            return Err(QueryError::NotFound);
        }
        Ok(Some(target_id.to_string()))
    }

    fn decode_entity_cursor(
        &self,
        cursor: &OpaqueCursor,
        scope: &CursorScope,
        incarnation: ControlPlaneIncarnation,
        stream: CursorStream,
        retained: RetainedFrom,
        now: i64,
    ) -> Result<Uuid, QueryError> {
        let inner = self.open_cursor(cursor)?;
        let position = CursorCodec::new(&self.vault)
            .verify(&inner, scope, incarnation, stream, now, retained)
            .map_err(|error| map_cursor_error(&error))?;
        match position {
            CursorPosition::Entity { after } => Ok(after.as_uuid()),
            _ => Err(QueryError::InvalidCursor),
        }
    }

    fn decode_row_cursor(
        &self,
        cursor: &OpaqueCursor,
        scope: &CursorScope,
        incarnation: ControlPlaneIncarnation,
        stream: CursorStream,
        retained: RetainedFrom,
        now: i64,
    ) -> Result<i64, QueryError> {
        let inner = self.open_cursor(cursor)?;
        let position = CursorCodec::new(&self.vault)
            .verify(&inner, scope, incarnation, stream, now, retained)
            .map_err(|error| map_cursor_error(&error))?;
        match position {
            CursorPosition::Row { after } => {
                i64::try_from(after.get()).map_err(|_| QueryError::InvalidCursor)
            }
            _ => Err(QueryError::InvalidCursor),
        }
    }

    fn issue_entity_cursor(
        &self,
        scope: &CursorScope,
        incarnation: ControlPlaneIncarnation,
        stream: CursorStream,
        after: &str,
        now: i64,
    ) -> Result<OpaqueCursor, QueryError> {
        let id = after.parse().map_err(|_| QueryError::MalformedData)?;
        self.issue_inner_cursor(
            scope,
            incarnation,
            stream,
            CursorPosition::Entity {
                after: crate::fleet::ids::NonNilUuid::try_new(id)
                    .map_err(|_| QueryError::MalformedData)?,
            },
            RetainedFrom::Catalog,
            now,
        )
    }

    fn issue_row_cursor(
        &self,
        scope: &CursorScope,
        incarnation: ControlPlaneIncarnation,
        stream: CursorStream,
        after: i64,
        now: i64,
    ) -> Result<OpaqueCursor, QueryError> {
        let after = u64::try_from(after).map_err(|_| QueryError::MalformedData)?;
        let after = crate::fleet::protocol::primitives::ByteOffset::new(after)
            .map_err(|_| QueryError::MalformedData)?;
        self.issue_inner_cursor(
            scope,
            incarnation,
            stream,
            CursorPosition::Row { after },
            RetainedFrom::Catalog,
            now,
        )
    }

    fn issue_inner_cursor(
        &self,
        scope: &CursorScope,
        incarnation: ControlPlaneIncarnation,
        stream: CursorStream,
        position: CursorPosition,
        _retained: RetainedFrom,
        now: i64,
    ) -> Result<OpaqueCursor, QueryError> {
        let inner = CursorCodec::new(&self.vault)
            .issue(
                scope,
                incarnation,
                stream,
                position,
                now,
                now.checked_add(CURSOR_TTL_MILLIS)
                    .ok_or(QueryError::InvalidCursor)?,
            )
            .map_err(|error| map_cursor_error(&error))?;
        if inner.len() > CURSOR_PADDED_BYTES {
            return Err(QueryError::InvalidCursor);
        }
        let padded = format!("{inner:<CURSOR_PADDED_BYTES$}");
        let sealed = self
            .vault
            .seal(&padded)
            .map_err(|_| QueryError::DependencyUnavailable)?;
        let envelope = format!(
            "{CURSOR_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(sealed.as_bytes())
        );
        OpaqueCursor::try_from(envelope).map_err(|_| QueryError::InvalidCursor)
    }

    fn open_cursor(&self, cursor: &OpaqueCursor) -> Result<String, QueryError> {
        let value: String = cursor.clone().into();
        let encoded = value
            .strip_prefix(CURSOR_PREFIX)
            .ok_or(QueryError::InvalidCursor)?;
        let sealed = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| QueryError::InvalidCursor)?;
        if URL_SAFE_NO_PAD.encode(&sealed) != encoded {
            return Err(QueryError::InvalidCursor);
        }
        let sealed = String::from_utf8(sealed).map_err(|_| QueryError::InvalidCursor)?;
        let opened = self
            .vault
            .open(&sealed)
            .map_err(|_| QueryError::InvalidCursor)?;
        if opened.len() != CURSOR_PADDED_BYTES {
            return Err(QueryError::InvalidCursor);
        }
        let inner = opened.trim_end_matches(' ');
        if inner.is_empty() || inner.contains(' ') {
            return Err(QueryError::InvalidCursor);
        }
        Ok(inner.to_owned())
    }
}

impl fmt::Display for Snapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("fleet-query-snapshot")
    }
}

fn checked_catalog(request: CatalogQuery) -> Result<CatalogQuery, QueryError> {
    if !(1..=MAX_PAGE_SIZE).contains(&request.limit) {
        return Err(QueryError::InvalidRequest(
            "limit must be between 1 and 100",
        ));
    }
    Ok(request)
}

fn checked_sample(request: SampleQuery) -> Result<SampleQuery, QueryError> {
    if !(1..=MAX_PAGE_SIZE).contains(&request.limit) {
        return Err(QueryError::InvalidRequest(
            "limit must be between 1 and 100",
        ));
    }
    let from = request.received_from.millis();
    let to = request.received_to.millis();
    if from >= to
        || to
            .checked_sub(from)
            .ok_or(QueryError::InvalidRequest("sample range overflowed"))?
            > 24 * 60 * 60 * 1000
    {
        return Err(QueryError::InvalidRequest(
            "sample range must be positive and at most 24 hours",
        ));
    }
    Ok(request)
}

fn checked_events(request: EventQuery) -> Result<EventQuery, QueryError> {
    if !(1..=MAX_PAGE_SIZE).contains(&request.limit) {
        return Err(QueryError::InvalidRequest(
            "limit must be between 1 and 100",
        ));
    }
    match (request.occurred_from, request.occurred_to) {
        (Some(from), Some(to)) if from.millis() < to.millis() => Ok(request),
        (None, None) => Ok(request),
        _ => Err(QueryError::InvalidRequest(
            "event bounds must be supplied as an increasing pair",
        )),
    }
}

fn catalog_scope(kind: &str, request: &CatalogQuery, target_id: Option<&str>) -> String {
    format!(
        "kind={kind};limit={};target={};order=id:asc",
        request.limit,
        target_id.unwrap_or("")
    )
}

fn sample_scope(request: &SampleQuery) -> String {
    format!(
        "limit={};domain={};received_from={};received_to={};order=id:asc",
        request.limit,
        request
            .domain_id
            .map_or_else(String::new, |id| id.to_string()),
        request.received_from.millis(),
        request.received_to.millis()
    )
}

fn event_scope(request: &EventQuery) -> String {
    format!(
        "limit={};target={};from={};to={};order=cursor:asc",
        request.limit,
        request
            .target_id
            .map_or_else(String::new, |id| id.to_string()),
        request
            .occurred_from
            .map_or_else(String::new, |v| v.millis().to_string()),
        request
            .occurred_to
            .map_or_else(String::new, |v| v.millis().to_string())
    )
}

fn browser_scope(
    principal: &Principal,
    resource: &str,
    normalized: &str,
) -> Result<CursorScope, QueryError> {
    CursorScope::for_browser(principal, resource, normalized).map_err(|_| QueryError::InvalidCursor)
}

fn map_cursor_error(error: &CursorError) -> QueryError {
    match error {
        CursorError::Expired => QueryError::CursorExpired,
        CursorError::RetentionGap => QueryError::CursorGap,
        CursorError::Invalid => QueryError::InvalidCursor,
        CursorError::Signing => QueryError::DependencyUnavailable,
    }
}

fn wire_timestamp(value: i64) -> Result<WireTimestamp, QueryError> {
    WireTimestamp::from_millis(value).map_err(|_| QueryError::MalformedData)
}

fn parse_host_id(value: &str) -> Result<HostId, QueryError> {
    value.parse().map_err(|_| QueryError::MalformedData)
}
fn parse_backend_id(value: &str) -> Result<BackendId, QueryError> {
    value.parse().map_err(|_| QueryError::MalformedData)
}
fn parse_domain_id(value: &str) -> Result<ResourceDomainId, QueryError> {
    value.parse().map_err(|_| QueryError::MalformedData)
}
fn parse_event_id(value: &str) -> Result<EventId, QueryError> {
    value.parse().map_err(|_| QueryError::MalformedData)
}
fn parse_target_id(value: &str) -> Result<CiTargetId, QueryError> {
    value.parse().map_err(|_| QueryError::MalformedData)
}

/// A single scalar-bound authorization relation.  Every query that needs a
/// readable target uses this CTE instead of expanding the caller's complete
/// target catalog into `IN (?, ?, ...)`.
fn readable_targets_cte() -> &'static str {
    r#"WITH readable_targets AS (
      SELECT t.id FROM fleet_ci_targets t
       WHERE ?=1
          OR (t.kind='github_repository' AND EXISTS (
                SELECT 1 FROM user_installations ui
                 JOIN installations i ON i.id=ui.installation_id
                 JOIN repositories r ON r.id=t.repository_id
                                      AND r.installation_id=t.installation_id
                WHERE ui.user_id=? AND ui.installation_id=t.installation_id
                  AND ui.permission IN ('read','admin')
                  AND i.suspended_at IS NULL))
          OR (t.kind='github_organization' AND EXISTS (
                SELECT 1 FROM user_installations ui
                 JOIN installations i ON i.id=ui.installation_id
                WHERE ui.user_id=? AND ui.installation_id=t.installation_id
                  AND ui.permission IN ('read','admin')
                  AND i.account_id=t.organization_id
                  AND i.suspended_at IS NULL))
          OR (t.kind IN ('bitbucket_workspace','bitbucket_repository') AND EXISTS (
                SELECT 1 FROM bitbucket_connections bc
                 JOIN bitbucket_user_grants bg ON bg.connection_id=bc.id
                WHERE bc.id=t.bitbucket_connection_id
                  AND lower(trim(bc.workspace_uuid,'{}'))=lower(trim(t.workspace_uuid,'{}'))
                  AND bg.user_id=? AND bg.permission IN ('read','admin')
                  AND bg.revoked_at IS NULL))
    )"#
}

async fn target_is_readable(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    target_id: CiTargetId,
) -> Result<bool, QueryError> {
    let sql = format!(
        "{} SELECT EXISTS (SELECT 1 FROM readable_targets WHERE id=?)",
        readable_targets_cte()
    );
    let value = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql))
        .bind(i64::from(snapshot.system_admin))
        .bind(&snapshot.user_id)
        .bind(&snapshot.user_id)
        .bind(&snapshot.user_id)
        .bind(target_id.to_string())
        .fetch_one(&mut **tx)
        .await?;
    Ok(value != 0)
}

async fn visible_host_ids(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    _targets: &ReadableTargets,
    target_filter: Option<&str>,
    after: Option<Uuid>,
    limit: usize,
) -> Result<Vec<String>, QueryError> {
    let sql = format!(
        "{} SELECT h.id FROM fleet_hosts h WHERE (
           ?=1
           OR EXISTS (SELECT 1 FROM host_user_grants ug WHERE ug.host_id=h.id AND ug.user_id=? AND ug.permission IN ('read','operator','admin') AND ug.revoked_at IS NULL)
           OR EXISTS (SELECT 1 FROM host_target_grants tg JOIN readable_targets rt ON rt.id=tg.target_id WHERE tg.host_id=h.id AND tg.revoked_at IS NULL)
           OR EXISTS (SELECT 1 FROM workload_placements p JOIN readable_targets rt ON rt.id=p.target_id WHERE p.host_id=h.id)
         ){}{} ORDER BY h.id ASC LIMIT ?",
        readable_targets_cte(),
        if target_filter.is_some() {
            " AND (EXISTS (SELECT 1 FROM host_target_grants tg WHERE tg.host_id=h.id AND tg.target_id=? AND tg.revoked_at IS NULL) OR EXISTS (SELECT 1 FROM workload_placements p WHERE p.host_id=h.id AND p.target_id=?))"
        } else {
            ""
        },
        if after.is_some() { " AND h.id > ?" } else { "" }
    );
    let mut query = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql));
    query = query
        .bind(i64::from(snapshot.system_admin))
        .bind(&snapshot.user_id)
        .bind(&snapshot.user_id)
        .bind(&snapshot.user_id)
        .bind(i64::from(snapshot.system_admin))
        .bind(&snapshot.user_id);
    if let Some(target) = target_filter {
        query = query.bind(target).bind(target);
    }
    if let Some(after) = after {
        query = query.bind(after.to_string());
    }
    Ok(query
        .bind(i64::try_from(limit + 1).map_err(|_| QueryError::InvalidRequest("limit overflowed"))?)
        .fetch_all(&mut **tx)
        .await?)
}

async fn host_visibility(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    _targets: &ReadableTargets,
    host_id: HostId,
    target_filter: Option<CiTargetId>,
) -> Result<HostVisibility, QueryError> {
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM fleet_hosts WHERE id=?")
        .bind(host_id.to_string())
        .fetch_optional(&mut **tx)
        .await?;
    if let Some(target_id) = target_filter {
        if !target_is_readable(tx, snapshot, target_id).await? {
            return Err(QueryError::NotFound);
        }
    }
    if exists.is_none() {
        return Err(QueryError::NotFound);
    }
    let physical: i64 = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM host_user_grants WHERE host_id=? AND user_id=? AND permission IN ('read','operator','admin') AND revoked_at IS NULL)")
        .bind(host_id.to_string())
        .bind(&snapshot.user_id)
        .fetch_one(&mut **tx)
        .await?;
    if snapshot.system_admin || physical != 0 {
        return Ok(HostVisibility::Physical);
    }
    let readable = if let Some(target_id) = target_filter {
        sqlx::query_scalar::<_, i64>("SELECT EXISTS (SELECT 1 FROM host_target_grants WHERE host_id=? AND target_id=? AND revoked_at IS NULL UNION SELECT 1 FROM workload_placements WHERE host_id=? AND target_id=?)")
            .bind(host_id.to_string())
            .bind(target_id.to_string())
            .bind(host_id.to_string())
            .bind(target_id.to_string())
            .fetch_one(&mut **tx)
            .await?
    } else {
        let sql = format!(
            "{} SELECT EXISTS (SELECT 1 FROM host_target_grants tg JOIN readable_targets rt ON rt.id=tg.target_id WHERE tg.host_id=? AND tg.revoked_at IS NULL UNION SELECT 1 FROM workload_placements p JOIN readable_targets rt ON rt.id=p.target_id WHERE p.host_id=?)",
            readable_targets_cte()
        );
        sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql))
            .bind(i64::from(snapshot.system_admin))
            .bind(&snapshot.user_id)
            .bind(&snapshot.user_id)
            .bind(&snapshot.user_id)
            .bind(host_id.to_string())
            .bind(host_id.to_string())
            .fetch_one(&mut **tx)
            .await?
    };
    if readable != 0 {
        Ok(HostVisibility::TargetOnly)
    } else {
        Err(QueryError::NotFound)
    }
}

async fn ensure_physical_host(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    host_id: HostId,
) -> Result<(), QueryError> {
    let exists: Option<String> = sqlx::query_scalar("SELECT h.id FROM fleet_hosts h WHERE h.id=? AND ( ? = 1 OR EXISTS (SELECT 1 FROM host_user_grants ug WHERE ug.host_id=h.id AND ug.user_id=? AND ug.permission IN ('read','operator','admin') AND ug.revoked_at IS NULL))").bind(host_id.to_string()).bind(i64::from(snapshot.system_admin)).bind(&snapshot.user_id).fetch_optional(&mut **tx).await?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(QueryError::NotFound)
    }
}

async fn ensure_domain(
    tx: &mut Transaction<'_, Sqlite>,
    host_id: HostId,
    domain_id: ResourceDomainId,
) -> Result<(), QueryError> {
    let exists: Option<String> =
        sqlx::query_scalar("SELECT id FROM host_resource_domains WHERE id=? AND host_id=?")
            .bind(domain_id.to_string())
            .bind(host_id.to_string())
            .fetch_optional(&mut **tx)
            .await?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(QueryError::NotFound)
    }
}

#[allow(clippy::too_many_arguments)]
async fn visible_backend_ids(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    _targets: &ReadableTargets,
    host_id: HostId,
    visibility: HostVisibility,
    target_filter: Option<CiTargetId>,
    after: Option<Uuid>,
    limit: usize,
) -> Result<Vec<String>, QueryError> {
    let full = visibility == HostVisibility::Physical && target_filter.is_none();
    let prefix = if full {
        ""
    } else if target_filter.is_none() {
        readable_targets_cte()
    } else {
        ""
    };
    let selector = profile_selector_predicate("p", "b");
    let target_clause = if full {
        "1=1".to_owned()
    } else if target_filter.is_some() {
        format!(
            "(EXISTS (SELECT 1 FROM workload_placements wp WHERE wp.backend_id=b.id AND wp.host_id=b.host_id AND wp.target_id=?) OR EXISTS (SELECT 1 FROM pool_execution_profiles p JOIN profile_ci_targets pct ON pct.profile_id=p.id JOIN host_target_grants tg ON tg.target_id=pct.target_id WHERE tg.host_id=b.host_id AND tg.target_id=? AND tg.revoked_at IS NULL AND pct.target_id=? AND pct.enabled=1 AND pct.revoked_at IS NULL AND p.enabled=1 AND p.runtime_kind=b.runtime_kind AND p.execution_os=b.execution_os AND p.architecture=b.architecture AND {selector}))",
        )
    } else {
        format!(
            "(EXISTS (SELECT 1 FROM workload_placements wp JOIN readable_targets rt ON rt.id=wp.target_id WHERE wp.backend_id=b.id AND wp.host_id=b.host_id) OR EXISTS (SELECT 1 FROM pool_execution_profiles p JOIN profile_ci_targets pct ON pct.profile_id=p.id JOIN readable_targets rt ON rt.id=pct.target_id JOIN host_target_grants tg ON tg.target_id=pct.target_id WHERE tg.host_id=b.host_id AND tg.revoked_at IS NULL AND pct.enabled=1 AND pct.revoked_at IS NULL AND p.enabled=1 AND p.runtime_kind=b.runtime_kind AND p.execution_os=b.execution_os AND p.architecture=b.architecture AND {selector}))",
        )
    };
    let sql = format!(
        "{prefix} SELECT b.id FROM host_backends b WHERE b.host_id=? AND {target_clause}{} ORDER BY b.id ASC LIMIT ?",
        if after.is_some() { " AND b.id > ?" } else { "" },
        prefix = prefix,
        target_clause = target_clause,
    );
    let mut query = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql));
    if !prefix.is_empty() {
        query = query
            .bind(i64::from(snapshot.system_admin))
            .bind(&snapshot.user_id)
            .bind(&snapshot.user_id)
            .bind(&snapshot.user_id);
    }
    query = query.bind(host_id.to_string());
    if let Some(target_id) = target_filter {
        query = query
            .bind(target_id.to_string())
            .bind(target_id.to_string())
            .bind(target_id.to_string());
    }
    if let Some(after) = after {
        query = query.bind(after.to_string());
    }
    Ok(query
        .bind(i64::try_from(limit + 1).map_err(|_| QueryError::InvalidRequest("limit overflowed"))?)
        .fetch_all(&mut **tx)
        .await?)
}

fn profile_selector_predicate(profile: &str, host: &str) -> String {
    format!(
        "((EXISTS (SELECT 1 FROM profile_host_selectors any_selector WHERE any_selector.profile_id={profile}.id) OR {profile}.all_authorized_hosts=1) AND (NOT EXISTS (SELECT 1 FROM profile_host_selectors hs WHERE hs.profile_id={profile}.id AND hs.kind='host') OR EXISTS (SELECT 1 FROM profile_host_selectors hs WHERE hs.profile_id={profile}.id AND hs.kind='host' AND hs.host_id={host}.host_id)) AND NOT EXISTS (SELECT 1 FROM profile_host_selectors ts WHERE ts.profile_id={profile}.id AND ts.kind='tag' AND NOT EXISTS (SELECT 1 FROM json_each((SELECT tags_json FROM fleet_hosts WHERE id={host}.host_id)) WHERE json_each.key=ts.tag_key AND json_each.value=ts.tag_value)))"
    )
}

async fn backend_visible_for_target(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    _targets: &ReadableTargets,
    host_id: HostId,
    backend_id: BackendId,
    target_filter: Option<CiTargetId>,
) -> Result<bool, QueryError> {
    let selector = profile_selector_predicate("p", "b");
    let (prefix, target_clause) = if target_filter.is_some() {
        (
            "",
            format!(
                "(EXISTS (SELECT 1 FROM workload_placements wp WHERE wp.backend_id=b.id AND wp.host_id=b.host_id AND wp.target_id=?) OR EXISTS (SELECT 1 FROM pool_execution_profiles p JOIN profile_ci_targets pct ON pct.profile_id=p.id JOIN host_target_grants tg ON tg.target_id=pct.target_id WHERE tg.host_id=b.host_id AND tg.target_id=? AND tg.revoked_at IS NULL AND pct.target_id=? AND pct.enabled=1 AND pct.revoked_at IS NULL AND p.enabled=1 AND p.runtime_kind=b.runtime_kind AND p.execution_os=b.execution_os AND p.architecture=b.architecture AND {selector}))",
            ),
        )
    } else {
        (
            readable_targets_cte(),
            format!(
                "(EXISTS (SELECT 1 FROM workload_placements wp JOIN readable_targets rt ON rt.id=wp.target_id WHERE wp.backend_id=b.id AND wp.host_id=b.host_id) OR EXISTS (SELECT 1 FROM pool_execution_profiles p JOIN profile_ci_targets pct ON pct.profile_id=p.id JOIN readable_targets rt ON rt.id=pct.target_id JOIN host_target_grants tg ON tg.target_id=pct.target_id WHERE tg.host_id=b.host_id AND tg.revoked_at IS NULL AND pct.enabled=1 AND pct.revoked_at IS NULL AND p.enabled=1 AND p.runtime_kind=b.runtime_kind AND p.execution_os=b.execution_os AND p.architecture=b.architecture AND {selector}))",
            ),
        )
    };
    let sql = format!(
        "{prefix} SELECT EXISTS (SELECT 1 FROM host_backends b WHERE b.id=? AND b.host_id=? AND {target_clause})"
    );
    let mut query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql));
    if !prefix.is_empty() {
        query = query
            .bind(i64::from(snapshot.system_admin))
            .bind(&snapshot.user_id)
            .bind(&snapshot.user_id)
            .bind(&snapshot.user_id);
    }
    query = query.bind(backend_id.to_string()).bind(host_id.to_string());
    if let Some(target_id) = target_filter {
        query = query
            .bind(target_id.to_string())
            .bind(target_id.to_string())
            .bind(target_id.to_string());
    }
    Ok(query.fetch_one(&mut **tx).await? > 0)
}

async fn current_backend_observation(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    host_id: HostId,
    backend_id: BackendId,
) -> Result<bool, QueryError> {
    let current: i64 = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM host_backends b
          JOIN fleet_observed_backends ob ON ob.host_id=b.host_id AND ob.backend_id=b.id
          JOIN fleet_observation_sessions s ON s.host_id=ob.host_id AND s.session_id=ob.session_id
          JOIN fleet_hosts h ON h.id=ob.host_id
          JOIN host_credentials c ON c.host_id=s.host_id
            AND c.id=s.current_credential_id
            AND c.generation=s.current_credential_generation
            AND c.host_epoch=s.host_epoch
            AND c.revoked_at IS NULL
            AND (c.overlap_expires_at IS NULL OR c.overlap_expires_at>?)
          JOIN fleet_inventory_snapshots snap ON snap.host_id=s.host_id
            AND snap.authority_session_id=s.session_id
            AND snap.inventory_sequence=s.current_inventory_sequence
            AND snap.host_epoch=s.host_epoch
            AND snap.control_plane_incarnation=s.control_plane_incarnation
            AND snap.digest=s.current_inventory_digest
            AND snap.received_at=s.current_inventory_received_at
            AND snap.inventory_revision=h.inventory_revision
            AND snap.receipt_state!='conflicted'
         WHERE b.host_id=? AND b.id=?
           AND b.probe_epoch=ob.host_epoch
           AND b.probe_session_id=ob.session_id
           AND b.probe_boot_id=s.boot_id
           AND b.probe_incarnation=ob.control_plane_incarnation
           AND ob.host_epoch=s.host_epoch
           AND ob.control_plane_incarnation=s.control_plane_incarnation
           AND s.ended_at IS NULL AND s.state!='conflicted'
           AND s.lease_expires_at>? AND s.host_epoch=h.epoch AND s.boot_id=h.boot_id
           AND h.agent_session_id=s.session_id AND h.inventory_session_id=s.session_id
           AND h.inventory_boot_id=s.boot_id AND h.boot_id=s.boot_id
           AND s.control_plane_incarnation=h.authority_incarnation
           AND s.control_plane_incarnation=?
           AND s.current_inventory_sequence=ob.inventory_sequence
           AND s.current_inventory_received_at=ob.received_at
           AND h.inventory_complete=1 AND h.inventory_epoch=s.host_epoch
           AND h.inventory_revision>0
           AND h.last_inventory_at=ob.received_at
           AND h.inventory_digest=s.current_inventory_digest
           AND ob.received_at<=?
           AND b.last_probed_at=ob.received_at)",
    )
    .bind(snapshot.now)
    .bind(host_id.to_string())
    .bind(backend_id.to_string())
    .bind(snapshot.now)
    .bind(snapshot.incarnation.to_string())
    .bind(snapshot.now)
    .fetch_one(&mut **tx)
    .await?;
    Ok(current > 0)
}

async fn domain_ids(
    tx: &mut Transaction<'_, Sqlite>,
    host_id: HostId,
    after: Option<Uuid>,
    limit: usize,
) -> Result<Vec<String>, QueryError> {
    let mut sql = "SELECT id FROM host_resource_domains WHERE host_id=?".to_owned();
    if after.is_some() {
        sql.push_str(" AND id > ?");
    }
    sql.push_str(" ORDER BY id ASC LIMIT ?");
    let mut query =
        sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql)).bind(host_id.to_string());
    if let Some(after) = after {
        query = query.bind(after.to_string());
    }
    Ok(query
        .bind(i64::try_from(limit + 1).map_err(|_| QueryError::InvalidRequest("limit overflowed"))?)
        .fetch_all(&mut **tx)
        .await?)
}

fn amount(cpu: i64, memory: i64, disk: i64) -> Result<ResourceAmountDto, QueryError> {
    let cpu = BrowserUint53::new(u64::try_from(cpu).map_err(|_| QueryError::MalformedData)?)?;
    let memory = BrowserUint53::new(u64::try_from(memory).map_err(|_| QueryError::MalformedData)?)?;
    let disk = BrowserUint53::new(u64::try_from(disk).map_err(|_| QueryError::MalformedData)?)?;
    Ok(ResourceAmountDto {
        cpu_millis: cpu,
        memory_mib: memory,
        disk_bytes: disk,
    })
}

async fn allocation_read(
    tx: &mut Transaction<'_, Sqlite>,
    host_id: HostId,
    backend_id: Option<BackendId>,
    domain_id: Option<ResourceDomainId>,
) -> Result<AllocationReadV1, QueryError> {
    let mut sql = "SELECT COALESCE(SUM(a.cpu_millis),0) AS cpu,COALESCE(SUM(a.memory_mib),0) AS memory,COALESCE(SUM(a.disk_bytes),0) AS disk,COALESCE(SUM(CASE WHEN a.state='uncertain' THEN a.cpu_millis ELSE 0 END),0) AS uncertain_cpu,COALESCE(SUM(CASE WHEN a.state='uncertain' THEN a.memory_mib ELSE 0 END),0) AS uncertain_memory,COALESCE(SUM(CASE WHEN a.state='uncertain' THEN a.disk_bytes ELSE 0 END),0) AS uncertain_disk FROM capacity_allocations a JOIN host_resource_domains d ON d.id=a.domain_id AND d.host_id=a.host_id WHERE a.host_id=? AND a.state!='released'".to_owned();
    if backend_id.is_some() {
        sql.push_str(" AND a.backend_id=?");
    }
    if domain_id.is_some() {
        sql.push_str(" AND a.domain_id=?");
    }
    if backend_id.is_some() && domain_id.is_none() {
        sql.push_str(" AND d.parent_domain_id IS NULL");
    }
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(host_id.to_string());
    if let Some(backend_id) = backend_id {
        query = query.bind(backend_id.to_string());
    }
    if let Some(domain_id) = domain_id {
        query = query.bind(domain_id.to_string());
    }
    let row = query.fetch_one(&mut **tx).await?;
    Ok(AllocationReadV1 {
        charged: amount(row.get("cpu"), row.get("memory"), row.get("disk"))?,
        uncertain: amount(
            row.get("uncertain_cpu"),
            row.get("uncertain_memory"),
            row.get("uncertain_disk"),
        )?,
    })
}

async fn counts(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    host_id: HostId,
    backend_id: Option<BackendId>,
    target_filter: Option<&str>,
    scope: CountScope,
) -> Result<FleetRecordCounts, QueryError> {
    let prefix = if target_filter.is_none() && scope == CountScope::VisibleTargets {
        readable_targets_cte()
    } else {
        ""
    };
    let target_clause = if target_filter.is_some() {
        " AND p.target_id=?"
    } else if scope == CountScope::VisibleTargets {
        " AND EXISTS (SELECT 1 FROM readable_targets rt WHERE rt.id=p.target_id)"
    } else {
        ""
    };
    let sql = format!(
        "{prefix} SELECT COUNT(DISTINCT CASE WHEN i.kind='ci_runner' AND p.runner_id IS NOT NULL AND r.deleted_at IS NULL THEN p.runner_id END) AS runners,COUNT(DISTINCT CASE WHEN i.kind='ci_runner' AND EXISTS (SELECT 1 FROM capacity_allocations ca WHERE ca.placement_id=p.id AND ca.state!='released') THEN p.id END) AS charged_ci,COUNT(DISTINCT CASE WHEN i.kind='fix_sandbox' AND EXISTS (SELECT 1 FROM capacity_allocations ca WHERE ca.placement_id=p.id AND ca.state!='released') THEN p.id END) AS charged_fix,COUNT(DISTINCT CASE WHEN i.kind='ci_runner' AND p.state='running' THEN p.id END) AS running_ci,COUNT(DISTINCT CASE WHEN i.kind='fix_sandbox' AND p.state='running' THEN p.id END) AS running_fix,COUNT(DISTINCT CASE WHEN p.state='uncertain' OR EXISTS (SELECT 1 FROM capacity_allocations ca WHERE ca.placement_id=p.id AND ca.state='uncertain' AND ca.state!='released') THEN p.id END) AS uncertain FROM workload_placements p JOIN workload_intents i ON i.id=p.intent_id LEFT JOIN runners r ON r.id=p.runner_id WHERE p.host_id=?{backend_clause}{target_clause}",
        prefix = prefix,
        backend_clause = if backend_id.is_some() {
            " AND p.backend_id=?"
        } else {
            ""
        },
        target_clause = target_clause,
    );
    if backend_id.is_some() {
        debug_assert!(sql.contains("p.backend_id=?"));
    }
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
    if !prefix.is_empty() {
        query = query
            .bind(i64::from(snapshot.system_admin))
            .bind(&snapshot.user_id)
            .bind(&snapshot.user_id)
            .bind(&snapshot.user_id);
    }
    query = query.bind(host_id.to_string());
    if let Some(backend_id) = backend_id {
        query = query.bind(backend_id.to_string());
    }
    if let Some(target_id) = target_filter {
        query = query.bind(target_id);
    }
    let row = query.fetch_one(&mut **tx).await?;
    let counter = |name: &str| -> Result<BrowserUint53, QueryError> {
        BrowserUint53::new(
            u64::try_from(row.get::<i64, _>(name)).map_err(|_| QueryError::MalformedData)?,
        )
        .map_err(Into::into)
    };
    Ok(FleetRecordCounts {
        scope,
        known_managed_runner_count: counter("runners")?,
        charged_ci_placement_count: counter("charged_ci")?,
        charged_fix_placement_count: counter("charged_fix")?,
        running_ci_placement_count: counter("running_ci")?,
        running_fix_placement_count: counter("running_fix")?,
        uncertain_placement_count: counter("uncertain")?,
    })
}

async fn root_budget(
    tx: &mut Transaction<'_, Sqlite>,
    host_id: HostId,
) -> Result<ResourceAmountDto, QueryError> {
    let row = sqlx::query("SELECT cpu_millis,memory_mib,disk_bytes FROM host_resource_domains WHERE host_id=? AND parent_domain_id IS NULL").bind(host_id.to_string()).fetch_optional(&mut **tx).await?.ok_or(QueryError::MalformedData)?;
    amount(
        row.get("cpu_millis"),
        row.get("memory_mib"),
        row.get("disk_bytes"),
    )
}

async fn latest_measurement(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    host_id: HostId,
    domain_id: ResourceDomainId,
) -> Result<Option<MeasurementReadV1>, QueryError> {
    let row = sqlx::query("SELECT id,host_id,domain_id,host_epoch,observed_at,received_at,cpu_used_millis,memory_used_mib,disk_free_bytes,uptime_seconds,coverage,authority_session_id,boot_id,control_plane_incarnation FROM host_samples WHERE host_id=? AND domain_id=? ORDER BY id DESC LIMIT 1").bind(host_id.to_string()).bind(domain_id.to_string()).fetch_optional(&mut **tx).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    measurement_from_row(tx, snapshot, &row).await.map(Some)
}

#[allow(clippy::too_many_arguments)]
async fn current_observation_mapping(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    host_id: &str,
    session_id: &str,
    host_epoch: i64,
    incarnation: &str,
    sequence: i64,
    received_at: i64,
) -> Result<bool, QueryError> {
    let current: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM fleet_observation_sessions s
          JOIN fleet_hosts h ON h.id=s.host_id
          JOIN host_credentials c ON c.host_id=s.host_id
            AND c.id=s.current_credential_id
            AND c.generation=s.current_credential_generation
            AND c.host_epoch=s.host_epoch
            AND c.revoked_at IS NULL
            AND (c.overlap_expires_at IS NULL OR c.overlap_expires_at>?)
          JOIN fleet_inventory_snapshots snap ON snap.host_id=s.host_id
            AND snap.authority_session_id=s.session_id
            AND snap.inventory_sequence=s.current_inventory_sequence
            AND snap.host_epoch=s.host_epoch
            AND snap.control_plane_incarnation=s.control_plane_incarnation
            AND snap.digest=s.current_inventory_digest
            AND snap.received_at=s.current_inventory_received_at
            AND snap.inventory_revision=h.inventory_revision
            AND snap.receipt_state!='conflicted'
        WHERE s.host_id=? AND s.session_id=? AND s.host_epoch=?
          AND s.control_plane_incarnation=? AND s.ended_at IS NULL
          AND s.state!='conflicted' AND s.lease_expires_at>?
          AND s.host_epoch=h.epoch AND s.boot_id=h.boot_id
          AND h.agent_session_id=s.session_id AND h.inventory_session_id=s.session_id
          AND h.inventory_boot_id=s.boot_id AND h.boot_id=s.boot_id
          AND h.inventory_epoch=s.host_epoch
          AND h.authority_incarnation=s.control_plane_incarnation
          AND s.control_plane_incarnation=?
          AND s.current_inventory_sequence=?
          AND s.current_inventory_received_at=?
          AND h.last_inventory_at=s.current_inventory_received_at
          AND h.inventory_complete=1 AND h.inventory_revision>0
          AND h.inventory_digest=s.current_inventory_digest
          AND ?-? BETWEEN 0 AND ?-1",
    )
    .bind(snapshot.now)
    .bind(host_id)
    .bind(session_id)
    .bind(host_epoch)
    .bind(incarnation)
    .bind(snapshot.now)
    .bind(snapshot.incarnation.to_string())
    .bind(sequence)
    .bind(received_at)
    .bind(snapshot.now)
    .bind(received_at)
    .bind(FRESH_INVENTORY_MILLIS)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(current.is_some())
}

async fn measurement_from_row(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    row: &sqlx::sqlite::SqliteRow,
) -> Result<MeasurementReadV1, QueryError> {
    let observed_at = wire_timestamp(row.get("observed_at"))?;
    let received_at_value: i64 = row.get("received_at");
    let received_at = wire_timestamp(received_at_value)?;
    let coverage = coverage(row.get::<String, _>("coverage").as_str())?;
    let provenance = sample_provenance(tx, snapshot, row, received_at_value).await?;
    Ok(MeasurementReadV1 {
        observed_at,
        received_at,
        coverage,
        provenance,
        cpu_used_millis: optional_browser(row.get("cpu_used_millis"))?,
        memory_used_mib: optional_browser(row.get("memory_used_mib"))?,
        disk_free_bytes: optional_browser(row.get("disk_free_bytes"))?,
        uptime_seconds: optional_browser(row.get("uptime_seconds"))?,
    })
}

fn optional_browser(value: Option<i64>) -> Result<Option<BrowserUint53>, QueryError> {
    value
        .map(|value| {
            BrowserUint53::new(u64::try_from(value).map_err(|_| ContractError::IntegerRange)?)
        })
        .transpose()
        .map_err(Into::into)
}

fn coverage(value: &str) -> Result<CoverageV1, QueryError> {
    match value {
        "complete" => Ok(CoverageV1::Complete),
        "partial" => Ok(CoverageV1::Partial),
        "unknown" => Ok(CoverageV1::Unknown),
        _ => Err(QueryError::MalformedData),
    }
}

async fn sample_provenance(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    row: &sqlx::sqlite::SqliteRow,
    received_at: i64,
) -> Result<SampleProvenanceV1, QueryError> {
    let authority: Option<String> = row.get("authority_session_id");
    let Some(authority) = authority else {
        return Ok(SampleProvenanceV1::LegacyUnknown);
    };
    let host_id: String = row.get("host_id");
    let current = sample_is_current(
        tx,
        snapshot,
        &host_id,
        row.get("host_epoch"),
        &authority,
        row.get::<Option<String>, _>("boot_id").as_deref(),
        row.get::<Option<String>, _>("control_plane_incarnation")
            .as_deref(),
        received_at,
    )
    .await?;
    Ok(if current {
        SampleProvenanceV1::Current
    } else {
        SampleProvenanceV1::Historical
    })
}

#[allow(clippy::too_many_arguments)]
async fn sample_is_current(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    host_id: &str,
    host_epoch: i64,
    authority_session_id: &str,
    boot_id: Option<&str>,
    incarnation: Option<&str>,
    received_at: i64,
) -> Result<bool, QueryError> {
    let host = sqlx::query(
        "SELECT epoch,agent_session_id,boot_id,authority_incarnation FROM fleet_hosts WHERE id=?",
    )
    .bind(host_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(QueryError::MalformedData)?;
    let snapshot_incarnation = snapshot.incarnation.to_string();
    let current: Option<i64> = sqlx::query_scalar("SELECT 1 FROM fleet_observation_sessions s JOIN host_credentials c ON c.host_id=s.host_id AND c.id=s.current_credential_id AND c.generation=s.current_credential_generation AND c.host_epoch=s.host_epoch AND c.revoked_at IS NULL AND (c.overlap_expires_at IS NULL OR c.overlap_expires_at>?) JOIN fleet_hosts h ON h.id=s.host_id WHERE s.host_id=? AND s.session_id=? AND s.ended_at IS NULL AND s.state!='conflicted' AND s.lease_expires_at>? AND s.host_epoch=h.epoch AND s.boot_id=h.boot_id AND h.agent_session_id=s.session_id AND h.boot_id=s.boot_id AND s.control_plane_incarnation=h.authority_incarnation AND s.control_plane_incarnation=? AND ?-? BETWEEN 0 AND ?-1")
        .bind(snapshot.now)
        .bind(host_id)
        .bind(authority_session_id)
        .bind(snapshot.now)
        .bind(&snapshot_incarnation)
        .bind(snapshot.now)
        .bind(received_at)
        .bind(FRESH_INVENTORY_MILLIS)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(
        host.get::<Option<String>, _>("agent_session_id").as_deref() == Some(authority_session_id)
            && host.get::<i64, _>("epoch") == host_epoch
            && host.get::<Option<String>, _>("boot_id").as_deref() == boot_id
            && host
                .get::<Option<String>, _>("authority_incarnation")
                .as_deref()
                == incarnation
            && incarnation == Some(snapshot_incarnation.as_str())
            && current.is_some(),
    )
}

#[derive(Debug)]
struct SampleRow {
    id: i64,
    host_id: String,
    domain_id: Option<String>,
    host_epoch: i64,
    observed_at: i64,
    received_at: i64,
    cpu: Option<i64>,
    memory: Option<i64>,
    disk: Option<i64>,
    uptime: Option<i64>,
    coverage: String,
    authority_session_id: Option<String>,
    boot_id: Option<String>,
    incarnation: Option<String>,
}

async fn sample_rows(
    tx: &mut Transaction<'_, Sqlite>,
    host_id: HostId,
    request: &SampleQuery,
    after: Option<i64>,
    limit: usize,
) -> Result<Vec<SampleRow>, QueryError> {
    let mut sql = "SELECT id,host_id,domain_id,host_epoch,observed_at,received_at,cpu_used_millis,memory_used_mib,disk_free_bytes,uptime_seconds,coverage,authority_session_id,boot_id,control_plane_incarnation FROM host_samples WHERE host_id=? AND received_at>=? AND received_at<?".to_owned();
    if request.domain_id.is_some() {
        sql.push_str(" AND domain_id=?");
    }
    if after.is_some() {
        sql.push_str(" AND id>?");
    }
    sql.push_str(" ORDER BY id ASC LIMIT ?");
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(host_id.to_string())
        .bind(request.received_from.millis())
        .bind(request.received_to.millis());
    if let Some(domain_id) = request.domain_id {
        query = query.bind(domain_id.to_string());
    }
    if let Some(after) = after {
        query = query.bind(after);
    }
    let rows = query
        .bind(i64::try_from(limit + 1).map_err(|_| QueryError::InvalidRequest("limit overflowed"))?)
        .fetch_all(&mut **tx)
        .await?;
    rows.into_iter()
        .map(|row| {
            Ok(SampleRow {
                id: row.get("id"),
                host_id: row.get("host_id"),
                domain_id: row.get("domain_id"),
                host_epoch: row.get("host_epoch"),
                observed_at: row.get("observed_at"),
                received_at: row.get("received_at"),
                cpu: row.get("cpu_used_millis"),
                memory: row.get("memory_used_mib"),
                disk: row.get("disk_free_bytes"),
                uptime: row.get("uptime_seconds"),
                coverage: row.get("coverage"),
                authority_session_id: row.get("authority_session_id"),
                boot_id: row.get("boot_id"),
                incarnation: row.get("control_plane_incarnation"),
            })
        })
        .collect()
}

async fn sample_projection(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    row: &SampleRow,
) -> Result<SampleReadV1, QueryError> {
    let provenance = sample_provenance_values(tx, snapshot, row).await?;
    Ok(SampleReadV1 {
        domain_id: row.domain_id.as_deref().map(parse_domain_id).transpose()?,
        observed_at: wire_timestamp(row.observed_at)?,
        received_at: wire_timestamp(row.received_at)?,
        coverage: coverage(&row.coverage)?,
        provenance,
        cpu_used_millis: optional_browser(row.cpu)?,
        memory_used_mib: optional_browser(row.memory)?,
        disk_free_bytes: optional_browser(row.disk)?,
        uptime_seconds: optional_browser(row.uptime)?,
    })
}

async fn retention_floor(
    tx: &mut Transaction<'_, Sqlite>,
    stream: &str,
) -> Result<RetainedFrom, QueryError> {
    let floor: Option<i64> = sqlx::query_scalar(
        "SELECT first_retained FROM fleet_query_retention_floors WHERE stream=?",
    )
    .bind(stream)
    .fetch_optional(&mut **tx)
    .await?;
    let floor = floor.ok_or(QueryError::DependencyUnavailable)?;
    let floor = u64::try_from(floor).map_err(|_| QueryError::MalformedData)?;
    Ok(RetainedFrom::Row(
        crate::fleet::protocol::primitives::ByteOffset::new(floor)
            .map_err(|_| QueryError::MalformedData)?,
    ))
}

#[derive(Debug)]
struct EventRow {
    cursor: i64,
    id: String,
    host_id: String,
    placement_id: Option<String>,
    target_id: Option<String>,
    kind: String,
    occurred_at: i64,
    placement_host_id: Option<String>,
    placement_target_id: Option<String>,
}

async fn event_rows(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    _targets: &ReadableTargets,
    host_id: HostId,
    visibility: HostVisibility,
    request: &EventQuery,
    after: Option<i64>,
) -> Result<Vec<EventRow>, QueryError> {
    let sql = format!(
        "{} SELECT e.cursor,e.id,e.host_id,e.placement_id,e.target_id,e.kind,e.created_at,p.host_id AS placement_host_id,p.target_id AS placement_target_id FROM fleet_events e LEFT JOIN workload_placements p ON p.id=e.placement_id WHERE e.host_id=? AND ((e.target_id IS NULL AND ?=1) OR (e.target_id IS NOT NULL AND EXISTS (SELECT 1 FROM readable_targets rt WHERE rt.id=e.target_id))){}{}{} ORDER BY e.cursor ASC LIMIT ?",
        readable_targets_cte(),
        if request.target_id.is_some() {
            " AND e.target_id=?"
        } else {
            ""
        },
        if request.occurred_from.is_some() {
            " AND e.created_at>=? AND e.created_at<?"
        } else {
            ""
        },
        if after.is_some() {
            " AND e.cursor>?"
        } else {
            ""
        },
    );
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(i64::from(snapshot.system_admin))
        .bind(&snapshot.user_id)
        .bind(&snapshot.user_id)
        .bind(&snapshot.user_id)
        .bind(host_id.to_string())
        .bind(i64::from(visibility == HostVisibility::Physical));
    if let Some(target) = request.target_id {
        query = query.bind(target.to_string());
    }
    if let (Some(from), Some(to)) = (request.occurred_from, request.occurred_to) {
        query = query.bind(from.millis()).bind(to.millis());
    }
    if let Some(after) = after {
        query = query.bind(after);
    }
    let rows = query
        .bind(
            i64::try_from(request.limit + 1)
                .map_err(|_| QueryError::InvalidRequest("limit overflowed"))?,
        )
        .fetch_all(&mut **tx)
        .await?;
    rows.into_iter()
        .map(|row| {
            Ok(EventRow {
                cursor: row.get("cursor"),
                id: row.get("id"),
                host_id: row.get("host_id"),
                placement_id: row.get("placement_id"),
                target_id: row.get("target_id"),
                kind: row.get("kind"),
                occurred_at: row.get("created_at"),
                placement_host_id: row.get("placement_host_id"),
                placement_target_id: row.get("placement_target_id"),
            })
        })
        .collect()
}

fn event_projection(
    service: &RegistryService,
    _tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    scope: &CursorScope,
    row: &EventRow,
) -> Result<HostEventReadV1, QueryError> {
    if row.placement_id.is_some()
        && (row.placement_host_id.as_deref() != Some(row.host_id.as_str())
            || row.placement_target_id.as_deref() != row.target_id.as_deref())
    {
        return Err(QueryError::MalformedData);
    }
    let id = parse_event_id(&row.id)?;
    let host_id = parse_host_id(&row.host_id)?;
    let target_id = row.target_id.as_deref().map(parse_target_id).transpose()?;
    let placement_id = row
        .placement_id
        .as_deref()
        .map(|value| value.parse().map_err(|_| QueryError::MalformedData))
        .transpose()?;
    let cursor = service.issue_row_cursor(
        scope,
        snapshot.incarnation,
        CursorStream::HostEvents,
        row.cursor,
        snapshot.now,
    )?;
    Ok(HostEventReadV1 {
        id,
        host_id,
        target_id,
        placement_id,
        kind: event_kind(&row.kind),
        occurred_at: wire_timestamp(row.occurred_at)?,
        cursor,
    })
}

fn event_kind(value: &str) -> HostEventKindV1 {
    match value {
        "fleet.enrollment.issued" => HostEventKindV1::EnrollmentIssued,
        "fleet.enrollment.consumed" => HostEventKindV1::EnrollmentConsumed,
        "fleet.enrollment.revoked" => HostEventKindV1::EnrollmentRevoked,
        "fleet.host.approved" => HostEventKindV1::HostApproved,
        "fleet.host.recovered" => HostEventKindV1::HostRecovered,
        "fleet.credential.rotation_exchanged" | "fleet.credential.rotation_acknowledged" => {
            HostEventKindV1::HostCredentialRotated
        }
        "fleet.credential.revoked" | "fleet.credential.rotation_revoked" => {
            HostEventKindV1::HostCredentialRevoked
        }
        "fleet.inventory.received" => HostEventKindV1::InventoryReceived,
        "fleet.inventory.conflict" => HostEventKindV1::InventoryConflict,
        value if value.starts_with("fleet.backend.") => HostEventKindV1::BackendChanged,
        value if value.starts_with("fleet.placement.queued") => HostEventKindV1::PlacementQueued,
        value if value.starts_with("fleet.placement.admitted") => {
            HostEventKindV1::PlacementAdmitted
        }
        value if value.starts_with("fleet.placement.") => HostEventKindV1::PlacementChanged,
        value if value.starts_with("fleet.command.") => HostEventKindV1::CommandChanged,
        _ => HostEventKindV1::Other,
    }
}

async fn inventory_evidence(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    row: &sqlx::sqlite::SqliteRow,
) -> Result<InventoryEvidenceV1, QueryError> {
    let revision: i64 = row.get("inventory_revision");
    let digest: Option<String> = row.get("inventory_digest");
    let received_at: Option<i64> = row.get("last_inventory_at");
    let digest = digest
        .map(|value| Sha256Digest::try_from(value).map_err(|_| QueryError::MalformedData))
        .transpose()?;
    let revision_wire = if revision > 0 {
        Some(BrowserCounter::new(
            u64::try_from(revision).map_err(|_| QueryError::MalformedData)?,
        )?)
    } else {
        None
    };
    let current = row.get::<i64, _>("inventory_complete") == 1
        && revision_wire.is_some()
        && digest.is_some()
        && received_at.is_some_and(|value| {
            value <= snapshot.now && snapshot.now.saturating_sub(value) < FRESH_INVENTORY_MILLIS
        })
        && current_inventory_receipt(tx, snapshot, row, revision, digest.as_ref(), received_at)
            .await?;
    if current {
        Ok(InventoryEvidenceV1::Current {
            inventory_revision: revision_wire.ok_or(QueryError::MalformedData)?,
            digest: digest.ok_or(QueryError::MalformedData)?,
            received_at: wire_timestamp(received_at.ok_or(QueryError::MalformedData)?)?,
        })
    } else if revision_wire.is_some() || digest.is_some() || received_at.is_some() {
        Ok(InventoryEvidenceV1::Stale {
            inventory_revision: revision_wire,
            digest,
            received_at: received_at.map(wire_timestamp).transpose()?,
        })
    } else {
        Ok(InventoryEvidenceV1::Missing)
    }
}

async fn current_inventory_receipt(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    row: &sqlx::sqlite::SqliteRow,
    revision: i64,
    digest: Option<&Sha256Digest>,
    received_at: Option<i64>,
) -> Result<bool, QueryError> {
    let Some(session_id) = row.get::<Option<String>, _>("inventory_session_id") else {
        return Ok(false);
    };
    let digest: String = digest
        .map(|value| value.to_owned().into())
        .ok_or(QueryError::MalformedData)?;
    let Some(received_at) = received_at else {
        return Ok(false);
    };
    let host_id: String = row.get("id");
    let result: Option<i64> = sqlx::query_scalar("SELECT 1 FROM fleet_observation_sessions s JOIN host_credentials c ON c.host_id=s.host_id AND c.id=s.current_credential_id AND c.generation=s.current_credential_generation AND c.host_epoch=s.host_epoch AND c.revoked_at IS NULL AND (c.overlap_expires_at IS NULL OR c.overlap_expires_at>?) JOIN fleet_hosts h ON h.id=s.host_id WHERE s.host_id=? AND s.session_id=? AND s.ended_at IS NULL AND s.state!='conflicted' AND s.lease_expires_at>? AND s.host_epoch=h.epoch AND s.boot_id=h.boot_id AND s.boot_id=h.inventory_boot_id AND s.control_plane_incarnation=h.authority_incarnation AND s.control_plane_incarnation=? AND h.inventory_complete=1 AND h.inventory_epoch=h.epoch AND h.inventory_session_id=s.session_id AND h.agent_session_id=s.session_id AND h.boot_id=s.boot_id AND h.inventory_boot_id=s.boot_id AND h.last_inventory_at<=? AND ?-h.last_inventory_at<? AND s.current_inventory_sequence>0 AND s.current_inventory_digest=? AND s.current_inventory_received_at=? AND h.inventory_revision=? AND h.inventory_digest=? AND h.last_inventory_at=? AND EXISTS (SELECT 1 FROM fleet_inventory_snapshots snap WHERE snap.host_id=s.host_id AND snap.authority_session_id=s.session_id AND snap.inventory_sequence=s.current_inventory_sequence AND snap.host_epoch=s.host_epoch AND snap.control_plane_incarnation=s.control_plane_incarnation AND snap.inventory_revision=h.inventory_revision AND snap.digest=h.inventory_digest AND snap.received_at=h.last_inventory_at)").bind(snapshot.now).bind(host_id).bind(session_id).bind(snapshot.now).bind(snapshot.incarnation.to_string()).bind(snapshot.now).bind(snapshot.now).bind(FRESH_INVENTORY_MILLIS).bind(&digest).bind(received_at).bind(revision).bind(&digest).bind(received_at).fetch_optional(&mut **tx).await?;
    Ok(result.is_some())
}

async fn approval_evidence(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    host_id: HostId,
    inventory: InventoryEvidenceV1,
) -> Result<ApprovalEvidenceV1, QueryError> {
    if !snapshot.system_admin {
        return Ok(ApprovalEvidenceV1::Unavailable {
            reason: ApprovalUnavailableReasonV1::RequiresSystemAdmin,
        });
    }
    let row = sqlx::query("SELECT enrollment_state,lifecycle_state,revision,inventory_revision,inventory_digest,last_inventory_at,inventory_session_id FROM fleet_hosts WHERE id=?").bind(host_id.to_string()).fetch_optional(&mut **tx).await?.ok_or(QueryError::MalformedData)?;
    if row.get::<String, _>("lifecycle_state") == "retired" {
        return Ok(ApprovalEvidenceV1::Unavailable {
            reason: ApprovalUnavailableReasonV1::RetiredHost,
        });
    }
    if row.get::<String, _>("enrollment_state") != "pending" {
        return Ok(ApprovalEvidenceV1::Unavailable {
            reason: ApprovalUnavailableReasonV1::PendingHost,
        });
    }
    let InventoryEvidenceV1::Current {
        inventory_revision,
        digest,
        received_at,
    } = inventory
    else {
        return Ok(ApprovalEvidenceV1::Unavailable {
            reason: ApprovalUnavailableReasonV1::StaleInventory,
        });
    };
    let Some(session_id) = row.get::<Option<String>, _>("inventory_session_id") else {
        return Ok(ApprovalEvidenceV1::Unavailable {
            reason: ApprovalUnavailableReasonV1::MissingObservation,
        });
    };
    let lease: Option<i64> = sqlx::query_scalar("SELECT lease_expires_at FROM fleet_observation_sessions WHERE host_id=? AND session_id=? AND ended_at IS NULL AND state!='conflicted' AND lease_expires_at>?").bind(host_id.to_string()).bind(&session_id).bind(snapshot.now).fetch_optional(&mut **tx).await?;
    let Some(lease) = lease else {
        return Ok(ApprovalEvidenceV1::Unavailable {
            reason: ApprovalUnavailableReasonV1::ObservationExpired,
        });
    };
    Ok(ApprovalEvidenceV1::Available {
        expected_revision: BrowserCounter::new(
            u64::try_from(row.get::<i64, _>("revision")).map_err(|_| QueryError::MalformedData)?,
        )?,
        expected_inventory_revision: inventory_revision,
        expected_inventory_digest: digest,
        inventory_received_at: received_at,
        observation_lease_expires_at: wire_timestamp(lease)?,
    })
}

async fn sample_provenance_values(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &Snapshot,
    row: &SampleRow,
) -> Result<SampleProvenanceV1, QueryError> {
    let Some(authority) = row.authority_session_id.as_deref() else {
        return Ok(SampleProvenanceV1::LegacyUnknown);
    };
    let current = sample_is_current(
        tx,
        snapshot,
        &row.host_id,
        row.host_epoch,
        authority,
        row.boot_id.as_deref(),
        row.incarnation.as_deref(),
        row.received_at,
    )
    .await?;
    Ok(if current {
        SampleProvenanceV1::Current
    } else {
        SampleProvenanceV1::Historical
    })
}

impl RegistryService {
    async fn host_projection(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        snapshot: &Snapshot,
        _targets: &ReadableTargets,
        host_id: HostId,
        visibility: HostVisibility,
        target_filter: Option<&str>,
    ) -> Result<HostReadV1, QueryError> {
        let row = sqlx::query("SELECT id,name,host_os,architecture,enrollment_state,lifecycle_state,integrity_state,revision,agent_version,tags_json,last_heartbeat_at,last_inventory_at,pressure_state,inventory_complete,inventory_revision,inventory_digest,authority_incarnation,inventory_session_id,inventory_boot_id,inventory_epoch FROM fleet_hosts WHERE id=?").bind(host_id.to_string()).fetch_optional(&mut **tx).await?.ok_or(QueryError::NotFound)?;
        let counts = counts(
            tx,
            snapshot,
            host_id,
            None,
            target_filter,
            if visibility == HostVisibility::Physical && target_filter.is_none() {
                CountScope::Host
            } else {
                CountScope::VisibleTargets
            },
        )
        .await?;
        let id = parse_host_id(row.get::<String, _>("id").as_str())?;
        let name = checked_text(row.get("name"))?;
        let host_os = execution_os(row.get::<String, _>("host_os").as_str())?;
        let architecture = architecture(row.get::<String, _>("architecture").as_str())?;
        if visibility == HostVisibility::TargetOnly {
            return Ok(HostReadV1::Target {
                id,
                name,
                host_os,
                architecture,
                counts: counts.try_into()?,
            });
        }
        let enrollment = enrollment(row.get::<String, _>("enrollment_state").as_str())?;
        let lifecycle = lifecycle(row.get::<String, _>("lifecycle_state").as_str())?;
        let integrity = integrity(row.get::<String, _>("integrity_state").as_str())?;
        let heartbeat_at: Option<i64> = row.get("last_heartbeat_at");
        let inventory_at: Option<i64> = row.get("last_inventory_at");
        let heartbeat_freshness = freshness(heartbeat_at, snapshot.now);
        let pressure = pressure(row.get::<String, _>("pressure_state").as_str())?;
        let policy_revision = BrowserCounter::new(
            u64::try_from(row.get::<i64, _>("revision")).map_err(|_| QueryError::MalformedData)?,
        )?;
        let backend_count: BrowserUint53 = BrowserUint53::new(
            u64::try_from(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM host_backends WHERE host_id=?")
                    .bind(host_id.to_string())
                    .fetch_one(&mut **tx)
                    .await?,
            )
            .map_err(|_| QueryError::MalformedData)?,
        )?;
        let tags_value: BTreeMap<String, String> =
            serde_json::from_str(row.get::<String, _>("tags_json").as_str())
                .map_err(|_| QueryError::MalformedData)?;
        let tags = CheckedTags::new(tags_value).map_err(|_| QueryError::MalformedData)?;
        let root_id: String = sqlx::query_scalar(
            "SELECT id FROM host_resource_domains WHERE host_id=? AND parent_domain_id IS NULL",
        )
        .bind(host_id.to_string())
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(QueryError::MalformedData)?;
        let root_id = parse_domain_id(&root_id)?;
        let inventory_evidence = inventory_evidence(tx, snapshot, &row).await?;
        let approval_evidence =
            approval_evidence(tx, snapshot, host_id, inventory_evidence.clone()).await?;
        Ok(HostReadV1::Host {
            id,
            name,
            host_os,
            architecture,
            enrollment,
            lifecycle,
            integrity,
            heartbeat_freshness,
            pressure,
            policy_revision,
            agent_version: row
                .get::<Option<String>, _>("agent_version")
                .map(checked_text)
                .transpose()?,
            last_heartbeat_at: heartbeat_at.map(wire_timestamp).transpose()?,
            last_inventory_at: inventory_at.map(wire_timestamp).transpose()?,
            backend_count,
            counts,
            tags,
            physical_budget: root_budget(tx, host_id).await?,
            physical_allocation: allocation_read(tx, host_id, None, Some(root_id)).await?,
            latest_root_measurement: latest_measurement(tx, snapshot, host_id, root_id).await?,
            inventory_evidence,
            approval_evidence,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn backend_projection(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        snapshot: &Snapshot,
        targets: &ReadableTargets,
        expected_host_id: HostId,
        backend_id: BackendId,
        visibility: HostVisibility,
        target_filter: Option<CiTargetId>,
    ) -> Result<BackendReadV1, QueryError> {
        let row = sqlx::query("SELECT b.id,b.host_id,b.domain_id,b.runtime_kind,b.execution_os,b.architecture,b.enabled,b.revision,b.readiness,b.reason_code,b.last_probed_at FROM host_backends b WHERE b.id=?").bind(backend_id.to_string()).fetch_optional(&mut **tx).await?.ok_or(QueryError::NotFound)?;
        let host_id = parse_host_id(row.get::<String, _>("host_id").as_str())?;
        if host_id != expected_host_id {
            return Err(QueryError::NotFound);
        }
        if visibility == HostVisibility::TargetOnly
            && !backend_visible_for_target(
                tx,
                snapshot,
                targets,
                expected_host_id,
                backend_id,
                target_filter,
            )
            .await?
        {
            return Err(QueryError::NotFound);
        }
        let count_target_filter = target_filter.map(|id| id.to_string());
        let counts = counts(
            tx,
            snapshot,
            host_id,
            Some(backend_id),
            count_target_filter.as_deref(),
            if visibility == HostVisibility::Physical && target_filter.is_none() {
                CountScope::Host
            } else {
                CountScope::VisibleTargets
            },
        )
        .await?;
        let runtime_kind = runtime_kind(row.get::<String, _>("runtime_kind").as_str())?;
        let execution_os = execution_os(row.get::<String, _>("execution_os").as_str())?;
        let architecture = architecture(row.get::<String, _>("architecture").as_str())?;
        if visibility == HostVisibility::TargetOnly {
            return Ok(BackendReadV1::Target {
                id: backend_id,
                host_id,
                runtime_kind,
                execution_os,
                architecture,
                counts: counts.try_into()?,
            });
        }
        let domain_id = parse_domain_id(row.get::<String, _>("domain_id").as_str())?;
        let policy_revision = BrowserCounter::new(
            u64::try_from(row.get::<i64, _>("revision")).map_err(|_| QueryError::MalformedData)?,
        )?;
        let probe_is_current =
            current_backend_observation(tx, snapshot, host_id, backend_id).await?;
        let probe_freshness = if probe_is_current {
            freshness(row.get("last_probed_at"), snapshot.now)
        } else {
            Freshness::Unknown
        };
        Ok(BackendReadV1::Host {
            id: backend_id,
            host_id,
            domain_id,
            runtime_kind,
            execution_os,
            architecture,
            enabled: row.get::<i64, _>("enabled") != 0,
            policy_revision,
            observed_readiness: backend_readiness(row.get::<String, _>("readiness").as_str())?,
            probe_freshness,
            safe_reason: row
                .get::<Option<String>, _>("reason_code")
                .map(|reason| backend_reason(&reason)),
            counts,
            allocation: allocation_read(tx, host_id, Some(backend_id), None).await?,
        })
    }

    async fn domain_projection(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        snapshot: &Snapshot,
        domain_id: ResourceDomainId,
        host_id: HostId,
    ) -> Result<DomainReadV1, QueryError> {
        let row = sqlx::query("SELECT id,host_id,parent_domain_id,name,cpu_millis,memory_mib,disk_bytes,revision FROM host_resource_domains WHERE id=? AND host_id=?").bind(domain_id.to_string()).bind(host_id.to_string()).fetch_optional(&mut **tx).await?.ok_or(QueryError::NotFound)?;
        let observed = sqlx::query("SELECT kind,cpu_millis,memory_mib,disk_bytes,session_id,host_epoch,control_plane_incarnation,inventory_sequence,received_at FROM fleet_observed_domains WHERE host_id=? AND domain_id=? ORDER BY inventory_sequence DESC LIMIT 1")
            .bind(host_id.to_string())
            .bind(domain_id.to_string())
            .fetch_optional(&mut **tx)
            .await?;
        let observed_capacity = if let Some(ref observed) = observed {
            let received_at_value: i64 = observed.get("received_at");
            let current = current_observation_mapping(
                tx,
                snapshot,
                &host_id.to_string(),
                observed.get("session_id"),
                observed.get("host_epoch"),
                observed
                    .get::<String, _>("control_plane_incarnation")
                    .as_str(),
                observed.get("inventory_sequence"),
                received_at_value,
            )
            .await?;
            let received_at = wire_timestamp(received_at_value)?;
            let cpu = optional_browser(observed.get("cpu_millis"))?;
            let memory = optional_browser(observed.get("memory_mib"))?;
            let disk = optional_browser(observed.get("disk_bytes"))?;
            if current {
                ObservedCapacityV1::Current {
                    received_at,
                    cpu_millis: cpu,
                    memory_mib: memory,
                    disk_bytes: disk,
                }
            } else {
                ObservedCapacityV1::Historical {
                    received_at,
                    cpu_millis: cpu,
                    memory_mib: memory,
                    disk_bytes: disk,
                }
            }
        } else {
            ObservedCapacityV1::Missing
        };
        let kind = observed
            .as_ref()
            .map(|value| observed_kind(value.get::<String, _>("kind").as_str()))
            .transpose()?;
        Ok(DomainReadV1 {
            id: domain_id,
            host_id,
            parent_id: row
                .get::<Option<String>, _>("parent_domain_id")
                .map(|value| parse_domain_id(&value))
                .transpose()?,
            name: checked_text(row.get("name"))?,
            kind,
            policy_revision: BrowserCounter::new(
                u64::try_from(row.get::<i64, _>("revision"))
                    .map_err(|_| QueryError::MalformedData)?,
            )?,
            budget: amount(
                row.get("cpu_millis"),
                row.get("memory_mib"),
                row.get("disk_bytes"),
            )?,
            allocation: allocation_read(tx, host_id, None, Some(domain_id)).await?,
            observed_capacity,
            latest_measurement: latest_measurement(tx, snapshot, host_id, domain_id).await?,
        })
    }
}

fn checked_text(value: String) -> Result<crate::fleet::ids::UpstreamText, QueryError> {
    crate::fleet::ids::UpstreamText::new(value).map_err(|_| QueryError::MalformedData)
}
fn execution_os(value: &str) -> Result<ExecutionOs, QueryError> {
    match value {
        "linux" => Ok(ExecutionOs::Linux),
        "macos" => Ok(ExecutionOs::Macos),
        "windows" => Ok(ExecutionOs::Windows),
        _ => Err(QueryError::MalformedData),
    }
}
fn architecture(value: &str) -> Result<Architecture, QueryError> {
    match value {
        "x64" => Ok(Architecture::X64),
        "arm64" => Ok(Architecture::Arm64),
        _ => Err(QueryError::MalformedData),
    }
}
fn enrollment(value: &str) -> Result<EnrollmentState, QueryError> {
    match value {
        "pending" => Ok(EnrollmentState::Pending),
        "approved" => Ok(EnrollmentState::Approved),
        "revoked" => Ok(EnrollmentState::Revoked),
        _ => Err(QueryError::MalformedData),
    }
}
fn lifecycle(value: &str) -> Result<HostIntent, QueryError> {
    match value {
        "active" => Ok(HostIntent::Active),
        "paused" => Ok(HostIntent::Paused),
        "draining" => Ok(HostIntent::Draining),
        "maintenance" => Ok(HostIntent::Maintenance),
        "retired" => Ok(HostIntent::Retired),
        _ => Err(QueryError::MalformedData),
    }
}
fn integrity(value: &str) -> Result<IntegrityState, QueryError> {
    match value {
        "unverified" => Ok(IntegrityState::Unverified),
        "verified" => Ok(IntegrityState::Verified),
        "quarantined" => Ok(IntegrityState::Quarantined),
        _ => Err(QueryError::MalformedData),
    }
}
fn pressure(value: &str) -> Result<PressureState, QueryError> {
    match value {
        "normal" => Ok(PressureState::Normal),
        "pressured" => Ok(PressureState::Pressured),
        "unknown" => Ok(PressureState::Unknown),
        _ => Err(QueryError::MalformedData),
    }
}
fn runtime_kind(value: &str) -> Result<RuntimeKind, QueryError> {
    match value {
        "docker" => Ok(RuntimeKind::Docker),
        "tart_vm" => Ok(RuntimeKind::TartVm),
        "native_process" => Ok(RuntimeKind::NativeProcess),
        _ => Err(QueryError::MalformedData),
    }
}
fn backend_readiness(value: &str) -> Result<BackendReadiness, QueryError> {
    match value {
        "ready" => Ok(BackendReadiness::Ready),
        "reconciling" => Ok(BackendReadiness::Reconciling),
        "unavailable" => Ok(BackendReadiness::Unavailable),
        "unsupported" => Ok(BackendReadiness::Unsupported),
        "unknown" => Ok(BackendReadiness::Unknown),
        _ => Err(QueryError::MalformedData),
    }
}
fn observed_kind(value: &str) -> Result<ObservedDomainKindV1, QueryError> {
    match value {
        "physical" => Ok(ObservedDomainKindV1::Physical),
        "virtual_machine" => Ok(ObservedDomainKindV1::VirtualMachine),
        "container" => Ok(ObservedDomainKindV1::Container),
        _ => Err(QueryError::MalformedData),
    }
}
fn backend_reason(value: &str) -> BackendSafeReasonV1 {
    match value {
        "capability_unverified" => BackendSafeReasonV1::CapabilityUnverified,
        "incompatible_runtime" => BackendSafeReasonV1::IncompatibleRuntime,
        "incompatible_os" => BackendSafeReasonV1::IncompatibleOs,
        "incompatible_architecture" => BackendSafeReasonV1::IncompatibleArchitecture,
        "offline" => BackendSafeReasonV1::Offline,
        "pressure_unknown" => BackendSafeReasonV1::PressureUnknown,
        "resource_pressure" => BackendSafeReasonV1::ResourcePressure,
        "unsupported" => BackendSafeReasonV1::Unsupported,
        _ => BackendSafeReasonV1::Unknown,
    }
}
fn freshness(value: Option<i64>, now: i64) -> Freshness {
    match value {
        Some(value) if value > now => Freshness::Unknown,
        Some(value) if now.saturating_sub(value) < FRESH_PROBE_MILLIS => Freshness::Fresh,
        Some(value) if now.saturating_sub(value) < FRESH_HEARTBEAT_MILLIS => Freshness::Stale,
        Some(_) => Freshness::Offline,
        None => Freshness::Unknown,
    }
}

#[cfg(test)]
#[path = "query_snapshot_tests.rs"]
mod snapshot_tests;
