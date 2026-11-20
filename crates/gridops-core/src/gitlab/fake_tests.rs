//! `FakeGitLab`'s own tests (#137 A-3). Mounted from `gitlab/fake.rs` with
//! `#[cfg(test)] #[path = "fake_tests.rs"] mod tests;`, the way
//! `fleet/admission.rs` mounts `admission_tests.rs`.
//!
//! The fake is what #138, #139, #140 and #148 test against, so these pin what
//! they may rely on: per-instance state keyed by base URL alone, one-shot
//! first-in-first-out failures per method, and an exact call log.

use std::{sync::Arc, time::Duration};

use anyhow::Result;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::{
    fleet::ExternalId,
    gitlab::{
        GitLabApi, GitLabCredentials, GitLabError, Job, JobStatus, Scope, ScopeKind,
        fake::{FakeCall, FakeGitLab, FakeMethod},
    },
};

const INSTANCE_A: &str = "https://gitlab.example.com";
const INSTANCE_B: &str = "https://gitlab.other.example";

fn instance(base: &str) -> Result<Url> {
    Ok(Url::parse(base)?)
}

fn credentials(base: &Url, token: &str) -> GitLabCredentials {
    GitLabCredentials {
        base_url: base.clone(),
        access_token: SecretString::from(token.to_owned()),
    }
}

fn id(value: u64) -> Result<ExternalId> {
    Ok(ExternalId::new(value)?)
}

fn job(job_id: u64, project_id: u64, status: JobStatus) -> Result<Job> {
    Ok(Job {
        id: id(job_id)?,
        project_id: id(project_id)?,
        pipeline_id: id(500)?,
        status,
        tag_list: vec!["docker".to_owned()],
    })
}

fn project(project_id: u64, path: &str) -> Result<Scope> {
    Ok(Scope::Project {
        id: id(project_id)?,
        path: path.to_owned(),
    })
}

#[tokio::test]
async fn runners_are_numbered_per_instance_with_fake_tokens_and_listed() -> Result<()> {
    let fake = FakeGitLab::default();
    let api: Arc<dyn GitLabApi> = Arc::new(fake.clone());
    let (a, b) = (instance(INSTANCE_A)?, instance(INSTANCE_B)?);
    let scope = project(42, "acme/api")?;
    let tags = vec!["docker".to_owned(), "linux".to_owned()];
    let first = api
        .create_runner(&credentials(&a, "glpat-a"), &scope, &tags, false, "first")
        .await?;
    let second = api
        .create_runner(&credentials(&a, "glpat-a"), &scope, &[], true, "second")
        .await?;
    let other = api
        .create_runner(&credentials(&b, "glpat-b"), &scope, &tags, false, "other")
        .await?;
    assert_eq!(
        [
            (first.id, first.token.expose_secret()),
            (second.id, second.token.expose_secret()),
            (other.id, other.token.expose_secret()),
        ],
        [
            (id(1)?, "glrt-fake-runner-token-0001"),
            (id(2)?, "glrt-fake-runner-token-0002"),
            (id(1)?, "glrt-fake-runner-token-0001"),
        ]
    );
    let mut listed: Vec<_> = fake
        .runners(&a)
        .into_iter()
        .map(|runner| {
            (
                runner.id,
                runner.scope,
                runner.tags,
                runner.run_untagged,
                runner.description,
            )
        })
        .collect();
    listed.sort_by_key(|runner| runner.0);
    assert_eq!(
        listed,
        [
            (
                id(1)?,
                scope.clone(),
                tags.clone(),
                false,
                "first".to_owned()
            ),
            (id(2)?, scope.clone(), Vec::new(), true, "second".to_owned()),
        ]
    );
    assert_eq!(fake.runners(&b).len(), 1);
    Ok(())
}

#[tokio::test]
async fn deleting_a_runner_removes_it_and_unknown_runners_are_not_found() -> Result<()> {
    let fake = FakeGitLab::default();
    let (a, b) = (instance(INSTANCE_A)?, instance(INSTANCE_B)?);
    let scope = project(42, "acme/api")?;
    for description in ["first", "second"] {
        fake.create_runner(&credentials(&a, "glpat-a"), &scope, &[], false, description)
            .await?;
    }
    // Another token on the same instance shares its state, as on a real server.
    fake.delete_runner(&credentials(&a, "glpat-another"), id(1)?)
        .await?;
    let remaining: Vec<_> = fake
        .runners(&a)
        .into_iter()
        .map(|runner| runner.id)
        .collect();
    assert_eq!(remaining, [id(2)?]);
    assert_eq!(
        fake.delete_runner(&credentials(&a, "glpat-a"), id(1)?)
            .await
            .err(),
        Some(GitLabError::NotFound)
    );
    // Instances never see each other's runners.
    assert_eq!(
        fake.delete_runner(&credentials(&b, "glpat-b"), id(2)?)
            .await
            .err(),
        Some(GitLabError::NotFound)
    );
    assert_eq!(fake.runners(&a).len(), 1);
    Ok(())
}

