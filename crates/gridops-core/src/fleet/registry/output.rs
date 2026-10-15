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

/// Output files are written outside `SQLite`'s write lock, so two appends to
/// a stream (an agent retry racing the original) must not interleave their
/// file writes. Appends are small and short; one lock for all streams is
/// plenty.
static FILE_WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A placement's output stream as stored.
struct Stream {
    id: String,
    path: String,
    size: u64,
    received: u64,
    /// Not created yet: the first chunk creates it.
    new: bool,
}

impl RegistryService {
    /// Appends a chunk of a placement's runner output to its log stream in
    /// `directory`, creating the stream on the first chunk.
    ///
    /// The file is written between two short transactions, never inside one:
    /// a slow disk would otherwise hold `SQLite`'s single write lock and stall
    /// every other fleet request. Bytes that reach the file without being
    /// recorded are dropped by the next append, which starts from the
    /// recorded size.
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
        let _writing = FILE_WRITES.lock().await;
        let now = self.clock.now_millis();
        let mut read = self.pool.begin().await?;
        let runner_id = authorize_output(&mut read, authenticated, chunk, now).await?;
        let stream = sqlx::query(
            "SELECT s.id,s.path,s.size_bytes,s.complete,o.received_bytes FROM fleet_output_streams o
               JOIN log_streams s ON s.id=o.log_stream_id WHERE o.placement_id=?",
        )
        .bind(chunk.placement_id.to_string())
        .fetch_optional(&mut *read)
        .await?;
        read.commit().await?;
        let stream = match stream {
            Some(row) if row.get::<bool, _>("complete") => {
                // Late or replayed output for a closed stream changes nothing.
                return Ok(RunnerOutputReceipt {
                    received: row.get::<i64, _>("received_bytes").max(0).cast_unsigned(),
                    finished: true,
                });
            }
            Some(row) => Stream {
                id: row.get("id"),
                path: row.get("path"),
                size: row.get::<i64, _>("size_bytes").max(0).cast_unsigned(),
                received: row.get::<i64, _>("received_bytes").max(0).cast_unsigned(),
                new: false,
            },
            None => {
                let id = Uuid::new_v4().to_string();
                Stream {
                    path: format!("{id}.log"),
                    id,
                    size: 0,
                    received: 0,
                    new: true,
                }
            }
        };
        let skip = usize::try_from(stream.received.saturating_sub(chunk.offset))
            .unwrap_or(usize::MAX)
            .min(data.len());
        let fresh = &data[skip..];
        let current = directory.join(&stream.path);
        let (path, size) = if fresh.is_empty() {
            (stream.path.clone(), stream.size)
        } else if stream.size + fresh.len() as u64 <= MAX_STREAM_BYTES {
            tokio::fs::create_dir_all(directory).await?;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&current)
                .await?;
            // Drop anything a write that never committed left behind.
            file.set_len(stream.size).await?;
            file.seek(SeekFrom::Start(stream.size)).await?;
            file.write_all(fresh).await?;
            file.flush().await?;
            (stream.path.clone(), stream.size + fresh.len() as u64)
        } else {
            // A trimmed stream moves to a new file; the old one goes once the
            // move commits, or the new one if it doesn't.
            let kept = newest_output(&current, stream.size, fresh).await?;
            let next = format!("{}.{end}.log", stream.id);
            tokio::fs::write(directory.join(&next), &kept).await?;
            (next, kept.len() as u64)
        };
        let received = stream.received.max(end);
        let recorded = self
            .record_output(
                authenticated,
                chunk,
                &runner_id,
                &stream,
                &path,
                size,
                received,
            )
            .await;
        if path != stream.path {
            let _ = tokio::fs::remove_file(if matches!(recorded, Ok(true)) {
                current
            } else {
                directory.join(&path)
            })
            .await;
        }
        if !recorded? {
            // The stream closed while this chunk was being written.
            return Ok(RunnerOutputReceipt {
                received: stream.received,
                finished: true,
            });
        }
        Ok(RunnerOutputReceipt {
            received,
            finished: chunk.finished,
        })
    }

    /// Records what the stream's file now holds, creating the stream for its
    /// first chunk. Returns false when the stream was closed meanwhile.
    #[allow(clippy::too_many_arguments)]
    async fn record_output(
        &self,
        authenticated: &AuthenticatedHost,
        chunk: &RunnerOutputChunk,
        runner_id: &str,
        stream: &Stream,
        path: &str,
        size: u64,
        received: u64,
    ) -> Result<bool, RegistryError> {
        let mut tx = self.begin().await?;
        let now = self.clock.now_millis();
        authorize_output(&mut tx, authenticated, chunk, now).await?;
        if stream.new {
            let placement = sqlx::query(
                "SELECT r.name AS runner_name,COALESCE(r.provider,'fleet') AS provider,pool.name AS pool_name,
                        pool.installation_id,repo.full_name AS repository
                   FROM runners r
                   JOIN runner_pools pool ON pool.id=r.pool_id
                   LEFT JOIN repositories repo ON repo.id=COALESCE(r.target_repository_id,pool.repository_id)
                  WHERE r.id=?",
            )
            .bind(runner_id)
            .fetch_one(&mut *tx)
            .await?;
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
            .bind(&stream.id)
            .bind(runner_id)
            .bind(placement.get::<Option<i64>, _>("installation_id"))
            .bind(placement.get::<String, _>("runner_name"))
            .bind(placement.get::<String, _>("pool_name"))
            .bind(placement.get::<Option<String>, _>("repository"))
            .bind(placement.get::<String, _>("provider"))
            .bind(path)
            .bind(now.saturating_add(retention_days.saturating_mul(86_400_000)))
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO fleet_output_streams (placement_id,log_stream_id) VALUES (?,?)",
            )
            .bind(chunk.placement_id.to_string())
            .bind(&stream.id)
            .execute(&mut *tx)
            .await?;
        }
        let updated = sqlx::query(
            "UPDATE log_streams SET path=?,size_bytes=?,complete=?,updated_at=?,
               job_id=CASE WHEN ? THEN COALESCE(job_id,(SELECT job.id FROM workflow_jobs job
                 JOIN runners r ON job.id=COALESCE(r.current_job_id,r.last_job_id) WHERE r.id=?))
               ELSE job_id END
             WHERE id=? AND complete=0",
        )
        .bind(path)
        .bind(i64::try_from(size).unwrap_or(i64::MAX))
        .bind(chunk.finished)
        .bind(now)
        .bind(chunk.finished)
        .bind(runner_id)
        .bind(&stream.id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if updated == 0 {
            tx.commit().await?;
            return Ok(false);
        }
        sqlx::query("UPDATE fleet_output_streams SET received_bytes=? WHERE log_stream_id=?")
            .bind(i64::try_from(received).unwrap_or(i64::MAX))
            .bind(&stream.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }
}

