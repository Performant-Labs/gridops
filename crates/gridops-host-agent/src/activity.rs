//! What the agent's log says about the work it does: each command as it
//! starts and how it ended, the runner a start registers, and the jobs its
//! runners pick up. Routine checks stay at debug, so at info the log reads as
//! a timeline of this machine's work.

use std::{collections::HashMap, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use gridops_core::fleet::protocol::{
    agent::{AgentCommandEnvelope, AgentOperation, CommandScope, LocalResourceIdentity, StopMode},
    outcomes::{CommandOutcome, FailureCode, SuccessEvidence},
};
use serde::{Deserialize, Serialize};

use crate::helper::Bootstrap;

/// Logs at debug for routine work, at info otherwise.
macro_rules! note {
    ($quiet:expr, $($arg:tt)+) => {
        if $quiet {
            tracing::debug!($($arg)+)
        } else {
            tracing::info!($($arg)+)
        }
    };
}

/// A command this agent is about to run.
pub(crate) fn starting(envelope: &AgentCommandEnvelope, bootstrap: Option<&Bootstrap>) {
    let placement = placement(envelope);
    let placement = placement.as_deref().map(tracing::field::display);
    match &envelope.operation {
        AgentOperation::PrepareEnvironment(prepare) => tracing::info!(
            placement,
            image = %prepare.image.as_str(),
            labels = %prepare
                .labels
                .iter()
                .map(|label| label.as_str())
                .collect::<Vec<_>>()
                .join(","),
            cpus = f64::from(prepare.resource_request.cpu_millis()) / 1000.0,
            memory_mib = prepare.resource_request.memory_mib(),
            "preparing a {} runner ({})",
            snake(&prepare.runtime_kind),
            snake(&prepare.mode),
        ),
        AgentOperation::StartRunner(_) => {
            if let Some(Runner { name, url }) = bootstrap.and_then(runner) {
                tracing::info!(placement, "starting runner {name} for {url}");
            } else {
                tracing::info!(placement, "starting a runner");
            }
        }
        operation => note!(quiet(operation), placement, "{}", doing(operation)),
    }
}

/// How a command this agent ran ended.
pub(crate) fn finished(envelope: &AgentCommandEnvelope, outcome: &CommandOutcome, took: Duration) {
    let placement = placement(envelope);
    let placement = placement.as_deref().map(tracing::field::display);
    let operation = &envelope.operation;
    let command = tracing::field::display(kind(operation));
    let took = tracing::field::display(format!("{:.1}s", took.as_secs_f64()));
    match outcome {
        CommandOutcome::Succeeded { evidence } => match evidence.as_ref() {
            SuccessEvidence::PreparedEnvironment { .. } => {
                tracing::info!(placement, took, "runner environment ready");
            }
            SuccessEvidence::RunnerStarted { local_identity, .. } => tracing::info!(
                placement,
                took,
                "runner started as {}",
                resource(local_identity)
            ),
            _ => note!(quiet(operation), placement, took, "{}", done(operation)),
        },
        // The reconciler's check found the runner gone; a stop and cleanup
        // follow.
        CommandOutcome::Failed {
            code: FailureCode::ResourceUnavailable,
            ..
        } if matches!(operation, AgentOperation::Inspect { .. }) => {
            tracing::info!(placement, "runner is no longer running");
        }
        CommandOutcome::Failed {
            code, retryable, ..
        } => tracing::warn!(
            placement,
            command,
            took,
            retryable,
            "command failed: {}",
            snake(code)
        ),
        CommandOutcome::Uncertain { code, .. } => tracing::warn!(
            placement,
            command,
            took,
            "command ended with its effect unknown: {}",
            snake(code)
        ),
        CommandOutcome::Accepted { .. } | CommandOutcome::Running { .. } => {}
    }
}

/// The command's name on the wire.
pub(crate) const fn kind(operation: &AgentOperation) -> &'static str {
    match operation {
        AgentOperation::PrepareEnvironment(_) => "prepare_environment",
        AgentOperation::StartRunner(_) => "start_runner",
        AgentOperation::InspectHost => "inspect_host",
        AgentOperation::DrainHost => "drain_host",
        AgentOperation::Inspect { .. } => "inspect",
        AgentOperation::Drain { .. } => "drain",
        AgentOperation::Stop { .. } => "stop",
        AgentOperation::Cleanup { .. } => "cleanup",
        AgentOperation::UpgradeAgent { .. } => "upgrade_agent",
        AgentOperation::ExecuteSandbox { .. } => "execute_sandbox",
        AgentOperation::SandboxFile { .. } => "sandbox_file",
        AgentOperation::SandboxArtifact { .. } => "sandbox_artifact",
    }
}

/// The first block of the command's placement ID, enough to follow one
/// runner through the log.
pub(crate) fn placement(envelope: &AgentCommandEnvelope) -> Option<String> {
    match &envelope.scope {
        CommandScope::Placement { placement_id, .. } => Some(short(&placement_id.to_string())),
        CommandScope::Host => None,
    }
}

pub(crate) fn short(id: &str) -> String {
    id.split('-').next().unwrap_or(id).to_owned()
}

