# Handoff-F: Phase 6 - GitLab: run gitlab-runner in Linux Docker runners (host agent)

**Date:** 2026-11-20
**Branch:** claude/gitlab-147-hostagent-e4a01d
**Issue:** #147

## What was done

**Core (`crates/gridops-core/src/provisioning.rs`):** additive items next to the existing `GITLAB_*` constants. No existing contract item changed.
- `GITLAB_RUNNER_IMAGE = "gitlab/gitlab-runner:ubuntu-v19.6.0"` and `GITLAB_DEFAULT_JOB_IMAGE = "alpine:3.24.4"` (B-1).
- `GITLAB_READY_MARKER = "[GRIDOPS GITLAB RUNNER READY]"`.
- `GITLAB_JOB_RECEIVED_SIGNAL`, which is run-single's build-limit line. The idle line's prefix is a private constant.
- `GITLAB_START_READY_TIMEOUT_SECS = 15` and `GITLAB_VERIFY_MAX_TIME_SECS = 10`. Their doc comments cite the 30 s start deadline and the 5 s delivery margin.
- `GITLAB_EXIT_VERIFY_UNAVAILABLE = 69`.
- `valid_gitlab_url`, shared by the manager and the fleet path. It requires an absolute http(s) URL with a host, no credentials, no whitespace and no control characters.
- `gitlab_runner_env(url, wait)`, which gives `GRIDOPS_GITLAB_URL`, `GRIDOPS_GITLAB_WAIT_TIMEOUT` and `GRIDOPS_GITLAB_JOB_IMAGE`.
- `gitlab_runner_script()`. Its text is constant. It is separate from `instrumented_runner_script` (W-8) and safe for bash 3.2.

**Other production files:**
- `crates/gridops-core/src/fleet/protocol/outcomes.rs`: only the text of the `DiagnosticCode::RegistrationRefused` doc comment changes. It now says the code is carried on Start as `Configuration` and on Inspect as `ResourceUnavailable` (R2 W-4, D147-2).
- `crates/gridops-host-agent/src/helper.rs`: `HelperResponse::Failed` gains `diagnostic: Option<DiagnosticReference>` with `#[serde(default, skip_serializing_if = "Option::is_none")]` (B-3). The three existing literals get `diagnostic: None`.
- `crates/gridops-host-agent/src/dispatch.rs`: `outcome()` passes the helper's diagnostic through to `CommandOutcome::Failed`.
- `crates/gridops-host-agent/src/native.rs` and `runtime.rs`: mechanical `diagnostic: None` only.
- `crates/gridops-host-agent/src/docker.rs`:
  - Prepare refuses a GitLab target without `requires_docker_socket` with `Configuration`, before any pull (W-5).
  - `start_gitlab` is the GitLab branch of Start (W-4, B-2).
  - `start_gitlab_runner` feeds the token and waits for readiness.
  - On Inspect, an exited GitLab container carries `IdleExpired` or `RegistrationRefused` (`gitlab_exit_diagnostic`).
  - The backend advertises `gitlab_project` and `gitlab_group` in ephemeral mode (D6).
  - The shared create flags moved into `create_args`.
- `crates/gridops-manager/src/main.rs`:
  - The GitLab validation arm (D10).
  - GitLab is branched on platform in `runner_command` and `take_bootstrap_secret` (W-6).
  - A GitLab start without a runner socket gets 400.
  - The `io.gridops.platform=gitlab` label.
  - `exitReason` in the listing, from `inspect_container` and `GITLAB_EXIT_REASONS`.
  - `ManagedRunner.exit_reason` (D15).
  - `RUNNER_ALLOW_RUNASROOT` is set for GitHub only.
