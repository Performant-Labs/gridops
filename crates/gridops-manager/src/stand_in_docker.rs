//! A stand-in Docker engine on a Unix socket for the manager's tests: enough
//! of the Engine API for a Docker admission, a runner start, a container start
//! and a sandbox creation to succeed. It knows one container, `CONTAINER_ID`,
//! and lists none, unless a test gives it `containers` to list. It records
//! what a runner start sent it (`recorded`).

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
    net::{UnixListener, UnixStream},
};

/// The one container the stand-in engine knows.
pub(crate) const CONTAINER_ID: &str = "c0ffee0123456789";

/// How the stand-in engine treats images.
#[derive(Clone, Default)]
pub(crate) struct Images {
    /// `None`: every image is already there. `Some(delay)`: none is, and
    /// pulling one takes `delay`.
    pub(crate) pull: Option<Duration>,
    /// Pulls the engine is working on now.
    pub(crate) pulling: Arc<AtomicUsize>,
    /// `GET /containers/json` lists `CONTAINER_ID`, a running managed runner,
    /// instead of nothing.
    pub(crate) listed: bool,
    /// When not empty, `GET /containers/json` lists these instead, and
    /// `GET /containers/{id}/json` answers each with its state and exit code.
    pub(crate) containers: Vec<StandInContainer>,
    /// What runner starts sent the engine.
    pub(crate) recorded: Recorded,
}

/// A container the stand-in engine lists and inspects.
#[derive(Clone)]
pub(crate) struct StandInContainer {
    pub(crate) id: String,
    /// `running` or `exited`.
    pub(crate) state: &'static str,
    pub(crate) exit_code: i64,
    pub(crate) labels: Vec<(&'static str, String)>,
}

/// Every `POST /containers/create` body, and every byte written to an
/// attached container's stdin.
#[derive(Clone, Default)]
pub(crate) struct Recorded {
    pub(crate) creates: Arc<Mutex<Vec<serde_json::Value>>>,
    pub(crate) stdin: Arc<Mutex<Vec<u8>>>,
}

/// Starts the engine on the current Tokio runtime; answers the path of its
/// socket, which the caller removes.
pub(crate) fn start(images: Images) -> anyhow::Result<PathBuf> {
    let socket = std::env::temp_dir().join(format!(
        "gridops-stand-in-docker-{}.sock",
        uuid::Uuid::new_v4()
    ));
    let listener = UnixListener::bind(&socket)?;
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve_connection(stream, images.clone()));
        }
    });
    Ok(socket)
}

/// Answers HTTP/1.1 requests on one connection until the client closes it.
async fn serve_connection(stream: UnixStream, images: Images) -> std::io::Result<()> {
    let mut stream = BufReader::new(stream);
    loop {
        let mut request_line = String::new();
        if stream.read_line(&mut request_line).await? == 0 {
            return Ok(());
        }
        let mut content_length = 0_usize;
        loop {
            let mut line = String::new();
            if stream.read_line(&mut line).await? == 0 {
                return Ok(());
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                content_length = value.trim().parse().unwrap_or_default();
            }
        }
        let mut body = vec![0; content_length];
        stream.read_exact(&mut body).await?;

        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_owned();
        let target = parts.next().unwrap_or_default();
        // `/v1.49/containers/json?all=true` -> `/containers/json`
        let path = target.split('?').next().unwrap_or_default();
        let path = match path.strip_prefix("/v") {
            Some(rest) => rest.find('/').map_or(path, |slash| &rest[slash..]),
            None => path,
        }
        .to_owned();

        if method == "POST" && path.ends_with("/attach") {
            // The manager writes the runner's credential to stdin, then
            // closes it; read until it does.
            stream
                .get_mut()
                .write_all(
                    b"HTTP/1.1 101 UPGRADED\r\nContent-Type: application/vnd.docker.raw-stream\r\nConnection: Upgrade\r\nUpgrade: tcp\r\n\r\n",
                )
                .await?;
            let mut sink = Vec::new();
            let _ = stream.read_to_end(&mut sink).await;
            if let Ok(mut stdin) = images.recorded.stdin.lock() {
                stdin.extend_from_slice(&sink);
            }
            return Ok(());
        }
        if method == "POST"
            && path == "/images/create"
            && let Some(delay) = images.pull
        {
            images.pulling.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(delay).await;
            images.pulling.fetch_sub(1, Ordering::SeqCst);
        }
        if method == "POST" && path == "/containers/create" {
            if let (Ok(body), Ok(mut creates)) = (
                serde_json::from_slice::<serde_json::Value>(&body),
                images.recorded.creates.lock(),
            ) {
                creates.push(body);
            }
        }
        let (status, answer) = if images.containers.is_empty() {
            answer(&method, &path, images.pull.is_none(), images.listed)
        } else {
            listed_answer(&method, &path, &images.containers)
                .unwrap_or_else(|| answer(&method, &path, images.pull.is_none(), images.listed))
        };
        let response = if answer.is_empty() {
            format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n")
        } else {
            format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{answer}",
                answer.len()
            )
        };
        stream.get_mut().write_all(response.as_bytes()).await?;
    }
}

