//! Console output of runners on fleet machines.
//!
//! The host agent ships each placement's runner output in ordered chunks. A
//! chunk says where in the output it starts, so a retried or replayed chunk is
//! appended once: whatever the stream already holds is skipped. Only the host
//! that owns the placement, in its current epoch and live session, can write
//! it. Output goes to a runner log stream (`log_streams`, linked to its
//! placement in `fleet_output_streams`), whose file keeps only the newest output
//! once it outgrows its cap and expires like other runner logs.

use std::{io::SeekFrom, path::Path};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sqlx::Row as _;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _};
use uuid::Uuid;

use super::{AuthenticatedHost, RegistryError, RegistryService};
use crate::fleet::ids::{AuthoritySessionId, PlacementId};

/// The most output one chunk may carry.
pub const MAX_OUTPUT_CHUNK_BYTES: usize = 256 * 1024;
/// A stream that would grow past this keeps only its newest output.
const MAX_STREAM_BYTES: u64 = 32 * 1024 * 1024;
const KEEP_STREAM_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerOutputChunk {
    pub authority_session_id: AuthoritySessionId,
    pub placement_id: PlacementId,
    /// Where `data` starts in the runner's output.
    pub offset: u64,
    /// The output bytes, base64-encoded.
    pub data: String,
    /// The runner's output ended with this chunk.
    #[serde(default)]
    pub finished: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerOutputReceipt {
    /// How much of the output `GridOps` has; the next chunk starts here.
    pub received: u64,
    /// The stream is closed and takes no more output.
    pub finished: bool,
}

impl RegistryService {
    /// Appends a chunk of a placement's runner output to its log stream in
    /// `directory`, creating the stream on the first chunk.
    pub async fn append_runner_output(
        &self,
        authenticated: &AuthenticatedHost,
        chunk: &RunnerOutputChunk,
        directory: &Path,
    ) -> Result<RunnerOutputReceipt, RegistryError> {
        let data = STANDARD
            .decode(&chunk.data)
            .map_err(|_| RegistryError::InvalidRequest("runner output is not base64"))?;
        if data.len() > MAX_OUTPUT_CHUNK_BYTES {
            return Err(RegistryError::InvalidRequest(
                "runner output chunk is too large",
            ));
        }
        let end = chunk
            .offset
            .checked_add(data.len() as u64)
            .filter(|end| i64::try_from(*end).is_ok())
            .ok_or(RegistryError::InvalidRequest(
                "runner output offset is too large",
            ))?;
        let mut tx = self.begin().await?;
        let now = self.clock.now_millis();
        super::ensure_authenticated_generation(&mut tx, authenticated, now).await?;
        let host_id = authenticated.host_id.to_string();
        let authorized: bool = sqlx::query_scalar(
            "SELECT agent_session_id=? AND authority_expires_at>? FROM fleet_hosts WHERE id=?",
        )
        .bind(chunk.authority_session_id.to_string())
        .bind(now)
        .bind(&host_id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten()
        .unwrap_or(false);
        if !authorized {
            return Err(RegistryError::Forbidden);
        }
        // Another host's placement, or this host's from before a recovery,
        // looks the same as one that doesn't exist.
        let placement = sqlx::query(
            "SELECT r.id AS runner_id,r.name AS runner_name,COALESCE(r.provider,'fleet') AS provider,pool.name AS pool_name,
                    pool.installation_id,repo.full_name AS repository
               FROM workload_placements p
               JOIN runners r ON r.id=p.runner_id
               JOIN runner_pools pool ON pool.id=r.pool_id
               LEFT JOIN repositories repo ON repo.id=COALESCE(r.target_repository_id,pool.repository_id)
              WHERE p.id=? AND p.host_id=? AND p.host_epoch=?",
        )
        .bind(chunk.placement_id.to_string())
        .bind(&host_id)
        .bind(i64::try_from(authenticated.epoch.get()).unwrap_or(i64::MAX))
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RegistryError::NotFound)?;
        let runner_id: String = placement.get("runner_id");
        let stream = sqlx::query(
            "SELECT s.id,s.path,s.size_bytes,s.complete,o.received_bytes FROM fleet_output_streams o
               JOIN log_streams s ON s.id=o.log_stream_id WHERE o.placement_id=?",
        )
        .bind(chunk.placement_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let (id, mut path, mut size, received) = match stream {
            Some(row) if row.get::<bool, _>("complete") => {
                // Late or replayed output for a closed stream changes nothing.
                let received = row.get::<i64, _>("received_bytes").max(0).cast_unsigned();
                tx.commit().await?;
                return Ok(RunnerOutputReceipt {
                    received,
                    finished: true,
                });
            }
            Some(row) => (
                row.get::<String, _>("id"),
                row.get::<String, _>("path"),
                row.get::<i64, _>("size_bytes").max(0).cast_unsigned(),
                row.get::<i64, _>("received_bytes").max(0).cast_unsigned(),
            ),
            None => {
                let id = Uuid::new_v4().to_string();
                let path = format!("{id}.log");
                let retention_days: i64 =
                    sqlx::query_scalar("SELECT value FROM settings WHERE key='logRetentionDays'")
                        .fetch_optional(&mut *tx)
                        .await?
                        .and_then(|value: String| serde_json::from_str(&value).ok())
                        .unwrap_or(30);
                sqlx::query(
                    "INSERT INTO log_streams (
                       id,runner_id,installation_id,runner_name,pool_name,repository,source,path,
                       size_bytes,complete,expires_at,created_at,updated_at
                     ) VALUES (?,?,?,?,?,?,?,?,0,0,?,?,?)",
                )
                .bind(&id)
                .bind(&runner_id)
                .bind(placement.get::<Option<i64>, _>("installation_id"))
                .bind(placement.get::<String, _>("runner_name"))
                .bind(placement.get::<String, _>("pool_name"))
                .bind(placement.get::<Option<String>, _>("repository"))
                .bind(placement.get::<String, _>("provider"))
                .bind(&path)
                .bind(now.saturating_add(retention_days.saturating_mul(86_400_000)))
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "INSERT INTO fleet_output_streams (placement_id,log_stream_id) VALUES (?,?)",
                )
                .bind(chunk.placement_id.to_string())
                .bind(&id)
                .execute(&mut *tx)
                .await?;
                (id, path, 0, 0)
            }
        };
        let skip = usize::try_from(received.saturating_sub(chunk.offset))
            .unwrap_or(usize::MAX)
            .min(data.len());
        let fresh = &data[skip..];
        // A trimmed stream moves to a new file; the old one goes once the
        // move commits, or the new one if it doesn't.
        let mut replaced = None;
        if !fresh.is_empty() {
            tokio::fs::create_dir_all(directory).await?;
            let current = directory.join(&path);
            if size + fresh.len() as u64 <= MAX_STREAM_BYTES {
                let mut file = tokio::fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .write(true)
                    .open(&current)
                    .await?;
                // Drop anything a write that never committed left behind.
                file.set_len(size).await?;
                file.seek(SeekFrom::Start(size)).await?;
                file.write_all(fresh).await?;
                file.flush().await?;
                size += fresh.len() as u64;
            } else {
                let kept = newest_output(&current, size, fresh).await?;
                let next = format!("{id}.{end}.log");
                tokio::fs::write(directory.join(&next), &kept).await?;
                replaced = Some((current, directory.join(&next)));
                path = next;
                size = kept.len() as u64;
            }
        }
        let received = received.max(end);
        let updated = sqlx::query(
            "UPDATE log_streams SET path=?,size_bytes=?,complete=?,updated_at=?,
               job_id=CASE WHEN ? THEN COALESCE(job_id,(SELECT job.id FROM workflow_jobs job
                 JOIN runners r ON job.id=COALESCE(r.current_job_id,r.last_job_id) WHERE r.id=?))
               ELSE job_id END
             WHERE id=?",
        )
        .bind(&path)
        .bind(i64::try_from(size).unwrap_or(i64::MAX))
        .bind(chunk.finished)
        .bind(now)
        .bind(chunk.finished)
        .bind(&runner_id)
        .bind(&id)
        .execute(&mut *tx)
        .await;
        let updated = match updated {
            Ok(_) => {
                sqlx::query(
                    "UPDATE fleet_output_streams SET received_bytes=? WHERE log_stream_id=?",
                )
                .bind(i64::try_from(received).unwrap_or(i64::MAX))
                .bind(&id)
                .execute(&mut *tx)
                .await
            }
            Err(error) => Err(error),
        };
        let committed = match updated {
            Ok(_) => tx.commit().await,
            Err(error) => Err(error),
        };
        if let Some((old, new)) = replaced {
            let _ = tokio::fs::remove_file(if committed.is_ok() { old } else { new }).await;
        }
        committed?;
        Ok(RunnerOutputReceipt {
            received,
            finished: chunk.finished,
        })
    }
}

/// The newest `KEEP_STREAM_BYTES` of the stream's `size` bytes followed by
/// `fresh`, starting at a line.
async fn newest_output(path: &Path, size: u64, fresh: &[u8]) -> std::io::Result<Vec<u8>> {
    let kept = KEEP_STREAM_BYTES
        .saturating_sub(fresh.len() as u64)
        .min(size);
    let mut output = Vec::with_capacity(usize::try_from(kept).unwrap_or(0) + fresh.len());
    let mut file = tokio::fs::File::open(path).await?;
    file.seek(SeekFrom::Start(size - kept)).await?;
    file.take(kept).read_to_end(&mut output).await?;
    output.extend_from_slice(fresh);
    let start = output
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(0, |newline| newline + 1);
    output.drain(..start);
    Ok(output)
}
