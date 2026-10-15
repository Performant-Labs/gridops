//! Local durable journal. A command's receipt is written before any work
//! starts and its result before it is acknowledged, so a restart or a lost
//! response never repeats non-repeatable work. The journal also records which
//! prepared environments and local resources this agent owns; dispatch refuses
//! any operation on a resource that isn't recorded here.

use std::{path::Path, str::FromStr, time::Duration};

use anyhow::{Context as _, Result};
use gridops_core::fleet::{
    domain::{ExecutionOs, RuntimeKind},
    ids::{
        CommandId, Generation, HostEpoch, HostId, OperationId, PlacementId, PreparedEnvironmentId,
    },
    protocol::{
        agent::{
            AgentCommandEnvelope, LocalResourceIdentity, PrepareEnvironment, PreparedWorkload,
        },
        outcomes::AgentCommandResult,
    },
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Serialize, de::DeserializeOwned};
use sqlx::{
    Row as _, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS identity (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  host_id TEXT NOT NULL,
  host_epoch INTEGER NOT NULL,
  credential TEXT NOT NULL,
  credential_generation INTEGER NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS rotation (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  operation_id TEXT NOT NULL,
  next_generation INTEGER NOT NULL,
  next_secret TEXT NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS commands (
  command_id TEXT PRIMARY KEY,
  request_hash TEXT NOT NULL,
  envelope TEXT NOT NULL,
  received_at INTEGER NOT NULL,
  result TEXT
) STRICT;
CREATE TABLE IF NOT EXISTS environments (
  id TEXT PRIMARY KEY,
  placement_id TEXT NOT NULL,
  generation INTEGER NOT NULL,
  workload TEXT NOT NULL,
  runtime_kind TEXT NOT NULL,
  execution_os TEXT NOT NULL,
  docker_socket INTEGER NOT NULL,
  prepare TEXT NOT NULL,
  local_identity TEXT,
  UNIQUE (placement_id, generation)
) STRICT;
CREATE TABLE IF NOT EXISTS shipped_output (
  environment_id TEXT PRIMARY KEY
) STRICT;
CREATE TABLE IF NOT EXISTS delivered_results (
  command_id TEXT PRIMARY KEY
) STRICT;
";

/// The host's enrolled identity and its opaque credential.
pub struct Identity {
    pub host_id: HostId,
    pub host_epoch: HostEpoch,
    pub credential: SecretString,
    pub credential_generation: Generation,
}

/// What the journal already knows about a command.
#[derive(Debug)]
pub enum Receipt {
    /// First delivery: the receipt is now durable and work may start.
    New,
    /// Received before but no result was recorded, so the agent stopped
    /// mid-operation and can't know whether the work happened.
    Interrupted,
    /// Already finished; the stored result is returned unchanged.
    Completed(Box<AgentCommandResult>),
}

/// An environment this agent prepared for one placement generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub id: PreparedEnvironmentId,
    pub placement_id: PlacementId,
    pub generation: Generation,
    pub workload: PreparedWorkload,
    pub runtime_kind: RuntimeKind,
    pub execution_os: ExecutionOs,
    pub docker_socket: bool,
    pub prepare: PrepareEnvironment,
    pub local_identity: Option<LocalResourceIdentity>,
}

#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("command ID was reused with a different request")]
    Conflict,
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

impl From<sqlx::Error> for JournalError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(error.into())
    }
}

#[derive(Clone)]
pub struct Journal {
    pool: SqlitePool,
}

