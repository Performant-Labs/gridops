//! The seeded fleet the fleet health tests share (#197 section 7, behind
//! `test-support`): the five fixture machines with their root budgets, one
//! repository-scoped pool that runs the default job, and writers for jobs,
//! samples, start attempts and start failures that build the rows the
//! rollups and the live view read (the placement graph `fleet_ready.sql`
//! documents). [`build_week`] is the synthetic week at realistic volume.
//!
//! Writes go in short batches (one transaction each, well under
//! `speed_check`'s 50 ms), so the week itself passes the check it is used
//! with.

use std::{collections::BTreeMap, sync::OnceLock};

use serde_json::json;
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use super::{
    JobCategory,
    category::{self, Classifier},
};
use crate::fleet::PlacementFailureReason;

const GIB: i64 = 1024 * 1024 * 1024;
const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

/// The five fixture machines and their root CPU budgets (millis).
pub const FLEET_HOSTS: [(&str, u32); 5] = [
    ("io", 22_800),
    ("Jupiter", 19_200),
    ("callisto", 8_000),
    ("europa", 12_800),
    ("MacBookPro", 0),
];

/// Their memory budgets (MiB), in [`FLEET_HOSTS`]' order.
const MEMORY_MIB: [u32; 5] = [65_536, 49_152, 16_384, 32_768, 16_384];

/// Every seeded root disk's size.
pub const DISK_BYTES: i64 = 100 * GIB;

/// Held by tests that classify through the process-wide classifier cache, so
/// no other test's rules move it under them.
pub static CLASSIFIER_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const INSTALLATION_ID: i64 = 4_242;
const REPOSITORY_ID: i64 = 4_343;
const REPOSITORY: &str = "acme/app";
const GITLAB_CONNECTION: &str = "testdata-gitlab";
const GITLAB_PROJECT_ID: i64 = 77;
const INCARNATION: &str = "7d000000-0000-4000-8000-000000000001";

/// The batch sizes: each batch is one short transaction.
const JOB_BATCH: usize = 75;
const SAMPLE_BATCH: usize = 500;

/// A deterministic UUID-shaped id: `kind` names the row kind, `n` the row.
fn uuid(kind: u32, n: i64) -> String {
    format!("{kind:08x}-0000-4000-8000-{:012x}", n.rem_euclid(1 << 48))
}

/// One seeded machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeededHost {
    pub id: String,
    pub name: String,
    pub cpu_budget_millis: u32,
    pub memory_budget_mib: u32,
    pub disk_bytes: i64,
    pub root_domain_id: String,
    pub backend_id: String,
}

/// The seeded fleet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeededFleet {
    /// In [`FLEET_HOSTS`]' order.
    pub hosts: Vec<SeededHost>,
    /// The one pool, repository-scoped, labelled `["self-hosted","gridops"]`.
    pub pool_id: String,
    /// `acme/app`.
    pub repository: String,
    pub installation_id: i64,
    pub repository_id: i64,
    pub target_id: String,
    pub profile_id: String,
}

impl SeededFleet {
    /// The seeded machine named `name`.
    ///
    /// # Errors
    /// No seeded machine has that name.
    pub fn host(&self, name: &str) -> anyhow::Result<&SeededHost> {
        self.hosts
            .iter()
            .find(|host| host.name == name)
            .ok_or_else(|| anyhow::anyhow!("no seeded machine named {name}"))
    }

    fn host_by_id(&self, id: &str) -> anyhow::Result<&SeededHost> {
        self.hosts
            .iter()
            .find(|host| host.id == id)
            .ok_or_else(|| anyhow::anyhow!("no seeded machine with id {id}"))
    }
}

