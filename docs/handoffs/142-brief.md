# [GitLab: macOS runners on Tart pools (Tart agent side)](https://github.com/Performant-Labs/gridops/issues/142)

Part of #136. **Wave 1.** Depends only on #137 (merged as `b22c451`). Review rigor: `second-opinion`.

This brief restates the issue with the contract deviations shipped by #137 (the "Contract deviations shipped by #137" table on epic #136, and `docs/handoffs/137-brief.md` § "R2-1 (block)"). **Where the issue text and the deviation table differ, the table wins.** The decisive one is **D10**: the Tart agent does **not** receive a GitLab `PrepareEnvironment`. It receives a GitLab `POST /v1/runners` request (forwarded by `gridops-manager`, whose side is #147), and it reports `exitReason` in `GET /v1/runners`.

## Need

GitLab jobs that need macOS run on Tart pools, as Bitbucket's macOS runners do: one ephemeral Tart VM per runner, running `gitlab-runner run-single` with the shell executor for at most one job.

## The contract this issue implements (already on master; do not change it)

From #137's brief, R2-1 items 2 and 3, and deviations D9, D10, D12, D15:

- `crates/gridops-tart-agent/src/main.rs` already has, on `ProvisionRunner` (camelCase, no `deny_unknown_fields`):
  - `gitlab: Option<GitlabBootstrap>`, wire `{"url": "<base_url>", "waitTimeoutSeconds": <u32>}`; `GitlabBootstrap` is `#[serde(rename_all = "camelCase", deny_unknown_fields)] { url: String, wait_timeout_seconds: u32 }`.
  - `gitlab_runner_token: Option<SecretString>`, wire `gitlabRunnerToken`, the secret `glrt-` token.
  - Private copies of `GITLAB_EXIT_COMPLETED = 0`, `GITLAB_EXIT_IDLE_EXPIRED = 75`, `GITLAB_EXIT_REGISTRATION_REFUSED = 77` and `GITLAB_EXIT_REASONS = [(0,"completed"),(75,"idle_expired"),(77,"registration_refused")]`, with the pinning test `gitlab_exit_reasons_match_the_core_table`. Keep the names, values and that test.
  - A placeholder `("gitlab", _)` arm in `ProvisionRunner::validate` returning `AgentError::BadRequest("GitLab runners are not supported yet: Tart is #142.")`, and its test `gitlab_runners_are_refused_by_name_until_142_lands`. **This issue replaces both** (the placeholder goes away; the test is replaced by tests of the real validation).
  - `#[allow(dead_code, reason = "... #142 ...")]` attributes on those items. Remove them once the code reads the items.
