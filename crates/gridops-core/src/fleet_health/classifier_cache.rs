//! One compiled job classifier per process, and the seam every job writer
//! binds `category` through (#190 fills both in).
//!
//! The writers (the GitHub webhook, the GitHub poll and the GitLab job
//! writer) call [`category_for_insert`] only when they insert a job row, and
//! bind its result in the INSERT's `VALUES` only: an `ON CONFLICT` update
//! never touches `category`, so a stored category is never changed by a later
//! write (W-15). #190's backfill classifies what is still NULL.
//!
//! #197 delivers the seam: [`category_for_insert`] returns `None` (the job
//! stays unclassified) and [`ClassifierCache::current`] compiles the rules on
//! every call. #190 makes the cache rebuild only when the rules change
//! (`MAX(updated_at)` or `COUNT(*)`), checked at most once a minute.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use sqlx::SqliteConnection;

use crate::fleet::ExternalId;

use super::{
    JobCategory,
    category::{self, Classifier},
};

/// A process's compiled classifier.
#[derive(Debug, Default)]
pub struct ClassifierCache {
    compilations: AtomicUsize,
}

impl ClassifierCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The classifier for the stored rules as of `now`.
    ///
    /// # Errors
    /// Reading the rules fails.
    pub async fn current(
        &self,
        conn: &mut SqliteConnection,
        now: i64,
    ) -> anyhow::Result<Arc<Classifier>> {
        let _ = now;
        let rules = category::load_rules(conn).await?;
        self.compilations.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(category::compile(&rules)))
    }

    /// How many times this cache has compiled the rules.
    #[must_use]
    pub fn compilations(&self) -> usize {
        self.compilations.load(Ordering::Relaxed)
    }
}

/// Where a job being inserted came from, for the fields its own row lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobOrigin<'a> {
    /// A GitHub job of this run: the workflow name and repository come from
    /// `workflow_runs`.
    GitHubRun(i64),
    /// A GitLab job of this connection's project. `path` is the project's
    /// path when the writer has it (a webhook) and `None` when it does not
    /// (the poll), in which case the ids let #190 find it, for example from
    /// another of the project's `gitlab_jobs` rows.
    GitLabProject {
        connection_id: &'a str,
        project_id: ExternalId,
        path: Option<&'a str>,
    },
}

/// A job a writer is about to insert.
#[derive(Debug, Clone, Copy)]
pub struct NewJob<'a> {
    pub job_name: Option<&'a str>,
    pub labels: &'a [String],
    pub origin: JobOrigin<'a>,
}

/// The category to bind when inserting `job`, or `None` to leave it for the
/// backfill. Never fails a write: an error reading the rules or the run is
/// logged and gives `None`.
#[allow(
    clippy::unused_async,
    reason = "#190 reads the rules and the run here; the writers already await it"
)]
pub async fn category_for_insert(
    conn: &mut SqliteConnection,
    job: &NewJob<'_>,
    now: i64,
) -> Option<JobCategory> {
    let _ = (conn, job, now);
    None
}
