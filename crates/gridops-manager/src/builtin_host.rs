//! Whether this manager may start runners on its own Docker host, the machine
//! `GridOps` runs on. `GridOps` decides (an instance setting) and pushes the
//! decision with the provisioning policy; the manager enforces it where
//! runners start, so a request that slips past the API or the reconciler is
//! still refused.
//!
//! Until `GridOps` has said, the host counts as off: a manager that has just
//! restarted never starts a runner on a machine it has not been told to use.
//! Agent sandboxes are not runners and are not covered; they need Docker, not
//! the setting.

use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use axum::http::StatusCode;

use super::ManagerError;

const UNKNOWN: u8 = 0;
const ENABLED: u8 = 1;
const DISABLED: u8 = 2;

pub(super) use gridops_core::builtin_host::DISABLED_CODE;

#[derive(Clone, Debug)]
pub(super) struct BuiltinHost {
    /// Whether the manager has a Docker engine at all. Without one (it runs
    /// with `GRIDOPS_MANAGER_DOCKER=off` and no socket), nothing can start on
    /// this host whatever `GridOps` says.
    docker: bool,
    state: Arc<AtomicU8>,
}

impl BuiltinHost {
    pub(super) fn new(docker: bool, initial: Option<bool>) -> Self {
        let state = match initial {
            None => UNKNOWN,
            Some(true) => ENABLED,
            Some(false) => DISABLED,
        };
        Self {
            docker,
            state: Arc::new(AtomicU8::new(state)),
        }
    }

    pub(super) fn docker_available(&self) -> bool {
        self.docker
    }

    pub(super) fn set(&self, enabled: bool) {
        self.state
            .store(if enabled { ENABLED } else { DISABLED }, Ordering::Relaxed);
    }

    /// What `GridOps` last said, or `None` before it has said.
    pub(super) fn setting(&self) -> Option<bool> {
        match self.state.load(Ordering::Relaxed) {
            ENABLED => Some(true),
            DISABLED => Some(false),
            _ => None,
        }
    }

    /// Refuses unless this host may start runners.
    pub(super) fn require_runners(&self) -> Result<(), ManagerError> {
        self.require_docker()?;
        match self.setting() {
            Some(true) => Ok(()),
            Some(false) => Err(refusal(gridops_core::builtin_host::DISABLED_MESSAGE)),
            None => Err(refusal(
                "The built-in runner host is treated as off until GridOps tells this manager otherwise, which it does within one reconcile interval of starting.",
            )),
        }
    }

    /// Refuses when there is no Docker engine to talk to.
    pub(super) fn require_docker(&self) -> Result<(), ManagerError> {
        if self.docker {
            return Ok(());
        }
        Err(ManagerError::Upstream {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "docker_unavailable".into(),
            message: "This runner manager runs without Docker access (GRIDOPS_MANAGER_DOCKER=off), so it cannot start containers on its host.".into(),
        })
    }
}

