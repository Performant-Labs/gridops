//! Retiring a machine with an API token: the guard rails that keep a leaked
//! `fleet:retire` token from stranding a working fleet, every reason named in
//! one refusal, and a browser (session) retire left exactly as it was.

use super::*;
use crate::fleet::protocol::browser::HostRetireBlocker as Blocker;

const MEMBER: &str = "registry-member";
/// The policy the tests run under: a machine must have been silent this long.
const QUIET: i64 = 15 * 60_000;
const NOW: i64 = 10_000_000;

fn policy() -> RetirePolicy {
    RetirePolicy {
        token_min_offline_millis: QUIET,
    }
}

fn token(user_id: &str) -> Principal {
    Principal::ApiToken {
        user_id: user_id.to_owned(),
        token_id: "retire-token".to_owned(),
        token_name: "retire-bot".to_owned(),
    }
}

/// An approved machine with the given lifecycle that last reported in at
/// `last_seen`.
async fn approved_machine(
    fixture: &Fixture,
    lifecycle: &str,
    last_seen: Option<i64>,
) -> Result<(HostId, String)> {
    let issue = fixture
        .issue_empty(&format!("retire-host-{}", Uuid::new_v4()))
        .await?;
    let enrolled = fixture.consume(&issue).await?;
    let id = enrolled.host_id.to_string();
    sqlx::query(
        "UPDATE fleet_hosts SET enrollment_state='approved',lifecycle_state=?,
           last_heartbeat_at=?,last_inventory_at=? WHERE id=?",
    )
    .bind(lifecycle)
    .bind(last_seen)
    .bind(last_seen)
    .bind(&id)
    .execute(&fixture.pool)
    .await?;
    Ok((enrolled.host_id, enrolled.credential.expose().to_owned()))
}

async fn revision(fixture: &Fixture, host: HostId) -> Result<Revision> {
    let value: i64 = sqlx::query_scalar("SELECT revision FROM fleet_hosts WHERE id=?")
        .bind(host.to_string())
        .fetch_one(&fixture.pool)
        .await?;
    Ok(Revision::new(u64::try_from(value)?)?)
}

async fn lifecycle(fixture: &Fixture, host: HostId) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT lifecycle_state FROM fleet_hosts WHERE id=?")
            .bind(host.to_string())
            .fetch_one(&fixture.pool)
            .await?,
    )
}

async fn retire_as(
    fixture: &Fixture,
    principal: &Principal,
    host: HostId,
) -> Result<(), RegistryError> {
    let expected = revision(fixture, host)
        .await
        .map_err(|_| RegistryError::NotFound)?;
    fixture
        .service
        .retire_host_with_policy(principal, host, expected, policy())
        .await
}

