//! Agent polls and heartbeats hold `SQLite`'s single write lock while they
//! cancel and deliver commands, and every other writer waits behind them.
//! Each of those statements must reach only a host's open commands through
//! `host_commands_poll_idx`, however long its command history grows: on
//! 2026-11-21 a host with 59,136 commands held the lock for about 550 ms per
//! poll, the lock was never free, and the API answered 503 (#179).

use uuid::Uuid;

use super::super::commands::{NEXT_DELIVERY, UNDELIVERABLE, UNRECEIVED};
use super::*;
use crate::fleet::{
    ids::{AuthoritySessionId, CommandId, HostEpoch, OperationId},
    protocol::{
        agent::{AgentCommandEnvelope, AgentOperation, CommandScope},
        primitives::{IdempotencyKey, ProtocolVersion, WireTimestamp},
    },
};

const NOW: i64 = 1_800_000_000_000;
/// Finished commands seeded ahead of the open ones.
const HISTORY: i64 = 20_000;

/// The plan step every hot-path command statement must take.
const BOUNDED: &str = "INDEX host_commands_poll_idx (host_id=? AND state=?)";

async fn plan(pool: &SqlitePool, statement: &str) -> Result<Vec<String>> {
    Ok(sqlx::query(sqlx::AssertSqlSafe(format!(
        "EXPLAIN QUERY PLAN {statement}"
    )))
    .fetch_all(pool)
    .await?
    .iter()
    .map(|row| row.get("detail"))
    .collect())
}

#[tokio::test]
async fn hot_path_command_statements_read_only_open_commands_through_the_poll_index() -> Result<()>
{
    let history = History::new().await?;
    let mut unbounded = Vec::new();
    for (name, statement) in [
        ("poll_commands delivery", NEXT_DELIVERY),
        ("cancel_undeliverable", UNDELIVERABLE),
        ("settle_unreceived", UNRECEIVED),
    ] {
        let plan = plan(&history.f.pool, statement).await?;
        // `(host_id=?)` alone, through whichever index, reads every command
        // the host was ever sent; so does a scan.
        if !plan.iter().any(|step| step.contains(BOUNDED))
            || plan.iter().any(|step| step.ends_with("(host_id=?)"))
            || plan.iter().any(|step| step.starts_with("SCAN "))
        {
            unbounded.push(format!("{name}: {plan:?}"));
        }
    }
    assert!(
        unbounded.is_empty(),
        "these must reach host_commands through {BOUNDED} only:\n{}",
        unbounded.join("\n")
    );
    history.f.close().await
}