/// Seeds the control plane, one installation and repository (`acme/app`),
/// its pool with a fleet profile, and the five machines (approved and
/// active, each with a 100 GiB root domain and a ready Docker backend).
///
/// # Errors
/// A write fails (for example the fleet is already seeded).
pub async fn seed_fleet(db: &SqlitePool) -> anyhow::Result<SeededFleet> {
    let pool_id = uuid(0x0f00_0004, 1);
    let target_id = uuid(0x0f00_0005, 1);
    let profile_id = uuid(0x0f00_0006, 1);
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT OR IGNORE INTO fleet_control_plane
           (singleton,incarnation,readiness,schema_version,protocol_min,protocol_max,updated_at)
         VALUES (1,?,'active',1,1,1,1)",
    )
    .bind(INCARNATION)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
         VALUES (?,?,'acme','Organization','Organization','all',1,1)",
    )
    .bind(INSTALLATION_ID)
    .bind(INSTALLATION_ID)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
         VALUES (?,?,'acme','app',?,0,'main','https://github.test/acme/app',1,1,1)",
    )
    .bind(REPOSITORY_ID)
    .bind(INSTALLATION_ID)
    .bind(REPOSITORY)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,labels,image,created_at,updated_at)
         VALUES (?,?,?,'linux-docker','repository','[\"self-hosted\",\"gridops\"]','runner:testdata',1,1)",
    )
    .bind(&pool_id)
    .bind(INSTALLATION_ID)
    .bind(REPOSITORY_ID)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT OR IGNORE INTO runner_pool_installations (pool_id,installation_id) VALUES (?,?)",
    )
    .bind(&pool_id)
    .bind(INSTALLATION_ID)
    .execute(&mut *tx)
    .await?;
    let mut hosts = Vec::new();
    for (index, ((name, cpu), memory)) in (1_i64..).zip(FLEET_HOSTS.into_iter().zip(MEMORY_MIB)) {
        let host = SeededHost {
            id: uuid(0x0f00_0001, index),
            name: name.to_owned(),
            cpu_budget_millis: cpu,
            memory_budget_mib: memory,
            disk_bytes: DISK_BYTES,
            root_domain_id: uuid(0x0f00_0002, index),
            backend_id: uuid(0x0f00_0003, index),
        };
        sqlx::query(
            "INSERT INTO fleet_hosts (id,name,host_os,architecture,enrollment_state,lifecycle_state,integrity_state,created_at,updated_at)
             VALUES (?,?,'linux','x64','approved','active','verified',1,1)",
        )
        .bind(&host.id)
        .bind(&host.name)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO host_resource_domains (id,host_id,parent_domain_id,name,cpu_millis,memory_mib,disk_bytes)
             VALUES (?,?,NULL,'Physical',?,?,?)",
        )
        .bind(&host.root_domain_id)
        .bind(&host.id)
        .bind(i64::from(host.cpu_budget_millis))
        .bind(i64::from(host.memory_budget_mib))
        .bind(host.disk_bytes)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO host_backends (id,host_id,domain_id,runtime_kind,execution_os,architecture,readiness,enabled,created_at,updated_at)
             VALUES (?,?,?,'docker','linux','x64','ready',1,1,1)",
        )
        .bind(&host.backend_id)
        .bind(&host.id)
        .bind(&host.root_domain_id)
        .execute(&mut *tx)
        .await?;
        hosts.push(host);
    }
    sqlx::query(
        "INSERT INTO fleet_ci_targets (id,kind,canonical_key,installation_id,repository_id)
         VALUES (?,'github_repository',?,?,?)",
    )
    .bind(&target_id)
    .bind(format!("github:{INSTALLATION_ID}:{REPOSITORY_ID}"))
    .bind(INSTALLATION_ID)
    .bind(REPOSITORY_ID)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO pool_execution_profiles (id,pool_id,name,enabled,runtime_kind,execution_os,architecture,mode,image,cpu_millis,memory_mib,disk_bytes,created_at,updated_at)
         VALUES (?,?,'Docker',1,'docker','linux','x64','ephemeral','runner:testdata',2000,4096,0,1,1)",
    )
    .bind(&profile_id)
    .bind(&pool_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO profile_ci_targets (profile_id,target_id) VALUES (?,?)")
        .bind(&profile_id)
        .bind(&target_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(SeededFleet {
        hosts,
        pool_id,
        repository: REPOSITORY.to_owned(),
        installation_id: INSTALLATION_ID,
        repository_id: REPOSITORY_ID,
        target_id,
        profile_id,
    })
}

/// How a job's placement ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementEnd {
    Stopped,
    /// An infrastructure failure.
    Failed,
    Uncertain,
    /// Still running (a running job's).
    Running,
}