- `docs/fleet-agent.md`: a "GitLab runners" section covering:
  - the socket requirements on both paths, naming `GRIDOPS_AGENT_DOCKER_SOCKET` with the grant and profile, and `GRIDOPS_RUNNER_DOCKER_SOCKET`;
  - the token handling;
  - the outcomes;
  - disabled cache volumes, and job containers that can be orphaned mid-job (#143);
  - a line that `compose.no-docker-socket.yaml` disables GitLab runners too (W-7, R2 W-5).
- `compose.no-docker-socket.yaml`: the header comment now says GitLab runners are off too and need `GRIDOPS_RUNNER_DOCKER_SOCKET`.
- Shared notes for #139 and #142, outside the repo: `/Users/andreangelantoni/ClaudeAangel100/side-session-notes/local_e1c0b548-f83a-4d0b-8ba3-069dc9dfc38b/147-hostagent-notes.md` (an appended "Implemented" section).

## Design decisions

- **How the token reaches `gitlab-runner` (NV-2).** The script sets `CI_SERVER_TOKEN="$S" gitlab-runner run-single ...`, so the variable is in that child's environment only.
  - `S` is never exported. Bash's own environment and the container's `Config.Env` never hold the token. With `--init`, PID 1 is Docker's init, whose environment is the container env, which has no token either.
  - I rejected `--token` in argv. A process's argv is readable through `/proc/<pid>/cmdline` by every user, including from the host's `ps`. A process's environment is readable only by the same uid or root.
  - I rejected a config file. It would put the token on the container's writable layer, on the host's disk.
- **How the token reaches verify (NV-4).** `printf '%s' "$S" | curl --data-urlencode 'token@-' ...` sends it. It is URL-encoded, so a token with `+`, `&`, `=`, `%` or `/` arrives intact; a headless check against a local server confirmed this. It never appears in curl's argv or environment.
  - The verify call does **not** send `system_id`, even though GitLab's API docs say it is required for `glrt-` tokens. GitLab's source makes it optional: `lib/api/ci/helpers/runner.rb:78-94` falls back to the legacy manager, which "doesn't count toward the limit".
  - A made-up `system_id` would create a manager that counts toward the per-runner creation limit, and could take the slot the runner's own manager needs.
- **Completed vs idle (NV-3).**
  - Both endings exit 0. The script tees run-single's output (stdout and stderr merged, so the fleet's stdout-only live log sees it) to a log through a FIFO, and searches it with `grep -aqF` for:
    - `GITLAB_JOB_RECEIVED_SIGNAL`, giving exit 0;
    - else the idle prefix, giving exit 75;
    - else exit 1.
  - The search matches a substring, not a whole line. `RunnerTextFormatter` pads the message to 50 columns and appends an ANSI reset (`log/runner_formatter.go:35`), so the real line is never exactly the message.
  - A non-zero runner status is passed on. If it happens to be 69, 75 or 77, the script exits 1 instead, so a runner failure can never be read as one of the reported endings.
