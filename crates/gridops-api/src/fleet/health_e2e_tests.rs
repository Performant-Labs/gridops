//! #204 items 3 and 4: the fleet health page end to end, against an
//! independent oracle.
//!
//! A database is seeded with two days of a five-machine fleet (jobs,
//! placements, samples, runner starts and failures, category rules and
//! snapshots); the reconciler's own fleet health passes run on it
//! (`gridops_reconciler::fleet_health_pass`: the backfill, both rollups and
//! the snapshots) until they have caught up; then `GET /api/v1/fleet/health`
//! is read for every window through the production source and compared with
//! `scripts/fleet-health-oracle.py`, which computes the page from the raw
//! tables by the design doc without any `GridOps` code. Scores must match to
//! the integer, speed indexes to 0.01, concerns by key, severity and order.
//! A few values are also checked against hand-computed numbers.
//!
//! The seeded cases (#204's 2026-11-24 list): a GitHub re-run across an hour
//! boundary (attempt 2 of one job name while the run's other job keeps
//! attempt 1); a job completing in the hour still in its grace, counted
//! live and then rolled; a GitLab job with no path on an unscoped
//! connection, NULL (`other`) until the backfill can classify it and `test`
//! after; a machine with a budget of 0 (`MacBookPro`) and one with 0 online
//! minutes (europa); a nearly full root disk with a fresh heartbeat
//! (callisto, critical) and a low one with a stale heartbeat (Jupiter, no
//! alarm); a busy GitLab runner with no job row; stored category rules on the
//! workflow name and the labels; and seeded snapshots for the trend.
//!
//! The evaluation runs at two moments: `NOW_A`, ten minutes into hour `T`
//! (the hour that just ended is in its grace and computed live), and
//! `NOW_B`, 31 minutes in (it has been rolled up, and the backfill has
//! classified the pathless GitLab job).

use std::{collections::BTreeMap, path::Path, process::Command};

use anyhow::{Context as _, Result, bail, ensure};
use axum::http::StatusCode;
use gridops_core::{
    fleet::PlacementFailureReason,
    fleet_health::testdata::{
        self, CLASSIFIER_TEST_LOCK, JobSeed, PlacementEnd, SampleSeed, SeededFleet, SeededHost,
    },
};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::health_test_support::{HEALTH, Harness};

const MINUTE: i64 = 60_000;
const HOUR: i64 = 3_600_000;
const GIB: i64 = 1024 * 1024 * 1024;
/// A whole hour: the hour in progress at both evaluations.
const T: i64 = 1_800_000_000_000;
/// The hour before `T` is in its grace (computed live).
const NOW_A: i64 = T + 10 * MINUTE;
/// The hour before `T` is rolled up; the pathless GitLab job is classified.
const NOW_B: i64 = T + 31 * MINUTE;

/// The re-run: attempt 1 completes five hours before `T`, attempt 2 in the
/// next hour.
const RERUN_RUN: i64 = 900_000;
/// The job that completes 20 s before `T`, in the hour in its grace at
/// `NOW_A`.
const GRACE_JOB: i64 = 950_001;
/// A job of the hour in progress.
const CURRENT_JOB: i64 = 950_002;
/// The GitLab job with no path, on a connection scoped to a group.
const PATHLESS: &str = "e2e-pathless";
const QUEUED_JOB: i64 = 960_003;
const UNMATCHED_JOB: i64 = 960_004;
const BUSY_GITLAB_JOB: i64 = 960_001;
const RUNNING_JOB: i64 = 960_002;

/// One of the regular jobs: (name, typical duration in seconds).
const JOBS: [(&str, i64); 6] = [
    ("unit tests", 120),
    ("lint", 45),
    ("build image", 240),
    ("e2e (1/2)", 300),
    ("e2e (2/2)", 310),
    ("deploy", 90),
];

/// The scenario's seeded fleet.
struct Scenario {
    fleet: SeededFleet,
}

impl Scenario {
    fn host(&self, name: &str) -> Result<&SeededHost> {
        self.fleet.host(name)
    }

    fn id(&self, name: &str) -> Result<String> {
        Ok(self.host(name)?.id.clone())
    }
}

/// A sample of `host` at `at`: the CPU, memory and disk free its scenario
/// gives it. callisto's newest sample (30 s before an evaluation) has 3 GiB
/// free of its 100 GiB disk.
fn sample(host: &str, at: i64, newest: i64) -> SampleSeed {
    let mut sample = SampleSeed::at(at);
    let minute = u32::try_from((at / MINUTE).rem_euclid(10)).unwrap_or(0);
    match host {
        "io" => {
            // Busy every third hour, so those hours are no capacity misfit.
            let busy = (at / HOUR).rem_euclid(3) == 0;
            sample.cpu_used_millis = if busy { 20_000 } else { 6_000 } + minute * 100;
            sample.memory_used_mib = 20_000;
            sample.disk_free_bytes = Some(50 * GIB);
        }
        "Jupiter" => {
            sample.cpu_used_millis = 3_000 + minute * 50;
            sample.memory_used_mib = 30_000;
            sample.disk_free_bytes = Some(8 * GIB);
        }
        "callisto" => {
            sample.cpu_used_millis = 2_000 + minute * 20;
            sample.memory_used_mib = 15_000;
            sample.disk_free_bytes = Some(if at == newest { 3 * GIB } else { 30 * GIB });
        }
        _ => {
            sample.cpu_used_millis = 500;
            sample.memory_used_mib = 4_000;
        }
    }
    sample
}

