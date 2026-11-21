//! The GitLab runner lifecycle (#139): the GitLab counterparts of
//! `next_bitbucket_connection`, `bitbucket_runner_labels` and
//! `deregister_bitbucket_runner`. The shared start, follow and delete paths in
//! `main.rs` and `fleet.rs` call into these helpers; this module is not a
//! pipeline of its own.
//!
//! - Register: a GitLab pool's runner is created on one of the pool's GitLab
//!   connections with the pool's labels as tags, its `run_untagged`, and a
//!   description naming this deployment and the runner row
//!   (`gridops-runner:<instance>:<runner uuid>`).
//! - Deregister: whenever a GitLab runner row leaves service. A failed delete
//!   is logged and never fails a pass; the orphan sweep retries it. A delete
//!   GitLab itself refuses (other than a runner it no longer has) is also a
//!   warning on the runner's pool (#171), so an operator sees it.
//! - Outcomes: a runner that completed its job or idled out proves the
//!   registration works and resets the pool's provisioning failures; one
//!   GitLab refused counts one failure. Either happens once per runner, when
//!   its row first leaves service.
//! - Sweep: GitLab runners this deployment marked whose runner row is gone.
//!   A delete GitLab refuses is a warning on each pool linked to the
//!   connection, at most once an hour per pool, connection and reason.
//!
//! The `glrt-` runner token goes to the runner manager's start request or the
//! fleet's sealed bootstrap material only: never to a row, event or log.

use anyhow::{Context as _, Result, anyhow};
use gridops_core::{
    fleet::{
        ExternalId,
        failure::PlacementFailure,
        placement::{self, Placement},
        protocol::{agent::GitlabRunnerMaterial, outcomes::DiagnosticCode},
    },
    gitlab::{CreatedRunner, GitLabCredentials, GitLabError, RunnerSummary, Scope, ScopeKind},
    now_millis,
    provisioning::{
        GITLAB_DEFAULT_WAIT_TIMEOUT_SECS, GITLAB_EXIT_COMPLETED, GITLAB_EXIT_IDLE_EXPIRED,
        GITLAB_EXIT_REASONS, GITLAB_EXIT_REGISTRATION_REFUSED,
    },
};
use reqwest::Url;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use sqlx::FromRow;

use super::{
    Reconciler, Runner, clear_provision_failures, load_pool, record_provision_failure,
    runner_event, runtime_secret, setting_i64, system_event,
};

/// The start of the description of every GitLab runner `GridOps` creates.
const MARKER_PREFIX: &str = "gridops-runner:";

/// How often the orphan sweep runs by default, and its bounds, in seconds.
const SWEEP_INTERVAL_DEFAULT_SECS: i64 = 600;
const SWEEP_INTERVAL_MIN_SECS: i64 = 60;
const SWEEP_INTERVAL_MAX_SECS: i64 = 3_600;

/// The warning a GitLab runner deletion that GitLab refused records on the
/// pool's events (#171).
const DELETION_REFUSED_EVENT: &str = "GitLab runner not deleted";

/// How long one orphan-sweep warning stands for its pool, connection and
/// reason, in milliseconds: the sweep writes no other in that time.
const SWEEP_WARNING_WINDOW_MS: i64 = 3_600_000;

/// A GitLab connection a pool registers runners on.
#[derive(Debug, Clone, FromRow)]
pub(super) struct GitlabConnection {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) base_url: String,
    scope_kind: String,
    scope_id: i64,
    scope_path: String,
    access_token_key: String,
}

impl GitlabConnection {
    fn scope(&self) -> Result<Scope> {
        let id = ExternalId::new(u64::try_from(self.scope_id)?)?;
        let path = self.scope_path.clone();
        match self.scope_kind.as_str() {
            kind if kind == ScopeKind::Project.as_str() => Ok(Scope::Project { id, path }),
            kind if kind == ScopeKind::Group.as_str() => Ok(Scope::Group { id, path }),
            kind => Err(anyhow!(
                "GitLab connection {} has an unknown scope kind {kind}",
                self.name
            )),
        }
    }

    /// The connection's instance and its access token, from the vault.
    async fn credentials(&self, app: &Reconciler) -> Result<GitLabCredentials> {
        let base_url = Url::parse(&self.base_url)
            .with_context(|| format!("GitLab connection {} has an invalid URL", self.name))?;
        let access_token = runtime_secret(app, &self.access_token_key)
            .await?
            .with_context(|| format!("GitLab connection {} has no access token", self.name))?;
        Ok(GitLabCredentials {
            base_url,
            access_token: SecretString::from(access_token),
        })
    }
}

