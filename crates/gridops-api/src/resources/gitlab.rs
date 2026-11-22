//! GitLab platform connections (#138): one GitLab project or group on one
//! GitLab instance, which runner pools with `scope: "gitlab"` target.
//!
//! Shaped like the Bitbucket connections in `resources.rs`, with the GitLab
//! contract of #137: every call goes through the shared `Arc<dyn GitLabApi>`
//! in [`AppState`] with the connection's own [`GitLabCredentials`], built per
//! call from its stored base URL and its decrypted token.
//!
//! Secrets: the access token and the webhook secret are sealed into
//! `runtime_secrets` under the connection row's `access_token_key` and
//! `webhook_secret_key`. Only the create and rotate responses carry the
//! webhook secret, and no response, log line or audit event carries the
//! token.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use gridops_core::{
    fleet::{
        ExternalId,
        targets::{ConnectionTargetsRetirement, retire_gitlab_connection_targets},
    },
    gitlab::{GitLabCredentials, GitLabError, Scope, ScopeKind},
    now_millis,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row as _;
use url::Url;

use super::{RunnerAccess, SameOrigin, iso, public_webhook_url};
use crate::{
    auth::{AuthUser, audit, random_token, require_system_admin},
    error::{ApiError, ApiResult},
    state::AppState,
};

/// Where GitLab delivers webhooks (#140 serves it), relative to the public
/// base URL. It has no override setting.
const GITLAB_WEBHOOK_PATH: &str = "/api/v1/webhooks/gitlab";
const DEFAULT_BASE_URL: &str = "https://gitlab.com";
/// Random bytes in a webhook secret (43 characters once encoded).
const WEBHOOK_SECRET_BYTES: usize = 32;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGitlabConnection {
    // Defaulted so a missing field is the same 400 as a blank one rather
    // than axum's 422.
    #[serde(default)]
    name: String,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    scope_kind: String,
    #[serde(default)]
    scope_path: String,
    #[serde(default)]
    access_token: String,
}

/// A partial update: a field left out is unchanged.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGitlabConnection {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
}

/// A stored connection, without its secrets.
struct StoredConnection {
    name: String,
    base_url: String,
    scope: Scope,
    access_token_key: String,
    webhook_secret_key: String,
}

const CONNECTION_COLUMNS: &str =
    "id,name,base_url,scope_kind,scope_id,scope_path,created_at,updated_at";

pub async fn gitlab_connections(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<Value>> {
    let can_manage = user.role == "admin";
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {CONNECTION_COLUMNS} FROM gitlab_connections ORDER BY lower(name),id"
    )))
    .fetch_all(&state.database)
    .await?;
    Ok(Json(json!({
        "canManage": can_manage,
        "items": rows
            .iter()
            .map(|row| connection_json(row, can_manage))
            .collect::<Vec<_>>(),
    })))
}

