//! #259: a pass finishes each ended placement once and takes it off the
//! finish queue, so the next pass neither reads nor writes the settled
//! history. Every pass used to finish every ended placement ever made.

use super::*;
use gridops_core::connect_database_path;
use gridops_core::fleet_health::testdata::{self, SeededFleet};
use std::fs;

struct Harness {
    app: Reconciler,
    fleet: SeededFleet,
    directory: std::path::PathBuf,
}

impl Harness {
    async fn new() -> Result<Self> {
        let directory = std::env::temp_dir().join(format!(
            "gridops-fleet-finish-test-{}",
            uuid::Uuid::new_v4()
        ));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        let fleet = testdata::seed_fleet(&database).await?;
        let config = Config::from_env()?;
        let app = Reconciler {
            github: GitHubClient::new(config.clone())?,
            config,
            database,
            bitbucket: BitbucketClient::new()?,
            gitlab: Arc::new(gridops_core::gitlab::fake::FakeGitLab::default()),
            vault: Vault::new([0; 32], b"reconciler-test".to_vec()),
            http: reqwest::Client::new(),
            pass: Arc::default(),
            tart: Arc::new(tart_lane::TartLane::new(
                Arc::default(),
                tart_lane::SLOT_WATCH_INTERVAL,
            )),
        };
        Ok(Self {
            app,
            fleet,
            directory,
        })
    }

    /// A placement in `state`, started, whose runner row has `runner`.
    /// Answers the runner id.
    async fn placement(&self, state: &str, runner: &str) -> Result<String> {
        let (operation, placement, workload) = (
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
        );
        let host = &self.fleet.hosts[0];
        let at = now_millis() - 3_600_000;
        let mut tx = self.app.database.begin().await?;
        sqlx::query(
            "INSERT INTO runners (id,pool_id,name,status,created_at,updated_at) VALUES (?,?,?,?,?,?)",
        )
        .bind(&workload)
        .bind(&self.fleet.pool_id)
        .bind(format!("runner-{workload}"))
        .bind(runner)
        .bind(at)
        .bind(at)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO fleet_operations (id,kind,principal_kind,principal_scope,idempotency_key,request_hash,status,created_at,updated_at)
             VALUES (?1,'submit_workload','autoscaler','autoscaler:test',?1,'test','succeeded',?2,?2)",
        )
        .bind(&operation)
        .bind(at)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO workload_intents (id,workload_id,generation,kind,pool_id,profile_id,target_id,expected_profile_revision,status,created_at,updated_at)
             VALUES (?,?,1,'ci_runner',?,?,?,1,'admitted',?,?)",
        )
        .bind(&operation)
        .bind(&workload)
        .bind(&self.fleet.pool_id)
        .bind(&self.fleet.profile_id)
        .bind(&self.fleet.target_id)
        .bind(at)
        .bind(at)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO workload_placements (id,intent_id,workload_id,generation,host_id,backend_id,host_epoch,profile_revision,target_id,config_snapshot_json,cpu_millis,memory_mib,disk_bytes,state,created_at,updated_at)
             VALUES (?,?,?,1,?,?,1,1,?,'{}',2000,4096,0,?,?,?)",
        )
        .bind(&placement)
        .bind(&operation)
        .bind(&workload)
        .bind(&host.id)
        .bind(&host.backend_id)
        .bind(&self.fleet.target_id)
        .bind(state)
        .bind(at)
        .bind(at)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO host_commands (id,operation_id,scope,placement_id,host_id,backend_id,host_epoch,generation,kind,state,protocol_version,request_hash,payload_json,expected_revision,issued_at,deadline_at,idempotency_key)
             VALUES (?1,?2,'placement',?3,?4,?5,1,1,'start_runner','succeeded',1,'test','{}',1,?6,?7,?1)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&operation)
        .bind(&placement)
        .bind(&host.id)
        .bind(&host.backend_id)
        .bind(at)
        .bind(at + 60_000)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(workload)
    }

    async fn queued_states(&self) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT p.state FROM placement_finish_queue q JOIN workload_placements p ON p.id=q.placement_id
              ORDER BY p.state",
        )
        .fetch_all(&self.app.database)
        .await?)
    }

    async fn runner(&self, id: &str) -> Result<(String, i64)> {
        Ok(
            sqlx::query_as("SELECT status,updated_at FROM runners WHERE id=?")
                .bind(id)
                .fetch_one(&self.app.database)
                .await?,
        )
    }

    async fn finish(self) -> Result<()> {
        self.app.database.close().await;
        fs::remove_dir_all(self.directory)?;
        Ok(())
    }
}

#[tokio::test]
async fn a_pass_finishes_each_ended_placement_once() -> Result<()> {
    let h = Harness::new().await?;
    for _ in 0..25 {
        h.placement("failed", "failed").await?;
    }
    // Cleaned, but its runner row is still in service: the pass retires it.
    let in_service = h.placement("cleaned", "online").await?;
    h.placement("uncertain", "deleted").await?;
    assert_eq!(h.queued_states().await?.len(), 27);

    fleet::tick(&h.app).await?;
    // Only the uncertain placement is left: it is cleaned up until it is
    // cleaned.
    assert_eq!(h.queued_states().await?, ["uncertain"]);
    assert_eq!(h.runner(&in_service).await?.0, "failed");

    // The next pass reads the queue, which holds nothing of the settled
    // history, and leaves it as it was.
    fleet::tick(&h.app).await?;
    assert_eq!(h.queued_states().await?, ["uncertain"]);
    h.finish().await
}
