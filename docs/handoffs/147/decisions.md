# Decision journal: #147 GitLab runners in Linux Docker (host agent and manager)

## A (Phase 3, up-front plan review): 2026-11-20T15:19:02-08:00
- **Decided:** BLOCK, with 3 block findings and 5 warn findings (see `handoff-A.md`).
  - (1) **[correctness]** The plan puts a pinned default `gitlab/gitlab-runner` image in `docker.rs`, "overridable by the pool's `docker_image`". The established layering doesn't allow that:
    - the server chooses the image: `targets.rs::pool_docker_image` and `STOCK_DOCKER_IMAGE` on the fleet path, `ProvisioningPlan.image` on the manager path;
    - the agent runs `PrepareEnvironment.image`, which is always non-empty, and Docker pools must have a `docker_image` (#103);
    - so the default can't apply on either path, and the two paths would diverge.

    The meaning of `docker_image` for a GitLab pool (runner image or default job image) is also unpinned.
  - (2) **[correctness]** The brief drops #137's "start command's result" for `RegistrationRefused`, and today's Start (`feed_secret`) can't observe a refusal. An Inspect reported as `Configuration` would never satisfy `reported_gone` (which needs `ResourceUnavailable`), so the exited runner would never be cleaned up.
  - (3) **[completeness]** `HelperResponse::Failed` can't carry a diagnostic, and `dispatch::outcome` hardcodes `diagnostic: None`. `helper.rs` and `dispatch.rs` aren't in the owned list.
  - The warnings:
    - parse `GitlabRunnerMaterial` instead of adding a private `Material` copy;
    - refuse at Prepare when the profile doesn't request the socket, and #139 must set `requires_docker_socket` for GitLab profiles;
    - the manager's `runner_command` and `take_bootstrap_secret` branch on mode only, its containers have no platform label, and its listing has no exit code;
    - the docs should name `GRIDOPS_RUNNER_DOCKER_SOCKET` and `GRIDOPS_AGENT_DOCKER_SOCKET`, not just `compose.no-docker-socket.yaml`;
    - the GitLab script should be its own function, not wrapped in the GitHub-specific `instrumented_runner_script`.
  - Accepted as consistent:
    - one shared GitLab script in `gridops-core::provisioning`;
    - the token on stdin;
    - parsing the core `GitlabRunnerMaterial`;
    - advertising the D6 kinds in `docker_backend`;
    - reusing the existing socket grant;
    - replacing the manager's `("gitlab", _)` placeholder;
    - `exitReason` through `GITLAB_EXIT_REASONS`;
    - `ManagedRunner.exit_reason` (D15).
- **Assumed:**
  - `gitlab-runner` learns that GitLab refused its token only from a network request after it starts, so the refusal usually arrives after `feed_secret`'s roughly 0.5 s detach. I didn't verify this against gitlab-runner's source; the brief review's NV-1 to NV-3 cover the runner's behaviour.
  - The host agent and its privileged helper run from the same build. That is why an optional, skip-if-none `diagnostic` field is safe across an upgrade.
- **Hedged:**
  - Finding 1 offers two resolutions: (a) `docker_image` is the runner image and the server applies the default from a core constant; or (b) `docker_image` is the default job image and the runner image is a fixed core constant. I recommend (a) because it leaves the agent and manager contracts unchanged.
  - Finding 2 leaves the Start mechanism to F and pins only the reported outcomes. A late exit 77 on Inspect needs either a new `ResourceUnavailable` + `RegistrationRefused` pairing or no diagnostic; O must choose and record it for #139.
  - Finding 3 is marked block, not warn. It touches the privileged helper boundary, and T needs to know which layer to test. No other wave-1 issue owns these files, so the fix is only a brief amendment.
  - No Reuse map (`survey.md`) was provided, so the brief's named analogues were the baseline.
- **Evidence:**
  - Read: the brief, `147-brief-result-r1.md`, `137-brief.md` (D5, D9, D10; lines 419-435 and 673-678), `docs/handoffs/137/handoff-A.md` and `decisions.md`, epic #136, and issue #139 (`gh issue view`).
  - Host agent: `crates/gridops-host-agent/src/{docker.rs (full),helper.rs (full),dispatch.rs (full),runtime.rs (full)}`, plus `native.rs` and `main.rs` (socket settings).
  - Core: `crates/gridops-core/src/provisioning.rs` (full), `fleet/protocol/agent.rs` (`ExecutionConfiguration`, `PrepareEnvironment`, `GitlabRunnerMaterial`), `fleet/protocol/outcomes.rs` (`DiagnosticCode`, `FailureCode`), `fleet/protocol/primitives.rs` (`ImageReference`), `fleet/targets.rs:430-480`, `fleet/placement.rs:163-189`, `fleet/admission.rs:725` and `models.rs:400-440`.
  - Reconciler: `crates/gridops-reconciler/src/fleet.rs:480-560` (`follow`).
  - Manager: `crates/gridops-manager/src/main.rs` (`ProvisionRunner`, `GitlabBootstrap`, `validate`, `ManagedRunner`, `shared_docker_socket`, `list_runners`, `tart_runners`, `start_runner`, `provision_runner_inner`, `runner_command`, `take_bootstrap_secret`).
  - Deployment and docs: `compose.no-docker-socket.yaml` and `docs/fleet-agent.md` (the socket rows).

## A (Phase 3, up-front plan review, round 2): 2026-11-20T15:33:39-08:00
- **Decided:** BLOCK, with 1 block finding and 4 warn findings (see `handoff-A.md`; round 1 is at `5339e22`).
  - The three round-1 blocks and five round-1 warnings are resolved by the brief's "Amendments (architecture review R1)". Each was checked against the code: the image is chosen server-side (D147-1); Inspect always reports `ResourceUnavailable`, so `reported_gone` fires; `prepared` reaches Inspect; `is_gitlab()` exists; the helper's `CALL_TIMEOUT` is 330 s; manager containers are created with `auto_remove: false`.
  - (1) **[correctness]** B-2's readiness wait, "a pinned timeout such as 60 s", is twice the reconciler's 30 s start-command deadline (`START_COMMAND_DEADLINE_MILLIS`, `gridops-reconciler/src/fleet.rs:48`). The agent runs one command at a time (`supervisor.rs` `in_flight`; one command per poll in `registry/commands.rs:91-97`). The heartbeat's `cancel_undeliverable` turns a late-running start `uncertain` and fails the placements queued behind it (`commands.rs:735-790`, the 2026-10-16 incident). Fix: pin the wait at 15 s or less, and give the script's verify `curl` a smaller `--max-time`. Or drop the Start wait and record a deviation from #137 D5.
  - The warnings:
    - the `HelperResponse::Failed` field addition breaks struct literals in `crates/gridops-host-agent/tests/` and `crates/gridops-api/tests/fleet_http/placement.rs`, which aren't in Owns;
    - non-secret values reach runner scripts through env vars elsewhere (`GRIDOPS_REGISTRATION_URL`), not shell-quoted script text;
    - the late `ResourceUnavailable` with `RegistrationRefused` pairing contradicts the `outcomes.rs:41-43` doc comment and needs deviation row D147-2;
    - the Docker executor's sibling containers and cache volumes aren't covered by Stop or Cleanup.
- **Assumed:**
  - Start commands are normally delivered with close to 30 s left. The agent polls every 0.5 s, so the 15 s bound is a slow-path cap, and the typical verify takes a second or two.
  - `run-single` accepts the runner-config Docker flags (for example `--docker-disable-cache`). I didn't verify this against the pinned gitlab-runner version; it's F's to confirm with the brief review's NV items.
- **Hedged:**
  - Finding 1 gives two resolutions: (a) a bounded wait, recommended because it keeps #137 D5; or (b) no wait, with the refusal reported only on Inspect.
  - Finding 1 traces back to my own round-1 suggestion, which left the mechanism unbounded.
  - W-2 is a warn, not a block. The seam is planned and reviewed; only the list of mechanical fallout sites is incomplete. No wave-1 issue owns `crates/gridops-api/tests/fleet_http/placement.rs` (epic #136 table).
- **Evidence:**
  - Read: the amended brief, `147-brief-result-r1.md`, round-1 `handoff-A.md`, epic #136 (`gh issue view 136`) and `docs/handoffs/137/handoff-A.md` (the round-numbering precedent).
  - Host agent: `crates/gridops-host-agent/src/{docker.rs (full),helper.rs (full),dispatch.rs (full)}` and `supervisor.rs:430-500`.
  - Core: `crates/gridops-core/src/fleet/registry/commands.rs:20-125,200-290,625-790`, `registry/inventory.rs:321`, `fleet/protocol/outcomes.rs:15-80,195-330`, `fleet/protocol/agent.rs:120-150`, `fleet/domain.rs:195-215`, `fleet/placement.rs:155-195,210-290`, `fleet/targets.rs:440-475` and `provisioning.rs:1-175`.
  - Reconciler: `crates/gridops-reconciler/src/fleet.rs:36-56,446-478`.
  - Manager: `crates/gridops-manager/src/main.rs:320-610,770-810,1485-1600,1700-2000,2645-2730`.
  - Every `HelperResponse::Failed` literal in the workspace: `grep -rn "HelperResponse::Failed" crates/`.
