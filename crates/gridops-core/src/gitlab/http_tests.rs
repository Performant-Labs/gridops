//! `GitLabClient`'s tests (#146). Mounted from `gitlab/http.rs` with
//! `#[cfg(test)] #[path = "http_tests.rs"] mod tests;`, the way `fake.rs`
//! mounts `fake_tests.rs`.
//!
//! Every test talks to [`Mock`], an in-process HTTP/1.1 server on a tokio
//! `TcpListener` (no new dependency). It is deliberately small: one request
//! per connection, a `Content-Length` body, `Connection: close`, no chunked
//! encoding. It records each request's method, target, headers and body, so
//! the tests assert the exact path, query, `PRIVATE-TOKEN` header and JSON
//! body the client sends.

use std::{
    fmt::Write as _,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use anyhow::{Context as _, Result, bail};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
};
use url::Url;

use crate::{
    fleet::ExternalId,
    gitlab::{
        GitLabApi, GitLabClient, GitLabCredentials, GitLabError, Job, JobStatus, Scope, ScopeKind,
    },
};

/// The access token every test sends. Distinctive, so a leak is easy to find.
const TOKEN: &str = "glpat-test-access-token-0123456789";
/// The runner token the mock's `POST /user/runners` hands back.
const RUNNER_TOKEN: &str = "glrt-test-runner-token-9876543210";

// ---------------------------------------------------------------------------
// The mock GitLab
// ---------------------------------------------------------------------------

/// One request the mock received.
#[derive(Debug, Clone)]
struct Request {
    method: String,
    /// The request target as sent: path and query, still percent-encoded.
    target: String,
    /// Header names lower-cased.
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    /// The percent-encoded path, without the query.
    fn path(&self) -> &str {
        self.target
            .split_once('?')
            .map_or(self.target.as_str(), |(path, _)| path)
    }

    /// The decoded query pairs, sorted, so `scope[]=…` and `scope%5B%5D=…`
    /// compare equal and parameter order does not matter.
    fn query(&self) -> Vec<(String, String)> {
        let raw = self.target.split_once('?').map_or("", |(_, query)| query);
        let mut pairs: Vec<(String, String)> = url::form_urlencoded::parse(raw.as_bytes())
            .into_owned()
            .collect();
        pairs.sort();
        pairs
    }

    fn query_value(&self, name: &str) -> Option<String> {
        self.query()
            .into_iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn json(&self) -> Result<Value> {
        Ok(serde_json::from_slice(&self.body)?)
    }
}

/// Sorted owned query pairs, to compare with [`Request::query`].
fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = expected
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    pairs.sort();
    pairs
}

/// The response the mock sends.
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn json(status: u16, body: &Value) -> Self {
        Self {
            status,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: body.to_string(),
        }
    }

    fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
            body: body.to_owned(),
        }
    }

    fn empty(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: String::new(),
        }
    }

    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// Answers a request. The second argument is the mock's own origin
/// (`http://127.0.0.1:<port>/`), for replies that name the mock's URL.
type Handler = dyn Fn(&Request, &Url) -> Reply + Send + Sync;

struct Mock {
    origin: Url,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Mock {
    async fn start(
        handler: impl Fn(&Request, &Url) -> Reply + Send + Sync + 'static,
    ) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let origin = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let (log, served_origin) = (Arc::clone(&requests), origin.clone());
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let handler = Arc::clone(&handler);
                let log = Arc::clone(&log);
                let origin = served_origin.clone();
                tokio::spawn(async move {
                    // A broken connection fails the test through its assertions.
                    let _ = serve(stream, handler.as_ref(), &log, &origin).await;
                });
            }
        });
        Ok(Self { origin, requests })
    }

    /// Credentials whose base URL is the mock's origin.
    fn credentials(&self) -> GitLabCredentials {
        credentials(self.origin.clone())
    }

    fn port(&self) -> Result<u16> {
        self.origin.port().context("the mock origin has a port")
    }

    fn requests(&self) -> Vec<Request> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

