//! The built-in runner host: the machine `GridOps` itself runs on, where the
//! runner manager starts Docker runners through the Docker socket.
//!
//! Whether it may be used is one instance setting, stored with the other
//! settings. The API, the reconciler and the runner manager all consult it so
//! that turning it off is enforced where runners start, not only hidden in the
//! UI. Pools run on enrolled fleet machines instead.

use anyhow::Result;
use sqlx::SqlitePool;

use crate::now_millis;

/// Key in the `settings` table; the value is a JSON boolean.
pub const SETTING_KEY: &str = "builtinRunnerHostEnabled";

/// Error code for a request that would start a runner on the built-in host
/// while it is off. The runner manager, the API and the reconciler's events
/// all use it.
pub const DISABLED_CODE: &str = "builtin_runner_host_disabled";

/// What a refusal says.
pub const DISABLED_MESSAGE: &str = "The built-in runner host is turned off, so GridOps does not start runners on the machine it runs on. Run the pool on enrolled machines (Fleet execution), or turn the built-in runner host on in Settings > Runner host.";

/// `runner_pools.state` of a pool that could only run on the built-in host
/// while it is off.
pub const POOL_STATE: &str = "builtin-host-off";

/// What the pool shows while it is in [`POOL_STATE`].
pub const POOL_REASON: &str = "Cannot run: the built-in runner host is off and this pool has no fleet execution. Turn on Fleet execution in the pool's settings.";

/// A pool that uses only Docker and has no enabled, current fleet platform. It
/// has nowhere to run but the built-in host. A pool that also has another
/// provider (Tart) can still start those runners, so it is not counted.
/// Expects the pool table to be aliased `p`.
const DEPENDS_ON_HOST: &str = "
    NOT EXISTS (SELECT 1 FROM pool_execution_profiles x
                 WHERE x.pool_id=p.id AND x.enabled=1 AND x.archived_at IS NULL)
    AND p.state <> 'deleting'
    AND EXISTS (SELECT 1 FROM json_each(p.providers) j WHERE j.value='docker')
    AND NOT EXISTS (SELECT 1 FROM json_each(p.providers) j WHERE j.value<>'docker')";

/// Whether runners may be started on the built-in host. A missing setting
/// counts as off: the default for an install is decided when it is migrated.
pub async fn is_enabled(database: &SqlitePool) -> bool {
    sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
        .bind(SETTING_KEY)
        .fetch_optional(database)
        .await
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str::<bool>(&value).ok())
        .unwrap_or(false)
}

/// The setting before and after a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub before: bool,
    pub after: bool,
}

impl Change {
    #[must_use]
    pub const fn changed(self) -> bool {
        self.before != self.after
    }
}

/// Stores the setting and says what it was.
///
/// # Errors
/// Fails when the database cannot be read or written.
pub async fn set_enabled(database: &SqlitePool, user_id: &str, enabled: bool) -> Result<Change> {
    let before = is_enabled(database).await;
    sqlx::query(
        "INSERT INTO settings (key,value,updated_by,updated_at) VALUES (?,?,?,?)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_by=excluded.updated_by,updated_at=excluded.updated_at",
    )
    .bind(SETTING_KEY)
    .bind(serde_json::to_string(&enabled)?)
    .bind(user_id)
    .bind(now_millis())
    .execute(database)
    .await?;
    Ok(Change {
        before,
        after: enabled,
    })
}

/// Whether the pool has nowhere to run because the built-in host is off: it
/// uses only Docker and has no enabled fleet platform.
///
/// # Errors
/// Fails when the database cannot be read.
pub async fn pool_blocked(database: &SqlitePool, pool_id: &str) -> Result<bool> {
    if is_enabled(database).await {
        return Ok(false);
    }
    depends_on_host(database, pool_id).await
}

/// Whether the pool has no enabled fleet platform and uses only Docker, so
/// the built-in host is its only place to run, on or off.
///
/// # Errors
/// Fails when the database cannot be read.
pub async fn depends_on_host(database: &SqlitePool, pool_id: &str) -> Result<bool> {
    let sql =
        format!("SELECT EXISTS (SELECT 1 FROM runner_pools p WHERE p.id=? AND {DEPENDS_ON_HOST})");
    Ok(sqlx::query_scalar::<_, bool>(sqlx::AssertSqlSafe(sql))
        .bind(pool_id)
        .fetch_one(database)
        .await?)
}

/// Brings the stored state of pools in line with the setting: while the host
/// is off, a pool that has nowhere else to run is marked [`POOL_STATE`]; a pool
/// marked so that is no longer blocked is released to `active`, and the next
/// reconcile pass recomputes it. Paused and deleting pools are left alone.
/// Returns how many pools changed.
///
/// # Errors
/// Fails when the database cannot be written.
pub async fn sync_pool_states(database: &SqlitePool) -> Result<u64> {
    let enabled = is_enabled(database).await;
    let now = now_millis();
    let mut changed = 0;
    if !enabled {
        let sql = format!(
            "UPDATE runner_pools AS p SET state=?,updated_at=?
              WHERE p.paused=0 AND p.state NOT IN ('paused','deleting',?) AND {DEPENDS_ON_HOST}"
        );
        changed += sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(POOL_STATE)
            .bind(now)
            .bind(POOL_STATE)
            .execute(database)
            .await?
            .rows_affected();
    }
    let sql = if enabled {
        "UPDATE runner_pools AS p SET state='active',updated_at=? WHERE p.state=?".to_owned()
    } else {
        format!(
            "UPDATE runner_pools AS p SET state='active',updated_at=?
              WHERE p.state=? AND NOT ({DEPENDS_ON_HOST})"
        )
    };
    changed += sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(now)
        .bind(POOL_STATE)
        .execute(database)
        .await?
        .rows_affected();
    Ok(changed)
}

/// Ids of the pools that cannot run because the built-in host is off. Empty
/// while it is on.
///
/// # Errors
/// Fails when the database cannot be read.
pub async fn blocked_pool_ids(database: &SqlitePool) -> Result<std::collections::HashSet<String>> {
    if is_enabled(database).await {
        return Ok(std::collections::HashSet::new());
    }
    let sql = format!("SELECT p.id FROM runner_pools p WHERE {DEPENDS_ON_HOST}");
    Ok(sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql))
        .fetch_all(database)
        .await?
        .into_iter()
        .collect())
}

/// Live runners that have a container on the built-in host: what stays behind
/// when it is turned off, until each finishes its job.
///
/// # Errors
/// Fails when the database cannot be read.
pub async fn live_runner_count(database: &SqlitePool) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM runners
          WHERE deleted_at IS NULL AND container_id IS NOT NULL
            AND status IN ('starting','online','idle','busy','paused','stopped')",
    )
    .fetch_one(database)
    .await?)
}