impl Journal {
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Full)
                    .busy_timeout(Duration::from_secs(5)),
            )
            .await
            .with_context(|| format!("opening journal {}", path.display()))?;
        sqlx::raw_sql(SCHEMA).execute(&pool).await?;
        restrict_permissions(path)?;
        Ok(Self { pool })
    }

    pub async fn identity(&self) -> Result<Option<Identity>> {
        let Some(row) =
            sqlx::query("SELECT host_id,host_epoch,credential,credential_generation FROM identity")
                .fetch_optional(&self.pool)
                .await?
        else {
            return Ok(None);
        };
        Ok(Some(Identity {
            host_id: HostId::from_str(row.get("host_id"))?,
            host_epoch: HostEpoch::new(row.get::<i64, _>("host_epoch").try_into()?)?,
            credential: SecretString::from(row.get::<String, _>("credential")),
            credential_generation: Generation::new(
                row.get::<i64, _>("credential_generation").try_into()?,
            )?,
        }))
    }

    pub async fn store_identity(&self, identity: &Identity) -> Result<()> {
        sqlx::query(
            "INSERT INTO identity (singleton,host_id,host_epoch,credential,credential_generation)
             VALUES (1,?,?,?,?)
             ON CONFLICT(singleton) DO UPDATE SET host_id=excluded.host_id,
             host_epoch=excluded.host_epoch,credential=excluded.credential,
             credential_generation=excluded.credential_generation",
        )
        .bind(identity.host_id.to_string())
        .bind(i64::try_from(identity.host_epoch.get())?)
        .bind(identity.credential.expose_secret())
        .bind(i64::try_from(identity.credential_generation.get())?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The replacement secret for a rotation, created and stored before the
    /// exchange so a crash can replay the same secret instead of conflicting.
    pub async fn rotation_secret(
        &self,
        operation_id: OperationId,
        next_generation: Generation,
        fresh: impl FnOnce() -> String,
    ) -> Result<SecretString> {
        let operation = operation_id.to_string();
        if let Some(row) = sqlx::query("SELECT operation_id,next_secret FROM rotation")
            .fetch_optional(&self.pool)
            .await?
        {
            if row.get::<String, _>("operation_id") == operation {
                return Ok(SecretString::from(row.get::<String, _>("next_secret")));
            }
        }
        let secret = fresh();
        sqlx::query(
            "INSERT INTO rotation (singleton,operation_id,next_generation,next_secret) VALUES (1,?,?,?)
             ON CONFLICT(singleton) DO UPDATE SET operation_id=excluded.operation_id,
             next_generation=excluded.next_generation,next_secret=excluded.next_secret",
        )
        .bind(&operation)
        .bind(i64::try_from(next_generation.get())?)
        .bind(&secret)
        .execute(&self.pool)
        .await?;
        Ok(SecretString::from(secret))
    }

    /// Switches to the acknowledged credential and forgets the pending secret
    /// in one transaction.
    pub async fn finish_rotation(
        &self,
        credential: &SecretString,
        generation: Generation,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE identity SET credential=?,credential_generation=?")
            .bind(credential.expose_secret())
            .bind(i64::try_from(generation.get())?)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM rotation")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Records receipt before any work. A reused command ID with a different
    /// request hash is a conflict, never a second execution.
    pub async fn receive(&self, envelope: &AgentCommandEnvelope) -> Result<Receipt, JournalError> {
        let command_id = envelope.command_id.to_string();
        let inserted = sqlx::query(
            "INSERT INTO commands (command_id,request_hash,envelope,received_at)
             VALUES (?,?,?,?) ON CONFLICT(command_id) DO NOTHING",
        )
        .bind(&command_id)
        .bind(&envelope.request_hash)
        .bind(to_json(envelope)?)
        .bind(gridops_core::now_millis())
        .execute(&self.pool)
        .await?
        .rows_affected();
        if inserted == 1 {
            return Ok(Receipt::New);
        }
        let row = sqlx::query("SELECT request_hash,result FROM commands WHERE command_id=?")
            .bind(&command_id)
            .fetch_one(&self.pool)
            .await?;
        if row.get::<String, _>("request_hash") != envelope.request_hash {
            return Err(JournalError::Conflict);
        }
        Ok(match row.get::<Option<String>, _>("result") {
            Some(result) => Receipt::Completed(Box::new(from_json(&result)?)),
            None => Receipt::Interrupted,
        })
    }

    pub async fn complete(&self, result: &AgentCommandResult) -> Result<()> {
        let updated =
            sqlx::query("UPDATE commands SET result=? WHERE command_id=? AND result IS NULL")
                .bind(to_json(result)?)
                .bind(result.command_id.to_string())
                .execute(&self.pool)
                .await?
                .rows_affected();
        anyhow::ensure!(updated == 1, "command result was already recorded");
        Ok(())
    }

    /// Commands received since `since` (Unix milliseconds) whose result the
    /// server hasn't taken yet, oldest first. A command that was interrupted
    /// before it finished has no result.
    pub async fn undelivered(
        &self,
        since: i64,
    ) -> Result<Vec<(AgentCommandEnvelope, Option<AgentCommandResult>)>> {
        sqlx::query(
            "SELECT envelope,result FROM commands c WHERE received_at>=?
               AND NOT EXISTS (SELECT 1 FROM delivered_results d WHERE d.command_id=c.command_id)
             ORDER BY received_at,command_id",
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|row| {
            Ok((
                from_json(&row.get::<String, _>("envelope"))?,
                row.get::<Option<String>, _>("result")
                    .map(|result| from_json(&result))
                    .transpose()?,
            ))
        })
        .collect()
    }

    /// The server has this command's result.
    pub async fn delivered(&self, command_id: CommandId) -> Result<()> {
        sqlx::query("INSERT INTO delivered_results (command_id) VALUES (?) ON CONFLICT DO NOTHING")
            .bind(command_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn record_environment(&self, environment: &Environment) -> Result<()> {
        sqlx::query(
            "INSERT INTO environments
             (id,placement_id,generation,workload,runtime_kind,execution_os,docker_socket,prepare,local_identity)
             VALUES (?,?,?,?,?,?,?,?,?)",
        )
        .bind(environment.id.to_string())
        .bind(environment.placement_id.to_string())
        .bind(i64::try_from(environment.generation.get())?)
        .bind(to_json(&environment.workload)?)
        .bind(to_json(&environment.runtime_kind)?)
        .bind(to_json(&environment.execution_os)?)
        .bind(environment.docker_socket)
        .bind(to_json(&environment.prepare)?)
        .bind(environment.local_identity.as_ref().map(to_json).transpose()?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Binds the runtime object created by a start. An environment owns at
    /// most one local resource, and it can't be rebound to another.
    pub async fn bind_local_identity(
        &self,
        id: PreparedEnvironmentId,
        identity: &LocalResourceIdentity,
    ) -> Result<()> {
        let encoded = to_json(identity)?;
        let updated = sqlx::query(
            "UPDATE environments SET local_identity=?
             WHERE id=? AND (local_identity IS NULL OR local_identity=?)",
        )
        .bind(&encoded)
        .bind(id.to_string())
        .bind(&encoded)
        .execute(&self.pool)
        .await?
        .rows_affected();
        anyhow::ensure!(updated == 1, "environment already owns another resource");
        Ok(())
    }

    pub async fn environment(&self, id: PreparedEnvironmentId) -> Result<Option<Environment>> {
        sqlx::query("SELECT * FROM environments WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .map(|row| environment(&row))
            .transpose()
    }

    pub async fn placement_environment(
        &self,
        placement_id: PlacementId,
        generation: Generation,
    ) -> Result<Option<Environment>> {
        sqlx::query("SELECT * FROM environments WHERE placement_id=? AND generation=?")
            .bind(placement_id.to_string())
            .bind(i64::try_from(generation.get())?)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| environment(&row))
            .transpose()
    }

    /// Started environments whose runner output hasn't been fully shipped.
    pub async fn unshipped_output(&self) -> Result<Vec<Environment>> {
        sqlx::query(
            "SELECT * FROM environments WHERE local_identity IS NOT NULL
               AND id NOT IN (SELECT environment_id FROM shipped_output)",
        )
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(environment)
        .collect()
    }

    /// The environment's runner output is shipped, or can't be any more.
    pub async fn output_shipped(&self, id: PreparedEnvironmentId) -> Result<()> {
        sqlx::query("INSERT OR IGNORE INTO shipped_output (environment_id) VALUES (?)")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

fn environment(row: &sqlx::sqlite::SqliteRow) -> Result<Environment> {
    Ok(Environment {
        id: PreparedEnvironmentId::from_str(row.get("id"))?,
        placement_id: PlacementId::from_str(row.get("placement_id"))?,
        generation: Generation::new(row.get::<i64, _>("generation").try_into()?)?,
        workload: from_json(row.get("workload"))?,
        runtime_kind: from_json(row.get("runtime_kind"))?,
        execution_os: from_json(row.get("execution_os"))?,
        docker_socket: row.get("docker_socket"),
        prepare: from_json(row.get("prepare"))?,
        local_identity: row
            .get::<Option<String>, _>("local_identity")
            .map(|value| from_json(&value))
            .transpose()?,
    })
}

fn to_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn from_json<T: DeserializeOwned>(value: &str) -> Result<T> {
    Ok(serde_json::from_str(value)?)
}

/// The journal holds the host credential, so only the agent's account may
/// read it.
fn restrict_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