pub async fn create_gitlab_connection(
    State(state): State<AppState>,
    _same_origin: SameOrigin,
    user: AuthUser,
    Json(input): Json<CreateGitlabConnection>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    require_system_admin(&user)?;
    let name = connection_name(&input.name)?;
    let base_url = normalised_base_url(input.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL))?;
    let scope_kind = scope_kind(&input.scope_kind)?;
    let scope_path = scope_path(&input.scope_path)?;
    let access_token = access_token(&input.access_token)?;

    let credentials = credentials(&base_url, access_token.to_owned())?;
    let scope = state
        .gitlab
        .resolve_scope(&credentials, scope_kind, scope_path)
        .await
        .map_err(|error| gitlab_refusal(&error, scope_path))?;
    // Each project or group is connected once per instance; checking before
    // the access check names the connection to edit instead.
    if let Some(existing) = sqlx::query_scalar::<_, String>(
        "SELECT name FROM gitlab_connections WHERE base_url=? AND scope_kind=? AND scope_id=?",
    )
    .bind(&base_url)
    .bind(scope.kind().as_str())
    .bind(scope_id(&scope)?)
    .fetch_optional(&state.database)
    .await?
    {
        return Err(duplicate_connection(&base_url, scope.path(), &existing));
    }
    state
        .gitlab
        .verify_access(&credentials, &scope)
        .await
        .map_err(|error| gitlab_refusal(&error, scope.path()))?;

    let id = uuid::Uuid::new_v4().to_string();
    let access_token_key = format!("gitlab.connection.{id}.access_token");
    let webhook_secret_key = format!("gitlab.connection.{id}.webhook_secret");
    let webhook_secret = random_token(WEBHOOK_SECRET_BYTES);
    let sealed_token = state.vault.seal(access_token).map_err(ApiError::Internal)?;
    let sealed_secret = state
        .vault
        .seal(&webhook_secret)
        .map_err(ApiError::Internal)?;
    let now = now_millis();
    let mut transaction = state.database.begin().await?;
    let result = async {
        for (key, sealed) in [
            (&access_token_key, &sealed_token),
            (&webhook_secret_key, &sealed_secret),
        ] {
            sqlx::query(
                "INSERT INTO runtime_secrets (key,value,updated_by,updated_at) VALUES (?,?,?,?)",
            )
            .bind(key)
            .bind(sealed)
            .bind(&user.id)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            r#"INSERT INTO gitlab_connections
              (id,name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key,created_by,created_at,updated_at)
              VALUES (?,?,?,?,?,?,?,?,?,?,?)"#,
        )
        .bind(&id)
        .bind(name)
        .bind(&base_url)
        .bind(scope.kind().as_str())
        .bind(scope_id(&scope)?)
        .bind(scope.path())
        .bind(&access_token_key)
        .bind(&webhook_secret_key)
        .bind(&user.id)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        Ok::<(), ApiError>(())
    }
    .await;
    if let Err(error) = result {
        transaction.rollback().await?;
        if is_unique_violation(&error) {
            return Err(duplicate_connection(
                &base_url,
                scope.path(),
                "another connection",
            ));
        }
        return Err(error);
    }
    transaction.commit().await?;
    audit(
        &state,
        &user,
        "gitlab.connection_created",
        "gitlab_connection",
        Some(&id),
        json!({
            "name": name,
            "baseUrl": base_url,
            "scopeKind": scope.kind().as_str(),
            "scopeId": scope.id().get(),
            "scopePath": scope.path(),
        }),
    )
    .await?;
    let row = connection_row(&state, &id).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "connection": connection_json(&row, true),
            "webhookUrl": webhook_url(&state),
            "webhookSecret": webhook_secret,
        })),
    ))
}

