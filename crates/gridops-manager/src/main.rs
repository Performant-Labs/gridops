use std::{
    collections::HashMap,
    env,
    os::unix::fs::MetadataExt as _,
    path::Path as FsPath,
    process::Command,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use axum::{
    Json, Router,
    body::Body,
    extract::{FromRequestParts, Path, Query, State},
    http::{StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use bollard::{
    API_DEFAULT_VERSION, Docker,
    errors::Error as DockerError,
    models::{ContainerCreateBody, HostConfig, HostConfigLogConfig, NetworkCreateRequest},
    query_parameters::{
        AttachContainerOptionsBuilder, CreateContainerOptionsBuilder, CreateImageOptionsBuilder,
        ListContainersOptionsBuilder, LogsOptionsBuilder, RemoveContainerOptionsBuilder,
        RestartContainerOptionsBuilder, StartContainerOptions, StopContainerOptionsBuilder,
    },
};
use futures_util::{StreamExt as _, TryStreamExt as _};
use gridops_core::provisioning::instrumented_runner_script;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use subtle::ConstantTimeEq as _;
use tokio::{io::AsyncWriteExt as _, sync::Mutex};
use tower_http::{catch_panic::CatchPanicLayer, timeout::TimeoutLayer, trace::TraceLayer};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _};
use url::Url;

mod builtin_host;
#[cfg(test)]
mod provision_lock_tests;
#[cfg(test)]
mod route_timeout_tests;
mod sandbox;
#[cfg(test)]
mod stand_in_docker;
mod vm_slots;

#[derive(Clone)]
struct ManagerState {
    docker: Docker,
    tart: Option<TartAgent>,
    token: SecretString,
    limits: ManagerLimits,
    shared_docker_socket: Option<SharedDockerSocket>,
    reservations: Arc<Mutex<HashMap<String, CapacityReservation>>>,
    provisioning_paused: Arc<AtomicBool>,
    builtin_host: builtin_host::BuiltinHost,
    provision_locks: ProvisionLocks,
    /// The Tart host's macOS VM slot limit and the line for its slots.
    vm_slots: Arc<vm_slots::VmSlots>,
}

/// One start lock per provider. Each serialises the work that puts load on
/// its own host: admission, which reads the host's usage and then adds a
/// capacity lease, against the starts that turn leases into running
/// runners, so an admission never reads a host's usage from before a start
/// and its leases from after it. Docker admissions, runner starts, container
/// start, resume and restart, and sandbox creation take the Docker lock; Tart
/// admissions and runner starts take the Tart lock. Nothing either lock
/// covers is read by the other provider: admission counts only its own
/// provider's leases, the Mac's VM slots are Tart's alone, and the built-in
/// host's budget and disk are Docker's alone. The capacity leases, which both
/// providers keep in one map, have their own short lock (`reservations`).
/// A Tart start holds the Tart lock for its whole call to the agent, which
/// can take minutes, so Docker work no longer waits behind it.
#[derive(Clone, Default)]
struct ProvisionLocks {
    docker: Arc<Mutex<()>>,
    tart: Arc<Mutex<()>>,
}

impl ProvisionLocks {
    /// Waits for `provider`'s start lock; anything but Tart is Docker.
    async fn lock(&self, provider: &str) -> tokio::sync::MutexGuard<'_, ()> {
        if provider == "tart" {
            self.tart.lock().await
        } else {
            self.docker.lock().await
        }
    }
}

/// The Docker host's engine, shared with runner containers so jobs can run
/// `docker` against it. Every job on a Docker runner then controls that engine.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SharedDockerSocket {
    host_path: String,
    /// The socket's owning group, which the runner user must join to open it.
    group_id: u32,
}

#[derive(Clone)]
struct TartAgent {
    base_url: Url,
    token: SecretString,
    http: reqwest::Client,
}

#[derive(Clone, Debug)]
struct ManagerLimits {
    /// The Docker host's own name, so the UI can say which machine a budget is about.
    host_name: String,
    available_cpus: f64,
    total_memory_mb: i64,
    cpu_budget: f64,
    memory_budget_mb: i64,
    max_runners: usize,
    min_free_disk_mb: u64,
    min_free_disk_percent: u64,
    runner_network: String,
    log_max_size: String,
    log_max_files: u64,
    runner_pids_limit: i64,
    runner_swap_percent: i64,
    /// Where the fallback figures above came from, so the budget in force can
    /// say so and a runner count fixed in the environment stays fixed.
    budget_origin: BudgetOrigin,
}

/// Which of the manager's own budget figures were set in its environment
/// rather than derived from the host.
#[derive(Clone, Copy, Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
struct BudgetOrigin {
    cpu_from_env: bool,
    memory_from_env: bool,
    max_runners_from_env: bool,
}

/// The built-in host's budget in force, and where each figure came from:
/// `setting` (`GridOps`), `environment` (the manager's variables) or `default`
/// (95% of the host).
#[derive(Clone, Copy, Debug)]
struct EffectiveBudget {
    budget: CapacityBudget,
    cpu_source: &'static str,
    memory_source: &'static str,
}

impl EffectiveBudget {
    fn to_json(self) -> Value {
        json!({
            "cpuBudget": self.budget.cpu,
            "memoryBudgetMb": self.budget.memory_mb,
            "maxRunners": self.budget.max_runners,
            "source": { "cpu": self.cpu_source, "memory": self.memory_source },
        })
    }
}

/// Two cores and 2 GB per runner, at least one.
fn derived_max_runners(cpu: f64, memory_mb: i64) -> usize {
    usize::try_from(((cpu / 2.0).floor() as i64).min(memory_mb / 2_048).max(1)).unwrap_or(1)
}

#[derive(Clone, Copy, Debug)]
struct CapacityBudget {
    cpu: f64,
    memory_mb: i64,
    max_runners: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TartCapacity {
    cpu_budget: f64,
    memory_budget_mb: i64,
    max_runners: usize,
    active: CapacityUsage,
    /// Who holds the Mac's VM slots; absent from agents that predate it.
    #[serde(default)]
    vm_slots: Option<vm_slots::AgentSlotUsage>,
}

#[derive(Clone, Debug)]
struct CapacityReservation {
    runner_id: String,
    pool_id: String,
    provider: String,
    cpu_limit: f64,
    memory_limit_mb: i64,
    /// Holds one of the Tart host's macOS VM slots until the VM is started.
    vm_slot: bool,
    expires_at: Instant,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CapacityUsage {
    active_runners: usize,
    cpu: f64,
    memory_mb: i64,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_field_names)]
struct DiskCapacity {
    total_mb: u64,
    available_mb: u64,
    minimum_free_mb: u64,
}

struct ManagerAuth;

impl FromRequestParts<ManagerState> for ManagerAuth {
    type Rejection = ManagerError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &ManagerState,
    ) -> Result<Self, Self::Rejection> {
        let supplied = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(ManagerError::Unauthorized)?;
        let expected = state.token.expose_secret();
        if supplied.len() != expected.len()
            || !bool::from(supplied.as_bytes().ct_eq(expected.as_bytes()))
        {
            return Err(ManagerError::Unauthorized);
        }
        Ok(Self)
    }
}

#[derive(Debug)]
enum ManagerError {
    Unauthorized,
    Forbidden(String),
    BadRequest(String),
    Conflict(String),
    PayloadTooLarge(String),
    Guardrail {
        code: &'static str,
        message: String,
    },
    NotFound(String),
    Upstream {
        status: StatusCode,
        code: String,
        message: String,
    },
    /// A request that ran out of time, with what timed out.
    TimedOut {
        code: &'static str,
        message: String,
    },
    Internal(anyhow::Error),
}

impl IntoResponse for ManagerError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Unauthorized".into(),
            ),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, "forbidden", message),
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, "bad_request", message),
            Self::Conflict(message) => (StatusCode::CONFLICT, "conflict", message),
            Self::PayloadTooLarge(message) => {
                (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large", message)
            }
            Self::Guardrail { code, message } => (StatusCode::TOO_MANY_REQUESTS, code, message),
            Self::NotFound(message) => (StatusCode::NOT_FOUND, "not_found", message),
            Self::TimedOut { code, message } => (StatusCode::REQUEST_TIMEOUT, code, message),
            Self::Upstream {
                status,
                code,
                message,
            } => return (status, Json(json!({ "code": code, "error": message }))).into_response(),
            Self::Internal(error) => {
                tracing::error!(error = ?error, "runner manager request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "Runner manager request failed.".into(),
                )
            }
        };
        (status, Json(json!({ "code": code, "error": message }))).into_response()
    }
}

