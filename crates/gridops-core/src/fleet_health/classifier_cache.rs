//! One compiled job classifier per process, and the seam every job writer
//! binds `category` through (#190).
//!
//! The writers (the GitHub webhook, the GitHub poll and the GitLab job
//! writer) call [`category_for_insert`] only when they insert a job row, and
//! bind its result in the INSERT's `VALUES` only: an `ON CONFLICT` update
//! never touches `category`, so a stored category is never changed by a later
//! write (W-15). #190's backfill classifies what is still NULL.
//!
//! [`category_for_insert`] classifies through the process-wide cache
//! ([`ClassifierCache::shared`]), which compiles the rules on its first call
//! and rebuilds only when they change (`MAX(updated_at)` or `COUNT(*)` of
//! `job_category_rules`), checked at most once a minute — and again whenever
//! `now` is earlier than the last check, so a caller whose clock moved back
//! still sees the rules it wrote.
//!
//! [`category_for_insert`] may run while its caller holds the write lock: the
//! GitLab writer classifies inside the webhook's `BEGIN IMMEDIATE` and the
//! GitLab poll's one transaction per page of jobs, and the GitHub poll
//! (`upsert_workflow_job`) inside its page's transaction. Only the GitHub
//! webhook classifies outside a transaction. So the rebuild must stay cheap
//! even for a large rule list (#199 allows many rules): the writers' path
//! rebuilds through [`compile_chunked`], which compiles position-ordered
//! chunks of rules on at most one thread per core, keeping the caller's write
//! transaction under the speed check's 50 ms
//! (`classifying_inside_a_gitlab_write_transaction_holds_the_lock_briefly`,
//! a #190 contract test). [`ClassifierCache::current`] serves the one
//! `Arc<Classifier>` the rest of the process wants (the backfill), rebuilt
//! the same way and on the same clock.

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
    thread,
};

use anyhow::Context as _;
use sqlx::SqliteConnection;
use tokio::sync::Mutex;

use crate::fleet::ExternalId;

use super::{
    JobCategory,
    category::{self, CategoryRule, Classifier},
};

/// How long the cache serves one compiled classifier without looking at the
/// rules again.
const CHECK_INTERVAL_MS: i64 = 60_000;

/// Rules per chunk of a parallel rebuild, before the core count caps how
/// many chunks there are: enough chunks to keep every core busy, few enough
/// that thread start-up stays noise.
const CHUNK_RULES: usize = 16;

/// What the rules were when the cache compiled them: `COUNT(*)` and
/// `MAX(updated_at)` of `job_category_rules`. Either one moving is a change.
/// The backfill stores the same pair as its `fleet_health.rules_version`
/// setting, through [`RulesVersion::key`], so the reclassify trigger and the
/// cache's rebuild see one version.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub struct RulesVersion {
    count: i64,
    max_updated_at: Option<i64>,
}

impl RulesVersion {
    /// The stored rules' version on this connection.
    ///
    /// # Errors
    /// The read fails.
    pub async fn of(conn: &mut SqliteConnection) -> anyhow::Result<Self> {
        let (count, max_updated_at): (i64, Option<i64>) =
            sqlx::query_as("SELECT COUNT(*),MAX(updated_at) FROM job_category_rules")
                .fetch_one(&mut *conn)
                .await?;
        Ok(Self {
            count,
            max_updated_at,
        })
    }

    /// The settings key form of this version: two different rule sets give
    /// two different keys.
    #[must_use]
    pub fn key(self) -> String {
        match self.max_updated_at {
            Some(updated_at) => format!("{}:{updated_at}", self.count),
            None => self.count.to_string(),
        }
    }
}

/// The compiled classifier, and when the cache last looked at the rules.
#[derive(Debug)]
struct Cached {
    classifier: Arc<Classifier>,
    version: RulesVersion,
    last_check: i64,
}

/// A process's compiled classifier.
#[derive(Debug, Default)]
pub struct ClassifierCache {
    /// The writers' classifier, rebuilt in parallel chunks.
    chunked: Mutex<Option<Arc<Chunked>>>,
    /// The one `Arc<Classifier>` for callers that want it whole.
    cached: Mutex<Option<Cached>>,
    compilations: AtomicUsize,
}

