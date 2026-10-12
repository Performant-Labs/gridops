//! The privileged runtime helper boundary.
//!
//! The network-facing agent runs unprivileged and never touches a runtime
//! directly. It sends the helper one typed, already-authorized operation per
//! request over a local socket. The request type is a closed set of fleet
//! operations, so there is no way to express a host shell, a desktop action or
//! another host's resource, and anything that doesn't parse is refused before
//! the helper sees it.

use std::future::Future;

use gridops_core::fleet::{
    ids::PreparedEnvironmentId,
    protocol::{
        agent::AgentOperation,
        outcomes::{FailureCode, SuccessEvidence},
    },
};
use serde::{Deserialize, Serialize};

/// Requests and responses on the socket are bounded like agent JSON.
pub const MAX_MESSAGE_BYTES: usize = gridops_core::fleet::protocol::primitives::MAX_JSON_BYTES;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperRequest {
    /// The journaled environment the operation runs in, when it has one.
    pub environment_id: Option<PreparedEnvironmentId>,
    /// Whether the operator declared Docker socket sharing as trusted on this
    /// host. The helper mounts the socket only when this and the profile agree.
    pub docker_socket: bool,
    pub operation: AgentOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HelperResponse {
    Succeeded { evidence: Box<SuccessEvidence> },
    Failed { code: FailureCode, retryable: bool },
}

pub trait RuntimeHelper: Send + Sync {
    fn call(&self, request: HelperRequest) -> impl Future<Output = HelperResponse> + Send;
}

impl<H: RuntimeHelper> RuntimeHelper for std::sync::Arc<H> {
    fn call(&self, request: HelperRequest) -> impl Future<Output = HelperResponse> + Send {
        (**self).call(request)
    }
}

/// Used until a platform adapter is installed: every operation fails as
/// unsupported, so nothing runs by accident.
pub struct NoRuntime;

impl RuntimeHelper for NoRuntime {
    async fn call(&self, _request: HelperRequest) -> HelperResponse {
        HelperResponse::Failed {
            code: FailureCode::Unsupported,
            retryable: false,
        }
    }
}

#[cfg(unix)]
pub use socket::{SocketHelper, serve};

#[cfg(unix)]
mod socket {
    use std::{path::PathBuf, sync::Arc, time::Duration};

    use anyhow::Result;
    use gridops_core::fleet::protocol::{outcomes::FailureCode, primitives::parse_json};
    use tokio::{
        io::{AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
        net::{UnixListener, UnixStream},
    };

    use super::{HelperRequest, HelperResponse, MAX_MESSAGE_BYTES, RuntimeHelper};

    /// Sandbox commands may run for five minutes; the socket allows a margin.
    const CALL_TIMEOUT: Duration = Duration::from_secs(330);

    /// The agent's side of the socket.
    pub struct SocketHelper {
        path: PathBuf,
    }

    impl SocketHelper {
        pub const fn new(path: PathBuf) -> Self {
            Self { path }
        }

        async fn exchange(&self, request: &HelperRequest) -> Result<HelperResponse> {
            let mut stream = UnixStream::connect(&self.path).await?;
            let mut line = serde_json::to_vec(request)?;
            line.push(b'\n');
            stream.write_all(&line).await?;
            let response = read_line(&mut BufReader::new(stream)).await?;
            Ok(parse_json(&response, MAX_MESSAGE_BYTES)?)
        }
    }

    impl RuntimeHelper for SocketHelper {
        async fn call(&self, request: HelperRequest) -> HelperResponse {
            match tokio::time::timeout(CALL_TIMEOUT, self.exchange(&request)).await {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    tracing::warn!(%error, "runtime helper call failed");
                    unavailable()
                }
                Err(_) => unavailable(),
            }
        }
    }

    /// The helper's side: one request and one response per connection.
    /// Unparseable or oversized requests are refused without reaching the
    /// runtime.
    pub async fn serve<H: RuntimeHelper + 'static>(
        listener: UnixListener,
        helper: Arc<H>,
    ) -> Result<()> {
        loop {
            let (stream, _) = listener.accept().await?;
            let helper = Arc::clone(&helper);
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let response = match read_line(&mut BufReader::new(read)).await {
                    Ok(line) => match parse_json::<HelperRequest>(&line, MAX_MESSAGE_BYTES) {
                        Ok(request) => helper.call(request).await,
                        Err(_) => HelperResponse::Failed {
                            code: FailureCode::Configuration,
                            retryable: false,
                        },
                    },
                    Err(_) => return,
                };
                if let Ok(mut line) = serde_json::to_vec(&response) {
                    line.push(b'\n');
                    let _ = write.write_all(&line).await;
                }
            });
        }
    }

    async fn read_line<R: AsyncRead + Unpin>(reader: &mut BufReader<R>) -> Result<Vec<u8>> {
        let mut line = Vec::new();
        let limit = u64::try_from(MAX_MESSAGE_BYTES + 1)?;
        reader.take(limit).read_until(b'\n', &mut line).await?;
        anyhow::ensure!(
            line.last() == Some(&b'\n'),
            "helper message is unterminated or too large"
        );
        line.pop();
        Ok(line)
    }

    const fn unavailable() -> HelperResponse {
        HelperResponse::Failed {
            code: FailureCode::Dependency,
            retryable: true,
        }
    }
}
