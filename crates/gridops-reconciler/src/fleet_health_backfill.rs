//! The job category backfill (#190): classifies `workflow_jobs` and
//! `gitlab_jobs` rows whose `category` is still NULL, in bounded batches,
//! and, when the rules change, classifies the last 7 days again and lists
//! their hours in `fleet_health_recompute` for the rollups. The rules
//! version it last applied is kept as the `fleet_health.rules_version`
//! setting (no schema of its own).
//!
//! [`run_pass`] runs every reconciler pass, log-and-continue (`reconcile()`).
//! Reads happen outside the write transactions, bounded by the
//! `completed_at` indexes and the unclassified partial indexes; each batch's
//! writes are one short transaction.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use sqlx::{Connection as _, QueryBuilder, Sqlite, SqliteConnection, SqlitePool};

use gridops_core::{
    fleet::ExternalId,
    fleet_health::{
        HOUR_MILLIS, JobCategory,
        category::Classifier,
        classifier_cache::{CHECK_INTERVAL_MS, RulesVersion, gitlab_project_path, shared},
    },
};

/// Rows per batch: one short transaction each.
const BATCH: i64 = 500;

/// Batches of NULL rows one pass classifies at most, so the first pass after
/// the migration (every historical job NULL) doesn't run hundreds of write
/// transactions back to back; the rest is picked up by later passes. The same
/// bound as retention's (#152, #222).
const MAX_BATCHES: usize = 40;

/// The pause between two batches, so other writers get the lock in between.
const BATCH_PAUSE: std::time::Duration = std::time::Duration::from_millis(25);

/// How far back a rules change reclassifies completed jobs.
const RECLASSIFY_WINDOW_MS: i64 = 7 * 24 * HOUR_MILLIS;

/// The settings key holding the rules version the backfill last applied.
const RULES_VERSION_KEY: &str = "fleet_health.rules_version";

/// The settings key holding the rules version the settle pass last applied.
const RULES_SETTLED_KEY: &str = "fleet_health.rules_version_settled";

/// How long after applying a rules version the settle pass reclassifies
/// again: by then every process's writer cache (at most
/// [`CHECK_INTERVAL_MS`] behind) has the new rules, so the jobs written
/// with the old ones in between are moved too.
const SETTLE_MS: i64 = 2 * CHECK_INTERVAL_MS;

/// One reconciler pass of the backfill.
///
/// # Errors
/// A database read or write failed; `reconcile()` logs it and goes on.
pub(crate) async fn run_pass(db: &SqlitePool, now: i64) -> anyhow::Result<()> {
    let mut conn = db.acquire().await?;
    reclassify_on_rules_change(&mut conn, now).await?;
    backfill_unclassified(&mut conn, now, MAX_BATCHES).await?;
    Ok(())
}

/// Reclassifies the last [`RECLASSIFY_WINDOW_MS`] of completed jobs, and the
/// jobs still queued or running, when the rules version moved, listing the
/// hours whose rollups the moves make stale, and records the version it
/// applied. [`SETTLE_MS`] later it reclassifies once more (the settle pass):
/// the writers' caches may have stored old-rules categories until then.
async fn reclassify_on_rules_change(conn: &mut SqliteConnection, now: i64) -> anyhow::Result<()> {
    let key = RulesVersion::of(conn).await?.key();
    let applied = setting(conn, RULES_VERSION_KEY).await?;
    if applied.as_ref().map(|(value, _)| value.as_str()) != Some(key.as_str()) {
        let classifier = shared().refresh(conn, now).await?;
        reclassify_all(conn, &classifier, now).await?;
        store_setting(conn, RULES_VERSION_KEY, &key, now).await?;
        return Ok(());
    }
    let settled = setting(conn, RULES_SETTLED_KEY).await?;
    if settled.as_ref().map(|(value, _)| value.as_str()) == Some(key.as_str()) {
        return Ok(());
    }
    let applied_at = applied.map_or(now, |(_, at)| at);
    if now >= applied_at && now - applied_at < SETTLE_MS {
        return Ok(());
    }
    let classifier = shared().current(conn, now).await?;
    reclassify_all(conn, &classifier, now).await?;
    store_setting(conn, RULES_SETTLED_KEY, &key, now).await?;
    Ok(())
}