impl ClassifierCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The classifier for the stored rules as of `now`. The rules are looked
    /// at at most once a minute, and again whenever `now` is earlier than
    /// the last look; the classifier is recompiled only when its version
    /// moved.
    ///
    /// # Errors
    /// Reading the rules fails.
    pub async fn current(
        &self,
        conn: &mut SqliteConnection,
        now: i64,
    ) -> anyhow::Result<Arc<Classifier>> {
        let mut cached = self.cached.lock().await;
        let due = cached.as_ref().is_none_or(|cached| {
            now < cached.last_check || now - cached.last_check >= CHECK_INTERVAL_MS
        });
        if !due && let Some(cached) = cached.as_ref() {
            return Ok(Arc::clone(&cached.classifier));
        }
        self.reload(&mut cached, conn, now).await
    }

    /// The classifier for the stored rules whatever the clock says: the
    /// backfill calls this once it has seen the rules version move, so it
    /// reclassifies with the rules that changed and not the cached minute.
    ///
    /// # Errors
    /// Reading the rules fails.
    pub async fn refresh(
        &self,
        conn: &mut SqliteConnection,
        now: i64,
    ) -> anyhow::Result<Arc<Classifier>> {
        let mut cached = self.cached.lock().await;
        self.reload(&mut cached, conn, now).await
    }

    /// Reads the rules' version and compiles the rules again when it moved.
    /// The cache lock is held, so one recompile serves every caller waiting
    /// on it.
    async fn reload(
        &self,
        cached: &mut Option<Cached>,
        conn: &mut SqliteConnection,
        now: i64,
    ) -> anyhow::Result<Arc<Classifier>> {
        let version = RulesVersion::of(&mut *conn).await?;
        if let Some(cached) = cached.as_mut()
            && cached.version == version
        {
            cached.last_check = now;
            return Ok(Arc::clone(&cached.classifier));
        }
        let rules = category::load_rules(&mut *conn).await?;
        self.compilations.fetch_add(1, Ordering::Relaxed);
        let classifier = Arc::new(category::compile(&rules));
        *cached = Some(Cached {
            classifier: Arc::clone(&classifier),
            version,
            last_check: now,
        });
        Ok(classifier)
    }

    /// The writers' classifier for the stored rules as of `now`, on the same
    /// clock as [`Self::current`] but rebuilt as parallel chunks, so the
    /// first call after a rule change does not hold its caller's write
    /// transaction for a serial compile of every pattern.
    ///
    /// # Errors
    /// Reading the rules fails.
    async fn chunks_for(
        &self,
        conn: &mut SqliteConnection,
        now: i64,
    ) -> anyhow::Result<Arc<Chunked>> {
        let mut chunked = self.chunked.lock().await;
        let due = chunked.as_ref().is_none_or(|chunked| {
            let last_check = chunked.last_check.load(Ordering::Relaxed);
            now < last_check || now - last_check >= CHECK_INTERVAL_MS
        });
        if !due && let Some(chunked) = chunked.as_ref() {
            return Ok(Arc::clone(chunked));
        }
        let version = RulesVersion::of(&mut *conn).await?;
        if let Some(chunked) = chunked.as_ref()
            && chunked.version == version
        {
            chunked.last_check.store(now, Ordering::Relaxed);
            return Ok(Arc::clone(chunked));
        }
        let rules = category::load_rules(&mut *conn).await?;
        self.compilations.fetch_add(1, Ordering::Relaxed);
        let built = Arc::new(Chunked {
            classifier: compile_chunked(&rules),
            projects: Mutex::new(HashMap::new()),
            version,
            last_check: AtomicI64::new(now),
        });
        *chunked = Some(Arc::clone(&built));
        Ok(built)
    }

    /// How many times this cache has compiled the rules.
    #[must_use]
    pub fn compilations(&self) -> usize {
        self.compilations.load(Ordering::Relaxed)
    }
}

/// The process's cache: the one classifier every job writer and the backfill
/// share.
#[must_use]
pub fn shared() -> &'static ClassifierCache {
    static SHARED: OnceLock<ClassifierCache> = OnceLock::new();
    SHARED.get_or_init(ClassifierCache::new)
}

