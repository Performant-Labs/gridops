//! The host's runtimes behind one helper: Docker containers and native
//! runners. Each request goes to the runtime that prepared its environment.

use gridops_core::fleet::{
    domain::RuntimeKind,
    protocol::{
        agent::{AgentOperation, LocalResourceIdentity},
        inventory::ObservedBackend,
        outcomes::FailureCode,
    },
};

use crate::{
    docker::DockerRuntime,
    helper::{HelperRequest, HelperResponse, RunnerOutput, RuntimeHelper},
    native::NativeRuntime,
};

pub struct HostRuntime {
    docker: DockerRuntime,
    native: NativeRuntime,
}

impl HostRuntime {
    pub const fn new(docker: DockerRuntime, native: NativeRuntime) -> Self {
        Self { docker, native }
    }
}

impl RuntimeHelper for HostRuntime {
    async fn call(&self, request: HelperRequest) -> HelperResponse {
        let kind = match (&request.operation, &request.prepared) {
            (AgentOperation::PrepareEnvironment(prepare), _) => Some(prepare.runtime_kind),
            (_, Some(prepared)) => Some(prepared.runtime_kind),
            (
                AgentOperation::Inspect { expected_resource }
                | AgentOperation::Drain { expected_resource }
                | AgentOperation::Stop {
                    expected_resource, ..
                }
                | AgentOperation::Cleanup { expected_resource },
                None,
            ) => Some(match expected_resource {
                LocalResourceIdentity::Docker { .. } => RuntimeKind::Docker,
                LocalResourceIdentity::NativeProcess { .. } => RuntimeKind::NativeProcess,
                LocalResourceIdentity::TartVm { .. } => RuntimeKind::TartVm,
            }),
            (AgentOperation::InspectHost, _) => Some(RuntimeKind::Docker),
            _ => None,
        };
        match kind {
            Some(RuntimeKind::Docker) => self.docker.call(request).await,
            Some(RuntimeKind::NativeProcess) => self.native.call(request).await,
            _ => HelperResponse::Failed {
                code: FailureCode::Unsupported,
                retryable: false,
            },
        }
    }

    async fn output(&self, resource: LocalResourceIdentity) -> Option<RunnerOutput> {
        match resource {
            LocalResourceIdentity::Docker { .. } => self.docker.output(resource).await,
            LocalResourceIdentity::NativeProcess { .. } => self.native.output(resource).await,
            LocalResourceIdentity::TartVm { .. } => None,
        }
    }

    async fn backends(&self) -> Vec<ObservedBackend> {
        let mut backends = self.docker.backends().await;
        backends.extend(self.native.backends().await);
        backends
    }
}
