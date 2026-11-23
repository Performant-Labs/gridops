//! D18 (epic #136): the one writer of `gitlab_jobs`, shared by the webhook
//! (#140) and the pending-jobs poll (#148), so both apply one status rule.
//!
//! #140 owns this module; #148 carries this copy, at the same path with the
//! same signatures and rules, until #140 merges. Whichever of the two merges
//! second keeps master's file. Its tests are `gitlab/jobs_tests.rs`, mounted
//! from `gitlab/mod.rs`.
//!
//! The rules:
//! - tiers ([`status_tier`]): 0 `created`, `waiting_for_resource`,
//!   `preparing`, `manual`, `scheduled`; 1 `pending`; 2 `running`; 3
//!   `success`, `failed`, `canceled`, `skipped`. `unknown` has no tier and
//!   writes nothing.
//! - `status` and `status_changed_at` (with `pipeline_id`, and
//!   `retries_count` when given) change only when the stored row is not
//!   terminal and the incoming tier is higher, or equal with a strictly newer
//!   `status_changed_at` ([`status_write_applies`]).
//! - `gitlab_runner_id` is `COALESCE(stored, incoming)`: never cleared or
//!   overwritten. `tag_list` is replaced only by non-empty tags. Both are
//!   filled on a rejected status write when the stored value is empty and the
//!   row is not terminal.
//! - A terminal row never changes in any column. `updated_at` is `now_ms`
//!   whenever anything changed; `pool_id` is never written.
//! - Fleet health (#197, D-1): `job_name` and `project_path` are
//!   `COALESCE(stored, incoming)`, filled like the runner id (also on a
//!   rejected status write of a row that is not terminal). `started_at` is
//!   set once, to the status time of the first applied `running` write;
//!   `completed_at` once, to the status time of the first applied terminal
//!   write. `category` is bound on insert only
//!   ([`category_for_insert`](crate::fleet_health::classifier_cache::category_for_insert))
//!   and never written by an update (W-15): only #190's backfill or
//!   reclassify changes it.
//!
//! Callers own transactions, `retry_when_busy`, `runner_events` rows, and
//! when a runner's busy flag changes.

use sqlx::SqliteConnection;

use super::JobStatus;
use crate::{
    fleet::ExternalId,
    fleet_health::{
        JobCategory,
        classifier_cache::{JobOrigin, NewJob, category_for_insert},
    },
};

/// The tier of a concluded job: success, failed, canceled or skipped.
const TERMINAL_TIER: u8 = 3;

/// How often [`record_gitlab_job`] reads the row again when another writer
/// changed it between the read and the write, before it gives up.
const CONCURRENT_WRITE_ATTEMPTS: usize = 5;

/// One writer's view of a GitLab job at `status_changed_at`.
pub struct JobObservation<'a> {
    pub connection_id: &'a str,
    pub project_id: ExternalId,
    pub job_id: ExternalId,
    pub pipeline_id: ExternalId,
    pub status: JobStatus,
    /// Epoch milliseconds: when the writer saw this status.
    pub status_changed_at: i64,
    /// `None` keeps the stored value (0 on insert).
    pub retries_count: Option<u32>,
    /// `Some` only when the writer has GitLab's job tags.
    pub tag_list: Option<&'a [String]>,
    /// `Some` only when the writer knows the runner.
    pub gitlab_runner_id: Option<ExternalId>,
    /// GitLab's job name (`build_name` in a webhook, `name` in the API),
    /// when the writer has it.
    pub job_name: Option<&'a str>,
    /// The project's `path_with_namespace`, when the writer has it.
    pub project_path: Option<&'a str>,
}

/// What [`record_gitlab_job`] did.
pub enum JobWrite {
    /// No row existed; one was created.
    Inserted,
    /// The status write applied (and any fills with it).
    StatusApplied,
    /// The status write was rejected, but an empty runner id, tag list, job
    /// name or project path was filled.
    FilledOnly,
    /// Nothing was written.
    Ignored,
}

pub struct JobWriteOutcome {
    pub write: JobWrite,
    /// The stored status before the call; `None` when the row was inserted
    /// (or, for `unknown`, when there was no row).
    pub previous_status: Option<String>,
    /// The stored status after the call.
    pub status: String,
    /// The stored runner id after the call.
    pub gitlab_runner_id: Option<ExternalId>,
    /// The stored runner id before the call; `None` when the call inserted
    /// the row (or there was none).
    pub previous_gitlab_runner_id: Option<ExternalId>,
}

