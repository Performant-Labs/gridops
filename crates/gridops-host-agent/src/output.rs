//! Ships each started runner's console output to `GridOps`, so a job on this
//! machine shows its steps live in the job log view.
//!
//! Every few seconds the shipper looks in the journal for started
//! environments whose output isn't shipped yet and follows each one from the
//! beginning. It asks `GridOps` how much it already has, skips that much, and
//! uploads the rest as it appears, at most a second behind. Reading a runner's
//! output again yields the same bytes, so a new session or a restarted agent
//! resumes without sending anything twice. Once the output ends, or `GridOps`
//! won't take it, the environment is recorded as shipped and left alone.

use std::{collections::HashSet, sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use gridops_core::fleet::{
    ids::{AuthoritySessionId, HostId, PlacementId},
    protocol::{browser::ErrorCode, primitives::WireTimestamp},
    registry::output::{RunnerOutputChunk, RunnerOutputReceipt},
};
use tokio::{io::AsyncReadExt as _, sync::watch, task::JoinSet, time::Instant};

use crate::{
    client::{Client, ClientError},
    helper::{RunnerOutput, RuntimeHelper},
    journal::{Environment, Journal},
    supervisor::Authority,
};

const SCAN_INTERVAL: Duration = Duration::from_secs(2);
/// New output waits at most this long before it's uploaded.
const BATCH_WINDOW: Duration = Duration::from_secs(1);
/// Upload size; `GridOps` takes up to 256 KiB per chunk.
const CHUNK_BYTES: usize = 192 * 1024;
const READ_BYTES: usize = 16 * 1024;
const RETRY: Duration = Duration::from_secs(2);

/// What a session lends the shipper.
pub(crate) struct Shipper<H> {
    pub client: Client,
    pub journal: Journal,
    pub runtime: Arc<H>,
    pub host_id: HostId,
    pub authority_session: AuthoritySessionId,
    pub authority: watch::Receiver<Authority>,
}

/// Follows started runners until the session ends; never returns. Dropping
/// it stops every upload and output reader.
pub(crate) async fn ship<H: RuntimeHelper + 'static>(session: Shipper<H>) {
    let session = Arc::new(session);
    let mut followers = JoinSet::new();
    let mut following = HashSet::new();
    loop {
        while let Some(done) = followers.try_join_next() {
            if let Ok(id) = done {
                following.remove(&id);
            }
        }
        match session.journal.unshipped_output().await {
            Ok(environments) => {
                for environment in environments {
                    if following.insert(environment.id) {
                        followers.spawn(follow(Arc::clone(&session), environment));
                    }
                }
            }
            Err(error) => tracing::warn!(%error, "couldn't list started runners"),
        }
        tokio::time::sleep(SCAN_INTERVAL).await;
    }
}

async fn follow<H: RuntimeHelper>(
    session: Arc<Shipper<H>>,
    environment: Environment,
) -> gridops_core::fleet::ids::PreparedEnvironmentId {
    let id = environment.id;
    if let Some(resource) = environment.local_identity.clone()
        && let Some(output) = session.runtime.output(resource).await
    {
        let mut upload = Upload {
            session: &session,
            authority: session.authority.clone(),
            placement: environment.placement_id,
            received: 0,
        };
        upload.stream(output).await;
    }
    if let Err(error) = session.journal.output_shipped(id).await {
        tracing::warn!(%error, "couldn't record shipped runner output");
    }
    id
}

struct Upload<'a, H> {
    session: &'a Shipper<H>,
    authority: watch::Receiver<Authority>,
    placement: PlacementId,
    /// How much of the output `GridOps` has.
    received: u64,
}

impl<H> Upload<'_, H> {
    /// Uploads `output` until it ends or `GridOps` won't take more.
    async fn stream(&mut self, mut output: RunnerOutput) {
        // Ask where GridOps is, so output it has isn't sent again.
        if self
            .send(0, &[], false)
            .await
            .is_none_or(|receipt| receipt.finished)
        {
            return;
        }
        let mut position = 0_u64;
        let mut buffer = Vec::with_capacity(CHUNK_BYTES + READ_BYTES);
        let mut scratch = vec![0; READ_BYTES];
        loop {
            let ended = gather(&mut output, &mut buffer, &mut scratch).await;
            let start = position;
            position += buffer.len() as u64;
            let skip = usize::try_from(self.received.saturating_sub(start))
                .unwrap_or(usize::MAX)
                .min(buffer.len());
            if (skip < buffer.len() || ended)
                && self
                    .send(start + skip as u64, &buffer[skip..], ended)
                    .await
                    .is_none_or(|receipt| receipt.finished)
            {
                return;
            }
            buffer.clear();
        }
    }

    /// Sends one chunk once the session holds work authority, retrying while
    /// `GridOps` is unreachable. `None` when it refused.
    async fn send(
        &mut self,
        offset: u64,
        data: &[u8],
        finished: bool,
    ) -> Option<RunnerOutputReceipt> {
        let chunk = RunnerOutputChunk {
            authority_session_id: self.session.authority_session,
            placement_id: self.placement,
            offset,
            data: STANDARD.encode(data),
            finished,
        };
        loop {
            let current = self.authority.borrow_and_update().clone();
            let now = WireTimestamp::from_millis(gridops_core::now_millis()).ok();
            if current
                .expires_at
                .zip(now)
                .is_none_or(|(expires, now)| expires <= now)
            {
                // GridOps takes output only from a session with work
                // authority; wait for the heartbeat that grants it. Without
                // a sender the session is over and this task is being
                // stopped, so nothing may be recorded as shipped.
                if self.authority.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
                continue;
            }
            match self
                .session
                .client
                .post_output(self.session.host_id, &current.credential, &chunk)
                .await
            {
                Ok(receipt) => {
                    self.received = receipt.received;
                    return Some(receipt);
                }
                Err(
                    ClientError::Unavailable(_)
                    | ClientError::Unauthenticated
                    | ClientError::Refused(ErrorCode::Forbidden | ErrorCode::RateLimited),
                ) => tokio::time::sleep(RETRY).await,
                Err(error) => {
                    tracing::warn!(%error, placement = %self.placement, "GridOps refused runner output");
                    return None;
                }
            }
        }
    }
}

/// Reads output until a chunk is full, a batch window has passed since the
/// first new byte, or the output ends. Returns whether it ended.
async fn gather(output: &mut RunnerOutput, buffer: &mut Vec<u8>, scratch: &mut [u8]) -> bool {
    let mut deadline = None;
    while buffer.len() < CHUNK_BYTES {
        let read = match deadline {
            None => output.read(scratch).await,
            Some(deadline) => match tokio::time::timeout_at(deadline, output.read(scratch)).await {
                Ok(read) => read,
                Err(_) => return false,
            },
        };
        match read {
            Ok(0) | Err(_) => return true,
            Ok(read) => {
                buffer.extend_from_slice(&scratch[..read]);
                deadline.get_or_insert_with(|| Instant::now() + BATCH_WINDOW);
            }
        }
    }
    false
}
