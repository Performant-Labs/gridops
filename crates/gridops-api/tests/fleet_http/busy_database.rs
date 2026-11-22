//! A database too busy to answer must not turn into a reconnect storm. On
//! 2026-11-21 the API answered agents 503 while `SQLite`'s write lock was
//! held, every 503 ended the agent's session, and each reconnect (handshake,
//! full inventory, new commands) added write load (#179). A heartbeat or an
//! inventory refresh the API can't answer is now resent, unchanged, within
//! the observation lease; everything else still ends the session as before.
//!
//! The real agent talks to the API binary through the recovery proxy, which
//! fails the requests a test picks and records every request it answers.

use std::{sync::Arc, time::Instant};

use gridops_host_agent::supervisor::Stop;

use super::{
    placement::{FakeDocker, Machine, active_machine, prepared_placement, seed_pool},
    recovery::{Proxy, Seen, agent_session, within},
    *,
};

/// The agent never backs off for less than a second; this allows for
/// scheduling.
const MIN_GAP: Duration = Duration::from_millis(900);

/// An active machine whose agent reaches the API through a proxy, with the
/// session it opened.
struct Proxied {
    f: Fixture,
    proxy: Proxy,
    machine: Machine,
    host: String,
    session: String,
}

async fn proxied_machine() -> Result<Proxied> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let proxy = Proxy::start(&f.origin).await?;
    let machine = active_machine(&f, &proxy.origin, Arc::new(FakeDocker::default())).await?;
    let host = machine.host.to_string();
    let session = agent_session(&f, &host).await?.context("agent session")?;
    assert_eq!(handshakes(&proxy), 1);
    Ok(Proxied {
        f,
        proxy,
        machine,
        host,
        session,
    })
}

fn handshakes(proxy: &Proxy) -> usize {
    proxy.faults.seen("/handshake").len()
}

