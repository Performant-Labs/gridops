//! Starts the `gridops-api` binary for a test fixture and proves that the API
//! answering on its port is the process the fixture started (#124).
//!
//! Shared by the binary-level fixtures in `tests/control_plane_contracts.rs`
//! and `tests/fleet_http.rs`, which both include this file with `#[path]`.
//!
//! A fixture chooses a port by binding `127.0.0.1:0` and letting it go, so the
//! port is free until the API binds it, and a restart leaves it free while the
//! API is down. Another process can take it meanwhile: another fixture's API,
//! another test binary, another CI job on the host. The API then exits on its
//! bind error and the other process answers the health check and every
//! request. A health check cannot tell, so each attempt also needs the API it
//! started still running and a `GET /api/v1/auth/me` with a session only the
//! fixture's own database holds to answer 200. The fixture seeds that session
//! before it starts the API, once, so an attempt on another port reuses it.

use std::{net::TcpListener, process::Child, time::Duration};

use anyhow::{Context as _, Result, bail};
use reqwest::{Client, StatusCode, header};

/// The most ports one start tries before it gives up.
const ATTEMPTS: usize = 5;

/// How often an attempt asks whether the API is up yet.
const POLL: Duration = Duration::from_millis(50);

/// Starts the API on each port `ports` offers in turn, until one attempt is
/// proven to be the fixture's own, and returns that API and its origin.
///
/// - `ports`: each attempt's port; `None` chooses a free one. A fixture that
///   must keep its origin (a restart) offers only that port.
/// - `health_wait`: how long one attempt waits for the API to be healthy.
/// - `cookie`: the session cookie the fixture seeded in its own database.
/// - `spawn`: starts the API with the given origin as its base URL and bind.
///
/// An attempt that fails is killed and reaped. After `ATTEMPTS` failures, or
/// once `ports` runs out, the error names every origin tried and why it was
/// not the fixture's own.
pub async fn start(
    client: &Client,
    ports: impl IntoIterator<Item = Option<u16>>,
    health_wait: Duration,
    cookie: &str,
    mut spawn: impl FnMut(&str) -> Result<Child>,
) -> Result<(Child, String)> {
    let mut failures = Vec::new();
    for port in ports.into_iter().take(ATTEMPTS) {
        let port = match port {
            Some(port) => port,
            None => TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port(),
        };
        let origin = format!("http://127.0.0.1:{port}");
        let mut child =
            spawn(&origin).with_context(|| format!("could not start the API on {origin}"))?;
        match prove_own(client, &mut child, &origin, health_wait, cookie).await {
            Ok(()) => return Ok((child, origin)),
            Err(cause) => {
                let _ = child.kill();
                let _ = child.wait();
                failures.push(format!("{origin}: {cause:#}"));
            }
        }
    }
    bail!(
        "the API fixture got no API of its own after {} attempts: {}",
        failures.len(),
        failures.join("; ")
    )
}

/// Waits until something on `origin` is healthy, then proves it is `child`:
/// `child` is still running and knows the fixture's session.
async fn prove_own(
    client: &Client,
    child: &mut Child,
    origin: &str,
    health_wait: Duration,
    cookie: &str,
) -> Result<()> {
    let polls = (health_wait.as_millis() / POLL.as_millis()).max(1);
    let mut healthy = false;
    for _ in 0..polls {
        if let Some(status) = child.try_wait()? {
            bail!("the API exited ({status}) before it was healthy");
        }
        if client
            .get(format!("{origin}/api/health"))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            healthy = true;
            break;
        }
        tokio::time::sleep(POLL).await;
    }
    if !healthy {
        bail!("the API was not healthy within {health_wait:?}");
    }
    if let Some(status) = child.try_wait()? {
        bail!("another process answers on {origin}: the API exited ({status})");
    }
    let me = client
        .get(format!("{origin}/api/v1/auth/me"))
        .header(header::COOKIE, cookie)
        .send()
        .await
        .with_context(|| format!("no answer on {origin} to the session check"))?;
    if me.status() != StatusCode::OK {
        bail!(
            "another process answers on {origin}: it does not know the fixture's session ({})",
            me.status()
        );
    }
    Ok(())
}