/// The writers' classifier: the rules in position order, compiled chunk by
/// chunk on at most one thread per core.
///
/// `Classifier::classify` answers [`JobCategory::Other`] both when no rule
/// matched and when the first rule that matched is an `other` rule, so a
/// chunk cannot tell the two apart on its own. [`compile_chunked`] therefore
/// compiles every `other` rule as [`JobCategory::FixAgent`], a category no
/// rule may store: a chunk that answers `fix-agent` matched an `other` rule,
/// and a chunk that answers `other` matched nothing, so the next chunk is
/// asked. The answer is the one `Classifier::classify` on the whole rule
/// list would have given.
#[derive(Debug, Clone, Default)]
struct ChunkedClassifier {
    chunks: Vec<Arc<Classifier>>,
}

impl ChunkedClassifier {
    fn classify(
        &self,
        job_name: &str,
        workflow_name: Option<&str>,
        repository: Option<&str>,
        labels: &[String],
    ) -> JobCategory {
        for chunk in &self.chunks {
            match chunk.classify(job_name, workflow_name, repository, labels) {
                // No rule in this chunk matched; the next chunk is asked.
                JobCategory::Other => {}
                JobCategory::FixAgent => return JobCategory::Other,
                category => return category,
            }
        }
        JobCategory::Other
    }
}

/// The writers' cached classifier, and when the cache last looked at the
/// rules. `projects` remembers the GitLab project paths the poll had to
/// look up, one per project instead of one per job; a rebuild clears it.
/// `last_check` is atomic so a check can move it through the shared
/// `Arc<Chunked>` the writers hold.
#[derive(Debug)]
struct Chunked {
    classifier: ChunkedClassifier,
    projects: Mutex<HashMap<(String, i64), Option<String>>>,
    version: RulesVersion,
    last_check: AtomicI64,
}

/// Compiles `rules` as position-ordered chunks on at most one thread per
/// core (however many rules there are). A chunk whose thread panics is left
/// empty, like a rule whose pattern does not compile: the rest still
/// classify.
fn compile_chunked(rules: &[CategoryRule]) -> ChunkedClassifier {
    let cores = thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let chunk_count = rules.len().div_ceil(CHUNK_RULES).min(cores).max(1);
    compile_in_chunks(rules, chunk_count)
}

/// The chunk compiler behind [`compile_chunked`]: sorts `rules` by position,
/// rewrites every `other` rule as [`JobCategory::FixAgent`] (the sentinel
/// [`ChunkedClassifier::classify`] maps back), and splits the list into
/// `chunk_count` even chunks, one thread each (none when one chunk is asked
/// for).
fn compile_in_chunks(rules: &[CategoryRule], chunk_count: usize) -> ChunkedClassifier {
    let mut ordered: Vec<CategoryRule> = rules.to_vec();
    ordered.sort_by_key(|rule| rule.position);
    for rule in &mut ordered {
        if rule.category == JobCategory::Other {
            rule.category = JobCategory::FixAgent;
        }
    }
    let chunk_count = chunk_count.max(1);
    let chunk_len = ordered.len().div_ceil(chunk_count).max(1);
    let chunks: Vec<&[CategoryRule]> = ordered.chunks(chunk_len).collect();
    if chunks.len() <= 1 {
        return ChunkedClassifier {
            chunks: vec![Arc::new(category::compile(
                chunks.first().copied().unwrap_or_default(),
            ))],
        };
    }
    let compiled = thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .iter()
            .map(|chunk| scope.spawn(move || Arc::new(category::compile(chunk))))
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                if let Ok(chunk) = handle.join() {
                    chunk
                } else {
                    tracing::warn!("job category rules: a compile chunk panicked and was skipped");
                    Arc::new(Classifier::default())
                }
            })
            .collect()
    });
    ChunkedClassifier { chunks: compiled }
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

/// The workflow name and repository of a GitHub job's run.
async fn github_run(
    conn: &mut SqliteConnection,
    run_id: i64,
) -> anyhow::Result<(String, Option<String>)> {
    sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT runs.workflow_name,repositories.full_name
           FROM workflow_runs runs
           LEFT JOIN repositories ON repositories.id=runs.repository_id
          WHERE runs.id=?",
    )
    .bind(run_id)
    .fetch_optional(&mut *conn)
    .await?
    .context("no workflow run for the job")
}

