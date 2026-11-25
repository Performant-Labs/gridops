//! #259: a reconciler pass finishes only the ended placements that still
//! have something to finish. Every pass used to read and finish every ended
//! placement ever made, so its work and the reconciler's memory grew with the
//! fleet's history until it was killed at its memory limit.
//!
//! A placement that ends (`failed`, `cleaned` or `uncertain`) joins the
//! finish queue; the reconciler finishes it and takes a failed or cleaned one
//! off the queue, so a pass reads the queue and nothing of the history.
//! Placements that ended before the queue existed are queued by a sweep in
//! short batches, and only when they still have something to finish.

use std::sync::{
    Arc,
    atomic::{AtomicI64, AtomicU64, Ordering},
};

use anyhow::Result;
use gridops_core::{
    connect_database_path,
    fleet::{
        failure::PlacementFailure,
        placement::{self, Placement},
    },
    fleet_health::{
        speed_check,
        testdata::{self, SeededFleet, SeededHost},
    },
};
use sqlx::{
    SqliteConnection, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use uuid::Uuid;

/// How many placements [`ended_placement`] has made. Each is a second older
/// than the one before it, so the oldest-first order is the reverse of the
/// order they are made in, not the table's own order.
static MADE: AtomicI64 = AtomicI64::new(0);

struct Database {
    directory: std::path::PathBuf,
    path: std::path::PathBuf,
    db: SqlitePool,
    fleet: SeededFleet,
}

impl Database {
    async fn new() -> Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("gridops-ended-placements-{}", Uuid::new_v4()));
        let path = directory.join("db.sqlite");
        let db = connect_database_path(&path).await?;
        let fleet = testdata::seed_fleet(&db).await?;
        Ok(Self {
            directory,
            path,
            db,
            fleet,
        })
    }

    fn host(&self) -> &SeededHost {
        &self.fleet.hosts[0]
    }

    /// One placement left as `ended` describes.
    async fn ended(&self, ended: &Ended<'_>) -> Result<String> {
        let mut conn = self.db.acquire().await?;
        ended_placement(&mut conn, &self.fleet, self.host(), ended).await
    }

    /// `count` placements left as `ended` describes, in one transaction.
    async fn many(&self, count: usize, ended: &Ended<'_>) -> Result<Vec<String>> {
        let mut tx = self.db.begin().await?;
        let mut made = Vec::with_capacity(count);
        for _ in 0..count {
            made.push(ended_placement(&mut tx, &self.fleet, self.host(), ended).await?);
        }
        tx.commit().await?;
        Ok(made)
    }

    async fn queued(&self) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM placement_finish_queue")
                .fetch_one(&self.db)
                .await?,
        )
    }

    /// The database as migration 0052 found one with placements already
    /// ended: nothing queued, and the sweep of every existing placement still
    /// to run from the first placement id.
    async fn as_before_the_queue(&self) -> Result<()> {
        sqlx::query("DELETE FROM placement_finish_queue")
            .execute(&self.db)
            .await?;
        sqlx::query("UPDATE placement_finish_sweep SET last_id='',done=0")
            .execute(&self.db)
            .await?;
        Ok(())
    }

    async fn close(self) -> Result<()> {
        self.db.close().await;
        std::fs::remove_dir_all(self.directory)?;
        Ok(())
    }
}

/// How an ended placement is left.
#[derive(Clone, Copy)]
struct Ended<'a> {
    state: &'a str,
    /// Its runner row's status; `deleted` also sets `deleted_at`.
    runner: &'a str,
    /// A `start_runner` command was issued.
    started: bool,
    /// `local_absence_proven_at` and `delayed_starts_excluded_at` are set.
    absence_recorded: bool,
    /// It still has an unreleased capacity allocation; otherwise its
    /// allocation was released, or it has none when no absence is recorded.
    holds_capacity: bool,
}

/// Cleaned, its runner deleted: nothing left to do.
const SETTLED: Ended<'static> = Ended {
    state: "cleaned",
    runner: "deleted",
    started: true,
    absence_recorded: true,
    holds_capacity: false,
};

/// A start that failed on its machine, its runner retired. Abandonment never
/// touches a started placement, so its absence stays unrecorded for good.
const STARTED_FAILURE: Ended<'static> = Ended {
    state: "failed",
    runner: "failed",
    started: true,
    absence_recorded: false,
    holds_capacity: false,
};

