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
//! (`workflow_jobs`, read through `workflow_jobs_completed_idx` and limited
//! to its `name`, `run_id` and `labels` columns, with the workflow name and
//! repository joined in by `run_id` for rules on those fields) and saves
//! nothing. GitLab jobs are not previewed: their names are `gitlab_jobs`
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

/// One window job read for the preview: its name, run, labels, and the
/// workflow name and repository the run joins to.
type WindowJob = (String, i64, String, Option<String>, Option<String>);

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
    let since = now_millis() - PREVIEW_WINDOW_MILLIS;
    let jobs: Vec<WindowJob> = sqlx::query_as(
        "SELECT wj.name,wj.run_id,wj.labels,wr.workflow_name,repo.full_name
         FROM workflow_jobs wj
         LEFT JOIN workflow_runs wr ON wr.id=wj.run_id
         LEFT JOIN repositories repo ON repo.id=wr.repository_id
         WHERE wj.completed_at IS NOT NULL AND wj.completed_at>=?",
    )
    .bind(since)
    .fetch_all(&state.database)
    .await?;
    let mut matches = vec![0_u64; rules.len()];
    let mut other: HashMap<String, u64> = HashMap::new();
    for (name, _run_id, labels, workflow_name, repository) in jobs {
        let labels: Vec<String> = serde_json::from_str(&labels).unwrap_or_default();
        let matched = rules
            .iter()
            .zip(&compiled)
            .position(|(rule, regex)| match rule.field {
                RuleField::JobName => regex.is_match(&name),
                RuleField::WorkflowName => workflow_name
                    .as_deref()
                    .is_some_and(|value| regex.is_match(value)),
                RuleField::Repository => repository
                    .as_deref()
                    .is_some_and(|value| regex.is_match(value)),
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

    use super::*;

    /// The 422 of `parse_rules`, as its `details`.
    fn problems(body: &Value) -> Result<Value> {
        match parse_rules(body) {
            Ok(rules) => bail!("the body parsed as {rules:?}"),
            Err(ApiError::Invalid { code, details, .. }) if code == CODE => Ok(details),
            Err(other) => bail!("not the rules 422: {other:?}"),
        }
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
}
