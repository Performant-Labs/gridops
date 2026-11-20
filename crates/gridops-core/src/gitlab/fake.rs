//! An in-memory [`GitLabApi`] for tests (#137 A-3), built with `cfg(test)` or
//! the `test-support` feature so other crates' tests can use it.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use async_trait::async_trait;
use secrecy::SecretString;
use url::Url;

use super::{
    CreatedRunner, GitLabApi, GitLabCredentials, GitLabError, Job, JobStatus, Scope, ScopeKind,
};
use crate::fleet::ExternalId;

/// One variant per [`GitLabApi`] method, for [`FakeGitLab::fail_next`] and
/// the call log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FakeMethod {
    CreateRunner,
    DeleteRunner,
    Job,
    PendingJobs,
    GroupProjects,
    ResolveScope,
    VerifyAccess,
}

/// One recorded call. The access token is never recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeCall {
    pub base_url: Url,
    pub method: FakeMethod,
}

/// A live runner on a fake instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeRunner {
    pub id: ExternalId,
    pub scope: Scope,
    pub tags: Vec<String>,
    pub run_untagged: bool,
    pub description: String,
}

/// The state of one GitLab instance (one `base_url`).
#[derive(Default)]
struct Instance {
    scopes: Vec<Scope>,
    jobs: Vec<Job>,
    group_projects: Vec<(ExternalId, ExternalId)>,
    runners: Vec<FakeRunner>,
    last_runner_id: u64,
    failures: HashMap<FakeMethod, VecDeque<GitLabError>>,
}

#[derive(Default)]
struct State {
    instances: HashMap<Url, Instance>,
    calls: Vec<FakeCall>,
}

/// A cloneable in-memory GitLab. Every clone shares one state.
///
/// Its limits, which tests built on it must respect:
/// - all state (scopes, jobs, group projects, runners, the runner-id counter
///   and queued failures) is keyed by `GitLabCredentials.base_url` alone, so
///   two connections on the same base URL share one instance's state, as two
///   tokens on one real GitLab server do;
/// - the access token is never inspected or recorded, so token-level
///   permission differences can only be simulated with [`Self::fail_next`];
/// - tests that need two connections isolated must give them different base
///   URLs. A test that needs per-token isolation on one base URL extends the
///   fake in its own issue.
#[derive(Clone, Default)]
pub struct FakeGitLab {
    state: Arc<Mutex<State>>,
}

impl FakeGitLab {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Seeds a project or group, for `resolve_scope` and `verify_access`.
    pub fn with_scope(&self, base_url: &Url, scope: Scope) {
        self.state()
            .instances
            .entry(base_url.clone())
            .or_default()
            .scopes
            .push(scope);
    }

    /// Seeds a job, for `job` and, when it is `Pending`, `pending_jobs`.
    pub fn with_job(&self, base_url: &Url, job: Job) {
        self.state()
            .instances
            .entry(base_url.clone())
            .or_default()
            .jobs
            .push(job);
    }

    /// Seeds a project of a group, for `group_projects`.
    pub fn with_group_project(&self, base_url: &Url, group_id: ExternalId, project_id: ExternalId) {
        self.state()
            .instances
            .entry(base_url.clone())
            .or_default()
            .group_projects
            .push((group_id, project_id));
    }

    /// Queues a one-shot failure for the next `method` call on `base_url`,
    /// whatever its token. Queued failures for one method come back first in,
    /// first out, and a failed call has no other effect.
    pub fn fail_next(&self, base_url: &Url, method: FakeMethod, error: GitLabError) {
        self.state()
            .instances
            .entry(base_url.clone())
            .or_default()
            .failures
            .entry(method)
            .or_default()
            .push_back(error);
    }

    /// The live runners on `base_url`.
    #[must_use]
    pub fn runners(&self, base_url: &Url) -> Vec<FakeRunner> {
        self.state()
            .instances
            .get(base_url)
            .map(|instance| instance.runners.clone())
            .unwrap_or_default()
    }

    /// Every call made on any clone, in invocation order, failed ones included.
    #[must_use]
    pub fn calls(&self) -> Vec<FakeCall> {
        self.state().calls.clone()
    }

