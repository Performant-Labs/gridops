//! Drives `GitLabClient` (#146) against a real GitLab: resolve a project,
//! verify the token can manage its runners, create a project runner, then
//! delete it (twice: the second delete finds it gone, which is success).
//!
//! Ignored by default. With a token that has the `create_runner` and
//! `read_api` scopes (or `api`) and Maintainer on the project, run:
//!
//! ```text
//! GITLAB_TEST_TOKEN=glpat-... GITLAB_TEST_PROJECT=group/project \
//! cargo test -p gridops-core --test gitlab_live -- --ignored --nocapture
//! ```
//!
//! `GITLAB_TEST_URL` names another instance; it defaults to
//! `https://gitlab.com`. The runner is created with the tag
//! `gridops-live-test`, so a run interrupted before the delete leaves a runner
//! that is easy to find.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use gridops_core::gitlab::{GitLabApi, GitLabClient, GitLabCredentials, Scope, ScopeKind};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

#[tokio::test]
#[ignore = "needs GITLAB_TEST_TOKEN, GITLAB_TEST_PROJECT and network access"]
async fn creates_then_deletes_a_project_runner_on_a_real_gitlab() -> Result<()> {
    let token = std::env::var("GITLAB_TEST_TOKEN").context("GITLAB_TEST_TOKEN is required")?;
    let project =
        std::env::var("GITLAB_TEST_PROJECT").context("GITLAB_TEST_PROJECT is required")?;
    let base = std::env::var("GITLAB_TEST_URL").unwrap_or_else(|_| "https://gitlab.com".to_owned());
    let credentials = GitLabCredentials {
        base_url: Url::parse(&base).context("GITLAB_TEST_URL is not a URL")?,
        access_token: SecretString::from(token),
    };
    let api: Arc<dyn GitLabApi> = Arc::new(GitLabClient::new()?);

    let scope = api
        .resolve_scope(&credentials, ScopeKind::Project, &project)
        .await?;
    assert!(
        matches!(&scope, Scope::Project { path, .. } if path.eq_ignore_ascii_case(&project)),
        "resolved {scope:?} for {project}"
    );
    api.verify_access(&credentials, &scope).await?;

    let runner = api
        .create_runner(
            &credentials,
            &scope,
            &["gridops-live-test".to_owned()],
            false,
            "GridOps #146 live test",
        )
        .await?;
    // Delete before asserting, so a failed assertion leaves no runner behind.
    let deleted = api.delete_runner(&credentials, runner.id).await;
    let deleted_again = api.delete_runner(&credentials, runner.id).await;
    assert!(
        runner.token.expose_secret().starts_with("glrt-"),
        "GitLab returned a runner authentication token"
    );
    deleted?;
    deleted_again?;
    Ok(())
}
