//! #259: a reconciler pass finishes only the ended placements that still
//! have something to finish, never the settled ones. The settled ones are
//! nearly every placement ever made; finishing all of them on every pass made
//! the reconciler's work and memory grow with the fleet's history until it
//! was killed at its memory limit.

use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::Result;
use gridops_core::{
    connect_database_path,
    fleet::{
        failure::PlacementFailure,
        placement::{self, Placement},
    },
    fleet_health::testdata::{self, SeededFleet, SeededHost},
};
use sqlx::SqlitePool;
use uuid::Uuid;

/// How many placements [`ended_placement`] has made. Each is a second older
/// than the one before it, so the oldest-first order is the reverse of the
/// order they are made in, not the table's own order.
static MADE: AtomicI64 = AtomicI64::new(0);

async fn database() -> Result<(std::path::PathBuf, SqlitePool)> {
    let directory =
        std::env::temp_dir().join(format!("gridops-ended-placements-{}", Uuid::new_v4()));
    let pool = connect_database_path(&directory.join("db.sqlite")).await?;
    Ok((directory, pool))
}

/// How an ended placement is left.
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

/// A placement on `host` left as `ended` describes, with its runner row.
/// Answers the placement id.
async fn ended_placement(
    db: &SqlitePool,
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
    let mut tx = db.begin().await?;
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
    .bind(&fleet.pool_id)
    .bind(&fleet.profile_id)
    .bind(&fleet.target_id)
    .bind(at)
    .bind(at)
    .execute(&mut *tx)
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
    .execute(&mut *tx)
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
        .execute(&mut *tx)
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
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(placement)
}

async fn to_finish(db: &SqlitePool) -> Result<Vec<String>> {
    Ok(placement::ended_placements_to_finish(db)
        .await?
        .into_iter()
        .map(|placement: Placement| placement.id.to_string())
        .collect())
}

#[tokio::test]
async fn only_ended_placements_with_work_left_are_finished() -> Result<()> {
    let (directory, db) = database().await?;
    let fleet = testdata::seed_fleet(&db).await?;
    let host = &fleet.hosts[0];
    let settled = Ended {
        state: "cleaned",
        runner: "deleted",
        started: true,
        absence_recorded: true,
        holds_capacity: false,
    };
    // In the order they are made; the newest first.
    let mut expected = Vec::new();
    let mut ignored = Vec::new();

    // Settled: the runner is retired and nothing is held or unrecorded.
    ignored.push(ended_placement(&db, &fleet, host, &settled).await?);
    ignored.push(
        ended_placement(
            &db,
            &fleet,
            host,
            &Ended {
                runner: "failed",
                ..settled
            },
        )
        .await?,
    );
    // A failed start whose runner is retired: abandonment never touches a
    // started placement, so there is nothing left, even with no absence
    // recorded.
    ignored.push(
        ended_placement(
            &db,
            &fleet,
            host,
            &Ended {
                state: "failed",
                runner: "failed",
                absence_recorded: false,
                ..settled
            },
        )
        .await?,
    );
    // An unstarted failure that abandonment already settled.
    ignored.push(
        ended_placement(
            &db,
            &fleet,
            host,
            &Ended {
                state: "failed",
                runner: "failed",
                started: false,
                ..settled
            },
        )
        .await?,
    );
    // Not ended: followed elsewhere in the pass.
    ignored.push(
        ended_placement(
            &db,
            &fleet,
            host,
            &Ended {
                state: "running",
                runner: "online",
                ..settled
            },
        )
        .await?,
    );

    // Its runner is still in service and must be retired.
    expected.push(
        ended_placement(
            &db,
            &fleet,
            host,
            &Ended {
                runner: "online",
                ..settled
            },
        )
        .await?,
    );
    // A failed start whose runner still waits: it is retried or retired.
    expected.push(
        ended_placement(
            &db,
            &fleet,
            host,
            &Ended {
                state: "failed",
                runner: "starting",
                ..settled
            },
        )
        .await?,
    );
    // Uncertain placements are cleaned up until they are cleaned.
    expected.push(
        ended_placement(
            &db,
            &fleet,
            host,
            &Ended {
                state: "uncertain",
                ..settled
            },
        )
        .await?,
    );
    // Unstarted failures that abandonment has not settled yet.
    let unrecorded = ended_placement(
        &db,
        &fleet,
        host,
        &Ended {
            state: "failed",
            runner: "failed",
            started: false,
            absence_recorded: false,
            holds_capacity: false,
        },
    )
    .await?;
    let holding = ended_placement(
        &db,
        &fleet,
        host,
        &Ended {
            state: "failed",
            runner: "failed",
            started: false,
            absence_recorded: true,
            holds_capacity: true,
        },
    )
    .await?;
    expected.push(unrecorded.clone());
    expected.push(holding.clone());

    // Oldest first.
    expected.reverse();
    let found = to_finish(&db).await?;
    assert_eq!(found, expected, "settled: {ignored:?}");

    // Finishing them once settles them: the next pass leaves them alone.
    for placement_id in [&unrecorded, &holding] {
        placement::abandon_unstarted(&db, placement_id.parse()?, &PlacementFailure::unknown())
            .await?;
    }
    expected.retain(|placement_id| ![&unrecorded, &holding].contains(&placement_id));
    assert_eq!(to_finish(&db).await?, expected);

    db.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