impl PlacementEnd {
    const fn state(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Failed => "failed",
            Self::Uncertain => "uncertain",
            Self::Running => "running",
        }
    }
}

fn default_classifier() -> &'static Classifier {
    static DEFAULT: OnceLock<Classifier> = OnceLock::new();
    DEFAULT.get_or_init(|| category::compile(&category::default_rules()))
}

/// One job to seed. A job on a `host` has a runner and a placement there
/// (its runner's `github_runner_id`, or `gitlab_runner_id`, is the job's
/// `runner_id`); `host: None` on a started job is a runner `GridOps` doesn't
/// manage (a `runner_id` with no `runners` row). A `gitlab` job is written
/// to `gitlab_jobs` instead of `workflow_jobs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSeed {
    pub id: i64,
    /// Defaults to `id`.
    pub run_id: i64,
    pub run_attempt: i64,
    pub workflow: String,
    pub name: String,
    pub labels: Vec<String>,
    pub host: Option<String>,
    pub cpu_millis: u32,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    /// `queued`, `in_progress` or `completed`.
    pub status: String,
    pub conclusion: Option<String>,
    /// Defaults to what the default rules give the name.
    pub category: Option<JobCategory>,
    pub placement: PlacementEnd,
    pub gitlab: bool,
}

impl JobSeed {
    fn new(id: i64, name: &str, created_at: i64) -> Self {
        Self {
            id,
            run_id: id,
            run_attempt: 1,
            workflow: "CI".to_owned(),
            name: name.to_owned(),
            labels: vec!["self-hosted".to_owned(), "gridops".to_owned()],
            host: None,
            cpu_millis: 2_000,
            created_at,
            started_at: None,
            completed_at: None,
            status: "queued".to_owned(),
            conclusion: None,
            category: Some(default_classifier().classify(name, Some("CI"), Some(REPOSITORY), &[])),
            placement: PlacementEnd::Stopped,
            gitlab: false,
        }
    }

    /// A successful job that ran on `host` (or an unmanaged runner).
    pub fn completed(
        id: i64,
        name: &str,
        host: Option<impl Into<String>>,
        created_at: i64,
        started_at: i64,
        completed_at: i64,
    ) -> Self {
        Self {
            host: host.map(Into::into),
            started_at: Some(started_at),
            completed_at: Some(completed_at),
            status: "completed".to_owned(),
            conclusion: Some("success".to_owned()),
            ..Self::new(id, name, created_at)
        }
    }

    /// A job waiting to start.
    pub fn queued(id: i64, name: &str, created_at: i64) -> Self {
        Self::new(id, name, created_at)
    }

    /// A job running now on `host`: its runner is busy with it.
    pub fn running(
        id: i64,
        name: &str,
        host: Option<impl Into<String>>,
        created_at: i64,
        started_at: i64,
    ) -> Self {
        Self {
            host: host.map(Into::into),
            started_at: Some(started_at),
            status: "in_progress".to_owned(),
            placement: PlacementEnd::Running,
            ..Self::new(id, name, created_at)
        }
    }

    fn started(&self) -> bool {
        self.status != "queued"
    }

    fn running_now(&self) -> bool {
        self.status == "in_progress"
    }

    /// The row id space: GitLab rows apart from GitHub ones.
    fn row_number(&self) -> i64 {
        if self.gitlab {
            self.id + (1 << 40)
        } else {
            self.id
        }
    }

