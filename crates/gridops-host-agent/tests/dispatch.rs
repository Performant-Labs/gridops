//! Drives the dispatcher, journal and helper socket together. Commands are
//! real protocol envelopes; the runtime is a recording stand-in, so each test
//! also proves what never reached the privileged side.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use gridops_core::fleet::{
    ids::Generation,
    protocol::{
        agent::{AgentCommandEnvelope, AgentOperation, ExpectedAgentSession},
        outcomes::{CommandOutcome, FailureCode, SuccessEvidence, UncertainCode},
        primitives::WireTimestamp,
    },
};
use gridops_host_agent::{
    dispatch::{DispatchError, Dispatcher, Trust},
    helper::{HelperRequest, HelperResponse, RuntimeHelper},
    journal::{Journal, JournalError},
};
use serde_json::{Value, json};

fn id(n: u32) -> String {
    format!("{n:08x}-0000-4000-8000-000000000001")
}

fn command(n: u32, operation: Value) -> Value {
    let mut value = json!({
        "protocol_version":1,"command_id":id(100 + n),"operation_id":id(2),"idempotency_key":format!("cmd:{n}"),
        "host_id":id(3),"host_epoch":1,"control_plane_incarnation":id(4),"authority_session_id":id(5),
        "scope":{"kind":"placement","placement_id":id(6),"backend_id":id(7),"generation":1},
        "expected_revision":1,"issued_at":"2026-10-09T12:00:00.000Z","deadline_at":"2026-10-09T12:01:00.000Z",
        "request_hash":""
    });
    value["operation"] = operation;
    value
}

fn prepare(workload: &str, docker_socket: bool) -> Value {
    json!({
        "kind":"prepare_environment","execution_profile_id":id(8),"profile_revision":1,"workload":workload,
        "target":{"platform":"github_repository","installation_id":1,"repository_id":10},
        "runtime_kind":"docker","execution_os":"linux","architecture":"x64","mode":"ephemeral",
        "image":"runner:fixture","labels":[],"configuration":{"requires_docker_socket":docker_socket},
        "resource_request":{"cpu_millis":1000,"memory_mib":1024,"disk_bytes":1_073_741_824},
        "allocation_ids":[id(9)]
    })
}

fn start(environment: &str) -> Value {
    json!({"kind":"start_runner","prepared_environment_id":environment,"execution_profile_id":id(8),
        "profile_revision":1,"bootstrap_reference":id(15),"bootstrap_expires_at":"2026-10-09T12:01:00.000Z",
        "resource_request":{"cpu_millis":1000,"memory_mib":1024,"disk_bytes":1_073_741_824},"allocation_ids":[id(9)]})
}

fn exec(container: &str) -> Value {
    json!({"kind":"execute_sandbox","expected_resource":{"kind":"docker","object_id":container},
        "program":"/bin/sh","argv":["-c","true"],"working_directory":null,"timeout_ms":1000,"output_limit_bytes":1024})
}

fn sealed(value: Value) -> Result<Vec<u8>> {
    let envelope = serde_json::from_value::<AgentCommandEnvelope>(value)?.seal_hash()?;
    Ok(serde_json::to_vec(&envelope)?)
}

/// Records every request and plays a runtime that creates `container`.
#[derive(Default)]
struct Recorder {
    requests: Mutex<Vec<HelperRequest>>,
}

impl Recorder {
    fn calls(&self) -> usize {
        self.requests.lock().map_or(0, |requests| requests.len())
    }
}

impl RuntimeHelper for Recorder {
    async fn call(&self, request: HelperRequest) -> HelperResponse {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(request.clone());
        }
        let evidence = match (&request.operation, request.environment_id) {
            (AgentOperation::PrepareEnvironment(_), Some(prepared_environment_id)) => {
                SuccessEvidence::PreparedEnvironment {
                    prepared_environment_id,
                }
            }
            (AgentOperation::StartRunner(start), _) => SuccessEvidence::RunnerStarted {
                prepared_environment_id: start.prepared_environment_id,
                execution_profile_id: start.execution_profile_id,
                profile_revision: start.profile_revision,
                local_identity: serde_json::from_value(
                    json!({"kind":"docker","object_id":"owned-container"}),
                )
                .unwrap_or_else(|_| unreachable!()),
            },
            (
                AgentOperation::ExecuteSandbox {
                    expected_resource, ..
                },
                _,
            ) => SuccessEvidence::SandboxExecuted {
                local_identity: expected_resource.clone(),
                exit_status: serde_json::from_value(json!({"kind":"exited","code":0}))
                    .unwrap_or_else(|_| unreachable!()),
                stdout: None,
                stderr: None,
            },
            _ => {
                return HelperResponse::Failed {
                    code: FailureCode::Unsupported,
                    retryable: false,
                };
            }
        };
        HelperResponse::Succeeded {
            evidence: Box::new(evidence),
        }
    }
}