pub async fn update_gitlab_connection(
    State(state): State<AppState>,
    _same_origin: SameOrigin,
    user: AuthUser,
    Path(connection_id): Path<String>,
    Json(input): Json<UpdateGitlabConnection>,
) -> ApiResult<Json<Value>> {
    require_system_admin(&user)?;
    let name = input.name.as_deref().map(connection_name).transpose()?;
    let replacement = input
        .access_token
        .as_deref()
        .map(access_token)
        .transpose()?;
    let connection = stored_connection(&state, &connection_id).await?;
    if name.is_none() && replacement.is_none() {
        let row = connection_row(&state, &connection_id).await?;
        return Ok(Json(connection_json(&row, true)));
    }
    // A new token is checked against the stored scope before it is stored:
    // the scope itself never changes, so it is not resolved again.
    let sealed = match replacement {
        Some(token) => {
            let credentials = credentials(&connection.base_url, token.to_owned())?;
            state
                .gitlab
                .verify_access(&credentials, &connection.scope)
                .await
                .map_err(|error| gitlab_refusal(&error, connection.scope.path()))?;
            Some(state.vault.seal(token).map_err(ApiError::Internal)?)
        }
        None => None,
    };
    let now = now_millis();
    let mut transaction = state.database.begin().await?;
    if let Some(sealed) = &sealed {
        store_secret(
            &mut transaction,
            &connection.access_token_key,
            sealed,
            &user.id,
            now,
        )
        .await?;
    }
    sqlx::query("UPDATE gitlab_connections SET name=?,updated_at=? WHERE id=?")
        .bind(name.unwrap_or(&connection.name))
        .bind(now)
        .bind(&connection_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    audit(
        &state,
        &user,
        "gitlab.connection_updated",
        "gitlab_connection",
        Some(&connection_id),
        json!({
            "name": name.unwrap_or(&connection.name),
            "previousName": connection.name,
            "tokenReplaced": sealed.is_some(),
        }),
    )
    .await?;
    let row = connection_row(&state, &connection_id).await?;
    Ok(Json(connection_json(&row, true)))
}

pub async fn verify_gitlab_connection(
    State(state): State<AppState>,
    _same_origin: SameOrigin,
    user: AuthUser,
    Path(connection_id): Path<String>,
) -> ApiResult<StatusCode> {
    require_system_admin(&user)?;
    let connection = stored_connection(&state, &connection_id).await?;
    let token = state
        .runtime_secret(&connection.access_token_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| {
            ApiError::BadRequest(
                "GridOps has no access token for this GitLab connection. Replace the token.".into(),
            )
        })?;
    let credentials = credentials(&connection.base_url, token)?;
    state
        .gitlab
        .verify_access(&credentials, &connection.scope)
        .await
        .map_err(|error| gitlab_refusal(&error, connection.scope.path()))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn rotate_gitlab_webhook_secret(
    State(state): State<AppState>,
    _same_origin: SameOrigin,
    user: AuthUser,
    Path(connection_id): Path<String>,
) -> ApiResult<Json<Value>> {
    require_system_admin(&user)?;
    let connection = stored_connection(&state, &connection_id).await?;
    let webhook_secret = random_token(WEBHOOK_SECRET_BYTES);
    let sealed = state
        .vault
        .seal(&webhook_secret)
        .map_err(ApiError::Internal)?;
    let now = now_millis();
    let mut transaction = state.database.begin().await?;
    // Overwritten under the same key (#140 reads it there), so the previous
    // secret stops working at once.
    store_secret(
        &mut transaction,
        &connection.webhook_secret_key,
        &sealed,
        &user.id,
        now,
    )
    .await?;
    sqlx::query("UPDATE gitlab_connections SET updated_at=? WHERE id=?")
        .bind(now)
        .bind(&connection_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    audit(
        &state,
        &user,
        "gitlab.webhook_secret_rotated",
        "gitlab_connection",
        Some(&connection_id),
        json!({ "name": connection.name }),
    )
    .await?;
    Ok(Json(json!({
        "webhookUrl": webhook_url(&state),
        "webhookSecret": webhook_secret,
    })))
}

pub async fn delete_gitlab_connection(
    State(state): State<AppState>,
    _same_origin: SameOrigin,
    user: AuthUser,
    Path(connection_id): Path<String>,
) -> ApiResult<StatusCode> {
    require_system_admin(&user)?;
    let connection = stored_connection(&state, &connection_id).await?;
    let name = &connection.name;
    let now = now_millis();
    let mut transaction = state.database.begin_with("BEGIN IMMEDIATE").await?;
    // Checked in the transaction that deletes, so nothing starts referencing
    // the connection in between. Pool memberships would cascade, a live
    // runner needs this connection's token to be removed from GitLab, and
    // work on its fleet targets must finish first. Its fleet targets are then
    // retired (#183): their foreign key is NO ACTION, and fleet history keeps
    // referencing them.
    let pools = sqlx::query_scalar::<_, String>(
        r#"SELECT pool.name FROM runner_pool_gitlab_connections membership
          JOIN runner_pools pool ON pool.id=membership.pool_id
          WHERE membership.connection_id=? ORDER BY lower(pool.name)"#,
    )
    .bind(&connection_id)
    .fetch_all(&mut *transaction)
    .await?;
    if !pools.is_empty() {
        return Err(ApiError::Conflict(format!(
            "{name} is used by {} {}. Remove it from {} first.",
            if pools.len() == 1 { "pool" } else { "pools" },
            pools.join(", "),
            if pools.len() == 1 {
                "that pool"
            } else {
                "those pools"
            },
        )));
    }
    let runners = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM runners WHERE gitlab_connection_id=? AND deleted_at IS NULL",
    )
    .bind(&connection_id)
    .fetch_one(&mut *transaction)
    .await?;
    if runners > 0 {
        return Err(ApiError::Conflict(format!(
            "{runners} runner{} registered through {name} still exist. Remove {} first.",
            if runners == 1 { "" } else { "s" },
            if runners == 1 { "it" } else { "them" },
        )));
    }
    let retired = match retire_gitlab_connection_targets(
        &mut transaction,
        &connection_id,
        &user.id,
        now,
    )
    .await
    .map_err(|error| ApiError::Internal(error.into()))?
    {
        ConnectionTargetsRetirement::Retired(retired) => retired,
        ConnectionTargetsRetirement::LiveWork(pools) => {
            return Err(ApiError::Conflict(format!(
                "Fleet work of {} {} still runs or waits on {name}. Stop it or let it finish first.",
                if pools.len() == 1 { "pool" } else { "pools" },
                pools.join(", "),
            )));
        }
    };
    sqlx::query("DELETE FROM gitlab_connections WHERE id=?")
        .bind(&connection_id)
        .execute(&mut *transaction)
        .await?;
    for key in [&connection.access_token_key, &connection.webhook_secret_key] {
        sqlx::query("DELETE FROM runtime_secrets WHERE key=?")
            .bind(key)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    audit(
        &state,
        &user,
        "gitlab.connection_deleted",
        "gitlab_connection",
        Some(&connection_id),
        json!({
            "name": name,
            "baseUrl": connection.base_url,
            "scopePath": connection.scope.path(),
            "retiredTargetIds": retired.target_ids,
            "hostGrantsRevoked": retired.host_grants_revoked,
            "sessionGrantsRevoked": retired.session_grants_revoked,
            "profileLinksRevoked": retired.profile_links_revoked,
        }),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Runner pools with GitLab targets (called from the pool handlers)
// ---------------------------------------------------------------------------

/// The requested GitLab connections, sorted and deduplicated, once each is
/// known to exist. GitLab has no user grants yet (D11), so a pool with GitLab
/// targets is a system administrator's to create or change.
pub(super) async fn selected_gitlab_connections(
    state: &AppState,
    user: &AuthUser,
    connection_ids: &[String],
) -> ApiResult<Vec<String>> {
    if connection_ids.is_empty() {
        return Ok(Vec::new());
    }
    require_system_admin(user)?;
    let mut requested = connection_ids.to_vec();
    requested.sort();
    requested.dedup();
    let mut selected = Vec::with_capacity(requested.len());
    for connection_id in &requested {
        if let Some(connection) =
            sqlx::query_scalar::<_, String>("SELECT id FROM gitlab_connections WHERE id=?")
                .bind(connection_id)
                .fetch_optional(&state.database)
                .await?
        {
            selected.push(connection);
        }
    }
    if selected.len() != requested.len() {
        return Err(ApiError::BadRequest(
            "One or more GitLab connections do not exist.".into(),
        ));
    }
    Ok(selected)
}

/// The connection ids a pool targets now, in the order they were added.
pub(super) async fn pool_gitlab_connection_ids(
    state: &AppState,
    pool_id: &str,
) -> ApiResult<Vec<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT connection_id FROM runner_pool_gitlab_connections WHERE pool_id=? ORDER BY created_at,connection_id",
    )
    .bind(pool_id)
    .fetch_all(&state.database)
    .await?)
}

/// A pool's GitLab connections as [`pool_connection_json`] reads them, with
/// the membership's pool id. The pool detail and the pool list both select
/// through this and order by [`POOL_CONNECTION_ORDER`], so the two show the
/// same connections in the same order.
const POOL_CONNECTIONS: &str = "SELECT membership.pool_id,connection.id,connection.name,connection.base_url,connection.scope_kind,connection.scope_path
   FROM runner_pool_gitlab_connections membership
   JOIN gitlab_connections connection ON connection.id=membership.connection_id";
/// The order a pool's connections were added in.
const POOL_CONNECTION_ORDER: &str = "membership.created_at,connection.id";

/// The GitLab connections a pool targets, as the pool detail shows them.
pub(super) async fn pool_gitlab_connections(
    state: &AppState,
    pool_id: &str,
) -> ApiResult<Vec<Value>> {
    Ok(sqlx::query(sqlx::AssertSqlSafe(format!(
        "{POOL_CONNECTIONS} WHERE membership.pool_id=? ORDER BY {POOL_CONNECTION_ORDER}"
    )))
    .bind(pool_id)
    .fetch_all(&state.database)
    .await?
    .iter()
    .map(pool_connection_json)
    .collect())
}

/// The GitLab connections of each pool in `pool_ids`, by pool id, as the pool
/// detail shows them: one query for a whole page of the pool list. A pool
/// with none has no entry.
pub(super) async fn pools_gitlab_connections(
    state: &AppState,
    pool_ids: &[String],
) -> ApiResult<HashMap<String, Vec<Value>>> {
    let mut connections = HashMap::<String, Vec<Value>>::new();
    if pool_ids.is_empty() {
        return Ok(connections);
    }
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "{POOL_CONNECTIONS} WHERE membership.pool_id IN (SELECT value FROM json_each(?))
         ORDER BY membership.pool_id,{POOL_CONNECTION_ORDER}"
    )))
    .bind(json!(pool_ids).to_string())
    .fetch_all(&state.database)
    .await?;
    for row in &rows {
        connections
            .entry(row.get("pool_id"))
            .or_default()
            .push(pool_connection_json(row));
    }
    Ok(connections)
}

