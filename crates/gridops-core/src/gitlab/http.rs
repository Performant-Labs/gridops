//! [`GitLabClient`], the real [`GitLabApi`] (#146): GitLab's REST API v4 over
//! `reqwest`.
//!
//! The client holds no credentials. Each call takes the connection's
//! [`GitLabCredentials`], so one `Arc<dyn GitLabApi>` serves every connection
//! and every GitLab instance.
//!
//! Two secrets pass through here: the connection's access token (sent in a
//! sensitive `PRIVATE-TOKEN` header, never in a URL) and the `glrt-` runner
//! token `create_runner` receives. Neither may reach an error, a log line or
//! `Debug` output, so no error carries a response body or a `reqwest`/`serde`
//! message: errors hold a status code or a fixed description only. The token
//! never leaves `credentials.base_url`'s origin: redirects are not followed,
//! and a pagination link to another origin is refused.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{
    Method, Response, StatusCode,
    header::{ACCEPT, HeaderMap, HeaderValue, LINK, RETRY_AFTER},
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use url::Url;

use super::{
    CreatedRunner, GitLabApi, GitLabCredentials, GitLabError, Job, JobStatus, RunnerSummary, Scope,
    ScopeKind,
};
use crate::fleet::ExternalId;

/// How long establishing a connection to GitLab may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one whole request may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The most pages one list call reads. A server still paginating past this is
/// treated as broken (an `Upstream` error), not truncated silently.
const MAX_PAGES: usize = 100;
/// GitLab's largest page size.
const PER_PAGE: &str = "100";
/// GitLab's Maintainer access level: enough to create a project runner.
const MAINTAINER_ACCESS: u64 = 40;
/// GitLab's Owner access level: needed to create a group runner.
const OWNER_ACCESS: u64 = 50;
/// The scope `verify_access` reports when a 403 `insufficient_scope` body
/// names none `GridOps` can safely repeat.
const DEFAULT_MISSING_SCOPE: &str = "api";

/// The real GitLab client. It holds only the HTTP client, never a token.
#[derive(Debug, Clone)]
pub struct GitLabClient {
    http: reqwest::Client,
}

impl GitLabClient {
    pub fn new() -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("GridOps/0.1")
            // A redirect could carry PRIVATE-TOKEN to another host.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()?;
        Ok(Self { http })
    }

    /// Sends one authenticated request and returns the response whatever its
    /// status. Only transport failures are errors here; [`error_for`] maps
    /// statuses. This is the only place the access token is exposed.
    async fn send(
        &self,
        credentials: &GitLabCredentials,
        method: Method,
        url: Url,
        body: Option<&Value>,
    ) -> Result<Response, GitLabError> {
        let mut token = HeaderValue::from_str(credentials.access_token.expose_secret())
            .map_err(|_| upstream("the access token is not a valid HTTP header value"))?;
        token.set_sensitive(true);
        let mut request = self
            .http
            .request(method, url)
            .header("PRIVATE-TOKEN", token)
            .header(ACCEPT, "application/json");
        if let Some(body) = body {
            request = request.json(body);
        }
        request
            .send()
            .await
            .map_err(|error| transport_error(&error))
    }

    /// `GET`s one resource and decodes it; `what` names it in a decode error.
    async fn get_json<T: DeserializeOwned>(
        &self,
        credentials: &GitLabCredentials,
        url: Url,
        what: &str,
    ) -> Result<T, GitLabError> {
        let response = success(self.send(credentials, Method::GET, url, None).await?).await?;
        decode(response, what).await
    }

    /// `GET`s every page of a list, starting at `first`. The next page is the
    /// `Link` header's `rel="next"` URL (same origin only), else `first` with
    /// `page` set to `x-next-page`; neither means the last page. Any failed
    /// page fails the whole list.
    async fn get_all_pages<T: DeserializeOwned>(
        &self,
        credentials: &GitLabCredentials,
        first: Url,
        what: &str,
    ) -> Result<Vec<T>, GitLabError> {
        let mut items = Vec::new();
        let mut url = first.clone();
        for _ in 0..MAX_PAGES {
            let response = success(
                self.send(credentials, Method::GET, url.clone(), None)
                    .await?,
            )
            .await?;
            let next = next_page(&url, &first, &credentials.base_url, response.headers())?;
            items.extend(decode::<Vec<T>>(response, what).await?);
            match next {
                Some(next) => url = next,
                None => return Ok(items),
            }
        }
        Err(upstream(format!(
            "GitLab listed {what} across more than {MAX_PAGES} pages"
        )))
    }
}