struct Harness {
    dispatcher: Dispatcher<Arc<Recorder>>,
    recorder: Arc<Recorder>,
    journal: Journal,
    session: ExpectedAgentSession,
    now: WireTimestamp,
    _dir: TempDir,
}

struct TempDir(std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Harness {
    async fn new(trust: Trust) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!("gridops-host-agent-{}", uuid::Uuid::new_v4()));
        let journal = Journal::open(&dir.join("journal.sqlite")).await?;
        let recorder = Arc::new(Recorder::default());
        let sample = serde_json::from_value::<AgentCommandEnvelope>(command(
            0,
            json!({"kind":"inspect_host"}),
        ))?;
        Ok(Self {
            dispatcher: Dispatcher::new(journal.clone(), Arc::clone(&recorder), trust),
            recorder,
            journal,
            session: ExpectedAgentSession {
                host_id: sample.host_id,
                host_epoch: sample.host_epoch,
                control_plane_incarnation: sample.control_plane_incarnation,
                authority_session_id: sample.authority_session_id,
                credential_generation: Generation::new(1)?,
                authority_expires_at: WireTimestamp::from_millis(
                    sample.issued_at.millis() + 90_000,
                )?,
            },
            now: sample.issued_at,
            _dir: TempDir(dir),
        })
    }

    async fn send(&self, body: &[u8]) -> Result<CommandOutcome, DispatchError> {
        Ok(self
            .dispatcher
            .dispatch(body, &self.session, self.now)
            .await?
            .outcome)
    }

    /// Prepares and starts one environment, returning its ID.
    async fn started(&self, workload: &str) -> Result<String> {
        let CommandOutcome::Succeeded { evidence } = self
            .send(&sealed(command(1, prepare(workload, false)))?)
            .await?
        else {
            anyhow::bail!("prepare failed");
        };
        let SuccessEvidence::PreparedEnvironment {
            prepared_environment_id,
        } = *evidence
        else {
            anyhow::bail!("unexpected prepare evidence");
        };
        let environment = prepared_environment_id.to_string();
        let outcome = self.send(&sealed(command(2, start(&environment)))?).await?;
        assert!(
            matches!(outcome, CommandOutcome::Succeeded { .. }),
            "{outcome:?}"
        );
        Ok(environment)
    }
}

fn refused(outcome: &CommandOutcome, expected: FailureCode) -> bool {
    matches!(outcome, CommandOutcome::Failed { code, retryable: false, .. } if *code == expected)
}

#[tokio::test]
async fn sandbox_commands_reach_only_the_owned_fix_container() -> Result<()> {
    let h = Harness::new(Trust::default()).await?;
    h.started("fix_sandbox").await?;
    let outcome = h
        .send(&sealed(command(3, exec("owned-container")))?)
        .await?;
    assert!(
        matches!(outcome, CommandOutcome::Succeeded { .. }),
        "{outcome:?}"
    );
    let calls = h.recorder.calls();

    // Another container, a container from another placement generation, and
    // a CI runner's container are all refused before the helper sees them.
    assert!(refused(
        &h.send(&sealed(command(4, exec("someone-elses-container")))?)
            .await?,
        FailureCode::Permission
    ));
    let mut other_generation = command(5, exec("owned-container"));
    other_generation["scope"]["generation"] = 2.into();
    assert!(refused(
        &h.send(&sealed(other_generation)?).await?,
        FailureCode::Permission
    ));
    assert_eq!(h.recorder.calls(), calls);

    let ci = Harness::new(Trust::default()).await?;
    ci.started("ci_runner").await?;
    let calls = ci.recorder.calls();
    assert!(refused(
        &ci.send(&sealed(command(3, exec("owned-container")))?)
            .await?,
        FailureCode::Permission
    ));
    assert_eq!(ci.recorder.calls(), calls);
    Ok(())
}

#[tokio::test]
async fn arbitrary_and_cross_host_commands_never_reach_the_helper() -> Result<()> {
    let h = Harness::new(Trust::default()).await?;
    for kind in ["host_shell", "powershell", "desktop_click", "exec"] {
        let mut value = command(1, exec("owned-container"));
        value["operation"]["kind"] = kind.into();
        let body = serde_json::to_vec(&value)?;
        assert!(
            matches!(h.send(&body).await, Err(DispatchError::Contract(_))),
            "{kind}"
        );
    }
    for (field, value) in [
        ("host_id", json!(id(99))),
        ("host_epoch", json!(2)),
        ("authority_session_id", json!(id(98))),
    ] {
        let mut other = command(2, json!({"kind":"inspect_host"}));
        other["scope"] = json!({"kind":"host"});
        other[field] = value;
        assert!(
            matches!(
                h.send(&sealed(other)?).await,
                Err(DispatchError::Command(_))
            ),
            "{field}"
        );
    }
    // Lifecycle commands for resources this agent never created are refused.
    let stop = json!({"kind":"stop","expected_resource":{"kind":"docker","object_id":"owned-container"},"mode":"cancel_busy"});
    assert!(refused(
        &h.send(&sealed(command(3, stop))?).await?,
        FailureCode::Permission
    ));
    assert_eq!(h.recorder.calls(), 0);
    Ok(())
}