/// Every GitLab connection, as the pool form offers them.
pub(super) async fn gitlab_connection_options(state: &AppState) -> ApiResult<Vec<Value>> {
    Ok(sqlx::query(
        "SELECT id,name,base_url,scope_kind,scope_path FROM gitlab_connections ORDER BY lower(name),id",
    )
    .fetch_all(&state.database)
    .await?
    .iter()
    .map(pool_connection_json)
    .collect())
}

/// Whether a pool serves GitLab: it has GitLab targets, or its scope says it
/// does (a GitLab pool can't lose its last target, so these agree).
pub(super) async fn pool_has_gitlab_targets(state: &AppState, pool_id: &str) -> ApiResult<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM runner_pool_gitlab_connections WHERE pool_id=?)
             OR EXISTS (SELECT 1 FROM runner_pools WHERE id=? AND scope='gitlab')",
    )
    .bind(pool_id)
    .bind(pool_id)
    .fetch_one(&state.database)
    .await?)
}

fn pool_connection_json(row: &sqlx::sqlite::SqliteRow) -> Value {
    json!({
        "id": row.get::<String, _>("id"),
        "name": row.get::<String, _>("name"),
        "baseUrl": row.get::<String, _>("base_url"),
        "scopeKind": row.get::<String, _>("scope_kind"),
        "scopePath": row.get::<String, _>("scope_path"),
    })
}