/// Reclassifies every job a rules change can move: GitHub and GitLab, the
/// completed ones of the window and the open ones.
async fn reclassify_all(
    conn: &mut SqliteConnection,
    classifier: &Classifier,
    now: i64,
) -> anyhow::Result<()> {
    for rows in [Rows::Completed, Rows::Open] {
        reclassify_github(conn, classifier, now, rows).await?;
        reclassify_gitlab(conn, classifier, now, rows).await?;
    }
    Ok(())
}

/// A setting's value and when it was written.
async fn setting(conn: &mut SqliteConnection, key: &str) -> anyhow::Result<Option<(String, i64)>> {
    Ok(
        sqlx::query_as("SELECT value,updated_at FROM settings WHERE key=?")
            .bind(key)
            .fetch_optional(&mut *conn)
            .await?,
    )
}

/// Writes a setting the backfill owns.
async fn store_setting(
    conn: &mut SqliteConnection,
    key: &str,
    value: &str,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO settings (key,value,updated_by,updated_at) VALUES (?,?,NULL,?)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Which classified jobs a reclassify reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rows {
    /// Completed within [`RECLASSIFY_WINDOW_MS`], through the `completed_at`
    /// indexes; their hours are listed for the rollups.
    Completed,
    /// Still queued or running (`completed_at` NULL): no rollup counts them
    /// yet, but they keep their category when they complete.
    Open,
}

/// GitLab project paths the backfill already resolved this pass.
type ProjectPaths = HashMap<(String, i64), Option<String>>;

/// The repository a GitLab job classifies with: its stored path, else the
/// same scope or sibling lookup the writers use
/// ([`gitlab_project_path`]), memoized for the pass.
async fn gitlab_repository(
    conn: &mut SqliteConnection,
    paths: &mut ProjectPaths,
    row: &GitlabJobRow,
) -> anyhow::Result<Option<String>> {
    if let Some(path) = &row.project_path {
        return Ok(Some(path.clone()));
    }
    let project = (row.connection_id.clone(), row.project_id);
    if let Some(path) = paths.get(&project) {
        return Ok(path.clone());
    }
    let found = match u64::try_from(row.project_id)
        .ok()
        .and_then(|id| ExternalId::new(id).ok())
    {
        Some(id) => gitlab_project_path(conn, &row.connection_id, id).await?,
        None => None,
    };
    paths.insert(project, found.clone());
    Ok(found)
}

/// One workflow job the backfill looks at: what a rule matches, and what a
/// reclassify also needs (the stored category and the hour it completed in).
#[derive(Debug, sqlx::FromRow)]
struct WorkflowJobRow {
    id: i64,
    run_id: i64,
    name: String,
    labels: String,
    category: Option<String>,
    completed_at: Option<i64>,
}

/// One GitLab job the backfill looks at, on the same terms.
#[derive(Debug, sqlx::FromRow)]
struct GitlabJobRow {
    id: String,
    connection_id: String,
    job_id: i64,
    project_id: i64,
    job_name: Option<String>,
    tag_list: String,
    project_path: Option<String>,
    category: Option<String>,
    completed_at: Option<i64>,
}

/// Classifies the workflow jobs `rows` names again, one batch per short
/// transaction, and lists the hours of the completed jobs the rules moved.
async fn reclassify_github(
    conn: &mut SqliteConnection,
    classifier: &Classifier,
    now: i64,
    rows: Rows,
) -> anyhow::Result<()> {
    let batch = usize::try_from(BATCH)?;
    let mut after = (0_i64, 0_i64);
    let mut first = true;
    loop {
        if !first {
            tokio::time::sleep(BATCH_PAUSE).await;
        }
        first = false;
        let page: Vec<WorkflowJobRow> = match rows {
            Rows::Completed => sqlx::query_as(
                "SELECT id,run_id,name,labels,category,completed_at FROM workflow_jobs
                  WHERE category IS NOT NULL AND completed_at>=?
                    AND (completed_at>? OR (completed_at=? AND id>?))
                  ORDER BY completed_at,id LIMIT ?",
            )
            .bind(now - RECLASSIFY_WINDOW_MS)
            .bind(after.0)
            .bind(after.0)
            .bind(after.1)
            .bind(BATCH),
            Rows::Open => sqlx::query_as(
                "SELECT id,run_id,name,labels,category,completed_at FROM workflow_jobs
                  WHERE category IS NOT NULL AND completed_at IS NULL AND id>?
                  ORDER BY id LIMIT ?",
            )
            .bind(after.1)
            .bind(BATCH),
        }
        .fetch_all(&mut *conn)
        .await?;
        let Some(last) = page.last() else {
            break;
        };
        let runs = run_contexts(conn, page.iter().map(|row| row.run_id)).await?;
        let mut moved: Vec<(i64, JobCategory)> = Vec::new();
        let mut hours: BTreeSet<i64> = BTreeSet::new();
        for row in &page {
            let (workflow_name, repository) = runs.get(&row.run_id).cloned().unwrap_or_default();
            let category = classifier.classify(
                &row.name,
                Some(&workflow_name),
                repository.as_deref(),
                &labels_of(&row.labels),
            );
            if category.as_str() != row.category.as_deref().unwrap_or_default() {
                moved.push((row.id, category));
                if let Some(completed_at) = row.completed_at {
                    hours.insert(hour_of(completed_at));
                }
            }
        }
        apply_workflow(conn, &moved, &hours, now, Scope::Classified).await?;
        after = (last.completed_at.unwrap_or(0), last.id);
        if page.len() < batch {
            break;
        }
    }
    Ok(())
}

/// Classifies the GitLab jobs `rows` names again the same way.
async fn reclassify_gitlab(
    conn: &mut SqliteConnection,
    classifier: &Classifier,
    now: i64,
    rows: Rows,
) -> anyhow::Result<()> {
    let batch = usize::try_from(BATCH)?;
    let mut paths = ProjectPaths::new();
    let mut after = (0_i64, String::new(), 0_i64);
    let mut after_id = String::new();
    let mut first = true;
    loop {
        if !first {
            tokio::time::sleep(BATCH_PAUSE).await;
        }
        first = false;
        let page: Vec<GitlabJobRow> = match rows {
            Rows::Completed => sqlx::query_as(
                "SELECT id,connection_id,job_id,project_id,job_name,tag_list,project_path,category,completed_at FROM gitlab_jobs
                  WHERE category IS NOT NULL AND completed_at>=?
                    AND (completed_at>? OR (completed_at=? AND (connection_id>? OR (connection_id=? AND job_id>?))))
                  ORDER BY completed_at,connection_id,job_id LIMIT ?",
            )
            .bind(now - RECLASSIFY_WINDOW_MS)
            .bind(after.0)
            .bind(after.0)
            .bind(after.1.clone())
            .bind(after.1.clone())
            .bind(after.2)
            .bind(BATCH),
            Rows::Open => sqlx::query_as(
                "SELECT id,connection_id,job_id,project_id,job_name,tag_list,project_path,category,completed_at FROM gitlab_jobs
                  WHERE category IS NOT NULL AND completed_at IS NULL AND id>?
                  ORDER BY id LIMIT ?",
            )
            .bind(after_id.clone())
            .bind(BATCH),
        }
        .fetch_all(&mut *conn)
        .await?;
        let Some(last) = page.last() else {
            break;
        };
        let mut moved: Vec<(String, i64, JobCategory)> = Vec::new();
        let mut hours: BTreeSet<i64> = BTreeSet::new();
        for row in &page {
            let repository = gitlab_repository(conn, &mut paths, row).await?;
            let category = classifier.classify(
                row.job_name.as_deref().unwrap_or(""),
                None,
                repository.as_deref(),
                &labels_of(&row.tag_list),
            );
            if category.as_str() != row.category.as_deref().unwrap_or_default() {
                moved.push((row.connection_id.clone(), row.job_id, category));
                if let Some(completed_at) = row.completed_at {
                    hours.insert(hour_of(completed_at));
                }
            }
        }
        apply_gitlab(conn, &moved, &hours, now, Scope::Classified).await?;
        after = (
            last.completed_at.unwrap_or(0),
            last.connection_id.clone(),
            last.job_id,
        );
        after_id.clone_from(&last.id);
        if page.len() < batch {
            break;
        }
    }
    Ok(())
}

/// Classifies the rows whose `category` is still NULL, GitHub then GitLab,
/// through the unclassified partial indexes, one short transaction per
/// batch, in at most `max_batches` batches shared across both tables (the
/// pass passes [`MAX_BATCHES`]); the next pass picks up the rest.
async fn backfill_unclassified(
    conn: &mut SqliteConnection,
    now: i64,
    max_batches: usize,
) -> anyhow::Result<()> {
    let classifier = shared().current(conn, now).await?;
    let batch = usize::try_from(BATCH)?;
    let mut batches = 0_usize;
    let mut after_id = 0_i64;
    loop {
        if batches >= max_batches {
            return Ok(());
        }
        if batches > 0 {
            tokio::time::sleep(BATCH_PAUSE).await;
        }
        batches += 1;
        let page: Vec<WorkflowJobRow> = sqlx::query_as(
            "SELECT id,run_id,name,labels,category,completed_at FROM workflow_jobs
              WHERE category IS NULL AND id>? ORDER BY id LIMIT ?",
        )
        .bind(after_id)
        .bind(BATCH)
        .fetch_all(&mut *conn)
        .await?;
        let Some(last) = page.last() else {
            break;
        };
        let runs = run_contexts(conn, page.iter().map(|row| row.run_id)).await?;
        let mut classified: Vec<(i64, JobCategory)> = Vec::new();
        for row in &page {
            let (workflow_name, repository) = runs.get(&row.run_id).cloned().unwrap_or_default();
            let category = classifier.classify(
                &row.name,
                Some(&workflow_name),
                repository.as_deref(),
                &labels_of(&row.labels),
            );
            classified.push((row.id, category));
        }
        apply_workflow(conn, &classified, &BTreeSet::new(), now, Scope::Null).await?;
        after_id = last.id;
        if page.len() < batch {
            break;
        }
    }
    let mut after_id = String::new();
    let mut paths = ProjectPaths::new();
    loop {
        if batches >= max_batches {
            return Ok(());
        }
        if batches > 0 {
            tokio::time::sleep(BATCH_PAUSE).await;
        }
        batches += 1;
        let page: Vec<GitlabJobRow> = sqlx::query_as(
            "SELECT id,connection_id,job_id,project_id,job_name,tag_list,project_path,category,completed_at FROM gitlab_jobs
              WHERE category IS NULL AND id>? ORDER BY id LIMIT ?",
        )
        .bind(&after_id)
        .bind(BATCH)
        .fetch_all(&mut *conn)
        .await?;
        let Some(last) = page.last() else {
            break;
        };
        let mut classified: Vec<(String, i64, JobCategory)> = Vec::new();
        for row in &page {
            let repository = gitlab_repository(conn, &mut paths, row).await?;
            let category = classifier.classify(
                row.job_name.as_deref().unwrap_or(""),
                None,
                repository.as_deref(),
                &labels_of(&row.tag_list),
            );
            classified.push((row.connection_id.clone(), row.job_id, category));
        }
        apply_gitlab(conn, &classified, &BTreeSet::new(), now, Scope::Null).await?;
        after_id = last.id.clone();
        if page.len() < batch {
            break;
        }
    }
    Ok(())
}

/// Which rows a batch of category updates may touch: the backfill's
/// unclassified ones, or a reclassify's already-classified ones. The guard
/// keeps a batch to the rows its read saw.
#[derive(Debug, Clone, Copy)]
enum Scope {
    /// `category IS NULL`: the backfill.
    Null,
    /// `category IS NOT NULL`: a reclassify.
    Classified,
}

impl Scope {
    const fn condition(self) -> &'static str {
        match self {
            Self::Null => " IS NULL",
            Self::Classified => " IS NOT NULL",
        }
    }
}