#[tokio::test]
async fn docker_socket_sharing_requires_declared_trust() -> Result<()> {
    let untrusted = Harness::new(Trust::default()).await?;
    let body = sealed(command(1, prepare("ci_runner", true)))?;
    assert!(refused(
        &untrusted.send(&body).await?,
        FailureCode::Permission
    ));
    assert_eq!(untrusted.recorder.calls(), 0);

    let trusted = Harness::new(Trust {
        docker_socket: true,
    })
    .await?;
    assert!(matches!(
        trusted.send(&body).await?,
        CommandOutcome::Succeeded { .. }
    ));
    let requests = trusted
        .recorder
        .requests
        .lock()
        .map_err(|_| anyhow::anyhow!("poisoned"))?;
    assert!(requests[0].docker_socket);
    Ok(())
}

#[tokio::test]
async fn redelivery_replays_the_journal_and_interrupted_work_is_uncertain() -> Result<()> {
    let h = Harness::new(Trust::default()).await?;
    let body = sealed(command(1, prepare("ci_runner", false)))?;
    let first = h.dispatcher.dispatch(&body, &h.session, h.now).await?;
    let again = h.dispatcher.dispatch(&body, &h.session, h.now).await?;
    assert_eq!(first, again);
    assert_eq!(h.recorder.calls(), 1);

    // Same command ID, different request: a conflict, never a second run.
    let mut changed = command(1, prepare("ci_runner", false));
    changed["idempotency_key"] = "cmd:changed".into();
    assert!(matches!(
        h.send(&sealed(changed)?).await,
        Err(DispatchError::Journal(JournalError::Conflict))
    ));

    // A receipt with no result means the agent stopped mid-operation.
    let interrupted = sealed(command(2, json!({"kind":"inspect_host"})).tap_host())?;
    h.journal
        .receive(&serde_json::from_slice(&interrupted)?)
        .await?;
    let outcome = h.send(&interrupted).await?;
    assert_eq!(
        outcome,
        CommandOutcome::Uncertain {
            code: UncertainCode::LocalStateUnknown,
            diagnostic: None
        }
    );
    assert_eq!(h.recorder.calls(), 1);
    Ok(())
}

trait HostScope {
    fn tap_host(self) -> Self;
}
impl HostScope for Value {
    fn tap_host(mut self) -> Self {
        self["scope"] = json!({"kind":"host"});
        self
    }
}

#[cfg(unix)]
#[tokio::test]
async fn helper_socket_refuses_requests_outside_the_protocol() -> Result<()> {
    use gridops_host_agent::helper::{SocketHelper, serve};
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

    let dir = std::env::temp_dir().join(format!("gha-{}", &uuid::Uuid::new_v4().to_string()[..8]));
    std::fs::create_dir_all(&dir)?;
    let _cleanup = TempDir(dir.clone());
    let path = dir.join("helper.sock");
    let recorder = Arc::new(Recorder::default());
    tokio::spawn(serve(
        tokio::net::UnixListener::bind(&path)?,
        Arc::clone(&recorder),
    ));

    for raw in [
        json!({"kind":"host_shell","command":"rm -rf /"}),
        json!({"environment_id":null,"docker_socket":false,"operation":{"kind":"shell","command":"id"}}),
        json!({"environment_id":null,"docker_socket":false,"operation":{"kind":"inspect_host"},"extra":true}),
    ] {
        let mut stream = tokio::net::UnixStream::connect(&path).await?;
        stream.write_all(format!("{raw}\n").as_bytes()).await?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).await?;
        let response: HelperResponse = serde_json::from_str(&line)?;
        assert_eq!(
            response,
            HelperResponse::Failed {
                code: FailureCode::Configuration,
                retryable: false
            }
        );
    }
    assert_eq!(recorder.calls(), 0);

    let response = SocketHelper::new(path)
        .call(serde_json::from_value(json!({"environment_id":null,"docker_socket":false,"operation":{"kind":"inspect_host"}}))?)
        .await;
    assert_eq!(
        response,
        HelperResponse::Failed {
            code: FailureCode::Unsupported,
            retryable: false
        }
    );
    assert_eq!(recorder.calls(), 1);
    Ok(())
}