/// A GitLab project's path, memoized on the cache entry: the poll
/// classifies a whole page of one project's jobs, often inside one write
/// transaction, and the lookup is per job.
async fn project_path(
    conn: &mut SqliteConnection,
    cache: &Chunked,
    connection_id: &str,
    project_id: ExternalId,
) -> anyhow::Result<Option<String>> {
    let key = (
        connection_id.to_owned(),
        i64::try_from(project_id.get()).unwrap_or_default(),
    );
    {
        let projects = cache.projects.lock().await;
        if let Some(path) = projects.get(&key) {
            return Ok(path.clone());
        }
    }
    let found = gitlab_project_path(conn, connection_id, project_id).await?;
    cache.projects.lock().await.insert(key, found.clone());
    Ok(found)
}

/// A GitLab project's path: the connection's scope when the connection is
/// scoped to that project, else the path another of the project's
/// `gitlab_jobs` rows already stored.
async fn gitlab_project_path(
    conn: &mut SqliteConnection,
    connection_id: &str,
    project_id: ExternalId,
) -> anyhow::Result<Option<String>> {
    let project = i64::try_from(project_id.get())?;
    if let Some(scope) = sqlx::query_scalar::<_, String>(
        "SELECT scope_path FROM gitlab_connections
          WHERE id=? AND scope_kind='project' AND scope_id=?",
    )
    .bind(connection_id)
    .bind(project)
    .fetch_optional(&mut *conn)
    .await?
    {
        return Ok(Some(scope));
    }
    Ok(sqlx::query_scalar::<_, Option<String>>(
        "SELECT project_path FROM gitlab_jobs
              WHERE connection_id=? AND project_id=? AND project_path IS NOT NULL
              ORDER BY job_id LIMIT 1",
    )
    .bind(connection_id)
    .bind(project)
    .fetch_optional(&mut *conn)
    .await?
    .flatten())
}