/// Writes one batch's GitHub categories, and the hours a reclassify's moves
/// make stale, in one short transaction.
async fn apply_workflow(
    conn: &mut SqliteConnection,
    updates: &[(i64, JobCategory)],
    hours: &BTreeSet<i64>,
    now: i64,
    scope: Scope,
) -> anyhow::Result<()> {
    if updates.is_empty() {
        return Ok(());
    }
    let mut transaction = conn.begin().await?;
    let mut query =
        QueryBuilder::<Sqlite>::new("UPDATE workflow_jobs SET category=v.column2 FROM (");
    query.push_values(updates.iter(), |mut row, (id, category)| {
        row.push_bind(*id).push_bind(category.as_str());
    });
    query.push(") AS v WHERE workflow_jobs.id=v.column1 AND workflow_jobs.category");
    query.push(scope.condition());
    query.build().execute(&mut *transaction).await?;
    list_hours(&mut transaction, hours, now).await?;
    transaction.commit().await?;
    Ok(())
}

/// Writes one batch's GitLab categories, and the hours a reclassify's moves
/// make stale, in one short transaction.
async fn apply_gitlab(
    conn: &mut SqliteConnection,
    updates: &[(String, i64, JobCategory)],
    hours: &BTreeSet<i64>,
    now: i64,
    scope: Scope,
) -> anyhow::Result<()> {
    if updates.is_empty() {
        return Ok(());
    }
    let mut transaction = conn.begin().await?;
    let mut query = QueryBuilder::<Sqlite>::new("UPDATE gitlab_jobs SET category=v.column3 FROM (");
    query.push_values(updates.iter(), |mut row, (connection, job, category)| {
        row.push_bind(connection)
            .push_bind(*job)
            .push_bind(category.as_str());
    });
    query.push(
        ") AS v WHERE gitlab_jobs.connection_id=v.column1 AND gitlab_jobs.job_id=v.column2
             AND gitlab_jobs.category",
    );
    query.push(scope.condition());
    query.build().execute(&mut *transaction).await?;
    list_hours(&mut transaction, hours, now).await?;
    transaction.commit().await?;
    Ok(())
}