async fn serve(
    mut stream: TcpStream,
    handler: &Handler,
    log: &Mutex<Vec<Request>>,
    origin: &Url,
) -> Result<()> {
    let mut buffer = Vec::new();
    let head_end = loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            bail!("the connection closed before the request head ended");
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8(buffer[..head_end].to_vec())?;
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().context("no request line")?.split(' ');
    let method = request_line.next().context("no method")?.to_owned();
    let target = request_line.next().context("no target")?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    if headers
        .iter()
        .any(|(name, value)| name == "transfer-encoding" && value.contains("chunked"))
    {
        bail!("the mock does not read chunked request bodies");
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .map(|(_, value)| value.parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    let mut body = buffer[head_end + 4..].to_vec();
    while body.len() < length {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            bail!("the connection closed before the request body ended");
        }
        body.extend_from_slice(&chunk[..read]);
    }
    let request = Request {
        method,
        target,
        headers,
        body,
    };
    let reply = handler(&request, origin);
    log.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(request);
    let reason = reqwest::StatusCode::from_u16(reply.status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or("Status");
    let mut response = format!(
        "HTTP/1.1 {} {reason}\r\ncontent-length: {}\r\nconnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (name, value) in &reply.headers {
        write!(response, "{name}: {value}\r\n")?;
    }
    response.push_str("\r\n");
    response.push_str(&reply.body);
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Fixtures and helpers
// ---------------------------------------------------------------------------

fn client() -> Result<GitLabClient> {
    GitLabClient::new().context("building the GitLab client")
}

fn credentials(base_url: Url) -> GitLabCredentials {
    GitLabCredentials {
        base_url,
        access_token: SecretString::from(TOKEN),
    }
}

fn id(value: u64) -> Result<ExternalId> {
    Ok(ExternalId::new(value)?)
}

fn project_scope() -> Result<Scope> {
    Ok(Scope::Project {
        id: id(42)?,
        path: "acme/api".to_owned(),
    })
}

fn group_scope() -> Result<Scope> {
    Ok(Scope::Group {
        id: id(7)?,
        path: "acme".to_owned(),
    })
}

/// A job as GitLab's jobs API returns it: no top-level `project_id`, the
/// pipeline nested.
fn job_json(job_id: u64, status: &str) -> Value {
    json!({
        "id": job_id,
        "status": status,
        "stage": "test",
        "name": "build",
        "ref": "main",
        "tag": false,
        "allow_failure": false,
        "created_at": "2026-11-20T10:00:00.000Z",
        "started_at": null,
        "tag_list": ["docker", "linux"],
        "pipeline": {
            "id": 555,
            "project_id": 42,
            "ref": "main",
            "sha": "0123456789abcdef0123456789abcdef01234567",
            "status": "running"
        },
        "runner": null,
        "user": { "id": 1, "username": "dev" }
    })
}

fn expected_job(job_id: u64, status: JobStatus) -> Result<Job> {
    Ok(Job {
        id: id(job_id)?,
        project_id: id(42)?,
        pipeline_id: id(555)?,
        status,
        tag_list: vec!["docker".to_owned(), "linux".to_owned()],
    })
}

/// The request carries the token in `PRIVATE-TOKEN`, never in the URL, and
/// identifies itself as `GridOps`.
fn assert_authenticated(request: &Request) {
    assert_eq!(
        request.header("private-token"),
        Some(TOKEN),
        "{} {} lacks the PRIVATE-TOKEN header",
        request.method,
        request.target
    );
    assert_eq!(request.header("user-agent"), Some("GridOps/0.1"));
    assert!(
        !request.target.contains("test-access-token"),
        "{} put the token in the URL",
        request.method
    );
}

/// `tag_list` sent as a JSON array or as a comma-joined string; absent is
/// empty.
fn sent_tags(body: &Value) -> Result<Vec<String>> {
    match body.get("tag_list") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(joined)) => Ok(joined
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
            .collect()),
        Some(Value::Array(tags)) => tags
            .iter()
            .map(|tag| {
                tag.as_str()
                    .map(str::to_owned)
                    .context("tag_list holds a non-string")
            })
            .collect(),
        Some(other) => bail!("tag_list is neither a string nor an array: {other}"),
    }
}

/// Neither secret appears in `text`.
fn assert_secret_free(text: &str, context: &str) {
    assert!(
        !text.contains("test-access-token"),
        "{context} leaked the access token: {text}"
    );
    assert!(
        !text.contains("test-runner-token"),
        "{context} leaked the runner token: {text}"
    );
}

/// Each trait method's name with its result, values dropped.
type MethodResults = Vec<(&'static str, Result<(), GitLabError>)>;

/// Calls every trait method once, through `dyn GitLabApi`, and returns each
/// result with its method name (values dropped).
async fn every_method(
    api: &dyn GitLabApi,
    credentials: &GitLabCredentials,
) -> Result<MethodResults> {
    let project = project_scope()?;
    Ok(vec![
        (
            "create_runner",
            api.create_runner(credentials, &project, &[], false, "runner")
                .await
                .map(drop),
        ),
        (
            "delete_runner",
            api.delete_runner(credentials, id(9001)?).await,
        ),
        (
            "list_runners",
            api.list_runners(credentials, &project).await.map(drop),
        ),
        (
            "job",
            api.job(credentials, id(42)?, id(1234)?).await.map(drop),
        ),
        (
            "pending_jobs",
            api.pending_jobs(credentials, id(42)?).await.map(drop),
        ),
        (
            "group_projects",
            api.group_projects(credentials, id(7)?).await.map(drop),
        ),
        (
            "resolve_scope",
            api.resolve_scope(credentials, ScopeKind::Project, "acme/api")
                .await
                .map(drop),
        ),
        (
            "verify_access",
            api.verify_access(credentials, &project).await,
        ),
    ])
}

// ---------------------------------------------------------------------------
// create_runner and delete_runner
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_runner_registers_a_project_runner_and_returns_its_one_time_token() -> Result<()> {
    let mock = Mock::start(|_, _| {
        Reply::json(
            201,
            &json!({ "id": 9001, "token": RUNNER_TOKEN, "token_expires_at": null }),
        )
    })
    .await?;
    let tags = vec!["docker".to_owned(), "linux".to_owned()];
    let runner = client()?
        .create_runner(
            &mock.credentials(),
            &project_scope()?,
            &tags,
            false,
            "GridOps pool linux",
        )
        .await?;
    assert_eq!(runner.id, id(9001)?);
    assert_eq!(runner.token.expose_secret(), RUNNER_TOKEN);

    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(
        (request.method.as_str(), request.path()),
        ("POST", "/api/v4/user/runners")
    );
    assert_authenticated(request);
    assert!(
        request
            .header("content-type")
            .is_some_and(|value| value.starts_with("application/json")),
        "the body is sent as JSON"
    );
    let body = request.json()?;
    assert_eq!(body["runner_type"], "project_type");
    assert_eq!(body["project_id"], 42);
    assert!(
        body.get("group_id").is_none(),
        "a project runner names no group"
    );
    assert_eq!(body["run_untagged"], false);
    assert_eq!(body["description"], "GridOps pool linux");
    assert_eq!(sent_tags(&body)?, tags);
    Ok(())
}

#[tokio::test]
async fn create_runner_registers_a_group_runner_with_the_group_id() -> Result<()> {
    let mock =
        Mock::start(|_, _| Reply::json(201, &json!({ "id": 9002, "token": RUNNER_TOKEN }))).await?;
    let runner = client()?
        .create_runner(&mock.credentials(), &group_scope()?, &[], true, "untagged")
        .await?;
    assert_eq!(runner.id, id(9002)?);

    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    let body = requests[0].json()?;
    assert_eq!(body["runner_type"], "group_type");
    assert_eq!(body["group_id"], 7);
    assert!(
        body.get("project_id").is_none(),
        "a group runner names no project"
    );
    assert_eq!(body["run_untagged"], true);
    assert_eq!(body["description"], "untagged");
    assert_eq!(sent_tags(&body)?, Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn delete_runner_deletes_by_id_and_counts_an_already_gone_runner_as_deleted() -> Result<()> {
    let mock = Mock::start(|request, _| match request.path() {
        "/api/v4/runners/9001" => Reply::empty(204),
        "/api/v4/runners/9002" => Reply::json(404, &json!({ "message": "404 Not found" })),
        _ => Reply::json(403, &json!({ "message": "403 Forbidden" })),
    })
    .await?;
    let api = client()?;
    let credentials = mock.credentials();
    assert_eq!(api.delete_runner(&credentials, id(9001)?).await, Ok(()));
    assert_eq!(
        api.delete_runner(&credentials, id(9002)?).await,
        Ok(()),
        "a runner GitLab no longer has is already deleted"
    );
    assert_eq!(
        api.delete_runner(&credentials, id(9003)?).await,
        Err(GitLabError::MissingRole),
        "only 404 is forgiven"
    );

    let requests = mock.requests();
    let sent: Vec<(&str, &str)> = requests
        .iter()
        .map(|request| (request.method.as_str(), request.path()))
        .collect();
    assert_eq!(
        sent,
        [
            ("DELETE", "/api/v4/runners/9001"),
            ("DELETE", "/api/v4/runners/9002"),
            ("DELETE", "/api/v4/runners/9003"),
        ]
    );
    requests.iter().for_each(assert_authenticated);
    Ok(())
}

// ---------------------------------------------------------------------------
// job, pending_jobs and group_projects
// ---------------------------------------------------------------------------

#[tokio::test]
async fn job_reads_one_job_with_its_pipeline_tags_and_the_callers_project() -> Result<()> {
    let mock = Mock::start(|_, _| Reply::json(200, &job_json(1234, "running"))).await?;
    let job = client()?
        .job(&mock.credentials(), id(42)?, id(1234)?)
        .await?;
    assert_eq!(job, expected_job(1234, JobStatus::Running)?);

    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        (requests[0].method.as_str(), requests[0].path()),
        ("GET", "/api/v4/projects/42/jobs/1234")
    );
    assert_authenticated(&requests[0]);
    Ok(())
}

#[tokio::test]
async fn pending_jobs_follows_the_link_header_to_every_page() -> Result<()> {
    let mock = Mock::start(|request, origin| {
        if request.query_value("cursor").is_some() {
            // The last page: a Link with no `next`.
            let first = format!("<{origin}api/v4/projects/42/jobs?page=1&per_page=100>");
            Reply::json(200, &json!([job_json(3, "pending")]))
                .header("link", &format!("{first}; rel=\"first\""))
                .header("x-next-page", "")
        } else {
            // `next` is not the first link, and carries a parameter only the
            // Link URL has, so following it verbatim is observable.
            let first = format!("<{origin}api/v4/projects/42/jobs?page=1&per_page=100>");
            let next = format!(
                "<{origin}api/v4/projects/42/jobs?cursor=link-only&page=2&per_page=100&scope%5B%5D=pending>"
            );
            Reply::json(200, &json!([job_json(1, "pending"), job_json(2, "pending")]))
                .header(
                    "link",
                    &format!("{first}; rel=\"first\", {next}; rel=\"next\""),
                )
                .header("x-next-page", "2")
        }
    })
    .await?;
    let jobs = client()?.pending_jobs(&mock.credentials(), id(42)?).await?;
    assert_eq!(
        jobs,
        [
            expected_job(1, JobStatus::Pending)?,
            expected_job(2, JobStatus::Pending)?,
            expected_job(3, JobStatus::Pending)?,
        ]
    );

    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path(), "/api/v4/projects/42/jobs");
    assert_eq!(
        requests[0].query(),
        pairs(&[("per_page", "100"), ("scope[]", "pending")])
    );
    assert_eq!(requests[1].path(), "/api/v4/projects/42/jobs");
    assert_eq!(
        requests[1].query(),
        pairs(&[
            ("cursor", "link-only"),
            ("page", "2"),
            ("per_page", "100"),
            ("scope[]", "pending"),
        ]),
        "the Link `next` URL is requested as given"
    );
    requests.iter().for_each(assert_authenticated);
    Ok(())
}

#[tokio::test]
async fn pending_jobs_follows_x_next_page_when_there_is_no_link() -> Result<()> {
    let mock = Mock::start(|request, _| match request.query_value("page").as_deref() {
        None | Some("1") => {
            Reply::json(200, &json!([job_json(1, "pending")])).header("x-next-page", "2")
        }
        // A status GitLab adds later still decodes, as `Unknown`.
        _ => {
            Reply::json(200, &json!([job_json(2, "some_future_status")])).header("x-next-page", "")
        }
    })
    .await?;
    let jobs = client()?.pending_jobs(&mock.credentials(), id(42)?).await?;
    assert_eq!(
        jobs,
        [
            expected_job(1, JobStatus::Pending)?,
            expected_job(2, JobStatus::Unknown)?,
        ]
    );

    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].query(),
        pairs(&[("page", "2"), ("per_page", "100"), ("scope[]", "pending")])
    );
    assert_eq!(requests[1].path(), "/api/v4/projects/42/jobs");
    Ok(())
}