fn refusal(message: &str) -> ManagerError {
    ManagerError::Upstream {
        status: StatusCode::CONFLICT,
        code: DISABLED_CODE.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::atomic::AtomicBool};

    use secrecy::SecretString;
    use serde_json::{Value, json};
    use tokio::sync::Mutex;

    use super::*;
    use crate::{ManagerLimits, ManagerState};

    const TOKEN: &str = "builtin-host-test-token";

    fn limits() -> ManagerLimits {
        ManagerLimits {
            host_name: "docker-host".into(),
            available_cpus: 8.0,
            total_memory_mb: 16_384,
            cpu_budget: 8.0,
            memory_budget_mb: 16_384,
            max_runners: 4,
            min_free_disk_mb: 1_024,
            min_free_disk_percent: 10,
            runner_network: "gridops-runners".into(),
            log_max_size: "20m".into(),
            log_max_files: 5,
            runner_pids_limit: 1_024,
            runner_swap_percent: 100,
        }
    }

    /// The real router against a Docker endpoint that refuses every
    /// connection, so a request that gets past the gate fails at Docker with
    /// a different answer than the gate's.
    struct TestManager {
        base_url: String,
        state: ManagerState,
        socket: std::path::PathBuf,
    }

    impl TestManager {
        async fn start(docker: bool, setting: Option<bool>) -> anyhow::Result<Self> {
            let socket = std::env::temp_dir().join(format!(
                "gridops-builtin-host-test-{}.sock",
                uuid::Uuid::new_v4()
            ));
            std::fs::write(&socket, "")?;
            let state = ManagerState {
                docker: bollard::Docker::connect_with_socket(
                    &socket.to_string_lossy(),
                    2,
                    bollard::API_DEFAULT_VERSION,
                )?,
                tart: None,
                token: SecretString::from(TOKEN),
                limits: limits(),
                shared_docker_socket: None,
                reservations: std::sync::Arc::new(Mutex::new(HashMap::new())),
                provisioning_paused: std::sync::Arc::new(AtomicBool::new(false)),
                builtin_host: BuiltinHost::new(docker, setting),
                provision_lock: std::sync::Arc::new(Mutex::new(())),
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let base_url = format!("http://{}", listener.local_addr()?);
            let app = crate::router(state.clone());
            tokio::spawn(async move { axum::serve(listener, app).await });
            Ok(Self {
                base_url,
                state,
                socket,
            })
        }

        fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
            reqwest::Client::new()
                .request(method, format!("{}{path}", self.base_url))
                .bearer_auth(TOKEN)
        }
    }

    impl Drop for TestManager {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.socket);
        }
    }

    async fn code(response: reqwest::Response) -> anyhow::Result<(u16, String)> {
        let status = response.status().as_u16();
        let body = response.json::<Value>().await.unwrap_or(Value::Null);
        Ok((
            status,
            body.get("code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        ))
    }

    fn admission(provider: &str, pool: &str) -> Value {
        json!({
            "runnerId": "3f2b6c1e-0d4a-4d0e-9a51-7c1f0b2e8d44", "poolId": pool,
            "provider": provider, "cpuLimit": 2.0, "memoryLimitMb": 2048,
        })
    }

    fn provision_body() -> Value {
        json!({
            "runnerId": "3f2b6c1e-0d4a-4d0e-9a51-7c1f0b2e8d44", "poolId": "pool-1",
            "name": "pool-1-abcd1234", "image": "runner:fixture", "mode": "ephemeral",
            "provider": "docker", "platform": "github",
            "jitConfig": "fake-jit-credential-123456", "labels": ["self-hosted"],
            "cpuLimit": 2.0, "memoryLimitMb": 2048, "network": "gridops-runners",
            "capacityLease": "lease-1",
        })
    }

    #[tokio::test]
    async fn a_runner_admission_is_refused_while_the_host_is_off_or_not_yet_told()
    -> anyhow::Result<()> {
        for setting in [Some(false), None] {
            let manager = TestManager::start(true, setting).await?;
            let response = manager
                .request(reqwest::Method::POST, "/v1/admissions")
                .json(&admission("docker", "pool-1"))
                .send()
                .await?;
            assert_eq!(
                code(response).await?,
                (409, DISABLED_CODE.to_owned()),
                "setting {setting:?}"
            );
            assert!(manager.state.reservations.lock().await.is_empty());
        }
        Ok(())
    }

    #[tokio::test]
    async fn a_runner_start_is_refused_while_the_host_is_off() -> anyhow::Result<()> {
        let manager = TestManager::start(true, Some(false)).await?;
        let response = manager
            .request(reqwest::Method::POST, "/v1/runners")
            .json(&provision_body())
            .send()
            .await?;
        assert_eq!(code(response).await?, (409, DISABLED_CODE.to_owned()));
        Ok(())
    }

    #[tokio::test]
    async fn starting_restarting_or_resuming_a_container_is_refused_while_the_host_is_off()
    -> anyhow::Result<()> {
        let manager = TestManager::start(true, Some(false)).await?;
        for action in ["start", "restart", "resume"] {
            let response = manager
                .request(
                    reqwest::Method::POST,
                    &format!("/v1/runners/0123456789abcdef/{action}"),
                )
                .send()
                .await?;
            assert_eq!(
                code(response).await?,
                (409, DISABLED_CODE.to_owned()),
                "{action}"
            );
        }
        // Stopping reduces load on the host, so it is not refused by the gate.
        let response = manager
            .request(reqwest::Method::POST, "/v1/runners/0123456789abcdef/stop")
            .send()
            .await?;
        assert_ne!(code(response).await?.1, DISABLED_CODE);
        Ok(())
    }

    #[tokio::test]
    async fn the_host_works_as_before_when_it_is_on() -> anyhow::Result<()> {
        let manager = TestManager::start(true, Some(true)).await?;
        // Past the gate, Docker is unreachable here and answers differently.
        let response = manager
            .request(reqwest::Method::POST, "/v1/admissions")
            .json(&admission("docker", "pool-1"))
            .send()
            .await?;
        assert_ne!(code(response).await?.1, DISABLED_CODE);
        let response = manager
            .request(reqwest::Method::POST, "/v1/runners")
            .json(&provision_body())
            .send()
            .await?;
        assert_ne!(code(response).await?.1, DISABLED_CODE);
        Ok(())
    }

    #[tokio::test]
    async fn agent_sandboxes_and_other_providers_are_not_covered_by_the_setting()
    -> anyhow::Result<()> {
        let manager = TestManager::start(true, Some(false)).await?;
        // The agent's sandbox pool is not a runner pool.
        let response = manager
            .request(reqwest::Method::POST, "/v1/admissions")
            .json(&admission("docker", "gridops-agent"))
            .send()
            .await?;
        assert_ne!(code(response).await?.1, DISABLED_CODE);
        // Tart runs on a separate Mac, and answers for itself.
        let response = manager
            .request(reqwest::Method::POST, "/v1/admissions")
            .json(&admission("tart", "pool-1"))
            .send()
            .await?;
        assert_eq!(code(response).await?.1, "tart_provider_disabled");
        Ok(())
    }

    #[tokio::test]
    async fn the_policy_carries_the_setting_and_older_callers_leave_it_alone() -> anyhow::Result<()>
    {
        let manager = TestManager::start(true, None).await?;
        assert_eq!(manager.state.builtin_host.setting(), None);
        let put = |body: Value| {
            manager
                .request(reqwest::Method::PUT, "/v1/policy")
                .json(&body)
                .send()
        };
        let response =
            put(json!({"provisioningPaused": false, "builtinHostEnabled": true})).await?;
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(response.json::<Value>().await?["builtinHostEnabled"], true);
        assert_eq!(manager.state.builtin_host.setting(), Some(true));
        put(json!({"provisioningPaused": false})).await?;
        assert_eq!(
            manager.state.builtin_host.setting(),
            Some(true),
            "left alone"
        );
        put(json!({"provisioningPaused": false, "builtinHostEnabled": false})).await?;
        assert_eq!(manager.state.builtin_host.setting(), Some(false));
        Ok(())
    }

    #[tokio::test]
    async fn without_a_docker_socket_the_manager_still_answers_and_starts_nothing()
    -> anyhow::Result<()> {
        // GridOps says the host is on, but there is no engine behind it.
        let manager = TestManager::start(false, Some(true)).await?;
        let health = manager
            .request(reqwest::Method::GET, "/v1/health")
            .send()
            .await?;
        assert_eq!(health.status().as_u16(), 200);
        let health = health.json::<Value>().await?;
        assert_eq!(health["status"], "ok");
        assert_eq!(health["providers"]["docker"]["available"], false);
        assert_eq!(health["builtinHost"]["dockerAvailable"], false);

        let runners = manager
            .request(reqwest::Method::GET, "/v1/runners")
            .send()
            .await?;
        assert_eq!(runners.status().as_u16(), 200);
        assert_eq!(runners.json::<Value>().await?["runners"], json!([]));

        let response = manager
            .request(reqwest::Method::POST, "/v1/admissions")
            .json(&admission("docker", "pool-1"))
            .send()
            .await?;
        assert_eq!(
            code(response).await?,
            (503, "docker_unavailable".to_owned())
        );
        let response = manager
            .request(reqwest::Method::POST, "/v1/admissions")
            .json(&admission("docker", "gridops-agent"))
            .send()
            .await?;
        assert_eq!(
            code(response).await?,
            (503, "docker_unavailable".to_owned())
        );
        Ok(())
    }

    #[tokio::test]
    async fn health_reports_what_gridops_last_said() -> anyhow::Result<()> {
        // Docker is unreachable in these tests, so only the no-Docker form of
        // health can be read; it carries the same builtinHost block.
        for (setting, expected) in [
            (Some(false), json!(false)),
            (Some(true), json!(true)),
            (None, Value::Null),
        ] {
            let manager = TestManager::start(false, setting).await?;
            let health = manager
                .request(reqwest::Method::GET, "/v1/health")
                .send()
                .await?
                .json::<Value>()
                .await?;
            assert_eq!(health["builtinHost"]["enabled"], expected);
        }
        Ok(())
    }
}
