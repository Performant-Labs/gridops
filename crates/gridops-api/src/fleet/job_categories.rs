//! The job category rules in Settings (#199): list, save (the whole list,
//! in order) and preview. Session-only and administrators only: the routes
//! are registered in `session_routes()` (`main.rs`), under `refuse_bearer`
//! (W-5).
//!
//! The bodies (`job_categories_contract_tests.rs`):
//! - `GET` → `{"rules": [{position, category, field, pattern}]}`;
//! - `PUT` `{"rules": [{category, field, pattern}]}` → the saved list, or a
//!   422 `ApiError::Invalid` whose `details` are `[{position, message}]`;
//! - `POST …/preview` with a proposed list → `{"rules": [{position,
//!   matches}], "other": [{name, jobs}]}`.
//!
//! Saving replaces the table in one short transaction (delete, then batched
//! multi-row inserts of at most [`INSERT_BATCH`] rules). A proposed rule is
//! checked with `fleet_health::category::validate`, which names every rule
//! whose pattern does not compile (D-3(d)); anything else off-contract
//! (an unknown category, `fix-agent`, an unknown field, an empty pattern,
//! an unknown key) is named the same way. The preview compiles the proposed
//! rules the same way, then classifies the jobs completed in the last 7 days
//! and saves nothing. It reads `workflow_jobs` through
//! `workflow_jobs_completed_idx`, selecting only its `name`, `run_id` and
//! `labels` columns, in six-hour chunks (one statement over the whole week
//! would run far past the 100 ms slow-statement limit, #179); the workflow
//! name and repository a rule on those fields matches are resolved by
//! `run_id`, in batches of [`INSERT_BATCH`] runs, only when such a rule is
//! proposed. GitLab jobs are not previewed: their names are `gitlab_jobs`
//! rows, which carry no `run_id`.

use std::collections::HashMap;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse as _, Response},
};
use gridops_core::{
    fleet_health::{
        JobCategory,
        category::{self, CategoryRule, RuleField},
    },
    now_millis,
};
use serde_json::{Value, json};
use sqlx::SqliteConnection;

use crate::{
    auth::{AuthUser, require_system_admin},
    error::{ApiError, ApiResult},
    resources::SameOrigin,
    state::AppState,
};

/// The stable `code` of the 422 (`docs/openapi/gridops-v1.json`).
const CODE: &str = "invalid_job_category_rules";

/// How many names the preview's `other` list holds at most.
const OTHER_NAMES_LIMIT: usize = 20;

/// One multi-row insert holds at most this many rules (#179's batch bound).
const INSERT_BATCH: usize = 500;

/// The window the preview counts over, in milliseconds (7 days).
const PREVIEW_WINDOW_MILLIS: i64 = 7 * 86_400_000;

/// How much of the window one preview statement reads at most (6 hours),
/// bounded on both ends by the `completed_at` index.
const READ_CHUNK_MILLIS: i64 = 6 * 3_600_000;

/// One window job read for the preview: its name, run and labels.
type WindowJob = (String, i64, String);

/// `{"rules": [...]}`, the shape of `GET`'s and `PUT`'s answers.
fn rules_json(rules: &[CategoryRule]) -> Value {
    json!({"rules": rules.iter().map(|rule| json!({
        "position": rule.position,
        "category": rule.category.as_str(),
        "field": rule.field.as_str(),
        "pattern": rule.pattern,
    })).collect::<Vec<_>>()})
}

/// The 422 for rules that are off-contract or do not compile: `details`
/// names each failing rule (`[{position, message}]`, D-3(d)).
fn invalid_rules(details: &[Value]) -> ApiError {
    let message = if details.len() == 1 {
        "1 rule is invalid.".to_owned()
    } else {
        format!("{} rules are invalid.", details.len())
    };
    ApiError::Invalid {
        code: CODE,
        message,
        details: json!(details),
    }
}