async fn wait_for<T>(
    limit: Duration,
    what: &str,
    mut probe: impl FnMut() -> Option<T>,
) -> Result<T> {
    let started = Instant::now();
    loop {
        if let Some(value) = probe() {
            return Ok(value);
        }
        if started.elapsed() > limit {
            bail!("timed out after {limit:?} waiting for {what}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn shut_down(machine: Machine) -> Result<()> {
    let _ = machine.stop.send(());
    assert_eq!(machine.running.await??, Stop::Shutdown);
    Ok(())
}

/// The API fails `endpoint` three times, then answers. The agent keeps its
/// session and resends the same request, byte for byte, backing off between
/// attempts.
async fn resent_unchanged(endpoint: &'static str) -> Result<()> {
    let p = proxied_machine().await?;
    let armed = Instant::now();
    p.proxy
        .faults
        .fail(endpoint, StatusCode::SERVICE_UNAVAILABLE, 3);
    // An inventory refresh comes 30 s into the session.
    let attempts = wait_for(
        Duration::from_secs(90),
        "three failed attempts and an answered one",
        || {
            let sent: Vec<Seen> = p
                .proxy
                .faults
                .seen(endpoint)
                .into_iter()
                .filter(|seen| seen.at >= armed)
                .collect();
            let first = sent
                .iter()
                .position(|seen| seen.status == StatusCode::SERVICE_UNAVAILABLE)?;
            let attempts = sent.get(first..first + 4)?;
            Some(attempts.to_vec())
        },
    )
    .await?;

    assert_eq!(
        handshakes(&p.proxy),
        1,
        "a failed {endpoint} ended the session: the agent handshook again"
    );
    assert_eq!(
        agent_session(&p.f, &p.host).await?.as_deref(),
        Some(p.session.as_str())
    );
    let statuses: Vec<StatusCode> = attempts.iter().map(|seen| seen.status).collect();
    assert_eq!(
        statuses,
        [
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::OK
        ]
    );
    for pair in attempts.windows(2) {
        assert_eq!(
            pair[1].body, pair[0].body,
            "{endpoint} was resent with a different body"
        );
        let gap = pair[1].at - pair[0].at;
        assert!(
            gap >= MIN_GAP,
            "{endpoint} was resent after {gap:?}, without backing off"
        );
    }
    let body: Value = serde_json::from_slice(&attempts[0].body)?;
    let sealed = if endpoint == "/inventory" {
        &body["report"]
    } else {
        &body
    };
    assert_eq!(sealed["authority_session_id"], p.session.as_str());
    shut_down(p.machine).await
}

#[tokio::test]
async fn a_heartbeat_the_api_cannot_answer_is_resent_unchanged_in_the_same_session() -> Result<()> {
    resent_unchanged("/heartbeat").await
}

#[tokio::test]
async fn an_inventory_refresh_the_api_cannot_answer_is_resent_unchanged_in_the_same_session()
-> Result<()> {
    resent_unchanged("/inventory").await
}

/// The agent resends a heartbeat only until the observation lease it held
/// when the outage began runs out. Then the session ends and the agent
/// reconnects, as it always did.
#[tokio::test]
async fn a_heartbeat_outage_that_outlasts_the_lease_ends_the_session() -> Result<()> {
    let p = proxied_machine().await?;
    p.proxy.faults.shorten_leases(4_000);
    wait_for(Duration::from_secs(10), "two short leases", || {
        (p.proxy
            .faults
            .seen("/heartbeat")
            .iter()
            .filter(|seen| seen.lease_expires_at.is_some())
            .count()
            >= 2)
            .then_some(())
    })
    .await?;
    p.proxy
        .faults
        .fail("/heartbeat", StatusCode::SERVICE_UNAVAILABLE, usize::MAX);
    wait_for(Duration::from_secs(45), "a second handshake", || {
        (handshakes(&p.proxy) >= 2).then_some(())
    })
    .await?;
    p.proxy.faults.heal("/heartbeat");

    let heartbeats = p.proxy.faults.seen("/heartbeat");
    let outage = heartbeats
        .iter()
        .find(|seen| seen.status == StatusCode::SERVICE_UNAVAILABLE)
        .context("a failed heartbeat")?
        .at;
    // The lease of the last heartbeat answered before the outage.
    let deadline = heartbeats
        .iter()
        .filter(|seen| seen.at < outage && seen.status.is_success())
        .filter_map(|seen| seen.lease_expires_at)
        .next_back()
        .context("a short lease granted before the outage")?;
    let shakes = p.proxy.faults.seen("/handshake");
    assert_eq!(
        shakes
            .iter()
            .filter(|seen| seen.at_millis < deadline)
            .count(),
        1,
        "the agent handshook again {} ms before the lease it held ran out",
        deadline - shakes[1].at_millis
    );
    assert!(shakes[1].at_millis >= deadline);
    let first: Value = serde_json::from_slice(&shakes[0].body)?;
    let second: Value = serde_json::from_slice(&shakes[1].body)?;
    assert_ne!(first["agent_session_id"], second["agent_session_id"]);
    within("the new session", || async {
        Ok(agent_session(&p.f, &p.host)
            .await?
            .filter(|session| *session != p.session)
            .map(|_| ()))
    })
    .await?;
    shut_down(p.machine).await
}

/// A rejected credential is never retried: the agent stops for recovery.
#[tokio::test]
async fn a_heartbeat_refused_as_unauthenticated_still_stops_the_agent() -> Result<()> {
    // The fixture is the API process: it must outlive the agent.
    let Proxied {
        f: _api,
        proxy,
        machine,
        ..
    } = proxied_machine().await?;
    let Machine {
        stop: _keep_running,
        running,
        ..
    } = machine;
    proxy.faults.fail("/heartbeat", StatusCode::UNAUTHORIZED, 1);
    let stopped = tokio::time::timeout(Duration::from_secs(20), running).await???;
    assert_eq!(stopped, Stop::CredentialRejected);
    assert_eq!(handshakes(&proxy), 1);
    Ok(())
}

/// A result post that fails still ends the session: the API ties no result
/// to a session, so a retry could loop on a result it can never take. The
/// next session delivers it from the journal.
#[tokio::test]
async fn a_result_the_api_cannot_take_still_ends_the_session() -> Result<()> {
    let p = proxied_machine().await?;
    p.proxy
        .faults
        .fail("/commands/result", StatusCode::SERVICE_UNAVAILABLE, 1);
    prepared_placement(&p.f, &p.machine, Uuid::new_v4()).await?;
    within("a new session", || async {
        Ok(agent_session(&p.f, &p.host)
            .await?
            .filter(|session| *session != p.session)
            .map(|_| ()))
    })
    .await?;
    assert_eq!(handshakes(&p.proxy), 2);
    let results: Vec<StatusCode> = p
        .proxy
        .faults
        .seen("/commands/result")
        .iter()
        .map(|seen| seen.status)
        .collect();
    assert_eq!(results, [StatusCode::SERVICE_UNAVAILABLE, StatusCode::OK]);
    shut_down(p.machine).await
}

/// The incident itself: another writer holds `SQLite`'s write lock past the
/// 5 s busy timeout, so the API answers the agent's heartbeat 503. With no
/// open command the poll only reads. The session survives the outage and
/// carries on once the lock is released.
#[tokio::test]
async fn a_database_held_past_the_busy_timeout_leaves_the_session_alone() -> Result<()> {
    let p = proxied_machine().await?;
    let open: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM host_commands WHERE host_id=? AND state IN ('pending','delivered')",
    )
    .bind(&p.host)
    .fetch_one(&p.f.database)
    .await?;
    assert_eq!(open, 0, "an open command would make the poll write");
    let database = p.f.database.clone();
    let sessions = || async {
        Ok::<i64, anyhow::Error>(
            sqlx::query_scalar("SELECT COUNT(*) FROM fleet_observation_sessions WHERE host_id=?")
                .bind(&p.host)
                .fetch_one(&database)
                .await?,
        )
    };
    let sessions_before = sessions().await?;

    let writer = p.f.database.begin_with("BEGIN IMMEDIATE").await?;
    let held = Instant::now();
    tokio::time::sleep(Duration::from_millis(7_500)).await;
    let released = Instant::now();
    writer.rollback().await?;

    assert!(
        p.proxy
            .faults
            .seen("/heartbeat")
            .iter()
            .any(|seen| seen.at >= held && seen.status == StatusCode::SERVICE_UNAVAILABLE),
        "no heartbeat failed while the lock was held, so this proves nothing"
    );
    wait_for(
        Duration::from_secs(35),
        "a heartbeat answered after the release",
        || {
            p.proxy
                .faults
                .seen("/heartbeat")
                .iter()
                .any(|seen| seen.answered >= released && seen.status.is_success())
                .then_some(())
        },
    )
    .await?;
    assert_eq!(
        handshakes(&p.proxy),
        1,
        "the busy database ended the session: the agent handshook again"
    );
    assert_eq!(sessions().await?, sessions_before);
    assert_eq!(
        agent_session(&p.f, &p.host).await?.as_deref(),
        Some(p.session.as_str())
    );
    shut_down(p.machine).await
}
