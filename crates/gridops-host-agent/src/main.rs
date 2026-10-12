//! Command-line entry point.
//!
//! - `gridops-host-agent` (no arguments, in a terminal) enrolls the machine if
//!   it isn't enrolled yet, asking for anything missing, then runs.
//! - `gridops-host-agent enroll [--replace]` enrolls with a one-time code from
//!   `GRIDOPS_ENROLLMENT_CODE` or a prompt, so it never appears in a process
//!   list. `--replace` takes a recovery code for an already enrolled machine.
//! - `gridops-host-agent run` keeps the fleet session open.
//! - `gridops-host-agent helper <socket>` runs the privileged runtime helper.
//! - `gridops-host-agent probe` prints what this host would report.
//!
//! The `GridOps` address given at enrollment is saved in the state directory,
//! so `run` needs no configuration. Optional settings: `GRIDOPS_FLEET_URL`
//! (overrides the saved address), `GRIDOPS_AGENT_STATE_DIR` (default
//! `~/.gridops/host-agent`), `GRIDOPS_AGENT_NAME` (defaults to the hostname),
//! `GRIDOPS_AGENT_HELPER_SOCKET` (use a separate privileged helper; otherwise
//! the runtime runs in-process), `GRIDOPS_AGENT_IMAGES` (comma-separated runner
//! image allowlist), `GRIDOPS_AGENT_DOCKER_NETWORK` and
//! `GRIDOPS_AGENT_DOCKER_SOCKET` (share this socket with profiles granted
//! Docker access).

use std::{
    env,
    io::{BufRead as _, IsTerminal as _, Write as _},
    path::{Path, PathBuf},
    process::ExitCode,
};

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
use serde::{Deserialize, Serialize};

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

/// Settings saved next to the journal at enrollment.
#[derive(Default, Serialize, Deserialize)]
struct Saved {
    fleet_url: Option<String>,
}

async fn run() -> Result<ExitCode> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let flags: Vec<String> = args.collect();
    match command.as_str() {
        "probe" => {
            use gridops_host_agent::helper::RuntimeHelper as _;
            let report = serde_json::json!({
                "os": gridops_host_agent::probe::host_os(),
                "architecture": gridops_host_agent::probe::architecture(),
                "measurement": format!("{:?}", gridops_host_agent::probe::measure()),
                "backends": host_runtime()?.backends().await,
            });
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(ExitCode::SUCCESS)
        }
        "helper" => {
            helper(
                flags
                    .first()
                    .cloned()
                    .context("usage: gridops-host-agent helper <socket>")?,
            )
            .await
        }
        "enroll" => {
            let journal = journal().await?;
            enroll(&journal, flags.iter().any(|flag| flag == "--replace")).await?;
            if interactive() && confirm("Start the agent now?")? {
                return serve(journal).await;
            }
            Ok(ExitCode::SUCCESS)
        }
        "run" => serve(journal().await?).await,
        "" if interactive() => {
            let journal = journal().await?;
            if journal.identity().await?.is_none() {
                enroll(&journal, false).await?;
            }
            serve(journal).await
        }
        "" | "help" | "--help" | "-h" => {
            println!(
                "Usage: gridops-host-agent [enroll [--replace] | run | probe | helper <socket>]\n\n\
                 Run it with no arguments in a terminal to enroll this machine and start it."
            );
            Ok(ExitCode::SUCCESS)
        }
        other => anyhow::bail!("unknown command {other:?}; try `gridops-host-agent help`"),
    }
}

/// Enrolls with a code from the environment or a prompt, and saves the
/// `GridOps` address for later runs.
async fn enroll(journal: &Journal, replace: bool) -> Result<()> {
    let saved = load_saved()?;
    let url = match env::var("GRIDOPS_FLEET_URL").ok().or(saved.fleet_url) {
        Some(url) if !replace || !interactive() => url,
        current => prompt("GridOps address", current.as_deref())?,
    };
    let url = url.trim().trim_end_matches('/').to_owned();
    let client = Client::new(&url)?;
    let code = match env::var("GRIDOPS_ENROLLMENT_CODE") {
        Ok(code) => code,
        Err(_) if interactive() => prompt(
            if replace {
                "Recovery code"
            } else {
                "Enrollment code (from Machines → Add machine)"
            },
            None,
        )?,
        Err(_) => std::io::stdin()
            .lock()
            .lines()
            .next()
            .context("no enrollment code on stdin")??,
    };
    let supervisor = Supervisor::new(client, journal.clone(), name());
    if replace {
        supervisor.enroll_replacing(code.trim(), None).await?;
    } else {
        supervisor.enroll(code.trim(), None).await?;
    }
    save(&Saved {
        fleet_url: Some(url),
    })?;
    println!(
        "Enrolled as {}. Approve it in GridOps under Machines.",
        name()
    );
    Ok(())
}

async fn serve(journal: Journal) -> Result<ExitCode> {
    let url = env::var("GRIDOPS_FLEET_URL")
        .ok()
        .or(load_saved()?.fleet_url)
        .context("this machine isn't enrolled yet; run `gridops-host-agent enroll`")?;
    let supervisor = Supervisor::new(Client::new(&url)?, journal, name());
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
                .with_runtime(host_runtime()?, trust)
                .run(shutdown)
                .await?
        }
    };
    Ok(match stop {
        Stop::Shutdown => ExitCode::SUCCESS,
        Stop::Incompatible => {
            tracing::error!("the server doesn't support this agent version; upgrade the agent");
            ExitCode::from(3)
        }
        Stop::CredentialRejected => {
            tracing::error!(
                "the server rejected this machine's credential; recover it in GridOps and run `gridops-host-agent enroll --replace`"
            );
            ExitCode::from(4)
        }
    })
}

fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn prompt(label: &str, default: Option<&str>) -> Result<String> {
    loop {
        match default {
            Some(default) => print!("{label} [{default}]: "),
            None => print!("{label}: "),
        }
        std::io::stdout().flush()?;
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line)? == 0 {
            anyhow::bail!("no input");
        }
        let value = line.trim();
        if !value.is_empty() {
            return Ok(value.to_owned());
        }
        if let Some(default) = default {
            return Ok(default.to_owned());
        }
    }
}

fn confirm(question: &str) -> Result<bool> {
    let answer = prompt(&format!("{question} [Y/n]"), Some("y"))?;
    Ok(!answer.to_ascii_lowercase().starts_with('n'))
}

async fn journal() -> Result<Journal> {
    Journal::open(&state_dir()?.join("journal.sqlite")).await
}

fn saved_path() -> Result<PathBuf> {
    Ok(state_dir()?.join("agent.json"))
}

fn load_saved() -> Result<Saved> {
    let path = saved_path()?;
    if !path.exists() {
        return Ok(Saved::default());
    }
    serde_json::from_slice(&std::fs::read(&path)?)
        .with_context(|| format!("reading {}", path.display()))
}

fn save(saved: &Saved) -> Result<()> {
    let path = saved_path()?;
    write_private(&path, &serde_json::to_vec_pretty(saved)?)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
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

/// `GRIDOPS_AGENT_STATE_DIR`, else `.gridops/host-agent` in the user's home,
/// so enrolling never needs administrator rights.
fn state_dir() -> Result<PathBuf> {
    if let Ok(dir) = env::var("GRIDOPS_AGENT_STATE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .context("set GRIDOPS_AGENT_STATE_DIR; no home directory found")?;
    Ok(PathBuf::from(home).join(".gridops").join("host-agent"))
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
