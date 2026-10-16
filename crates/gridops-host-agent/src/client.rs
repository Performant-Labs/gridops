//! Outbound HTTPS client for the agent routes. The agent never listens on the
//! network; every exchange starts here.

use std::time::Duration;

use gridops_core::fleet::{
    domain::{Architecture, ExecutionOs},
    ids::{AuthoritySessionId, Generation, HostId, OperationId},
    protocol::{
        browser::{ErrorCode, FleetErrorResponse},
        inventory::{
            HandshakeRequest, HandshakeResponse, HeartbeatRequest, HeartbeatResponse,
            InventoryRequest, InventoryResponse,
        },
        outcomes::AgentCommandResult,
    },
    registry::output::{RunnerOutputChunk, RunnerOutputReceipt},
};
use reqwest::StatusCode;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("server URL must use HTTPS (plain HTTP is allowed only for loopback)")]
    InsecureUrl,
    #[error("the server rejected the host credential")]
    Unauthenticated,
    #[error("the server refused the request: {0:?}")]
    Refused(ErrorCode),
    #[error("the server is unavailable: {0}")]
    Unavailable(String),
}

#[derive(Serialize)]
struct EnrollBody<'a> {
    code: &'a str,
    name: &'a str,
    host_os: ExecutionOs,
    architecture: Architecture,
    hardware_fingerprint: Option<&'a str>,
}

#[derive(Deserialize)]
pub struct Enrollment {
    pub host_id: HostId,
    pub credential_generation: Generation,
    pub credential: String,
}

impl std::fmt::Debug for Enrollment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Enrollment")
            .field("host_id", &self.host_id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: Url,
}