async fn refusal(fixture: &Fixture, host: HostId) -> Result<Vec<Blocker>> {
    match retire_as(fixture, &token(ADMIN), host).await {
        Err(RegistryError::HostNotRetirable(reasons)) => Ok(reasons),
        other => anyhow::bail!("expected a retire refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_token_retires_an_offline_paused_idle_machine_and_names_itself() -> Result<()> {
    let fixture = Fixture::new(NOW).await?;
    let (host, credential) = approved_machine(&fixture, "paused", Some(NOW - QUIET)).await?;

    retire_as(&fixture, &token(ADMIN), host).await?;

    assert_eq!(lifecycle(&fixture, host).await?, "retired");
    // Retiring revokes the credential, exactly as the browser retire does.
    assert!(matches!(
        fixture.service.authenticate(host, &credential).await,
        Err(RegistryError::InvalidCredential)
    ));
    let (label, metadata): (String, String) = sqlx::query_as(
        "SELECT actor_label,metadata FROM audit_events WHERE action='fleet.host.retired'",
    )
    .fetch_one(&fixture.pool)
    .await?;
    assert!(label.contains("(token: retire-bot)"), "{label}");
    let metadata: serde_json::Value = serde_json::from_str(&metadata)?;
    assert_eq!(metadata["viaApiToken"]["name"], "retire-bot");
    assert_eq!(metadata["viaApiToken"]["id"], "retire-token");
    fixture.close().await
}

#[tokio::test]
async fn a_machine_in_service_is_refused_with_every_reason_and_nothing_changes() -> Result<()> {
    let fixture = Fixture::new(NOW).await?;
    fixture.install_target().await?;
    let (host, credential) = approved_machine(&fixture, "active", Some(NOW - 1_000)).await?;
    fixture.seed_history(host).await?;
    let audit_before = fixture.audit_event_count().await?;
    let revision_before = revision(&fixture, host).await?;

    assert_eq!(
        refusal(&fixture, host).await?,
        [
            Blocker::LifecycleActive,
            Blocker::RecentlySeen,
            Blocker::LivePlacement,
            Blocker::PendingCommand,
        ]
    );

    // Nothing moved: still active, same revision, credential live, no audit.
    assert_eq!(lifecycle(&fixture, host).await?, "active");
    assert_eq!(revision(&fixture, host).await?, revision_before);
    fixture.service.authenticate(host, &credential).await?;
    assert_eq!(fixture.audit_event_count().await?, audit_before);
    fixture.close().await
}

#[tokio::test]
async fn each_guard_rail_is_named_on_its_own() -> Result<()> {
    let fixture = Fixture::new(NOW).await?;
    fixture.install_target().await?;
    let (host, _credential) = approved_machine(&fixture, "draining", Some(NOW - QUIET)).await?;
    fixture.seed_history(host).await?;

    // Draining is out of service, so only the live work is named.
    assert_eq!(
        refusal(&fixture, host).await?,
        [Blocker::LivePlacement, Blocker::PendingCommand]
    );

    // An unconfirmed start is its own reason; the capacity it still holds
    // keeps it a live placement too.
    sqlx::query("UPDATE workload_placements SET state='uncertain' WHERE id='registry-placement'")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("UPDATE host_commands SET state='succeeded' WHERE id='registry-command'")
        .execute(&fixture.pool)
        .await?;
    assert_eq!(
        refusal(&fixture, host).await?,
        [Blocker::LivePlacement, Blocker::UnconfirmedStart]
    );

    // Capacity still charged counts as a live placement even when the
    // placement itself has failed.
    sqlx::query(
        "UPDATE workload_placements SET local_absence_proven_at=1,delayed_starts_excluded_at=1 WHERE id='registry-placement'",
    )
    .execute(&fixture.pool)
    .await?;
    sqlx::query("UPDATE workload_placements SET state='failed' WHERE id='registry-placement'")
        .execute(&fixture.pool)
        .await?;
    assert_eq!(refusal(&fixture, host).await?, [Blocker::LivePlacement]);

    // A runner placed on the machine that has not finished.
    sqlx::query("UPDATE capacity_allocations SET state='released',released_at=1 WHERE id='registry-allocation'")
        .execute(&fixture.pool)
        .await?;
    sqlx::query(
        "INSERT INTO runners (id,pool_id,name,provider,ci_platform,os,architecture,status,ephemeral,configuration_version,created_at,updated_at)
         VALUES ('registry-workload','registry-pool','r','docker','github','linux','X64','online',1,1,1,1)",
    )
    .execute(&fixture.pool)
    .await?;
    assert_eq!(refusal(&fixture, host).await?, [Blocker::LiveRunner]);

    // A command still within its deadline; once past it, it no longer blocks.
    sqlx::query("UPDATE runners SET status='deleted' WHERE id='registry-workload'")
        .execute(&fixture.pool)
        .await?;
    sqlx::query("UPDATE host_commands SET state='pending' WHERE id='registry-command'")
        .execute(&fixture.pool)
        .await?;
    assert_eq!(refusal(&fixture, host).await?, [Blocker::PendingCommand]);
    let deadline: i64 =
        sqlx::query_scalar("SELECT deadline_at FROM host_commands WHERE id='registry-command'")
            .fetch_one(&fixture.pool)
            .await?;
    fixture.clock.set(deadline);

    retire_as(&fixture, &token(ADMIN), host).await?;
    assert_eq!(lifecycle(&fixture, host).await?, "retired");
    fixture.close().await
}

#[tokio::test]
async fn the_quiet_period_is_the_policy_and_its_boundary_is_exact() -> Result<()> {
    let fixture = Fixture::new(NOW).await?;
    // One millisecond short of the quiet period is still "recently seen";
    // the inventory time counts as well as the heartbeat.
    let (host, _credential) = approved_machine(&fixture, "maintenance", None).await?;
    sqlx::query("UPDATE fleet_hosts SET last_heartbeat_at=?,last_inventory_at=? WHERE id=?")
        .bind(NOW - QUIET - 10_000)
        .bind(NOW - QUIET + 1)
        .bind(host.to_string())
        .execute(&fixture.pool)
        .await?;
    assert_eq!(refusal(&fixture, host).await?, [Blocker::RecentlySeen]);

    // A longer policy refuses what the shorter one allows.
    sqlx::query("UPDATE fleet_hosts SET last_inventory_at=? WHERE id=?")
        .bind(NOW - QUIET)
        .bind(host.to_string())
        .execute(&fixture.pool)
        .await?;
    let expected = revision(&fixture, host).await?;
    assert!(matches!(
        fixture
            .service
            .retire_host_with_policy(
                &token(ADMIN),
                host,
                expected,
                RetirePolicy {
                    token_min_offline_millis: QUIET * 4,
                },
            )
            .await,
        Err(RegistryError::HostNotRetirable(reasons)) if reasons == [Blocker::RecentlySeen]
    ));
    retire_as(&fixture, &token(ADMIN), host).await?;
    assert_eq!(lifecycle(&fixture, host).await?, "retired");
    fixture.close().await
}

#[tokio::test]
async fn a_machine_that_never_reported_in_counts_as_quiet() -> Result<()> {
    let fixture = Fixture::new(NOW).await?;
    let (host, _credential) = approved_machine(&fixture, "paused", None).await?;
    retire_as(&fixture, &token(ADMIN), host).await?;
    assert_eq!(lifecycle(&fixture, host).await?, "retired");
    fixture.close().await
}

#[tokio::test]
async fn a_session_retire_keeps_its_original_rules() -> Result<()> {
    let fixture = Fixture::new(NOW).await?;
    // Active and reporting in right now: a token is refused, the browser is not.
    let (host, _credential) = approved_machine(&fixture, "active", Some(NOW)).await?;
    assert_eq!(
        refusal(&fixture, host).await?,
        [Blocker::LifecycleActive, Blocker::RecentlySeen]
    );
    retire_as(&fixture, &Fixture::principal(), host).await?;
    assert_eq!(lifecycle(&fixture, host).await?, "retired");

    // The original entry point is the session path and is unchanged.
    let (other, _credential) = approved_machine(&fixture, "active", Some(NOW)).await?;
    let expected = revision(&fixture, other).await?;
    fixture
        .service
        .retire_host(&Fixture::principal(), other, expected)
        .await?;
    assert_eq!(lifecycle(&fixture, other).await?, "retired");
    fixture.close().await
}

#[tokio::test]
async fn the_original_entry_point_applies_the_default_policy_to_a_token() -> Result<()> {
    let fixture = Fixture::new(NOW).await?;
    let (host, _credential) = approved_machine(&fixture, "active", Some(NOW)).await?;
    let expected = revision(&fixture, host).await?;
    assert!(matches!(
        fixture
            .service
            .retire_host(&token(ADMIN), host, expected)
            .await,
        Err(RegistryError::HostNotRetirable(_))
    ));
    assert_eq!(lifecycle(&fixture, host).await?, "active");
    assert_eq!(
        RetirePolicy::default().token_min_offline_millis,
        QUIET,
        "the default quiet period is fifteen minutes"
    );
    fixture.close().await
}

#[tokio::test]
async fn retiring_twice_is_a_no_op_and_an_owner_who_lost_admin_is_forbidden() -> Result<()> {
    let fixture = Fixture::new(NOW).await?;
    sqlx::query(
        "INSERT INTO users (id,github_id,login,role,access_token,last_login_at,created_at,updated_at)
         VALUES (?,100,'member','member','',1,1,1)",
    )
    .bind(MEMBER)
    .execute(&fixture.pool)
    .await?;
    let (host, _credential) = approved_machine(&fixture, "paused", Some(NOW - QUIET)).await?;

    // A token whose owner is not an administrator changes nothing.
    let before = fixture.audit_event_count().await?;
    assert!(matches!(
        retire_as(&fixture, &token(MEMBER), host).await,
        Err(RegistryError::Forbidden)
    ));
    assert_eq!(lifecycle(&fixture, host).await?, "paused");
    assert_eq!(fixture.audit_event_count().await?, before);

    retire_as(&fixture, &token(ADMIN), host).await?;
    let after = fixture.audit_event_count().await?;
    // The second retire succeeds without writing anything.
    retire_as(&fixture, &token(ADMIN), host).await?;
    assert_eq!(fixture.audit_event_count().await?, after);
    fixture.close().await
}