- A GitLab request has `"platform":"gitlab"`, `"mode":"ephemeral"`, `"runtime":"vm"`, `"provider":"tart"`, both `gitlab` and `gitlabRunnerToken`, and no `jitConfig`, `registrationToken`, `bitbucket` or `bitbucketOauthClientSecret` (D9: GitLab is always ephemeral with the vm runtime).
- `waitTimeoutSeconds` is the runner's idle timeout (D12). The reconciler sends `GITLAB_DEFAULT_WAIT_TIMEOUT_SECS = 600` when a pool sets none.
- `gitlabRunnerToken` is a secret, handled exactly like `jitConfig` and `bitbucketOauthClientSecret` today: never logged, never in an error message, never on a command line (the host's `tart exec` argv or the guest's argv) and never in the host process environment; delivered to the guest through the agent's existing secret channel (`start_runner` writes the bootstrap secret to `tart exec -i`'s stdin).
- `GET /v1/runners` entries gain an optional `exitReason`, **present only for a GitLab runner whose `state` is `"exited"`**, with one of `"completed"`, `"idle_expired"`, `"registration_refused"`. Its value is the runner script's exit status looked up in the agent's `GITLAB_EXIT_REASONS`. An exit status not in the table yields no `exitReason` (keep it absent). Non-GitLab entries never carry the key. The manager relays the field unchanged (D15, #147's side).

## Scope

All in files this issue owns. **Do not edit `crates/gridops-manager`, `crates/gridops-host-agent`, `crates/gridops-core` or any shared contract type** (six other wave-1 issues run in parallel). If the contract truly needs a change, stop and report it.

1. **GitLab module of the Tart agent** — a new module file `crates/gridops-tart-agent/src/gitlab.rs` (declared from `main.rs`), holding the GitLab-specific code:
   - **Validation** (called from `ProvisionRunner::validate`'s `("gitlab", _)` arm, which replaces the placeholder): require `mode == "ephemeral"` and the vm runtime (a native GitLab request is refused, as native Bitbucket is); require `gitlab` present with an `http`/`https` URL (single line, no whitespace or control characters) and `wait_timeout_seconds > 0`; require `gitlabRunnerToken` present, non-empty, starting with `glrt-`, single line; refuse a request that also carries `jitConfig`, `bitbucket` or `bitbucketOauthClientSecret`. Each refusal is `AgentError::BadRequest` with a readable message that never contains the token.
   - **The guest runner script**, built like `bitbucket_runner_script` (every interpolated value through `shell_single_quote`): it checks the pinned `gitlab-runner` binary exists in the VM (exit 78 with a readable message if not, as the other scripts do), reads the token from stdin (`IFS= read -r`), and runs `gitlab-runner run-single --url <url> --executor shell --max-builds 1 --wait-timeout <s>` with a builds/working directory under the guest user's home. The token reaches `gitlab-runner` through its environment variable inside the guest (`CI_SERVER_TOKEN`), never argv. The script then **exits with exactly 0, 75 or 77**:
     - `77` (registration refused): GitLab refused the token. Detect it reliably, e.g. by verifying the token before `run-single` (`POST <url>/api/v4/runners/verify` with the token sent in the request body read from stdin/a pipe, never argv; 403 → 77), and/or by recognising `run-single`'s forbidden/refused output. Pick a mechanism whose behaviour you can show from `gitlab-runner`'s documented or source behaviour, and say which in the handoff.
     - `0` (completed): the runner took a job (whatever the job's own result).
     - `75` (idle expired): the wait timeout passed with no job.
     - Any other failure keeps its own non-table status (so the listing reports no `exitReason`).
     The Linux Docker runner script is #147's (in `gridops-core`); this guest script is a separate copy for the macOS VM and may not import it.
   - **Exit-reason lookup**: a function mapping an exit status to `Option<&'static str>` via `GITLAB_EXIT_REASONS`.
   - Make the runner binary path configurable the way `GRIDOPS_TART_BITBUCKET_RUNNER_ROOT` is (e.g. `GRIDOPS_TART_GITLAB_RUNNER_BINARY`, default the path the image script installs to, validated with `valid_guest_path`).
2. **Wiring in `crates/gridops-tart-agent/src/main.rs`** (minimal, delegating to the module):
   - `provision_runner`'s platform `match` gains a `"gitlab"` arm that builds the GitLab script and passes the token as the bootstrap secret to `start_runner`, like the Bitbucket arm.
   - **Exit status capture**: today `start_runner` waits on the `tart exec` child, logs the status and stops the VM, and `list_runners` maps a `stopped` VM to `"exited"` without knowing why. Record the guest script's exit code for GitLab runners (`tart exec` propagates the guest command's exit status) so `list_runners` can emit `exitReason`. It must survive an agent restart while the stopped VM and its record still exist (persist it in the runner's record file, e.g. an optional `#[serde(default)]` field on `TartRecord` so old records still load), and it must be written before the VM is stopped so a listing never sees `exited` without the recorded code when one exists. Record which platform a record is (GitLab vs others) so only GitLab entries carry `exitReason`.
   - `list_runners` adds `"exitReason"` only for GitLab records whose mapped state is `"exited"` and whose recorded exit code is in the table.
3. **Base image script** `scripts/prepare-tart-gitlab-runner-image.sh`, modelled on `scripts/prepare-tart-bitbucket-runner-image.sh` (same argument validation style, `tart clone`, boot, wait for `tart exec`, cleanup trap, final summary line): installs a **pinned** `gitlab-runner` release (version and the per-architecture SHA-256 checked in the script; download over HTTPS with the same curl hardening flags; verify the checksum before installing; `chmod +x`; `gitlab-runner --version` printed) into the guest at the default binary path. It checks the guest has `git` (shell executor jobs need it) without installing packages implicitly. It never takes a token. Add it to whatever shell-lint coverage the repo already runs for the other `scripts/prepare-tart-*.sh` (check CI and `scripts/test-*.sh`).
4. **Docs**: a "GitLab" section in `docs/macos-runners.md` next to "Bitbucket Cloud workspaces": how to prepare the image with the new script, the pinned version, that GitLab macOS runners are ephemeral one-job VMs whose idle timeout is the pool's wait timeout (default 600 s), and that **the shell executor ignores a job's `image:`**, so macOS GitLab jobs run on the base VM's own toolchain. Mention the `macos` tag/labels only as far as existing code defines them; do not invent pool UI behaviour (#141/#139 own that).

## Out of scope

- The manager's `("gitlab", "docker")` launch, its validation arm and D15's `ManagedRunner.exit_reason` relay (#147).
- Choosing `CiPlatform::Gitlab`, filling `gitlab`/`gitlabRunnerToken`, runner registration/deletion at GitLab, and mapping `exitReason` to outcomes (#139).
- Starting, stopping or modifying real Tart VMs on the development Mac. Tests must not run `tart`; use the crate's existing fake/stub patterns (e.g. the scripted fake binaries used by the slot-wait tests) or pure functions.

## Done when

- `cargo test -p gridops-tart-agent`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` pass.
- Tart-agent tests cover:
  - **Launch**: a valid GitLab request passes validation and the generated script runs `gitlab-runner run-single` with `--executor shell --max-builds 1 --wait-timeout <s>` and the quoted URL; hostile URL/path values are single-quoted; each invalid request shape (missing/empty/non-`glrt-` token, missing bootstrap, zero timeout, bad URL, non-ephemeral mode, native runtime, extra GitHub/Bitbucket secrets) is refused with `BadRequest`, and no message contains the token.
  - **Token delivery**: the script reads the token from stdin and the token never appears in the script text, in the `tart exec` arguments or in an error/log message (assert on the generated script and argv; a test using a fake `tart` that records its argv and stdin is the strongest form).
  - **Exit mapping**: exit statuses 0/75/77 map to `completed`/`idle_expired`/`registration_refused`; another status maps to none; a GitLab record that exited with a recorded code lists `exitReason`; a running GitLab runner, a GitHub/Bitbucket runner, and a GitLab runner with no recorded/unknown code list no `exitReason` key; a record persisted with an exit code reloads with it, and an old record without the field still loads.
  - The script's own exit-code behaviour (77 on a refused token, 75 on idle, 0 after a job), exercised by running the generated script under `bash` against a fake `gitlab-runner`/`curl` on `PATH` if feasible without a VM.
- The pinning test `gitlab_exit_reasons_match_the_core_table` still passes unchanged.
- **Manual check — not run, deferred to #143**: a real `glrt-` token on a real Tart VM (runner online in GitLab, runs a macOS-tagged job, exits). No GitLab test project or token exists yet. The PR body says exactly this; do not fake it.

## Source material read

- `gh issue view 142 --repo Performant-Labs/gridops` (issue body)
- epic #136, latest comment ("Contract deviations shipped by #137", D1–D16)
- docs/handoffs/137-brief.md (§ R2-1 items 1–4, § deviations table, NV-7, NV-8)
- crates/gridops-tart-agent/src/main.rs (`ProvisionRunner`, `GitlabBootstrap`, `GITLAB_EXIT_*`, `validate`, `list_runners`, `provision_runner`, `start_runner`, `bitbucket_runner_script`, `runner_script`, `TartRecord`, GitLab tests)
- scripts/prepare-tart-bitbucket-runner-image.sh
- docs/macos-runners.md (section list, "Bitbucket Cloud workspaces")