/// The connection's columns, for the `GitlabConnection` row.
macro_rules! connection_columns {
    () => {
        "connection.id,connection.name,connection.base_url,connection.scope_kind,connection.scope_id,connection.scope_path,connection.access_token_key"
    };
}

/// The connection the pool's next GitLab runner registers on: the member with
/// the fewest live GitLab runners of the pool, then the earliest membership.
/// GitLab runners are ephemeral (one job each), so a connection is not
/// limited to one live runner as a Bitbucket one is. `None` when the pool has
/// no GitLab connection, so it is not a GitLab pool.
pub(super) async fn next_connection(
    app: &Reconciler,
    pool_id: &str,
) -> Result<Option<GitlabConnection>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        connection_columns!(),
        " FROM runner_pool_gitlab_connections membership
           JOIN gitlab_connections connection ON connection.id=membership.connection_id
          WHERE membership.pool_id=?
          ORDER BY (SELECT COUNT(*) FROM runners runner
                     WHERE runner.pool_id=membership.pool_id
                       AND runner.ci_platform='gitlab'
                       AND runner.gitlab_connection_id=connection.id
                       AND runner.deleted_at IS NULL
                       AND runner.status IN ('starting','online','idle','busy','paused','stopped')),
                   membership.created_at,connection.id
          LIMIT 1"
    ))
    .bind(pool_id)
    .fetch_optional(&app.database)
    .await?)
}

async fn connection(app: &Reconciler, connection_id: &str) -> Result<Option<GitlabConnection>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        connection_columns!(),
        " FROM gitlab_connections connection WHERE connection.id=?"
    ))
    .bind(connection_id)
    .fetch_optional(&app.database)
    .await?)
}

/// What an operator is told about a connection whose name is no longer known.
async fn connection_name(app: &Reconciler, connection_id: Option<&str>) -> Result<String> {
    let name = match connection_id {
        Some(id) => connection(app, id).await?.map(|connection| connection.name),
        None => None,
    };
    Ok(name.unwrap_or_else(|| "(a removed GitLab connection)".to_owned()))
}

/// This deployment as runner descriptions name it: the host of
/// `GRIDOPS_BASE_URL`, and its port when it is not the scheme's default.
fn instance(base_url: &Url) -> String {
    let host = base_url.host_str().unwrap_or_default();
    match base_url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    }
}

/// The description of the GitLab runner of runner row `runner_id`.
fn marker(app: &Reconciler, runner_id: &str) -> String {
    format!(
        "{MARKER_PREFIX}{}:{runner_id}",
        instance(app.config.base_url())
    )
}

/// The instance and runner row a `gridops-runner:` description names; `None`
/// for any other description.
fn parse_marker(description: &str) -> Option<(&str, uuid::Uuid)> {
    let (instance, runner_id) = description.strip_prefix(MARKER_PREFIX)?.rsplit_once(':')?;
    let runner_id = uuid::Uuid::parse_str(runner_id).ok()?;
    (!instance.is_empty()).then_some((instance, runner_id))
}

/// Registers the GitLab runner of runner row `runner_id` on `connection`,
/// tagged with the pool's labels and taking untagged jobs as the pool says.
/// The error names the connection and what GitLab answered.
pub(super) async fn register(
    app: &Reconciler,
    pool_id: &str,
    connection: &GitlabConnection,
    runner_id: &str,
) -> Result<CreatedRunner> {
    let (labels, run_untagged): (String, bool) =
        sqlx::query_as("SELECT labels,run_untagged FROM runner_pools WHERE id=?")
            .bind(pool_id)
            .fetch_one(&app.database)
            .await?;
    let tags = serde_json::from_str::<Vec<String>>(&labels).unwrap_or_default();
    let credentials = connection.credentials(app).await?;
    app.gitlab
        .create_runner(
            &credentials,
            &connection.scope()?,
            &tags,
            run_untagged,
            &marker(app, runner_id),
        )
        .await
        .map_err(|error| {
            anyhow!(
                "GitLab could not register a runner for connection {}: {error}",
                connection.name
            )
        })
}