    /// Records the call, then either returns its queued failure or runs `act`
    /// on the instance's state.
    fn call<T>(
        &self,
        credentials: &GitLabCredentials,
        method: FakeMethod,
        act: impl FnOnce(&mut Instance) -> Result<T, GitLabError>,
    ) -> Result<T, GitLabError> {
        let mut state = self.state();
        state.calls.push(FakeCall {
            base_url: credentials.base_url.clone(),
            method,
        });
        let instance = state
            .instances
            .entry(credentials.base_url.clone())
            .or_default();
        if let Some(error) = instance
            .failures
            .get_mut(&method)
            .and_then(VecDeque::pop_front)
        {
            return Err(error);
        }
        act(instance)
    }
}

#[async_trait]
impl GitLabApi for FakeGitLab {
    async fn create_runner(
        &self,
        credentials: &GitLabCredentials,
        scope: &Scope,
        tags: &[String],
        run_untagged: bool,
        description: &str,
    ) -> Result<CreatedRunner, GitLabError> {
        self.call(credentials, FakeMethod::CreateRunner, |instance| {
            let next = instance.last_runner_id + 1;
            let id =
                ExternalId::new(next).map_err(|error| GitLabError::Upstream(error.to_string()))?;
            instance.last_runner_id = next;
            instance.runners.push(FakeRunner {
                id,
                scope: scope.clone(),
                tags: tags.to_vec(),
                run_untagged,
                description: description.to_owned(),
            });
            Ok(CreatedRunner {
                id,
                token: SecretString::from(format!("glrt-fake-runner-token-{next:04}")),
            })
        })
    }

    async fn delete_runner(
        &self,
        credentials: &GitLabCredentials,
        runner_id: ExternalId,
    ) -> Result<(), GitLabError> {
        self.call(credentials, FakeMethod::DeleteRunner, |instance| {
            let before = instance.runners.len();
            instance.runners.retain(|runner| runner.id != runner_id);
            if instance.runners.len() == before {
                Err(GitLabError::NotFound)
            } else {
                Ok(())
            }
        })
    }

    async fn job(
        &self,
        credentials: &GitLabCredentials,
        project_id: ExternalId,
        job_id: ExternalId,
    ) -> Result<Job, GitLabError> {
        self.call(credentials, FakeMethod::Job, |instance| {
            instance
                .jobs
                .iter()
                .find(|job| job.id == job_id && job.project_id == project_id)
                .cloned()
                .ok_or(GitLabError::NotFound)
        })
    }

    async fn pending_jobs(
        &self,
        credentials: &GitLabCredentials,
        project_id: ExternalId,
    ) -> Result<Vec<Job>, GitLabError> {
        self.call(credentials, FakeMethod::PendingJobs, |instance| {
            Ok(instance
                .jobs
                .iter()
                .filter(|job| job.project_id == project_id && job.status == JobStatus::Pending)
                .cloned()
                .collect())
        })
    }

    async fn group_projects(
        &self,
        credentials: &GitLabCredentials,
        group_id: ExternalId,
    ) -> Result<Vec<ExternalId>, GitLabError> {
        self.call(credentials, FakeMethod::GroupProjects, |instance| {
            Ok(instance
                .group_projects
                .iter()
                .filter(|(group, _)| *group == group_id)
                .map(|(_, project)| *project)
                .collect())
        })
    }

    async fn resolve_scope(
        &self,
        credentials: &GitLabCredentials,
        kind: ScopeKind,
        path: &str,
    ) -> Result<Scope, GitLabError> {
        self.call(credentials, FakeMethod::ResolveScope, |instance| {
            instance
                .scopes
                .iter()
                .find(|scope| scope.kind() == kind && scope.path() == path)
                .cloned()
                .ok_or(GitLabError::NotFound)
        })
    }

    async fn verify_access(
        &self,
        credentials: &GitLabCredentials,
        scope: &Scope,
    ) -> Result<(), GitLabError> {
        self.call(credentials, FakeMethod::VerifyAccess, |instance| {
            if instance
                .scopes
                .iter()
                .any(|seeded| seeded.kind() == scope.kind() && seeded.id() == scope.id())
            {
                Ok(())
            } else {
                Err(GitLabError::NotFound)
            }
        })
    }
}

#[cfg(test)]
#[path = "fake_tests.rs"]
mod tests;
