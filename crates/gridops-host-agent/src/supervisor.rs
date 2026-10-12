//! The protocol supervisor: enrolls once, then keeps one authenticated session
//! open with a handshake, a complete inventory and regular heartbeats. Any
//! failure drops the session and reconnects with capped, jittered backoff; a
//! long outage never strands the host. A rejected credential or an
//! incompatible server stops the loop, because retrying can't fix either.

use std::{future::Future, time::Duration};

use anyhow::{Context as _, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use gridops_core::fleet::{
    ids::{AuthoritySessionId, HostId, OperationId, UpstreamText},
    protocol::{
        inventory::{
            HandshakeRequest, HeartbeatRequest, InventoryReport, InventoryRequest, PendingRotation,
            ProtocolNegotiation, ProtocolRange,
        },
        primitives::{BrowserCounter, WireTimestamp},
    },
};
use rand::RngExt as _;
use secrecy::SecretString;
use uuid::Uuid;

use crate::{
    client::{Client, ClientError},
    journal::{Identity, Journal},
    probe,
};

const MAX_HEARTBEAT: Duration = Duration::from_secs(15);
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Why the supervisor stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Shutdown,
    /// The server doesn't speak this agent's protocol. The host stays in
    /// maintenance until the agent is upgraded.
    Incompatible,
    /// The credential was revoked or replaced; the host needs recovery.
    CredentialRejected,
}

pub struct Supervisor {
    client: Client,
    journal: Journal,
    name: String,
    heartbeat: Duration,
}

impl Supervisor {
    pub const fn new(client: Client, journal: Journal, name: String) -> Self {
        Self {
            client,
            journal,
            name,
            heartbeat: MAX_HEARTBEAT,
        }
    }

    /// Heartbeats more often than the server's lease requires (tests, or a
    /// host that wants faster command pickup). Never slower than the lease.
    #[must_use]
    pub const fn with_heartbeat(mut self, interval: Duration) -> Self {
        self.heartbeat = interval;
        self
    }

    /// Consumes a one-time enrollment code and stores the credential. The
    /// host stays pending until an administrator approves it.
    pub async fn enroll(&self, code: &str, hardware_fingerprint: Option<&str>) -> Result<()> {
        anyhow::ensure!(
            self.journal.identity().await?.is_none(),
            "this agent is already enrolled"
        );
        let enrollment = self
            .client
            .enroll(code, &self.name, hardware_fingerprint)
            .await?;
        self.journal
            .store_identity(&Identity {
                host_id: enrollment.host_id,
                host_epoch: gridops_core::fleet::ids::HostEpoch::new(1)?,
                credential: SecretString::from(enrollment.credential),
                credential_generation: enrollment.credential_generation,
            })
            .await
    }

    pub async fn run(&self, shutdown: impl Future<Output = ()>) -> Result<Stop> {
        self.journal
            .identity()
            .await?
            .context("this agent isn't enrolled yet")?;
        tokio::pin!(shutdown);
        let mut failures = 0_u32;
        loop {
            let error = tokio::select! {
                () = &mut shutdown => return Ok(Stop::Shutdown),
                result = self.session(&mut failures) => match result {
                    Ok(stop) => return Ok(stop),
                    Err(error) => error,
                },
            };
            if matches!(error, ClientError::Unauthenticated) {
                return Ok(Stop::CredentialRejected);
            }
            let delay = backoff(failures);
            failures = failures.saturating_add(1);
            tracing::warn!(%error, ?delay, "fleet session ended; reconnecting");
            tokio::select! {
                () = &mut shutdown => return Ok(Stop::Shutdown),
                () = tokio::time::sleep(delay) => {}
            }
        }
    }