/// Deletes GitLab runner `gitlab_runner_id` of connection `connection_id`,
/// the registration of runner row `runner_id` of pool `pool_id`. A runner
/// GitLab no longer has is already gone. A delete GitLab refuses is also a
/// warning on the pool, linked to the row (#171); a local failure (the
/// connection gone, its token unreadable) is not GitLab's refusal and is only
/// returned. Every caller logs the error and leaves the runner to the sweep.
pub(super) async fn deregister(
    app: &Reconciler,
    pool_id: &str,
    runner_id: &str,
    connection_id: &str,
    gitlab_runner_id: i64,
) -> Result<()> {
    let connection = connection(app, connection_id)
        .await?
        .context("the GitLab connection no longer exists")?;
    let credentials = connection.credentials(app).await?;
    let id = ExternalId::new(u64::try_from(gitlab_runner_id)?)?;
    match app.gitlab.delete_runner(&credentials, id).await {
        Ok(()) | Err(GitLabError::NotFound) => Ok(()),
        Err(error) => {
            // The event's own failure is logged and never hides the
            // deletion's error.
            if let Err(event_error) = record_refused_deletion(
                app,
                pool_id,
                Some(runner_id),
                &connection,
                gitlab_runner_id,
                &error,
            )
            .await
            {
                tracing::warn!(runner_id = %runner_id, error = ?event_error, "could not record the refused GitLab runner deletion in the pool's events");
            }
            Err(anyhow!(
                "GitLab could not delete runner {gitlab_runner_id} of connection {}: {error}",
                connection.name
            ))
        }
    }
}

/// A stable code for why GitLab refused a request, for the sweep's
/// deduplication: the `GitLabError` variant, with the scope for a missing
/// one. Never the message, which names the connection and may be reworded.
fn refusal_code(error: &GitLabError) -> String {
    match error {
        GitLabError::Unauthorized => "unauthorized".to_owned(),
        GitLabError::MissingScope(scope) => format!("missing_scope:{scope}"),
        GitLabError::MissingRole => "missing_role".to_owned(),
        GitLabError::NotFound => "not_found".to_owned(),
        GitLabError::RateLimited { .. } => "rate_limited".to_owned(),
        GitLabError::Upstream(_) => "upstream".to_owned(),
    }
}

/// What an operator may be told about why GitLab refused a request: the
/// fixed text of each `GitLabError` variant (a missing scope's name is
/// checked to look like one by the client), but never an `Upstream` error's
/// detail, which is free-form and stays in the log, as the API's
/// `gitlab_message` keeps it out of its responses.
fn client_safe_reason(error: &GitLabError) -> String {
    match error {
        GitLabError::Upstream(_) => "GitLab request failed".to_owned(),
        GitLabError::Unauthorized
        | GitLabError::MissingScope(_)
        | GitLabError::MissingRole
        | GitLabError::NotFound
        | GitLabError::RateLimited { .. } => error.to_string(),
    }
}

/// What an operator is told when GitLab refused to delete one of the
/// connection's runners: the connection, the GitLab runner, the
/// client-safe reason (`client_safe_reason`: no upstream detail, no token),
/// and, for a token problem only, what to give the connection.
fn refused_deletion_message(
    connection_name: &str,
    gitlab_runner_id: i64,
    error: &GitLabError,
) -> String {
    let reason = client_safe_reason(error);
    let remedy = match error {
        GitLabError::MissingScope(scope) => format!(
            "Give the connection a token with {scope} (or api); the orphan sweep retries it."
        ),
        GitLabError::Unauthorized => "Give the connection a valid token with create_runner, manage_runner and read_api (or api); the orphan sweep retries it.".to_owned(),
        _ => "The orphan sweep retries it.".to_owned(),
    };
    format!(
        "GitLab could not delete runner {gitlab_runner_id} of connection {connection_name}: {reason}. {remedy}"
    )
}

/// Records on pool `pool_id` that GitLab refused to delete GitLab runner
/// `gitlab_runner_id` of `connection`, linked to runner row `runner_id` when
/// there is one.
async fn record_refused_deletion(
    app: &Reconciler,
    pool_id: &str,
    runner_id: Option<&str>,
    connection: &GitlabConnection,
    gitlab_runner_id: i64,
    error: &GitLabError,
) -> Result<()> {
    runner_event(
        &app.database,
        Some(pool_id),
        runner_id,
        "warning",
        DELETION_REFUSED_EVENT,
        &refused_deletion_message(&connection.name, gitlab_runner_id, error),
        json!({
            "connectionId": connection.id,
            "connection": connection.name,
            "gitlabRunnerId": gitlab_runner_id,
            "reason": refusal_code(error),
        }),
    )
    .await
}