#[async_trait]
impl GitLabApi for GitLabClient {
    async fn create_runner(
        &self,
        credentials: &GitLabCredentials,
        scope: &Scope,
        tags: &[String],
        run_untagged: bool,
        description: &str,
    ) -> Result<CreatedRunner, GitLabError> {
        let (runner_type, id_field, id) = match scope {
            Scope::Project { id, .. } => ("project_type", "project_id", id),
            Scope::Group { id, .. } => ("group_type", "group_id", id),
        };
        let mut body = Map::new();
        body.insert("runner_type".to_owned(), runner_type.into());
        body.insert(id_field.to_owned(), id.get().into());
        body.insert("tag_list".to_owned(), tags.into());
        body.insert("run_untagged".to_owned(), run_untagged.into());
        body.insert("description".to_owned(), description.into());
        let url = api_url(&credentials.base_url, &["user", "runners"])?;
        let response = self
            .send(credentials, Method::POST, url, Some(&Value::Object(body)))
            .await?;
        let created: NewRunner = decode(success(response).await?, "runner").await?;
        Ok(CreatedRunner {
            id: created.id,
            token: SecretString::from(created.token),
        })
    }

    async fn delete_runner(
        &self,
        credentials: &GitLabCredentials,
        runner_id: ExternalId,
    ) -> Result<(), GitLabError> {
        let url = api_url(
            &credentials.base_url,
            &["runners", &runner_id.get().to_string()],
        )?;
        let response = self.send(credentials, Method::DELETE, url, None).await?;
        // A runner GitLab no longer has is already deleted. Checked before
        // `error_for`, which would make it `NotFound`.
        if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        Err(error_for(response).await)
    }

    async fn list_runners(
        &self,
        credentials: &GitLabCredentials,
        scope: &Scope,
    ) -> Result<Vec<RunnerSummary>, GitLabError> {
        // Only the scope's own runners (#139 D17): `type` keeps instance and
        // inherited group runners, which GridOps never registered there, out
        // of the sweep's listing.
        let (collection, runner_type) = match scope {
            Scope::Project { .. } => ("projects", "project_type"),
            Scope::Group { .. } => ("groups", "group_type"),
        };
        let mut url = api_url(
            &credentials.base_url,
            &[collection, &scope.id().get().to_string(), "runners"],
        )?;
        url.query_pairs_mut()
            .append_pair("type", runner_type)
            .append_pair("per_page", PER_PAGE);
        let runners: Vec<RunnerResponse> = self.get_all_pages(credentials, url, "runners").await?;
        Ok(runners
            .into_iter()
            .map(|runner| RunnerSummary {
                id: runner.id.get(),
                description: runner.description.unwrap_or_default(),
            })
            .collect())
    }

    async fn job(
        &self,
        credentials: &GitLabCredentials,
        project_id: ExternalId,
        job_id: ExternalId,
    ) -> Result<Job, GitLabError> {
        let url = api_url(
            &credentials.base_url,
            &[
                "projects",
                &project_id.get().to_string(),
                "jobs",
                &job_id.get().to_string(),
            ],
        )?;
        let job: JobResponse = self.get_json(credentials, url, "job").await?;
        Ok(job.into_job(project_id))
    }