#[tokio::test]
async fn group_projects_lists_every_project_of_the_group_and_its_subgroups() -> Result<()> {
    let mock = Mock::start(|request, _| match request.query_value("page").as_deref() {
        None | Some("1") => Reply::json(
            200,
            &json!([
                { "id": 101, "path_with_namespace": "acme/web" },
                { "id": 102, "path_with_namespace": "acme/sub/api" }
            ]),
        )
        .header("x-next-page", "2"),
        _ => Reply::json(
            200,
            &json!([{ "id": 201, "path_with_namespace": "acme/sub/deep/cli" }]),
        ),
    })
    .await?;
    let projects = client()?
        .group_projects(&mock.credentials(), id(7)?)
        .await?;
    assert_eq!(projects, [id(101)?, id(102)?, id(201)?]);

    let requests = mock.requests();
    assert_eq!(requests.len(), 2, "an absent x-next-page ends the list");
    assert_eq!(requests[0].path(), "/api/v4/groups/7/projects");
    assert_eq!(
        requests[0].query(),
        pairs(&[("include_subgroups", "true"), ("per_page", "100")])
    );
    assert_eq!(
        requests[1].query(),
        pairs(&[
            ("include_subgroups", "true"),
            ("page", "2"),
            ("per_page", "100")
        ])
    );
    requests.iter().for_each(assert_authenticated);
    Ok(())
}