/// The bounded statements choose exactly what the unbounded ones chose: the
/// same commands cancelled or made uncertain, the same placements failed for
/// the same reasons, the same commands delivered in the same order, and the
/// finished history left alone.
#[tokio::test]
async fn a_long_history_changes_nothing_poll_cancels_or_delivers() -> Result<()> {
    let history = History::new().await?;
    let h = &history;
    let finished_before = h.finished().await?;

    // First poll: cancels what can't be delivered, then hands out the oldest
    // deliverable command, which was queued before the whole history.
    assert_eq!(h.poll().await?, ["deliver-first"]);
    for (id, state) in [
        ("deliver-first", "delivered"),
        ("stale-session-start", "cancelled"),
        ("past-margin-prepare", "cancelled"),
        ("old-epoch-inspect", "cancelled"),
        ("registry-command", "cancelled"),
        ("expired-start", "uncertain"),
        ("expired-inspect", "uncertain"),
        ("accepted-old", "accepted"),
        ("uncertain-old", "uncertain"),
        ("delivered-recent", "delivered"),
        ("redeliver", "delivered"),
        ("deliver-second", "pending"),
    ] {
        assert_eq!(h.command_state(id).await?, state, "command {id}");
    }
    assert_eq!(h.attempts("deliver-first").await?, (1, 1));
    h.assert_failed(
        "stale-session",
        "agent_session_lost",
        "start_runner",
        Some("replaced"),
    )
    .await?;
    h.assert_failed(
        "past-margin",
        "prepare_deadline_expired",
        "prepare_environment",
        None,
    )
    .await?;
    assert_eq!(h.placement_state("expired").await?, ("uncertain".into(), 0));
    assert_eq!(h.allocation("expired").await?, "reserved");
    // An unanswered inspect leaves its placement as it was.
    assert_eq!(
        h.placement_state("registry-placement").await?,
        ("running".into(), 0)
    );

    // Then a delivered command whose answer is overdue, and then the next
    // pending one, by queue position. A command delivered a moment ago is
    // not offered again.
    assert_eq!(h.poll().await?, ["redeliver"]);
    assert_eq!(h.attempts("redeliver").await?, (2, 2));
    assert_eq!(h.poll().await?, ["deliver-second"]);
    assert_eq!(h.poll().await?, Vec::<String>::new());
    assert_eq!(h.attempts("delivered-recent").await?, (1, 1));

    // With the agent's session over, a pending start fails its placement.
    h.add_placement("ended-session", "starting").await?;
    h.pending_start("ended-session", h.session, NOW + 60_000)
        .await?;
    sqlx::query("UPDATE fleet_hosts SET agent_session_id=NULL WHERE id=?")
        .bind(h.host.to_string())
        .execute(&h.f.pool)
        .await?;
    assert_eq!(h.poll().await?, Vec::<String>::new());
    assert_eq!(h.command_state("ended-session-start").await?, "cancelled");
    h.assert_failed(
        "ended-session",
        "agent_session_lost",
        "start_runner",
        Some("ended"),
    )
    .await?;

    assert_eq!(h.finished().await?, finished_before);
    history.f.close().await
}

/// A host with a long finished command history and a handful of open
/// commands, one of each kind the cancel and delivery rules tell apart.
struct History {
    f: Fixture,
    host: HostId,
    session: AuthoritySessionId,
    authenticated: AuthenticatedHost,
}

impl History {
    async fn new() -> Result<Self> {
        let f = Fixture::new(NOW).await?;
        f.install_target().await?;
        let enrolled = f.consume(&f.issue_empty("query-plan").await?).await?;
        let host = enrolled.host_id;
        f.seed_history(host).await?;
        let authenticated = f
            .service
            .authenticate(host, enrolled.credential.expose())
            .await?;
        let session = AuthoritySessionId::try_new(Uuid::new_v4())?;
        sqlx::query("UPDATE fleet_hosts SET agent_session_id=?,authority_expires_at=? WHERE id=?")
            .bind(session.to_string())
            .bind(NOW + 90_000)
            .bind(host.to_string())
            .execute(&f.pool)
            .await?;
        let history = Self {
            f,
            host,
            session,
            authenticated,
        };
        history.seed().await?;
        Ok(history)
    }

