//! The GitLab pending-jobs poll (#148): a missed webhook must not lose a job.
//!
//! On its own schedule, beside the pass, for each GitLab connection a pool
//! targets it lists the pending jobs of the connection's project (or of each
//! project of its group) and records them, then asks GitLab about a few of
//! the connection's recorded active jobs it no longer lists as pending, and
//! records how they stand. Every `gitlab_jobs` write goes through D18's
//! shared writer ([`record_gitlab_job`]), so the poll and the webhook (#140)
//! apply one status rule: a status only moves forward, and a finished job
//! never changes.

use std::{collections::HashMap, future::Future, time::Instant};

use gridops_core::fleet::ExternalId;
use gridops_core::gitlab::jobs::{
    JobObservation, JobWrite, record_gitlab_job, set_gitlab_runner_busy,
};
use gridops_core::gitlab::{GitLabCredentials, GitLabError, Job, JobStatus, ScopeKind};

use super::{
    Duration, FromRow, Reconciler, Result, gitlab_credentials, json, now_millis, retry_when_busy,
    setting_i64,
};

/// How long a recorded active job may go unwritten (by the webhook, the
/// poll's pending list or an earlier check) before the poll asks GitLab about
/// it, and how long before it asks again: GitHub's job check's
/// `JOB_CHECK_AGE_MS`. Measured on `gitlab_jobs.updated_at`.
pub(crate) const GITLAB_JOB_CHECK_AGE_MS: i64 = 5 * 60 * 1_000;

/// Most recorded jobs the poll asks GitLab about per connection per poll, one
/// request each: GitHub's job check's `JOB_CHECK_LIMIT`.
pub(crate) const GITLAB_JOB_CHECK_LIMIT: i64 = 10;

/// How long one GitLab request may take. A request that takes longer is
/// treated as failed: its project is not polled this time, or its job is
/// checked again later.
pub(crate) const GITLAB_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a rate-limited connection is skipped when GitLab sends no
/// `Retry-After`.
pub(crate) const GITLAB_RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(60);

/// The longest a `Retry-After` is honoured, so a wild value cannot stop a
/// connection's polling for days.
const GITLAB_RATE_LIMIT_MAX_BACKOFF: Duration = Duration::from_hours(1);

/// Polls every connection, then waits for the reconcile interval (the
/// `reconcileIntervalSeconds` setting the pass uses), for ever. The
/// rate-limit backoff lives here, between polls.
pub(crate) async fn worker(app: Reconciler) {
    let mut backoff = HashMap::new();
    loop {
        if let Err(error) = poll_once(&app, &mut backoff, Instant::now()).await {
            tracing::warn!(error = ?error, "GitLab pending-jobs poll failed");
        }
        let interval = setting_i64(&app.database, "reconcileIntervalSeconds", 30)
            .await
            .clamp(5, 3_600);
        tokio::time::sleep(Duration::from_secs(interval.unsigned_abs())).await;
    }
}

#[derive(Debug, FromRow)]
struct PolledConnection {
    id: String,
    scope_kind: String,
    scope_id: i64,
}

/// One poll of every GitLab connection a pool targets. A connection in
/// `backoff` until after `now` is skipped; a connection GitLab rate-limits is
/// put there until its `Retry-After` (or [`GITLAB_RATE_LIMIT_BACKOFF`]) has
/// passed. Any other failure is logged for its connection and the poll goes
/// on to the next one.
pub(crate) async fn poll_once(
    app: &Reconciler,
    backoff: &mut HashMap<String, Instant>,
    now: Instant,
) -> Result<()> {
    backoff.retain(|_, until| *until > now);
    let connections = sqlx::query_as::<_, PolledConnection>(
        "SELECT connection.id,connection.scope_kind,connection.scope_id FROM gitlab_connections connection
         WHERE EXISTS (SELECT 1 FROM runner_pool_gitlab_connections link WHERE link.connection_id=connection.id)
         ORDER BY connection.id",
    )
    .fetch_all(&app.database)
    .await?;
    for connection in connections {
        if backoff.contains_key(&connection.id) {
            tracing::debug!(connection_id = %connection.id, "GitLab rate limit: skipping the connection");
            continue;
        }
        match poll_connection(app, &connection).await {
            Ok(()) => {}
            Err(Interrupted::RateLimited(retry_after)) => {
                let wait = retry_after
                    .unwrap_or(GITLAB_RATE_LIMIT_BACKOFF)
                    .min(GITLAB_RATE_LIMIT_MAX_BACKOFF);
                tracing::info!(connection_id = %connection.id, wait_seconds = wait.as_secs(), "GitLab rate-limited the connection; skipping it until the wait is over");
                backoff.insert(connection.id, now + wait);
            }
            Err(Interrupted::Failed(error)) => {
                tracing::warn!(connection_id = %connection.id, error = ?error, "GitLab pending-jobs poll of a connection failed");
            }
        }
    }
    Ok(())
}