impl From<DockerError> for ManagerError {
    fn from(error: DockerError) -> Self {
        match &error {
            DockerError::DockerResponseServerError {
                status_code: 404, ..
            } => Self::NotFound("Managed runner container was not found.".into()),
            DockerError::DockerResponseServerError {
                status_code: 409,
                message,
            } => Self::Conflict(message.clone()),
            _ => Self::Internal(error.into()),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProvisionRunner {
    runner_id: String,
    pool_id: String,
    name: String,
    image: String,
    mode: String,
    #[serde(default = "default_provider")]
    provider: String,
    #[serde(default = "default_platform")]
    platform: String,
    jit_config: Option<String>,
    registration_token: Option<String>,
    registration_url: Option<String>,
    #[serde(default)]
    labels: Vec<String>,
    runner_group: Option<String>,
    bitbucket: Option<BitbucketBootstrap>,
    bitbucket_oauth_client_secret: Option<String>,
    #[serde(default)]
    pull_image: bool,
    cpu_limit: f64,
    memory_limit_mb: i64,
    network: String,
    capacity_lease: String,
    /// Forwarded verbatim to the macOS agent: "vm" clones a Tart VM per job,
    /// "native" runs the runner on the agent host. Docker ignores it.
    #[serde(default = "default_runtime")]
    runtime: String,
}

fn default_runtime() -> String {
    "vm".into()
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BitbucketBootstrap {
    account_uuid: String,
    repository_uuid: Option<String>,
    runner_uuid: String,
    oauth_client_id: String,
}

impl ProvisionRunner {
    fn validate(&self) -> Result<(), ManagerError> {
        if self.runner_id.is_empty()
            || self.pool_id.is_empty()
            || self.capacity_lease.is_empty()
            || self.capacity_lease.len() > 128
        {
            return Err(ManagerError::BadRequest(
                "Runner, pool, and capacity lease identifiers are required.".into(),
            ));
        }
        if !valid_docker_name(&self.name)
            || (self.provider == "docker" && !valid_docker_name(&self.network))
        {
            return Err(ManagerError::BadRequest(
                "Runner or network name contains unsupported characters.".into(),
            ));
        }
        // Native macOS runners execute on the agent host and have nothing to
        // clone or pull, so they legitimately carry no image.
        let image_required = self.runtime != "native";
        if (image_required && self.image.trim().is_empty()) || self.image.len() > 512 {
            return Err(ManagerError::BadRequest("Runner image is invalid.".into()));
        }
        if !matches!(self.runtime.as_str(), "vm" | "native") {
            return Err(ManagerError::BadRequest(
                "Runner runtime must be vm or native.".into(),
            ));
        }
        if self.runtime == "native" && self.provider != "tart" {
            return Err(ManagerError::BadRequest(
                "Native execution is only available on the macOS agent.".into(),
            ));
        }
        match (self.platform.as_str(), self.mode.as_str()) {
            ("github", "ephemeral") => {
                if self.jit_config.as_ref().is_none_or(|value| {
                    !(20..=900_000).contains(&value.len()) || value.contains(['\n', '\r'])
                }) {
                    return Err(ManagerError::BadRequest(
                        "Runner JIT configuration is invalid.".into(),
                    ));
                }
            }
            ("github", "persistent") => {
                if self.registration_token.as_ref().is_none_or(|value| {
                    !(20..=2_048).contains(&value.len()) || value.contains(['\n', '\r'])
                }) || self
                    .registration_url
                    .as_deref()
                    .is_none_or(|value| !valid_registration_url(value))
                    || self.labels.is_empty()
                    || self.labels.len() > 100
                    || self.labels.iter().any(|label| {
                        label.is_empty() || label.len() > 100 || label.contains([',', '\n', '\r'])
                    })
                    || self.runner_group.as_ref().is_some_and(|group| {
                        group.is_empty() || group.len() > 100 || group.contains(['\n', '\r'])
                    })
                {
                    return Err(ManagerError::BadRequest(
                        "Persistent runner registration is invalid.".into(),
                    ));
                }
            }
            ("github", _) => {
                return Err(ManagerError::BadRequest(
                    "Runner mode must be ephemeral or persistent.".into(),
                ));
            }
            ("bitbucket", "persistent") => {
                let valid_value = |value: &str| {
                    !value.is_empty() && value.len() <= 255 && !value.contains(['\n', '\r'])
                };
                if self.provider != "tart"
                    || self.bitbucket.as_ref().is_none_or(|bootstrap| {
                        !valid_value(&bootstrap.account_uuid)
                            || bootstrap
                                .repository_uuid
                                .as_deref()
                                .is_some_and(|value| !valid_value(value))
                            || !valid_value(&bootstrap.runner_uuid)
                            || !valid_value(&bootstrap.oauth_client_id)
                    })
                    || self
                        .bitbucket_oauth_client_secret
                        .as_ref()
                        .is_none_or(|secret| {
                            !(20..=2_048).contains(&secret.len()) || secret.contains(['\n', '\r'])
                        })
                {
                    return Err(ManagerError::BadRequest(
                        "Bitbucket macOS runner registration is invalid.".into(),
                    ));
                }
            }
            ("bitbucket", _) => {
                return Err(ManagerError::BadRequest(
                    "Bitbucket runners are persistent until their VM is drained.".into(),
                ));
            }
            _ => {
                return Err(ManagerError::BadRequest(
                    "Runner platform is invalid.".into(),
                ));
            }
        }
        if !matches!(self.provider.as_str(), "docker" | "tart") {
            return Err(ManagerError::BadRequest(
                "Runner provider must be Docker or Tart.".into(),
            ));
        }
        if self.provider == "tart" && self.platform == "github" && self.mode != "ephemeral" {
            return Err(ManagerError::BadRequest(
                "Tart runner pools must be ephemeral.".into(),
            ));
        }
        if !self.cpu_limit.is_finite() || self.cpu_limit <= 0.0 || self.memory_limit_mb <= 0 {
            return Err(ManagerError::BadRequest(
                "Runner CPU and memory must be positive values.".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapacityRequest {
    runner_id: String,
    pool_id: String,
    #[serde(default = "default_provider")]
    provider: String,
    cpu_limit: f64,
    memory_limit_mb: i64,
    /// For Tart, "vm" (the default) needs one of the Mac's VM slots;
    /// "native" runs on the host and needs none. Docker ignores it, whatever
    /// it says.
    #[serde(default)]
    runtime: Option<String>,
    /// The reconciler pass asking, which keeps a waiting pool's place in the
    /// line for VM slots. Starts from the web app send none.
    #[serde(default)]
    reconcile_pass: Option<i64>,
}

impl CapacityRequest {
    fn validate(&self) -> Result<(), ManagerError> {
        if self.runner_id.is_empty()
            || self.runner_id.len() > 128
            || self.pool_id.is_empty()
            || self.pool_id.len() > 128
        {
            return Err(ManagerError::BadRequest(
                "Runner and pool identifiers are invalid.".into(),
            ));
        }
        if !self.cpu_limit.is_finite() || self.cpu_limit <= 0.0 || self.memory_limit_mb <= 0 {
            return Err(ManagerError::BadRequest(
                "Runner CPU and memory must be positive values.".into(),
            ));
        }
        if !matches!(self.provider.as_str(), "docker" | "tart") {
            return Err(ManagerError::BadRequest(
                "Runner provider must be Docker or Tart.".into(),
            ));
        }
        if self.provider == "tart"
            && self
                .runtime
                .as_deref()
                .is_some_and(|runtime| !matches!(runtime, "vm" | "native"))
        {
            return Err(ManagerError::BadRequest(
                "Runner runtime must be vm or native.".into(),
            ));
        }
        Ok(())
    }

    /// Whether the runner is a Tart VM, which takes one of the Mac's slots.
    fn needs_vm_slot(&self) -> bool {
        self.provider == "tart" && self.runtime.as_deref() != Some("native")
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagerPolicy {
    provisioning_paused: bool,
    /// Whether runners may start on this manager's own host. Callers that
    /// predate the setting leave it out, which leaves it as it was.
    builtin_host_enabled: Option<bool>,
    /// The host's budget as set in `GridOps`; a `null` figure falls back to
    /// the manager's own. Left out (older callers), it stays as it was.
    builtin_host_budget: Option<builtin_host::Budget>,
    /// The Tart host's macOS VM slots, as set in `GridOps`. Left out (older
    /// callers), the limit stays as it was.
    #[serde(default)]
    tart_vm_slots: Option<usize>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedRunner {
    id: String,
    names: Vec<String>,
    image: String,
    state: String,
    status: String,
    labels: HashMap<String, String>,
    created_at: String,
    provider: String,
}

fn default_provider() -> String {
    "docker".into()
}

fn default_platform() -> String {
    "github".into()
}

#[derive(Deserialize)]
struct LogsQuery {
    #[serde(default)]
    follow: bool,
    tail: Option<String>,
}

impl ManagerLimits {
    fn from_environment(
        host_name: String,
        available_cpus: i64,
        total_memory_bytes: i64,
    ) -> Result<Self> {
        let available_cpus = available_cpus.max(1) as f64;
        let total_memory_mb = (total_memory_bytes / 1_024 / 1_024).max(512);
        let default_cpu_budget = (available_cpus * 0.95).max(1.0);
        let default_memory_budget_mb = (total_memory_mb * 95 / 100).max(512);
        let env_cpu_budget = optional_env("GRIDOPS_RUNNER_CPU_BUDGET")?;
        let env_memory_budget_mb = optional_env("GRIDOPS_RUNNER_MEMORY_BUDGET_MB")?;
        let cpu_budget = env_cpu_budget.unwrap_or(default_cpu_budget);
        let memory_budget_mb = env_memory_budget_mb.unwrap_or(default_memory_budget_mb);
        anyhow::ensure!(
            cpu_budget > 0.0 && cpu_budget <= available_cpus,
            "GRIDOPS_RUNNER_CPU_BUDGET must be positive and no greater than the Docker host CPU count"
        );
        anyhow::ensure!(
            memory_budget_mb >= 256 && memory_budget_mb <= total_memory_mb,
            "GRIDOPS_RUNNER_MEMORY_BUDGET_MB must be between 256 MB and Docker host memory"
        );
        let env_max_runners = optional_env("GRIDOPS_MAX_MANAGED_RUNNERS")?;
        let max_runners =
            env_max_runners.unwrap_or_else(|| derived_max_runners(cpu_budget, memory_budget_mb));
        anyhow::ensure!(
            (1..=1_000).contains(&max_runners),
            "GRIDOPS_MAX_MANAGED_RUNNERS must be between 1 and 1000"
        );
        let min_free_disk_mb = optional_env("GRIDOPS_MIN_FREE_DISK_MB")?.unwrap_or(10_240);
        let min_free_disk_percent = optional_env("GRIDOPS_MIN_FREE_DISK_PERCENT")?.unwrap_or(10);
        anyhow::ensure!(
            min_free_disk_percent <= 95,
            "GRIDOPS_MIN_FREE_DISK_PERCENT must be between 0 and 95"
        );
        let runner_pids_limit = optional_env("GRIDOPS_RUNNER_PIDS_LIMIT")?.unwrap_or(1_024);
        anyhow::ensure!(
            (64..=32_768).contains(&runner_pids_limit),
            "GRIDOPS_RUNNER_PIDS_LIMIT must be between 64 and 32768"
        );
        // Swap headroom as a percentage of each runner's memory limit, so memory
        // spikes such as release linking slow down instead of being OOM-killed.
        let runner_swap_percent = optional_env("GRIDOPS_RUNNER_SWAP_PERCENT")?.unwrap_or(100);
        anyhow::ensure!(
            (0..=400).contains(&runner_swap_percent),
            "GRIDOPS_RUNNER_SWAP_PERCENT must be between 0 and 400"
        );
        let log_max_size = env::var("GRIDOPS_RUNNER_LOG_MAX_SIZE").unwrap_or_else(|_| "20m".into());
        anyhow::ensure!(
            valid_log_size(&log_max_size),
            "GRIDOPS_RUNNER_LOG_MAX_SIZE must use Docker's positive k, m, or g size syntax"
        );
        let log_max_files = optional_env("GRIDOPS_RUNNER_LOG_MAX_FILES")?.unwrap_or(5);
        anyhow::ensure!(
            (1..=100).contains(&log_max_files),
            "GRIDOPS_RUNNER_LOG_MAX_FILES must be between 1 and 100"
        );
        let runner_network =
            env::var("GRIDOPS_RUNNER_NETWORK").unwrap_or_else(|_| "gridops-runners".into());
        anyhow::ensure!(
            valid_docker_name(&runner_network),
            "GRIDOPS_RUNNER_NETWORK is invalid"
        );
        Ok(Self {
            host_name,
            available_cpus,
            total_memory_mb,
            cpu_budget,
            memory_budget_mb,
            max_runners,
            min_free_disk_mb,
            min_free_disk_percent,
            runner_network,
            log_max_size,
            log_max_files,
            runner_pids_limit,
            runner_swap_percent,
            budget_origin: BudgetOrigin {
                cpu_from_env: env_cpu_budget.is_some(),
                memory_from_env: env_memory_budget_mb.is_some(),
                max_runners_from_env: env_max_runners.is_some(),
            },
        })
    }

    /// The manager's own budget, before any setting from `GridOps`.
    #[cfg(test)]
    fn docker_budget(&self) -> CapacityBudget {
        self.effective_budget(builtin_host::Budget::default())
            .budget
    }

    /// The budget in force: a figure `GridOps` sets replaces the manager's
    /// own, and never exceeds what the host has. Unless the environment fixes
    /// the runner count, it follows the budget as it does at startup.
    fn effective_budget(&self, setting: builtin_host::Budget) -> EffectiveBudget {
        let origin = |from_env: bool| if from_env { "environment" } else { "default" };
        let (cpu, cpu_source) = match setting.cpu_millis {
            Some(millis) => (
                (millis as f64 / 1_000.0).min(self.available_cpus),
                "setting",
            ),
            None => (self.cpu_budget, origin(self.budget_origin.cpu_from_env)),
        };
        let (memory_mb, memory_source) = match setting.memory_mib {
            Some(mib) => (mib.min(self.total_memory_mb), "setting"),
            None => (
                self.memory_budget_mb,
                origin(self.budget_origin.memory_from_env),
            ),
        };
        let max_runners = if self.budget_origin.max_runners_from_env {
            self.max_runners
        } else {
            derived_max_runners(cpu, memory_mb)
        };
        EffectiveBudget {
            budget: CapacityBudget {
                cpu,
                memory_mb,
                max_runners,
            },
            cpu_source,
            memory_source,
        }
    }
}

impl ManagerState {
    /// The built-in host's budget in force.
    fn effective_budget(&self) -> EffectiveBudget {
        self.limits.effective_budget(self.builtin_host.budget())
    }

    fn docker_budget(&self) -> CapacityBudget {
        self.effective_budget().budget
    }
}

fn optional_env<T>(name: &str) -> Result<Option<T>>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            value
                .parse::<T>()
                .with_context(|| format!("{name} is invalid"))
        })
        .transpose()
}

/// `host_path` is the socket's path on the Docker host, which is what a bind
/// mount resolves against. The manager's own mount of the same engine supplies
/// the socket's group.
fn shared_docker_socket(
    host_path: Option<String>,
    manager_socket: &FsPath,
) -> Result<Option<SharedDockerSocket>> {
    let Some(host_path) = host_path.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    anyhow::ensure!(
        host_path.starts_with('/') && !host_path.contains([':', ',']),
        "GRIDOPS_RUNNER_DOCKER_SOCKET must be an absolute path on the Docker host"
    );
    let group_id = std::fs::metadata(manager_socket)
        .with_context(|| format!("could not inspect {}", manager_socket.display()))?
        .gid();
    Ok(Some(SharedDockerSocket {
        host_path,
        group_id,
    }))
}

fn tart_agent_from_environment() -> Result<Option<TartAgent>> {
    let url = env::var("GRIDOPS_TART_AGENT_URL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let token = env::var("GRIDOPS_TART_AGENT_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    match (url, token) {
        (None, None) => Ok(None),
        (Some(url), Some(token)) => {
            let base_url = Url::parse(&url).context("GRIDOPS_TART_AGENT_URL is invalid")?;
            anyhow::ensure!(
                matches!(base_url.scheme(), "http" | "https"),
                "GRIDOPS_TART_AGENT_URL must use HTTP or HTTPS"
            );
            let http = reqwest::Client::builder()
                .user_agent("GridOps manager/0.1")
                .timeout(Duration::from_mins(45))
                .build()?;
            Ok(Some(TartAgent {
                base_url,
                token: SecretString::from(token),
                http,
            }))
        }
        _ => anyhow::bail!(
            "GRIDOPS_TART_AGENT_URL and GRIDOPS_TART_AGENT_TOKEN must be configured together"
        ),
    }
}

async fn tart_agent_response(
    agent: &TartAgent,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<reqwest::Response, ManagerError> {
    let url = agent
        .base_url
        .join(path.trim_start_matches('/'))
        .map_err(|error| ManagerError::Internal(error.into()))?;
    let mut request = agent
        .http
        .request(method, url)
        .bearer_auth(agent.token.expose_secret());
    if let Some(body) = body {
        request = request.json(&body);
    }
    request
        .send()
        .await
        .map_err(|error| ManagerError::Upstream {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "tart_agent_unavailable".into(),
            message: format!("Tart agent is unavailable: {error}"),
        })
}

async fn tart_agent_value(
    agent: &TartAgent,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, ManagerError> {
    let response = tart_agent_response(agent, method, path, body).await?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| ManagerError::Internal(error.into()))?;
    if !status.is_success() {
        let payload = serde_json::from_str::<Value>(&text).ok();
        let message = payload
            .as_ref()
            .and_then(|value| value.get("error"))
            .and_then(Value::as_str)
            .unwrap_or(&text)
            .trim();
        return Err(ManagerError::Upstream {
            status,
            code: payload
                .as_ref()
                .and_then(|value| value.get("code"))
                .and_then(Value::as_str)
                .unwrap_or("tart_agent_error")
                .to_owned(),
            // An empty reason would reach the operator as nothing at all.
            message: if message.is_empty() {
                format!(
                    "the Tart agent answered {status} without saying why; see its log on the Mac"
                )
            } else {
                message.chars().take(2_000).collect()
            },
        });
    }
    serde_json::from_str(&text).map_err(|error| ManagerError::Internal(error.into()))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "gridops_manager=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let token = env::var("GRIDOPS_MANAGER_TOKEN")
        .context("GRIDOPS_MANAGER_TOKEN is required by the runner manager")?;
    let socket =
        env::var("GRIDOPS_DOCKER_SOCKET").unwrap_or_else(|_| "/var/run/docker.sock".into());
    let bind = env::var("GRIDOPS_MANAGER_BIND").unwrap_or_else(|_| "127.0.0.1:8788".into());
    // `GRIDOPS_MANAGER_DOCKER=off` runs the manager with no Docker socket at
    // all: it keeps serving health and the Tart provider but starts nothing
    // on its own host. Use it once the built-in runner host is off.
    let docker_available = !env::var("GRIDOPS_MANAGER_DOCKER").is_ok_and(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "off" | "false" | "0" | "no"
        )
    });
    let docker = Docker::connect_with_socket(&socket, 120, API_DEFAULT_VERSION)
        .context("could not connect to Docker")?;
    let (host_name, ncpu, mem_total) = if docker_available {
        let info = docker
            .info()
            .await
            .context("could not inspect Docker capacity")?;
        (
            info.name.clone().unwrap_or_default(),
            info.ncpu.unwrap_or(1),
            info.mem_total.unwrap_or(512 * 1_024 * 1_024),
        )
    } else {
        tracing::warn!(
            "GRIDOPS_MANAGER_DOCKER=off: no runner or agent sandbox can start on this host"
        );
        (String::new(), 1, 512 * 1_024 * 1_024)
    };
    let limits = ManagerLimits::from_environment(host_name, ncpu, mem_total)?;
    let tart = tart_agent_from_environment()?;
    let shared_docker_socket = shared_docker_socket(
        env::var("GRIDOPS_RUNNER_DOCKER_SOCKET").ok(),
        FsPath::new(&socket),
    )?;
    let provisioning_paused = env::var("GRIDOPS_PROVISIONING_PAUSED")
        .is_ok_and(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"));
    tracing::info!(
        cpu_budget = limits.cpu_budget,
        memory_budget_mb = limits.memory_budget_mb,
        max_runners = limits.max_runners,
        tart_enabled = tart.is_some(),
        docker_socket_shared = shared_docker_socket.is_some(),
        docker_available,
        min_free_disk_mb = limits.min_free_disk_mb,
        provisioning_paused,
        "runner host guardrails initialized"
    );
    let state = ManagerState {
        docker,
        tart,
        token: SecretString::from(token),
        limits,
        shared_docker_socket,
        reservations: Arc::new(Mutex::new(HashMap::new())),
        provisioning_paused: Arc::new(AtomicBool::new(provisioning_paused)),
        builtin_host: builtin_host::BuiltinHost::new(docker_available, None),
        provision_locks: ProvisionLocks::default(),
        vm_slots: Arc::default(),
    };

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(address = bind, "GridOps Rust runner manager listening");
    axum::serve(listener, router(state)).await?;
    Ok(())
}

fn router(state: ManagerState) -> Router {
    let standard = Router::new()
        .route("/v1/health", get(health))
        .route("/v1/policy", put(update_policy))
        .route("/v1/admissions", post(reserve_capacity))
        .route("/v1/admissions/{lease_id}", delete(release_capacity))
        .route("/v1/runners/{container_id}", delete(delete_runner))
        .route("/v1/runners/{container_id}/logs", get(logs))
        .route("/v1/runners/{container_id}/{action}", post(control_runner))
        .route(
            "/v1/sandboxes/{sandbox_id}",
            delete(sandbox::delete_sandbox),
        )
        .layer(request_timeout(STANDARD_REQUEST_TIMEOUT));
    // Agent commands and repository uploads outlive the standard timeout, so
    // each carries its own ceiling instead.
    let long_running = Router::new()
        // Starting a runner sets its own limit by provider, inside the
        // handler (see `provision_time_limit`); this only backs it up.
        .route(
            "/v1/runners",
            get(list_runners)
                .layer(request_timeout(STANDARD_REQUEST_TIMEOUT))
                .merge(post(provision_runner).layer(request_timeout(PROVISION_ROUTE_TIMEOUT))),
        )
        // Creating a sandbox may first pull its image.
        .route(
            "/v1/sandboxes",
            get(sandbox::list_sandboxes)
                .layer(request_timeout(STANDARD_REQUEST_TIMEOUT))
                .merge(
                    post(sandbox::create_sandbox).layer(request_timeout(SANDBOX_CREATE_TIMEOUT)),
                ),
        )
        .route(
            "/v1/sandboxes/{sandbox_id}/exec",
            post(sandbox::exec_in_sandbox).layer(request_timeout(sandbox::EXEC_ROUTE_TIMEOUT)),
        )
        .route(
            "/v1/sandboxes/{sandbox_id}/archive",
            put(sandbox::upload_sandbox_archive)
                .layer(request_timeout(sandbox::ARCHIVE_ROUTE_TIMEOUT)),
        );
    standard
        .merge(long_running)
        .with_state(state)
        .layer(axum::middleware::from_fn(explain_timeouts))
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http())
}

/// How long most manager requests may take.
const STANDARD_REQUEST_TIMEOUT: Duration = Duration::from_mins(3);

/// How long a Tart runner start may take, from the request reaching the
/// manager to the agent's answer. The Tart agent fits its whole start inside
/// this: it refuses a `GRIDOPS_TART_VM_SLOT_WAIT_SECONDS` above 600, and after
/// that wait, which counts from its own start request, it needs at most three
/// minutes to boot the VM and a little to start the runner. Keep it in step
/// with `MANAGER_TART_START_TIMEOUT` in the Tart agent, whose tests compare
/// the two.
const TART_PROVISION_TIMEOUT: Duration = Duration::from_mins(15);

/// How long starting a runner on `provider` may take. A Tart runner can wait
/// for one of the Mac's VM slots, so it gets `TART_PROVISION_TIMEOUT`; a
/// Docker runner may first pull its image, so it gets `DOCKER_START_TIMEOUT`.
fn provision_time_limit(provider: &str) -> Duration {
    if provider == "tart" {
        TART_PROVISION_TIMEOUT
    } else {
        DOCKER_START_TIMEOUT
    }
}

/// The route's backstop for `POST /v1/runners`: a minute past the longer of
/// the two limits the handler enforces, so the handler's own answer, which
/// says what timed out, always comes first.
const PROVISION_ROUTE_TIMEOUT: Duration =
    longer(TART_PROVISION_TIMEOUT, DOCKER_START_TIMEOUT).saturating_add(Duration::from_mins(1));

const fn longer(first: Duration, second: Duration) -> Duration {
    if first.as_nanos() >= second.as_nanos() {
        first
    } else {
        second
    }
}

fn request_timeout(timeout: Duration) -> TimeoutLayer {
    TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, timeout)
}

/// Gives a request that ran out of time a body that says so. The timeout
/// layers answer `408` with no body, which reached operators as an empty
/// error; a `408` a handler produced already explains itself.
async fn explain_timeouts(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let started = tokio::time::Instant::now();
    let response = next.run(request).await;
    if response.status() != StatusCode::REQUEST_TIMEOUT
        || axum::body::HttpBody::size_hint(response.body()).exact() != Some(0)
    {
        return response;
    }
    ManagerError::TimedOut {
        code: "request_timeout",
        message: format!(
            "the runner manager stopped {method} {path} after {} seconds, its time limit for that request",
            started.elapsed().as_secs()
        ),
    }
    .into_response()
}

async fn health(
    State(state): State<ManagerState>,
    _auth: ManagerAuth,
) -> Result<Json<Value>, ManagerError> {
    let (status, tart) = match &state.tart {
        Some(agent) => match tart_agent_value(agent, reqwest::Method::GET, "v1/health", None).await
        {
            Ok(health) => {
                // The slots as admission counts them: the agent's view plus
                // VMs being started now, and the line for them.
                let vm_slots = match tart_capacity_from_health(&health) {
                    Ok(capacity) => {
                        let starting = vm_slot_reservations(&state).await;
                        state.vm_slots.report(&capacity.slot_usage(), starting)
                    }
                    Err(_) => Value::Null,
                };
                (
                    "ok",
                    json!({ "available": true, "health": health, "vmSlots": vm_slots }),
                )
            }
            Err(error) => {
                tracing::warn!(error = ?error, "Tart agent health check failed");
                (
                    "degraded",
                    json!({ "available": false, "error": "Tart agent is unavailable." }),
                )
            }
        },
        None => ("ok", json!({ "available": false, "configured": false })),
    };
    let builtin_host = json!({
        "enabled": state.builtin_host.setting(),
        "dockerAvailable": state.builtin_host.docker_available(),
    });
    if !state.builtin_host.docker_available() {
        // Running without Docker access: nothing on this host to measure.
        return Ok(Json(json!({
            "status": status,
            "providers": { "docker": { "available": false }, "tart": tart },
            "provisioningPaused": state.provisioning_paused.load(Ordering::Relaxed),
            "builtinHost": builtin_host,
        })));
    }
    state.docker.ping().await?;
    let version = state.docker.version().await?;
    // `capacity` describes the Docker host only; Tart reports its own budget
    // and usage under `providers.tart.health.capacity`.
    // `active` includes agent sandboxes, which hold runner slots on this host;
    // `agentSandboxes` breaks them out.
    let usage = docker_active_usage(&state.docker).await?;
    let reserved = reserved_capacity(&state, "docker").await;
    let disk = disk_capacity(&state.limits)?;
    let budget = state.effective_budget();
    Ok(Json(json!({
        "status": status,
        "dockerVersion": version.version,
        "apiVersion": version.api_version,
        "providers": { "docker": { "available": true }, "tart": tart },
        "hostName": state.limits.host_name,
        "availableCpus": state.limits.available_cpus,
        "totalMemoryMb": state.limits.total_memory_mb,
        "provisioningPaused": state.provisioning_paused.load(Ordering::Relaxed),
        "builtinHost": builtin_host,
        "capacity": {
            "cpuBudget": budget.budget.cpu,
            "memoryBudgetMb": budget.budget.memory_mb,
            "maxRunners": budget.budget.max_runners,
            "budgetSource": { "cpu": budget.cpu_source, "memory": budget.memory_source },
            "active": usage.total(),
            "reserved": reserved,
            "agentSandboxes": {
                "active": usage.sandboxes.active_runners,
                "cpu": usage.sandboxes.cpu,
                "memoryMb": usage.sandboxes.memory_mb,
            },
        },
        "disk": disk,
    })))
}

async fn update_policy(
    State(state): State<ManagerState>,
    _auth: ManagerAuth,
    Json(input): Json<ManagerPolicy>,
) -> Result<Json<Value>, ManagerError> {
    // Checked before anything is applied, so a refused policy changes nothing.
    if let Some(budget) = input.builtin_host_budget
        && (budget.cpu_millis.is_some_and(|millis| millis <= 0)
            || budget.memory_mib.is_some_and(|mib| mib <= 0))
    {
        return Err(ManagerError::BadRequest(
            "The built-in host's budget must be above zero; leave a figure null to use the manager's own.".into(),
        ));
    }
    if let Some(slots) = input.tart_vm_slots
        && !(1..=vm_slots::MAX_VM_SLOTS).contains(&slots)
    {
        return Err(ManagerError::BadRequest(format!(
            "The Tart host's macOS VM slots must be 1 to {}; macOS runs at most {} macOS VMs at once.",
            vm_slots::MAX_VM_SLOTS,
            vm_slots::MAX_VM_SLOTS
        )));
    }
    state
        .provisioning_paused
        .store(input.provisioning_paused, Ordering::Relaxed);
    if let Some(enabled) = input.builtin_host_enabled {
        state.builtin_host.set(enabled);
    }
    if let Some(slots) = input.tart_vm_slots
        && slots != state.vm_slots.limit()
    {
        state.vm_slots.set_limit(slots)?;
        tracing::info!(vm_slots = slots, "Tart host macOS VM slot limit changed");
    }
    if let Some(budget) = input.builtin_host_budget
        && budget != state.builtin_host.budget()
    {
        state.builtin_host.set_budget(budget);
        let effective = state.effective_budget();
        tracing::info!(
            cpu_budget = effective.budget.cpu,
            cpu_source = effective.cpu_source,
            memory_budget_mb = effective.budget.memory_mb,
            memory_source = effective.memory_source,
            max_runners = effective.budget.max_runners,
            "built-in runner host budget changed"
        );
    }
    Ok(Json(json!({
        "provisioningPaused": input.provisioning_paused,
        "builtinHostEnabled": state.builtin_host.setting(),
        "builtinHostBudget": state.effective_budget().to_json(),
        "tartVmSlots": state.vm_slots.limit(),
    })))
}

async fn reserve_capacity(
    State(state): State<ManagerState>,
    _auth: ManagerAuth,
    Json(input): Json<CapacityRequest>,
) -> Result<(StatusCode, Json<Value>), ManagerError> {
    input.validate()?;
    if input.provider == "docker" {
        // Agent sandboxes reserve under their own pool and need Docker, not
        // the built-in runner host; every other Docker admission is a runner.
        if input.pool_id == sandbox::SANDBOX_POOL_ID {
            state.builtin_host.require_docker()?;
        } else {
            state.builtin_host.require_runners()?;
        }
    }
    let _guard = state.provision_locks.lock(&input.provider).await;
    ensure_provisioning_enabled(&state)?;
    // Each provider is admitted against its own host. Tart VMs run on the Mac,
    // not inside the Docker host, and the agent enforces its own budget; adding
    // them to the Docker budget starved Linux runners of capacity.
    let (budget, active, disk, slot_usage) = if input.provider == "tart" {
        let agent = state.tart.as_ref().ok_or_else(tart_provider_disabled)?;
        let capacity = tart_capacity(agent).await?;
        (
            capacity.budget(),
            capacity.active,
            None,
            Some(capacity.slot_usage()),
        )
    } else {
        (
            state.docker_budget(),
            docker_active_capacity(&state.docker).await?,
            Some(disk_capacity(&state.limits)?),
            None,
        )
    };
    let mut reservations = state.reservations.lock().await;
    reservations.retain(|_, reservation| reservation.expires_at > Instant::now());
    let reserved = provider_reservations(&reservations, &input.provider);
    let requested = CapacityUsage {
        active_runners: 1,
        cpu: input.cpu_limit,
        memory_mb: input.memory_limit_mb,
    };
    // A Tart VM waits in the one line for the Mac, for a VM slot or for room
    // in GridOps's own budget there, under the same lock as the reservation,
    // so two pools cannot both take the last and the pool asked first cannot
    // jump a pool that has waited longer.
    let vm_slot = input.needs_vm_slot();
    if vm_slot {
        let starting = reservations
            .values()
            .filter(|reservation| reservation.vm_slot)
            .count();
        state.vm_slots.admit(
            &input.pool_id,
            input.reconcile_pass,
            &slot_usage.unwrap_or_default(),
            starting,
            &tart_budget_room(budget, active, reserved, requested),
        )?;
    }
    enforce_capacity(budget, active, reserved, requested, disk)?;
    let lease_id = uuid::Uuid::new_v4().to_string();
    reservations.insert(
        lease_id.clone(),
        CapacityReservation {
            runner_id: input.runner_id,
            pool_id: input.pool_id,
            provider: input.provider,
            cpu_limit: input.cpu_limit,
            memory_limit_mb: input.memory_limit_mb,
            vm_slot,
            expires_at: Instant::now() + Duration::from_mins(5),
        },
    );
    Ok((
        StatusCode::CREATED,
        Json(json!({ "leaseId": lease_id, "expiresInSeconds": 300 })),
    ))
}

async fn release_capacity(
    State(state): State<ManagerState>,
    Path(lease_id): Path<String>,
    _auth: ManagerAuth,
) -> Json<Value> {
    let released = state.reservations.lock().await.remove(&lease_id).is_some();
    Json(json!({ "released": released }))
}

async fn list_runners(
    State(state): State<ManagerState>,
    _auth: ManagerAuth,
) -> Result<Json<Value>, ManagerError> {
    let filters = HashMap::from([(
        "label".to_owned(),
        vec!["io.gridops.managed=true".to_owned()],
    )]);
    let options = ListContainersOptionsBuilder::default()
        .all(true)
        .filters(&filters)
        .build();
    // Without Docker access there are no containers to list; Tart runners are
    // still reported so the reconciler keeps working.
    let containers = if state.builtin_host.docker_available() {
        state.docker.list_containers(Some(options)).await?
    } else {
        Vec::new()
    };
    let mut runners = containers
        .into_iter()
        .map(|container| ManagedRunner {
            id: container.id.unwrap_or_default(),
            names: container.names.unwrap_or_default(),
            image: container.image.unwrap_or_default(),
            state: container
                .state
                .map_or_else(String::new, |value| value.to_string()),
            status: container.status.unwrap_or_default(),
            labels: container.labels.unwrap_or_default(),
            created_at: chrono::DateTime::from_timestamp(container.created.unwrap_or_default(), 0)
                .map_or_else(String::new, |value| value.to_rfc3339()),
            provider: "docker".into(),
        })
        .collect::<Vec<_>>();
    if let Some(agent) = &state.tart {
        let response = tart_agent_value(agent, reqwest::Method::GET, "v1/runners", None).await?;
        let tart_runners = response.get("runners").cloned().ok_or_else(|| {
            ManagerError::Internal(anyhow::anyhow!("Tart agent runner list is invalid"))
        })?;
        runners.extend(
            serde_json::from_value::<Vec<ManagedRunner>>(tart_runners)
                .map_err(|error| ManagerError::Internal(error.into()))?,
        );
    }
    Ok(Json(json!({ "runners": runners })))
}

/// Capabilities dropped from every provisioned runner container.
fn runner_cap_drop() -> Vec<String> {
    vec!["ALL".into()]
}

/// Capabilities added back on top of `runner_cap_drop()`.
///
/// The runner images this provisions (e.g. myoung34/github-runner derivatives) run with
/// `RUN_AS_ROOT=true` by default, and nothing here overrides that, so jobs execute directly as
/// root with no chown/gosu privilege drop. Dropping every capability from that root process
/// makes the kernel enforce normal DAC checks against it instead of letting root bypass them,
/// so even `touch`/`mkdir` under the runner's own working directory fails with "Permission
/// denied" despite `id` reporting uid=0, and the container exits with code 126.
/// Job steps routinely need more than `DAC_OVERRIDE`'s raw read/write bypass: `actions/setup-*`
/// actions unpack a downloaded tool into a cache directory and then `chown` it, and build steps
/// may `chmod`/change ownership of files they create. Without CHOWN/FOWNER/SETUID/SETGID those
/// calls fail even as uid 0, since dropping ALL capabilities also strips root's usual exemption
/// from capability checks (it does not just restore DAC checks). This set matches
/// `sandbox::SANDBOX_CAPABILITIES`, which grants the same root-run build sandbox exactly these
/// five for the same reason.
fn runner_cap_add() -> Vec<String> {
    vec![
        "CHOWN".into(),
        "DAC_OVERRIDE".into(),
        "FOWNER".into(),
        "SETUID".into(),
        "SETGID".into(),
    ]
}

async fn provision_runner(
    State(state): State<ManagerState>,
    auth: ManagerAuth,
    Json(input): Json<ProvisionRunner>,
) -> Result<(StatusCode, Json<Value>), ManagerError> {
    let limit = provision_time_limit(&input.provider);
    let provider = input.provider.clone();
    let lease_id = input.capacity_lease.clone();
    if let Ok(result) =
        tokio::time::timeout(limit, provision_runner_in_time(state.clone(), auth, input)).await
    {
        return result;
    }
    // Abandoned part way: give the capacity back as finishing would have.
    state.reservations.lock().await.remove(&lease_id);
    let seconds = limit.as_secs();
    Err(if provider == "tart" {
        tracing::warn!(
            seconds,
            "the Tart agent did not finish a runner start in time"
        );
        ManagerError::TimedOut {
            code: "tart_start_timeout",
            message: format!(
                "the Tart agent did not finish starting the macOS runner within {seconds} seconds, the runner manager's limit for a Tart runner start; see the Tart agent's log on the Mac"
            ),
        }
    } else {
        ManagerError::TimedOut {
            code: "runner_start_timeout",
            message: format!(
                "the runner manager did not finish starting the {provider} runner within {seconds} seconds, its limit for a runner start"
            ),
        }
    })
}

async fn provision_runner_in_time(
    state: ManagerState,
    _auth: ManagerAuth,
    mut input: ProvisionRunner,
) -> Result<(StatusCode, Json<Value>), ManagerError> {
    input.validate()?;
    if input.provider != "tart" {
        state.builtin_host.require_runners()?;
    }
    let _guard = state.provision_locks.lock(&input.provider).await;
    ensure_provisioning_enabled(&state)?;
    if input.network != state.limits.runner_network {
        return Err(ManagerError::Forbidden(
            "Runner containers may only join the configured GridOps runner network.".into(),
        ));
    }
    let lease_id = input.capacity_lease.clone();
    validate_capacity_lease(&state, &input.lease_claim()).await?;
    if input.provider == "tart" {
        let result = match &state.tart {
            Some(agent) => {
                let body = serde_json::to_value(&input)
                    .map_err(|error| ManagerError::Internal(error.into()))?;
                tart_agent_value(agent, reqwest::Method::POST, "v1/runners", Some(body))
                    .await
                    .map(|value| (StatusCode::CREATED, Json(value)))
            }
            None => Err(tart_provider_disabled()),
        };
        state.reservations.lock().await.remove(&lease_id);
        return result;
    }
    let result = provision_runner_inner(&state, &mut input).await;
    state.reservations.lock().await.remove(&lease_id);
    result
}

async fn provision_runner_inner(
    state: &ManagerState,
    input: &mut ProvisionRunner,
) -> Result<(StatusCode, Json<Value>), ManagerError> {
    let bootstrap_secret = take_bootstrap_secret(input)?;
    ensure_network(&state.docker, &input.network).await?;
    ensure_image(&state.docker, &input.image, input.pull_image).await?;
    ensure_disk_capacity(&state.limits)?;

    let filters = HashMap::from([("name".to_owned(), vec![input.name.clone()])]);
    let existing = state
        .docker
        .list_containers(Some(
            ListContainersOptionsBuilder::default()
                .all(true)
                .filters(&filters)
                .build(),
        ))
        .await?;
    if existing.iter().any(|container| {
        container
            .names
            .as_ref()
            .is_some_and(|names| names.iter().any(|name| name == &format!("/{}", input.name)))
    }) {
        return Err(ManagerError::Conflict(
            "A runner container with this name already exists.".into(),
        ));
    }

    let memory_bytes = input.memory_limit_mb * 1_024 * 1_024;
    let (cmd, mut env) = runner_command(input)?;
    // Images that run as root (myoung34's) export this from the entrypoint skipped below,
    // and the runner's own run-helper.sh refuses to start as root without it, exiting 0
    // with "Must not run interactively with sudo" and no indication anything is wrong.
    // GitHub's image runs as `runner`, which this doesn't affect.
    env.push("RUNNER_ALLOW_RUNASROOT=1".into());
    let labels = HashMap::from([
        ("io.gridops.managed".into(), "true".into()),
        ("io.gridops.runner-id".into(), input.runner_id.clone()),
        ("io.gridops.pool-id".into(), input.pool_id.clone()),
        ("io.gridops.mode".into(), input.mode.clone()),
    ]);
    let body = ContainerCreateBody {
        image: Some(input.image.clone()),
        // An empty entrypoint runs `cmd` as given. myoung34/github-runner's
        // `/entrypoint.sh` would otherwise take it as arguments and first run a
        // registration from env vars GridOps never sets, which fails before
        // `cmd` runs. GitHub's image has no entrypoint, so this changes nothing
        // there.
        entrypoint: Some(vec![String::new()]),
        cmd: Some(cmd),
        env: Some(env),
        attach_stdin: Some(true),
        open_stdin: Some(true),
        labels: Some(labels),
        host_config: Some(HostConfig {
            auto_remove: Some(false),
            network_mode: Some(input.network.clone()),
            nano_cpus: Some((input.cpu_limit * 1_000_000_000.0) as i64),
            memory: Some(memory_bytes),
            memory_swap: Some(
                memory_bytes.saturating_add(memory_bytes * state.limits.runner_swap_percent / 100),
            ),
            pids_limit: Some(state.limits.runner_pids_limit),
            binds: state
                .shared_docker_socket
                .as_ref()
                .map(|socket| vec![format!("{}:/var/run/docker.sock", socket.host_path)]),
            group_add: state
                .shared_docker_socket
                .as_ref()
                .map(|socket| vec![socket.group_id.to_string()]),
            // Docker's own init reaps the job's orphaned processes, whatever
            // the image ships.
            init: Some(true),
            cap_drop: Some(runner_cap_drop()),
            cap_add: Some(runner_cap_add()),
            security_opt: Some(vec!["no-new-privileges:true".into()]),
            oom_score_adj: Some(500),
            log_config: Some(HostConfigLogConfig {
                typ: Some("json-file".into()),
                config: Some(HashMap::from([
                    ("max-size".into(), state.limits.log_max_size.clone()),
                    ("max-file".into(), state.limits.log_max_files.to_string()),
                ])),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let container = state
        .docker
        .create_container(
            Some(
                CreateContainerOptionsBuilder::default()
                    .name(&input.name)
                    .build(),
            ),
            body,
        )
        .await?;
    if let Err(error) = state
        .docker
        .start_container(&container.id, None::<StartContainerOptions>)
        .await
    {
        let _ = state
            .docker
            .remove_container(
                &container.id,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await;
        return Err(error.into());
    }
    let bootstrap_result = async {
        let mut connection = state
            .docker
            .attach_container(
                &container.id,
                Some(
                    AttachContainerOptionsBuilder::default()
                        .stdin(true)
                        .stream(true)
                        .build(),
                ),
            )
            .await?;
        connection
            .input
            .write_all(bootstrap_secret.expose_secret().as_bytes())
            .await
            .map_err(|error| ManagerError::Internal(error.into()))?;
        connection
            .input
            .write_all(b"\n")
            .await
            .map_err(|error| ManagerError::Internal(error.into()))?;
        connection
            .input
            .shutdown()
            .await
            .map_err(|error| ManagerError::Internal(error.into()))?;
        Result::<(), ManagerError>::Ok(())
    }
    .await;
    if let Err(error) = bootstrap_result {
        let _ = state
            .docker
            .remove_container(
                &container.id,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await;
        return Err(error);
    }
    let details = state.docker.inspect_container(&container.id, None).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": details.id.unwrap_or(container.id),
            "name": details.name.unwrap_or_default().trim_start_matches('/'),
            "state": details.state.and_then(|value| value.status).map_or_else(|| "unknown".into(), |value| value.to_string()),
            "createdAt": details.created,
        })),
    ))
}

async fn control_runner(
    State(state): State<ManagerState>,
    Path((container_id, action)): Path<(String, String)>,
    _auth: ManagerAuth,
) -> Result<Json<Value>, ManagerError> {
    if is_tart_id(&container_id) {
        let agent = state.tart.as_ref().ok_or_else(|| ManagerError::Upstream {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "tart_provider_disabled".into(),
            message: "The Tart macOS runner provider is not configured.".into(),
        })?;
        return tart_agent_value(
            agent,
            reqwest::Method::POST,
            &format!("v1/runners/{container_id}/{action}"),
            None,
        )
        .await
        .map(Json);
    }
    validate_container_id(&container_id)?;
    // Bringing a container back to life puts work on this host again.
    if matches!(action.as_str(), "start" | "resume" | "restart") {
        state.builtin_host.require_runners()?;
    }
    state.builtin_host.require_docker()?;
    let labels = managed_container_labels(&state.docker, &container_id).await?;
    if matches!(action.as_str(), "start" | "restart")
        && labels.get("io.gridops.mode").map(String::as_str) == Some("ephemeral")
    {
        return Err(ManagerError::Conflict(
            "Ephemeral runners cannot be started or restarted; rebuild the runner instead.".into(),
        ));
    }
    let _guard = if matches!(action.as_str(), "start" | "resume" | "restart") {
        Some(state.provision_locks.lock("docker").await)
    } else {
        None
    };
    if matches!(action.as_str(), "start" | "resume" | "restart") {
        ensure_provisioning_enabled(&state)?;
    }
    if action == "start" {
        let details = state.docker.inspect_container(&container_id, None).await?;
        let current_state = details
            .state
            .as_ref()
            .and_then(|value| value.status.as_ref())
            .map_or_else(String::new, ToString::to_string);
        if !active_container_state(&current_state) {
            let host = details.host_config.unwrap_or_default();
            let requested = CapacityUsage {
                active_runners: 1,
                cpu: host.nano_cpus.unwrap_or_default() as f64 / 1_000_000_000.0,
                memory_mb: host.memory.unwrap_or_default() / 1_024 / 1_024,
            };
            enforce_capacity(
                state.docker_budget(),
                docker_active_capacity(&state.docker).await?,
                reserved_capacity(&state, "docker").await,
                requested,
                Some(disk_capacity(&state.limits)?),
            )?;
        }
    }
    let status = match action.as_str() {
        "start" => {
            state
                .docker
                .start_container(&container_id, None::<StartContainerOptions>)
                .await
                .or_else(ignore_not_modified)?;
            "running"
        }
        "stop" => {
            state
                .docker
                .stop_container(
                    &container_id,
                    Some(StopContainerOptionsBuilder::default().t(30).build()),
                )
                .await
                .or_else(ignore_not_modified)?;
            "stopped"
        }
        "pause" => {
            state.docker.pause_container(&container_id).await?;
            "paused"
        }
        "resume" => {
            state.docker.unpause_container(&container_id).await?;
            "running"
        }
        "restart" => {
            state
                .docker
                .restart_container(
                    &container_id,
                    Some(RestartContainerOptionsBuilder::default().t(30).build()),
                )
                .await?;
            "running"
        }
        _ => {
            return Err(ManagerError::BadRequest(
                "Runner action must be start, stop, pause, resume, or restart.".into(),
            ));
        }
    };
    Ok(Json(json!({ "status": status })))
}

async fn logs(
    State(state): State<ManagerState>,
    Path(container_id): Path<String>,
    Query(query): Query<LogsQuery>,
    _auth: ManagerAuth,
) -> Result<Response, ManagerError> {
    let tail = query.tail.unwrap_or_else(|| "500".into());
    if !valid_log_tail(&tail) {
        return Err(ManagerError::BadRequest(
            "Log tail must be all or a number from 0 to 100000.".into(),
        ));
    }
    if is_tart_id(&container_id) {
        let agent = state.tart.as_ref().ok_or_else(|| ManagerError::Upstream {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "tart_provider_disabled".into(),
            message: "The Tart macOS runner provider is not configured.".into(),
        })?;
        let path = format!(
            "v1/runners/{container_id}/logs?tail={tail}&follow={}",
            query.follow
        );
        let response = tart_agent_response(agent, reqwest::Method::GET, &path, None).await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(ManagerError::Upstream {
                status,
                code: "tart_log_error".into(),
                message: text.chars().take(2_000).collect(),
            });
        }
        return Ok((
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store"),
                (header::HeaderName::from_static("x-accel-buffering"), "no"),
            ],
            Body::from_stream(response.bytes_stream()),
        )
            .into_response());
    }
    validate_container_id(&container_id)?;
    managed_container_labels(&state.docker, &container_id).await?;
    let options = LogsOptionsBuilder::default()
        .stdout(true)
        .stderr(true)
        .timestamps(true)
        .follow(query.follow)
        .tail(&tail)
        .build();
    let stream = state
        .docker
        .logs(&container_id, Some(options))
        .map(|result| result.map(|line| line.to_string()));
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

async fn delete_runner(
    State(state): State<ManagerState>,
    Path(container_id): Path<String>,
    _auth: ManagerAuth,
) -> Result<Json<Value>, ManagerError> {
    if is_tart_id(&container_id) {
        let agent = state.tart.as_ref().ok_or_else(|| ManagerError::Upstream {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "tart_provider_disabled".into(),
            message: "The Tart macOS runner provider is not configured.".into(),
        })?;
        return tart_agent_value(
            agent,
            reqwest::Method::DELETE,
            &format!("v1/runners/{container_id}"),
            None,
        )
        .await
        .map(Json);
    }
    validate_container_id(&container_id)?;
    state.builtin_host.require_docker()?;
    managed_container_labels(&state.docker, &container_id).await?;
    state
        .docker
        .remove_container(
            &container_id,
            Some(
                RemoveContainerOptionsBuilder::default()
                    .force(true)
                    .v(true)
                    .build(),
            ),
        )
        .await?;
    Ok(Json(json!({ "status": "deleted" })))
}

async fn ensure_network(docker: &Docker, name: &str) -> Result<(), ManagerError> {
    match docker.inspect_network(name, None).await {
        Ok(_) => Ok(()),
        Err(DockerError::DockerResponseServerError {
            status_code: 404, ..
        }) => {
            docker
                .create_network(NetworkCreateRequest {
                    name: name.into(),
                    driver: Some("bridge".into()),
                    internal: Some(false),
                    labels: Some(HashMap::from([(
                        "io.gridops.managed".into(),
                        "true".into(),
                    )])),
                    ..Default::default()
                })
                .await?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// A first pull of a runner image can take minutes; Docker's default client timeout is two.
const IMAGE_PULL_TIMEOUT: Duration = Duration::from_mins(10);
/// What starting a container needs besides pulling its image: checking the
/// network and the name, creating, starting and attaching to it. Each of those
/// is one Docker call and usually takes seconds; two minutes is one call's
/// client timeout (the manager connects to Docker with 120 seconds).
const CONTAINER_START_ALLOWANCE: Duration = Duration::from_mins(2);
/// How long starting a Docker runner or creating an agent sandbox may take:
/// the image pull it may begin with, then the container start. The time spent
/// waiting for the Docker start lock counts too.
const DOCKER_START_TIMEOUT: Duration = IMAGE_PULL_TIMEOUT.saturating_add(CONTAINER_START_ALLOWANCE);
const SANDBOX_CREATE_TIMEOUT: Duration = DOCKER_START_TIMEOUT;

async fn ensure_image(docker: &Docker, image: &str, pull_image: bool) -> Result<(), ManagerError> {
    let puller = docker.clone().with_timeout(IMAGE_PULL_TIMEOUT);
    match docker.inspect_image(image).await {
        Ok(_) if !pull_image => Ok(()),
        Ok(_) => match puller
            .create_image(
                Some(
                    CreateImageOptionsBuilder::default()
                        .from_image(image)
                        .build(),
                ),
                None,
                None,
            )
            .try_collect::<Vec<_>>()
            .await
        {
            Ok(_) => Ok(()),
            // A local copy exists, so a registry outage, auth failure, or
            // local-only tag must not fail provisioning.
            Err(error) => {
                tracing::warn!(
                    image,
                    error = %error,
                    "could not refresh local runner image; continuing with the available local image"
                );
                Ok(())
            }
        },
        Err(DockerError::DockerResponseServerError {
            status_code: 404, ..
        }) => {
            puller
                .create_image(
                    Some(
                        CreateImageOptionsBuilder::default()
                            .from_image(image)
                            .build(),
                    ),
                    None,
                    None,
                )
                .try_collect::<Vec<_>>()
                .await?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn ensure_provisioning_enabled(state: &ManagerState) -> Result<(), ManagerError> {
    if state.provisioning_paused.load(Ordering::Relaxed) {
        return Err(ManagerError::Guardrail {
            code: "provisioning_paused",
            message: "Runner provisioning is globally paused.".into(),
        });
    }
    Ok(())
}

/// What a provisioning request says its capacity lease was reserved for.
struct LeaseClaim<'a> {
    lease_id: &'a str,
    runner_id: &'a str,
    pool_id: &'a str,
    provider: &'a str,
    cpu_limit: f64,
    memory_limit_mb: i64,
}

impl LeaseClaim<'_> {
    fn matches(&self, reservation: &CapacityReservation) -> bool {
        reservation.runner_id == self.runner_id
            && reservation.pool_id == self.pool_id
            && reservation.provider == self.provider
            && (reservation.cpu_limit - self.cpu_limit).abs() <= f64::EPSILON
            && reservation.memory_limit_mb == self.memory_limit_mb
    }
}

impl ProvisionRunner {
    fn lease_claim(&self) -> LeaseClaim<'_> {
        LeaseClaim {
            lease_id: &self.capacity_lease,
            runner_id: &self.runner_id,
            pool_id: &self.pool_id,
            provider: &self.provider,
            cpu_limit: self.cpu_limit,
            memory_limit_mb: self.memory_limit_mb,
        }
    }
}

async fn validate_capacity_lease(
    state: &ManagerState,
    claim: &LeaseClaim<'_>,
) -> Result<(), ManagerError> {
    let mut reservations = state.reservations.lock().await;
    reservations.retain(|_, reservation| reservation.expires_at > Instant::now());
    let reservation = reservations
        .get(claim.lease_id)
        .ok_or_else(|| ManagerError::Guardrail {
            code: "capacity_lease_invalid",
            message: "Runner capacity reservation is missing or expired.".into(),
        })?;
    if !claim.matches(reservation) {
        return Err(ManagerError::Forbidden(
            "Runner capacity reservation does not match the provisioning request.".into(),
        ));
    }
    Ok(())
}

/// Docker host usage split by workload. Runners and agent sandboxes share the
/// host, so admission charges their combined total against one budget.
#[derive(Clone, Copy, Debug, Default)]
struct DockerUsage {
    runners: CapacityUsage,
    sandboxes: CapacityUsage,
}

impl DockerUsage {
    fn total(self) -> CapacityUsage {
        CapacityUsage {
            active_runners: self
                .runners
                .active_runners
                .saturating_add(self.sandboxes.active_runners),
            cpu: self.runners.cpu + self.sandboxes.cpu,
            memory_mb: self
                .runners
                .memory_mb
                .saturating_add(self.sandboxes.memory_mb),
        }
    }
}

async fn docker_active_capacity(docker: &Docker) -> Result<CapacityUsage, ManagerError> {
    Ok(docker_active_usage(docker).await?.total())
}

async fn docker_active_usage(docker: &Docker) -> Result<DockerUsage, ManagerError> {
    let runners = active_container_ids(docker, "io.gridops.managed=true").await?;
    let sandboxes = active_container_ids(docker, &format!("{}=true", sandbox::SANDBOX_LABEL))
        .await?
        .into_iter()
        .filter(|id| !runners.contains(id))
        .collect();
    Ok(DockerUsage {
        runners: containers_usage(docker, runners).await?,
        sandboxes: containers_usage(docker, sandboxes).await?,
    })
}

async fn active_container_ids(docker: &Docker, label: &str) -> Result<Vec<String>, ManagerError> {
    let filters = HashMap::from([("label".to_owned(), vec![label.to_owned()])]);
    let containers = docker
        .list_containers(Some(
            ListContainersOptionsBuilder::default()
                .all(true)
                .filters(&filters)
                .build(),
        ))
        .await?;
    Ok(containers
        .into_iter()
        .filter(|container| {
            container
                .state
                .as_ref()
                .is_some_and(|state| active_container_state(state.as_ref()))
        })
        .filter_map(|container| container.id)
        .collect())
}

async fn containers_usage(
    docker: &Docker,
    ids: Vec<String>,
) -> Result<CapacityUsage, ManagerError> {
    let details = futures_util::stream::iter(ids)
        .map(|id| async move {
            match docker.inspect_container(&id, None).await {
                Ok(details) => Ok(Some(details)),
                // Removed between listing and inspection, so it no longer
                // holds capacity.
                Err(DockerError::DockerResponseServerError {
                    status_code: 404, ..
                }) => Ok(None),
                Err(error) => Err(error),
            }
        })
        .buffer_unordered(16)
        .try_collect::<Vec<_>>()
        .await?;
    let mut usage = CapacityUsage::default();
    for details in details.into_iter().flatten() {
        let host = details.host_config.unwrap_or_default();
        usage.active_runners += 1;
        usage.cpu += host.nano_cpus.unwrap_or_default() as f64 / 1_000_000_000.0;
        usage.memory_mb = usage
            .memory_mb
            .saturating_add(host.memory.unwrap_or_default() / 1_024 / 1_024);
    }
    Ok(usage)
}

async fn tart_capacity(agent: &TartAgent) -> Result<TartCapacity, ManagerError> {
    let health = tart_agent_value(agent, reqwest::Method::GET, "v1/health", None).await?;
    tart_capacity_from_health(&health)
}

fn tart_capacity_from_health(health: &Value) -> Result<TartCapacity, ManagerError> {
    let capacity = health.get("capacity").cloned().ok_or_else(|| {
        ManagerError::Internal(anyhow::anyhow!("Tart agent health omitted capacity"))
    })?;
    serde_json::from_value::<TartCapacity>(capacity)
        .map_err(|error| ManagerError::Internal(error.into()))
}

impl TartCapacity {
    fn budget(&self) -> CapacityBudget {
        CapacityBudget {
            cpu: self.cpu_budget,
            memory_mb: self.memory_budget_mb,
            max_runners: self.max_runners,
        }
    }

    /// Who holds the slots. An agent that predates slot reporting is taken
    /// to run every active runner in a VM and to see no other software's.
    fn slot_usage(&self) -> vm_slots::AgentSlotUsage {
        self.vm_slots
            .clone()
            .unwrap_or_else(|| vm_slots::AgentSlotUsage {
                gridops: self.active.active_runners,
                ..vm_slots::AgentSlotUsage::default()
            })
    }
}

/// Admissions holding a VM slot for a VM that is being started.
async fn vm_slot_reservations(state: &ManagerState) -> usize {
    let mut reservations = state.reservations.lock().await;
    reservations.retain(|_, reservation| reservation.expires_at > Instant::now());
    reservations
        .values()
        .filter(|reservation| reservation.vm_slot)
        .count()
}

async fn reserved_capacity(state: &ManagerState, provider: &str) -> CapacityUsage {
    let mut reservations = state.reservations.lock().await;
    reservations.retain(|_, reservation| reservation.expires_at > Instant::now());
    provider_reservations(&reservations, provider)
}

fn provider_reservations(
    reservations: &HashMap<String, CapacityReservation>,
    provider: &str,
) -> CapacityUsage {
    reservations
        .values()
        .filter(|reservation| reservation.provider == provider)
        .fold(CapacityUsage::default(), |mut usage, reservation| {
            usage.active_runners += 1;
            usage.cpu += reservation.cpu_limit;
            usage.memory_mb = usage.memory_mb.saturating_add(reservation.memory_limit_mb);
            usage
        })
}

fn tart_provider_disabled() -> ManagerError {
    ManagerError::Upstream {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "tart_provider_disabled".into(),
        message: "The Tart macOS runner provider is not configured.".into(),
    }
}

fn active_container_state(state: &str) -> bool {
    matches!(state, "created" | "running" | "restarting" | "paused")
}

fn disk_capacity(limits: &ManagerLimits) -> Result<DiskCapacity, ManagerError> {
    let root = FsPath::new("/");
    let output = Command::new("df")
        .args(["-Pk", root.as_os_str().to_string_lossy().as_ref()])
        .output()
        .map_err(|error| ManagerError::Internal(error.into()))?;
    if !output.status.success() {
        return Err(ManagerError::Internal(anyhow::anyhow!(
            "could not inspect runner-host disk capacity"
        )));
    }
    let stdout =
        String::from_utf8(output.stdout).map_err(|error| ManagerError::Internal(error.into()))?;
    let (total_mb, available_mb) = parse_df_capacity(&stdout).ok_or_else(|| {
        ManagerError::Internal(anyhow::anyhow!("runner-host disk capacity was invalid"))
    })?;
    Ok(DiskCapacity {
        total_mb,
        available_mb,
        minimum_free_mb: free_disk_floor_mb(
            limits.min_free_disk_mb,
            limits.min_free_disk_percent,
            total_mb,
        ),
    })
}

/// The absolute floor, lowered to `percent` of the disk on disks too small for
/// it so small hosts still admit runners. A `percent` of 0 keeps the absolute
/// floor everywhere.
fn free_disk_floor_mb(minimum_mb: u64, percent: u64, total_mb: u64) -> u64 {
    if percent == 0 {
        return minimum_mb;
    }
    minimum_mb.min(total_mb.saturating_mul(percent) / 100)
}

fn parse_df_capacity(output: &str) -> Option<(u64, u64)> {
    let row = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .nth(1)?;
    let columns = row.split_whitespace().collect::<Vec<_>>();
    let total_kb = columns.get(1)?.parse::<u64>().ok()?;
    let available_kb = columns.get(3)?.parse::<u64>().ok()?;
    Some((total_kb / 1_024, available_kb / 1_024))
}

fn ensure_disk_capacity(limits: &ManagerLimits) -> Result<(), ManagerError> {
    let disk = disk_capacity(limits)?;
    if disk.available_mb < disk.minimum_free_mb {
        return Err(ManagerError::Guardrail {
            code: "host_disk_guardrail",
            message: format!(
                "Runner provisioning stopped because only {} MB disk space remains; {} MB is reserved.",
                disk.available_mb, disk.minimum_free_mb
            ),
        });
    }
    Ok(())
}

/// How many more runners the size of `requested` the budget has room for, or
/// none with the reason it gives.
fn tart_budget_room(
    budget: CapacityBudget,
    active: CapacityUsage,
    reserved: CapacityUsage,
    requested: CapacityUsage,
) -> vm_slots::BudgetRoom {
    if let Err(refusal) = enforce_capacity(budget, active, reserved, requested, None) {
        let refusal = match refusal {
            ManagerError::Guardrail { message, .. } => message,
            _ => "GridOps's own budget on the Tart host is full.".into(),
        };
        return vm_slots::BudgetRoom {
            room: 0,
            refusal: Some(refusal),
        };
    }
    let runners = budget.max_runners.saturating_sub(
        active
            .active_runners
            .saturating_add(reserved.active_runners),
    );
    let cpu = ((budget.cpu - active.cpu - reserved.cpu + f64::EPSILON) / requested.cpu)
        .floor()
        .max(0.0) as usize;
    let memory = usize::try_from(
        (budget.memory_mb - active.memory_mb - reserved.memory_mb).max(0)
            / requested.memory_mb.max(1),
    )
    .unwrap_or(0);
    vm_slots::BudgetRoom {
        room: runners.min(cpu).min(memory),
        refusal: None,
    }
}

/// `disk` is `None` for providers whose host enforces its own disk reserve.
fn enforce_capacity(
    budget: CapacityBudget,
    active: CapacityUsage,
    reserved: CapacityUsage,
    requested: CapacityUsage,
    disk: Option<DiskCapacity>,
) -> Result<(), ManagerError> {
    if let Some(disk) = disk
        && disk.available_mb < disk.minimum_free_mb
    {
        return Err(ManagerError::Guardrail {
            code: "host_disk_guardrail",
            message: format!(
                "Runner provisioning stopped because only {} MB disk space remains; {} MB is reserved.",
                disk.available_mb, disk.minimum_free_mb
            ),
        });
    }
    let projected_runners = active
        .active_runners
        .saturating_add(reserved.active_runners)
        .saturating_add(requested.active_runners);
    if projected_runners > budget.max_runners {
        return Err(ManagerError::Guardrail {
            code: "host_capacity_exhausted",
            message: format!(
                "Host runner limit reached ({projected_runners}/{} including reservations).",
                budget.max_runners
            ),
        });
    }
    let projected_cpu = active.cpu + reserved.cpu + requested.cpu;
    if projected_cpu > budget.cpu + f64::EPSILON {
        return Err(ManagerError::Guardrail {
            code: "host_capacity_exhausted",
            message: format!(
                "Host CPU budget would be exceeded ({projected_cpu:.2}/{:.2} CPUs including reservations).",
                budget.cpu
            ),
        });
    }
    let projected_memory = active
        .memory_mb
        .saturating_add(reserved.memory_mb)
        .saturating_add(requested.memory_mb);
    if projected_memory > budget.memory_mb {
        return Err(ManagerError::Guardrail {
            code: "host_capacity_exhausted",
            message: format!(
                "Host memory budget would be exceeded ({projected_memory}/{} MB including reservations).",
                budget.memory_mb
            ),
        });
    }
    Ok(())
}

fn valid_log_size(value: &str) -> bool {
    let Some((number, suffix)) = value.split_at_checked(value.len().saturating_sub(1)) else {
        return false;
    };
    matches!(suffix, "k" | "m" | "g") && number.parse::<u64>().is_ok_and(|size| size > 0)
}

fn runner_command(input: &ProvisionRunner) -> Result<(Vec<String>, Vec<String>), ManagerError> {
    match input.mode.as_str() {
        "ephemeral" => Ok((
            vec![
                "/bin/bash".into(),
                "-lc".into(),
                instrumented_runner_script(
                    r#"set -euo pipefail
IFS= read -r GRIDOPS_BOOTSTRAP_SECRET
[ -n "$GRIDOPS_BOOTSTRAP_SECRET" ]
runner_args=(--jitconfig "$GRIDOPS_BOOTSTRAP_SECRET")
unset GRIDOPS_BOOTSTRAP_SECRET"#,
                    r#"./run.sh "${runner_args[@]}""#,
                ),
            ],
            vec![],
        )),
        "persistent" => {
            let registration_url = input.registration_url.as_ref().ok_or_else(|| {
                ManagerError::BadRequest("Runner registration URL is required.".into())
            })?;
            let script = r#"set -euo pipefail
if [ ! -f .runner ]; then
IFS= read -r GRIDOPS_BOOTSTRAP_SECRET
[ -n "$GRIDOPS_BOOTSTRAP_SECRET" ]
args=(--unattended --url "$GRIDOPS_REGISTRATION_URL" --token "$GRIDOPS_BOOTSTRAP_SECRET" --name "$GRIDOPS_RUNNER_NAME" --labels "$GRIDOPS_RUNNER_LABELS" --work _work --replace --disableupdate)
if [ -n "${GRIDOPS_RUNNER_GROUP:-}" ]; then args+=(--runnergroup "$GRIDOPS_RUNNER_GROUP"); fi
./config.sh "${args[@]}"
unset GRIDOPS_BOOTSTRAP_SECRET
fi"#;
            Ok((
                vec![
                    "/bin/bash".into(),
                    "-lc".into(),
                    instrumented_runner_script(script, "./run.sh"),
                ],
                vec![
                    format!("GRIDOPS_REGISTRATION_URL={registration_url}"),
                    format!("GRIDOPS_RUNNER_NAME={}", input.name),
                    format!("GRIDOPS_RUNNER_LABELS={}", input.labels.join(",")),
                    format!(
                        "GRIDOPS_RUNNER_GROUP={}",
                        input.runner_group.as_deref().unwrap_or_default()
                    ),
                ],
            ))
        }
        _ => Err(ManagerError::BadRequest("Runner mode is invalid.".into())),
    }
}

fn take_bootstrap_secret(input: &mut ProvisionRunner) -> Result<SecretString, ManagerError> {
    let value = match input.mode.as_str() {
        "ephemeral" => input.jit_config.take(),
        "persistent" => input.registration_token.take(),
        _ => None,
    }
    .ok_or_else(|| ManagerError::BadRequest("Runner bootstrap credential is required.".into()))?;
    Ok(SecretString::from(value))
}

fn ignore_not_modified(error: DockerError) -> Result<(), DockerError> {
    if matches!(
        error,
        DockerError::DockerResponseServerError {
            status_code: 304,
            ..
        }
    ) {
        return Ok(());
    }
    Err(error)
}

fn validate_container_id(id: &str) -> Result<(), ManagerError> {
    if (12..=64).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(());
    }
    Err(ManagerError::BadRequest(
        "Container identifier is invalid.".into(),
    ))
}

fn is_tart_id(id: &str) -> bool {
    id.strip_prefix("tart-").is_some_and(|suffix| {
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

async fn managed_container_labels(
    docker: &Docker,
    id: &str,
) -> Result<HashMap<String, String>, ManagerError> {
    let details = docker.inspect_container(id, None).await?;
    let labels = details.config.and_then(|config| config.labels);
    if let Some(labels) = labels.filter(managed_labels) {
        return Ok(labels);
    }
    Err(ManagerError::Forbidden(
        "GridOps can only control containers carrying its managed-runner label.".into(),
    ))
}

fn managed_labels(labels: &HashMap<String, String>) -> bool {
    labels.get("io.gridops.managed").map(String::as_str) == Some("true")
        && labels
            .get("io.gridops.runner-id")
            .is_some_and(|value| !value.is_empty())
        && labels
            .get("io.gridops.pool-id")
            .is_some_and(|value| !value.is_empty())
}

fn valid_docker_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_registration_url(value: &str) -> bool {
    value.starts_with("https://github.com/")
        && value.len() <= 512
        && !value.bytes().any(|byte| byte.is_ascii_whitespace())
}

fn valid_log_tail(value: &str) -> bool {
    value == "all" || value.parse::<u32>().is_ok_and(|lines| lines <= 100_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_disk_floor_scales_down_on_small_disks() {
        assert_eq!(free_disk_floor_mb(10_240, 10, 500_000), 10_240);
        assert_eq!(free_disk_floor_mb(10_240, 10, 40_000), 4_000);
        assert_eq!(free_disk_floor_mb(10_240, 0, 40_000), 10_240);
    }

    #[test]
    fn runner_containers_drop_all_capabilities_but_keep_build_essentials() {
        assert_eq!(runner_cap_drop(), vec!["ALL".to_string()]);
        assert_eq!(
            runner_cap_add(),
            vec![
                "CHOWN".to_string(),
                "DAC_OVERRIDE".to_string(),
                "FOWNER".to_string(),
                "SETUID".to_string(),
                "SETGID".to_string(),
            ]
        );
    }

    fn provision_request(mode: &str) -> ProvisionRunner {
        ProvisionRunner {
            provider: "docker".into(),
            runner_id: "runner-1".into(),
            pool_id: "pool-1".into(),
            name: "runner-1".into(),
            image: "ghcr.io/actions/actions-runner:latest".into(),
            mode: mode.into(),
            platform: "github".into(),
            jit_config: (mode == "ephemeral").then(|| "fake-jit-credential-123456".into()),
            registration_token: (mode == "persistent").then(|| "fake-registration-123456".into()),
            registration_url: (mode == "persistent")
                .then(|| "https://github.com/iamngoni/gridops".into()),
            labels: vec!["self-hosted".into(), "linux".into()],
            runner_group: None,
            bitbucket: None,
            bitbucket_oauth_client_secret: None,
            pull_image: false,
            runtime: "vm".into(),
            cpu_limit: 2.0,
            memory_limit_mb: 4_096,
            network: "gridops-runners".into(),
            capacity_lease: "capacity-lease-1".into(),
        }
    }

    #[test]
    fn requires_complete_gridops_management_labels() {
        let mut labels = HashMap::from([
            ("io.gridops.managed".into(), "true".into()),
            ("io.gridops.runner-id".into(), "runner-1".into()),
            ("io.gridops.pool-id".into(), "pool-1".into()),
        ]);
        assert!(managed_labels(&labels));
        labels.remove("io.gridops.pool-id");
        assert!(!managed_labels(&labels));
        labels.insert("io.gridops.pool-id".into(), "pool-1".into());
        labels.insert("io.gridops.managed".into(), "false".into());
        assert!(!managed_labels(&labels));
    }

    #[test]
    fn validates_registration_urls() {
        assert!(valid_registration_url(
            "https://github.com/iamngoni/gridops"
        ));
        assert!(valid_registration_url("https://github.com/iamngoni"));
        assert!(!valid_registration_url(
            "https://example.com/iamngoni/gridops"
        ));
        assert!(!valid_registration_url("https://github.com/iamngoni bad"));
    }

    #[test]
    fn validates_log_tails() {
        assert!(valid_log_tail("all"));
        assert!(valid_log_tail("0"));
        assert!(valid_log_tail("500"));
        assert!(!valid_log_tail("100001"));
        assert!(!valid_log_tail("recent"));
    }

    #[test]
    fn enforces_aggregate_runner_budgets() {
        let limits = ManagerLimits {
            host_name: "docker-host".into(),
            available_cpus: 10.0,
            total_memory_mb: 8_192,
            cpu_budget: 6.0,
            memory_budget_mb: 6_144,
            max_runners: 3,
            min_free_disk_mb: 1_024,
            min_free_disk_percent: 10,
            runner_network: "gridops-runners".into(),
            log_max_size: "20m".into(),
            log_max_files: 5,
            runner_pids_limit: 1_024,
            runner_swap_percent: 100,
            budget_origin: BudgetOrigin::default(),
        };
        let budget = limits.docker_budget();
        let disk = Some(DiskCapacity {
            total_mb: 100_000,
            available_mb: 50_000,
            minimum_free_mb: 10_000,
        });
        let active = CapacityUsage {
            active_runners: 2,
            cpu: 4.0,
            memory_mb: 4_096,
        };
        let requested = CapacityUsage {
            active_runners: 1,
            cpu: 2.0,
            memory_mb: 2_048,
        };
        assert!(
            enforce_capacity(budget, active, CapacityUsage::default(), requested, disk).is_ok()
        );
        let over_budget = CapacityUsage {
            active_runners: 1,
            cpu: 2.0,
            memory_mb: 2_049,
        };
        assert!(matches!(
            enforce_capacity(budget, active, CapacityUsage::default(), over_budget, disk),
            Err(ManagerError::Guardrail {
                code: "host_capacity_exhausted",
                ..
            })
        ));
    }

    #[test]
    fn runners_share_the_host_docker_socket_only_when_configured() -> Result<()> {
        let manager_socket =
            std::env::temp_dir().join(format!("gridops-socket-test-{}", uuid::Uuid::new_v4()));
        std::fs::write(&manager_socket, "")?;
        let group_id = std::fs::metadata(&manager_socket)?.gid();

        assert_eq!(shared_docker_socket(None, &manager_socket)?, None);
        assert_eq!(
            shared_docker_socket(Some(" ".into()), &manager_socket)?,
            None
        );
        assert_eq!(
            shared_docker_socket(Some("/var/run/docker.sock".into()), &manager_socket)?,
            Some(SharedDockerSocket {
                host_path: "/var/run/docker.sock".into(),
                group_id,
            })
        );
        // A relative path or bind-mount syntax could mount something else.
        for invalid in ["docker.sock", "/var/run/docker.sock:/etc", "/a,/b"] {
            assert!(shared_docker_socket(Some(invalid.into()), &manager_socket).is_err());
        }
        assert!(
            shared_docker_socket(
                Some("/var/run/docker.sock".into()),
                &manager_socket.with_extension("missing"),
            )
            .is_err()
        );

        std::fs::remove_file(manager_socket)?;
        Ok(())
    }

    #[test]
    fn reservations_only_count_against_their_own_provider() {
        let reservation = |provider: &str| CapacityReservation {
            runner_id: "runner".into(),
            pool_id: "pool".into(),
            provider: provider.into(),
            cpu_limit: 2.0,
            memory_limit_mb: 2_048,
            vm_slot: false,
            expires_at: Instant::now() + Duration::from_mins(5),
        };
        let reservations = HashMap::from([
            ("docker-lease".to_owned(), reservation("docker")),
            ("tart-lease".to_owned(), reservation("tart")),
            ("other-tart-lease".to_owned(), reservation("tart")),
        ]);
        let docker = provider_reservations(&reservations, "docker");
        assert_eq!(docker.active_runners, 1);
        assert_eq!(docker.memory_mb, 2_048);
        let tart = provider_reservations(&reservations, "tart");
        assert_eq!(tart.active_runners, 2);
        assert!((tart.cpu - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn agent_sandboxes_count_against_the_docker_host_budget() {
        let usage = DockerUsage {
            runners: CapacityUsage {
                active_runners: 2,
                cpu: 4.0,
                memory_mb: 4_096,
            },
            sandboxes: CapacityUsage {
                active_runners: 1,
                cpu: 2.0,
                memory_mb: 2_048,
            },
        };
        let total = usage.total();
        assert_eq!(total.active_runners, 3);
        assert!((total.cpu - 6.0).abs() < f64::EPSILON);
        assert_eq!(total.memory_mb, 6_144);
        let budget = CapacityBudget {
            cpu: 8.0,
            memory_mb: 16_384,
            max_runners: 3,
        };
        let requested = CapacityUsage {
            active_runners: 1,
            cpu: 1.0,
            memory_mb: 1_024,
        };
        // Two runners alone leave a slot; the running sandbox takes it.
        assert!(
            enforce_capacity(
                budget,
                usage.runners,
                CapacityUsage::default(),
                requested,
                None
            )
            .is_ok()
        );
        assert!(matches!(
            enforce_capacity(budget, total, CapacityUsage::default(), requested, None),
            Err(ManagerError::Guardrail {
                code: "host_capacity_exhausted",
                ..
            })
        ));
    }

    #[test]
    fn provider_hosts_without_a_manager_disk_view_skip_the_disk_guardrail() {
        let budget = CapacityBudget {
            cpu: 4.0,
            memory_mb: 6_144,
            max_runners: 1,
        };
        let requested = CapacityUsage {
            active_runners: 1,
            cpu: 2.0,
            memory_mb: 2_048,
        };
        assert!(
            enforce_capacity(
                budget,
                CapacityUsage::default(),
                CapacityUsage::default(),
                requested,
                None
            )
            .is_ok()
        );
    }

    #[test]
    fn validates_explicit_log_rotation_sizes() {
        assert!(valid_log_size("20m"));
        assert!(valid_log_size("1g"));
        assert!(!valid_log_size("20"));
        assert!(!valid_log_size("0m"));
        assert!(!valid_log_size("twentym"));
    }

    #[test]
    fn parses_portable_df_capacity_output() {
        let output = "Filesystem 1024-blocks Used Available Capacity Mounted on\noverlay 104857600 52428800 52428800 50% /\n";
        assert_eq!(parse_df_capacity(output), Some((102_400, 51_200)));
        assert_eq!(parse_df_capacity("Filesystem 1024-blocks\ninvalid"), None);
    }

    #[test]
    fn keeps_ephemeral_bootstrap_credentials_out_of_docker_metadata() -> Result<(), ManagerError> {
        let mut input = provision_request("ephemeral");
        input.validate()?;
        let secret = take_bootstrap_secret(&mut input)?;
        let (command, environment) = runner_command(&input)?;
        let metadata = format!("{command:?}{environment:?}");

        assert_eq!(secret.expose_secret(), "fake-jit-credential-123456");
        assert!(input.jit_config.is_none());
        assert!(!metadata.contains(secret.expose_secret()));
        assert!(metadata.contains("read -r GRIDOPS_BOOTSTRAP_SECRET"));
        assert!(metadata.contains("_diag/pages/*.log"));
        assert!(metadata.contains("GRIDOPS JOB LOG"));
        assert!(metadata.contains("gridops-runner-diagnostics.log"));
        assert!(!metadata.contains("ACTIONS_RUNNER_PRINT_LOG_TO_STDOUT"));
        Ok(())
    }

    #[test]
    fn keeps_persistent_bootstrap_credentials_out_of_docker_metadata() -> Result<(), ManagerError> {
        let mut input = provision_request("persistent");
        input.validate()?;
        let secret = take_bootstrap_secret(&mut input)?;
        let (command, environment) = runner_command(&input)?;
        let metadata = format!("{command:?}{environment:?}");

        assert_eq!(secret.expose_secret(), "fake-registration-123456");
        assert!(input.registration_token.is_none());
        assert!(!metadata.contains(secret.expose_secret()));
        assert!(metadata.contains("if [ ! -f .runner ]"));
        assert!(metadata.contains("_diag/pages/*.log"));
        assert!(metadata.contains("GRIDOPS JOB LOG"));
        assert!(metadata.contains("gridops-runner-diagnostics.log"));
        assert!(!metadata.contains("ACTIONS_RUNNER_PRINT_LOG_TO_STDOUT"));
        Ok(())
    }

    #[test]
    fn generated_runner_shell_scripts_are_valid_bash() -> Result<(), ManagerError> {
        for mode in ["ephemeral", "persistent"] {
            let (command, _) = runner_command(&provision_request(mode))?;
            let Some(script) = command.get(2) else {
                return Err(ManagerError::Internal(anyhow::anyhow!(
                    "runner command should include an inline script"
                )));
            };
            let status = std::process::Command::new("bash")
                .args(["-n", "-c", script])
                .status()
                .map_err(|error| ManagerError::Internal(error.into()))?;
            assert!(
                status.success(),
                "generated {mode} runner script is invalid"
            );
        }
        Ok(())
    }

    #[test]
    fn runner_exit_status_survives_the_login_shells_logout() -> Result<(), ManagerError> {
        // Ubuntu's ~/.bash_logout, as GitHub's runner image ships it.
        let home =
            std::env::temp_dir().join(format!("gridops-runner-home-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).map_err(|error| ManagerError::Internal(error.into()))?;
        std::fs::write(
            home.join(".bash_logout"),
            "if [ \"$SHLVL\" = 1 ]; then\n    [ -x /usr/bin/clear_console ] && /usr/bin/clear_console -q\nfi\n",
        )
        .map_err(|error| ManagerError::Internal(error.into()))?;
        let exit_code = |status: i32| {
            let script = instrumented_runner_script(
                "set -euo pipefail",
                &format!("/bin/bash -c 'exit {status}'"),
            );
            std::process::Command::new("bash")
                .args(["-lc", &script])
                .current_dir(&home)
                .env_clear()
                .env("HOME", &home)
                .env("PATH", "/usr/bin:/bin")
                .status()
                .map(|status| status.code())
                .map_err(|error| ManagerError::Internal(error.into()))
        };
        let (succeeded, failed) = (exit_code(0)?, exit_code(3)?);
        std::fs::remove_dir_all(&home).map_err(|error| ManagerError::Internal(error.into()))?;
        assert_eq!((succeeded, failed), (Some(0), Some(3)));
        Ok(())
    }

    #[test]
    fn job_console_pages_are_forwarded_to_runner_stdout() -> Result<(), ManagerError> {
        let directory =
            std::env::temp_dir().join(format!("gridops-runner-log-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(directory.join("_diag/pages"))
            .map_err(|error| ManagerError::Internal(error.into()))?;
        let script = instrumented_runner_script(
            "set -euo pipefail",
            r#"/bin/bash -lc 'sleep 0.2; printf "secret-masked job output\n" > _diag/pages/job.log; sleep 0.5'"#,
        );
        // Started in the runner's directory, as the image's working directory
        // starts it, with nothing saying where that is.
        let output = std::process::Command::new("bash")
            .args(["-c", &script])
            .current_dir(&directory)
            .env_remove("GRIDOPS_RUNNER_ROOT")
            .output()
            .map_err(|error| ManagerError::Internal(error.into()))?;
        std::fs::remove_dir_all(&directory)
            .map_err(|error| ManagerError::Internal(error.into()))?;
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("GRIDOPS JOB LOG job.log"));
        assert!(
            stdout.contains("secret-masked job output"),
            "forwarded stdout was: {stdout}"
        );
        Ok(())
    }
}