fn answer(method: &str, path: &str, images_present: bool, listed: bool) -> (&'static str, String) {
    let container_path = format!("/containers/{CONTAINER_ID}");
    match (method, path) {
        ("GET", "/containers/json") if listed => (
            "200 OK",
            json!([{
                "Id": CONTAINER_ID,
                "Names": ["/pool-1-abcd1234"],
                "Image": "runner:fixture",
                "State": "running",
                "Status": "Up 5 minutes",
                "Created": 1_762_128_000_i64,
                "Labels": {
                    "io.gridops.managed": "true",
                    "io.gridops.runner-id": "runner-docker-1",
                    "io.gridops.pool-id": "pool-docker",
                    "io.gridops.mode": "persistent",
                },
            }])
            .to_string(),
        ),
        ("GET", "/containers/json") => ("200 OK", "[]".into()),
        ("GET", path) if path.starts_with("/networks/") => {
            ("200 OK", json!({ "Name": "gridops-runners" }).to_string())
        }
        ("GET", path) if path.starts_with("/images/") && images_present => {
            ("200 OK", json!({ "Id": "sha256:fixture" }).to_string())
        }
        ("POST", "/images/create") => (
            "200 OK",
            json!({ "status": "Downloaded newer image" }).to_string(),
        ),
        ("POST", "/containers/create") => (
            "201 Created",
            json!({ "Id": CONTAINER_ID, "Warnings": [] }).to_string(),
        ),
        ("POST", path) if path == format!("{container_path}/start") => {
            ("204 No Content", String::new())
        }
        ("GET", path) if path == format!("{container_path}/json") => (
            "200 OK",
            json!({
                "Id": CONTAINER_ID,
                "Name": "/pool-1-abcd1234",
                "Created": "2026-11-03T00:00:00Z",
                "State": { "Status": "running" },
                "HostConfig": { "NanoCpus": 2_000_000_000_i64, "Memory": 2_147_483_648_i64 },
                "Config": { "Labels": {
                    "io.gridops.managed": "true",
                    "io.gridops.runner-id": "runner-docker-1",
                    "io.gridops.pool-id": "pool-docker",
                    "io.gridops.mode": "persistent",
                } },
            })
            .to_string(),
        ),
        _ => (
            "404 Not Found",
            json!({ "message": format!("no such object: {method} {path}") }).to_string(),
        ),
    }
}

/// The list and inspect answers for `containers`; `None` for anything else.
fn listed_answer(
    method: &str,
    path: &str,
    containers: &[StandInContainer],
) -> Option<(&'static str, String)> {
    let labels = |container: &StandInContainer| {
        container
            .labels
            .iter()
            .map(|(name, value)| ((*name).to_owned(), json!(value)))
            .collect::<serde_json::Map<_, _>>()
    };
    if method != "GET" {
        return None;
    }
    if path == "/containers/json" {
        let listed = containers
            .iter()
            .map(|container| {
                json!({
                    "Id": container.id,
                    "Names": [format!("/runner-{}", container.id)],
                    "Image": "registry.example/gitlab-runner:pool-pinned",
                    "State": container.state,
                    "Status": if container.state == "running" {
                        "Up 5 minutes".to_owned()
                    } else {
                        format!("Exited ({}) 1 minute ago", container.exit_code)
                    },
                    "Created": 1_762_128_000_i64,
                    "Labels": labels(container),
                })
            })
            .collect::<Vec<_>>();
        return Some(("200 OK", json!(listed).to_string()));
    }
    let container = containers.iter().find(|container| {
        path.strip_prefix("/containers/")
            .and_then(|rest| rest.strip_suffix("/json"))
            == Some(container.id.as_str())
    })?;
    Some((
        "200 OK",
        json!({
            "Id": container.id,
            "Name": format!("/runner-{}", container.id),
            "Created": "2026-11-03T00:00:00Z",
            "State": {
                "Status": container.state,
                "Running": container.state == "running",
                "ExitCode": container.exit_code,
            },
            "Config": { "Labels": labels(container) },
        })
        .to_string(),
    ))
}