/// `delete_runner`'s GitLab arm: removes the GitLab registration of runner
/// `runner` of pool `pool_id`, if it ever got one. An error is logged; the
/// orphan sweep retries it.
pub(super) async fn deregister_runner(app: &Reconciler, pool_id: &str, runner: &Runner) {
    let (Some(connection_id), Some(gitlab_runner_id)) = (
        runner.gitlab_connection_id.as_deref(),
        runner.gitlab_runner_id,
    ) else {
        return;
    };
    if let Err(error) = deregister(app, pool_id, &runner.id, connection_id, gitlab_runner_id).await
    {
        tracing::warn!(runner_id = %runner.id, error = ?error, "could not remove the GitLab runner registration; the orphan sweep retries it");
    }
}

/// How a GitLab runner left service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Exit {
    /// It ran its one job.
    Completed,
    /// It waited out `--wait-timeout` without a job.
    IdleExpired,
    /// GitLab refused its token.
    RegistrationRefused,
    /// Nothing says how: an older manager, or a runtime that disappeared.
    Unknown,
}

impl Exit {
    /// From the manager's `exitReason` for an exited runner (D10).
    pub(super) fn from_exit_reason(reason: Option<&str>) -> Self {
        let code = GITLAB_EXIT_REASONS
            .iter()
            .find(|(_, name)| Some(*name) == reason)
            .map(|(code, _)| *code);
        match code {
            Some(GITLAB_EXIT_COMPLETED) => Self::Completed,
            Some(GITLAB_EXIT_IDLE_EXPIRED) => Self::IdleExpired,
            Some(GITLAB_EXIT_REGISTRATION_REFUSED) => Self::RegistrationRefused,
            _ => Self::Unknown,
        }
    }

    /// From the diagnostic of a fleet Inspect that reported the runner gone
    /// (D5): none for a completed runner.
    fn from_inspect(diagnostic: Option<DiagnosticCode>) -> Self {
        match diagnostic {
            None => Self::Completed,
            Some(DiagnosticCode::IdleExpired) => Self::IdleExpired,
            Some(DiagnosticCode::RegistrationRefused) => Self::RegistrationRefused,
            Some(_) => Self::Unknown,
        }
    }

    /// The published name of the outcome (`GITLAB_EXIT_REASONS`).
    fn name(self) -> &'static str {
        let code = match self {
            Self::Completed => GITLAB_EXIT_COMPLETED,
            Self::IdleExpired => GITLAB_EXIT_IDLE_EXPIRED,
            Self::RegistrationRefused => GITLAB_EXIT_REGISTRATION_REFUSED,
            Self::Unknown => return "unknown",
        };
        GITLAB_EXIT_REASONS
            .iter()
            .find(|(published, _)| *published == code)
            .map_or("unknown", |(_, name)| name)
    }
}

/// What an operator is told when GitLab refused a runner's token.
fn refusal_reason(connection_name: &str) -> String {
    format!(
        "GitLab refused the runner's registration ({}) for connection {connection_name}: check the connection's access token and that it may create runners.",
        Exit::RegistrationRefused.name()
    )
}

/// Records how a GitLab runner row that just left service (this call's
/// guarded update took it out) ended: a refusal fails it with a readable
/// diagnostic and counts one provisioning failure; a completed or idled-out
/// runner proved the registration works and resets the pool's failures.
/// Answers whether the pool's circuit changed.
async fn left_service(
    app: &Reconciler,
    pool_id: &str,
    runner_id: &str,
    connection_id: Option<&str>,
    exit: Exit,
) -> Result<bool> {
    match exit {
        Exit::Completed | Exit::IdleExpired => {
            clear_provision_failures(app, pool_id).await?;
            Ok(true)
        }
        Exit::RegistrationRefused => {
            let reason = refusal_reason(&connection_name(app, connection_id).await?);
            sqlx::query("UPDATE runners SET failure_reason=?,updated_at=? WHERE id=?")
                .bind(&reason)
                .bind(now_millis())
                .bind(runner_id)
                .execute(&app.database)
                .await?;
            system_event(
                app,
                Some(pool_id),
                "error",
                "GitLab runner registration refused",
                &reason,
                json!({ "runnerId": runner_id, "outcome": Exit::RegistrationRefused.name() }),
            )
            .await?;
            count_failure(app, pool_id).await?;
            Ok(true)
        }
        Exit::Unknown => Ok(false),
    }
}