    fn external_runner_id(&self) -> Option<i64> {
        if !self.started() {
            return None;
        }
        let base = match (self.gitlab, self.host.is_some()) {
            (false, true) => 1_000_000_000,
            (true, true) => 3_000_000_000,
            (_, false) => 2_000_000_000,
        };
        Some(base + self.id)
    }

    fn gitlab_status(&self) -> &'static str {
        match self.status.as_str() {
            "queued" => "pending",
            "in_progress" => "running",
            _ if self.conclusion.as_deref() == Some("success") => "success",
            _ => "failed",
        }
    }

    fn row<'a>(&'a self, fleet: &'a SeededFleet) -> anyhow::Result<JobRow<'a>> {
        let managed = self.started() && self.host.is_some();
        let host = match (&self.host, managed) {
            (Some(id), true) => Some(fleet.host_by_id(id)?),
            _ => None,
        };
        let n = self.row_number();
        let external_runner_id = self.external_runner_id();
        Ok(JobRow {
            job: self,
            labels: serde_json::to_string(&self.labels)?,
            updated_at: self
                .completed_at
                .or(self.started_at)
                .unwrap_or(self.created_at),
            host,
            external_runner_id,
            runner_name: external_runner_id.map(|id| format!("testdata-runner-{id}")),
            runner_row: uuid(0x0a00_0004, n),
            operation: uuid(0x0a00_0001, n),
            workload: uuid(0x0a00_0003, n),
            placement: uuid(0x0a00_0002, n),
        })
    }
}

/// One job's rows, as [`insert_jobs`] writes them.
struct JobRow<'a> {
    job: &'a JobSeed,
    labels: String,
    updated_at: i64,
    /// The machine of a managed job (started, with a `host`).
    host: Option<&'a SeededHost>,
    external_runner_id: Option<i64>,
    runner_name: Option<String>,
    runner_row: String,
    operation: String,
    workload: String,
    placement: String,
}

impl JobRow<'_> {
    fn busy(&self) -> bool {
        self.host.is_some() && self.job.running_now()
    }
}

/// A run's row: its highest seeded attempt, `completed` once every seeded
/// job of it is.
struct RunRow<'a> {
    id: i64,
    workflow: &'a str,
    attempt: i64,
    completed: bool,
    created_at: i64,
    updated_at: i64,
}