/// A sample a minute for each sampled machine from `from` (exclusive) up to
/// `until` (inclusive): io, callisto and `MacBookPro` up to `until`, Jupiter
/// stopping four minutes earlier (a stale heartbeat while still online).
async fn insert_samples(db: &SqlitePool, scenario: &Scenario, from: i64, until: i64) -> Result<()> {
    for (name, last) in [
        ("io", until),
        ("callisto", until),
        ("MacBookPro", until),
        ("Jupiter", until - 210_000),
    ] {
        let host = scenario.host(name)?;
        let mut samples = Vec::new();
        let mut at = until;
        while at > from {
            if at <= last {
                samples.push(sample(name, at, until));
            }
            at -= MINUTE;
        }
        samples.reverse();
        testdata::insert_samples(db, host, &samples).await?;
    }
    Ok(())
}

/// Seeds the scenario around the whole hour `t` (`t` in the test), with
/// samples up to `samples_until` (`NOW_A - 30 s` in the test).
async fn seed(db: &SqlitePool, t: i64, samples_until: i64) -> Result<Scenario> {
    let now_a = t + 10 * MINUTE;
    let rerun_hour = t - 5 * HOUR;
    let fleet = testdata::seed_fleet(db).await?;
    let scenario = Scenario { fleet };
    let io = scenario.id("io")?;
    let jupiter = scenario.id("Jupiter")?;
    let callisto = scenario.id("callisto")?;

    // Two stored rules after the defaults: the Release workflow deploys, and
    // a job asking for a GPU builds.
    sqlx::query(
        "INSERT INTO job_category_rules (position,category,field,pattern,created_at,updated_at) VALUES
           (4,'deploy','workflow_name','^Release$',1,1),(5,'build','labels','^gpu$',1,1)",
    )
    .execute(db)
    .await?;

    // Two days of jobs, one every 10 minutes, each name on io, Jupiter and
    // callisto in turn (callisto taking half again as long).
    let mut jobs = Vec::new();
    for index in 0..288_i64 {
        let slot = usize::try_from(index)?;
        let (name, typical) = JOBS[slot % JOBS.len()];
        let host_slot = (index + index / 6) % 3;
        let host = match host_slot {
            0 => Some(io.as_str()),
            1 => Some(jupiter.as_str()),
            _ => Some(callisto.as_str()),
        };
        let factor = if host_slot == 2 { 150 } else { 100 };
        let duration = typical * 1_000 * factor / 100 * (100 + (index % 5) * 5) / 100;
        let wait = match index % 31 {
            0 => 3 * MINUTE,
            7 | 19 => 50_000,
            _ => 20_000 + (index % 4) * 5_000,
        };
        let completed = t - 48 * HOUR + index * 10 * MINUTE + 5 * MINUTE;
        let started = completed - duration;
        let mut job = JobSeed::completed(
            index + 1,
            name,
            if index % 23 == 0 { None } else { host },
            started - wait,
            started,
            completed,
        );
        if index % 13 == 0 && host_slot == 0 {
            job.cpu_millis = 3_000;
        }
        if index % 41 == 1 {
            job.placement = PlacementEnd::Failed;
            job.conclusion = Some("failure".to_owned());
        }
        if index % 29 == 5 {
            job.name = "notify".to_owned();
            job.workflow = "Release".to_owned();
        }
        if index == 250 {
            job.placement = PlacementEnd::Uncertain;
        }
        jobs.push(job);
    }

    // The re-run: "lint" fails at attempt 1 and passes at attempt 2 an hour
    // later; "build image" ran once, at attempt 1, and still counts.
    let rerun = |id: i64, name: &str, attempt: i64, completed: i64, success: bool| {
        let mut job = JobSeed::completed(
            id,
            name,
            Some(io.as_str()),
            completed - 4 * MINUTE,
            completed - 3 * MINUTE,
            completed,
        );
        job.run_id = RERUN_RUN;
        job.run_attempt = attempt;
        if !success {
            job.conclusion = Some("failure".to_owned());
        }
        job
    };
    jobs.push(rerun(
        RERUN_RUN + 1,
        "lint",
        1,
        rerun_hour + 50 * MINUTE,
        false,
    ));
    jobs.push(rerun(
        RERUN_RUN + 2,
        "build image",
        1,
        rerun_hour + 40 * MINUTE,
        true,
    ));
    jobs.push(rerun(
        RERUN_RUN + 3,
        "lint",
        2,
        rerun_hour + 70 * MINUTE,
        true,
    ));

    // The job finishing 20 s before `t`, and one in the hour in progress.
    jobs.push(JobSeed::completed(
        GRACE_JOB,
        "unit tests",
        Some(jupiter.as_str()),
        t - 4 * MINUTE,
        t - 3 * MINUTE,
        t - 20_000,
    ));
    jobs.push(JobSeed::completed(
        CURRENT_JOB,
        "lint",
        Some(io.as_str()),
        t,
        t + MINUTE,
        t + 5 * MINUTE,
    ));

    // GitLab jobs on io, on the project-scoped connection (path known).
    for index in 0..6_i64 {
        let completed = t - 20 * HOUR + index * 2 * HOUR;
        let mut job = JobSeed::completed(
            700 + index,
            "rspec",
            Some(io.as_str()),
            completed - 5 * MINUTE,
            completed - 4 * MINUTE,
            completed,
        );
        job.gitlab = true;
        jobs.push(job);
    }

    // Running now: a GitHub job on Jupiter, and a GitLab job on io whose row
    // is deleted below (a busy runner with no job row).
    jobs.push(JobSeed::running(
        RUNNING_JOB,
        "unit tests",
        Some(jupiter.as_str()),
        now_a - 6 * MINUTE,
        now_a - 5 * MINUTE,
    ));
    let mut busy = JobSeed::running(
        BUSY_GITLAB_JOB,
        "rspec",
        Some(io.as_str()),
        now_a - 6 * MINUTE,
        now_a - 5 * MINUTE,
    );
    busy.gitlab = true;
    jobs.push(busy);

    // Waiting: one the pool runs, queued 7 minutes before `now_a`, and one
    // asking for a label no pool offers.
    jobs.push(JobSeed::queued(
        QUEUED_JOB,
        "unit tests",
        now_a - 7 * MINUTE,
    ));
    let mut gpu = JobSeed::queued(UNMATCHED_JOB, "train model", now_a - 2 * MINUTE);
    gpu.labels = vec!["self-hosted".to_owned(), "gpu".to_owned()];
    jobs.push(gpu);

    testdata::insert_jobs(db, &scenario.fleet, &jobs).await?;
    sqlx::query("DELETE FROM gitlab_jobs WHERE job_id=?")
        .bind(BUSY_GITLAB_JOB)
        .execute(db)
        .await?;
    sqlx::query(
        "UPDATE runners SET current_job_id=NULL WHERE ci_platform='gitlab' AND status='busy'",
    )
    .execute(db)
    .await?;
    charge_capacity(db, now_a).await?;

    // The pathless GitLab job: a group-scoped connection, no path, and no
    // other row of its project, so the backfill leaves it NULL for an hour.
    sqlx::query(
        "INSERT INTO gitlab_connections (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_at,updated_at)
         VALUES ('e2e-unscoped','e2e group','https://gitlab.test','group',5,'acme','e2e.token','e2e.webhook',1,1)",
    )
    .execute(db)
    .await?;
    sqlx::query(
        "INSERT INTO gitlab_jobs (id,connection_id,project_id,job_id,pipeline_id,status,tag_list,status_changed_at,gitlab_runner_id,pool_id,created_at,updated_at,category,job_name,project_path,started_at,completed_at)
         VALUES (?,'e2e-unscoped',999,77001,1,'success','[\"docker\"]',?,NULL,NULL,?,?,NULL,'rspec unit',NULL,?,?)",
    )
    .bind(PATHLESS)
    .bind(t + 5 * MINUTE)
    .bind(now_a - 40 * MINUTE)
    .bind(t + 5 * MINUTE)
    .bind(t)
    .bind(t + 5 * MINUTE)
    .execute(db)
    .await?;

    // Runner starts: 24 on Jupiter, every fourth failing on the machine; 20
    // on io, all succeeding. Every one over an hour old, so no start gate.
    let jupiter_host = scenario.host("Jupiter")?;
    for index in 0..24_i64 {
        let at = t - 22 * HOUR + index * 30 * MINUTE;
        let failed = index % 4 == 0;
        testdata::insert_start_attempt(
            db,
            &scenario.fleet,
            jupiter_host,
            at,
            if failed {
                PlacementEnd::Failed
            } else {
                PlacementEnd::Stopped
            },
        )
        .await?;
        if failed {
            testdata::insert_placement_failure(
                db,
                jupiter_host,
                at,
                PlacementFailureReason::StartDeadlineExpired,
                None,
            )
            .await?;
        }
    }
    // A machine revoked since: its starts in the window still count for the
    // fleet, though it is no longer one of the page's machines.
    let revoked = SeededHost {
        id: "0f000001-0000-4000-8000-0000000000ff".to_owned(),
        name: "retired-box".to_owned(),
        root_domain_id: "0f000002-0000-4000-8000-0000000000ff".to_owned(),
        backend_id: "0f000003-0000-4000-8000-0000000000ff".to_owned(),
        ..scenario.host("europa")?.clone()
    };
    sqlx::query(
        "INSERT INTO fleet_hosts (id,name,host_os,architecture,enrollment_state,lifecycle_state,integrity_state,created_at,updated_at,deleted_at)
         VALUES (?,?,'linux','x64','revoked','retired','verified',1,1,?)",
    )
    .bind(&revoked.id)
    .bind(&revoked.name)
    .bind(t - 2 * HOUR)
    .execute(db)
    .await?;
    sqlx::query(
        "INSERT INTO host_resource_domains (id,host_id,parent_domain_id,name,cpu_millis,memory_mib,disk_bytes)
         VALUES (?,?,NULL,'Physical',4000,8192,0)",
    )
    .bind(&revoked.root_domain_id)
    .bind(&revoked.id)
    .execute(db)
    .await?;
    sqlx::query(
        "INSERT INTO host_backends (id,host_id,domain_id,runtime_kind,execution_os,architecture,readiness,enabled,created_at,updated_at)
         VALUES (?,?,?,'docker','linux','x64','ready',0,1,1)",
    )
    .bind(&revoked.backend_id)
    .bind(&revoked.id)
    .bind(&revoked.root_domain_id)
    .execute(db)
    .await?;
    for index in 0..4_i64 {
        let at = t - 12 * HOUR + index * 10 * MINUTE;
        testdata::insert_start_attempt(db, &scenario.fleet, &revoked, at, PlacementEnd::Failed)
            .await?;
        testdata::insert_placement_failure(
            db,
            &revoked,
            at,
            PlacementFailureReason::AgentSessionLost,
            None,
        )
        .await?;
    }
    let io_host = scenario.host("io")?;
    for index in 0..20_i64 {
        testdata::insert_start_attempt(
            db,
            &scenario.fleet,
            io_host,
            t - 20 * HOUR + index * 20 * MINUTE,
            PlacementEnd::Stopped,
        )
        .await?;
    }

    // Samples: europa's last was three days ago; the others report every
    // minute for two days.
    testdata::insert_samples(
        db,
        scenario.host("europa")?,
        &[SampleSeed::at(t - 72 * HOUR)],
    )
    .await?;
    insert_samples(db, &scenario, t - 48 * HOUR, samples_until).await?;

    // Earlier snapshots for the trend; one is older than the 24 h window.
    sqlx::query(
        r#"INSERT INTO fleet_health_snapshots (recorded_at,"window",fleet_score,host_scores_json) VALUES
             (?,'24h',71,'{}'),(?,'24h',74,'{}'),(?,'24h',NULL,'{}'),(?,'24h',60,'{}'),
             (?,'7d',70,'{}'),(?,'1h',66,'{}')"#,
    )
    .bind(t - 20 * HOUR)
    .bind(t - 12 * HOUR)
    .bind(t - 4 * HOUR)
    .bind(t - 30 * HOUR)
    .bind(t - 3 * 24 * HOUR)
    .bind(t - 20 * MINUTE)
    .execute(db)
    .await?;
    Ok(scenario)
}