/// The D18 tier of a stored or incoming status; `None` for `unknown` (and any
/// spelling this build does not know).
pub fn status_tier(status: &str) -> Option<u8> {
    match status {
        "created" | "waiting_for_resource" | "preparing" | "manual" | "scheduled" => Some(0),
        "pending" => Some(1),
        "running" => Some(2),
        "success" | "failed" | "canceled" | "skipped" => Some(TERMINAL_TIER),
        _ => None,
    }
}

/// Whether `incoming` at `incoming_at` replaces `stored_status` written at
/// `stored_at`: never onto a terminal row and never for `unknown`; otherwise
/// a higher tier always applies, and the same tier only when strictly newer.
/// A stored status with no tier ranks below every known one.
pub fn status_write_applies(
    stored_status: &str,
    stored_at: i64,
    incoming: JobStatus,
    incoming_at: i64,
) -> bool {
    let Some(incoming_tier) = status_tier(incoming.as_str()) else {
        return false;
    };
    match status_tier(stored_status) {
        Some(TERMINAL_TIER) => false,
        Some(stored_tier) => {
            incoming_tier > stored_tier || (incoming_tier == stored_tier && incoming_at > stored_at)
        }
        None => true,
    }
}

/// `gitlab_jobs.status`'s tier in SQL, as [`status_tier`] ranks it, with -1
/// for a status that has none.
macro_rules! stored_tier {
    () => {
        "(CASE gitlab_jobs.status
            WHEN 'created' THEN 0 WHEN 'waiting_for_resource' THEN 0 WHEN 'preparing' THEN 0
            WHEN 'manual' THEN 0 WHEN 'scheduled' THEN 0
            WHEN 'pending' THEN 1 WHEN 'running' THEN 2
            WHEN 'success' THEN 3 WHEN 'failed' THEN 3 WHEN 'canceled' THEN 3 WHEN 'skipped' THEN 3
            ELSE -1 END)"
    };
}

/// [`status_write_applies`] in SQL: `?12` is the incoming tier.
macro_rules! status_applies {
    () => {
        concat!(
            "(?12>",
            stored_tier!(),
            " OR (?12=",
            stored_tier!(),
            " AND excluded.status_changed_at>gitlab_jobs.status_changed_at))"
        )
    };
}