/// Counts one provisioning failure against the pool as it is now.
async fn count_failure(app: &Reconciler, pool_id: &str) -> Result<()> {
    if let Some(pool) = load_pool(&app.database, pool_id).await? {
        record_provision_failure(&app.database, &pool).await?;
    }
    Ok(())
}

/// Manager path: the GitLab runner's container or VM exited (or vanished).
/// Takes the row out of service once and records how it ended. Answers
/// whether the pool's circuit changed.
pub(super) async fn runner_exited(
    app: &Reconciler,
    pool_id: &str,
    runner: &Runner,
    exit: Exit,
) -> Result<bool> {
    let left = sqlx::query(
        "UPDATE runners SET status='failed',failure_reason=NULL,updated_at=? WHERE id=? AND status NOT IN ('failed','deleted')",
    )
    .bind(now_millis())
    .bind(&runner.id)
    .execute(&app.database)
    .await?
    .rows_affected()
        == 1;
    if !left {
        return Ok(false);
    }
    left_service(
        app,
        pool_id,
        &runner.id,
        runner.gitlab_connection_id.as_deref(),
        exit,
    )
    .await
}

// ---------------------------------------------------------------------------
// Fleet path.
// ---------------------------------------------------------------------------

/// The fleet target a GitLab pool's next runner is placed for: the selected
/// connection's own project or group, linked to the pool's platforms. A
/// Docker platform without the host's Docker socket is refused (the runner
/// needs it) before anything is submitted, and counted.
pub(super) async fn fleet_target(
    app: &Reconciler,
    pool: &super::Pool,
    profile: &gridops_core::fleet::targets::Profile,
    connection: &GitlabConnection,
) -> Result<String> {
    if profile.input.runtime_kind == gridops_core::fleet::RuntimeKind::Docker
        && !profile.input.requires_docker_socket
    {
        record_provision_failure(&app.database, pool).await?;
        return Err(anyhow!(
            "GitLab runners need the host's Docker socket: turn on the Docker socket setting of the pool's Docker platform to start GitLab runners for connection {}.",
            connection.name
        ));
    }
    let target =
        gridops_core::fleet::targets::gitlab_connection_target(&app.database, &connection.id)
            .await?
            .with_context(|| format!("GitLab connection {} no longer exists", connection.name))?;
    gridops_core::fleet::targets::link_pool_target(&app.database, &pool.id, &target).await?;
    Ok(target)
}

/// The GitLab runner row of a fleet placement.
#[derive(FromRow)]
struct FleetRunner {
    ci_platform: String,
    gitlab_connection_id: Option<String>,
    gitlab_runner_id: Option<i64>,
}

async fn fleet_runner(app: &Reconciler, placement: &Placement) -> Result<Option<FleetRunner>> {
    Ok(sqlx::query_as(
        "SELECT ci_platform,gitlab_connection_id,gitlab_runner_id FROM runners WHERE id=?",
    )
    .bind(&placement.workload_id)
    .fetch_optional(&app.database)
    .await?
    .filter(|runner: &FleetRunner| runner.ci_platform == "gitlab"))
}