/// Lists `hours` in `fleet_health_recompute` for the rollups to take.
async fn list_hours(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    hours: &BTreeSet<i64>,
    now: i64,
) -> anyhow::Result<()> {
    if hours.is_empty() {
        return Ok(());
    }
    let mut query =
        QueryBuilder::<Sqlite>::new("INSERT INTO fleet_health_recompute (hour,requested_at) ");
    query.push_values(hours.iter(), |mut row, hour| {
        row.push_bind(*hour).push_bind(now);
    });
    query.push(" ON CONFLICT(hour) DO NOTHING");
    query.build().execute(&mut **transaction).await?;
    Ok(())
}

/// The workflow name and repository of each of `run_ids`' runs, for the
/// rules that match on them.
async fn run_contexts(
    conn: &mut SqliteConnection,
    run_ids: impl Iterator<Item = i64>,
) -> anyhow::Result<BTreeMap<i64, (String, Option<String>)>> {
    let ids: BTreeSet<i64> = run_ids.collect();
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT runs.id,runs.workflow_name,repositories.full_name
           FROM workflow_runs runs
           LEFT JOIN repositories ON repositories.id=runs.repository_id
          WHERE runs.id IN (",
    );
    let mut separated = query.separated(", ");
    for id in &ids {
        separated.push_bind(*id);
    }
    query.push(")");
    let rows: Vec<(i64, String, Option<String>)> =
        query.build_query_as().fetch_all(&mut *conn).await?;
    Ok(rows
        .into_iter()
        .map(|(id, workflow_name, repository)| (id, (workflow_name, repository)))
        .collect())
}