/// A placement on `host` left as `ended` describes, with its runner row.
/// Answers the placement id.
async fn ended_placement(
    conn: &mut SqliteConnection,
    fleet: &SeededFleet,
    host: &SeededHost,
    ended: &Ended<'_>,
) -> Result<String> {
    let (operation, placement, workload) = (
        Uuid::new_v4().to_string(),
        Uuid::new_v4().to_string(),
        Uuid::new_v4().to_string(),
    );
    // Each placement is a second older than the one made before it.
    let at = gridops_core::now_millis() - 3_600_000 - 1_000 * MADE.fetch_add(1, Ordering::Relaxed);
    sqlx::query(
        "INSERT INTO runners (id,pool_id,name,status,deleted_at,created_at,updated_at)
         VALUES (?,?,?,?,?,?,?)",
    )
    .bind(&workload)
    .bind(&fleet.pool_id)
    .bind(format!("runner-{workload}"))
    .bind(ended.runner)
    .bind((ended.runner == "deleted").then_some(at))
    .bind(at)
    .bind(at)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO fleet_operations (id,kind,principal_kind,principal_scope,idempotency_key,request_hash,status,created_at,updated_at)
         VALUES (?1,'submit_workload','autoscaler','autoscaler:test',?1,'test','succeeded',?2,?2)",
    )
    .bind(&operation)
    .bind(at)
    .execute(&mut *conn)
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
    .bind(at)
    .execute(&mut *conn)
    .await?;
    let absence = ended.absence_recorded.then_some(at);
    sqlx::query(
        "INSERT INTO workload_placements (id,intent_id,workload_id,generation,host_id,backend_id,host_epoch,profile_revision,target_id,config_snapshot_json,cpu_millis,memory_mib,disk_bytes,state,created_at,updated_at,local_absence_proven_at,delayed_starts_excluded_at)
         VALUES (?,?,?,1,?,?,1,1,?,'{}',2000,4096,0,?,?,?,?,?)",
    )
    .bind(&placement)
    .bind(&operation)
    .bind(&workload)
    .bind(&host.id)
    .bind(&host.backend_id)
    .bind(&fleet.target_id)
    .bind(ended.state)
    .bind(at)
    .bind(at)
    .bind(absence)
    .bind(absence)
    .execute(&mut *conn)
    .await?;
    if ended.started {
        sqlx::query(
            "INSERT INTO host_commands (id,operation_id,scope,placement_id,host_id,backend_id,host_epoch,generation,kind,state,protocol_version,request_hash,payload_json,expected_revision,issued_at,deadline_at,idempotency_key)
             VALUES (?1,?2,'placement',?3,?4,?5,1,1,'start_runner','succeeded',1,'test','{}',1,?6,?7,?1)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&operation)
        .bind(&placement)
        .bind(&host.id)
        .bind(&host.backend_id)
        .bind(at)
        .bind(at + 60_000)
        .execute(&mut *conn)
        .await?;
    }
    // A released allocation needs the absence recorded; without it the
    // placement holds none.
    if ended.holds_capacity || ended.absence_recorded {
        sqlx::query(
            "INSERT INTO capacity_allocations (id,placement_id,host_id,backend_id,host_epoch,domain_id,policy_revision,cpu_millis,memory_mib,disk_bytes,state,created_at,updated_at,released_at)
             VALUES (?,?,?,?,1,?,1,2000,4096,0,?,?,?,?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&placement)
        .bind(&host.id)
        .bind(&host.backend_id)
        .bind(&host.root_domain_id)
        .bind(if ended.holds_capacity {
            "committed"
        } else {
            "released"
        })
        .bind(at)
        .bind(at)
        .bind((!ended.holds_capacity).then_some(at))
        .execute(&mut *conn)
        .await?;
    }
    Ok(placement)
}

async fn to_finish(db: &SqlitePool) -> Result<Vec<Placement>> {
    Ok(placement::ended_placements_to_finish(db).await?)
}

async fn ids(db: &SqlitePool) -> Result<Vec<String>> {
    Ok(to_finish(db)
        .await?
        .into_iter()
        .map(|placement| placement.id.to_string())
        .collect())
}

/// Finishes every queued placement the way a pass does once its `finish`
/// has nothing left to do: takes it off the queue in the state it was read.
async fn finish_all(db: &SqlitePool) -> Result<()> {
    for placement in to_finish(db).await? {
        placement::finished(db, placement.id, &placement.state).await?;
    }
    Ok(())
}