async fn insert_batch(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    fleet: &SeededFleet,
    rows: &[JobRow<'_>],
) -> anyhow::Result<()> {
    let mut runs: BTreeMap<i64, RunRow<'_>> = BTreeMap::new();
    for row in rows.iter().filter(|row| !row.job.gitlab) {
        let job = row.job;
        let run = runs.entry(job.run_id).or_insert(RunRow {
            id: job.run_id,
            workflow: &job.workflow,
            attempt: job.run_attempt,
            completed: true,
            created_at: job.created_at,
            updated_at: row.updated_at,
        });
        run.attempt = run.attempt.max(job.run_attempt);
        run.updated_at = run.updated_at.max(row.updated_at);
        run.completed &= job.status == "completed";
    }
    if !runs.is_empty() {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,run_attempt,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at) ",
        );
        query.push_values(runs.values(), |mut values, run| {
            values
                .push_bind(run.id)
                .push_bind(fleet.repository_id)
                .push_bind(run.workflow.to_owned())
                .push_bind(run.id)
                .push_bind(run.attempt)
                .push_bind("push")
                .push_bind(if run.completed {
                    "completed"
                } else {
                    "in_progress"
                })
                .push_bind("testdata")
                .push_bind(format!(
                    "https://github.test/acme/app/actions/runs/{}",
                    run.id
                ))
                .push_bind(run.created_at)
                .push_bind(run.updated_at)
                .push_bind(run.created_at)
                .push_bind(run.updated_at);
        });
        query.push(
            " ON CONFLICT(id) DO UPDATE SET
               run_attempt=MAX(workflow_runs.run_attempt,excluded.run_attempt),
               status=CASE WHEN workflow_runs.status='completed' AND excluded.status='completed' THEN 'completed' ELSE 'in_progress' END,
               updated_at=MAX(workflow_runs.updated_at,excluded.updated_at)",
        );
        query.build().execute(&mut **tx).await?;
    }
    let managed: Vec<(&JobRow<'_>, &SeededHost)> = rows
        .iter()
        .filter_map(|row| row.host.map(|host| (row, host)))
        .collect();
    if !managed.is_empty() {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO runners (id,pool_id,github_runner_id,name,status,busy,ephemeral,current_job_id,registered_at,deleted_at,created_at,updated_at,ci_platform,gitlab_connection_id,gitlab_runner_id) ",
        );
        query.push_values(&managed, |mut values, (row, _)| {
            let job = row.job;
            values
                .push_bind(row.runner_row.clone())
                .push_bind(fleet.pool_id.clone())
                .push_bind(if job.gitlab {
                    None
                } else {
                    row.external_runner_id
                })
                .push_bind(row.runner_name.clone())
                .push_bind(if row.busy() { "busy" } else { "offline" })
                .push_bind(row.busy())
                .push_bind(true)
                .push_bind(row.busy().then_some(job.id))
                .push_bind(job.created_at)
                .push_bind(if job.running_now() {
                    None
                } else {
                    job.completed_at
                })
                .push_bind(job.created_at)
                .push_bind(row.updated_at)
                .push_bind(if job.gitlab { "gitlab" } else { "github" })
                .push_bind(job.gitlab.then_some(GITLAB_CONNECTION))
                .push_bind(if job.gitlab {
                    row.external_runner_id
                } else {
                    None
                });
        });
        query.build().execute(&mut **tx).await?;

        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO fleet_operations (id,kind,principal_kind,principal_scope,idempotency_key,request_hash,status,created_at,updated_at) ",
        );
        query.push_values(&managed, |mut values, (row, _)| {
            values
                .push_bind(row.operation.clone())
                .push_bind("submit_workload")
                .push_bind("autoscaler")
                .push_bind("autoscaler:testdata")
                .push_bind(row.operation.clone())
                .push_bind("testdata")
                .push_bind("succeeded")
                .push_bind(row.job.created_at)
                .push_bind(row.updated_at);
        });
        query.build().execute(&mut **tx).await?;

        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO workload_intents (id,workload_id,generation,kind,pool_id,profile_id,target_id,expected_profile_revision,status,created_at,updated_at) ",
        );
        query.push_values(&managed, |mut values, (row, _)| {
            values
                .push_bind(row.operation.clone())
                .push_bind(row.workload.clone())
                .push_bind(1_i64)
                .push_bind("ci_runner")
                .push_bind(fleet.pool_id.clone())
                .push_bind(fleet.profile_id.clone())
                .push_bind(fleet.target_id.clone())
                .push_bind(1_i64)
                .push_bind("admitted")
                .push_bind(row.job.created_at)
                .push_bind(row.updated_at);
        });
        query.build().execute(&mut **tx).await?;

        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO workload_placements (id,intent_id,workload_id,generation,host_id,backend_id,host_epoch,profile_revision,target_id,config_snapshot_json,cpu_millis,memory_mib,disk_bytes,state,runner_id,created_at,updated_at) ",
        );
        query.push_values(&managed, |mut values, (row, host)| {
            values
                .push_bind(row.placement.clone())
                .push_bind(row.operation.clone())
                .push_bind(row.workload.clone())
                .push_bind(1_i64)
                .push_bind(host.id.clone())
                .push_bind(host.backend_id.clone())
                .push_bind(1_i64)
                .push_bind(1_i64)
                .push_bind(fleet.target_id.clone())
                .push_bind("{}")
                .push_bind(i64::from(row.job.cpu_millis))
                .push_bind(4_096_i64)
                .push_bind(0_i64)
                .push_bind(row.job.placement.state())
                .push_bind(row.runner_row.clone())
                .push_bind(row.job.created_at)
                .push_bind(row.updated_at);
        });
        query.build().execute(&mut **tx).await?;
    }
    let github: Vec<&JobRow<'_>> = rows.iter().filter(|row| !row.job.gitlab).collect();
    if !github.is_empty() {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO workflow_jobs (id,run_id,name,status,conclusion,runner_id,runner_name,labels,html_url,started_at,completed_at,created_at,updated_at,category) ",
        );
        query.push_values(&github, |mut values, row| {
            let job = row.job;
            values
                .push_bind(job.id)
                .push_bind(job.run_id)
                .push_bind(job.name.clone())
                .push_bind(job.status.clone())
                .push_bind(job.conclusion.clone())
                .push_bind(row.external_runner_id)
                .push_bind(row.runner_name.clone())
                .push_bind(row.labels.clone())
                .push_bind(format!(
                    "https://github.test/acme/app/actions/runs/{}/job/{}",
                    job.run_id, job.id
                ))
                .push_bind(job.started_at)
                .push_bind(job.completed_at)
                .push_bind(job.created_at)
                .push_bind(row.updated_at)
                .push_bind(job.category.map(JobCategory::as_str));
        });
        query.build().execute(&mut **tx).await?;

        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO workflow_job_sync (job_id,run_attempt,confirmed_at,checked_at) ",
        );
        query.push_values(&github, |mut values, row| {
            values
                .push_bind(row.job.id)
                .push_bind(row.job.run_attempt)
                .push_bind(row.updated_at)
                .push_bind(row.updated_at);
        });
        query.build().execute(&mut **tx).await?;
    }
    let gitlab: Vec<&JobRow<'_>> = rows.iter().filter(|row| row.job.gitlab).collect();
    if !gitlab.is_empty() {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,gitlab_runner_id,pool_id,created_at,updated_at,category,job_name,project_path,started_at,completed_at) ",
        );
        query.push_values(&gitlab, |mut values, row| {
            let job = row.job;
            values
                .push_bind(format!("testdata-{}", job.id))
                .push_bind(GITLAB_CONNECTION)
                .push_bind(GITLAB_PROJECT_ID)
                .push_bind(job.id)
                .push_bind(job.run_id)
                .push_bind(job.gitlab_status())
                .push_bind(row.labels.clone())
                .push_bind(row.updated_at)
                .push_bind(row.external_runner_id)
                .push_bind(fleet.pool_id.clone())
                .push_bind(job.created_at)
                .push_bind(row.updated_at)
                .push_bind(job.category.map(JobCategory::as_str))
                .push_bind(job.name.clone())
                .push_bind(REPOSITORY)
                .push_bind(job.started_at)
                .push_bind(job.completed_at);
        });
        query.build().execute(&mut **tx).await?;
    }
    Ok(())
}