/// Registers the GitLab runner of a prepared placement and returns its
/// bootstrap material, the typed `GitlabRunnerMaterial`. A runner left from
/// an earlier attempt is deleted first. When registration fails the
/// placement is abandoned, its runner failed and the failure counted, and
/// `None` is returned.
pub(super) async fn fleet_material(
    app: &Reconciler,
    placement: &Placement,
) -> Result<Option<String>> {
    let runner = fleet_runner(app, placement)
        .await?
        .context("GitLab runner row is missing")?;
    if let (Some(connection_id), Some(previous)) = (
        runner.gitlab_connection_id.as_deref(),
        runner.gitlab_runner_id,
    ) {
        if let Err(error) = deregister(
            app,
            &placement.pool_id,
            &placement.workload_id,
            connection_id,
            previous,
        )
        .await
        {
            tracing::warn!(placement = %placement.id, error = ?error, "could not remove an earlier attempt's GitLab runner; the orphan sweep retries it");
        }
        forget_registration(app, placement).await?;
    }
    let connection = match runner.gitlab_connection_id.as_deref() {
        Some(id) => connection(app, id).await?,
        None => None,
    };
    let Some(connection) = connection else {
        fail_unstarted(
            app,
            placement,
            "The GitLab runner's connection was removed before it could register.",
        )
        .await?;
        return Ok(None);
    };
    let created = match register(app, &placement.pool_id, &connection, &placement.workload_id).await
    {
        Ok(created) => created,
        Err(error) => {
            fail_unstarted(app, placement, &error.to_string()).await?;
            return Ok(None);
        }
    };
    sqlx::query("UPDATE runners SET gitlab_runner_id=?,updated_at=? WHERE id=?")
        .bind(i64::try_from(created.id.get())?)
        .bind(now_millis())
        .bind(&placement.workload_id)
        .execute(&app.database)
        .await?;
    let material = GitlabRunnerMaterial::GitlabRunner {
        token: created.token.expose_secret().to_owned(),
        url: connection.base_url.clone(),
        wait_timeout_seconds: GITLAB_DEFAULT_WAIT_TIMEOUT_SECS,
    };
    Ok(Some(
        serde_json::to_string(&material).context("could not encode the GitLab runner material")?,
    ))
}

/// Rolls a fleet start back after its GitLab runner was registered but the
/// start could not be handed to the machine (`error`): deletes the GitLab
/// runner (the sweep is the backstop), forgets it on the row, and fails,
/// counts and abandons the placement as a failed registration is.
pub(super) async fn fleet_rollback(
    app: &Reconciler,
    placement: &Placement,
    error: &anyhow::Error,
) -> Result<()> {
    let runner = fleet_runner(app, placement).await?;
    let connection_id = runner
        .as_ref()
        .and_then(|runner| runner.gitlab_connection_id.clone());
    if let Some(FleetRunner {
        gitlab_connection_id: Some(connection_id),
        gitlab_runner_id: Some(gitlab_runner_id),
        ..
    }) = &runner
    {
        match deregister(
            app,
            &placement.pool_id,
            &placement.workload_id,
            connection_id,
            *gitlab_runner_id,
        )
        .await
        {
            Ok(()) => forget_registration(app, placement).await?,
            Err(error) => {
                tracing::warn!(placement = %placement.id, error = ?error, "could not remove the GitLab runner of a failed fleet start; the orphan sweep retries it");
            }
        }
    }
    let name = connection_name(app, connection_id.as_deref()).await?;
    tracing::warn!(placement = %placement.id, error = ?error, "could not hand a GitLab runner to its machine");
    fail_unstarted(
        app,
        placement,
        &format!(
            "The GitLab runner for connection {name} could not be handed to its machine: {error}"
        ),
    )
    .await
}

/// The row no longer holds a GitLab runner, so `delete_runner` does not
/// delete it a second time.
async fn forget_registration(app: &Reconciler, placement: &Placement) -> Result<()> {
    sqlx::query("UPDATE runners SET gitlab_runner_id=NULL,updated_at=? WHERE id=?")
        .bind(now_millis())
        .bind(&placement.workload_id)
        .execute(&app.database)
        .await?;
    Ok(())
}

/// A GitLab fleet start that failed before its start command: the runner row
/// fails with `reason` and counts once (when this call takes it out of
/// service), and the placement is abandoned, which frees its capacity while
/// no start command exists.
async fn fail_unstarted(app: &Reconciler, placement: &Placement, reason: &str) -> Result<()> {
    let reason = reason.chars().take(2_000).collect::<String>();
    let left = sqlx::query(
        "UPDATE runners SET status='failed',failure_reason=?,updated_at=? WHERE id=? AND status NOT IN ('deleted','failed')",
    )
    .bind(&reason)
    .bind(now_millis())
    .bind(&placement.workload_id)
    .execute(&app.database)
    .await?
    .rows_affected()
        == 1;
    if left {
        system_event(
            app,
            Some(&placement.pool_id),
            "error",
            "Runner provisioning failed",
            &reason,
            json!({ "runnerId": placement.workload_id }),
        )
        .await?;
        count_failure(app, &placement.pool_id).await?;
    }
    placement::abandon_unstarted(
        &app.database,
        placement.id,
        &PlacementFailure::runner_removed_before_start("failed"),
    )
    .await?;
    Ok(())
}