#[tokio::test]
async fn every_placement_that_ends_is_finished_once_oldest_first() -> Result<()> {
    let d = Database::new().await?;
    let cleaned = d.ended(&SETTLED).await?;
    let started_failure = d.ended(&STARTED_FAILURE).await?;
    let uncertain = d
        .ended(&Ended {
            state: "uncertain",
            ..SETTLED
        })
        .await?;
    let running = d
        .ended(&Ended {
            state: "running",
            runner: "online",
            ..SETTLED
        })
        .await?;

    // Every ended placement, settled or not, is finished once; oldest first,
    // which is the reverse of the order they were made in. A running one is
    // followed elsewhere in the pass.
    assert_eq!(
        ids(&d.db).await?,
        [&uncertain, &started_failure, &cleaned].map(String::clone)
    );

    // Finished, a failed or cleaned placement leaves the queue for good; an
    // uncertain one stays until it is cleaned.
    finish_all(&d.db).await?;
    assert_eq!(ids(&d.db).await?, std::slice::from_ref(&uncertain));
    finish_all(&d.db).await?;
    assert_eq!(ids(&d.db).await?, std::slice::from_ref(&uncertain));

    // A placement that ends later, or ends again in another state, is queued
    // again.
    sqlx::query("UPDATE workload_placements SET state='failed' WHERE id=?")
        .bind(&running)
        .execute(&d.db)
        .await?;
    sqlx::query("UPDATE workload_placements SET state='cleaned' WHERE id=?")
        .bind(&uncertain)
        .execute(&d.db)
        .await?;
    sqlx::query("UPDATE workload_placements SET state='cleaned' WHERE id=?")
        .bind(&started_failure)
        .execute(&d.db)
        .await?;
    assert_eq!(
        ids(&d.db).await?,
        [&running, &uncertain, &started_failure].map(String::clone)
    );

    // A placement read in one state that has since ended in another is not
    // taken off: it has not been finished in its new state. (Its runner is
    // retired first, as `finish` does.)
    sqlx::query(
        "UPDATE runners SET status='failed' WHERE id=(SELECT workload_id FROM workload_placements WHERE id=?)",
    )
    .bind(&running)
    .execute(&d.db)
    .await?;
    assert!(!placement::finished(&d.db, running.parse()?, "cleaned").await?);
    assert!(placement::finished(&d.db, running.parse()?, "failed").await?);
    assert_eq!(
        ids(&d.db).await?,
        [&uncertain, &started_failure].map(String::clone)
    );
    d.close().await
}

#[tokio::test]
async fn a_finished_placement_whose_runner_comes_back_is_finished_again() -> Result<()> {
    let d = Database::new().await?;
    let placement = d.ended(&STARTED_FAILURE).await?;
    let runner: String =
        sqlx::query_scalar("SELECT workload_id FROM workload_placements WHERE id=?")
            .bind(&placement)
            .fetch_one(&d.db)
            .await?;
    finish_all(&d.db).await?;
    assert_eq!(d.queued().await?, 0);

    // A late job event marks the retired runner busy again, by name, as
    // `associate_runner_with_job` does: the pass has to retire it again.
    sqlx::query("UPDATE runners SET busy=1,status='busy' WHERE id=? AND deleted_at IS NULL")
        .bind(&runner)
        .execute(&d.db)
        .await?;
    assert_eq!(ids(&d.db).await?, std::slice::from_ref(&placement));

    // Revived again after the pass retired it but before the pass took the
    // placement off: the placement stays queued, so the next pass retires
    // the runner again.
    assert!(!placement::finished(&d.db, placement.parse()?, "failed").await?);
    assert_eq!(ids(&d.db).await?, std::slice::from_ref(&placement));

    // Retired and finished again, it leaves; a runner that only changes
    // between retired states queues nothing.
    sqlx::query("UPDATE runners SET busy=0,status='failed' WHERE id=?")
        .bind(&runner)
        .execute(&d.db)
        .await?;
    finish_all(&d.db).await?;
    sqlx::query("UPDATE runners SET status='deleted',deleted_at=1 WHERE id=?")
        .bind(&runner)
        .execute(&d.db)
        .await?;
    assert_eq!(d.queued().await?, 0);
    d.close().await
}