/// The JSON array of labels (or tags) a job row stores.
fn labels_of(stored: &str) -> Vec<String> {
    serde_json::from_str(stored).unwrap_or_default()
}

/// The hour a completion time falls in.
fn hour_of(completed_at: i64) -> i64 {
    completed_at - completed_at.rem_euclid(HOUR_MILLIS)
}

#[cfg(test)]
mod tests {
    use super::{BATCH, SETTLE_MS, backfill_unclassified, run_pass};
    use gridops_core::{
        connect_database_path,
        fleet_health::testdata::{self, JobSeed, SeededFleet},
        now_millis,
    };
    use sqlx::SqlitePool;

    /// The test's batch budget: seeding the production budget's 20,000 rows
    /// takes many minutes in a debug build.
    const MAX: usize = 2;
    const MINUTE: i64 = 60_000;

    async fn database(name: &str) -> anyhow::Result<(std::path::PathBuf, SqlitePool, SeededFleet)> {
        let directory =
            std::env::temp_dir().join(format!("gridops-{name}-{}", uuid::Uuid::new_v4()));
        let db = connect_database_path(&directory.join("gridops.sqlite")).await?;
        let fleet = testdata::seed_fleet(&db).await?;
        Ok((directory, db, fleet))
    }

    async fn add_rule(
        db: &SqlitePool,
        position: i64,
        category: &str,
        field: &str,
        pattern: &str,
        now: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO job_category_rules (position,category,field,pattern,created_at,updated_at)
             VALUES (?,?,?,?,?,?)",
        )
        .bind(position)
        .bind(category)
        .bind(field)
        .bind(pattern)
        .bind(now)
        .bind(now)
        .execute(db)
        .await?;
        Ok(())
    }

    async fn workflow_category(db: &SqlitePool, id: i64) -> anyhow::Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT category FROM workflow_jobs WHERE id=?")
                .bind(id)
                .fetch_one(db)
                .await?,
        )
    }

    async fn gitlab_category(db: &SqlitePool, job_id: i64) -> anyhow::Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT category FROM gitlab_jobs WHERE job_id=?")
                .bind(job_id)
                .fetch_one(db)
                .await?,
        )
    }

    async fn finish(directory: std::path::PathBuf, db: SqlitePool) {
        db.close().await;
        let _ = std::fs::remove_dir_all(directory);
    }

    /// One pass classifies at most its batch budget of NULL rows, so the
    /// first pass after the migration can't run the whole history's
    /// transactions back to back; the next pass picks up the rest.
    #[tokio::test]
    async fn one_pass_classifies_at_most_its_batch_budget() -> anyhow::Result<()> {
        let _serial = testdata::CLASSIFIER_TEST_LOCK.lock().await;
        let (directory, db, fleet) = database("backfill-cap").await?;
        let io = fleet.host("io")?.id.clone();
        let now = now_millis();
        let budget = i64::try_from(MAX)? * BATCH;
        let total = budget + 300;
        let jobs: Vec<JobSeed> = (0..total)
            .map(|id| {
                let completed = now - 3_600_000 - id * 1_000;
                let mut job = JobSeed::completed(
                    id + 1,
                    "unit tests",
                    Some(&io),
                    completed - 120_000,
                    completed - 60_000,
                    completed,
                );
                job.category = None;
                job
            })
            .collect();
        testdata::insert_jobs(&db, &fleet, &jobs).await?;
        let unclassified = || async {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM workflow_jobs WHERE category IS NULL",
            )
            .fetch_one(&db)
            .await
        };
        let mut conn = db.acquire().await?;
        backfill_unclassified(&mut conn, now, MAX).await?;
        assert_eq!(
            unclassified().await?,
            total - budget,
            "one pass stops at its budget"
        );
        backfill_unclassified(&mut conn, now, MAX).await?;
        assert_eq!(unclassified().await?, 0, "the next pass finishes the rest");
        drop(conn);
        finish(directory, db).await;
        Ok(())
    }

    /// A rules change moves the jobs still queued or running too, not only
    /// the completed ones: they keep their category when they complete.
    #[tokio::test]
    async fn a_rule_change_moves_queued_and_running_jobs() -> anyhow::Result<()> {
        let _serial = testdata::CLASSIFIER_TEST_LOCK.lock().await;
        let (directory, db, fleet) = database("backfill-open").await?;
        let io = fleet.host("io")?.id.clone();
        let now = now_millis();
        testdata::insert_jobs(
            &db,
            &fleet,
            &[
                JobSeed::queued(1, "notify slack", now - MINUTE),
                JobSeed::running(2, "notify slack", Some(&io), now - 2 * MINUTE, now - MINUTE),
            ],
        )
        .await?;
        run_pass(&db, now).await?;
        assert_eq!(workflow_category(&db, 1).await?.as_deref(), Some("other"));
        add_rule(&db, 4, "deploy", "job_name", "notify", now).await?;
        run_pass(&db, now + 2 * MINUTE).await?;
        assert_eq!(
            workflow_category(&db, 1).await?.as_deref(),
            Some("deploy"),
            "queued"
        );
        assert_eq!(
            workflow_category(&db, 2).await?.as_deref(),
            Some("deploy"),
            "running"
        );
        finish(directory, db).await;
        Ok(())
    }

    /// A writer whose cache still had the old rules (up to a minute) stores
    /// an old category after the reclassify ran; the settle pass moves it,
    /// once, [`SETTLE_MS`] after the version was applied.
    #[tokio::test]
    async fn the_settle_pass_moves_jobs_written_with_the_old_rules() -> anyhow::Result<()> {
        let _serial = testdata::CLASSIFIER_TEST_LOCK.lock().await;
        let (directory, db, fleet) = database("backfill-settle").await?;
        let io = fleet.host("io")?.id.clone();
        let now = now_millis();
        run_pass(&db, now).await?;
        add_rule(&db, 4, "deploy", "job_name", "notify", now).await?;
        let applied = now + 2 * MINUTE;
        run_pass(&db, applied).await?;
        // A stale writer: completed just now, stored under the old rules.
        let mut stale = JobSeed::completed(
            1,
            "notify slack",
            Some(&io),
            applied - 3 * MINUTE,
            applied - 2 * MINUTE,
            applied,
        );
        stale.category = Some(gridops_core::fleet_health::JobCategory::Other);
        testdata::insert_jobs(&db, &fleet, &[stale]).await?;
        run_pass(&db, applied + SETTLE_MS - 1).await?;
        assert_eq!(
            workflow_category(&db, 1).await?.as_deref(),
            Some("other"),
            "not before the writers' caches have all caught up"
        );
        run_pass(&db, applied + SETTLE_MS).await?;
        assert_eq!(workflow_category(&db, 1).await?.as_deref(), Some("deploy"));
        finish(directory, db).await;
        Ok(())
    }

    /// A GitLab row stored without a project path classifies with the path
    /// the writers would have used (here the connection's project scope), in
    /// the backfill and in a reclassify alike.
    #[tokio::test]
    async fn a_pathless_gitlab_job_classifies_with_its_project_path() -> anyhow::Result<()> {
        let _serial = testdata::CLASSIFIER_TEST_LOCK.lock().await;
        let (directory, db, fleet) = database("backfill-gitlab-path").await?;
        let io = fleet.host("io")?.id.clone();
        let now = now_millis();
        add_rule(&db, 4, "deploy", "repository", "^acme/app$", now).await?;
        let mut job = JobSeed::completed(
            9,
            "notify slack",
            Some(&io),
            now - 3 * MINUTE,
            now - 2 * MINUTE,
            now - MINUTE,
        );
        job.gitlab = true;
        job.category = None;
        testdata::insert_jobs(&db, &fleet, &[job]).await?;
        sqlx::query("UPDATE gitlab_jobs SET project_path=NULL")
            .execute(&db)
            .await?;
        run_pass(&db, now).await?;
        assert_eq!(
            gitlab_category(&db, 9).await?.as_deref(),
            Some("deploy"),
            "the backfill"
        );

        // A stored category the rules no longer give, then an unrelated rule
        // change: the reclassify must resolve the path the same way.
        sqlx::query("UPDATE gitlab_jobs SET category='other'")
            .execute(&db)
            .await?;
        add_rule(&db, 5, "build", "job_name", "^never$", now).await?;
        run_pass(&db, now + 2 * MINUTE).await?;
        assert_eq!(
            gitlab_category(&db, 9).await?.as_deref(),
            Some("deploy"),
            "the reclassify"
        );
        finish(directory, db).await;
        Ok(())
    }
}