/// The capacity admission charges, which the seed's placements otherwise
/// lack (the walkthrough showed "0 cores reserved" beside running jobs for
/// that reason alone): a committed allocation on the root domain for each
/// running placement, an uncertain one for the uncertain placement (it
/// still holds its capacity), and a released one for a finished placement,
/// which no longer counts.
async fn charge_capacity(db: &SqlitePool, at: i64) -> Result<()> {
    let placements: Vec<(String, String, String, i64, i64, String)> = sqlx::query_as(
        "SELECT p.id,p.host_id,p.backend_id,p.cpu_millis,p.memory_mib,p.state FROM workload_placements p
          WHERE p.state IN ('running','uncertain')
             OR p.id=(SELECT id FROM workload_placements WHERE state='stopped' ORDER BY id LIMIT 1)",
    )
    .fetch_all(db)
    .await?;
    ensure!(
        placements.len() >= 3,
        "running, uncertain and stopped placements"
    );
    for (id, host, backend, cpu, memory, state) in placements {
        let allocation = match state.as_str() {
            "running" => "committed",
            "uncertain" => "uncertain",
            _ => "released",
        };
        if allocation == "released" {
            // Released only once its runner is proven gone, as the
            // reconciler releases it.
            sqlx::query(
                "UPDATE workload_placements SET local_absence_proven_at=?,delayed_starts_excluded_at=? WHERE id=?",
            )
            .bind(at)
            .bind(at)
            .bind(&id)
            .execute(db)
            .await?;
        }
        sqlx::query(
            "INSERT INTO capacity_allocations (id,placement_id,host_id,backend_id,host_epoch,domain_id,policy_revision,cpu_millis,memory_mib,disk_bytes,state,created_at,updated_at,released_at)
             SELECT ?,?,?,?,1,d.id,1,?,?,0,?,?,?,? FROM host_resource_domains d
              WHERE d.host_id=? AND d.parent_domain_id IS NULL",
        )
        .bind(format!("e2e-allocation-{id}"))
        .bind(&id)
        .bind(&host)
        .bind(&backend)
        .bind(cpu)
        .bind(memory)
        .bind(allocation)
        .bind(at - 6 * MINUTE)
        .bind(at)
        .bind((allocation == "released").then_some(at))
        .bind(&host)
        .execute(db)
        .await?;
    }
    Ok(())
}