#[tokio::test]
async fn placements_that_ended_before_the_queue_are_queued_only_with_work_left() -> Result<()> {
    let d = Database::new().await?;
    // Settled: nothing left to do.
    d.ended(&SETTLED).await?;
    d.ended(&Ended {
        runner: "failed",
        ..SETTLED
    })
    .await?;
    d.ended(&STARTED_FAILURE).await?;
    // An unstarted failure that abandonment already settled.
    d.ended(&Ended {
        state: "failed",
        runner: "failed",
        started: false,
        ..SETTLED
    })
    .await?;
    d.ended(&Ended {
        state: "running",
        runner: "online",
        ..SETTLED
    })
    .await?;

    // In the order they are made; the newest first.
    let mut expected = vec![
        // Its runner is still in service and must be retired.
        d.ended(&Ended {
            runner: "online",
            ..SETTLED
        })
        .await?,
        // A failed start whose runner still waits: retried or retired.
        d.ended(&Ended {
            state: "failed",
            runner: "starting",
            ..SETTLED
        })
        .await?,
        // Cleaned up until it is cleaned.
        d.ended(&Ended {
            state: "uncertain",
            ..SETTLED
        })
        .await?,
    ];
    // Unstarted failures that abandonment has not settled yet.
    let unrecorded = d
        .ended(&Ended {
            state: "failed",
            runner: "failed",
            started: false,
            absence_recorded: false,
            holds_capacity: false,
        })
        .await?;
    let holding = d
        .ended(&Ended {
            state: "failed",
            runner: "failed",
            started: false,
            absence_recorded: true,
            holds_capacity: true,
        })
        .await?;
    expected.extend([unrecorded.clone(), holding.clone()]);
    expected.reverse();

    d.as_before_the_queue().await?;
    assert_eq!(placement::queue_earlier_ended_placements(&d.db).await?, 5);
    assert_eq!(ids(&d.db).await?, expected);
    // The sweep is done: it queues nothing more, ever.
    assert_eq!(placement::queue_earlier_ended_placements(&d.db).await?, 0);

    // Abandonment settles the unstarted failures; finished, they leave.
    for placement_id in [&unrecorded, &holding] {
        placement::abandon_unstarted(&d.db, placement_id.parse()?, &PlacementFailure::unknown())
            .await?;
        placement::finished(&d.db, placement_id.parse()?, "failed").await?;
    }
    expected.retain(|placement_id| ![&unrecorded, &holding].contains(&placement_id));
    assert_eq!(ids(&d.db).await?, expected);
    d.close().await
}

#[tokio::test]
async fn the_sweep_resumes_after_the_last_placement_id_it_reached() -> Result<()> {
    let d = Database::new().await?;
    let in_service = Ended {
        runner: "online",
        ..SETTLED
    };
    let mut made = d.many(4, &in_service).await?;
    d.as_before_the_queue().await?;
    // Halfway: the sweep goes by placement id, which a VACUUM (or a backup
    // made with VACUUM INTO) never changes, unlike the rowid.
    made.sort();
    sqlx::query("UPDATE placement_finish_sweep SET last_id=?")
        .bind(&made[1])
        .execute(&d.db)
        .await?;
    assert_eq!(placement::queue_earlier_ended_placements(&d.db).await?, 2);
    let mut queued: Vec<String> =
        sqlx::query_scalar("SELECT placement_id FROM placement_finish_queue ORDER BY placement_id")
            .fetch_all(&d.db)
            .await?;
    queued.sort();
    assert_eq!(queued, made[2..]);
    let done: bool = sqlx::query_scalar("SELECT done FROM placement_finish_sweep")
        .fetch_one(&d.db)
        .await?;
    assert!(done);
    d.close().await
}

/// SQLite VM instructions run by `work` on `pool`, a one-connection pool.
async fn vm_steps<T>(
    pool: &SqlitePool,
    work: impl std::future::Future<Output = Result<T>>,
) -> Result<(T, u64)> {
    let steps = Arc::new(AtomicU64::new(0));
    {
        let mut conn = pool.acquire().await?;
        let mut handle = conn.lock_handle().await?;
        let counter = Arc::clone(&steps);
        handle.set_progress_handler(1, move || {
            counter.fetch_add(1, Ordering::Relaxed);
            true
        });
    }
    let value = work.await?;
    let mut conn = pool.acquire().await?;
    conn.lock_handle().await?.remove_progress_handler();
    Ok((value, steps.load(Ordering::Relaxed)))
}

