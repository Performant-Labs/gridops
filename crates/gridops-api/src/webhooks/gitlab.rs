//! #140: GitLab Job Hook ingestion and busy tracking.
//!
//! `POST /api/v1/webhooks/gitlab` matches a delivery to one GitLab connection
//! by its `X-Gitlab-Token`, records it in `webhook_deliveries` beside GitHub's
//! deliveries, and applies `object_kind: "build"` events to `gitlab_jobs`
//! through D18's shared writer ([`record_gitlab_job`]), which owns the status
//! rule. This module decides only what the writer does not: when to fetch a
//! job's tags, and when a runner becomes busy or idle.
//!
//! Delivery ids are `gitlab:<connection_id>:<key>` for a delivery matched to
//! a connection, and `gitlab:unmatched:<key>` for one that matched none.
//! `retry` re-runs [`reprocess`] on the recorded connection.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use gridops_core::{
    fleet::ExternalId,
    gitlab::{
        GitLabApi, GitLabCredentials, JobStatus,
        jobs::{JobObservation, JobWrite, record_gitlab_job, set_gitlab_runner_busy},
    },
    now_millis,
};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::{FromRow, Row as _, SqlitePool};
use subtle::ConstantTimeEq as _;

use super::{
    MAX_WEBHOOK_BYTES, NewDelivery, date_value, i64_value, mark_delivery_failed,
    mark_delivery_processed, nested_object, nested_string, record_delivery, reject_delivery,
    required_header, string,
};
use crate::{
    error::{ApiError, ApiResult},
    resources::gitlab_credentials,
    state::AppState,
};

/// The prefix of every GitLab delivery id.
const DELIVERY_PREFIX: &str = "gitlab:";
/// The connection part of the id of a delivery no connection matched.
const UNMATCHED: &str = "unmatched";

/// A GitLab connection a delivery can be matched to.
#[derive(FromRow)]
struct Connection {
    id: String,
    base_url: String,
    access_token_key: String,
    webhook_secret_key: String,
}

/// Receives a GitLab webhook delivery. Answers 202 with no body once the
/// delivery is recorded (including a duplicate, and one whose processing
/// failed), 400 when the body is not a JSON object, and 401 when no
/// connection of the payload's project has the token as its webhook secret.
pub async fn receive_gitlab(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    if body.len() > MAX_WEBHOOK_BYTES {
        return Err(ApiError::PayloadTooLarge(
            "Webhook payload exceeds 25 MB.".into(),
        ));
    }
    let event = required_header(&headers, "x-gitlab-event")?;
    let payload: Value = serde_json::from_slice(&body)
        .map_err(|_| ApiError::BadRequest("The payload is not a JSON object.".into()))?;
    let object = payload
        .as_object()
        .ok_or_else(|| ApiError::BadRequest("The payload is not a JSON object.".into()))?;
    let key = delivery_key(&headers, &body);
    let token = headers
        .get("x-gitlab-token")
        .and_then(|value| value.to_str().ok());
    let connection = match token {
        Some(token) => matching_connection(&state, object, token).await?,
        None => None,
    };
    let delivery_id = format!(
        "{DELIVERY_PREFIX}{}:{key}",
        connection
            .as_ref()
            .map_or(UNMATCHED, |connection| connection.id.as_str())
    );
    let now = now_millis();
    let recorded = record_delivery(
        &state.database,
        &NewDelivery {
            id: &delivery_id,
            event: &event,
            action: string(object, "build_status"),
            hook_id: None,
            installation_id: None,
            repository_id: None,
            signature_valid: connection.is_some(),
            payload: &payload,
            received_at: now,
        },
    )
    .await?;
    let Some(connection) = connection else {
        if recorded {
            let reason = if token.is_some() {
                "No GitLab connection's webhook secret matches the token."
            } else {
                "Missing X-Gitlab-Token header."
            };
            reject_delivery(&state.database, &delivery_id, reason).await?;
        }
        return Err(ApiError::Unauthorized);
    };
    if !recorded {
        return Ok(StatusCode::ACCEPTED);
    }

    match process_with_connection(&state, &connection, object, now).await {
        Ok(()) => mark_delivery_processed(&state.database, &delivery_id).await?,
        Err(error) => {
            mark_delivery_failed(&state.database, &delivery_id, &error).await?;
            tracing::error!(delivery_id, event, error = ?error, "GitLab webhook processing failed");
        }
    }
    Ok(StatusCode::ACCEPTED)
}

/// The connection id recorded in a GitLab delivery id, or `None` when
/// `delivery_id` is not a GitLab one. The id is `gitlab:<connection>:<key>`:
/// it is split once, at the first `:` after the prefix, so a key that itself
/// contains `:` stays whole. A rejected delivery's connection is `unmatched`;
/// `retry` refuses it before asking, as it is recorded unverified.
pub(super) fn recorded_connection(delivery_id: &str) -> Option<&str> {
    let (connection_id, _key) = delivery_id.strip_prefix(DELIVERY_PREFIX)?.split_once(':')?;
    Some(connection_id)
}

