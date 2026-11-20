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
