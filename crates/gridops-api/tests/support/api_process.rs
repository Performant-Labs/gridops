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
//!
//! How long the API takes to come up is not a reason to give up on it. On a
//! loaded CI runner it can take many seconds, and another port is no faster.
//! While the API the fixture started is alive and not yet healthy, an attempt
//! keeps waiting: the API either binds and answers, or exits on its bind
//! error, which moves the fixture to another port at once. Only
//! [`HANG_GUARD`] bounds the wait.

use std::{net::TcpListener, process::Child, time::Duration};

use anyhow::{Context as _, Result, bail};
use reqwest::{Client, StatusCode, header};

/// The most ports one start tries before it gives up.
const ATTEMPTS: usize = 5;

/// How often an attempt asks whether the API is up yet.
const POLL: Duration = Duration::from_millis(50);

/// The longest one attempt waits for the API it started to be healthy and to
/// answer the session check. A guard against a hung API or a port that
/// accepts and never answers, not the readiness signal: readiness is a
/// healthy answer from a live child that knows the fixture's session, and a
/// child that cannot bind exits and ends the attempt long before this.
const HANG_GUARD: Duration = Duration::from_secs(60);

/// Starts the API on each port `ports` offers in turn, until one attempt is
/// proven to be the fixture's own, and returns that API and its origin.
///
/// - `ports`: each attempt's port; `None` chooses a free one. A fixture that
///   must keep its origin (a restart) offers only that port.
/// - `cookie`: the session cookie the fixture seeded in its own database.
/// - `spawn`: starts the API with the given origin as its base URL and bind.
///
/// An attempt that fails is killed and reaped. After `ATTEMPTS` failures, or
/// once `ports` runs out, the error names every origin tried and why it was
/// not the fixture's own.
pub async fn start(
    client: &Client,
    ports: impl IntoIterator<Item = Option<u16>>,
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
        let proven =
            tokio::time::timeout(HANG_GUARD, prove_own(client, &mut child, &origin, cookie))
                .await
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!(
                        "no healthy API of the fixture's own within {HANG_GUARD:?}"
                    ))
                });
        match proven {
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

/// Waits, for as long as `child` runs, until something on `origin` is
/// healthy, then proves it is `child`: `child` is still running and knows the
/// fixture's session. The caller bounds the whole of it with [`HANG_GUARD`].
async fn prove_own(client: &Client, child: &mut Child, origin: &str, cookie: &str) -> Result<()> {
    loop {
        if let Some(status) = child.try_wait()? {
            bail!("the API exited ({status}) before it was healthy");
        }
        if client
            .get(format!("{origin}/api/health"))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            break;
        }
        tokio::time::sleep(POLL).await;
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