    async fn pending_jobs(
        &self,
        credentials: &GitLabCredentials,
        project_id: ExternalId,
    ) -> Result<Vec<Job>, GitLabError> {
        let mut url = api_url(
            &credentials.base_url,
            &["projects", &project_id.get().to_string(), "jobs"],
        )?;
        url.query_pairs_mut()
            .append_pair("scope[]", "pending")
            .append_pair("per_page", PER_PAGE);
        let jobs: Vec<JobResponse> = self.get_all_pages(credentials, url, "jobs").await?;
        Ok(jobs
            .into_iter()
            .map(|job| job.into_job(project_id))
            .collect())
    }

    async fn group_projects(
        &self,
        credentials: &GitLabCredentials,
        group_id: ExternalId,
    ) -> Result<Vec<ExternalId>, GitLabError> {
        let mut url = api_url(
            &credentials.base_url,
            &["groups", &group_id.get().to_string(), "projects"],
        )?;
        url.query_pairs_mut()
            .append_pair("include_subgroups", "true")
            .append_pair("per_page", PER_PAGE);
        let projects: Vec<IdOnly> = self.get_all_pages(credentials, url, "projects").await?;
        Ok(projects.into_iter().map(|project| project.id).collect())
    }

    async fn resolve_scope(
        &self,
        credentials: &GitLabCredentials,
        kind: ScopeKind,
        path: &str,
    ) -> Result<Scope, GitLabError> {
        // `path` is one URL segment: `group/sub` is sent as `group%2Fsub`.
        match kind {
            ScopeKind::Project => {
                let url = api_url(&credentials.base_url, &["projects", path])?;
                let project: ProjectResponse = self.get_json(credentials, url, "project").await?;
                Ok(Scope::Project {
                    id: project.id,
                    path: project.path_with_namespace,
                })
            }
            ScopeKind::Group => {
                let mut url = api_url(&credentials.base_url, &["groups", path])?;
                // Without this GitLab embeds the group's projects in the reply.
                url.query_pairs_mut().append_pair("with_projects", "false");
                let group: GroupResponse = self.get_json(credentials, url, "group").await?;
                Ok(Scope::Group {
                    id: group.id,
                    path: group.full_path,
                })
            }
        }
    }