#[tokio::test]
async fn jobs_and_pending_jobs_come_from_the_seeded_instance() -> Result<()> {
    let fake = FakeGitLab::default();
    let (a, b) = (instance(INSTANCE_A)?, instance(INSTANCE_B)?);
    fake.with_job(&a, job(100, 42, JobStatus::Pending)?);
    fake.with_job(&a, job(101, 42, JobStatus::Running)?);
    fake.with_job(&a, job(102, 43, JobStatus::Pending)?);
    fake.with_job(&a, job(104, 42, JobStatus::Pending)?);
    fake.with_job(&b, job(103, 42, JobStatus::Pending)?);
    let on_a = credentials(&a, "glpat-a");
    assert_eq!(
        fake.job(&on_a, id(42)?, id(101)?).await?,
        job(101, 42, JobStatus::Running)?
    );
    assert_eq!(
        fake.job(&on_a, id(42)?, id(999)?).await.err(),
        Some(GitLabError::NotFound)
    );
    assert_eq!(
        fake.job(&credentials(&b, "glpat-b"), id(42)?, id(100)?)
            .await
            .err(),
        Some(GitLabError::NotFound)
    );
    let mut pending: Vec<_> = fake
        .pending_jobs(&on_a, id(42)?)
        .await?
        .into_iter()
        .map(|job| job.id)
        .collect();
    pending.sort();
    assert_eq!(
        pending,
        [id(100)?, id(104)?],
        "pending jobs of that project only"
    );
    let on_b: Vec<_> = fake
        .pending_jobs(&credentials(&b, "glpat-b"), id(42)?)
        .await?
        .into_iter()
        .map(|job| job.id)
        .collect();
    assert_eq!(on_b, [id(103)?]);
    Ok(())
}

#[tokio::test]
async fn scopes_resolve_by_kind_and_path_and_groups_list_their_projects() -> Result<()> {
    let fake = FakeGitLab::default();
    let (a, b) = (instance(INSTANCE_A)?, instance(INSTANCE_B)?);
    let api_project = project(42, "acme/api")?;
    let acme = Scope::Group {
        id: id(7)?,
        path: "acme".to_owned(),
    };
    fake.with_scope(&a, api_project.clone());
    fake.with_scope(&a, acme.clone());
    let on_a = credentials(&a, "glpat-a");
    let on_b = credentials(&b, "glpat-b");
    assert_eq!(
        fake.resolve_scope(&on_a, ScopeKind::Project, "acme/api")
            .await?,
        api_project
    );
    assert_eq!(
        fake.resolve_scope(&on_a, ScopeKind::Group, "acme").await?,
        acme
    );
    for (credentials, kind, path) in [
        (&on_a, ScopeKind::Group, "acme/api"),
        (&on_a, ScopeKind::Project, "acme/missing"),
        (&on_b, ScopeKind::Project, "acme/api"),
    ] {
        assert_eq!(
            fake.resolve_scope(credentials, kind, path).await.err(),
            Some(GitLabError::NotFound),
            "{kind:?} {path} on {}",
            credentials.base_url
        );
    }
    fake.verify_access(&on_a, &api_project).await?;
    fake.verify_access(&on_a, &acme).await?;
    fake.with_group_project(&a, id(7)?, id(42)?);
    fake.with_group_project(&a, id(7)?, id(43)?);
    fake.with_group_project(&b, id(7)?, id(44)?);
    let mut projects = fake.group_projects(&on_a, id(7)?).await?;
    projects.sort();
    assert_eq!(projects, [id(42)?, id(43)?]);
    assert_eq!(fake.group_projects(&on_b, id(7)?).await?, [id(44)?]);
    Ok(())
}