/// The one statement of a write. Parameters: ?1 id, ?2 connection, ?3
/// project, ?4 job, ?5 pipeline, ?6 status, ?7 tags (JSON), ?8 status time,
/// ?9 retries (given or 0), ?10 runner, ?11 now, ?12 incoming tier, ?13 tags
/// given and non-empty, ?14 retries given, ?15 and ?16 the stored status and
/// status time the caller read (NULL when it read no row), ?17 the category
/// to insert with, ?18 the job name, ?19 the project path.
///
/// The D18 rules are all in the statement, so it is correct on its own. The
/// last two terms of its `WHERE` also make it change nothing when another
/// writer changed the row's status since the caller read it, so the outcome
/// reported for the write is the one that happened.
const UPSERT: &str = concat!(
    "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,tag_list,
       status_changed_at,retries_count,gitlab_runner_id,pool_id,created_at,updated_at,
       category,job_name,project_path,started_at,completed_at)
     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,NULL,?11,?11,?17,?18,?19,
       CASE WHEN ?12=2 THEN ?8 END,CASE WHEN ?12=3 THEN ?8 END)
     ON CONFLICT(connection_id,job_id) DO UPDATE SET
       status=CASE WHEN ",
    status_applies!(),
    " THEN excluded.status ELSE gitlab_jobs.status END,
       status_changed_at=CASE WHEN ",
    status_applies!(),
    " THEN excluded.status_changed_at ELSE gitlab_jobs.status_changed_at END,
       pipeline_id=CASE WHEN ",
    status_applies!(),
    " THEN excluded.pipeline_id ELSE gitlab_jobs.pipeline_id END,
       retries_count=CASE WHEN ?14 AND ",
    status_applies!(),
    " THEN excluded.retries_count ELSE gitlab_jobs.retries_count END,
       tag_list=CASE WHEN ?13 AND (gitlab_jobs.tag_list='[]' OR ",
    status_applies!(),
    ") THEN excluded.tag_list ELSE gitlab_jobs.tag_list END,
       gitlab_runner_id=COALESCE(gitlab_jobs.gitlab_runner_id,excluded.gitlab_runner_id),
       job_name=COALESCE(gitlab_jobs.job_name,excluded.job_name),
       project_path=COALESCE(gitlab_jobs.project_path,excluded.project_path),
       started_at=COALESCE(gitlab_jobs.started_at,CASE WHEN ?12=2 AND ",
    status_applies!(),
    " THEN excluded.status_changed_at END),
       completed_at=COALESCE(gitlab_jobs.completed_at,CASE WHEN ?12=3 AND ",
    status_applies!(),
    " THEN excluded.status_changed_at END),
       updated_at=excluded.updated_at
     WHERE ",
    stored_tier!(),
    "<3
       AND (",
    status_applies!(),
    "
         OR (gitlab_jobs.gitlab_runner_id IS NULL AND excluded.gitlab_runner_id IS NOT NULL)
         OR (?13 AND gitlab_jobs.tag_list='[]')
         OR (gitlab_jobs.job_name IS NULL AND excluded.job_name IS NOT NULL)
         OR (gitlab_jobs.project_path IS NULL AND excluded.project_path IS NOT NULL))
       AND gitlab_jobs.status=?15 AND gitlab_jobs.status_changed_at=?16
     RETURNING status,gitlab_runner_id"
);

#[derive(sqlx::FromRow)]
struct StoredJob {
    status: String,
    status_changed_at: i64,
    tag_list: String,
    gitlab_runner_id: Option<i64>,
    job_name: Option<String>,
    project_path: Option<String>,
}

/// Records `obs` under the D18 rules (see the module docs) with one guarded
/// `INSERT ... ON CONFLICT(connection_id, job_id) DO UPDATE ... WHERE`.
///
/// The stored row is read first only to report what the write did: if
/// another writer changes the row's status between that read and the write,
/// the write changes nothing and the row is read again.
pub async fn record_gitlab_job(
    conn: &mut SqliteConnection,
    obs: &JobObservation<'_>,
    now_ms: i64,
) -> sqlx::Result<JobWriteOutcome> {
    let project_id = sql_integer(obs.project_id)?;
    let job_id = sql_integer(obs.job_id)?;
    let pipeline_id = sql_integer(obs.pipeline_id)?;
    let gitlab_runner_id = obs.gitlab_runner_id.map(sql_integer).transpose()?;
    let tags = obs.tag_list.filter(|tags| !tags.is_empty());
    let tag_list = match tags {
        Some(tags) => contract_tag_list(tags)?,
        None => "[]".to_owned(),
    };

    for _ in 0..CONCURRENT_WRITE_ATTEMPTS {
        let stored = sqlx::query_as::<_, StoredJob>(
            "SELECT status,status_changed_at,tag_list,gitlab_runner_id,job_name,project_path
               FROM gitlab_jobs WHERE connection_id=? AND job_id=?",
        )
        .bind(obs.connection_id)
        .bind(job_id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some(incoming_tier) = status_tier(obs.status.as_str()) else {
            return Ok(ignored(stored, obs));
        };
        let mut category = None;
        let write = match &stored {
            None => {
                category = category_for_insert(
                    conn,
                    &NewJob {
                        job_name: obs.job_name,
                        labels: obs.tag_list.unwrap_or_default(),
                        origin: JobOrigin::GitLabProject(obs.project_path),
                    },
                    now_ms,
                )
                .await;
                JobWrite::Inserted
            }
            Some(row) => {
                if status_tier(&row.status) == Some(TERMINAL_TIER) {
                    return Ok(ignored(stored, obs));
                }
                let fills_runner = row.gitlab_runner_id.is_none() && gitlab_runner_id.is_some();
                let fills_tags = tags.is_some() && row.tag_list == "[]";
                let fills_name = row.job_name.is_none() && obs.job_name.is_some();
                let fills_path = row.project_path.is_none() && obs.project_path.is_some();
                if status_write_applies(
                    &row.status,
                    row.status_changed_at,
                    obs.status,
                    obs.status_changed_at,
                ) {
                    JobWrite::StatusApplied
                } else if fills_runner || fills_tags || fills_name || fills_path {
                    JobWrite::FilledOnly
                } else {
                    return Ok(ignored(stored, obs));
                }
            }
        };
        let written = sqlx::query_as::<_, (String, Option<i64>)>(UPSERT)
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(obs.connection_id)
            .bind(project_id)
            .bind(job_id)
            .bind(pipeline_id)
            .bind(obs.status.as_str())
            .bind(&tag_list)
            .bind(obs.status_changed_at)
            .bind(i64::from(obs.retries_count.unwrap_or(0)))
            .bind(gitlab_runner_id)
            .bind(now_ms)
            .bind(i64::from(incoming_tier))
            .bind(tags.is_some())
            .bind(obs.retries_count.is_some())
            .bind(stored.as_ref().map(|row| row.status.as_str()))
            .bind(stored.as_ref().map(|row| row.status_changed_at))
            .bind(category.map(JobCategory::as_str))
            .bind(obs.job_name)
            .bind(obs.project_path)
            .fetch_optional(&mut *conn)
            .await?;
        if let Some((status, gitlab_runner_id)) = written {
            return Ok(JobWriteOutcome {
                write,
                previous_gitlab_runner_id: stored
                    .as_ref()
                    .and_then(|row| external_id(row.gitlab_runner_id)),
                previous_status: stored.map(|row| row.status),
                status,
                gitlab_runner_id: external_id(gitlab_runner_id),
            });
        }
        // Another writer inserted the row, or changed its status, after the
        // read: read it again and decide on what it holds now.
    }
    Err(sqlx::Error::Protocol(format!(
        "gitlab_jobs row {}/{} kept changing while it was being written",
        obs.connection_id,
        obs.job_id.get()
    )))
}

/// Sets the busy flag of the live GitLab runner `gitlab_runner_id` on the
/// GitLab instance of `connection_id`: GitLab runner ids are unique per
/// instance, so the runner may have been registered through any connection
/// with the same `base_url` (a project connection and its group's). Writes
/// only `busy`, `status` (`busy` or `online`), `last_heartbeat_at` and
/// `updated_at`, and answers the runner's id and pool, or `None` when no live
/// GitLab runner has that id there.
pub async fn set_gitlab_runner_busy(
    conn: &mut SqliteConnection,
    connection_id: &str,
    gitlab_runner_id: ExternalId,
    busy: bool,
    now_ms: i64,
) -> sqlx::Result<Option<(String, String)>> {
    sqlx::query_as::<_, (String, String)>(
        "UPDATE runners
         SET busy=?1,status=CASE WHEN ?1 THEN 'busy' ELSE 'online' END,last_heartbeat_at=?2,updated_at=?2
         WHERE id=(
           SELECT runner.id FROM runners runner
           JOIN gitlab_connections runner_connection ON runner_connection.id=runner.gitlab_connection_id
           WHERE runner.gitlab_runner_id=?3 AND runner.deleted_at IS NULL AND runner.ci_platform='gitlab'
             AND runner_connection.base_url=(SELECT base_url FROM gitlab_connections WHERE id=?4)
           ORDER BY runner.created_at DESC,runner.id LIMIT 1)
         RETURNING id,pool_id",
    )
    .bind(busy)
    .bind(now_ms)
    .bind(sql_integer(gitlab_runner_id)?)
    .bind(connection_id)
    .fetch_optional(&mut *conn)
    .await
}

/// The outcome of a write that changed nothing.
fn ignored(stored: Option<StoredJob>, obs: &JobObservation<'_>) -> JobWriteOutcome {
    match stored {
        Some(row) => JobWriteOutcome {
            write: JobWrite::Ignored,
            previous_status: Some(row.status.clone()),
            status: row.status,
            gitlab_runner_id: external_id(row.gitlab_runner_id),
            previous_gitlab_runner_id: external_id(row.gitlab_runner_id),
        },
        None => JobWriteOutcome {
            write: JobWrite::Ignored,
            previous_status: None,
            status: obs.status.as_str().to_owned(),
            gitlab_runner_id: None,
            previous_gitlab_runner_id: None,
        },
    }
}

/// The contract `tag_list` encoding: `serde_json::to_string(&Vec<String>)`.
fn contract_tag_list(tags: &[String]) -> sqlx::Result<String> {
    serde_json::to_string(tags).map_err(|error| sqlx::Error::Encode(Box::new(error)))
}

/// A GitLab id as the SQLite integer it is stored as.
fn sql_integer(value: ExternalId) -> sqlx::Result<i64> {
    i64::try_from(u64::from(value)).map_err(|error| sqlx::Error::Encode(Box::new(error)))
}

/// A stored GitLab id; `None` for NULL (or a value no GitLab id can be).
fn external_id(value: Option<i64>) -> Option<ExternalId> {
    value
        .and_then(|id| u64::try_from(id).ok())
        .and_then(|id| ExternalId::new(id).ok())
}