/// One `{category, field, pattern}` of the request body, positioned by its
/// place in the list.
fn parse_rule(position: usize, item: &Value) -> Result<CategoryRule, String> {
    let object = item
        .as_object()
        .ok_or_else(|| "a rule must be an object".to_owned())?;
    for key in object.keys() {
        if !matches!(key.as_str(), "category" | "field" | "pattern") {
            return Err(format!("unknown key `{key}`"));
        }
    }
    let category = object
        .get("category")
        .and_then(Value::as_str)
        .ok_or_else(|| "a rule needs a `category`".to_owned())?;
    let category: JobCategory = category
        .parse()
        .map_err(|_| format!("unknown category `{category}`"))?;
    if category == JobCategory::FixAgent {
        return Err(
            "`fix-agent` is counted from fix placements and is never a rule's \
category"
                .to_owned(),
        );
    }
    let field = object
        .get("field")
        .and_then(Value::as_str)
        .ok_or_else(|| "a rule needs a `field`".to_owned())?;
    let field = RuleField::from_column(field).ok_or_else(|| format!("unknown field `{field}`"))?;
    let pattern = object
        .get("pattern")
        .and_then(Value::as_str)
        .ok_or_else(|| "a rule needs a `pattern` string".to_owned())?;
    if pattern.is_empty() {
        return Err("the pattern is empty".to_owned());
    }
    let position =
        i64::try_from(position).map_err(|_| "the list of rules is too long".to_owned())?;
    Ok(CategoryRule {
        position,
        category,
        field,
        pattern: pattern.to_owned(),
    })
}

/// The body's `{"rules": [...]}`, as positioned rules, or the 422 naming
/// every rule that is off-contract.
fn parse_rules(body: &Value) -> Result<Vec<CategoryRule>, ApiError> {
    if let Some(key) = body
        .as_object()
        .into_iter()
        .flat_map(|object| object.keys())
        .find(|key| key.as_str() != "rules")
    {
        return Err(ApiError::Invalid {
            code: CODE,
            message: format!("unknown key `{key}`"),
            details: json!([]),
        });
    }
    let Some(list) = body.get("rules").and_then(Value::as_array) else {
        return Err(ApiError::Invalid {
            code: CODE,
            message: "the body must carry a `rules` array".to_owned(),
            details: json!([]),
        });
    };
    let mut rules = Vec::with_capacity(list.len());
    let mut details = Vec::new();
    for (position, item) in list.iter().enumerate() {
        match parse_rule(position, item) {
            Ok(rule) => rules.push(rule),
            Err(message) => details.push(json!({"position": position, "message": message})),
        }
    }
    if details.is_empty() {
        Ok(rules)
    } else {
        Err(invalid_rules(&details))
    }
}

/// The commonest names left in `other`, most jobs first, ties by name, at
/// most [`OTHER_NAMES_LIMIT`] of them.
fn top_other(counts: HashMap<String, u64>) -> Vec<(String, u64)> {
    let mut list: Vec<(String, u64)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    list.truncate(OTHER_NAMES_LIMIT);
    list
}

/// `GET /api/v1/settings/job-categories`.
pub(crate) async fn rules(State(state): State<AppState>, user: AuthUser) -> ApiResult<Response> {
    require_system_admin(&user)?;
    let mut conn = state.database.acquire().await?;
    let rules = category::load_rules(&mut conn).await?;
    Ok((StatusCode::OK, Json(rules_json(&rules))).into_response())
}

/// Inserts `rules` (all at the same timestamps) as one multi-row `VALUES`.
async fn insert_batch(
    conn: &mut SqliteConnection,
    rules: &[CategoryRule],
    now: i64,
) -> sqlx::Result<()> {
    let mut builder = sqlx::QueryBuilder::new(
        "INSERT INTO job_category_rules (position,category,field,pattern,created_at,updated_at) ",
    );
    builder.push_values(rules.iter(), |mut row, rule| {
        row.push_bind(rule.position)
            .push_bind(rule.category.as_str())
            .push_bind(rule.field.as_str())
            .push_bind(rule.pattern.as_str())
            .push_bind(now)
            .push_bind(now);
    });
    builder.build().execute(conn).await?;
    Ok(())
}

/// `PUT /api/v1/settings/job-categories`.
pub(crate) async fn save_rules(
    State(state): State<AppState>,
    _same_origin: SameOrigin,
    user: AuthUser,
    Json(input): Json<Value>,
) -> ApiResult<Response> {
    require_system_admin(&user)?;
    let rules = parse_rules(&input)?;
    if let Err(errors) = category::validate(&rules) {
        let details: Vec<Value> = errors
            .iter()
            .map(|error| json!({"position": error.position, "message": error.message}))
            .collect();
        return Err(invalid_rules(&details));
    }
    let now = now_millis();
    let mut transaction = state.database.begin().await?;
    sqlx::query("DELETE FROM job_category_rules")
        .execute(&mut *transaction)
        .await?;
    for batch in rules.chunks(INSERT_BATCH) {
        insert_batch(&mut transaction, batch, now).await?;
    }
    transaction.commit().await?;
    Ok((StatusCode::OK, Json(rules_json(&rules))).into_response())
}