/// Runs the reconciler's fleet health passes at `now` until the catch-up
/// has nothing left (one catch-up hour per pass over 7 days).
async fn run_passes(db: &SqlitePool, now: i64, passes: usize) -> Result<()> {
    for _ in 0..passes {
        gridops_reconciler::fleet_health_pass(db, now).await?;
    }
    Ok(())
}

/// The oracle's answer for `window` at `now`.
fn oracle(path: &Path, window: &str, now: i64) -> Result<Value> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fleet-health-oracle.py");
    let output = Command::new("python3")
        .arg(&script)
        .arg(path)
        .arg("--now")
        .arg(now.to_string())
        .arg("--window")
        .arg(window)
        .output()
        .context("running python3 scripts/fleet-health-oracle.py")?;
    ensure!(
        output.status.success(),
        "the oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).context("the oracle's JSON")
}

/// Every difference between the API's response and the oracle's, as one
/// line each.
struct Diff {
    window: String,
    at: &'static str,
    lines: Vec<String>,
}

impl Diff {
    fn new(window: &str, at: &'static str) -> Self {
        Self {
            window: window.to_owned(),
            at,
            lines: Vec::new(),
        }
    }

    fn eq(&mut self, what: &str, api: &Value, oracle: &Value) {
        if api != oracle {
            self.lines
                .push(format!("{what}: API {api} != oracle {oracle}"));
        }
    }

