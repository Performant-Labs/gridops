# Evidence: #183 retire a GitLab connection's fleet targets on removal

Source facts in unchanged code that F's change and T's tests rely on.

- **Fact:** submit runs the target identity check, so a retired target (now `NotFound` there) can never get a new intent.
  **Source:** `crates/gridops-core/src/fleet/service.rs:249-250`
  **Verbatim excerpt:**
  > ensure_profile_and_target(tx, &request).await?;
  > ensure_target_identity(tx, &request).await?;

- **Fact:** the admission re-check and operation access both reach `ensure_current_intent_policy`, which runs the identity check first, so the one guard covers admission, placement and operation access.
  **Source:** `crates/gridops-core/src/fleet/service.rs:360-371` and `:414-420`
  **Verbatim excerpt:**
  > ensure_operation_access(tx, &principal, intent).await?;
  > ensure_current_intent_policy(tx, intent, true, &principal).await
  > }
  >
  > pub(crate) async fn ensure_operation_access(
  > ...
  > ensure_current_intent_policy(tx, intent, false, principal).await?;
  >
  > pub(crate) async fn ensure_current_intent_policy(
  > ...
  > ensure_target_identity_for_pool(tx, &intent.pool_id, &intent.target_id).await?;

- **Fact:** admission re-checks every intent's authorization through `recheck_intent_authorization` before admitting it.
  **Source:** `crates/gridops-core/src/fleet/admission.rs:233`
  **Verbatim excerpt:**
  > if let Err(error) = service::recheck_intent_authorization(tx, controller, &intent).await {

- **Fact:** a registry `NotFound` (what `set_host_grant` returns when its existence check fails) is answered with the `not_found` error code, so a grant on a retired target is a 404.
  **Source:** `crates/gridops-api/src/fleet/error.rs:78`
  **Verbatim excerpt:**
  > RegistryError::NotFound => ErrorCode::NotFound,

- **Fact:** the reconciler's GitLab fleet pass gets its target only through `gitlab_connection_target` (the key-path guard), then links it, so it can never place or link a retired target.
  **Source:** `crates/gridops-reconciler/src/gitlab_lifecycle.rs:548-552`
  **Verbatim excerpt:**
  > let target =
  >     gridops_core::fleet::targets::gitlab_connection_target(&app.database, &connection.id)
  >         .await?
  >         .with_context(|| format!("GitLab connection {} no longer exists", connection.name))?;
  > gridops_core::fleet::targets::link_pool_target(&app.database, &pool.id, &target).await?;

- **Fact:** the handler's transaction pattern (a `BEGIN IMMEDIATE` transaction passed into a core function that takes `&mut SqliteConnection`) already exists in the GitLab webhook.
  **Source:** `crates/gridops-api/src/webhooks/gitlab.rs:358-359`
  **Verbatim excerpt:**
  > let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
  > let outcome = record_gitlab_job(&mut transaction, &observation, now).await?;

- **Fact:** a fleet event may have no host and no placement, so `fleet.target.retired` with only `target_id` set is valid.
  **Source:** `migrations/0022_fleet_foundation.sql:393-401`
  **Verbatim excerpt:**
  > host_id TEXT REFERENCES fleet_hosts(id),
  > placement_id TEXT REFERENCES workload_placements(id),
  > target_id TEXT REFERENCES fleet_ci_targets(id),
  > actor_user_id TEXT REFERENCES users(id),
  > kind TEXT NOT NULL,
  > detail_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(detail_json) AND json_type(detail_json) = 'object'),
  > created_at INTEGER NOT NULL,
  > FOREIGN KEY (placement_id,host_id,target_id) REFERENCES workload_placements(id,host_id,target_id),
  > CHECK (placement_id IS NULL OR (host_id IS NOT NULL AND target_id IS NOT NULL))

- **Fact:** session grants carry `revision` and `revoked_at`, so the core function's session-grant revoke (with `revision+1`) is valid SQL.
  **Source:** `migrations/0022_fleet_foundation.sql:140-146`
  **Verbatim excerpt:**
  > CREATE TABLE host_session_target_grants (
  >   session_id TEXT NOT NULL REFERENCES host_sessions(id),
  >   target_id TEXT NOT NULL REFERENCES fleet_ci_targets(id),
  >   revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
  >   revoked_at INTEGER,
  >   PRIMARY KEY (session_id,target_id)
  > ) STRICT;

- **Fact:** an intent's `pool_id` is NOT NULL and references `runner_pools`, so the liveness query's inner `JOIN runner_pools` drops no live work.
  **Source:** `migrations/0022_fleet_foundation.sql:217`
  **Verbatim excerpt:**
  > pool_id TEXT NOT NULL REFERENCES runner_pools(id),

- **Fact:** the existing GitLab target insert does nothing on a taken canonical key, so a retired row's key is never re-inserted under a new id.
  **Source:** `crates/gridops-core/src/fleet/targets.rs:199-202` (post-change numbering; the statement is unchanged)
  **Verbatim excerpt:**
  > sqlx::query(
  >     "INSERT INTO fleet_ci_targets (id,kind,canonical_key,gitlab_connection_id,gitlab_project_id,gitlab_group_id)
  >      VALUES (?,?,?,?,?,?) ON CONFLICT(canonical_key) DO NOTHING",
  > )