// ---------------------------------------------------------------------------
// Runner delete: the GitLab arm (D17)
// ---------------------------------------------------------------------------

/// Removes a GitLab runner's registration at its GitLab instance. GitLab
/// not knowing the runner counts as removed. This never fails a delete:
/// when the runner can't be removed (no connection, no GitLab runner id, no
/// readable token, or GitLab refuses or is unreachable) it logs a warning
/// and returns the reason, and #139's orphan sweep removes the runner later.
pub(super) async fn cleanup_gitlab_runner(
    state: &AppState,
    runner: &RunnerAccess,
) -> Option<String> {
    let reason = match remove_gitlab_runner(state, runner).await {
        Ok(()) => return None,
        Err(reason) => reason,
    };
    tracing::warn!(
        runner_id = %runner.runner_id,
        reason = %reason,
        "GitLab runner left for the orphan sweep"
    );
    Some(reason)
}

async fn remove_gitlab_runner(state: &AppState, runner: &RunnerAccess) -> Result<(), String> {
    let connection_id = runner
        .gitlab_connection_id
        .as_deref()
        .ok_or("The runner's GitLab connection no longer exists.")?;
    let gitlab_runner_id = runner
        .gitlab_runner_id
        .and_then(|id| u64::try_from(id).ok())
        .and_then(|id| ExternalId::new(id).ok())
        .ok_or("The runner has no GitLab runner id.")?;
    let connection =
        sqlx::query("SELECT base_url,access_token_key FROM gitlab_connections WHERE id=?")
            .bind(connection_id)
            .fetch_optional(&state.database)
            .await
            .map_err(|error| {
                tracing::warn!(error = ?error, "could not load a GitLab connection");
                "GitLab connection could not be loaded.".to_owned()
            })?
            .ok_or("The runner's GitLab connection no longer exists.")?;
    let token = state
        .runtime_secret(connection.get::<&str, _>("access_token_key"))
        .await
        .map_err(|error| {
            tracing::warn!(error = ?error, "could not read a GitLab access token");
            "The GitLab access token could not be read.".to_owned()
        })?
        .ok_or("The GitLab access token is missing.")?;
    let credentials = credentials(connection.get::<&str, _>("base_url"), token)
        .map_err(|_| "The GitLab connection's base URL is invalid.".to_owned())?;
    match state
        .gitlab
        .delete_runner(&credentials, gitlab_runner_id)
        .await
    {
        Ok(()) | Err(GitLabError::NotFound) => Ok(()),
        Err(error) => Err(gitlab_message(&error, "the runner")),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn stored_connection(state: &AppState, connection_id: &str) -> ApiResult<StoredConnection> {
    let row = sqlx::query(
        "SELECT name,base_url,scope_kind,scope_id,scope_path,access_token_key,webhook_secret_key FROM gitlab_connections WHERE id=?",
    )
    .bind(connection_id)
    .fetch_optional(&state.database)
    .await?
    .ok_or_else(not_found)?;
    let id = u64::try_from(row.get::<i64, _>("scope_id"))
        .ok()
        .and_then(|id| ExternalId::new(id).ok())
        .ok_or_else(|| {
            ApiError::Internal(anyhow::anyhow!(
                "GitLab connection {connection_id} has an invalid scope id"
            ))
        })?;
    let path = row.get::<String, _>("scope_path");
    let scope = match row.get::<&str, _>("scope_kind") {
        "project" => Scope::Project { id, path },
        "group" => Scope::Group { id, path },
        other => {
            return Err(ApiError::Internal(anyhow::anyhow!(
                "GitLab connection {connection_id} has an unknown scope kind {other:?}"
            )));
        }
    };
    Ok(StoredConnection {
        name: row.get("name"),
        base_url: row.get("base_url"),
        scope,
        access_token_key: row.get("access_token_key"),
        webhook_secret_key: row.get("webhook_secret_key"),
    })
}

async fn connection_row(
    state: &AppState,
    connection_id: &str,
) -> ApiResult<sqlx::sqlite::SqliteRow> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {CONNECTION_COLUMNS} FROM gitlab_connections WHERE id=?"
    )))
    .bind(connection_id)
    .fetch_optional(&state.database)
    .await?
    .ok_or_else(not_found)
}