/// Processes the stored payload of delivery `delivery_id` again, on the
/// connection its id records: the token was checked when it was received,
/// and a re-resolved candidate set could pick another connection.
pub(super) async fn reprocess(
    state: &AppState,
    delivery_id: &str,
    connection_id: &str,
    payload: &Map<String, Value>,
) -> ApiResult<()> {
    if connection_id == UNMATCHED {
        return Err(ApiError::BadRequest(
            "Only verified webhook payloads can be retried.".into(),
        ));
    }
    let connection = sqlx::query_as::<_, Connection>(
        "SELECT id,base_url,access_token_key,webhook_secret_key FROM gitlab_connections WHERE id=?",
    )
    .bind(connection_id)
    .fetch_optional(&state.database)
    .await?
    .ok_or_else(|| {
        ApiError::BadRequest(format!(
            "GitLab connection {connection_id} no longer exists."
        ))
    })?;
    // The fallback status time is the time GridOps received the delivery,
    // so a retry records the job as the first attempt would have.
    let received_at =
        sqlx::query_scalar::<_, i64>("SELECT received_at FROM webhook_deliveries WHERE id=?")
            .bind(delivery_id)
            .fetch_one(&state.database)
            .await?;
    process_with_connection(state, &connection, payload, received_at).await
}

/// The delivery's key, unique per delivery and the same when GitLab sends it
/// again.
///
/// GitLab's docs name `webhook-id` as preferred over `Idempotency-Key`, but
/// `Idempotency-Key` comes first here because it is the one documented as
/// constant across webhook retries, which is what duplicate detection needs.
/// The fallbacks (`webhook-id`, `X-Gitlab-Event-UUID`, then the SHA-256 of
/// the body) matter for GitLab versions without `Idempotency-Key`, or a
/// future one that drops it.
fn delivery_key(headers: &HeaderMap, body: &[u8]) -> String {
    ["idempotency-key", "webhook-id", "x-gitlab-event-uuid"]
        .into_iter()
        .find_map(|name| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| hex::encode(Sha256::digest(body)))
}

/// The connection whose webhook secret is `token`, among the payload
/// project's candidates: every project connection for its `project_id`, and
/// every group connection whose path contains the project's path at a `/`
/// boundary. Project ids are unique only per GitLab instance, so there can be
/// several. Every candidate's secret is compared in constant time; when more
/// than one matches, a project connection wins over a group one, then the
/// oldest.
async fn matching_connection(
    state: &AppState,
    payload: &Map<String, Value>,
    token: &str,
) -> ApiResult<Option<Connection>> {
    // A Job Hook has a top-level `project_id`; other hooks (a Pipeline Hook)
    // carry the project only as `project.id`.
    let project_id = i64_value(payload, "project_id")
        .or_else(|| nested_object(payload, "project").and_then(|project| i64_value(project, "id")));
    let project_path = nested_string(payload, "project", "path_with_namespace");
    if project_id.is_none() && project_path.is_none() {
        return Ok(None);
    }
    let candidates = sqlx::query_as::<_, Connection>(
        "SELECT id,base_url,access_token_key,webhook_secret_key FROM gitlab_connections
         WHERE (scope_kind='project' AND scope_id=?1)
            OR (scope_kind='group' AND ?2 IS NOT NULL
                AND substr(?2,1,length(scope_path)+1)=scope_path||'/')
         ORDER BY CASE scope_kind WHEN 'project' THEN 0 ELSE 1 END,created_at,id",
    )
    .bind(project_id)
    .bind(project_path)
    .fetch_all(&state.database)
    .await?;
    let mut matched = None;
    for candidate in candidates {
        let Some(secret) = state
            .runtime_secret(&candidate.webhook_secret_key)
            .await
            .map_err(ApiError::Internal)?
        else {
            continue;
        };
        // No early exit: every candidate is compared, and the first match in
        // preference order is kept.
        let equal = bool::from(secret.as_bytes().ct_eq(token.as_bytes()));
        if equal && matched.is_none() {
            matched = Some(candidate);
        }
    }
    Ok(matched)
}

/// Builds `connection`'s credentials and processes `payload` on it.
async fn process_with_connection(
    state: &AppState,
    connection: &Connection,
    payload: &Map<String, Value>,
    received_at: i64,
) -> ApiResult<()> {
    let access_token = state
        .runtime_secret(&connection.access_token_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| {
            ApiError::Internal(anyhow::anyhow!(
                "GitLab connection {} has no stored access token",
                connection.id
            ))
        })?;
    let credentials = gitlab_credentials(&connection.base_url, access_token)?;
    process_job_hook(
        &state.database,
        state.gitlab.as_ref(),
        &connection.id,
        &credentials,
        payload,
        received_at,
    )
    .await
}

