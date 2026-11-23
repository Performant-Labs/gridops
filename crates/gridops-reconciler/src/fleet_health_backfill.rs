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

use std::collections::{BTreeMap, BTreeSet};

use sqlx::{Connection as _, QueryBuilder, Sqlite, SqliteConnection, SqlitePool};

use gridops_core::fleet_health::{
    HOUR_MILLIS, JobCategory, category::Classifier, classifier_cache::shared,
};

/// Rows per batch: one short transaction each.
const BATCH: i64 = 500;

/// How far back a rules change reclassifies completed jobs.
const RECLASSIFY_WINDOW_MS: i64 = 7 * 24 * HOUR_MILLIS;

/// The settings key holding the rules version the backfill last applied.
const RULES_VERSION_KEY: &str = "fleet_health.rules_version";

/// One reconciler pass of the backfill.
///
/// # Errors
/// A database read or write failed; `reconcile()` logs it and goes on.
pub(crate) async fn run_pass(db: &SqlitePool, now: i64) -> anyhow::Result<()> {
    let mut conn = db.acquire().await?;
    reclassify_on_rules_change(&mut conn, now).await?;
    backfill_unclassified(&mut conn, now).await?;
    Ok(())
}

/// Reclassifies the last [`RECLASSIFY_WINDOW_MS`] of completed jobs when the
/// rules version moved, listing the hours whose rollups the moves make
/// stale, and records the version it applied.
async fn reclassify_on_rules_change(conn: &mut SqliteConnection, now: i64) -> anyhow::Result<()> {
    let version = rules_version(conn).await?;
    let stored: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
        .bind(RULES_VERSION_KEY)
        .fetch_optional(&mut *conn)
        .await?;
    if stored.as_deref() == Some(version.as_str()) {
        return Ok(());
    }
    let classifier = shared().refresh(conn, now).await?;
    reclassify_github(conn, &classifier, now).await?;
    reclassify_gitlab(conn, &classifier, now).await?;
    sqlx::query(
        "INSERT INTO settings (key,value,updated_by,updated_at) VALUES (?,?,NULL,?)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",
    )
    .bind(RULES_VERSION_KEY)
    .bind(&version)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The rules' version, as the settings key stores it: `COUNT(*)` and
/// `MAX(updated_at)` of `job_category_rules`, the pair the classifier cache
/// compares.
async fn rules_version(conn: &mut SqliteConnection) -> anyhow::Result<String> {
    let (count, max_updated_at): (i64, Option<i64>) =
        sqlx::query_as("SELECT COUNT(*),MAX(updated_at) FROM job_category_rules")
            .fetch_one(&mut *conn)
            .await?;
    Ok(match max_updated_at {
        Some(updated_at) => format!("{count}:{updated_at}"),
        None => count.to_string(),
    })
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
    job_name: Option<String>,
    tag_list: String,
    project_path: Option<String>,
    category: Option<String>,
    completed_at: Option<i64>,
}

/// Classifies the completed workflow jobs of the last
/// [`RECLASSIFY_WINDOW_MS`] again, one batch per short transaction, and
/// lists the hours of the jobs the rules moved.
async fn reclassify_github(
    conn: &mut SqliteConnection,
    classifier: &Classifier,
    now: i64,
) -> anyhow::Result<()> {
    let batch = usize::try_from(BATCH)?;
    let mut after = (0_i64, 0_i64);
    loop {
        let page: Vec<WorkflowJobRow> = sqlx::query_as(
            "SELECT id,run_id,name,labels,category,completed_at FROM workflow_jobs
              WHERE category IS NOT NULL AND completed_at>=?
                AND (completed_at>? OR (completed_at=? AND id>?))
              ORDER BY completed_at,id LIMIT ?",
        )
        .bind(now - RECLASSIFY_WINDOW_MS)
        .bind(after.0)
        .bind(after.0)
        .bind(after.1)
        .bind(BATCH)
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

/// Classifies the completed GitLab jobs of the last
/// [`RECLASSIFY_WINDOW_MS`] again the same way.
async fn reclassify_gitlab(
    conn: &mut SqliteConnection,
    classifier: &Classifier,
    now: i64,
) -> anyhow::Result<()> {
    let batch = usize::try_from(BATCH)?;
    let mut after = (0_i64, String::new(), 0_i64);
    loop {
        let page: Vec<GitlabJobRow> = sqlx::query_as(
            "SELECT id,connection_id,job_id,job_name,tag_list,project_path,category,completed_at FROM gitlab_jobs
              WHERE category IS NOT NULL AND completed_at>=?
                AND (completed_at>? OR (completed_at=? AND (connection_id>? OR (connection_id=? AND job_id>?))))
              ORDER BY completed_at,connection_id,job_id LIMIT ?",
        )
        .bind(now - RECLASSIFY_WINDOW_MS)
        .bind(after.0)
        .bind(after.0)
        .bind(&after.1)
        .bind(&after.1)
        .bind(after.2)
        .bind(BATCH)
        .fetch_all(&mut *conn)
        .await?;
        let Some(last) = page.last() else {
            break;
        };
        let mut moved: Vec<(String, i64, JobCategory)> = Vec::new();
        let mut hours: BTreeSet<i64> = BTreeSet::new();
        for row in &page {
            let category = classifier.classify(
                row.job_name.as_deref().unwrap_or(""),
                None,
                row.project_path.as_deref(),
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
        if page.len() < batch {
            break;
        }
    }
    Ok(())
}

/// Classifies the rows whose `category` is still NULL, GitHub and GitLab, in
/// batches of one short transaction each, through the unclassified partial
/// indexes.
async fn backfill_unclassified(conn: &mut SqliteConnection, now: i64) -> anyhow::Result<()> {
    let classifier = shared().current(conn, now).await?;
    let batch = usize::try_from(BATCH)?;
    let mut after_id = 0_i64;
    loop {
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
    loop {
        let page: Vec<GitlabJobRow> = sqlx::query_as(
            "SELECT id,connection_id,job_id,job_name,tag_list,project_path,category,completed_at FROM gitlab_jobs
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
            let category = classifier.classify(
                row.job_name.as_deref().unwrap_or(""),
                None,
                row.project_path.as_deref(),
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