    /// One session. Returns only on failure or a terminal stop.
    async fn session(&self, failures: &mut u32) -> Result<Stop, ClientError> {
        let identity = self
            .journal
            .identity()
            .await
            .map_err(internal)?
            .ok_or_else(|| internal("identity is missing"))?;
        let mut credential = identity.credential;
        let session_id = new_id::<AuthoritySessionId>()?;
        let boot_id = text(&probe::boot_id())?;
        let handshake = self
            .client
            .handshake(
                identity.host_id,
                &credential,
                &HandshakeRequest::new(
                    new_id::<OperationId>()?,
                    session_id,
                    boot_id.clone(),
                    ProtocolRange::new(1, 1).map_err(internal)?,
                    text(env!("CARGO_PKG_VERSION"))?,
                    Some(identity.host_epoch),
                ),
            )
            .await?;
        if handshake.negotiated_protocol == ProtocolNegotiation::Incompatible {
            return Ok(Stop::Incompatible);
        }
        let observed_at = now()?;
        let report = InventoryReport::new(
            handshake.host_epoch,
            handshake.control_plane_incarnation,
            session_id,
            boot_id.clone(),
            BrowserCounter::new(1).map_err(internal)?,
            observed_at,
            vec![probe::root_domain(&self.name).map_err(internal)?],
            Vec::new(),
            Vec::new(),
            vec![probe::root_sample(observed_at).map_err(internal)?],
        )
        .map_err(internal)?;
        let inventory = self
            .client
            .inventory(
                identity.host_id,
                &credential,
                &InventoryRequest::new(new_id::<OperationId>()?, report).map_err(internal)?,
            )
            .await?;
        *failures = 0;
        let interval =
            Duration::from_millis(inventory.lease.heartbeat_ms.get()).min(self.heartbeat);
        let mut sequence = 1_u64;
        loop {
            tokio::time::sleep(interval).await;
            sequence += 1;
            let observed_at = now()?;
            let heartbeat = self
                .client
                .heartbeat(
                    identity.host_id,
                    &credential,
                    &HeartbeatRequest::new(
                        new_id::<OperationId>()?,
                        handshake.host_epoch,
                        handshake.control_plane_incarnation,
                        session_id,
                        boot_id.clone(),
                        BrowserCounter::new(sequence).map_err(internal)?,
                        observed_at,
                        vec![probe::root_sample(observed_at).map_err(internal)?],
                    )
                    .map_err(internal)?,
                )
                .await?;
            if let Some(rotation) = heartbeat.pending_rotation {
                // The session continues; only the credential changes.
                credential = self.rotate(identity.host_id, &credential, rotation).await?;
            }
        }
    }

    async fn rotate(
        &self,
        host_id: HostId,
        credential: &SecretString,
        rotation: PendingRotation,
    ) -> Result<SecretString, ClientError> {
        let next = self
            .journal
            .rotation_secret(rotation.operation_id, rotation.next_generation, || {
                URL_SAFE_NO_PAD.encode(rand::rng().random::<[u8; 32]>())
            })
            .await
            .map_err(internal)?;
        self.client
            .exchange_rotation(host_id, credential, rotation.operation_id, &next)
            .await?;
        self.client
            .acknowledge_rotation(host_id, &next, rotation.operation_id)
            .await?;
        self.journal
            .finish_rotation(&next, rotation.next_generation)
            .await
            .map_err(internal)?;
        tracing::info!(
            generation = rotation.next_generation.get(),
            "host credential rotated"
        );
        Ok(next)
    }
}

/// Full-jitter exponential backoff from 1 s, capped at 30 s.
fn backoff(failures: u32) -> Duration {
    let ceiling = MIN_BACKOFF
        .saturating_mul(2_u32.saturating_pow(failures.min(5)))
        .min(MAX_BACKOFF);
    let millis = u64::try_from(ceiling.as_millis()).unwrap_or(u64::MAX);
    Duration::from_millis(rand::rng().random_range(MIN_BACKOFF.as_millis() as u64..=millis))
}

fn new_id<T: TryFrom<Uuid>>() -> Result<T, ClientError> {
    T::try_from(Uuid::new_v4()).map_err(|_| ClientError::Unavailable("nil identifier".into()))
}

fn text(value: &str) -> Result<UpstreamText, ClientError> {
    UpstreamText::new(value).map_err(internal)
}

fn now() -> Result<WireTimestamp, ClientError> {
    WireTimestamp::from_millis(gridops_core::now_millis()).map_err(internal)
}

fn internal(error: impl std::fmt::Display) -> ClientError {
    ClientError::Unavailable(error.to_string())
}