    async fn verify_access(
        &self,
        credentials: &GitLabCredentials,
        scope: &Scope,
    ) -> Result<(), GitLabError> {
        // 1. The token's scopes: `api` grants both that GridOps needs.
        let url = api_url(&credentials.base_url, &["personal_access_tokens", "self"])?;
        let token: TokenResponse = self.get_json(credentials, url, "access token").await?;
        let has = |wanted: &str| {
            token
                .scopes
                .iter()
                .any(|held| held == wanted || held == "api")
        };
        for needed in ["create_runner", "read_api"] {
            if !has(needed) {
                return Err(GitLabError::MissingScope(needed.to_owned()));
            }
        }

        // 2. The token user's role. `POST /user/runners` needs Owner for a
        // group runner but only Maintainer for a project runner.
        let url = api_url(&credentials.base_url, &["user"])?;
        let user: UserResponse = self.get_json(credentials, url, "user").await?;
        if user.is_admin {
            return Ok(());
        }
        let (collection, required) = match scope {
            Scope::Project { .. } => ("projects", MAINTAINER_ACCESS),
            Scope::Group { .. } => ("groups", OWNER_ACCESS),
        };
        let url = api_url(
            &credentials.base_url,
            &[
                collection,
                &scope.id().get().to_string(),
                "members",
                "all",
                &user.id.get().to_string(),
            ],
        )?;
        let member: MemberResponse = match self.get_json(credentials, url, "membership").await {
            Ok(member) => member,
            // Not a member at all.
            Err(GitLabError::NotFound) => return Err(GitLabError::MissingRole),
            Err(error) => return Err(error),
        };
        if member.access_level < required {
            return Err(GitLabError::MissingRole);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// GitLab's response shapes (only the fields GridOps reads). None derives
// `Debug`: `NewRunner` holds the runner token.
// ---------------------------------------------------------------------------

/// `POST /user/runners`. The token is returned this once.
#[derive(Deserialize)]
struct NewRunner {
    id: ExternalId,
    token: String,
}

/// A job from `/projects/:id/jobs`. It has no top-level `project_id`.
#[derive(Deserialize)]
struct JobResponse {
    id: ExternalId,
    status: JobStatus,
    #[serde(default)]
    tag_list: Vec<String>,
    pipeline: IdOnly,
}

impl JobResponse {
    fn into_job(self, project_id: ExternalId) -> Job {
        Job {
            id: self.id,
            project_id,
            pipeline_id: self.pipeline.id,
            status: self.status,
            tag_list: self.tag_list,
        }
    }
}

/// A runner from `/projects/:id/runners` or `/groups/:id/runners`. GitLab
/// sends `description: null` for a runner registered without one.
#[derive(Deserialize)]
struct RunnerResponse {
    id: ExternalId,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Deserialize)]
struct IdOnly {
    id: ExternalId,
}

#[derive(Deserialize)]
struct ProjectResponse {
    id: ExternalId,
    path_with_namespace: String,
}

#[derive(Deserialize)]
struct GroupResponse {
    id: ExternalId,
    full_path: String,
}

/// `GET /personal_access_tokens/self`.
#[derive(Deserialize)]
struct TokenResponse {
    scopes: Vec<String>,
}

/// `GET /user`. GitLab shows `is_admin` to administrators only.
#[derive(Deserialize)]
struct UserResponse {
    id: ExternalId,
    #[serde(default)]
    is_admin: bool,
}

/// `GET /{projects|groups}/:id/members/all/:user_id`.
#[derive(Deserialize)]
struct MemberResponse {
    access_level: u64,
}

// ---------------------------------------------------------------------------
// URLs, statuses and pagination
// ---------------------------------------------------------------------------

fn upstream(message: impl Into<String>) -> GitLabError {
    GitLabError::Upstream(message.into())
}

/// `base_url` + `api/v4/` + `segments`, each segment percent-encoded on its
/// own (so `/` inside one becomes `%2F`). A base path prefix is kept and a
/// trailing slash on it does not double up.
fn api_url(base_url: &Url, segments: &[&str]) -> Result<Url, GitLabError> {
    let mut url = base_url.clone();
    url.set_query(None);
    url.set_fragment(None);
    url.path_segments_mut()
        .map_err(|()| upstream("the GitLab base URL cannot hold a path"))?
        .pop_if_empty()
        .extend(["api", "v4"])
        .extend(segments);
    Ok(url)
}

/// A fixed description of a transport failure. `reqwest`'s own message
/// names the URL and is not repeated.
fn transport_error(error: &reqwest::Error) -> GitLabError {
    upstream(if error.is_timeout() {
        "GitLab did not answer in time"
    } else if error.is_connect() {
        "could not connect to GitLab"
    } else {
        "could not reach GitLab"
    })
}

/// The response if it is a success, else its [`GitLabError`].
async fn success(response: Response) -> Result<Response, GitLabError> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(error_for(response).await)
    }
}

/// Maps an unsuccessful response. Only a 403's body is read (for
/// `insufficient_scope`), and no body ever goes into the error.
async fn error_for(response: Response) -> GitLabError {
    let status = response.status();
    match status {
        StatusCode::UNAUTHORIZED => GitLabError::Unauthorized,
        StatusCode::FORBIDDEN => {
            let body = response.bytes().await.unwrap_or_default();
            missing_scope(&body).map_or(GitLabError::MissingRole, GitLabError::MissingScope)
        }
        StatusCode::NOT_FOUND => GitLabError::NotFound,
        StatusCode::TOO_MANY_REQUESTS => GitLabError::RateLimited {
            retry_after: retry_after(response.headers()),
        },
        _ => upstream(format!("GitLab returned {status}")),
    }
}