/// Writes `jobs` with everything the rollups and the live view join them
/// to, in batches of short transactions. A run's `run_attempt` is its
/// highest seeded attempt; each job's `workflow_job_sync.run_attempt` is its
/// own.
///
/// # Errors
/// A job names a machine that is not seeded, or a write fails.
pub async fn insert_jobs(
    db: &SqlitePool,
    fleet: &SeededFleet,
    jobs: &[JobSeed],
) -> anyhow::Result<()> {
    if jobs.iter().any(|job| job.gitlab) {
        sqlx::query(
            "INSERT OR IGNORE INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
             VALUES (?,'testdata','https://gitlab.test','project',?,?,'testdata.token','testdata.webhook',1,1)",
        )
        .bind(GITLAB_CONNECTION)
        .bind(GITLAB_PROJECT_ID)
        .bind(REPOSITORY)
        .execute(db)
        .await?;
    }
    for batch in jobs.chunks(JOB_BATCH) {
        let rows = batch
            .iter()
            .map(|job| job.row(fleet))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mut tx = db.begin().await?;
        insert_batch(&mut tx, fleet, &rows).await?;
        tx.commit().await?;
    }
    Ok(())
}

/// One root-domain sample. [`SampleSeed::at`] reports 1,000 CPU millis,
/// 1,024 MiB used and half of [`DISK_BYTES`] free, received when observed,
/// with no session provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleSeed {
    pub observed_at: i64,
    pub received_at: i64,
    pub cpu_used_millis: u32,
    pub memory_used_mib: u32,
    pub disk_free_bytes: Option<i64>,
}