#[tokio::test]
async fn queued_failures_are_one_shot_and_first_in_first_out_per_instance_and_method() -> Result<()>
{
    let fake = FakeGitLab::default();
    let (a, b) = (instance(INSTANCE_A)?, instance(INSTANCE_B)?);
    let scope = project(42, "acme/api")?;
    fake.with_scope(&a, scope.clone());
    fake.with_job(&a, job(100, 42, JobStatus::Pending)?);
    fake.fail_next(&a, FakeMethod::CreateRunner, GitLabError::Unauthorized);
    fake.fail_next(
        &a,
        FakeMethod::CreateRunner,
        GitLabError::RateLimited {
            retry_after: Some(Duration::from_secs(30)),
        },
    );
    fake.fail_next(
        &a,
        FakeMethod::VerifyAccess,
        GitLabError::MissingScope("create_runner".to_owned()),
    );
    let create = |base: &Url, token: &str| {
        let fake = fake.clone();
        let credentials = credentials(base, token);
        let scope = scope.clone();
        async move {
            fake.create_runner(&credentials, &scope, &[], false, "runner")
                .await
        }
    };
    // Another instance, and another method, are unaffected by a queue.
    assert!(create(&b, "glpat-b").await.is_ok());
    assert_eq!(
        fake.verify_access(&credentials(&a, "glpat-a"), &scope)
            .await
            .err(),
        Some(GitLabError::MissingScope("create_runner".to_owned()))
    );
    // In order, whatever token makes the call; then the queue is empty.
    assert_eq!(
        create(&a, "glpat-a").await.err(),
        Some(GitLabError::Unauthorized)
    );
    assert_eq!(
        create(&a, "glpat-another").await.err(),
        Some(GitLabError::RateLimited {
            retry_after: Some(Duration::from_secs(30))
        })
    );
    assert!(fake.runners(&a).is_empty(), "a failed call creates nothing");
    assert!(create(&a, "glpat-a").await.is_ok());
    fake.verify_access(&credentials(&a, "glpat-a"), &scope)
        .await?;
    // Every method honours its queue, and the error comes back as given.
    let on_a = credentials(&a, "glpat-a");
    fake.fail_next(&a, FakeMethod::DeleteRunner, GitLabError::MissingRole);
    fake.fail_next(
        &a,
        FakeMethod::Job,
        GitLabError::Upstream("502 from GitLab".to_owned()),
    );
    fake.fail_next(&a, FakeMethod::PendingJobs, GitLabError::NotFound);
    fake.fail_next(
        &a,
        FakeMethod::GroupProjects,
        GitLabError::RateLimited { retry_after: None },
    );
    fake.fail_next(&a, FakeMethod::ResolveScope, GitLabError::Unauthorized);
    assert_eq!(
        fake.delete_runner(&on_a, id(1)?).await.err(),
        Some(GitLabError::MissingRole)
    );
    assert_eq!(
        fake.job(&on_a, id(42)?, id(100)?).await.err(),
        Some(GitLabError::Upstream("502 from GitLab".to_owned()))
    );
    assert_eq!(
        fake.pending_jobs(&on_a, id(42)?).await.err(),
        Some(GitLabError::NotFound)
    );
    assert_eq!(
        fake.group_projects(&on_a, id(7)?).await.err(),
        Some(GitLabError::RateLimited { retry_after: None })
    );
    assert_eq!(
        fake.resolve_scope(&on_a, ScopeKind::Project, "acme/api")
            .await
            .err(),
        Some(GitLabError::Unauthorized)
    );
    // Each was one-shot.
    assert_eq!(fake.job(&on_a, id(42)?, id(100)?).await?.id, id(100)?);
    Ok(())
}

#[tokio::test]
async fn every_call_is_recorded_in_invocation_order_across_clones_without_tokens() -> Result<()> {
    let fake = FakeGitLab::default();
    let first = fake.clone();
    let second: Arc<dyn GitLabApi> = Arc::new(fake.clone());
    let (a, b) = (instance(INSTANCE_A)?, instance(INSTANCE_B)?);
    let scope = project(42, "acme/api")?;
    fake.with_scope(&a, scope.clone());
    fake.fail_next(
        &a,
        FakeMethod::VerifyAccess,
        GitLabError::Upstream("unavailable".to_owned()),
    );
    let on_a = credentials(&a, "glpat-recorded-a-0001");
    let on_b = credentials(&b, "glpat-recorded-b-0001");
    first
        .create_runner(&on_a, &scope, &[], false, "runner")
        .await?;
    // Only the log matters here, so an unseeded lookup's answer is ignored.
    let _ = second.pending_jobs(&on_b, id(42)?).await;
    assert_eq!(
        second.verify_access(&on_a, &scope).await.err(),
        Some(GitLabError::Upstream("unavailable".to_owned())),
        "this call fails because of fail_next"
    );
    first.delete_runner(&on_a, id(1)?).await?;
    let _ = second.group_projects(&on_b, id(7)?).await;
    let call = |base_url: &Url, method: FakeMethod| FakeCall {
        base_url: base_url.clone(),
        method,
    };
    assert_eq!(
        fake.calls(),
        vec![
            call(&a, FakeMethod::CreateRunner),
            call(&b, FakeMethod::PendingJobs),
            call(&a, FakeMethod::VerifyAccess),
            call(&a, FakeMethod::DeleteRunner),
            call(&b, FakeMethod::GroupProjects),
        ]
    );
    let log = format!("{:?}", first.calls());
    assert!(!log.contains("glpat-recorded"), "{log}");
    Ok(())
}