/// The scope a 403 `{"error": "insufficient_scope", "scope": …}` body says
/// the token lacks, or `None` for any other 403. A `scope` that is absent or
/// does not look like a scope list is reported as `api`, so the body never
/// reaches the error.
fn missing_scope(body: &[u8]) -> Option<String> {
    let body: Value = serde_json::from_slice(body).ok()?;
    if body.get("error").and_then(Value::as_str) != Some("insufficient_scope") {
        return None;
    }
    let scope = body
        .get("scope")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|scope| {
            !scope.is_empty()
                && scope.len() <= 64
                && scope
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_' || byte == b' ')
        })
        .unwrap_or(DEFAULT_MISSING_SCOPE);
    Some(scope.to_owned())
}

/// `Retry-After` in whole seconds: a non-negative decimal integer, surrounding
/// whitespace ignored. Absent, an HTTP-date or anything else is `None`.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok().map(Duration::from_secs)
}

/// Decodes a success body. A decode error is reported without `serde`'s
/// message, which can quote the body (and so a token).
async fn decode<T: DeserializeOwned>(response: Response, what: &str) -> Result<T, GitLabError> {
    let body = response
        .bytes()
        .await
        .map_err(|_| upstream(format!("could not read GitLab's {what} response")))?;
    serde_json::from_slice(&body)
        .map_err(|_| upstream(format!("GitLab sent a {what} response GridOps cannot read")))
}

/// The page after `current`, from its response `headers`: the `Link`
/// `rel="next"` URL if there is one, else `first` with `page` set to
/// `x-next-page`. `None` is the last page. A `Link` to another origin than
/// `base_url`'s is an error: following it would send the token there.
fn next_page(
    current: &Url,
    first: &Url,
    base_url: &Url,
    headers: &HeaderMap,
) -> Result<Option<Url>, GitLabError> {
    if let Some(next) = link_next(headers) {
        let next = current
            .join(&next)
            .map_err(|_| upstream("GitLab sent an invalid next-page link"))?;
        if !same_origin(&next, base_url) {
            return Err(upstream("GitLab's next-page link points at another origin"));
        }
        return Ok(Some(next));
    }
    let Some(page) = headers.get("x-next-page") else {
        return Ok(None);
    };
    let page = page
        .to_str()
        .map_err(|_| upstream("GitLab sent an invalid x-next-page header"))?
        .trim();
    if page.is_empty() {
        return Ok(None);
    }
    if !page.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(upstream("GitLab sent an invalid x-next-page header"));
    }
    let mut next = first.clone();
    next.query_pairs_mut().append_pair("page", page);
    Ok(Some(next))
}

/// The target of the `rel="next"` entry of the `Link` header(s), if any.
fn link_next(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(LINK)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|value| {
            // Each entry is `<target>; param; param`, entries comma-separated.
            // Splitting on `<` keeps a comma inside a target intact.
            value.split('<').skip(1).find_map(|entry| {
                let (target, params) = entry.split_once('>')?;
                let is_next = params.split(';').any(|param| {
                    param.split_once('=').is_some_and(|(name, value)| {
                        name.trim().eq_ignore_ascii_case("rel")
                            && value
                                .trim()
                                .trim_end_matches(',')
                                .trim()
                                .trim_matches('"')
                                .split_ascii_whitespace()
                                .any(|rel| rel.eq_ignore_ascii_case("next"))
                    })
                });
                is_next.then(|| target.trim().to_owned())
            })
        })
}

fn same_origin(url: &Url, base_url: &Url) -> bool {
    url.scheme() == base_url.scheme()
        && url.host_str() == base_url.host_str()
        && url.port_or_known_default() == base_url.port_or_known_default()
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