- **Stop handling.** The runner runs in the background with `trap 'kill -TERM "$runner_pid"' TERM INT` and a `wait` loop. A `docker stop` therefore reaches gitlab-runner, and the script waits for it to finish. I checked this headlessly with a stand-in runner that traps TERM.
- **Fleet readiness wait.**
  - `start_gitlab_runner` keeps `docker start --interactive` attached until it sees the marker in `docker logs` or the status `exited`/`dead`, polling every 200 ms for at most `GITLAB_START_READY_TIMEOUT_SECS` from before the attach.
  - It checks the marker before the exit status. A runner that printed the marker and then ended counts as started, because a job may have run; Inspect then reports how it ended.
  - Every other outcome reads the exit status first, then runs `rm -f`, then reports (A's warning 1). Exit 77 gives `Configuration, false, Some(RegistrationRefused)`. Anything else gives `Runtime, true, None`.
  - A failed stdin write is ignored on purpose: the exit status decides the outcome.
- **The `create_args` extraction in `docker.rs`.** The GitLab container needs the same ~40 lines of create flags: limits, caps, `--init`, the empty entrypoint and the network. Copying them would create a parallel path. I moved them into one method that both branches call, in the same order. GitHub's flags are byte-for-byte the same: `docker_runtime`'s `a_runner_container_is_created_with_the_flags_its_image_needs` passes.
- **Manager image and label.** GitLab runs `/bin/bash -c`, not GitHub's `-lc`, which avoids the login-shell `~/.bash_logout` issue noted in `instrumented_runner_script`. Only GitLab containers get the platform label (W-6). GitHub containers are unchanged.
- **Socket refusal on the manager.** The check is in `provision_runner_inner`, not `validate()`, because it needs the manager's state. It runs inside `start_runner`, so the capacity lease is released as for any failed start.

## Reuse / extend-vs-new

- **Extended:**
  - `HelperResponse::Failed` (one new field) and `dispatch::outcome`;
  - `DockerRuntime::prepare`, `start`, `call` (Inspect) and `docker_backend`;
  - the manager's `validate`, `runner_command`, `take_bootstrap_secret`, `provision_runner_inner`, `list_runners` and `ManagedRunner`.
- **New** (each named in the brief):
  - the GitLab script, a separate function per W-8;
  - its env builder and URL check, which live in core per A's warning 2;
  - `start_gitlab`, the brief's "GitLab branch of `docker.rs`";
  - the `start_gitlab_runner` readiness wait. `feed_secret` detaches after 0.5 s and can't see the marker or tell 77 from other exits, so the brief (B-2) asks for a different wait. `feed_secret` stays unchanged for GitHub.
- No private copy of `GitlabRunnerMaterial` (W-4): the GitLab branch parses the core type.

## Architecture notes for A

- **Layers:** core (provisioning constants and functions), host agent (helper protocol, dispatch, Docker runtime), manager. No dependency direction changed. Core gains no dependency (it already depends on `url`).
- **Changed interfaces:**
  - `HelperResponse::Failed` is a public type of the `gridops-host-agent` crate. Its new optional field is skipped when `None`, so the wire form of every existing failure is unchanged (T's test pins this).
  - `ManagedRunner` gains optional `exitReason` (D15).
  - Core gains new public items. No shared contract type changed.
- **Behaviour change outside GitLab:** the manager no longer sets `RUNNER_ALLOW_RUNASROOT` on GitLab containers. GitHub still gets it.

## Deviations from spec / wireframe

- **`system_id` left out of verify.** GitLab's docs say it is required for `glrt-`; GitLab's source makes it optional. The brief's NV-4 spec (token only) is kept, and the reasoning is above. This is not a contract change. If GitLab enforces the docs later, the effect is a retryable verify-unavailable, not a silent failure.
- Otherwise none. No wireframe (no UI).

## Tier 1 self-check (incl. tests now GREEN)

    cargo test -p gridops-core --test gitlab_runner_script
      test result: ok. 11 passed; 0 failed
    cargo test -p gridops-host-agent           (all targets)
      lib 8/8, dispatch 8/8, docker_gitlab 10/10 (15.2 s), docker_runtime 2/2, version 1/1
    TMPDIR=/tmp cargo test -p gridops-manager
      test result: ok. 111 passed; 0 failed      (incl. gitlab_runner_tests 3/3 and the 4 new unit tests)
    TMPDIR=/tmp cargo test -p gridops-core -p gridops-host-agent -p gridops-manager -p gridops-tart-agent -p gridops-reconciler
      29 test binaries, 610 passed, 0 failed
    TMPDIR=/tmp cargo test -p gridops-api --test fleet_http
      143 passed; 1 failed: release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine
      (environmental, see Known issues)
    cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl   (CI's command)
      Finished, no diagnostics
    cargo fmt --all -- --check
      clean

`TMPDIR=/tmp` is the workstation's `SUN_LEN` limit that T recorded; CI is unaffected.

### External facts confirmed (NV-1 to NV-6, brief review W-3)

Sources were fetched headlessly into the session scratchpad.

- **NV-1.** gitlab-runner v19.6.0 has every flag the script passes:
  - `commands/single.go:24` `--wait-timeout` and `:27` `--max-builds`;
  - `common/config.go:1462` `--url`, `:1505` `--executor`, `:293` `--image` and `:324` `--disable-cache`, both under `namespace:"docker"` (`:1556`), which makes them `--docker-image` and `--docker-disable-cache`;
  - `RunnerCredentials` and `RunnerSettings` are embedded in `RunnerConfig` (`:1592-1593`), which `RunSingleCommand` embeds (`single.go:21`).
- **NV-2.** `common/config.go:1466` defines `Token ... long:"token" env:"CI_SERVER_TOKEN"`. `HandleArgs` refuses to run without it (`single.go:193-195`).
- **NV-3.** `single.go:167-176` (`checkFinishedConditions`) logs `"This runner has processed its build limit, so now exiting"` or `"This runner has not received a job in", N, "seconds, so now exiting"`. `postBuild` (`:77-82`) runs only after a build. Formatter padding is at `log/runner_formatter.go:35`.
- **NV-4.** GitLab `lib/api/ci/runner.rb:133-149` is `POST /runners/verify`: `requires :token`, `authenticate_runner!`, `forbidden!` unless the runner exists, and 200 otherwise. It also documents 422 "Runner is orphaned", which this design treats as verify-unavailable. `doc/api/runners.md:1093-1117` lists the 200 and 403 statuses and a form `token` field.
- **NV-6.** The `gitlab/gitlab-runner:ubuntu-v19.6.0` amd64 manifest has a config with `LABEL org.opencontainers.image.version=24.04` (Ubuntu 24.04). The base layer `sha256:26538842…` contains `usr/bin/bash`, and layer `sha256:668e014c…` contains `usr/bin/curl` and `usr/bin/dumb-init`. The entrypoint is `/usr/bin/dumb-init /entrypoint`, which the empty `--entrypoint` overrides.
- **Pinned tags (W-3).** Each was checked with `curl https://hub.docker.com/v2/repositories/<repo>/tags/<tag>`:
  - `gitlab/gitlab-runner` `ubuntu-v19.6.0` is `active`, updated 2026-11-19, with amd64 and arm64 among its platforms. Its digests are identical to `v19.6.0`.
  - `library/alpine` `3.24.4` is the newest patch, updated 2026-11-08.

## Evidence appendix

`docs/handoffs/147/evidence.md` has 8 entries:
- the socket flag's derivation in dispatch;
- the existing Prepare `Permission` refusal;
- `reported_gone` matching `ResourceUnavailable`;
- the 30 s start deadline and 5 s delivery margin;
- `CiTarget::is_gitlab`;
- the manager's conditional socket bind.

## Tests that look wrong (for T)

None.

## Known issues

- `fleet_http` `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on this macOS workstation only. The fixture's "Linux machine" runs the real native backend, which takes its OS from `cfg!(target_os)` (`crates/gridops-host-agent/src/native.rs:201`). On a Mac it reports macOS ARM64, so the preflight passes its `macos-machine` check. #147 touches neither the preflight script nor the native probe. On CI (Linux) it reports Linux.
- The manual check with a real `glrt-` token was **not run**. It is deferred to #143, as the brief requires, and the PR must say so.
- A `wait_timeout_seconds` of 0 makes `run-single` wait forever. Neither path rejects it: the contract allows it, and the reconciler (#139) sends the default of 600. This is recorded in the #139 notes.
- Job containers orphaned when a runner is stopped mid-job are a known gap for #143 (R2 W-5), documented in `docs/fleet-agent.md`.
- The PR description items are O's (Phase 11/12):
  - D147-1 and D147-2;
  - the env-var choice (brief review W-1);
  - the pinned image values with their registry checks, listed above;
  - the `system_id` note;
  - the deferred manual check.

## Files changed

- `crates/gridops-core/src/provisioning.rs`
- `crates/gridops-core/src/fleet/protocol/outcomes.rs`
- `crates/gridops-host-agent/src/helper.rs`
- `crates/gridops-host-agent/src/dispatch.rs`
- `crates/gridops-host-agent/src/docker.rs`
- `crates/gridops-host-agent/src/native.rs`
- `crates/gridops-host-agent/src/runtime.rs`
- `crates/gridops-manager/src/main.rs`
- `docs/fleet-agent.md`
- `compose.no-docker-socket.yaml`