/// Fleet path, `follow`: the placement's GitLab runner just left service
/// (`follow`'s guarded update took it out). `exited` says an Inspect reported
/// it gone; its diagnostic says how.
pub(super) async fn fleet_runner_exited(
    app: &Reconciler,
    placement: &Placement,
    exited: bool,
) -> Result<()> {
    let Some(runner) = fleet_runner(app, placement).await? else {
        return Ok(());
    };
    let exit = if exited {
        Exit::from_inspect(
            placement::reported_diagnostic(&app.database, placement.id, "inspect").await?,
        )
    } else {
        Exit::Unknown
    };
    left_service(
        app,
        &placement.pool_id,
        &placement.workload_id,
        runner.gitlab_connection_id.as_deref(),
        exit,
    )
    .await?;
    Ok(())
}

/// Fleet path, `finish`: the placement of a GitLab runner failed (its prepare
/// or start was refused or failed on the machine) and its row just left
/// service. Counted once; a refused registration also gets its diagnostic.
pub(super) async fn fleet_start_failed(app: &Reconciler, placement: &Placement) -> Result<()> {
    let Some(runner) = fleet_runner(app, placement).await? else {
        return Ok(());
    };
    let mut diagnostic =
        placement::reported_diagnostic(&app.database, placement.id, "start_runner").await?;
    if diagnostic.is_none() {
        diagnostic =
            placement::reported_diagnostic(&app.database, placement.id, "prepare_environment")
                .await?;
    }
    if diagnostic == Some(DiagnosticCode::RegistrationRefused) {
        left_service(
            app,
            &placement.pool_id,
            &placement.workload_id,
            runner.gitlab_connection_id.as_deref(),
            Exit::RegistrationRefused,
        )
        .await?;
    } else {
        count_failure(app, &placement.pool_id).await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The orphan sweep (D17).
// ---------------------------------------------------------------------------

/// Deletes, once per `gitlabRunnerSweepIntervalSeconds`, every GitLab runner
/// this deployment marked whose runner row is no longer live, on every GitLab
/// connection (a connection whose only pool was deleted included). It is the
/// retry for a failed delete on any exit path. One connection's error is
/// logged and does not stop the others.
pub(super) async fn maybe_sweep(app: &Reconciler) -> Result<()> {
    let now = now_millis();
    let interval_seconds = setting_i64(
        &app.database,
        "gitlabRunnerSweepIntervalSeconds",
        SWEEP_INTERVAL_DEFAULT_SECS,
    )
    .await
    .clamp(SWEEP_INTERVAL_MIN_SECS, SWEEP_INTERVAL_MAX_SECS);
    let last_sweep = setting_i64(&app.database, "lastGitlabRunnerSweepAt", 0).await;
    if now.saturating_sub(last_sweep) < interval_seconds * 1_000 {
        return Ok(());
    }
    sqlx::query(
        r#"INSERT INTO settings (key,value,updated_at) VALUES ('lastGitlabRunnerSweepAt',?,?)
           ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at"#,
    )
    .bind(now.to_string())
    .bind(now)
    .execute(&app.database)
    .await?;
    let connections: Vec<GitlabConnection> = sqlx::query_as(concat!(
        "SELECT ",
        connection_columns!(),
        " FROM gitlab_connections connection ORDER BY connection.created_at,connection.id"
    ))
    .fetch_all(&app.database)
    .await?;
    let own_instance = instance(app.config.base_url());
    for connection in connections {
        if let Err(error) = sweep_connection(app, &connection, &own_instance).await {
            tracing::warn!(connection = %connection.name, error = ?error, "could not sweep the GitLab connection's orphaned runners");
        }
    }
    Ok(())
}

async fn sweep_connection(
    app: &Reconciler,
    connection: &GitlabConnection,
    own_instance: &str,
) -> Result<()> {
    let credentials = connection.credentials(app).await?;
    let listed = app
        .gitlab
        .list_runners(&credentials, &connection.scope()?)
        .await
        .map_err(|error| anyhow!("GitLab could not list the runners: {error}"))?;
    for RunnerSummary { id, description } in listed {
        let Some((instance, runner_id)) = parse_marker(&description) else {
            continue;
        };
        if instance != own_instance || runner_live(app, &runner_id.to_string()).await? {
            continue;
        }
        let Ok(gitlab_runner_id) = ExternalId::try_from(id) else {
            tracing::warn!(connection = %connection.name, gitlab_runner_id = id, "GitLab listed a runner id that is not a valid id; skipping it");
            continue;
        };
        match app
            .gitlab
            .delete_runner(&credentials, gitlab_runner_id)
            .await
        {
            Ok(()) | Err(GitLabError::NotFound) => {
                tracing::info!(connection = %connection.name, gitlab_runner_id = id, runner_id = %runner_id, "removed an orphaned GitLab runner");
            }
            Err(error) => {
                tracing::warn!(connection = %connection.name, gitlab_runner_id = id, error = ?error, "could not remove an orphaned GitLab runner");
                // The event's own failure is logged and never stops the sweep.
                if let Err(event_error) = warn_linked_pools(app, connection, id, &error).await {
                    tracing::warn!(connection = %connection.name, gitlab_runner_id = id, error = ?event_error, "could not record the refused GitLab runner deletion in the pools' events");
                }
            }
        }
    }
    Ok(())
}

/// Records that GitLab refused the sweep's delete of GitLab runner
/// `gitlab_runner_id` of `connection` on every pool linked to the connection.
/// The runner's row, and its pool, may be gone, so every pool that may start
/// runners on the connection is told, and no row is linked (`runner_id` stays
/// NULL: a gone row would break the foreign key).
///
/// Deduplicated so a token that keeps failing does not write a row per runner
/// per sweep: at most one per pool, connection and reason code an hour
/// (`SWEEP_WARNING_WINDOW_MS`). What was written is read back from
/// `runner_events` (its `created_at` and the metadata's `connectionId` and
/// `reason`), as `record_provider_changes` does, so a reconciler restart
/// neither repeats nor resets it. A new reason warns at once; the same reason
/// warns again once the hour has passed. The window, not "only when the reason
/// changes", because a delete that succeeds writes nothing: a token fixed and
/// then broken the same way again would otherwise never warn. The per-runner
/// warnings `deregister` writes are not deduplicated.
///
/// One pool's failure is logged and the next pool is still told, so a row
/// that cannot be written never suppresses the other pools' warnings.
async fn warn_linked_pools(
    app: &Reconciler,
    connection: &GitlabConnection,
    gitlab_runner_id: u64,
    error: &GitLabError,
) -> Result<()> {
    let gitlab_runner_id = i64::try_from(gitlab_runner_id)?;
    let reason = refusal_code(error);
    let pools: Vec<String> = sqlx::query_scalar(
        "SELECT pool_id FROM runner_pool_gitlab_connections WHERE connection_id=? ORDER BY pool_id",
    )
    .bind(&connection.id)
    .fetch_all(&app.database)
    .await?;
    let since = now_millis().saturating_sub(SWEEP_WARNING_WINDOW_MS);
    for pool_id in pools {
        if let Err(event_error) = warn_pool(
            app,
            &pool_id,
            connection,
            gitlab_runner_id,
            error,
            &reason,
            since,
        )
        .await
        {
            tracing::warn!(pool_id = %pool_id, connection = %connection.name, gitlab_runner_id, error = ?event_error, "could not record the refused GitLab runner deletion in the pool's events");
        }
    }
    Ok(())
}

/// `warn_linked_pools` for pool `pool_id`: records the sweep's refused
/// delete unless the pool already has a warning for the connection and
/// reason code `reason` written after `since`.
async fn warn_pool(
    app: &Reconciler,
    pool_id: &str,
    connection: &GitlabConnection,
    gitlab_runner_id: i64,
    error: &GitLabError,
    reason: &str,
    since: i64,
) -> Result<()> {
    let recorded = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM runner_events
          WHERE pool_id=? AND event=? AND runner_id IS NULL AND created_at>?
            AND json_extract(metadata,'$.connectionId')=?
            AND json_extract(metadata,'$.reason')=?)",
    )
    .bind(pool_id)
    .bind(DELETION_REFUSED_EVENT)
    .bind(since)
    .bind(&connection.id)
    .bind(reason)
    .fetch_one(&app.database)
    .await?
        != 0;
    if !recorded {
        record_refused_deletion(app, pool_id, None, connection, gitlab_runner_id, error).await?;
    }
    Ok(())
}

/// Whether runner row `runner_id` is still in service: it exists and is not
/// deleted. A `starting` row is live, so a start under way is never raced.
async fn runner_live(app: &Reconciler, runner_id: &str) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM runners WHERE id=? AND deleted_at IS NULL AND status!='deleted')",
    )
    .bind(runner_id)
    .fetch_one(&app.database)
    .await?
        != 0)
}