async fn store_secret(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    key: &str,
    sealed: &str,
    user_id: &str,
    now: i64,
) -> ApiResult<()> {
    sqlx::query(
        r#"INSERT INTO runtime_secrets (key,value,updated_by,updated_at) VALUES (?,?,?,?)
           ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_by=excluded.updated_by,updated_at=excluded.updated_at"#,
    )
    .bind(key)
    .bind(sealed)
    .bind(user_id)
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// The `GitlabConnection` schema: never the token, the webhook secret or the
/// webhook URL.
fn connection_json(row: &sqlx::sqlite::SqliteRow, can_manage: bool) -> Value {
    json!({
        "id": row.get::<String, _>("id"),
        "provider": "gitlab",
        "name": row.get::<String, _>("name"),
        "baseUrl": row.get::<String, _>("base_url"),
        "scopeKind": row.get::<String, _>("scope_kind"),
        "scopeId": row.get::<i64, _>("scope_id"),
        "scopePath": row.get::<String, _>("scope_path"),
        "createdAt": iso(row.get::<i64, _>("created_at")),
        "updatedAt": iso(row.get::<i64, _>("updated_at")),
        "canManage": can_manage,
    })
}

fn webhook_url(state: &AppState) -> String {
    public_webhook_url(state.config.base_url(), GITLAB_WEBHOOK_PATH)
}

/// The credentials for one call. The stored base URL has no trailing slash;
/// the client gets one, so API paths join beneath an instance served under a
/// path prefix.
pub(crate) fn credentials(base_url: &str, access_token: String) -> ApiResult<GitLabCredentials> {
    let base_url = Url::parse(&format!("{base_url}/")).map_err(|error| {
        ApiError::Internal(anyhow::anyhow!(
            "stored GitLab base URL is invalid: {error}"
        ))
    })?;
    Ok(GitLabCredentials {
        base_url,
        access_token: SecretString::from(access_token),
    })
}

fn scope_id(scope: &Scope) -> ApiResult<i64> {
    i64::try_from(scope.id().get())
        .map_err(|_| ApiError::BadRequest("GitLab returned an id GridOps can't store.".into()))
}

fn not_found() -> ApiError {
    ApiError::NotFound("That GitLab connection does not exist.".into())
}

fn duplicate_connection(base_url: &str, path: &str, existing: &str) -> ApiError {
    ApiError::Conflict(format!(
        "{base_url}/{path} is already connected as {existing}. Edit that connection to rename it or replace its token."
    ))
}

fn is_unique_violation(error: &ApiError) -> bool {
    matches!(error, ApiError::Internal(cause) if cause
        .downcast_ref::<sqlx::Error>()
        .and_then(sqlx::Error::as_database_error)
        .is_some_and(|database| database.is_unique_violation()))
}

/// A GitLab refusal as the 400 the spec declares. `path` is the project or
/// group the call was about.
fn gitlab_refusal(error: &GitLabError, path: &str) -> ApiError {
    ApiError::BadRequest(gitlab_message(error, path))
}

/// What a client may be told about a GitLab error. The upstream detail of a
/// failed request stays in the log; no message carries the token.
fn gitlab_message(error: &GitLabError, path: &str) -> String {
    match error {
        GitLabError::Unauthorized => "GitLab rejected the access token.".into(),
        GitLabError::MissingScope(scope) => {
            format!("The GitLab access token is missing the {scope} scope.")
        }
        GitLabError::MissingRole => "The GitLab access token's user lacks the Maintainer (or Owner) role on this project or group.".into(),
        GitLabError::NotFound => format!("GitLab could not find {path}, or the token can't see it."),
        GitLabError::RateLimited { .. } => {
            "GitLab is rate limiting requests. Try again later.".into()
        }
        GitLabError::Upstream(detail) => {
            tracing::warn!(error = %detail, "GitLab request failed");
            "GitLab request failed".into()
        }
    }
}

fn connection_name(value: &str) -> ApiResult<&str> {
    let name = value.trim();
    if !(1..=80).contains(&name.chars().count()) || name.chars().any(char::is_control) {
        return Err(ApiError::BadRequest(
            "GitLab connection name must contain 1-80 visible characters.".into(),
        ));
    }
    Ok(name)
}

fn access_token(value: &str) -> ApiResult<&str> {
    let token = value.trim();
    if !(1..=4_096).contains(&token.len()) || token.chars().any(char::is_control) {
        return Err(ApiError::BadRequest(
            "GitLab access token is invalid.".into(),
        ));
    }
    Ok(token)
}

fn scope_kind(value: &str) -> ApiResult<ScopeKind> {
    match value {
        "project" => Ok(ScopeKind::Project),
        "group" => Ok(ScopeKind::Group),
        _ => Err(ApiError::BadRequest(
            "GitLab scopeKind must be project or group.".into(),
        )),
    }
}

fn scope_path(value: &str) -> ApiResult<&str> {
    let path = value.trim().trim_matches('/');
    if path.is_empty()
        || path.len() > 1_024
        || path
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(ApiError::BadRequest(
            "GitLab scopePath must be a project or group path such as group/project.".into(),
        ));
    }
    Ok(path)
}

/// The stored form of a GitLab base URL: scheme, host, optional port and
/// path, no trailing slash. Anything but http(s) with a host is refused, as
/// are credentials, a query or a fragment in the URL.
fn normalised_base_url(value: &str) -> ApiResult<String> {
    let invalid = || {
        ApiError::BadRequest(
            "GitLab baseUrl must be an http(s) URL with a host, such as https://gitlab.com.".into(),
        )
    };
    let value = value.trim();
    if value.len() > 2_048 {
        return Err(invalid());
    }
    let url = Url::parse(value).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let host = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or_else(invalid)?;
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Ok(format!(
        "{}://{host}{port}{}",
        url.scheme(),
        url.path().trim_end_matches('/')
    ))
}