#[tokio::test]
async fn list_runners_lists_every_page_of_the_scopes_own_runners() -> Result<()> {
    let mock = Mock::start(|request, _| match request.query_value("page").as_deref() {
        None | Some("1") => Reply::json(
            200,
            &json!([
                { "id": 11, "description": "gridops-runner:gitlab.example:abc" },
                { "id": 12, "description": null }
            ]),
        )
        .header("x-next-page", "2"),
        _ => Reply::json(200, &json!([{ "id": 13, "description": "other" }])),
    })
    .await?;
    let runners = client()?
        .list_runners(&mock.credentials(), &project_scope()?)
        .await?;
    let listed: Vec<(u64, &str)> = runners
        .iter()
        .map(|runner| (runner.id, runner.description.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            (11, "gridops-runner:gitlab.example:abc"),
            (12, ""),
            (13, "other")
        ]
    );
    let requests = mock.requests();
    assert_eq!(requests.len(), 2, "an absent x-next-page ends the list");
    assert_eq!(requests[0].path(), "/api/v4/projects/42/runners");
    assert_eq!(
        requests[0].query(),
        pairs(&[("type", "project_type"), ("per_page", "100")])
    );
    requests.iter().for_each(assert_authenticated);

    let group = group_scope()?;
    let mock = Mock::start(|_, _| Reply::json(200, &json!([]))).await?;
    let runners = client()?.list_runners(&mock.credentials(), &group).await?;
    assert_eq!(runners.len(), 0, "an empty listing");
    let requests = mock.requests();
    assert_eq!(
        requests[0].path(),
        format!("/api/v4/groups/{}/runners", group.id().get())
    );
    assert_eq!(
        requests[0].query(),
        pairs(&[("type", "group_type"), ("per_page", "100")])
    );
    Ok(())
}