/// What a pass reads to find what to finish: the sweep's progress and the
/// queue.
async fn pass_reads(pool: &SqlitePool) -> Result<Vec<Placement>> {
    placement::queue_earlier_ended_placements(pool).await?;
    to_finish(pool).await
}

#[tokio::test]
async fn a_pass_reads_none_of_the_settled_started_failures() -> Result<()> {
    const FAILURES: usize = 1_500;
    let d = Database::new().await?;
    let uncertain = d
        .ended(&Ended {
            state: "uncertain",
            ..SETTLED
        })
        .await?;
    d.many(FAILURES, &STARTED_FAILURE).await?;
    assert_eq!(to_finish(&d.db).await?.len(), FAILURES + 1);
    finish_all(&d.db).await?;
    assert_eq!(d.queued().await?, 1);

    // Counted on a connection of its own, so nothing else runs on it.
    let counted = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(&d.path))
        .await?;
    // Once uncounted: the first statement on a connection loads the schema.
    pass_reads(&counted).await?;
    let (found, settled_once) = vm_steps(&counted, pass_reads(&counted)).await?;
    assert_eq!(
        found.iter().map(|p| p.id.to_string()).collect::<Vec<_>>(),
        std::slice::from_ref(&uncertain)
    );

    // Twice the history of settled started failures: the pass reads exactly
    // as much as before, and far less than one instruction per failure.
    d.many(FAILURES, &STARTED_FAILURE).await?;
    finish_all(&d.db).await?;
    let (found, settled_twice) = vm_steps(&counted, pass_reads(&counted)).await?;
    assert_eq!(found.len(), 1);
    assert_eq!(
        settled_twice, settled_once,
        "the pass's reads grew with the settled history"
    );
    assert!(
        settled_once < 500,
        "the pass ran {settled_once} VM instructions with one placement left to finish"
    );
    counted.close().await;
    d.close().await
}

#[tokio::test]
async fn a_sweep_batch_reads_only_its_own_placements() -> Result<()> {
    const SETTLED_BEFORE: usize = 3_000;
    let d = Database::new().await?;
    let mut made = d.many(SETTLED_BEFORE, &SETTLED).await?;
    made.extend(
        d.many(
            3,
            &Ended {
                runner: "online",
                ..SETTLED
            },
        )
        .await?,
    );
    d.as_before_the_queue().await?;
    // Only the last few placement ids are left to sweep.
    made.sort();
    let last = made.len() - 4;
    sqlx::query("UPDATE placement_finish_sweep SET last_id=?")
        .bind(&made[last])
        .execute(&d.db)
        .await?;
    let counted = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(&d.path))
        .await?;
    // Once uncounted, on a copy of the sweep's state, to load the schema.
    counted.acquire().await?;
    sqlx::query("SELECT 1 FROM placement_finish_sweep")
        .execute(&counted)
        .await?;
    let (_, steps) = vm_steps(&counted, async {
        Ok(placement::queue_earlier_ended_placements(&counted).await?)
    })
    .await?;
    // A batch that visited the settled placements before its range would
    // run several instructions for each of them.
    assert!(
        steps < 2_000,
        "sweeping the last 3 placements ran {steps} VM instructions with {SETTLED_BEFORE} settled before them"
    );
    counted.close().await;
    d.close().await
}

#[tokio::test]
async fn sweeping_earlier_placements_holds_the_write_lock_briefly() -> Result<()> {
    const EARLIER: usize = 3_000;
    let d = Database::new().await?;
    // Half settled, half with a runner still in service.
    d.many(EARLIER / 2, &STARTED_FAILURE).await?;
    d.many(
        EARLIER / 2,
        &Ended {
            runner: "online",
            ..SETTLED
        },
    )
    .await?;
    d.as_before_the_queue().await?;

    // Every batch is its own short write transaction (#222's 50 ms rule).
    let queued = speed_check::check(&d.db, async {
        Ok(placement::queue_earlier_ended_placements(&d.db).await?)
    })
    .await?;
    assert_eq!(queued, EARLIER / 2);
    assert_eq!(d.queued().await?, i64::try_from(EARLIER / 2)?);
    d.close().await
}
