# Evidence: #147 (unchanged source the change relies on)

Entries by F (Phase 6). Facts about sources outside this repository (gitlab-runner v19.6.0, GitLab's runner API, Docker Hub) are cited in `handoff-F.md` under "External facts confirmed", because the gate can attach only repository files.

- **Fact:** the helper's `docker_socket` flag is true only when the host trusts socket sharing and the prepared profile asks for the socket, so the GitLab Start can refuse a profile without the socket by reading it.
  **Source:** `crates/gridops-host-agent/src/dispatch.rs:365-366`
  **Verbatim excerpt:**
  > docker_socket: self.trust.docker_socket
  > && environment.is_some_and(|environment| environment.docker_socket),

- **Fact:** the journaled environment's `docker_socket` is the profile's `requires_docker_socket`.
  **Source:** `crates/gridops-host-agent/src/dispatch.rs:336`
  **Verbatim excerpt:**
  > docker_socket: prepare.configuration.requires_docker_socket,

- **Fact:** a Prepare whose profile asks for the socket on a host that does not trust socket sharing is already refused with `Permission` in dispatch, before the helper. This is the existing `Permission` refusal that the GitLab `Configuration` refusal at Prepare (W-5) leaves unchanged.
  **Source:** `crates/gridops-host-agent/src/dispatch.rs:244-246`
  **Verbatim excerpt:**
  > if prepare.configuration.requires_docker_socket && !self.trust.docker_socket {
  > return Ok(refused(FailureCode::Permission));
  > }

- **Fact:** the server cleans up a placement whose Inspect failed with `ResourceUnavailable`, whatever the diagnostic, so an exited GitLab runner that carries `IdleExpired` or `RegistrationRefused` is still cleaned up (D147-2).
  **Source:** `crates/gridops-core/src/fleet/placement.rs:182-188`
  **Verbatim excerpt:**
  > Ok(matches!(
  > result.outcome,
  > CommandOutcome::Failed {
  > code: crate::fleet::protocol::outcomes::FailureCode::ResourceUnavailable,
  > ..
  > }
  > ))

- **Fact:** the reconciler gives a start command 30 s, the budget that `GITLAB_START_READY_TIMEOUT_SECS` (15 s) has to fit inside.
  **Source:** `crates/gridops-reconciler/src/fleet.rs:48`
  **Verbatim excerpt:**
  > const START_COMMAND_DEADLINE_MILLIS: i64 = 30_000;

- **Fact:** the command registry keeps a 5 s delivery margin off that deadline.
  **Source:** `crates/gridops-core/src/fleet/registry/commands.rs:32`
  **Verbatim excerpt:**
  > const DELIVERY_MARGIN_MILLIS: i64 = 5_000;

- **Fact:** a GitLab target is a GitLab project or group, which is what the host agent's GitLab branch keys on.
  **Source:** `crates/gridops-core/src/fleet/domain.rs:205-207`
  **Verbatim excerpt:**
  > pub const fn is_gitlab(&self) -> bool {
  > matches!(self, Self::GitlabProject { .. } | Self::GitlabGroup { .. })
  > }

- **Fact:** on the manager, a runner container is given the runners' Docker socket only when the manager shares one, which is why a GitLab start is refused without it.
  **Source:** `crates/gridops-manager/src/main.rs:1909-1912`
  **Verbatim excerpt:**
  > binds: state
  > .shared_docker_socket
  > .as_ref()
  > .map(|socket| vec![format!("{}:/var/run/docker.sock", socket.host_path)]),