/// The workflow name and repository of each of `run_ids`, read in batches so
/// no one statement scans more than [`INSERT_BATCH`] runs (#179).
async fn run_fields(
    database: &sqlx::SqlitePool,
    run_ids: &[i64],
) -> sqlx::Result<HashMap<i64, (String, Option<String>)>> {
    let mut runs = HashMap::with_capacity(run_ids.len());
    for batch in run_ids.chunks(INSERT_BATCH) {
        let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT wr.id,wr.workflow_name,repo.full_name
             FROM workflow_runs wr
             LEFT JOIN repositories repo ON repo.id=wr.repository_id
             WHERE wr.id IN (",
        );
        let mut ids = builder.separated(",");
        for id in batch {
            ids.push_bind(*id);
        }
        ids.push_unseparated(")");
        let rows: Vec<(i64, String, Option<String>)> =
            builder.build_query_as().fetch_all(database).await?;
        runs.extend(
            rows.into_iter()
                .map(|(id, workflow_name, repository)| (id, (workflow_name, repository))),
        );
    }
    Ok(runs)
}

/// `POST /api/v1/settings/job-categories/preview`.
pub(crate) async fn preview(
    State(state): State<AppState>,
    _same_origin: SameOrigin,
    user: AuthUser,
    Json(input): Json<Value>,
) -> ApiResult<Response> {
    require_system_admin(&user)?;
    let rules = parse_rules(&input)?;
    let compiled = category::validate(&rules).map_err(|errors| {
        let details: Vec<Value> = errors
            .iter()
            .map(|error| json!({"position": error.position, "message": error.message}))
            .collect();
        invalid_rules(&details)
    })?;
    let now = now_millis();
    let since = now - PREVIEW_WINDOW_MILLIS;
    // The window is read in time-bounded chunks, like the rollups read their
    // hours: one statement over the whole week would run far past the 100 ms
    // slow-statement limit (#179).
    let mut jobs = Vec::new();
    let mut low = since;
    while low <= now {
        let high = (low + READ_CHUNK_MILLIS).min(now + 1);
        let batch: Vec<WindowJob> = sqlx::query_as(
            "SELECT name,run_id,labels FROM workflow_jobs
             WHERE completed_at>=? AND completed_at<?",
        )
        .bind(low)
        .bind(high)
        .fetch_all(&state.database)
        .await?;
        jobs.extend(batch);
        low = high;
    }
    // Only a rule on the workflow name or repository needs the run's fields.
    let runs = if rules
        .iter()
        .any(|rule| matches!(rule.field, RuleField::WorkflowName | RuleField::Repository))
    {
        let mut ids: Vec<i64> = jobs.iter().map(|(_, run_id, _)| *run_id).collect();
        ids.sort_unstable();
        ids.dedup();
        run_fields(&state.database, &ids).await?
    } else {
        HashMap::new()
    };
    let mut matches = vec![0_u64; rules.len()];
    let mut other: HashMap<String, u64> = HashMap::new();
    for (name, run_id, labels) in jobs {
        let labels: Vec<String> = serde_json::from_str(&labels).unwrap_or_default();
        let (workflow_name, repository) = runs
            .get(&run_id)
            .map_or((None, None), |(workflow, repository)| {
                (Some(workflow.as_str()), repository.as_deref())
            });
        let matched = rules
            .iter()
            .zip(&compiled)
            .position(|(rule, regex)| match rule.field {
                RuleField::JobName => regex.is_match(&name),
                RuleField::WorkflowName => workflow_name.is_some_and(|value| regex.is_match(value)),
                RuleField::Repository => repository.is_some_and(|value| regex.is_match(value)),
                RuleField::Labels => labels.iter().any(|label| regex.is_match(label)),
            });
        match matched {
            Some(index) => matches[index] += 1,
            None => *other.entry(name).or_default() += 1,
        }
    }
    let body = json!({
        "rules": rules.iter().zip(&matches).map(|(rule, matches)| json!({
            "position": rule.position,
            "matches": matches,
        })).collect::<Vec<_>>(),
        "other": top_other(other).into_iter().map(|(name, jobs)| json!({
            "name": name,
            "jobs": jobs,
        })).collect::<Vec<_>>(),
    });
    Ok((StatusCode::OK, Json(body)).into_response())
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, bail};
    use axum::http::Method;
    use gridops_core::fleet_health::{
        category,
        testdata::{self, JobSeed},
    };

    use super::*;
    use crate::fleet::health_test_support::Harness;

    const PREVIEW_PATH: &str = "/api/v1/settings/job-categories/preview";
    /// `seed_fleet`'s repository, the one every seeded run joins to.
    const SEEDED_REPOSITORY: &str = "acme/app";

    /// The 422 of `parse_rules`, as its `details`.
    fn problems(body: &Value) -> Result<Value> {
        match parse_rules(body) {
            Ok(rules) => bail!("the body parsed as {rules:?}"),
            Err(ApiError::Invalid { code, details, .. }) if code == CODE => Ok(details),
            Err(other) => bail!("not the rules 422: {other:?}"),
        }
    }

    /// The rules, as the request body carries them.
    fn input(rules: &[CategoryRule]) -> Value {
        json!({"rules": rules.iter().map(|rule| json!({
            "category": rule.category.as_str(),
            "field": rule.field.as_str(),
            "pattern": rule.pattern,
        })).collect::<Vec<_>>()})
    }

    /// What the preview answered for each rule position.
    async fn preview_counts(harness: &Harness, body: Value) -> Result<Vec<i64>> {
        let admin = harness.admin.clone();
        let (status, answered) = harness
            .send(Method::POST, PREVIEW_PATH, Some(&admin), Some(body))
            .await?;
        assert_eq!(status, StatusCode::OK, "{answered}");
        Ok(answered
            .pointer("/rules")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|rule| {
                rule.get("matches")
                    .and_then(Value::as_i64)
                    .unwrap_or_default()
            })
            .collect())
    }

    /// A rule off-contract in each way the parser names.
    #[test]
    fn off_contract_rules_are_named_by_position() -> Result<()> {
        let details = problems(&json!({"rules": [
            {"category": "test", "field": "job_name", "pattern": "ok"},
            {"category": "fix-agent", "field": "job_name", "pattern": "x"},
            {"category": "build", "field": "nope", "pattern": "x"},
            {"category": "deploy", "field": "job_name", "pattern": ""},
            {"category": "check", "field": "job_name", "pattern": "x", "extra": 1},
            "not an object",
        ]}))?;
        let entries = details.as_array().cloned().unwrap_or_default();
        let positions: Vec<i64> = entries
            .iter()
            .filter_map(|detail| detail.get("position").and_then(Value::as_i64))
            .collect();
        assert_eq!(positions, [1, 2, 3, 4, 5], "{entries:?}");
        for detail in &entries {
            assert!(
                !detail
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .is_empty(),
                "{detail}"
            );
        }
        Ok(())
    }

    /// A body without a `rules` array, or with unknown keys, is refused too.
    #[test]
    fn the_body_itself_is_checked() -> Result<()> {
        for body in [
            json!({}),
            json!({"rules": null}),
            json!({"rules": [], "why": 1}),
        ] {
            problems(&body)?;
        }
        Ok(())
    }

    /// The commonest `other` names first, ties by name, twenty at most.
    #[test]
    fn the_other_list_is_the_commonest_twenty() {
        let mut counts = HashMap::new();
        for name in ["a", "b", "c"] {
            counts.insert(name.to_owned(), 3);
        }
        counts.insert("busy".to_owned(), 9);
        counts.insert("rare".to_owned(), 1);
        let list = top_other(counts);
        assert_eq!(
            list,
            [
                ("busy".to_owned(), 9),
                ("a".to_owned(), 3),
                ("b".to_owned(), 3),
                ("c".to_owned(), 3),
                ("rare".to_owned(), 1),
            ]
        );
        let many: HashMap<String, u64> = (0..30)
            .map(|index| (format!("job{index}"), index))
            .collect();
        assert_eq!(top_other(many).len(), OTHER_NAMES_LIMIT);
    }

    /// The preview attributes each job exactly as `category::compile`'s
    /// classifier would classify it, on every rule field (the `repository`
    /// rule is checked apart, because every seeded job shares one).
    #[tokio::test]
    async fn the_preview_counts_what_the_classifier_classifies() -> Result<()> {
        let harness = Harness::new().await?;
        let fleet = testdata::seed_fleet(&harness.database).await?;
        let io = fleet.host("io")?.id.clone();
        let now = gridops_core::now_millis();
        let job = |id: i64, name: &str, workflow: &str, labels: &[String]| {
            let mut seed = JobSeed::completed(
                id,
                name,
                Some(&io),
                now - 60_000,
                now - 50_000,
                now - 40_000,
            );
            seed.workflow = workflow.to_owned();
            seed.labels = labels.to_vec();
            seed
        };
        let gpu = vec!["gpu".to_owned()];
        let jobs = vec![
            job(101, "anything", "deploy on push", &[]),
            job(102, "misc", "CI", &gpu),
            job(103, "build wheels", "CI", &[]),
            job(104, "misc", "CI", &[]),
            job(105, "misc", "CI", &gpu),
            job(106, "build wheels", "CI", &[]),
        ];
        testdata::insert_jobs(&harness.database, &fleet, &jobs).await?;
        let rules = vec![
            CategoryRule {
                position: 0,
                category: JobCategory::Deploy,
                field: RuleField::WorkflowName,
                pattern: "^deploy on push$".to_owned(),
            },
            CategoryRule {
                position: 1,
                category: JobCategory::Check,
                field: RuleField::Labels,
                pattern: "^gpu$".to_owned(),
            },
            CategoryRule {
                position: 2,
                category: JobCategory::Build,
                field: RuleField::JobName,
                pattern: "wheels".to_owned(),
            },
        ];

        // What the classifier itself says, per category.
        let classifier = category::compile(&rules);
        let mut expected: HashMap<JobCategory, u64> = HashMap::new();
        for seed in &jobs {
            let classified = classifier.classify(
                &seed.name,
                Some(&seed.workflow),
                Some(SEEDED_REPOSITORY),
                &seed.labels,
            );
            *expected.entry(classified).or_default() += 1;
        }
        let counted = preview_counts(&harness, input(&rules)).await?;
        for (rule, matches) in rules.iter().zip(&counted) {
            assert_eq!(
                *matches,
                i64::try_from(expected.get(&rule.category).copied().unwrap_or_default())?,
                "rule {} ({}) counts what the classifier classifies",
                rule.position,
                rule.category
            );
        }
        assert_eq!(
            expected
                .get(&JobCategory::Other)
                .copied()
                .unwrap_or_default(),
            1,
            "the unmatched job is other"
        );

        // A rule on the repository matches every seeded job through the join.
        let repository_rule = vec![CategoryRule {
            position: 0,
            category: JobCategory::Build,
            field: RuleField::Repository,
            pattern: "^acme/".to_owned(),
        }];
        let counted = preview_counts(&harness, input(&repository_rule)).await?;
        assert_eq!(counted, [i64::try_from(jobs.len())?]);
        Ok(())
    }

    /// On a full synthetic week (#197's `build_week`, about 50,000 jobs) the
    /// preview answers with no statement over the 100 ms slow-statement limit
    /// and no write at all (`speed_check` fails the test otherwise), with
    /// job-name rules (one read of the window) and with a workflow-name rule
    /// (the run fields resolved in batches on top).
    #[tokio::test]
    async fn the_preview_stays_under_the_slow_statement_limit_on_a_week_of_jobs() -> Result<()> {
        let harness = Harness::new().await?;
        let hour = 3_600_000;
        let end = gridops_core::now_millis() / hour * hour;
        testdata::build_week(&harness.database, end).await?;
        let admin = harness.admin.clone();

        let classified_of = |answered: &Value| {
            answered
                .pointer("/rules")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .iter()
                .chain(
                    answered
                        .pointer("/other")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                        .iter(),
                )
                .map(|entry| entry.get("matches").or_else(|| entry.get("jobs")))
                .map(|count| count.and_then(Value::as_i64).unwrap_or_default())
                .sum::<i64>()
        };
        for (name, rules) in [
            ("job-name rules", category::default_rules()),
            (
                "a workflow-name rule",
                vec![CategoryRule {
                    position: 0,
                    category: JobCategory::Deploy,
                    field: RuleField::WorkflowName,
                    pattern: "^CI$".to_owned(),
                }],
            ),
        ] {
            let body = input(&rules);
            let started = std::time::Instant::now();
            let (status, answered) = gridops_core::fleet_health::speed_check::check(
                &harness.database,
                harness.send(Method::POST, PREVIEW_PATH, Some(&admin), Some(body)),
            )
            .await?;
            let elapsed = started.elapsed();
            assert_eq!(status, StatusCode::OK, "{answered}");
            let classified = classified_of(&answered);
            println!(
                "preview over the synthetic week, {name}: {classified} jobs classified in \
                 {elapsed:?} (queried through the completed_at index, nothing written)"
            );
            assert!(
                classified >= 45_000,
                "the week's jobs were not all counted: {classified}"
            );
        }
        Ok(())
    }
}
