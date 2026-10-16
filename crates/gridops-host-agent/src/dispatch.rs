//! Typed command dispatch.
//!
//! Each command is parsed against the closed protocol, fenced to this host's
//! current session, journaled, and checked against what this agent owns before
//! the helper sees it. A valid command is still refused when it names a
//! resource this agent didn't create for that placement, so a server bug or a
//! stale command can't reach another workload.

use gridops_core::fleet::{
    domain::{ExecutionOs, RuntimeKind},
    ids::{Generation, PlacementId, PreparedEnvironmentId, ResultId},
    protocol::{
        agent::{
            AgentCommandEnvelope, AgentOperation, CommandError, CommandScope, ExpectedAgentSession,
            LocalResourceIdentity, PreparedWorkload,
        },
        outcomes::{
            AgentCommandResult, CommandOutcome, FailureCode, SuccessEvidence, UncertainCode,
        },
        primitives::{ContractError, DEFAULT_JSON_BYTES, Sha256Digest, WireTimestamp, parse_json},
    },
};
use uuid::Uuid;

use crate::{
    helper::{Bootstrap, HelperRequest, HelperResponse, RuntimeHelper},
    journal::{Environment, Journal, JournalError, Receipt},
};

/// Capabilities the operator declared when installing the agent. They widen
/// what a host may do; nothing on the wire can turn them on.
#[derive(Debug, Clone, Copy, Default)]
pub struct Trust {
    pub docker_socket: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("command is malformed: {0}")]
    Contract(#[from] ContractError),
    #[error("command is not valid for this session: {0}")]
    Command(#[from] CommandError),
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

pub struct Dispatcher<H> {
    journal: Journal,
    helper: H,
    trust: Trust,
}

impl<H: RuntimeHelper> Dispatcher<H> {
    pub const fn new(journal: Journal, helper: H, trust: Trust) -> Self {
        Self {
            journal,
            helper,
            trust,
        }
    }

    /// Handles one command body. Returns the journaled result; a redelivered
    /// command returns the same result without running again.
    pub async fn dispatch(
        &self,
        body: &[u8],
        session: &ExpectedAgentSession,
        now: WireTimestamp,
    ) -> Result<AgentCommandResult, DispatchError> {
        self.dispatch_with(body, session, now, None).await
    }

    /// Like [`Self::dispatch`], with registration material for a start.
    pub async fn dispatch_with(
        &self,
        body: &[u8],
        session: &ExpectedAgentSession,
        now: WireTimestamp,
        bootstrap: Option<Bootstrap>,
    ) -> Result<AgentCommandResult, DispatchError> {
        let envelope: AgentCommandEnvelope = parse_json(body, DEFAULT_JSON_BYTES)?;
        envelope.validate_binding(session)?;
        // A command for this session that came too late (it waited behind
        // others, or its deadline ran out in transit) is answered as not run
        // rather than refused. With no answer the server holds it uncertain,
        // and that keeps the whole host from work until the agent restarts.
        let in_time = envelope.validate_against(session, now).is_ok();
        let outcome = match self.journal.receive(&envelope).await? {
            Receipt::Completed(result) => return Ok(*result),
            Receipt::Interrupted => CommandOutcome::Uncertain {
                code: UncertainCode::LocalStateUnknown,
                diagnostic: None,
            },
            Receipt::New if !in_time => refused(FailureCode::Deadline),
            Receipt::New => self.execute(&envelope, bootstrap).await?,
        };
        let mut result = result(&envelope, outcome, now)?;
        if result.validate_against(&envelope).is_err() {
            // The helper reported evidence for something other than this
            // command. Treat the local state as unknown rather than trust it.
            result.outcome = CommandOutcome::Uncertain {
                code: UncertainCode::LocalStateUnknown,
                diagnostic: None,
            };
        }
        self.journal.complete(&result).await?;
        Ok(result)
    }

    /// The result of a command this agent received but never finished: it
    /// stopped part way, so the command's local effect is unknown. Journaled
    /// like any other result, so it is reported once and stays the answer.
    pub async fn interrupted(
        &self,
        envelope: &AgentCommandEnvelope,
        now: WireTimestamp,
    ) -> Result<AgentCommandResult, DispatchError> {
        let result = result(
            envelope,
            CommandOutcome::Uncertain {
                code: UncertainCode::LocalStateUnknown,
                diagnostic: None,
            },
            now,
        )?;
        self.journal.complete(&result).await?;
        Ok(result)
    }

    async fn execute(
        &self,
        envelope: &AgentCommandEnvelope,
        bootstrap: Option<Bootstrap>,
    ) -> anyhow::Result<CommandOutcome> {
        let placement = match &envelope.scope {
            CommandScope::Placement {
                placement_id,
                generation,
                ..
            } => Some((*placement_id, *generation)),
            CommandScope::Host => None,
        };
        let owned = match placement {
            Some((placement_id, generation)) => {
                self.journal
                    .placement_environment(placement_id, generation)
                    .await?
            }
            None => None,
        };
        let operation = &envelope.operation;
        let environment = match (operation, placement) {
            (AgentOperation::PrepareEnvironment(prepare), Some((placement_id, generation))) => {
                if prepare.configuration.requires_docker_socket && !self.trust.docker_socket {
                    return Ok(refused(FailureCode::Permission));
                }
                if prepare.configuration.requires_interactive {
                    return Ok(refused(FailureCode::Unsupported));
                }
                if owned.is_some() {
                    return Ok(refused(FailureCode::Configuration));
                }
                return self.prepare(envelope, placement_id, generation).await;
            }
            (AgentOperation::StartRunner(start), Some(_)) => match owned {
                Some(environment) if environment.id == start.prepared_environment_id => environment,
                _ => return Ok(refused(FailureCode::Permission)),
            },
            (
                AgentOperation::Inspect { expected_resource }
                | AgentOperation::Drain { expected_resource }
                | AgentOperation::Stop {
                    expected_resource, ..
                }
                | AgentOperation::Cleanup { expected_resource },
                Some(_),
            ) => match owned {
                Some(environment)
                    if environment.local_identity.as_ref() == Some(expected_resource) =>
                {
                    environment
                }
                _ => return Ok(refused(FailureCode::Permission)),
            },
            (
                AgentOperation::ExecuteSandbox {
                    expected_resource, ..
                }
                | AgentOperation::SandboxFile {
                    expected_resource, ..
                }
                | AgentOperation::SandboxArtifact {
                    expected_resource, ..
                },
                Some(_),
            ) => match owned {
                Some(environment) if is_owned_fix_sandbox(&environment, expected_resource) => {
                    environment
                }
                _ => return Ok(refused(FailureCode::Permission)),
            },
            (AgentOperation::InspectHost | AgentOperation::DrainHost, None) => {
                return Ok(outcome(
                    self.helper.call(self.request(None, operation, None)).await,
                ));
            }
            // Upgrades arrive with the signed release work; until then the
            // agent refuses them rather than replace itself.
            _ => return Ok(refused(FailureCode::Unsupported)),
        };
        let bootstrap = if matches!(operation, AgentOperation::StartRunner(_)) {
            bootstrap
        } else {
            None
        };
        let response = self
            .helper
            .call(self.request(Some(&environment), operation, bootstrap))
            .await;
        if let HelperResponse::Succeeded { evidence } = &response {
            if let SuccessEvidence::RunnerStarted { local_identity, .. } = evidence.as_ref() {
                self.journal
                    .bind_local_identity(environment.id, local_identity)
                    .await?;
            }
        }
        Ok(outcome(response))
    }

    async fn prepare(
        &self,
        envelope: &AgentCommandEnvelope,
        placement_id: PlacementId,
        generation: Generation,
    ) -> anyhow::Result<CommandOutcome> {
        let AgentOperation::PrepareEnvironment(prepare) = &envelope.operation else {
            return Ok(refused(FailureCode::Configuration));
        };
        let environment = Environment {
            id: PreparedEnvironmentId::try_new(Uuid::new_v4())?,
            placement_id,
            generation,
            workload: prepare.workload,
            runtime_kind: prepare.runtime_kind,
            execution_os: prepare.execution_os,
            docker_socket: prepare.configuration.requires_docker_socket,
            prepare: prepare.clone(),
            local_identity: None,
        };
        let response = self
            .helper
            .call(self.request(Some(&environment), &envelope.operation, None))
            .await;
        if let HelperResponse::Succeeded { evidence } = &response {
            match evidence.as_ref() {
                SuccessEvidence::PreparedEnvironment {
                    prepared_environment_id,
                } if *prepared_environment_id == environment.id => {
                    self.journal.record_environment(&environment).await?;
                }
                _ => {
                    return Ok(CommandOutcome::Uncertain {
                        code: UncertainCode::LocalStateUnknown,
                        diagnostic: None,
                    });
                }
            }
        }
        Ok(outcome(response))
    }

    fn request(
        &self,
        environment: Option<&Environment>,
        operation: &AgentOperation,
        bootstrap: Option<Bootstrap>,
    ) -> HelperRequest {
        HelperRequest {
            environment_id: environment.map(|environment| environment.id),
            docker_socket: self.trust.docker_socket
                && environment.is_some_and(|environment| environment.docker_socket),
            prepared: environment.map(|environment| environment.prepare.clone()),
            bootstrap,
            operation: operation.clone(),
        }
    }
}

/// Sandbox operations reach only the container this agent created for a fix
/// placement on a Linux Docker backend.
fn is_owned_fix_sandbox(environment: &Environment, expected: &LocalResourceIdentity) -> bool {
    environment.workload == PreparedWorkload::FixSandbox
        && environment.runtime_kind == RuntimeKind::Docker
        && environment.execution_os == ExecutionOs::Linux
        && matches!(expected, LocalResourceIdentity::Docker { .. })
        && environment.local_identity.as_ref() == Some(expected)
}

const fn refused(code: FailureCode) -> CommandOutcome {
    CommandOutcome::Failed {
        code,
        retryable: false,
        diagnostic: None,
    }
}

fn outcome(response: HelperResponse) -> CommandOutcome {
    match response {
        HelperResponse::Succeeded { evidence } => CommandOutcome::Succeeded { evidence },
        HelperResponse::Failed { code, retryable } => CommandOutcome::Failed {
            code,
            retryable,
            diagnostic: None,
        },
    }
}

fn result(
    envelope: &AgentCommandEnvelope,
    outcome: CommandOutcome,
    now: WireTimestamp,
) -> anyhow::Result<AgentCommandResult> {
    Ok(AgentCommandResult {
        result_id: ResultId::try_new(Uuid::new_v4())?,
        command_id: envelope.command_id,
        operation_id: envelope.operation_id,
        host_id: envelope.host_id,
        host_epoch: envelope.host_epoch,
        control_plane_incarnation: envelope.control_plane_incarnation,
        authority_session_id: envelope.authority_session_id,
        scope: envelope.scope.clone(),
        generation: match envelope.scope {
            CommandScope::Placement { generation, .. } => Some(generation),
            CommandScope::Host => None,
        },
        request_hash: Sha256Digest::try_from(envelope.request_hash.clone())?,
        observed_at: now,
        outcome,
    })
}