impl SampleSeed {
    #[must_use]
    pub fn at(observed_at: i64) -> Self {
        Self {
            observed_at,
            received_at: observed_at,
            cpu_used_millis: 1_000,
            memory_used_mib: 1_024,
            disk_free_bytes: Some(DISK_BYTES / 2),
        }
    }
}

/// Writes `samples` for `host`'s root domain, in batches.
///
/// # Errors
/// A write fails.
pub async fn insert_samples(
    db: &SqlitePool,
    host: &SeededHost,
    samples: &[SampleSeed],
) -> anyhow::Result<()> {
    for batch in samples.chunks(SAMPLE_BATCH) {
        let wire = serde_json::to_string(
            &batch
                .iter()
                .map(|sample| {
                    json!([
                        sample.observed_at,
                        sample.received_at,
                        sample.cpu_used_millis,
                        sample.memory_used_mib,
                        sample.disk_free_bytes
                    ])
                })
                .collect::<Vec<_>>(),
        )?;
        sqlx::query(
            "INSERT INTO host_samples (host_id,domain_id,host_epoch,observed_at,received_at,cpu_used_millis,memory_used_mib,disk_free_bytes,coverage)
             SELECT ?1,?2,1,s.value->>0,s.value->>1,s.value->>2,s.value->>3,s.value->>4,'complete' FROM json_each(?3) s",
        )
        .bind(&host.id)
        .bind(&host.root_domain_id)
        .bind(&wire)
        .execute(db)
        .await?;
    }
    Ok(())
}

/// Writes one `placement_failed` event on `host` at `at`, as the reconciler
/// records one: `{"reason": <code>, "agent_code": <code or null>}`.
///
/// # Errors
/// The write fails.
pub async fn insert_placement_failure(
    db: &SqlitePool,
    host: &SeededHost,
    at: i64,
    reason: PlacementFailureReason,
    agent_code: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO fleet_events (id,host_id,kind,detail_json,created_at) VALUES (?,?,'placement_failed',?,?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&host.id)
    .bind(json!({"reason": reason.as_str(), "agent_code": agent_code}).to_string())
    .bind(at)
    .execute(db)
    .await?;
    Ok(())
}

/// One runner start on `host`: a placement that reaches `starting` at `at`
/// (its `start_runner` command is issued then) and ends `end` at `at + 1 s`.
/// It has no runner and writes no events.
///
/// # Errors
/// The write fails.
pub async fn insert_start_attempt(
    db: &SqlitePool,
    fleet: &SeededFleet,
    host: &SeededHost,
    at: i64,
    end: PlacementEnd,
) -> anyhow::Result<()> {
    let operation = uuid::Uuid::new_v4().to_string();
    let placement = uuid::Uuid::new_v4().to_string();
    let workload = uuid::Uuid::new_v4().to_string();
    let ended = at + 1_000;
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO fleet_operations (id,kind,principal_kind,principal_scope,idempotency_key,request_hash,status,created_at,updated_at)
         VALUES (?1,'submit_workload','autoscaler','autoscaler:testdata',?1,'testdata','succeeded',?2,?3)",
    )
    .bind(&operation)
    .bind(at)
    .bind(ended)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO workload_intents (id,workload_id,generation,kind,pool_id,profile_id,target_id,expected_profile_revision,status,created_at,updated_at)
         VALUES (?,?,1,'ci_runner',?,?,?,1,'admitted',?,?)",
    )
    .bind(&operation)
    .bind(&workload)
    .bind(&fleet.pool_id)
    .bind(&fleet.profile_id)
    .bind(&fleet.target_id)
    .bind(at)
    .bind(ended)
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
    .bind(&fleet.target_id)
    .bind(end.state())
    .bind(at)
    .bind(ended)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO host_commands (id,operation_id,scope,placement_id,host_id,backend_id,host_epoch,generation,kind,state,protocol_version,request_hash,payload_json,expected_revision,issued_at,deadline_at,idempotency_key)
         VALUES (?1,?2,'placement',?3,?4,?5,1,1,'start_runner',?6,1,'testdata','{}',1,?7,?8,?1)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&operation)
    .bind(&placement)
    .bind(&host.id)
    .bind(&host.backend_id)
    .bind(if end == PlacementEnd::Failed {
        "failed"
    } else {
        "succeeded"
    })
    .bind(at)
    .bind(at + 60_000)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// What [`build_week`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeekSummary {
    pub fleet: SeededFleet,
    /// GitHub jobs completed in the week.
    pub jobs: usize,
    /// Root samples, one every 30 s for each machine.
    pub samples: usize,
}

