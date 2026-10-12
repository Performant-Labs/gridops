//! Command-line entry point.
//!
//! - `gridops-host-agent enroll` reads a one-time code from
//!   `GRIDOPS_ENROLLMENT_CODE` or stdin, so it never appears in a process list.
//! - `gridops-host-agent run` keeps the fleet session open.
//! - `gridops-host-agent helper <socket>` runs the privileged runtime helper.
//! - `gridops-host-agent probe` prints what this host would report.
//!
//! Configuration: `GRIDOPS_FLEET_URL` (required for enroll/run),
//! `GRIDOPS_AGENT_STATE_DIR` (journal location), `GRIDOPS_AGENT_NAME`
//! (defaults to the hostname), `GRIDOPS_AGENT_HELPER_SOCKET` (use a separate
//! privileged helper; otherwise the runtime runs in-process),
//! `GRIDOPS_AGENT_IMAGES` (comma-separated runner image allowlist),
//! `GRIDOPS_AGENT_DOCKER_NETWORK` and `GRIDOPS_AGENT_DOCKER_SOCKET` (share
//! this socket with profiles granted Docker access).

use std::{env, io::BufRead as _, path::PathBuf, process::ExitCode};

use anyhow::{Context as _, Result};
#[cfg(unix)]
use gridops_host_agent::helper::SocketHelper;
use gridops_host_agent::{
    client::Client,
    dispatch::Trust,
    docker::DockerRuntime,
    journal::Journal,
    native::NativeRuntime,
    runtime::HostRuntime,
    supervisor::{Stop, Supervisor},
};

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    match run().await {
        Ok(code) => code,
        Err(error) => {
            tracing::error!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<ExitCode> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_default();
    if command == "probe" {
        // What this host would report: measurements and execution backends.
        use gridops_host_agent::helper::RuntimeHelper as _;
        let report = serde_json::json!({
            "os": gridops_host_agent::probe::host_os(),
            "architecture": gridops_host_agent::probe::architecture(),
            "measurement": format!("{:?}", gridops_host_agent::probe::measure()),
            "backends": host_runtime()?.backends().await,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(ExitCode::SUCCESS);
    }
    if command == "helper" {
        return helper(
            args.next()
                .context("usage: gridops-host-agent helper <socket>")?,
        )
        .await;
    }
    let journal = Journal::open(&state_dir()?.join("journal.sqlite")).await?;
    let client =
        Client::new(&env::var("GRIDOPS_FLEET_URL").context("GRIDOPS_FLEET_URL is not set")?)?;
    let supervisor = Supervisor::new(client, journal, name());
    match command.as_str() {
        "enroll" => {
            let code = match env::var("GRIDOPS_ENROLLMENT_CODE") {
                Ok(code) => code,
                Err(_) => std::io::stdin()
                    .lock()
                    .lines()
                    .next()
                    .context("no enrollment code on stdin")??,
            };
            supervisor.enroll(code.trim(), None).await?;
            tracing::info!("enrolled; waiting for an administrator to approve this host");
            Ok(ExitCode::SUCCESS)
        }
        "run" => {
            let trust = Trust {
                docker_socket: env::var("GRIDOPS_AGENT_DOCKER_SOCKET").is_ok(),
            };
            let shutdown = async {
                let _ = tokio::signal::ctrl_c().await;
            };
            let stop = match env::var("GRIDOPS_AGENT_HELPER_SOCKET") {
                #[cfg(unix)]
                Ok(socket) => {
                    supervisor
                        .with_runtime(SocketHelper::new(socket.into()), trust)
                        .run(shutdown)
                        .await?
                }
                _ => {
                    supervisor
                        .with_runtime(docker_runtime(), trust)
                        .run(shutdown)
                        .await?
                }
            };
            Ok(match stop {
                Stop::Shutdown => ExitCode::SUCCESS,
                Stop::Incompatible => {
                    tracing::error!(
                        "the server doesn't support this agent version; upgrade the agent"
                    );
                    ExitCode::from(3)
                }
                Stop::CredentialRejected => {
                    tracing::error!("the server rejected this host's credential; recover the host");
                    ExitCode::from(4)
                }
            })
        }
        _ => anyhow::bail!("usage: gridops-host-agent <enroll|run|helper|probe>"),
    }
}

#[cfg(unix)]
async fn helper(socket: String) -> Result<ExitCode> {
    use std::{os::unix::fs::PermissionsExt as _, sync::Arc};
    let _ = std::fs::remove_file(&socket);
    let listener = tokio::net::UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    gridops_host_agent::helper::serve(listener, Arc::new(host_runtime()?)).await?;
    Ok(ExitCode::SUCCESS)
}

#[cfg(not(unix))]
async fn helper(_socket: String) -> Result<ExitCode> {
    anyhow::bail!("the runtime helper socket isn't available on this platform yet")
}

fn host_runtime() -> Result<HostRuntime> {
    Ok(HostRuntime::new(
        docker_runtime(),
        NativeRuntime::new(state_dir()?.join("native")),
    ))
}

fn docker_runtime() -> DockerRuntime {
    let images = env::var("GRIDOPS_AGENT_IMAGES")
        .unwrap_or_else(|_| "ghcr.io/actions/actions-runner:latest".into())
        .split(',')
        .map(|image| image.trim().to_owned())
        .filter(|image| !image.is_empty())
        .collect();
    DockerRuntime::new(
        images,
        env::var("GRIDOPS_AGENT_DOCKER_NETWORK").ok(),
        env::var("GRIDOPS_AGENT_DOCKER_SOCKET").ok(),
    )
}

fn state_dir() -> Result<PathBuf> {
    if let Ok(dir) = env::var("GRIDOPS_AGENT_STATE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    Ok(if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/GridOps/agent")
    } else if cfg!(target_os = "windows") {
        PathBuf::from(env::var("ProgramData").context("ProgramData is not set")?)
            .join("GridOps")
            .join("agent")
    } else {
        PathBuf::from("/var/lib/gridops-agent")
    })
}

fn name() -> String {
    env::var("GRIDOPS_AGENT_NAME").ok().unwrap_or_else(|| {
        std::process::Command::new("hostname")
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "gridops-host".to_owned())
    })
}