/// The category to bind when inserting `job`, or `None` to leave it for the
/// backfill. Never fails a write: an error reading the rules or the run is
/// logged and gives `None`.
pub async fn category_for_insert(
    conn: &mut SqliteConnection,
    job: &NewJob<'_>,
    now: i64,
) -> Option<JobCategory> {
    let cache = match shared().chunks_for(conn, now).await {
        Ok(cache) => cache,
        Err(error) => {
            tracing::warn!(
                %error,
                "job category rules unreadable; the job is left for the backfill"
            );
            return None;
        }
    };
    let (workflow_name, repository) = match job.origin {
        JobOrigin::GitHubRun(run_id) => match github_run(conn, run_id).await {
            Ok((workflow_name, repository)) => (Some(workflow_name), repository),
            Err(error) => {
                tracing::warn!(
                    %error,
                    run_id,
                    "workflow run unreadable; the job is left for the backfill"
                );
                return None;
            }
        },
        JobOrigin::GitLabProject {
            connection_id,
            project_id,
            path,
        } => {
            let repository = match path {
                Some(path) => Some(path.to_owned()),
                None => match project_path(conn, &cache, connection_id, project_id).await {
                    Ok(path) => path,
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            connection_id,
                            "gitlab project unreadable; the job is left for the backfill"
                        );
                        return None;
                    }
                },
            };
            (None, repository)
        }
    };
    Some(cache.classifier.classify(
        job.job_name.unwrap_or(""),
        workflow_name.as_deref(),
        repository.as_deref(),
        job.labels,
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        ChunkedClassifier, JobCategory,
        category::{self, CategoryRule, RuleField},
        compile_chunked, compile_in_chunks,
    };

    fn rule(position: i64, category: JobCategory, field: RuleField, pattern: &str) -> CategoryRule {
        CategoryRule {
            position,
            category,
            field,
            pattern: pattern.to_owned(),
        }
    }

    /// A rule list that spans several chunks: an `other` rule at position 0
    /// ahead of a matching `deploy` rule and a last `other` rule (both would
    /// be lost by a chunk that cannot tell "matched an `other` rule" from
    /// "matched nothing"), a pattern that does not compile, and every rule
    /// field.
    fn spread() -> Vec<CategoryRule> {
        let mut rules = vec![
            rule(0, JobCategory::Other, RuleField::JobName, "sentinel"),
            rule(1, JobCategory::Test, RuleField::JobName, "([unclosed"),
            rule(2, JobCategory::Build, RuleField::WorkflowName, "^ci$"),
            rule(3, JobCategory::Check, RuleField::Repository, "acme/"),
            rule(4, JobCategory::Deploy, RuleField::JobName, "deploy"),
            rule(5, JobCategory::Test, RuleField::Labels, "docker"),
        ];
        for index in 0..12_i64 {
            rules.push(rule(
                6 + index,
                if index % 2 == 0 {
                    JobCategory::Build
                } else {
                    JobCategory::Other
                },
                RuleField::JobName,
                &format!("^filler-{index}$"),
            ));
        }
        rules.push(rule(18, JobCategory::Other, RuleField::JobName, "filler-5"));
        rules
    }

    fn answers_agree(chunked: &ChunkedClassifier, rules: &[CategoryRule]) {
        let whole = category::compile(rules);
        let labels = [vec!["docker".to_owned(), "self-hosted".to_owned()], vec![]];
        for job_name in [
            "sentinel",
            "deploy",
            "sentinel deploy",
            "filler-3",
            "filler-4",
            "unit tests",
            "",
        ] {
            for workflow_name in [None, Some("CI"), Some("Nightly")] {
                for repository in [None, Some("acme/app"), Some("team/other")] {
                    for labels in &labels {
                        assert_eq!(
                            chunked.classify(job_name, workflow_name, repository, labels),
                            whole.classify(job_name, workflow_name, repository, labels),
                            "{job_name}/{workflow_name:?}/{repository:?}/{} labels",
                            labels.len()
                        );
                    }
                }
            }
        }
    }

    /// The chunked rebuild answers what `Classifier::classify` on the whole
    /// rule list answers, however the rules fall into chunks — one chunk,
    /// chunks of a few rules, and the chunking [`compile_chunked`] picks.
    #[tokio::test]
    async fn chunked_classify_answers_what_the_whole_classifier_answers() {
        // Held so these compiles never contend with a speed check on a
        // one-core runner.
        let _serial = super::super::testdata::CLASSIFIER_TEST_LOCK.lock().await;
        let rules = spread();
        answers_agree(&compile_in_chunks(&rules, 1), &rules);
        let many = compile_in_chunks(&rules, 6);
        assert!(many.chunks.len() > 1, "several chunks must be pinned");
        answers_agree(&many, &rules);
        answers_agree(&compile_chunked(&rules), &rules);
    }

    /// An empty rule list (an admin may save one through #199's PUT)
    /// compiles, and every job is `other`. `chunks(0)` used to panic here,
    /// inside the writers' transactions.
    #[tokio::test]
    async fn an_empty_rule_list_classifies_everything_as_other() {
        let _serial = super::super::testdata::CLASSIFIER_TEST_LOCK.lock().await;
        for chunked in [
            compile_chunked(&[]),
            compile_in_chunks(&[], 1),
            compile_in_chunks(&[], 4),
        ] {
            assert_eq!(
                chunked.classify("unit tests", Some("ci"), Some("team/app"), &[]),
                JobCategory::Other
            );
        }
    }

    /// The sentinel a chunk answers instead of `other` never escapes the
    /// writers' path, even when a rule's own category would be it.
    #[tokio::test]
    async fn fix_agent_never_escapes_the_writers_path() {
        let _serial = super::super::testdata::CLASSIFIER_TEST_LOCK.lock().await;
        let rules = vec![rule(0, JobCategory::Other, RuleField::JobName, "sentinel")];
        let chunked = compile_in_chunks(&rules, 1);
        assert_eq!(
            chunked.classify("sentinel", None, None, &[]),
            JobCategory::Other
        );
        assert_eq!(
            chunked.classify("other job", None, None, &[]),
            JobCategory::Other
        );
    }
}