    async fn seed(&self) -> Result<()> {
        let other = AuthoritySessionId::try_new(Uuid::new_v4())?;
        // Queued before the history, so it goes out first.
        self.deliverable("deliver-first", "pending", 0, None)
            .await?;
        sqlx::query(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?)
             INSERT INTO host_commands
             (id,operation_id,scope,host_id,host_epoch,kind,state,protocol_version,request_hash,
              payload_json,expected_revision,issued_at,deadline_at,attempt_count,idempotency_key)
             SELECT 'history-'||i,'registry-command-op','host',?,1,'inspect',
                    CASE i%3 WHEN 0 THEN 'succeeded' WHEN 1 THEN 'failed' ELSE 'cancelled' END,
                    1,'history',json_object('authority_session_id',?),1,?-i-60000,?-i,1,'history-'||i
               FROM n",
        )
        .bind(HISTORY)
        .bind(self.host.to_string())
        .bind(other.to_string())
        .bind(NOW)
        .bind(NOW)
        .execute(&self.f.pool)
        .await?;
        self.add_placement("stale-session", "starting").await?;
        self.add_placement("past-margin", "preparing").await?;
        self.add_placement("expired", "starting").await?;
        // A start sealed to a session the agent has since replaced.
        self.pending_start("stale-session", other, NOW + 60_000)
            .await?;
        // A prepare whose deadline falls inside the delivery margin.
        self.raw(
            "past-margin-prepare",
            Some("past-margin"),
            "prepare_environment",
            "pending",
            1,
            self.session,
            NOW + 3_000,
        )
        .await?;
        // Delivered, then never answered by its deadline.
        self.raw(
            "expired-start",
            Some("expired"),
            "start_runner",
            "delivered",
            1,
            self.session,
            NOW - 1,
        )
        .await?;
        self.raw(
            "expired-inspect",
            Some("registry-placement"),
            "inspect",
            "delivered",
            1,
            self.session,
            NOW - 1,
        )
        .await?;
        // Sealed to an epoch the host has left.
        self.raw(
            "old-epoch-inspect",
            None,
            "inspect",
            "pending",
            2,
            self.session,
            NOW + 60_000,
        )
        .await?;
        // Past their deadlines, but no longer open: left alone.
        self.raw(
            "accepted-old",
            Some("registry-placement"),
            "inspect",
            "accepted",
            1,
            other,
            NOW - 1,
        )
        .await?;
        self.raw(
            "uncertain-old",
            Some("registry-placement"),
            "inspect",
            "uncertain",
            1,
            other,
            NOW - 1,
        )
        .await?;
        self.deliverable("delivered-recent", "delivered", 1, Some(NOW - 1_000))
            .await?;
        self.deliverable("redeliver", "delivered", 1, Some(NOW - 120_000))
            .await?;
        self.deliverable("deliver-second", "pending", 0, None)
            .await?;
        Ok(())
    }

    /// A host-scoped inspect sealed to the current session, with a real
    /// envelope because delivery hands it out.
    async fn deliverable(
        &self,
        id: &str,
        state: &str,
        attempts: i64,
        delivered_at: Option<i64>,
    ) -> Result<()> {
        let command_id = CommandId::try_new(Uuid::new_v4())?;
        let idempotency_key = IdempotencyKey::parse(id)?;
        let envelope = AgentCommandEnvelope {
            protocol_version: ProtocolVersion::new(1)?,
            command_id,
            operation_id: OperationId::try_new(Uuid::new_v4())?,
            idempotency_key,
            host_id: self.host,
            host_epoch: HostEpoch::new(1)?,
            control_plane_incarnation: crate::fleet::ids::ControlPlaneIncarnation::try_new(
                Uuid::new_v4(),
            )?,
            authority_session_id: self.session,
            scope: CommandScope::Host,
            expected_revision: Revision::new(1)?,
            issued_at: WireTimestamp::from_millis(NOW - 1_000)?,
            deadline_at: WireTimestamp::from_millis(NOW + 60_000)?,
            request_hash: String::new(),
            operation: AgentOperation::InspectHost,
        }
        .seal_hash()?;
        sqlx::query(
            "INSERT INTO host_commands
             (id,operation_id,scope,host_id,host_epoch,kind,state,protocol_version,request_hash,
              payload_json,expected_revision,issued_at,deadline_at,attempt_count,idempotency_key)
             VALUES (?,'registry-command-op','host',?,1,'inspect',?,1,?,?,1,?,?,?,?)",
        )
        .bind(id)
        .bind(self.host.to_string())
        .bind(state)
        .bind(&envelope.request_hash)
        .bind(serde_json::to_string(&envelope)?)
        .bind(NOW - 1_000)
        .bind(NOW + 60_000)
        .bind(attempts)
        .bind(id)
        .execute(&self.f.pool)
        .await?;
        if let Some(recorded_at) = delivered_at {
            sqlx::query(
                "INSERT INTO host_command_attempts (command_id,attempt,state,recorded_at)
                 VALUES (?,1,'delivered',?)",
            )
            .bind(id)
            .bind(recorded_at)
            .execute(&self.f.pool)
            .await?;
        }
        Ok(())
    }

    async fn pending_start(
        &self,
        placement: &str,
        session: AuthoritySessionId,
        deadline: i64,
    ) -> Result<()> {
        self.raw(
            &format!("{placement}-start"),
            Some(placement),
            "start_runner",
            "pending",
            1,
            session,
            deadline,
        )
        .await
    }

    /// A command that is never delivered here, so its payload only carries
    /// the session it was sealed to.
    #[allow(clippy::too_many_arguments)]
    async fn raw(
        &self,
        id: &str,
        placement: Option<&str>,
        kind: &str,
        state: &str,
        epoch: i64,
        session: AuthoritySessionId,
        deadline: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO host_commands
             (id,operation_id,scope,placement_id,host_id,backend_id,host_epoch,generation,kind,state,
              protocol_version,request_hash,payload_json,expected_revision,issued_at,deadline_at,
              attempt_count,idempotency_key)
             VALUES (?,'registry-command-op',?,?,?,?,?,?,?,?,1,'raw',json_object('authority_session_id',?),
                     1,?,?,?,?)",
        )
        .bind(id)
        .bind(if placement.is_some() {
            "placement"
        } else {
            "host"
        })
        .bind(placement)
        .bind(self.host.to_string())
        .bind(placement.map(|_| "registry-backend"))
        .bind(epoch)
        .bind(placement.map(|_| 1_i64))
        .bind(kind)
        .bind(state)
        .bind(session.to_string())
        .bind(NOW - 30_000)
        .bind(deadline)
        .bind(i64::from(state != "pending"))
        .bind(id)
        .execute(&self.f.pool)
        .await?;
        Ok(())
    }

    /// A placement of the seeded pool on this host, charged against its
    /// root domain.
    async fn add_placement(&self, id: &str, state: &str) -> Result<()> {
        let host = self.host.to_string();
        let operation = format!("{id}-op");
        let workload = format!("{id}-workload");
        sqlx::query(
            "INSERT INTO fleet_operations
             (id,kind,host_id,requested_by_user_id,principal_kind,principal_scope,idempotency_key,
              request_hash,status,created_at,updated_at)
             VALUES (?,'submit_workload',?,'registry-admin','user','user:registry-admin',?,?,'succeeded',?,?)",
        )
        .bind(&operation)
        .bind(&host)
        .bind(&operation)
        .bind(&operation)
        .bind(NOW)
        .bind(NOW)
        .execute(&self.f.pool)
        .await?;
        sqlx::query(
            "INSERT INTO workload_intents
             (id,workload_id,generation,kind,pool_id,profile_id,target_id,expected_profile_revision,
              status,created_at,updated_at)
             VALUES (?,?,1,'ci_runner','registry-pool','registry-profile',?,1,'admitted',?,?)",
        )
        .bind(&operation)
        .bind(&workload)
        .bind(TARGET)
        .bind(NOW)
        .bind(NOW)
        .execute(&self.f.pool)
        .await?;
        sqlx::query(
            "INSERT INTO workload_placements
             (id,intent_id,workload_id,generation,host_id,backend_id,host_epoch,profile_revision,
              target_id,config_snapshot_json,cpu_millis,memory_mib,disk_bytes,state,created_at,updated_at)
             VALUES (?,?,?,1,?,'registry-backend',1,1,?,'{}',100,128,0,?,?,?)",
        )
        .bind(id)
        .bind(&operation)
        .bind(&workload)
        .bind(&host)
        .bind(TARGET)
        .bind(state)
        .bind(NOW)
        .bind(NOW)
        .execute(&self.f.pool)
        .await?;
        sqlx::query(
            "INSERT INTO capacity_allocations
             (id,placement_id,host_id,backend_id,host_epoch,domain_id,policy_revision,
              cpu_millis,memory_mib,disk_bytes,created_at,updated_at)
             SELECT ?,?,?,'registry-backend',1,id,1,100,128,0,?,?
               FROM host_resource_domains WHERE host_id=? AND parent_domain_id IS NULL",
        )
        .bind(format!("{id}-allocation"))
        .bind(id)
        .bind(&host)
        .bind(NOW)
        .bind(NOW)
        .bind(&host)
        .execute(&self.f.pool)
        .await?;
        Ok(())
    }

    /// Polls as the agent does and names the commands handed out by the id
    /// they were seeded with.
    async fn poll(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for envelope in self
            .f
            .service
            .poll_commands(&self.authenticated, self.session)
            .await?
        {
            names.push(
                sqlx::query_scalar("SELECT id FROM host_commands WHERE json_extract(payload_json,'$.command_id')=?")
                    .bind(envelope.command_id.to_string())
                    .fetch_one(&self.f.pool)
                    .await?,
            );
        }
        Ok(names)
    }

    async fn command_state(&self, id: &str) -> Result<String> {
        Ok(
            sqlx::query_scalar("SELECT state FROM host_commands WHERE id=?")
                .bind(id)
                .fetch_one(&self.f.pool)
                .await?,
        )
    }

    /// The command's attempt count and how many deliveries were recorded.
    async fn attempts(&self, id: &str) -> Result<(i64, i64)> {
        Ok(sqlx::query_as(
            "SELECT attempt_count,(SELECT COUNT(*) FROM host_command_attempts WHERE command_id=c.id)
               FROM host_commands c WHERE id=?",
        )
        .bind(id)
        .fetch_one(&self.f.pool)
        .await?)
    }

    /// The placement's state and how many times it failed.
    async fn placement_state(&self, id: &str) -> Result<(String, i64)> {
        Ok(sqlx::query_as(
            "SELECT state,(SELECT COUNT(*) FROM fleet_events
                            WHERE placement_id=p.id AND kind='placement_failed')
               FROM workload_placements p WHERE id=?",
        )
        .bind(id)
        .fetch_one(&self.f.pool)
        .await?)
    }

    async fn allocation(&self, placement: &str) -> Result<String> {
        Ok(
            sqlx::query_scalar("SELECT state FROM capacity_allocations WHERE placement_id=?")
                .bind(placement)
                .fetch_one(&self.f.pool)
                .await?,
        )
    }

    /// The placement failed once, for `reason`, naming the command, and its
    /// capacity was released.
    async fn assert_failed(
        &self,
        placement: &str,
        reason: &str,
        kind: &str,
        session: Option<&str>,
    ) -> Result<()> {
        assert_eq!(
            self.placement_state(placement).await?,
            ("failed".into(), 1),
            "placement {placement}"
        );
        let detail: String = sqlx::query_scalar(
            "SELECT detail_json FROM fleet_events WHERE placement_id=? AND kind='placement_failed'",
        )
        .bind(placement)
        .fetch_one(&self.f.pool)
        .await?;
        let detail: serde_json::Value = serde_json::from_str(&detail)?;
        assert_eq!(detail["reason"], reason, "placement {placement}: {detail}");
        assert_eq!(
            detail["command_kind"], kind,
            "placement {placement}: {detail}"
        );
        assert_eq!(
            detail["session"].as_str(),
            session,
            "placement {placement}: {detail}"
        );
        assert_eq!(self.allocation(placement).await?, "released");
        Ok(())
    }

    /// How many finished history commands are in each state.
    async fn finished(&self) -> Result<Vec<(String, i64)>> {
        Ok(sqlx::query_as(
            "SELECT state,COUNT(*) FROM host_commands WHERE id LIKE 'history-%'
              GROUP BY state ORDER BY state",
        )
        .fetch_all(&self.f.pool)
        .await?)
    }
}