/// Checks the reconciler repeats every pass, and the fix agent's sandbox
/// calls, which come in bursts.
const fn quiet(operation: &AgentOperation) -> bool {
    matches!(
        operation,
        AgentOperation::Inspect { .. }
            | AgentOperation::InspectHost
            | AgentOperation::ExecuteSandbox { .. }
            | AgentOperation::SandboxFile { .. }
            | AgentOperation::SandboxArtifact { .. }
    )
}

const fn doing(operation: &AgentOperation) -> &'static str {
    match operation {
        AgentOperation::PrepareEnvironment(_) => "preparing a runner",
        AgentOperation::StartRunner(_) => "starting a runner",
        AgentOperation::InspectHost => "checking this machine",
        AgentOperation::DrainHost => "draining this machine",
        AgentOperation::Inspect { .. } => "checking a runner",
        AgentOperation::Drain { .. } => "draining a runner",
        AgentOperation::Stop {
            mode: StopMode::IdleOnly,
            ..
        } => "stopping an idle runner",
        AgentOperation::Stop {
            mode: StopMode::CancelBusy,
            ..
        } => "stopping a runner and cancelling its job",
        AgentOperation::Cleanup { .. } => "removing a runner",
        AgentOperation::UpgradeAgent { .. } => "agent upgrade requested",
        AgentOperation::ExecuteSandbox { .. } => "running a fix sandbox command",
        AgentOperation::SandboxFile { .. } => "fix sandbox file operation",
        AgentOperation::SandboxArtifact { .. } => "fix sandbox artifact operation",
    }
}

const fn done(operation: &AgentOperation) -> &'static str {
    match operation {
        AgentOperation::InspectHost => "machine checked",
        AgentOperation::DrainHost => "machine draining",
        AgentOperation::Inspect { .. } => "runner is running",
        AgentOperation::Drain { .. } => "runner draining",
        AgentOperation::Stop { .. } => "runner stopped",
        AgentOperation::Cleanup { .. } => "runner removed",
        _ => "command done",
    }
}

fn resource(identity: &LocalResourceIdentity) -> String {
    match identity {
        LocalResourceIdentity::Docker { object_id } => {
            let id = object_id.as_str();
            format!("container {}", id.get(..12).unwrap_or(id))
        }
        LocalResourceIdentity::TartVm { name } => format!("VM {}", name.as_str()),
        LocalResourceIdentity::NativeProcess { process_id, .. } => {
            format!("process {}", process_id.get())
        }
    }
}

pub(crate) fn snake(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        _ => "unknown".to_owned(),
    }
}

struct Runner {
    name: String,
    url: String,
}

/// Registration material, read only for the runner's name and address.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Material {
    GithubJit {
        jit_config: String,
    },
    GithubRegistration {
        name: String,
        url: String,
    },
    #[serde(other)]
    Other,
}

/// Who a start registers. A JIT config is GitHub's runner files, base64
/// encoded; its `.runner` file names the runner and its repository, and holds
/// no secret.
fn runner(bootstrap: &Bootstrap) -> Option<Runner> {
    match serde_json::from_str(&bootstrap.0).ok()? {
        Material::GithubRegistration { name, url } => Some(Runner { name, url }),
        Material::GithubJit { jit_config } => {
            let files: HashMap<String, String> =
                serde_json::from_slice(&STANDARD.decode(jit_config.trim()).ok()?).ok()?;
            let file = String::from_utf8(STANDARD.decode(files.get(".runner")?).ok()?).ok()?;
            let file: HashMap<String, serde_json::Value> =
                serde_json::from_str(file.trim_start_matches('\u{feff}')).ok()?;
            // The runner reads these keys without regard to case.
            let field = |name: &str| {
                file.iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .and_then(|(_, value)| value.as_str())
                    .map(str::to_owned)
            };
            Some(Runner {
                name: field("agentName")?,
                url: field("githubUrl")?,
            })
        }
        Material::Other => None,
    }
}

/// Picks the job a runner took out of its console output. The runner names
/// the job in its set-up step, which is mirrored into more than one page, so
/// each name is logged once.
#[derive(Default)]
pub(crate) struct JobLines {
    partial: Vec<u8>,
    job: Option<String>,
}

/// Longer lines are step output, never the line looked for.
const MAX_LINE: usize = 4096;

impl JobLines {
    pub(crate) fn scan(&mut self, placement: &str, data: &[u8]) {
        for chunk in data.split_inclusive(|byte| *byte == b'\n') {
            if self.partial.len() < MAX_LINE {
                self.partial.extend_from_slice(chunk);
            }
            if !chunk.ends_with(b"\n") {
                continue;
            }
            let line = std::mem::take(&mut self.partial);
            if let Some(job) = job_name(&String::from_utf8_lossy(&line))
                && self.job.as_deref() != Some(job)
            {
                tracing::info!(placement = %placement, "runner picked up job \"{job}\"");
                self.job = Some(job.to_owned());
            }
        }
    }
}

/// `2026-10-19T15:51:11.5828490Z Complete job name: Publish release`, with
/// Docker's own timestamp in front for a container.
fn job_name(line: &str) -> Option<&str> {
    let (_, job) = line.trim_end().split_once("Z Complete job name: ")?;
    (!job.is_empty()).then_some(job)
}