impl Client {
    pub fn new(server: &str) -> Result<Self, ClientError> {
        let base = Url::parse(server).map_err(|_| ClientError::InsecureUrl)?;
        let loopback = matches!(base.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        if base.scheme() != "https" && !(base.scheme() == "http" && loopback) {
            return Err(ClientError::InsecureUrl);
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| ClientError::Unavailable(error.to_string()))?;
        Ok(Self { http, base })
    }

    pub async fn enroll(
        &self,
        code: &str,
        name: &str,
        hardware_fingerprint: Option<&str>,
    ) -> Result<Enrollment, ClientError> {
        let request = self.http.post(self.url("agent/enroll")?).json(&EnrollBody {
            code,
            name,
            host_os: crate::probe::host_os(),
            architecture: crate::probe::architecture(),
            hardware_fingerprint,
        });
        send(request).await
    }

    pub async fn handshake(
        &self,
        host: HostId,
        credential: &SecretString,
        body: &HandshakeRequest,
    ) -> Result<HandshakeResponse, ClientError> {
        self.agent(host, credential, "handshake", body).await
    }

    pub async fn inventory(
        &self,
        host: HostId,
        credential: &SecretString,
        body: &InventoryRequest,
    ) -> Result<InventoryResponse, ClientError> {
        self.agent(host, credential, "inventory", body).await
    }

    pub async fn heartbeat(
        &self,
        host: HostId,
        credential: &SecretString,
        body: &HeartbeatRequest,
    ) -> Result<HeartbeatResponse, ClientError> {
        self.agent(host, credential, "heartbeat", body).await
    }

    /// Proposes the next credential, authenticated with the current one.
    pub async fn exchange_rotation(
        &self,
        host: HostId,
        credential: &SecretString,
        operation_id: OperationId,
        next_secret: &SecretString,
    ) -> Result<(), ClientError> {
        let body = serde_json::json!({
            "operation_id": operation_id,
            "next_secret": next_secret.expose_secret(),
        });
        self.agent::<_, serde_json::Value>(host, credential, "credentials/rotate", &body)
            .await
            .map(drop)
    }

    /// Confirms the agent holds the new credential, authenticated with it.
    pub async fn acknowledge_rotation(
        &self,
        host: HostId,
        next_credential: &SecretString,
        operation_id: OperationId,
    ) -> Result<(), ClientError> {
        let body = serde_json::json!({ "operation_id": operation_id });
        self.agent::<_, serde_json::Value>(host, next_credential, "credentials/ack", &body)
            .await
            .map(drop)
    }

    /// Long-polls (up to 25 s) for commands bound to this session.
    pub async fn poll_commands(
        &self,
        host: HostId,
        credential: &SecretString,
        session: AuthoritySessionId,
    ) -> Result<Vec<serde_json::Value>, ClientError> {
        #[derive(Deserialize)]
        struct Poll {
            commands: Vec<serde_json::Value>,
        }
        let body = serde_json::json!({
            "authority_session_id": session, "wait_ms": 25_000, "limit": 1,
        });
        let request = self
            .http
            .post(self.url(&format!("agent/hosts/{host}/commands/poll"))?)
            .timeout(Duration::from_secs(40))
            .bearer_auth(credential.expose_secret())
            .json(&body);
        Ok(send::<Poll>(request).await?.commands)
    }

    /// One-time registration material for a start command.
    pub async fn bootstrap(
        &self,
        host: HostId,
        credential: &SecretString,
        reference: &str,
    ) -> Result<String, ClientError> {
        #[derive(Deserialize)]
        struct Material {
            material: String,
        }
        let body = serde_json::json!({ "reference": reference });
        Ok(self
            .agent::<_, Material>(host, credential, "bootstrap", &body)
            .await?
            .material)
    }

    /// Uploads a batch of the agent's own log lines.
    pub async fn post_logs(
        &self,
        host: HostId,
        credential: &SecretString,
        lines: &[crate::logs::LogLine],
        dropped: u64,
    ) -> Result<(), ClientError> {
        let body = serde_json::json!({ "lines": lines, "dropped": dropped });
        self.agent::<_, serde_json::Value>(host, credential, "logs", &body)
            .await
            .map(drop)
    }

    /// Uploads a chunk of a runner's console output.
    pub async fn post_output(
        &self,
        host: HostId,
        credential: &SecretString,
        chunk: &RunnerOutputChunk,
    ) -> Result<RunnerOutputReceipt, ClientError> {
        self.agent(host, credential, "output", chunk).await
    }

    pub async fn post_result(
        &self,
        host: HostId,
        credential: &SecretString,
        result: &AgentCommandResult,
    ) -> Result<(), ClientError> {
        self.agent::<_, serde_json::Value>(host, credential, "commands/result", result)
            .await
            .map(drop)
    }

    async fn agent<B: Serialize, T: DeserializeOwned>(
        &self,
        host: HostId,
        credential: &SecretString,
        action: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        let request = self
            .http
            .post(self.url(&format!("agent/hosts/{host}/{action}"))?)
            .bearer_auth(credential.expose_secret())
            .json(body);
        send(request).await
    }

    fn url(&self, path: &str) -> Result<Url, ClientError> {
        self.base
            .join(&format!("api/v1/fleet/{path}"))
            .map_err(|_| ClientError::InsecureUrl)
    }
}

async fn send<T: DeserializeOwned>(request: reqwest::RequestBuilder) -> Result<T, ClientError> {
    let response = request
        .send()
        .await
        .map_err(|error| ClientError::Unavailable(error.without_url().to_string()))?;
    let status = response.status();
    if status.is_success() {
        return response
            .json()
            .await
            .map_err(|error| ClientError::Unavailable(error.without_url().to_string()));
    }
    if status == StatusCode::UNAUTHORIZED {
        return Err(ClientError::Unauthenticated);
    }
    match response.json::<FleetErrorResponse>().await {
        Ok(error) if !status.is_server_error() && status != StatusCode::TOO_MANY_REQUESTS => {
            Err(ClientError::Refused(error.code))
        }
        _ => Err(ClientError::Unavailable(status.to_string())),
    }
}