/// Applies one GitLab webhook payload, matched to connection `connection_id`
/// and received at `received_at`, to `gitlab_jobs` and the runners. Only
/// `object_kind: "build"` events have an effect; a status GitLab may add
/// later (`unknown`) writes nothing.
///
/// - `pending`: when the status rule would apply it, the job's own tags are
///   fetched with [`GitLabApi::job`] first (the payload carries only the
///   runner's), outside any transaction. A failed fetch fails the delivery
///   with nothing written.
/// - `running` with `runner.id`: records the runner. When this write is the
///   one that set the job's runner, that runner is marked busy.
/// - A conclusion that applies clears the busy flag of the runner recorded
///   on the job.
///
/// The job write, the busy change and its `runner_events` row are one
/// transaction: any failure rolls all three back, so `retry` can reapply the
/// delivery in full.
async fn process_job_hook(
    database: &SqlitePool,
    gitlab: &dyn GitLabApi,
    connection_id: &str,
    credentials: &GitLabCredentials,
    payload: &Map<String, Value>,
    received_at: i64,
) -> ApiResult<()> {
    if string(payload, "object_kind") != Some("build") {
        return Ok(());
    }
    let (Some(job_id), Some(project_id), Some(pipeline_id)) = (
        external_id(payload, "build_id"),
        external_id(payload, "project_id"),
        external_id(payload, "pipeline_id"),
    ) else {
        return Ok(());
    };
    let status = payload
        .get("build_status")
        .cloned()
        .and_then(|status| serde_json::from_value::<JobStatus>(status).ok())
        .unwrap_or(JobStatus::Unknown);
    if status == JobStatus::Unknown {
        return Ok(());
    }
    let status_changed_at = date_value(payload, "build_status_changed_at").unwrap_or(received_at);
    let runner_id = nested_object(payload, "runner").and_then(|runner| external_id(runner, "id"));

    let tags = if status == JobStatus::Pending
        && pending_applies(database, connection_id, job_id, status_changed_at).await?
    {
        let job = gitlab
            .job(credentials, project_id, job_id)
            .await
            .map_err(|error| {
                ApiError::Internal(anyhow::anyhow!(
                    "GitLab job {} lookup failed: {error}",
                    job_id.get()
                ))
            })?;
        Some(job.tag_list)
    } else {
        None
    };
    let observation = JobObservation {
        connection_id,
        project_id,
        job_id,
        pipeline_id,
        status,
        status_changed_at,
        retries_count: i64_value(payload, "retries_count")
            .and_then(|count| u32::try_from(count).ok()),
        tag_list: tags.as_deref(),
        gitlab_runner_id: if status == JobStatus::Running {
            runner_id
        } else {
            None
        },
        job_name: string(payload, "build_name"),
        project_path: nested_string(payload, "project", "path_with_namespace"),
    };

    let now = now_millis();
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let outcome = record_gitlab_job(&mut transaction, &observation, now).await?;
    let busy = match (outcome.gitlab_runner_id, &outcome.write) {
        // Whatever the write: this call set the running job's runner.
        (Some(runner), _)
            if outcome.status == "running" && outcome.previous_gitlab_runner_id.is_none() =>
        {
            Some((runner, true))
        }
        (Some(runner), JobWrite::StatusApplied) if status.is_terminal() => Some((runner, false)),
        _ => None,
    };
    if let Some((runner, busy)) = busy
        && let Some((runner_id, pool_id)) =
            set_gitlab_runner_busy(&mut transaction, connection_id, runner, busy, now).await?
    {
        sqlx::query(
            "INSERT INTO runner_events (id,runner_id,pool_id,level,event,message,metadata,created_at)
             VALUES (?,?,?,'info',?,?,?,?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&runner_id)
        .bind(&pool_id)
        .bind(format!("GitLab job {}", status.as_str()))
        .bind(format!(
            "{} · pipeline {}",
            string(payload, "build_name").unwrap_or("Job"),
            pipeline_id.get()
        ))
        .bind(
            json!({
                "jobId": job_id.get(),
                "projectId": project_id.get(),
                "pipelineId": pipeline_id.get(),
                "connectionId": connection_id,
            })
            .to_string(),
        )
        .bind(now)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

/// Whether a `pending` event at `at` would be applied to the stored job by
/// D18's status rule (always, when there is no row yet). The writer reads the
/// row again inside the transaction and decides on its own; this only spares
/// a tag lookup for an event that would be ignored.
async fn pending_applies(
    database: &SqlitePool,
    connection_id: &str,
    job_id: ExternalId,
    at: i64,
) -> ApiResult<bool> {
    let job_id = i64::try_from(job_id.get())
        .map_err(|error| ApiError::Internal(anyhow::anyhow!("GitLab job id: {error}")))?;
    let stored = sqlx::query(
        "SELECT status,status_changed_at FROM gitlab_jobs WHERE connection_id=? AND job_id=?",
    )
    .bind(connection_id)
    .bind(job_id)
    .fetch_optional(database)
    .await?;
    Ok(stored.is_none_or(|row| {
        gridops_core::gitlab::jobs::status_write_applies(
            row.get::<&str, _>("status"),
            row.get::<i64, _>("status_changed_at"),
            JobStatus::Pending,
            at,
        )
    }))
}

/// A GitLab id in the payload: a positive integer that fits SQLite.
fn external_id(object: &Map<String, Value>, key: &str) -> Option<ExternalId> {
    object
        .get(key)?
        .as_u64()
        .and_then(|value| ExternalId::new(value).ok())
}
