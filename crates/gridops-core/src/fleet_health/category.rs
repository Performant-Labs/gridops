//! The job classifier (#190, #199): ordered rules, each a case-insensitive
//! regex over one field of a job; the first rule by `position` that matches
//! gives the category, and a job no rule matches is `other`.
//!
//! The rules live in `job_category_rules` (migration 0050, which seeds
//! [`default_rules`]). [`compile`] turns them into a [`Classifier`], skipping
//! (and logging) any rule whose pattern does not compile; [`validate`] is the
//! settings page's check (#199), which names every such rule instead.

use regex::{Regex, RegexBuilder};
use sqlx::SqliteConnection;

use super::JobCategory;

/// The compiled size limit of one pattern (bytes of compiled program). A
/// pattern over it is refused, so a rule can't make every job write slow.
pub const PATTERN_SIZE_LIMIT: usize = 256 * 1024;

/// The field of a job a rule matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleField {
    JobName,
    WorkflowName,
    Repository,
    /// Matches when any one of the job's labels matches.
    Labels,
}

impl RuleField {
    pub const ALL: [Self; 4] = [
        Self::JobName,
        Self::WorkflowName,
        Self::Repository,
        Self::Labels,
    ];

    /// The `job_category_rules.field` value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::JobName => "job_name",
            Self::WorkflowName => "workflow_name",
            Self::Repository => "repository",
            Self::Labels => "labels",
        }
    }

    #[must_use]
    pub fn from_column(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.as_str() == value)
    }
}

/// One `job_category_rules` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryRule {
    pub position: i64,
    pub category: JobCategory,
    pub field: RuleField,
    pub pattern: String,
}

/// A rule whose pattern does not compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleError {
    pub position: i64,
    pub message: String,
}

/// The four rules migration 0050 seeds, all on the job name.
#[must_use]
pub fn default_rules() -> Vec<CategoryRule> {
    [
        (
            JobCategory::Test,
            "test|spec|e2e|playwright|vitest|jest|pytest|cargo test|shard",
        ),
        (
            JobCategory::Build,
            "build|compile|bundle|image|docker|package",
        ),
        (
            JobCategory::Check,
            "lint|clippy|fmt|format|typecheck|tsc|check|audit",
        ),
        (JobCategory::Deploy, "deploy|release|publish|pages"),
    ]
    .into_iter()
    .zip(0_i64..)
    .map(|((category, pattern), position)| CategoryRule {
        position,
        category,
        field: RuleField::JobName,
        pattern: pattern.to_owned(),
    })
    .collect()
}

fn build(pattern: &str) -> Result<Regex, regex::Error> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .size_limit(PATTERN_SIZE_LIMIT)
        .dfa_size_limit(PATTERN_SIZE_LIMIT)
        .build()
}

/// Every rule's compiled pattern, in the order given, or one error per rule
/// that does not compile (in the order given).
///
/// # Errors
/// The rules whose patterns do not compile or are over
/// [`PATTERN_SIZE_LIMIT`].
pub fn validate(rules: &[CategoryRule]) -> Result<Vec<Regex>, Vec<RuleError>> {
    let mut compiled = Vec::with_capacity(rules.len());
    let mut errors = Vec::new();
    for rule in rules {
        match build(&rule.pattern) {
            Ok(regex) => compiled.push(regex),
            Err(error) => errors.push(RuleError {
                position: rule.position,
                message: error.to_string(),
            }),
        }
    }
    if errors.is_empty() {
        Ok(compiled)
    } else {
        Err(errors)
    }
}

#[derive(Debug, Clone)]
struct CompiledRule {
    category: JobCategory,
    field: RuleField,
    regex: Regex,
}

/// Compiled rules, in position order.
#[derive(Debug, Clone, Default)]
pub struct Classifier {
    rules: Vec<CompiledRule>,
}

/// The rules in position order, compiled. A rule that does not compile is
/// left out (and logged): the rest still classify.
#[must_use]
pub fn compile(rules: &[CategoryRule]) -> Classifier {
    let mut ordered: Vec<&CategoryRule> = rules.iter().collect();
    ordered.sort_by_key(|rule| rule.position);
    let rules = ordered
        .into_iter()
        .filter_map(|rule| match build(&rule.pattern) {
            Ok(regex) => Some(CompiledRule {
                category: rule.category,
                field: rule.field,
                regex,
            }),
            Err(error) => {
                tracing::warn!(
                    position = rule.position,
                    %error,
                    "job category rule skipped: its pattern does not compile"
                );
                None
            }
        })
        .collect();
    Classifier { rules }
}

impl Classifier {
    /// The category of a job. A missing workflow name or repository matches
    /// no rule on that field; a labels rule matches when any label does.
    #[must_use]
    pub fn classify(
        &self,
        job_name: &str,
        workflow_name: Option<&str>,
        repository: Option<&str>,
        labels: &[String],
    ) -> JobCategory {
        self.rules
            .iter()
            .find(|rule| match rule.field {
                RuleField::JobName => rule.regex.is_match(job_name),
                RuleField::WorkflowName => {
                    workflow_name.is_some_and(|name| rule.regex.is_match(name))
                }
                RuleField::Repository => repository.is_some_and(|name| rule.regex.is_match(name)),
                RuleField::Labels => labels.iter().any(|label| rule.regex.is_match(label)),
            })
            .map_or(JobCategory::Other, |rule| rule.category)
    }
}

/// The stored rules, in position order. A row with an unknown category or
/// field (the table's CHECKs make that impossible) is skipped.
///
/// # Errors
/// The read fails.
pub async fn load_rules(conn: &mut SqliteConnection) -> anyhow::Result<Vec<CategoryRule>> {
    let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT position,category,field,pattern FROM job_category_rules ORDER BY position",
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(position, category, field, pattern)| {
            Some(CategoryRule {
                position,
                category: category.parse().ok()?,
                field: RuleField::from_column(&field)?,
                pattern,
            })
        })
        .collect())
}
