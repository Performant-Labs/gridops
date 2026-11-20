//! The GitLab client contract (#137): the types every GitLab issue shares and
//! the [`GitLabApi`] trait the real client (#146) implements.
//!
//! One `Arc<dyn GitLabApi>` serves every connection: each call takes that
//! connection's [`GitLabCredentials`], the way `BitbucketClient` takes its
//! token on each call. [`fake::FakeGitLab`] is the in-memory implementation
//! the other crates' tests use.

use std::{fmt, time::Duration};

use async_trait::async_trait;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::fleet::{CiTarget, ExternalId, GitlabConnectionId};

#[cfg(any(test, feature = "test-support"))]
pub mod fake;

/// The GitLab instance and the access token one connection calls it with.
pub struct GitLabCredentials {
    pub base_url: Url,
    pub access_token: SecretString,
}

impl fmt::Debug for GitLabCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitLabCredentials")
            .field("base_url", &self.base_url.as_str())
            .field("access_token", &"[redacted]")
            .finish()
    }
}

/// Whether a connection registers runners for one project or a whole group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    Project,
    Group,
}

impl ScopeKind {
    /// The `gitlab_connections.scope_kind` spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Group => "group",
        }
    }
}

/// A resolved GitLab project or group: `id` is `gitlab_connections.scope_id`
/// and `path` is `scope_path` (for example `group/sub/project`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Project { id: ExternalId, path: String },
    Group { id: ExternalId, path: String },
}

impl Scope {
    #[must_use]
    pub const fn kind(&self) -> ScopeKind {
        match self {
            Self::Project { .. } => ScopeKind::Project,
            Self::Group { .. } => ScopeKind::Group,
        }
    }

    #[must_use]
    pub const fn id(&self) -> ExternalId {
        match self {
            Self::Project { id, .. } | Self::Group { id, .. } => *id,
        }
    }

    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::Project { path, .. } | Self::Group { path, .. } => path,
        }
    }

    /// The fleet target this scope is for, on connection `connection_id`.
    #[must_use]
    pub fn target(&self, connection_id: GitlabConnectionId) -> CiTarget {
        match self {
            Self::Project { id, .. } => CiTarget::GitlabProject {
                connection_id,
                project_id: *id,
            },
            Self::Group { id, .. } => CiTarget::GitlabGroup {
                connection_id,
                group_id: *id,
            },
        }
    }
}

/// A runner GitLab created, with its secret `glrt-` authentication token.
pub struct CreatedRunner {
    pub id: ExternalId,
    pub token: SecretString,
}

impl fmt::Debug for CreatedRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreatedRunner")
            .field("id", &self.id)
            .field("token", &"[redacted]")
            .finish()
    }
}

/// A GitLab CI job. `id` is the GitLab job id (`gitlab_jobs.job_id`), not the
/// `gitlab_jobs.id` surrogate key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: ExternalId,
    pub project_id: ExternalId,
    pub pipeline_id: ExternalId,
    pub status: JobStatus,
    pub tag_list: Vec<String>,
}

/// A GitLab job status, in GitLab's spelling. A status GitLab adds later
/// deserializes as [`JobStatus::Unknown`] and is stored as `"unknown"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Created,
    WaitingForResource,
    Preparing,
    Pending,
    Running,
    Success,
    Failed,
    Canceled,
    Skipped,
    Manual,
    Scheduled,
    #[serde(other)]
    Unknown,
}

/// The `gitlab_jobs.status` values of active jobs, in the order of the
/// `gitlab_jobs_active_created_idx` predicate in `migrations/0046_gitlab.sql`,
/// which must match this list.
pub const GITLAB_ACTIVE_JOB_STATUSES: [&str; 5] = [
    "created",
    "waiting_for_resource",
    "preparing",
    "pending",
    "running",
];

impl JobStatus {
    /// The GitLab spelling, stored in `gitlab_jobs.status`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::WaitingForResource => "waiting_for_resource",
            Self::Preparing => "preparing",
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Skipped => "skipped",
            Self::Manual => "manual",
            Self::Scheduled => "scheduled",
            Self::Unknown => "unknown",
        }
    }

    /// The job is concluded: the single conclusion rule #140 and #148 share.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Success | Self::Failed | Self::Canceled | Self::Skipped
        )
    }

    /// The job needs a runner now, or holds one. `Manual`, `Scheduled` and
    /// `Unknown` are neither active nor terminal.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::Created
                | Self::WaitingForResource
                | Self::Preparing
                | Self::Pending
                | Self::Running
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitLabError {
    #[error("GitLab rejected the access token")]
    Unauthorized,
    /// The access token lacks this scope, for example `create_runner`.
    #[error("the GitLab access token is missing the {0} scope")]
    MissingScope(String),
    #[error("the GitLab access token's user lacks the required role")]
    MissingRole,
    #[error("GitLab could not find the resource")]
    NotFound,
    /// `retry_after` is taken from a `Retry-After` header in seconds.
    #[error("GitLab rate limited the request")]
    RateLimited { retry_after: Option<Duration> },
    #[error("GitLab request failed: {0}")]
    Upstream(String),
}

/// The GitLab REST calls `GridOps` makes. The real client is #146's.
#[async_trait]
pub trait GitLabApi: Send + Sync {
    async fn create_runner(
        &self,
        credentials: &GitLabCredentials,
        scope: &Scope,
        tags: &[String],
        run_untagged: bool,
        description: &str,
    ) -> Result<CreatedRunner, GitLabError>;

    async fn delete_runner(
        &self,
        credentials: &GitLabCredentials,
        runner_id: ExternalId,
    ) -> Result<(), GitLabError>;

    async fn job(
        &self,
        credentials: &GitLabCredentials,
        project_id: ExternalId,
        job_id: ExternalId,
    ) -> Result<Job, GitLabError>;

    async fn pending_jobs(
        &self,
        credentials: &GitLabCredentials,
        project_id: ExternalId,
    ) -> Result<Vec<Job>, GitLabError>;

    /// The projects of a group, subgroups included, so a group connection can
    /// be polled project by project (GitLab has no group-level jobs list).
    async fn group_projects(
        &self,
        credentials: &GitLabCredentials,
        group_id: ExternalId,
    ) -> Result<Vec<ExternalId>, GitLabError>;

    async fn resolve_scope(
        &self,
        credentials: &GitLabCredentials,
        kind: ScopeKind,
        path: &str,
    ) -> Result<Scope, GitLabError>;

    async fn verify_access(
        &self,
        credentials: &GitLabCredentials,
        scope: &Scope,
    ) -> Result<(), GitLabError>;
}