/// The job names the week cycles through, with a typical duration (ms).
const WEEK_JOBS: [(&str, i64); 8] = [
    ("unit tests", 120_000),
    ("e2e (1/2)", 600_000),
    ("e2e (2/2)", 600_000),
    ("lint", 60_000),
    ("clippy", 240_000),
    ("build image", 300_000),
    ("deploy", 180_000),
    ("notify", 15_000),
];

/// The jobs per hour of the week: 300, so 50,400 in all.
const WEEK_JOBS_PER_HOUR: i64 = 300;

/// Seeds the fleet and seven days ending at `end` (a whole hour): about
/// 50,000 successful GitHub jobs, each its own run at attempt 1, spread over
/// the four machines with a CPU budget, and a root sample every 30 s for all
/// five machines.
///
/// # Errors
/// A write fails.
pub async fn build_week(db: &SqlitePool, end: i64) -> anyhow::Result<WeekSummary> {
    let fleet = seed_fleet(db).await?;
    let start = end - 7 * DAY;
    let budgeted: Vec<&SeededHost> = fleet
        .hosts
        .iter()
        .filter(|host| host.cpu_budget_millis > 0)
        .collect();
    let total = 7 * 24 * WEEK_JOBS_PER_HOUR;
    let spacing = 7 * DAY / total;
    let mut jobs = Vec::with_capacity(usize::try_from(total)?);
    for index in 0..total {
        let slot = usize::try_from(index)?;
        let (name, typical) = WEEK_JOBS[slot % WEEK_JOBS.len()];
        let Some(host) = budgeted.get(slot / WEEK_JOBS.len() % budgeted.len()) else {
            continue;
        };
        // Durations within ±25% of typical; waits up to 2 minutes.
        let duration = typical * (75 + (index * 37) % 51) / 100;
        let wait = (index * 7_919) % 120_000;
        let completed = start + index * spacing + spacing / 2;
        let started = completed - duration;
        let mut job = JobSeed::completed(
            index + 1,
            name,
            Some(host.id.as_str()),
            started - wait,
            started,
            completed,
        );
        if index % 5 == 0 {
            job.cpu_millis = 3_000;
        }
        jobs.push(job);
    }
    insert_jobs(db, &fleet, &jobs).await?;
    let mut samples = 0;
    for host in &fleet.hosts {
        let series: Vec<SampleSeed> = (0..7 * 24 * 120)
            .map(|index: i64| {
                let mut sample = SampleSeed::at(start + index * 30_000);
                let wave = u32::try_from(index % 120).unwrap_or(0);
                sample.cpu_used_millis = host.cpu_budget_millis / 4 + wave * 20;
                sample.memory_used_mib = host.memory_budget_mib / 3 + wave * 8;
                sample
            })
            .collect();
        samples += series.len();
        insert_samples(db, host, &series).await?;
    }
    Ok(WeekSummary {
        fleet,
        jobs: jobs.len(),
        samples,
    })
}