    fn close(&mut self, what: &str, api: &Value, oracle: &Value, tolerance: f64) {
        match (api.as_f64(), oracle.as_f64()) {
            (Some(left), Some(right)) if (left - right).abs() <= tolerance => {}
            (None, None) if api.is_null() && oracle.is_null() => {}
            _ => self.lines.push(format!(
                "{what}: API {api} != oracle {oracle} (±{tolerance})"
            )),
        }
    }

    fn components(&mut self, what: &str, api: &Value, oracle: &Value) {
        let (api, oracle) = (array(api), array(oracle));
        if api.len() != oracle.len() {
            self.lines.push(format!(
                "{what}: {} components != {}",
                api.len(),
                oracle.len()
            ));
            return;
        }
        for (left, right) in api.iter().zip(oracle) {
            let key = right["key"].as_str().unwrap_or_default();
            self.eq(&format!("{what}.{key}.key"), &left["key"], &right["key"]);
            self.eq(
                &format!("{what}.{key}.score"),
                &left["score"],
                &right["score"],
            );
            self.eq(
                &format!("{what}.{key}.deduction"),
                &left["deduction"],
                &right["deduction"],
            );
            self.close(
                &format!("{what}.{key}.weight"),
                &left["weight"],
                &right["weight"],
                1e-9,
            );
        }
    }

    fn categories(&mut self, what: &str, api: &Value, oracle: &Value) {
        let (api, oracle) = (array(api), array(oracle));
        if api.len() != oracle.len() {
            self.lines
                .push(format!("{what}: API {api:?} != oracle {oracle:?}"));
            return;
        }
        for (left, right) in api.iter().zip(oracle) {
            let category = right["category"].as_str().unwrap_or_default();
            for field in ["category", "jobs", "medianDurationMs", "p90QueueWaitMs"] {
                self.eq(
                    &format!("{what}.{category}.{field}"),
                    &left[field],
                    &right[field],
                );
            }
            self.close(
                &format!("{what}.{category}.cpuHours"),
                &left["cpuHours"],
                &right["cpuHours"],
                1e-6,
            );
        }
    }