/// Why a connection's poll stopped early.
enum Interrupted {
    /// GitLab rate-limited a request: nothing more is asked of this
    /// connection this time.
    RateLimited(Option<Duration>),
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for Interrupted {
    fn from(error: anyhow::Error) -> Self {
        Self::Failed(error)
    }
}

impl From<sqlx::Error> for Interrupted {
    fn from(error: sqlx::Error) -> Self {
        Self::Failed(error.into())
    }
}

/// Polls one connection: records its pending jobs, then checks its stale
/// recorded ones.
async fn poll_connection(
    app: &Reconciler,
    connection: &PolledConnection,
) -> Result<(), Interrupted> {
    let Some(credentials) = gitlab_credentials(app, &connection.id).await? else {
        tracing::warn!(connection_id = %connection.id, "GitLab connection has no access token; not polling it");
        return Ok(());
    };
    // The time the pending answers are written at, taken before asking, so a
    // webhook status written after the answer was given is newer than it.
    let observed_at = now_millis();
    let scope_id = ExternalId::new(u64::try_from(connection.scope_id).unwrap_or_default())
        .map_err(|error| anyhow::anyhow!("GitLab connection has an invalid scope id: {error}"))?;
    let projects = if connection.scope_kind == ScopeKind::Group.as_str() {
        match request(app.gitlab.group_projects(&credentials, scope_id)).await {
            Ok(projects) => projects,
            Err(GitLabError::RateLimited { retry_after }) => {
                return Err(Interrupted::RateLimited(retry_after));
            }
            // GitLab no longer has the group (or the token can no longer see
            // it): it has no pending jobs, so every recorded job is checked
            // on its own, and those GitLab no longer has are concluded.
            Err(GitLabError::NotFound) => {
                tracing::warn!(connection_id = %connection.id, "GitLab no longer has this connection's group");
                Vec::new()
            }
            Err(error) => {
                // With no project list there is no pending answer at all, so
                // no recorded job can be judged no longer pending.
                tracing::warn!(connection_id = %connection.id, error = %error, "could not list a GitLab group's projects");
                return Ok(());
            }
        }
    } else {
        vec![scope_id]
    };

    let mut listed = Vec::<i64>::new();
    let mut unanswered = Vec::<i64>::new();
    for project in projects {
        match request(app.gitlab.pending_jobs(&credentials, project)).await {
            Ok(jobs) => {
                record_pending(app, &connection.id, &jobs, observed_at).await?;
                listed.extend(jobs.iter().filter_map(|job| sql_id(job.id)));
            }
            Err(GitLabError::RateLimited { retry_after }) => {
                return Err(Interrupted::RateLimited(retry_after));
            }
            // GitLab no longer has the project (or the token can no longer
            // see it): that is an answer, not a missing one, so its recorded
            // jobs are checked one by one like any other.
            Err(GitLabError::NotFound) => {
                tracing::warn!(connection_id = %connection.id, project_id = project.get(), "GitLab no longer has a polled project");
            }
            Err(error) => {
                tracing::warn!(connection_id = %connection.id, project_id = project.get(), error = %error, "could not list a GitLab project's pending jobs");
                unanswered.extend(sql_id(project));
            }
        }
    }
    check_stale_jobs(app, connection, &credentials, &listed, &unanswered).await
}

/// Records the jobs GitLab listed as pending, as seen at `observed_at`, in
/// one transaction.
async fn record_pending(
    app: &Reconciler,
    connection_id: &str,
    jobs: &[Job],
    observed_at: i64,
) -> Result<()> {
    if jobs.is_empty() {
        return Ok(());
    }
    retry_when_busy(|| async move {
        let mut transaction = app.database.begin().await?;
        let now = now_millis();
        for job in jobs {
            record_gitlab_job(
                &mut transaction,
                &observation(connection_id, job, observed_at),
                now,
            )
            .await?;
        }
        transaction.commit().await?;
        anyhow::Ok(())
    })
    .await
}

#[derive(Debug, FromRow)]
struct StaleJob {
    id: String,
    project_id: i64,
    job_id: i64,
    pipeline_id: i64,
}

/// Asks GitLab about the connection's recorded active jobs that it did not
/// list as pending and that nothing has written for
/// [`GITLAB_JOB_CHECK_AGE_MS`], at most [`GITLAB_JOB_CHECK_LIMIT`]: `pending`
/// ones first, then the other waiting ones, then `running` ones, each least
/// recently checked first. Jobs of a project whose pending list could not be
/// read are left alone: they are only missing from an answer that is missing.
/// A project or group GitLab answers `NotFound` for is an answer: its jobs
/// are checked, and each one GitLab no longer has is concluded.
///
/// Each job's `updated_at` is set before it is asked about, so it is not asked
/// about again within the age whatever the answer. The answer is recorded as
/// GitLab gives it; a job GitLab no longer has can never run and is recorded
/// `canceled`.
async fn check_stale_jobs(
    app: &Reconciler,
    connection: &PolledConnection,
    credentials: &GitLabCredentials,
    listed: &[i64],
    unanswered: &[i64],
) -> Result<(), Interrupted> {
    let stale = sqlx::query_as::<_, StaleJob>(
        "SELECT id,project_id,job_id,pipeline_id FROM gitlab_jobs
         WHERE connection_id=? AND status IN ('created','waiting_for_resource','preparing','pending','running')
           AND updated_at<?
           AND job_id NOT IN (SELECT value FROM json_each(?))
           AND project_id NOT IN (SELECT value FROM json_each(?))
         ORDER BY CASE status WHEN 'pending' THEN 0 WHEN 'running' THEN 2 ELSE 1 END,
           updated_at,created_at,id
         LIMIT ?",
    )
    .bind(&connection.id)
    .bind(now_millis() - GITLAB_JOB_CHECK_AGE_MS)
    .bind(json!(listed).to_string())
    .bind(json!(unanswered).to_string())
    .bind(GITLAB_JOB_CHECK_LIMIT)
    .fetch_all(&app.database)
    .await?;
    for row in stale {
        let (Some(project_id), Some(job_id), Some(pipeline_id)) = (
            external_id(row.project_id),
            external_id(row.job_id),
            external_id(row.pipeline_id),
        ) else {
            tracing::warn!(connection_id = %connection.id, job_id = row.job_id, "a recorded GitLab job has an invalid id; not checking it");
            continue;
        };
        let checked_at = now_millis();
        let id = row.id.as_str();
        retry_when_busy(|| async move {
            // Only the check mark, and never on a finished row.
            sqlx::query(
                "UPDATE gitlab_jobs SET updated_at=? WHERE id=?
                   AND status IN ('created','waiting_for_resource','preparing','pending','running')",
            )
            .bind(checked_at)
            .bind(id)
            .execute(&app.database)
            .await?;
            anyhow::Ok(())
        })
        .await?;
        let answer = match request(app.gitlab.job(credentials, project_id, job_id)).await {
            Ok(job) => Some(job),
            // GitLab no longer has the job: it can never run.
            Err(GitLabError::NotFound) => None,
            Err(GitLabError::RateLimited { retry_after }) => {
                return Err(Interrupted::RateLimited(retry_after));
            }
            Err(error) => {
                tracing::warn!(connection_id = %connection.id, job_id = row.job_id, error = %error, "could not check a GitLab job");
                continue;
            }
        };
        let observation = match &answer {
            Some(job) => observation(&connection.id, job, checked_at),
            None => JobObservation {
                connection_id: &connection.id,
                project_id,
                job_id,
                pipeline_id,
                status: JobStatus::Canceled,
                status_changed_at: checked_at,
                retries_count: None,
                tag_list: None,
                gitlab_runner_id: None,
            },
        };
        record_check(app, &observation).await?;
    }
    Ok(())
}

/// Records what GitLab said about a checked job. When that finishes a job
/// that was `running` on a known runner, frees the runner and records a
/// `runner_events` row as the webhook does, in the same transaction.
async fn record_check(app: &Reconciler, checked: &JobObservation<'_>) -> Result<()> {
    let written = retry_when_busy(|| async move {
        let mut transaction = app.database.begin().await?;
        let now = now_millis();
        let outcome = record_gitlab_job(&mut transaction, checked, now).await?;
        let freed = match (&outcome.write, outcome.previous_status.as_deref(), outcome.gitlab_runner_id) {
            (JobWrite::StatusApplied, Some("running"), Some(runner_id))
                if checked.status.is_terminal() =>
            {
                set_gitlab_runner_busy(&mut transaction, checked.connection_id, runner_id, false, now)
                    .await?
            }
            _ => None,
        };
        if let Some((runner_id, pool_id)) = &freed {
            sqlx::query(
                "INSERT INTO runner_events (id,runner_id,pool_id,level,event,message,metadata,created_at)
                 VALUES (?,?,?,'info',?,?,?,?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(runner_id)
            .bind(pool_id)
            .bind(format!("GitLab job {}", checked.status.as_str()))
            .bind(format!("Job · pipeline {}", checked.pipeline_id.get()))
            .bind(
                json!({
                    "jobId": checked.job_id.get(),
                    "projectId": checked.project_id.get(),
                    "pipelineId": checked.pipeline_id.get(),
                    "connectionId": checked.connection_id,
                })
                .to_string(),
            )
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        anyhow::Ok((outcome.status, freed.map(|(runner_id, _)| runner_id)))
    })
    .await?;
    tracing::info!(connection_id = checked.connection_id, job_id = checked.job_id.get(), answer = checked.status.as_str(), status = %written.0, freed_runner = ?written.1, "checked a GitLab job");
    Ok(())
}

/// What the poll saw of `job` on `connection_id` at `seen_at`: its status,
/// pipeline and tags; it never knows the runner or the retry count.
fn observation<'a>(connection_id: &'a str, job: &'a Job, seen_at: i64) -> JobObservation<'a> {
    JobObservation {
        connection_id,
        project_id: job.project_id,
        job_id: job.id,
        pipeline_id: job.pipeline_id,
        status: job.status,
        status_changed_at: seen_at,
        retries_count: None,
        tag_list: Some(&job.tag_list),
        gitlab_runner_id: None,
    }
}

/// `call`, failed as `Upstream` if GitLab has not answered within
/// [`GITLAB_REQUEST_TIMEOUT`].
async fn request<T>(call: impl Future<Output = Result<T, GitLabError>>) -> Result<T, GitLabError> {
    tokio::time::timeout(GITLAB_REQUEST_TIMEOUT, call)
        .await
        .unwrap_or_else(|_| {
            Err(GitLabError::Upstream(format!(
                "GitLab did not answer within {} s",
                GITLAB_REQUEST_TIMEOUT.as_secs()
            )))
        })
}

/// A GitLab id as stored in SQLite.
fn sql_id(id: ExternalId) -> Option<i64> {
    i64::try_from(id.get()).ok()
}

/// A stored GitLab id, if it is one.
fn external_id(value: i64) -> Option<ExternalId> {
    u64::try_from(value)
        .ok()
        .and_then(|value| ExternalId::new(value).ok())
}

#[cfg(test)]
pub(crate) use placeholder::UnavailableGitLab;

#[cfg(test)]
mod placeholder {
    use async_trait::async_trait;
    use gridops_core::fleet::ExternalId;
    use gridops_core::gitlab::{
        CreatedRunner, GitLabApi, GitLabCredentials, GitLabError, Job, Scope, ScopeKind,
    };

    /// A GitLab client that refuses every call as `Upstream`, for tests that need
    /// a reconciler with no GitLab. Production uses #146's `GitLabClient`.
    pub(crate) struct UnavailableGitLab;

    impl UnavailableGitLab {
        fn unavailable() -> GitLabError {
            GitLabError::Upstream("GitLab client not available yet".into())
        }
    }

    #[async_trait]
    impl GitLabApi for UnavailableGitLab {
        async fn create_runner(
            &self,
            _credentials: &GitLabCredentials,
            _scope: &Scope,
            _tags: &[String],
            _run_untagged: bool,
            _description: &str,
        ) -> Result<CreatedRunner, GitLabError> {
            Err(Self::unavailable())
        }

        async fn delete_runner(
            &self,
            _credentials: &GitLabCredentials,
            _runner_id: ExternalId,
        ) -> Result<(), GitLabError> {
            Err(Self::unavailable())
        }

        async fn job(
            &self,
            _credentials: &GitLabCredentials,
            _project_id: ExternalId,
            _job_id: ExternalId,
        ) -> Result<Job, GitLabError> {
            Err(Self::unavailable())
        }

        async fn pending_jobs(
            &self,
            _credentials: &GitLabCredentials,
            _project_id: ExternalId,
        ) -> Result<Vec<Job>, GitLabError> {
            Err(Self::unavailable())
        }

        async fn group_projects(
            &self,
            _credentials: &GitLabCredentials,
            _group_id: ExternalId,
        ) -> Result<Vec<ExternalId>, GitLabError> {
            Err(Self::unavailable())
        }

        async fn resolve_scope(
            &self,
            _credentials: &GitLabCredentials,
            _kind: ScopeKind,
            _path: &str,
        ) -> Result<Scope, GitLabError> {
            Err(Self::unavailable())
        }

        async fn verify_access(
            &self,
            _credentials: &GitLabCredentials,
            _scope: &Scope,
        ) -> Result<(), GitLabError> {
            Err(Self::unavailable())
        }
    }
}