#[tokio::test]
async fn a_later_page_failing_fails_the_whole_list() -> Result<()> {
    let mock = Mock::start(|request, _| match request.query_value("page").as_deref() {
        None | Some("1") => Reply::json(200, &json!([{ "id": 101 }])).header("x-next-page", "2"),
        _ => Reply::json(500, &json!({ "message": "500 Internal Server Error" })),
    })
    .await?;
    let result = client()?.group_projects(&mock.credentials(), id(7)?).await;
    assert!(
        matches!(result, Err(GitLabError::Upstream(_))),
        "no partial list: {result:?}"
    );
    let pages: Vec<Option<String>> = mock
        .requests()
        .iter()
        .map(|request| request.query_value("page"))
        .collect();
    assert_eq!(
        pages,
        [None, Some("2".to_owned())],
        "the error comes from page 2, after page 1 succeeded"
    );
    Ok(())
}

/// The last page the paginating mock serves: one past the brief's largest
/// cap (1000), so a client without a cap ends with a list, not a hang.
const LAST_PAGE: u64 = 1001;

#[tokio::test]
async fn a_server_paginating_past_the_page_cap_is_cut_off_with_an_error() -> Result<()> {
    let mock = Mock::start(|request, _| {
        let page: u64 = request
            .query_value("page")
            .and_then(|page| page.parse().ok())
            .unwrap_or(1);
        let reply = Reply::json(200, &json!([{ "id": page }]));
        if page < LAST_PAGE {
            reply.header("x-next-page", &(page + 1).to_string())
        } else {
            reply
        }
    })
    .await?;
    let result = client()?.group_projects(&mock.credentials(), id(7)?).await;
    assert!(
        matches!(result, Err(GitLabError::Upstream(_))),
        "a page cap ends the loop with an error, not a list: {:?}",
        result.map(|projects| projects.len())
    );
    let pages = mock.requests().len();
    assert!(
        (2..=1000).contains(&pages),
        "the cap stops the loop within 1000 pages, after {pages}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// resolve_scope and the API root
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolve_scope_looks_up_the_url_encoded_path_and_keeps_gitlabs_spelling() -> Result<()> {
    let mock = Mock::start(|request, _| match request.path() {
        "/api/v4/projects/Acme%2FSub%2FAPI" => Reply::json(
            200,
            &json!({
                "id": 42,
                "name": "API",
                "path": "api",
                "path_with_namespace": "acme/sub/api",
                "namespace": { "id": 8, "full_path": "acme/sub" }
            }),
        ),
        "/api/v4/groups/Acme%2FSub" => Reply::json(
            200,
            &json!({ "id": 8, "name": "Sub", "path": "sub", "full_path": "acme/sub" }),
        ),
        _ => Reply::json(404, &json!({ "message": "404 Project Not Found" })),
    })
    .await?;
    let api = client()?;
    let credentials = mock.credentials();
    assert_eq!(
        api.resolve_scope(&credentials, ScopeKind::Project, "Acme/Sub/API")
            .await?,
        Scope::Project {
            id: id(42)?,
            path: "acme/sub/api".to_owned()
        }
    );
    assert_eq!(
        api.resolve_scope(&credentials, ScopeKind::Group, "Acme/Sub")
            .await?,
        Scope::Group {
            id: id(8)?,
            path: "acme/sub".to_owned()
        }
    );
    assert_eq!(
        api.resolve_scope(&credentials, ScopeKind::Project, "acme/missing")
            .await,
        Err(GitLabError::NotFound)
    );

    let requests = mock.requests();
    let sent: Vec<(&str, &str)> = requests
        .iter()
        .map(|request| (request.method.as_str(), request.path()))
        .collect();
    assert_eq!(
        sent,
        [
            ("GET", "/api/v4/projects/Acme%2FSub%2FAPI"),
            ("GET", "/api/v4/groups/Acme%2FSub"),
            ("GET", "/api/v4/projects/acme%2Fmissing"),
        ]
    );
    requests.iter().for_each(assert_authenticated);
    Ok(())
}

#[tokio::test]
async fn the_api_root_keeps_a_base_url_path_prefix_with_or_without_a_trailing_slash() -> Result<()>
{
    let mock = Mock::start(|_, _| Reply::empty(204)).await?;
    let api = client()?;
    let port = mock.port()?;
    for base in [
        format!("http://127.0.0.1:{port}/gitlab"),
        format!("http://127.0.0.1:{port}/gitlab/"),
    ] {
        api.delete_runner(&credentials(Url::parse(&base)?), id(5)?)
            .await?;
    }
    let paths: Vec<String> = mock
        .requests()
        .iter()
        .map(|request| request.path().to_owned())
        .collect();
    assert_eq!(
        paths,
        ["/gitlab/api/v4/runners/5", "/gitlab/api/v4/runners/5"]
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/// Whether an error is the one a case expects.
type Check = fn(&GitLabError) -> bool;

/// What `job` returns for each error response.
#[tokio::test]
async fn error_statuses_map_to_gitlab_errors() -> Result<()> {
    let scope_error = |scope: Option<&str>| {
        let mut body = json!({
            "error": "insufficient_scope",
            "error_description": "The request requires higher privileges than provided by the access token."
        });
        if let Some(scope) = scope {
            body["scope"] = json!(scope);
        }
        Reply::json(403, &body)
    };
    let cases: Vec<(&str, Reply, Check)> = vec![
        (
            "401",
            Reply::json(401, &json!({ "message": "401 Unauthorized" })),
            |error| *error == GitLabError::Unauthorized,
        ),
        (
            "403 insufficient_scope naming read_api",
            scope_error(Some("read_api")),
            |error| *error == GitLabError::MissingScope("read_api".to_owned()),
        ),
        (
            "403 insufficient_scope naming no scope",
            scope_error(None),
            |error| *error == GitLabError::MissingScope("api".to_owned()),
        ),
        (
            "403 forbidden",
            Reply::json(403, &json!({ "message": "403 Forbidden" })),
            |error| *error == GitLabError::MissingRole,
        ),
        (
            "403 with a non-JSON body",
            Reply::text(403, "Forbidden"),
            |error| *error == GitLabError::MissingRole,
        ),
        (
            "404",
            Reply::json(404, &json!({ "message": "404 Not found" })),
            |error| *error == GitLabError::NotFound,
        ),
        (
            "400",
            Reply::json(400, &json!({ "message": "bad request" })),
            |error| matches!(error, GitLabError::Upstream(message) if message.contains("400")),
        ),
        (
            "500",
            Reply::json(500, &json!({ "message": "500 Internal Server Error" })),
            |error| matches!(error, GitLabError::Upstream(message) if message.contains("500")),
        ),
        (
            "502 with an HTML body",
            Reply::text(502, "<html>Bad Gateway</html>"),
            |error| matches!(error, GitLabError::Upstream(message) if message.contains("502")),
        ),
    ];
    let count = cases.len();
    let (labels, checks): (Vec<&str>, Vec<Check>) = cases
        .iter()
        .map(|(label, _, check)| (*label, *check))
        .unzip();
    // The job id selects the case's reply.
    let replies = Arc::new(Mutex::new(
        cases
            .into_iter()
            .map(|(_, reply, _)| Some(reply))
            .collect::<Vec<_>>(),
    ));
    let mock = Mock::start(move |request, _| {
        let index = request
            .path()
            .rsplit('/')
            .next()
            .and_then(|job| job.parse::<usize>().ok())
            .and_then(|job| job.checked_sub(1));
        index
            .and_then(|index| {
                replies
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get_mut(index)
                    .and_then(Option::take)
            })
            .unwrap_or_else(|| Reply::empty(418))
    })
    .await?;
    let api = client()?;
    let credentials = mock.credentials();
    for (index, (label, check)) in labels.iter().zip(&checks).enumerate() {
        let result = api.job(&credentials, id(42)?, id(index as u64 + 1)?).await;
        match result {
            Err(error) => assert!(check(&error), "{label}: got {error:?}"),
            Ok(job) => bail!("{label}: expected an error, got {job:?}"),
        }
    }
    assert_eq!(mock.requests().len(), count);
    Ok(())
}

#[tokio::test]
async fn retry_after_is_honoured_only_as_whole_seconds() -> Result<()> {
    let cases: [(Option<&str>, Option<Duration>); 6] = [
        (Some("30"), Some(Duration::from_secs(30))),
        (Some(" 30 "), Some(Duration::from_secs(30))),
        (Some("0"), Some(Duration::ZERO)),
        (None, None),
        (Some("Tue, 05 Jan 2021 11:00:00 GMT"), None),
        (Some("-1"), None),
    ];
    let headers: Vec<Option<String>> = cases
        .iter()
        .map(|(header, _)| header.map(str::to_owned))
        .collect();
    let mock = Mock::start(move |request, _| {
        let header = request
            .path()
            .rsplit('/')
            .next()
            .and_then(|job| job.parse::<usize>().ok())
            .and_then(|job| headers.get(job.checked_sub(1)?).cloned())
            .flatten();
        let reply = Reply::json(429, &json!({ "message": "Retry later" }));
        match header {
            Some(value) => reply.header("retry-after", &value),
            None => reply,
        }
    })
    .await?;
    let api = client()?;
    let credentials = mock.credentials();
    for (index, (header, expected)) in cases.iter().enumerate() {
        assert_eq!(
            api.job(&credentials, id(42)?, id(index as u64 + 1)?).await,
            Err(GitLabError::RateLimited {
                retry_after: *expected
            }),
            "Retry-After {header:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn every_method_maps_unauthorized_and_rate_limited_responses() -> Result<()> {
    let api: Arc<dyn GitLabApi> = Arc::new(client()?);
    let unauthorized =
        Mock::start(|_, _| Reply::json(401, &json!({ "message": "401 Unauthorized" }))).await?;
    for (method, result) in every_method(api.as_ref(), &unauthorized.credentials()).await? {
        assert_eq!(result, Err(GitLabError::Unauthorized), "{method}");
    }
    let limited = Mock::start(|_, _| {
        Reply::json(429, &json!({ "message": "Retry later" })).header("retry-after", "7")
    })
    .await?;
    for (method, result) in every_method(api.as_ref(), &limited.credentials()).await? {
        assert_eq!(
            result,
            Err(GitLabError::RateLimited {
                retry_after: Some(Duration::from_secs(7))
            }),
            "{method}"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// verify_access
// ---------------------------------------------------------------------------

/// A GitLab for `verify_access`: the token's scopes, the user (id 11, an
/// admin or not) and the user's access level on project 42 and group 7
/// (`None`: not a member, 404).
async fn access_mock(scopes: &[&str], is_admin: bool, access_level: Option<u64>) -> Result<Mock> {
    let scopes: Vec<String> = scopes.iter().map(|scope| (*scope).to_owned()).collect();
    Mock::start(move |request, _| match request.path() {
        "/api/v4/personal_access_tokens/self" => Reply::json(
            200,
            &json!({
                "id": 1,
                "name": "gridops",
                "revoked": false,
                "active": true,
                "created_at": "2026-11-01T00:00:00.000Z",
                "expires_at": null,
                "scopes": scopes,
                "user_id": 11
            }),
        ),
        "/api/v4/user" => {
            // GitLab shows `is_admin` only to administrators.
            let mut user = json!({ "id": 11, "username": "gridops-bot", "state": "active" });
            if is_admin {
                user["is_admin"] = json!(true);
            }
            Reply::json(200, &user)
        }
        "/api/v4/projects/42/members/all/11" | "/api/v4/groups/7/members/all/11" => {
            match access_level {
                Some(level) => Reply::json(
                    200,
                    &json!({ "id": 11, "username": "gridops-bot", "access_level": level }),
                ),
                None => Reply::json(404, &json!({ "message": "404 Not found" })),
            }
        }
        _ => Reply::json(404, &json!({ "message": "404 Not found" })),
    })
    .await
}

#[tokio::test]
async fn verify_access_reports_the_first_scope_the_token_lacks() -> Result<()> {
    let missing = |scope: &str| Err(GitLabError::MissingScope(scope.to_owned()));
    let cases: [(&[&str], Result<(), GitLabError>); 8] = [
        (&["api"], Ok(())),
        (&["create_runner", "read_api"], Ok(())),
        (&["api", "read_user"], Ok(())),
        (&[], missing("create_runner")),
        (&["read_api"], missing("create_runner")),
        (&["read_user", "read_repository"], missing("create_runner")),
        (&["create_runner"], missing("read_api")),
        (&["create_runner", "read_repository"], missing("read_api")),
    ];
    let api = client()?;
    for (scopes, expected) in cases {
        let mock = access_mock(scopes, false, Some(50)).await?;
        assert_eq!(
            api.verify_access(&mock.credentials(), &project_scope()?)
                .await,
            expected,
            "scopes {scopes:?}"
        );
    }
    Ok(())
}

/// A scope, whether the user is an administrator, the user's access level
/// (`None`: not a member) and what `verify_access` returns.
type RoleCase = (Scope, bool, Option<u64>, Result<(), GitLabError>);

#[tokio::test]
async fn verify_access_requires_maintainer_on_a_project_and_owner_on_a_group() -> Result<()> {
    let cases: [RoleCase; 9] = [
        (project_scope()?, false, Some(40), Ok(())),
        (project_scope()?, false, Some(50), Ok(())),
        (
            project_scope()?,
            false,
            Some(30),
            Err(GitLabError::MissingRole),
        ),
        (group_scope()?, false, Some(50), Ok(())),
        (
            group_scope()?,
            false,
            Some(40),
            Err(GitLabError::MissingRole),
        ),
        (project_scope()?, false, None, Err(GitLabError::MissingRole)),
        (group_scope()?, false, None, Err(GitLabError::MissingRole)),
        // An administrator passes, member or not.
        (group_scope()?, true, None, Ok(())),
        (project_scope()?, true, Some(10), Ok(())),
    ];
    let api = client()?;
    for (scope, is_admin, level, expected) in cases {
        let mock = access_mock(&["api"], is_admin, level).await?;
        assert_eq!(
            api.verify_access(&mock.credentials(), &scope).await,
            expected,
            "{:?} admin={is_admin} access_level={level:?}",
            scope.kind()
        );
        let requests = mock.requests();
        requests.iter().for_each(assert_authenticated);
        if !is_admin {
            let membership = match scope.kind() {
                ScopeKind::Project => "/api/v4/projects/42/members/all/11",
                ScopeKind::Group => "/api/v4/groups/7/members/all/11",
            };
            for path in [
                "/api/v4/personal_access_tokens/self",
                "/api/v4/user",
                membership,
            ] {
                assert!(
                    requests
                        .iter()
                        .any(|request| request.method == "GET" && request.path() == path),
                    "{:?}: GET {path} was not requested",
                    scope.kind()
                );
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Secrets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_token_is_never_sent_to_another_origin() -> Result<()> {
    #[derive(Clone, Copy)]
    enum Elsewhere {
        Port,
        Host,
        Scheme,
    }
    // Any request reaching the trap is a token sent away.
    let trap = Mock::start(|_, _| Reply::json(200, &json!([]))).await?;
    let api = client()?;
    for elsewhere in [Elsewhere::Port, Elsewhere::Host, Elsewhere::Scheme] {
        let trap_origin = trap.origin.clone();
        let mock = Mock::start(move |_, origin| {
            let port = origin.port().unwrap_or_default();
            let path = "api/v4/projects/42/jobs?page=2&per_page=100&scope%5B%5D=pending";
            let next = match elsewhere {
                Elsewhere::Port => format!("{trap_origin}{path}"),
                Elsewhere::Host => format!("http://localhost:{port}/{path}"),
                Elsewhere::Scheme => format!("https://127.0.0.1:{port}/{path}"),
            };
            Reply::json(200, &json!([job_json(1, "pending")]))
                .header("link", &format!("<{next}>; rel=\"next\""))
                .header("x-next-page", "2")
        })
        .await?;
        let result = api.pending_jobs(&mock.credentials(), id(42)?).await;
        assert_eq!(mock.requests().len(), 1, "only the first page is requested");
        assert!(
            matches!(result, Err(GitLabError::Upstream(_))),
            "a cross-origin Link is an error: {result:?}"
        );
    }

    let trap_url = format!("{}api/v4/projects/42/jobs/1234", trap.origin);
    let redirecting =
        Mock::start(move |_, _| Reply::empty(302).header("location", &trap_url)).await?;
    let result = api
        .job(&redirecting.credentials(), id(42)?, id(1234)?)
        .await;
    assert!(
        matches!(result, Err(GitLabError::Upstream(ref message)) if message.contains("302")),
        "a redirect is not followed: {result:?}"
    );
    assert!(
        trap.requests().is_empty(),
        "a request reached another origin"
    );
    Ok(())
}

#[tokio::test]
async fn neither_token_appears_in_any_error_or_debug_output() -> Result<()> {
    let api = client()?;
    let dyn_api: &dyn GitLabApi = &api;

    // Error responses whose bodies echo both secrets.
    let echo = json!({
        "message": format!("rejected {TOKEN}"),
        "token": RUNNER_TOKEN,
        "error_description": format!("{TOKEN} {RUNNER_TOKEN}")
    });
    for status in [400_u16, 401, 403, 404, 409, 429, 500, 503] {
        let body = echo.clone();
        let mock = Mock::start(move |_, _| Reply::json(status, &body)).await?;
        for (method, result) in every_method(dyn_api, &mock.credentials()).await? {
            if let Err(error) = result {
                assert_secret_free(&error.to_string(), &format!("{method} {status} Display"));
                assert_secret_free(&format!("{error:?}"), &format!("{method} {status} Debug"));
            }
        }
    }

    // Success responses GitLab could not have meant: decode errors must not
    // echo the body. A string where a number or a list belongs is the case
    // serde quotes back.
    let malformed = [
        Reply::json(
            201,
            &json!({
                "id": RUNNER_TOKEN,
                "token": RUNNER_TOKEN,
                "scopes": TOKEN,
                "path_with_namespace": 1,
                "full_path": 1
            }),
        ),
        Reply::json(200, &json!([RUNNER_TOKEN, TOKEN])),
        Reply::text(200, &format!("<html>{TOKEN} {RUNNER_TOKEN}</html>")),
    ];
    for reply in malformed {
        let shared = Arc::new(reply);
        let served = Arc::clone(&shared);
        let mock = Mock::start(move |_, _| Reply {
            status: served.status,
            headers: served.headers.clone(),
            body: served.body.clone(),
        })
        .await?;
        for (method, result) in every_method(dyn_api, &mock.credentials()).await? {
            if method == "delete_runner" {
                continue; // A 2xx delete reads no body.
            }
            let context = format!("{method} on a malformed {} body", shared.status);
            match result {
                Err(error @ GitLabError::Upstream(_)) => {
                    assert_secret_free(&error.to_string(), &context);
                    assert_secret_free(&format!("{error:?}"), &context);
                }
                other => bail!("{context}: expected an Upstream error, got {other:?}"),
            }
        }
    }

    // A transport error: nothing listens on the port.
    let closed = {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        Url::parse(&format!("http://{}/", listener.local_addr()?))?
    };
    for (method, result) in every_method(dyn_api, &credentials(closed)).await? {
        match result {
            Err(error @ GitLabError::Upstream(_)) => {
                assert_secret_free(&error.to_string(), method);
                assert_secret_free(&format!("{error:?}"), method);
            }
            other => bail!("{method} with nothing listening: expected Upstream, got {other:?}"),
        }
    }

    // The values that hold a secret, after a successful create.
    let creating =
        Mock::start(|_, _| Reply::json(201, &json!({ "id": 9001, "token": RUNNER_TOKEN }))).await?;
    let credentials = creating.credentials();
    let runner = api
        .create_runner(&credentials, &project_scope()?, &[], false, "runner")
        .await?;
    assert_eq!(runner.token.expose_secret(), RUNNER_TOKEN);
    assert_secret_free(&format!("{runner:?}"), "CreatedRunner Debug");
    assert_secret_free(&format!("{credentials:?}"), "GitLabCredentials Debug");
    assert_secret_free(&format!("{api:?}"), "GitLabClient Debug");
    Ok(())
}