    fn usage(&mut self, what: &str, api: &Value, oracle: &Value) {
        let (api, oracle) = (array(api), array(oracle));
        let hours = |rows: &[Value]| {
            rows.iter()
                .map(|row| row["hour"].clone())
                .collect::<Vec<_>>()
        };
        if hours(api) != hours(oracle) {
            self.lines.push(format!(
                "{what}: hours {:?} != {:?}",
                hours(api),
                hours(oracle)
            ));
            return;
        }
        for (left, right) in api.iter().zip(oracle) {
            let (left, right) = (&left["cpuHours"], &right["cpuHours"]);
            let keys = |value: &Value| {
                value
                    .as_object()
                    .map(|map| map.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            };
            if keys(left) != keys(right) {
                self.lines
                    .push(format!("{what}: categories {left} != {right}"));
                continue;
            }
            for key in keys(left) {
                self.close(&format!("{what}.{key}"), &left[&key], &right[&key], 1e-6);
            }
        }
    }

    fn finish(self) -> Result<()> {
        if self.lines.is_empty() {
            return Ok(());
        }
        bail!(
            "{} at {}: the API and the oracle disagree:\n{}",
            self.window,
            self.at,
            self.lines.join("\n")
        )
    }
}

fn array(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

/// A count map without its zero entries, as the oracle writes them.
fn counts(value: &Value) -> Value {
    Value::Object(
        value
            .as_object()
            .map(|map| {
                map.iter()
                    .filter(|(_, count)| count.as_u64() != Some(0))
                    .map(|(key, count)| (key.clone(), count.clone()))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// Compares every number of the API's response with the oracle's.
fn compare(window: &str, at: &'static str, api: &Value, oracle: &Value) -> Result<()> {
    let mut diff = Diff::new(window, at);
    diff.eq(
        "fleet.score",
        &api["fleet"]["score"],
        &oracle["fleet"]["score"],
    );
    diff.components(
        "fleet.components",
        &api["fleet"]["components"],
        &oracle["fleet"]["components"],
    );
    diff.eq(
        "fleet.trend",
        &api["fleet"]["trend"],
        &oracle["fleet"]["trend"],
    );
    diff.categories(
        "fleet.categories",
        &api["fleet"]["categories"],
        &oracle["fleet"]["categories"],
    );

    let (live, expected) = (&api["live"], &oracle["live"]);
    for field in ["queuedTotal", "queuedUnmatched", "runningTotal"] {
        diff.eq(&format!("live.{field}"), &live[field], &expected[field]);
    }
    for field in ["queuedByCategory", "runningByCategory"] {
        diff.eq(
            &format!("live.{field}"),
            &counts(&live[field]),
            &counts(&expected[field]),
        );
    }
    let pools: Vec<Value> = array(&live["queuedByPool"])
        .iter()
        .map(|pool| json!([pool["poolId"], pool["queued"]]))
        .collect();
    diff.eq(
        "live.queuedByPool",
        &Value::Array(pools),
        &expected["queuedByPool"],
    );
    let oldest = &live["oldestQueued"];
    diff.eq(
        "live.oldestQueued",
        &if oldest.is_null() {
            Value::Null
        } else {
            json!({"jobId": oldest["jobId"], "waitMs": oldest["waitMs"]})
        },
        &expected["oldestQueued"],
    );

    let (hosts, expected_hosts) = (array(&api["hosts"]), array(&oracle["hosts"]));
    let ids = |hosts: &[Value]| {
        hosts
            .iter()
            .map(|host| host["id"].clone())
            .collect::<Vec<_>>()
    };
    diff.eq(
        "hosts",
        &Value::Array(ids(hosts)),
        &Value::Array(ids(expected_hosts)),
    );
    for (host, expected) in hosts.iter().zip(expected_hosts) {
        let name = expected["name"].as_str().unwrap_or_default();
        for field in [
            "name",
            "cpuBudgetMillis",
            "memoryBudgetMib",
            "diskTotalBytes",
            "score",
        ] {
            diff.eq(&format!("{name}.{field}"), &host[field], &expected[field]);
        }
        diff.components(
            &format!("{name}.components"),
            &host["components"],
            &expected["components"],
        );
        let (speed, expected_speed) = (array(&host["speed"]), array(&expected["speed"]));
        if speed.len() == expected_speed.len() {
            for (left, right) in speed.iter().zip(expected_speed) {
                let size = right["cpuMillis"].to_string();
                diff.eq(
                    &format!("{name}.speed.cpuMillis"),
                    &left["cpuMillis"],
                    &right["cpuMillis"],
                );
                diff.close(
                    &format!("{name}.speed[{size}].index"),
                    &left["index"],
                    &right["index"],
                    0.01,
                );
                diff.eq(
                    &format!("{name}.speed[{size}].topKeys"),
                    &left["topKeys"],
                    &right["topKeys"],
                );
            }
        } else {
            diff.lines.push(format!(
                "{name}.speed: API {} sizes != oracle {}",
                speed.len(),
                expected_speed.len()
            ));
        }
        diff.categories(
            &format!("{name}.categories"),
            &host["categories"],
            &expected["categories"],
        );
        diff.usage(&format!("{name}.usage"), &host["usage"], &expected["usage"]);
        let (live, expected_live) = (&host["live"], &expected["live"]);
        for field in [
            "online",
            "lastSeenAt",
            "cpuUsedMillis",
            "memoryUsedMib",
            "diskFreeBytes",
            "cpuReservedMillis",
            "memoryReservedMib",
        ] {
            diff.eq(
                &format!("{name}.live.{field}"),
                &live[field],
                &expected_live[field],
            );
        }
        diff.eq(
            &format!("{name}.live.runningByCategory"),
            &counts(&live["runningByCategory"]),
            &counts(&expected_live["runningByCategory"]),
        );
    }

    let concerns = |concerns: &[Value]| {
        concerns
            .iter()
            .map(|concern| json!([concern["key"], concern["severity"], concern["since"]]))
            .collect::<Vec<_>>()
    };
    diff.eq(
        "concerns",
        &Value::Array(concerns(array(&api["concerns"]))),
        &Value::Array(concerns(array(&oracle["concerns"]))),
    );
    diff.finish()
}

/// The jobs whose stored category differs from what the stored rules give
/// them, as the oracle classifies: `(job, stored, rules)`.
fn misclassified(oracle: &Value) -> Vec<(String, Value, String)> {
    array(&oracle["classification"])
        .iter()
        .filter(|job| job["stored"].as_str() != job["rules"].as_str())
        .map(|job| {
            (
                job["job"].as_str().unwrap_or_default().to_owned(),
                job["stored"].clone(),
                job["rules"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// The category usage of `window` as `category -> jobs`.
fn jobs_by_category(api: &Value) -> BTreeMap<String, u64> {
    array(&api["fleet"]["categories"])
        .iter()
        .map(|entry| {
            (
                entry["category"].as_str().unwrap_or_default().to_owned(),
                entry["jobs"].as_u64().unwrap_or_default(),
            )
        })
        .collect()
}

fn host<'a>(api: &'a Value, id: &str) -> Result<&'a Value> {
    array(&api["hosts"])
        .iter()
        .find(|host| host["id"] == id)
        .with_context(|| format!("no machine {id} in the response"))
}

fn component<'a>(host: &'a Value, key: &str) -> &'a Value {
    array(&host["components"])
        .iter()
        .find(|component| component["key"] == key)
        .map_or(&Value::Null, |component| &component["score"])
}

fn concern_keys(api: &Value) -> Vec<String> {
    array(&api["concerns"])
        .iter()
        .map(|concern| concern["key"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// With `GRIDOPS_FLEET_HEALTH_E2E_KEEP=<dir>`, copies the seeded database
/// and the API's responses at `NOW_B` there, for a look by hand or a UI run
/// against the same data (#204's walkthrough).
fn keep(harness: &Harness, responses: &BTreeMap<&str, Value>) -> Result<()> {
    let Some(directory) = std::env::var_os("GRIDOPS_FLEET_HEALTH_E2E_KEEP") else {
        return Ok(());
    };
    let directory = Path::new(&directory);
    std::fs::create_dir_all(directory)?;
    std::fs::copy(harness.path(), directory.join("fleet-health-e2e.sqlite"))?;
    for suffix in ["-wal", "-shm"] {
        let mut wal = harness.path().as_os_str().to_owned();
        wal.push(suffix);
        if Path::new(&wal).exists() {
            std::fs::copy(
                &wal,
                directory.join(format!("fleet-health-e2e.sqlite{suffix}")),
            )?;
        }
    }
    for (window, response) in responses {
        std::fs::write(
            directory.join(format!("api-{window}.json")),
            serde_json::to_string_pretty(response)?,
        )?;
    }
    Ok(())
}

#[tokio::test]
async fn the_page_matches_the_oracle_on_the_seeded_fleet() -> Result<()> {
    // The backfill classifies through the process-wide classifier cache.
    let _classifier = CLASSIFIER_TEST_LOCK.lock().await;
    let harness = Harness::with_database_source().await?;
    let db = harness.database.clone();
    let scenario = seed(&db, T, NOW_A - 30_000).await?;
    let path = harness.path().to_owned();
    let io = scenario.id("io")?;
    let jupiter = scenario.id("Jupiter")?;
    let callisto = scenario.id("callisto")?;
    let europa = scenario.id("europa")?;
    let macbook = scenario.id("MacBookPro")?;

    // NOW_A: the catch-up rolls every hour up to the one in its grace.
    run_passes(&db, NOW_A, 200).await?;
    let listed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM fleet_health_recompute")
        .fetch_one(&db)
        .await?;
    assert_eq!(listed, 0, "the re-run's earlier hour was rolled again");
    harness.clock.set(NOW_A);
    let mut at_a = BTreeMap::new();
    for window in ["1h", "24h", "7d"] {
        let (status, api) = harness
            .admin_get(&format!("{HEALTH}?window={window}"))
            .await?;
        assert_eq!(status, StatusCode::OK, "{api}");
        let expected = oracle(&path, window, NOW_A)?;
        compare(window, "NOW_A", &api, &expected)?;
        at_a.insert(window, (api, expected));
    }
    // Before the backfill can classify it, the pathless GitLab job is NULL,
    // which every reader counts as `other`, and it is the only job the rules
    // would classify differently. The 1 h window holds the hour in progress:
    // the lint job and the pathless one.
    let (hour_a, oracle_a) = &at_a["1h"];
    assert_eq!(
        misclassified(oracle_a),
        [(format!("gitlab:{PATHLESS}"), Value::Null, "test".to_owned())]
    );
    assert_eq!(
        jobs_by_category(hour_a),
        BTreeMap::from([("check".to_owned(), 1), ("other".to_owned(), 1)])
    );

    // NOW_B: two more minutes of samples, the passes again.
    insert_samples(&db, &scenario, NOW_A - 30_000, NOW_B - 30_000).await?;
    run_passes(&db, NOW_B, 3).await?;
    harness.clock.set(NOW_B);
    // The API's speed refresher has read every window by now (every 4
    // minutes); the indexes cached at NOW_A are 21 minutes old.
    harness.refresh_speed(NOW_B).await?;
    let mut at_b = BTreeMap::new();
    for window in ["1h", "24h", "7d"] {
        let (status, api) = harness
            .admin_get(&format!("{HEALTH}?window={window}"))
            .await?;
        assert_eq!(status, StatusCode::OK, "{api}");
        let expected = oracle(&path, window, NOW_B)?;
        compare(window, "NOW_B", &api, &expected)?;
        // The snapshot the pass recorded at NOW_B holds the scores the
        // oracle computes at NOW_B: the fleet's on the trend, and every
        // machine's in the row.
        let recorded = array(&api["fleet"]["trend"])
            .iter()
            .find(|point| point["at"] == NOW_B)
            .with_context(|| format!("{window}: no trend point at NOW_B"))?;
        assert_eq!(
            recorded["score"], expected["fleet"]["score"],
            "{window}: the recorded fleet score"
        );
        let machines: String = sqlx::query_scalar(
            r#"SELECT host_scores_json FROM fleet_health_snapshots WHERE recorded_at=? AND "window"=?"#,
        )
        .bind(NOW_B)
        .bind(window)
        .fetch_one(&db)
        .await?;
        let machines: BTreeMap<String, Value> = serde_json::from_str(&machines)?;
        let expected_machines: BTreeMap<String, Value> = array(&expected["hosts"])
            .iter()
            .map(|host| {
                (
                    host["id"].as_str().unwrap_or_default().to_owned(),
                    host["score"].clone(),
                )
            })
            .collect();
        assert_eq!(
            machines, expected_machines,
            "{window}: the recorded machine scores"
        );
        assert!(
            misclassified(&expected).is_empty(),
            "every job has the rules' category: {:?}",
            misclassified(&expected)
        );
        at_b.insert(window, api);
    }
    keep(&harness, &at_b)?;

    // Hand-computed values.
    let (hour_b, day) = (&at_b["1h"], &at_b["24h"]);
    assert_eq!(
        jobs_by_category(hour_b),
        BTreeMap::from([("check".to_owned(), 1), ("test".to_owned(), 1)]),
        "the backfill moved the pathless GitLab job from other to test"
    );
    // 48 starts in the day: 6 failing on Jupiter and 4 on the revoked
    // machine, so 38/48 = 79.2% succeeded, under the 80% floor: 0.
    assert_eq!(day["fleet"]["components"][1]["key"], "start-reliability");
    assert_eq!(day["fleet"]["components"][1]["score"], 0);
    assert_eq!(
        component(host(day, &jupiter)?, "start-reliability"),
        0,
        "18 of 24"
    );
    assert_eq!(
        component(host(day, &io)?, "start-reliability"),
        100,
        "20 of 20"
    );
    // europa has rows with 0 online minutes: offline, not "not enough data".
    assert_eq!(component(host(day, &europa)?, "availability"), 0);
    // MacBookPro has no CPU budget: no score, every component without one.
    let macbook_host = host(day, &macbook)?;
    assert_eq!(macbook_host["score"], Value::Null);
    assert!(
        array(&macbook_host["components"])
            .iter()
            .all(|part| part["score"].is_null())
    );
    // Jupiter's disk floor reads 8 GiB free of 100: between the 10 GiB and 5
    // GiB floors, 60; its memory p95 30,000 of 49,152 MiB is under 80%.
    assert_eq!(component(host(day, &jupiter)?, "headroom"), 60);
    // callisto's newest sample has 3 GiB free: critical, and its heartbeat is
    // fresh. Jupiter's 8 GiB would warn, but its heartbeat is 4 minutes old.
    let keys = concern_keys(day);
    assert!(keys.contains(&format!("disk:{callisto}")), "{keys:?}");
    assert!(!keys.contains(&format!("disk:{jupiter}")), "{keys:?}");
    assert!(keys.contains(&format!("host-offline:{europa}")), "{keys:?}");
    // Job 250 ended uncertain, on the machine its slot gives it.
    let uncertain = scenario.id(["io", "Jupiter", "callisto"][(250 + 250 / 6) % 3])?;
    assert!(keys.contains(&format!("uncertain:{uncertain}")), "{keys:?}");
    assert!(keys.contains(&"unmatched-jobs".to_owned()), "{keys:?}");
    assert!(keys.contains(&"oldest-queued".to_owned()), "{keys:?}");
    assert!(
        !keys
            .iter()
            .any(|key| key.starts_with("host-score:") && key.ends_with(&macbook))
    );
    // The busy GitLab runner with no job row runs `other` on io.
    assert_eq!(
        host(day, &io)?["live"]["runningByCategory"],
        json!({"other": 1})
    );
    assert_eq!(
        host(day, &jupiter)?["live"]["runningByCategory"],
        json!({"test": 1})
    );
    assert_eq!(day["live"]["oldestQueued"]["jobId"], QUEUED_JOB);
    assert_eq!(
        day["live"]["oldestQueued"]["waitMs"],
        NOW_B - (NOW_A - 7 * MINUTE)
    );
    assert_eq!(day["live"]["queuedUnmatched"], 1);
    // The trend: the seeded snapshots of the day and the one the pass
    // recorded at NOW_B, oldest first.
    assert_eq!(
        day["fleet"]["trend"].as_array().map(|points| points
            .iter()
            .map(|point| point["at"].clone())
            .collect::<Vec<_>>()),
        Some(vec![
            json!(T - 20 * HOUR),
            json!(T - 12 * HOUR),
            json!(T - 4 * HOUR),
            json!(NOW_B)
        ])
    );
    Ok(())
}

/// Seeds the scenario around the current hour into
/// `GRIDOPS_FLEET_HEALTH_WALKTHROUGH_DB` (a new file) and runs the passes,
/// for the UI walkthrough (#204 item 5): the same fleet, with samples three
/// hours into the future so the machines stay online while it runs.
#[tokio::test]
#[ignore = "by hand, for the UI walkthrough (GRIDOPS_FLEET_HEALTH_WALKTHROUGH_DB)"]
async fn seed_for_walkthrough() -> Result<()> {
    let path = std::path::PathBuf::from(
        std::env::var_os("GRIDOPS_FLEET_HEALTH_WALKTHROUGH_DB")
            .context("GRIDOPS_FLEET_HEALTH_WALKTHROUGH_DB names the database to create")?,
    );
    ensure!(!path.exists(), "{} exists already", path.display());
    let _classifier = CLASSIFIER_TEST_LOCK.lock().await;
    let db = gridops_core::connect_database_path(&path).await?;
    let now = gridops_core::now_millis();
    let t = gridops_core::fleet_health::hour::floor_hour(now - 10 * MINUTE);
    let now_a = t + 10 * MINUTE;
    seed(&db, t, now + 3 * HOUR).await?;
    run_passes(&db, now_a, 200).await?;
    run_passes(&db, now, 3).await?;
    db.close().await;
    println!("seeded {} around {t}", path.display());
    Ok(())
}