/// Only the host that owns the placement, in its current epoch and live
/// session, may write its output. Returns the placement's runner.
async fn authorize_output(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    authenticated: &AuthenticatedHost,
    chunk: &RunnerOutputChunk,
    now: i64,
) -> Result<String, RegistryError> {
    super::ensure_authenticated_generation(tx, authenticated, now).await?;
    let host_id = authenticated.host_id.to_string();
    let authorized: bool = sqlx::query_scalar(
        "SELECT agent_session_id=? AND authority_expires_at>? FROM fleet_hosts WHERE id=?",
    )
    .bind(chunk.authority_session_id.to_string())
    .bind(now)
    .bind(&host_id)
    .fetch_optional(&mut **tx)
    .await?
    .flatten()
    .unwrap_or(false);
    if !authorized {
        return Err(RegistryError::Forbidden);
    }
    // Another host's placement, or this host's from before a recovery,
    // looks the same as one that doesn't exist.
    sqlx::query_scalar(
        "SELECT p.runner_id FROM workload_placements p JOIN runners r ON r.id=p.runner_id
          WHERE p.id=? AND p.host_id=? AND p.host_epoch=?",
    )
    .bind(chunk.placement_id.to_string())
    .bind(&host_id)
    .bind(i64::try_from(authenticated.epoch.get()).unwrap_or(i64::MAX))
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(RegistryError::NotFound)
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
