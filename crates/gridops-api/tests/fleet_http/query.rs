//! Exercises fleet read routes through the actual API binary.
//! Physical host identity, execution backend identity and read permissions stay distinct.

use super::{inventory::*, *};

async fn observed_fixture() -> Result<(Fixture, String)> {
    let f = Fixture::new().await?;
    let (host, secret) = enrolled(&f).await?;
    let incarnation = control_plane(&f).await?;
    let handshake = receipt(
        agent_post(&f, &host, &secret, "handshake")
            .json(&handshake_body())
            .send()
            .await?,
    )
    .await?;
    receipt(
        agent_post(&f, &host, &secret, "inventory")
            .json(&inventory_body(&handshake, &incarnation))
            .send()
            .await?,
    )
    .await?;
    Ok((f, host))
}

fn get(f: &Fixture, path: &str) -> reqwest::RequestBuilder {
    f.client.get(f.url(path)).header(header::COOKIE, &f.cookie)
}

fn query_path(path: &str, pairs: &[(&str, &str)]) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs.iter().copied())
        .finish();
    format!("{path}?{query}")
}

fn timestamp(millis: i64) -> Result<String> {
    Ok(chrono::DateTime::from_timestamp_millis(millis)
        .context("test timestamp")?
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

async fn state(
    f: &Fixture,
    host: &str,
) -> Result<(i64, i64, Option<i64>, Option<i64>, Option<i64>, i64, i64)> {
    Ok(sqlx::query_as("SELECT revision,inventory_revision,last_heartbeat_at,last_inventory_at,authority_expires_at,(SELECT COUNT(*) FROM fleet_events),(SELECT COUNT(*) FROM host_samples) FROM fleet_hosts WHERE id=?")
        .bind(host).fetch_one(&f.database).await?)
}

#[tokio::test]
async fn fleet_reads_keep_windows_host_and_linux_backend_identity_separate() -> Result<()> {
    let (f, host) = observed_fixture().await?;
    let before = state(&f, &host).await?;
    // A fixed old timestamp detects even authentication-side activity writes.
    sqlx::query("UPDATE sessions SET last_seen_at=1 WHERE id='fleet-session'")
        .execute(&f.database)
        .await?;
    let catalog = receipt(get(&f, "/hosts").send().await?).await?;
    assert_eq!(catalog["schemaVersion"], 1);
    assert_eq!(catalog["items"].as_array().context("host items")?.len(), 1);
    assert_eq!(catalog["items"][0]["id"], host);
    assert_eq!(catalog["items"][0]["hostOs"], "windows");
    assert_eq!(catalog["nextCursor"], Value::Null);

    let detail = receipt(get(&f, &format!("/hosts/{host}")).send().await?).await?;
    assert_eq!(detail["item"]["projection"], "host");
    assert_eq!(detail["item"]["hostOs"], "windows");
    assert_eq!(detail["item"]["backendCount"], 2);
    assert_eq!(detail["item"]["approvalEvidence"]["state"], "available");
    assert_eq!(detail["item"]["physicalBudget"]["cpuMillis"], 0);
    assert_eq!(
        detail["item"]["latestRootMeasurement"]["provenance"],
        "current"
    );
    assert_eq!(
        detail["item"]["latestRootMeasurement"]["coverage"],
        "partial"
    );
    assert_eq!(
        detail["item"]["latestRootMeasurement"]["cpuUsedMillis"],
        Value::Null
    );

    let first = receipt(
        get(&f, &format!("/hosts/{host}/backends?limit=1"))
            .send()
            .await?,
    )
    .await?;
    let cursor = first["nextCursor"].as_str().context("backend cursor")?;
    assert!(cursor.starts_with("q1."));
    assert!(cursor.len() <= 4096);
    let second = receipt(
        get(
            &f,
            &query_path(
                &format!("/hosts/{host}/backends"),
                &[("limit", "1"), ("cursor", cursor)],
            ),
        )
        .send()
        .await?,
    )
    .await?;
    let mut backends = vec![first["items"][0].clone(), second["items"][0].clone()];
    backends.sort_by_key(|item| item["runtimeKind"].as_str().unwrap_or_default().to_owned());
    assert_eq!(backends[0]["runtimeKind"], "docker");
    assert_eq!(backends[0]["executionOs"], "linux");
    assert_eq!(backends[1]["runtimeKind"], "native_process");
    assert_eq!(backends[1]["executionOs"], "windows");
    assert_eq!(second["nextCursor"], Value::Null);
    for backend in &backends {
        assert_eq!(backend["hostId"], host);
        let id = backend["id"].as_str().context("backend ID")?;
        let item = receipt(
            get(&f, &format!("/hosts/{host}/backends/{id}"))
                .send()
                .await?,
        )
        .await?;
        assert_eq!(&item["item"], backend);
    }
    error(
        get(
            &f,
            &query_path(
                &format!("/hosts/{host}/backends"),
                &[("limit", "2"), ("cursor", cursor)],
            ),
        )
        .send()
        .await?,
        StatusCode::BAD_REQUEST,
        "invalid_request",
    )
    .await?;

    let domains = receipt(get(&f, &format!("/hosts/{host}/domains")).send().await?).await?;
    assert_eq!(
        domains["items"].as_array().context("domain items")?.len(),
        2
    );
    for domain in domains["items"].as_array().context("domain items")? {
        let id = domain["id"].as_str().context("domain ID")?;
        let result = receipt(
            get(&f, &format!("/hosts/{host}/domains/{id}"))
                .send()
                .await?,
        )
        .await?;
        assert_eq!(&result["item"], domain);
    }
    let now = gridops_core::now_millis();
    let from = timestamp(now - 60_000)?;
    let to = timestamp(now + 1000)?;
    let samples = receipt(
        get(
            &f,
            &query_path(
                &format!("/hosts/{host}/samples"),
                &[("receivedFrom", &from), ("receivedTo", &to)],
            ),
        )
        .send()
        .await?,
    )
    .await?;
    assert_eq!(
        samples["items"].as_array().context("sample items")?.len(),
        1
    );
    assert_eq!(samples["items"][0]["coverage"], "partial");
    assert_eq!(samples["items"][0]["cpuUsedMillis"], Value::Null);
    assert_eq!(samples["items"][0]["memoryUsedMib"], 1024);
    assert_eq!(samples["items"][0]["provenance"], "current");
    let events = receipt(
        get(&f, &format!("/hosts/{host}/events?limit=1"))
            .send()
            .await?,
    )
    .await?;
    assert_eq!(events["items"].as_array().context("event items")?.len(), 1);
    let event = &events["items"][0];
    assert!(event["id"].as_str().is_some());
    assert!(
        event["cursor"]
            .as_str()
            .context("event cursor")?
            .starts_with("q1.")
    );
    assert!(event.get("detailJson").is_none());
    assert!(event.get("actorUserId").is_none());
    assert_eq!(state(&f, &host).await?, before);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT last_seen_at FROM sessions WHERE id='fleet-session'")
            .fetch_one(&f.database)
            .await?,
        1
    );
    // Legacy session activity behavior is preserved outside fleet query reads.
    let legacy = f
        .client
        .get(format!("{}/api/v1/auth/session", f.origin))
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?;
    assert_eq!(legacy.status(), StatusCode::OK);
    assert!(
        sqlx::query_scalar::<_, i64>("SELECT last_seen_at FROM sessions WHERE id='fleet-session'")
            .fetch_one(&f.database)
            .await?
            > 1
    );
    Ok(())
}

#[tokio::test]
async fn fleet_read_authentication_and_query_rejections_keep_safe_envelopes() -> Result<()> {
    let (f, host) = observed_fixture().await?;
    sqlx::query("UPDATE sessions SET last_seen_at=1 WHERE id='fleet-session'")
        .execute(&f.database)
        .await?;
    error(
        f.client.get(f.url("/hosts")).send().await?,
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
    )
    .await?;
    error(
        f.client
            .get(f.url("/hosts"))
            .bearer_auth("synthetic-agent")
            .send()
            .await?,
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
    )
    .await?;
    error(
        get(&f, "/hosts")
            .bearer_auth("synthetic-agent")
            .send()
            .await?,
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
    )
    .await?;
    for cookie in [
        "gridops_session=invalid.signature",
        "gridops_session=missing",
    ] {
        error(
            f.client
                .get(f.url("/hosts"))
                .header(header::COOKIE, cookie)
                .send()
                .await?,
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
        )
        .await?;
    }
    let expired_cookie = other_admin_cookie(&f).await?;
    sqlx::query("UPDATE sessions SET expires_at=? WHERE id='other-session'")
        .bind(gridops_core::now_millis() - 1)
        .execute(&f.database)
        .await?;
    error(
        f.client
            .get(f.url("/hosts"))
            .header(header::COOKIE, expired_cookie)
            .send()
            .await?,
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
    )
    .await?;
    // Reads retain ordinary cookie authentication and need no mutation Origin.
    receipt(
        get(&f, "/hosts")
            .header(header::ORIGIN, "https://unrelated.example")
            .send()
            .await?,
    )
    .await?;
    for query in [
        "limit=0",
        "limit=101",
        "limit=-1",
        "limit=18446744073709551616",
        "limit=1&limit=2",
        "limit=1&%6cimit=2",
        "unknown=1",
        "cursor=bad",
        "cursor=%FF",
        "limit=%",
        "limit=%GG",
        "targetId=not-a-uuid",
    ] {
        error(
            get(&f, &format!("/hosts?{query}")).send().await?,
            StatusCode::BAD_REQUEST,
            "invalid_request",
        )
        .await?;
    }
    error(
        get(&f, &format!("/hosts?cursor={}", "a".repeat(8192)))
            .send()
            .await?,
        StatusCode::BAD_REQUEST,
        "invalid_request",
    )
    .await?;
    for path in [
        format!("/hosts/{host}?limit=1"),
        format!("/hosts/{host}/events?occurredFrom=2026-10-09T00:00:00Z"),
        format!("/hosts/{host}/events?occurredTo=2026-10-09T00:00:00Z"),
        format!(
            "/hosts/{host}/events?occurredFrom=2026-10-09T00:00:00Z&occurredTo=2026-10-09T00:00:00Z"
        ),
        format!("/hosts/{host}/samples"),
        format!("/hosts/{host}/samples?receivedFrom=bad&receivedTo=bad"),
        format!(
            "/hosts/{host}/samples?receivedFrom=2026-10-08T00:00:00Z&receivedTo=2026-10-09T00:00:01Z"
        ),
        format!(
            "/hosts/{host}/samples?receivedFrom=2026-10-09T00:00:00Z&receivedTo=2026-10-09T00:00:00Z"
        ),
        format!("/hosts/{host}/domains?targetId={}", Uuid::new_v4()),
        "/hosts/invalid".to_owned(),
    ] {
        error(
            get(&f, &path).send().await?,
            StatusCode::BAD_REQUEST,
            "invalid_request",
        )
        .await?;
    }
    error(
        get(&f, &format!("/hosts/{host}/")).send().await?,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await?;
    let unknown_target = Uuid::new_v4();
    for path in [
        format!("/hosts?targetId={unknown_target}"),
        format!("/hosts/{host}/backends?targetId={unknown_target}"),
        format!("/hosts/{host}/events?targetId={unknown_target}"),
    ] {
        error(
            get(&f, &path).send().await?,
            StatusCode::NOT_FOUND,
            "not_found",
        )
        .await?;
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT last_seen_at FROM sessions WHERE id='fleet-session'")
            .fetch_one(&f.database)
            .await?,
        1
    );
    // Malformed persisted resource values fail closed instead of being rounded.
    sqlx::query("UPDATE host_resource_domains SET cpu_millis=9223372036854775807 WHERE host_id=? AND parent_domain_id IS NULL")
        .bind(&host).execute(&f.database).await?;
    error(
        get(&f, &format!("/hosts/{host}")).send().await?,
        StatusCode::SERVICE_UNAVAILABLE,
        "dependency_unavailable",
    )
    .await?;
    sqlx::query("DELETE FROM fleet_control_plane WHERE singleton=1")
        .execute(&f.database)
        .await?;
    error(
        get(&f, "/hosts").send().await?,
        StatusCode::SERVICE_UNAVAILABLE,
        "dependency_unavailable",
    )
    .await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM fleet_control_plane")
            .fetch_one(&f.database)
            .await?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn physical_grant_revocation_and_cross_host_backend_paths_are_rechecked() -> Result<()> {
    let (f, host) = observed_fixture().await?;
    let member_cookie = other_admin_cookie(&f).await?;
    sqlx::query("UPDATE users SET role='member' WHERE id='other-admin'")
        .execute(&f.database)
        .await?;
    let member_get = |path: &str| {
        f.client
            .get(f.url(path))
            .header(header::COOKIE, &member_cookie)
    };
    let empty = receipt(member_get("/hosts").send().await?).await?;
    assert_eq!(empty["items"], json!([]));
    error(
        member_get(&format!("/hosts/{host}")).send().await?,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await?;
    sqlx::query(
        "INSERT INTO host_user_grants(host_id,user_id,permission) VALUES (?,'other-admin','read')",
    )
    .bind(&host)
    .execute(&f.database)
    .await?;
    let detail = receipt(member_get(&format!("/hosts/{host}")).send().await?).await?;
    assert_eq!(detail["item"]["projection"], "host");
    assert_eq!(detail["item"]["approvalEvidence"]["state"], "unavailable");
    assert_eq!(
        detail["item"]["approvalEvidence"]["reason"],
        "requires_system_admin"
    );
    let backends = receipt(
        member_get(&format!("/hosts/{host}/backends?limit=1"))
            .send()
            .await?,
    )
    .await?;
    let cursor = backends["nextCursor"].as_str().context("backend cursor")?;
    let backend = backends["items"][0]["id"].as_str().context("backend ID")?;

    let issued = f.issue("other-query-host").await?;
    let other: Value = f
        .consume(issued["code"].as_str().context("enrollment code")?)
        .await?
        .json()
        .await?;
    let other_id = other["host_id"].as_str().context("other host ID")?;
    error(
        get(&f, &format!("/hosts/{other_id}/backends/{backend}"))
            .send()
            .await?,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await?;
    error(
        member_get(&format!("/hosts/{other_id}/backends/{backend}"))
            .send()
            .await?,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await?;

    sqlx::query(
        "UPDATE host_user_grants SET revoked_at=? WHERE host_id=? AND user_id='other-admin'",
    )
    .bind(gridops_core::now_millis())
    .bind(&host)
    .execute(&f.database)
    .await?;
    error(
        member_get(&query_path(
            &format!("/hosts/{host}/backends"),
            &[("limit", "1"), ("cursor", cursor)],
        ))
        .send()
        .await?,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await?;
    error(
        member_get(&format!("/hosts/{host}/domains")).send().await?,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await?;
    let empty = receipt(member_get("/hosts").send().await?).await?;
    assert_eq!(empty["items"], json!([]));
    Ok(())
}

#[tokio::test]
async fn sealed_read_cursors_bind_user_resource_and_durable_retention() -> Result<()> {
    let (f, host) = observed_fixture().await?;
    let member_cookie = other_admin_cookie(&f).await?;
    sqlx::query("UPDATE users SET role='member' WHERE id='other-admin'")
        .execute(&f.database)
        .await?;
    sqlx::query(
        "INSERT INTO host_user_grants(host_id,user_id,permission) VALUES (?,'other-admin','read')",
    )
    .bind(&host)
    .execute(&f.database)
    .await?;
    let path = format!("/hosts/{host}/events");
    let page = receipt(get(&f, &format!("{path}?limit=1")).send().await?).await?;
    let cursor = page["nextCursor"].as_str().context("event continuation")?;
    let continuation = query_path(&path, &[("limit", "1"), ("cursor", cursor)]);
    error(
        f.client
            .get(f.url(&continuation))
            .header(header::COOKIE, member_cookie)
            .send()
            .await?,
        StatusCode::BAD_REQUEST,
        "invalid_request",
    )
    .await?;
    error(
        get(
            &f,
            &query_path(
                &format!("/hosts/{host}/backends"),
                &[("limit", "1"), ("cursor", cursor)],
            ),
        )
        .send()
        .await?,
        StatusCode::BAD_REQUEST,
        "invalid_request",
    )
    .await?;
    let second = receipt(get(&f, &continuation).send().await?).await?;
    assert_ne!(page["items"][0]["id"], second["items"][0]["id"]);
    assert_eq!(
        page["items"][0]["cursor"]
            .as_str()
            .context("first row cursor")?
            .len(),
        second["items"][0]["cursor"]
            .as_str()
            .context("second row cursor")?
            .len()
    );
    sqlx::query("UPDATE fleet_query_retention_floors SET first_retained=(SELECT MAX(cursor)+1 FROM fleet_events) WHERE stream='fleet_events'")
        .execute(&f.database).await?;
    error(
        get(&f, &continuation).send().await?,
        StatusCode::GONE,
        "cursor_gap",
    )
    .await?;

    let now = gridops_core::now_millis();
    for observed in [now + 60_000, now - 60_000] {
        sqlx::query("INSERT INTO host_samples(host_id,host_epoch,observed_at,received_at,coverage) VALUES (?,1,?,?,'unknown')")
            .bind(&host).bind(observed).bind(now).execute(&f.database).await?;
    }
    let from = timestamp(now - 60_000)?;
    let to = timestamp(now + 1000)?;
    let samples_path = format!("/hosts/{host}/samples");
    let page_path = query_path(
        &samples_path,
        &[("limit", "1"), ("receivedFrom", &from), ("receivedTo", &to)],
    );
    let page = receipt(get(&f, &page_path).send().await?).await?;
    let cursor = page["nextCursor"].as_str().context("sample continuation")?;
    let continuation = query_path(
        &samples_path,
        &[
            ("limit", "1"),
            ("receivedFrom", &from),
            ("receivedTo", &to),
            ("cursor", cursor),
        ],
    );
    let next = receipt(get(&f, &continuation).send().await?).await?;
    assert_eq!(next["items"][0]["coverage"], "unknown");
    assert_eq!(next["items"][0]["provenance"], "legacy_unknown");
    assert_eq!(next["items"][0]["observedAt"], timestamp(now + 60_000)?);
    sqlx::query("UPDATE fleet_query_retention_floors SET first_retained=(SELECT MAX(id)+1 FROM host_samples) WHERE stream='samples'")
        .execute(&f.database).await?;
    error(
        get(&f, &continuation).send().await?,
        StatusCode::GONE,
        "cursor_gap",
    )
    .await?;
    Ok(())
}
