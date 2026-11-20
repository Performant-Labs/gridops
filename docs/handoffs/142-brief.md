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
   - **The guest runner script**, built like `bitbucket_runner_script` (every interpolated value through `shell_single_quote`): it checks the pinned `gitlab-runner` binary exists in the VM (exit 78 with a readable message if not, as the other scripts do), reads the token from stdin (`IFS= read -r`), and runs `gitlab-runner run-single --url <url> --executor shell --max-builds 1 --wait-timeout <s>` with a builds/working directory under the guest user's home. The token reaches `gitlab-runner` through its environment variable inside the guest (`CI_SERVER_TOKEN`), never argv. The script then **exits with 0, 75 or 77 in the three situations named below** (it does not force any other failure into the table):
     - `77` (registration refused): GitLab refused the token. Detect it reliably, e.g. by verifying the token before `run-single` (`POST <url>/api/v4/runners/verify` with the token sent in the request body read from stdin/a pipe, never argv; 403 → 77), and/or by recognising `run-single`'s forbidden/refused output. Pick a mechanism whose behaviour you can show from `gitlab-runner`'s documented or source behaviour, and say which in the handoff. **Time-of-check gap (review W-1):** a token valid at `verify` time can be revoked before or during `run-single`. If the script detects 77 by `verify` alone, the handoff states this gap explicitly (a token revoked after `verify` exits with `run-single`'s own status, so the listing reports no `exitReason` or a different one); if it also recognises `run-single`'s refused output, that path exits 77 too.
     - `0` (completed): the runner took a job (whatever the job's own result).
     - `75` (idle expired): the wait timeout passed with no job.
     - Any other failure keeps its own non-table status (so the listing reports no `exitReason`). This includes exit 78 for a missing `gitlab-runner` binary.
     The Linux Docker runner script is #147's (in `gridops-core`); this guest script is a separate copy for the macOS VM and may not import it.
   - **Exit-reason lookup**: a function mapping an exit status to `Option<&'static str>` via `GITLAB_EXIT_REASONS`.
   - Make the runner binary path configurable the way `GRIDOPS_TART_BITBUCKET_RUNNER_ROOT` is (e.g. `GRIDOPS_TART_GITLAB_RUNNER_BINARY`, default the path the image script installs to, validated with `valid_guest_path`).
2. **Wiring in `crates/gridops-tart-agent/src/main.rs`** (minimal, delegating to the module):
   - `provision_runner`'s platform `match` gains a `"gitlab"` arm that builds the GitLab script and passes the token as the bootstrap secret to `start_runner`, like the Bitbucket arm.
   - **Exit status capture**: today `start_runner` waits on the `tart exec` child, logs the status and stops the VM, and `list_runners` maps a `stopped` VM to `"exited"` without knowing why. Record the guest script's exit code for GitLab runners (`tart exec` propagates the guest command's exit status) so `list_runners` can emit `exitReason`. It must survive an agent restart while the stopped VM and its record still exist (persist it in the runner's record file, e.g. an optional `#[serde(default)]` field on `TartRecord` so old records still load), and it must be written before the VM is stopped so a listing never sees `exited` without the recorded code when one exists. **Crash window (review W-2):** if the agent process dies after `tart exec` returns but before the record write completes, no code is recorded; that entry later lists as `exited` with no `exitReason`, which is the accepted behaviour for that case (state it in a code comment and the handoff). The invariant above holds whenever the agent stays up through the write. Record which platform a record is (GitLab vs others) so only GitLab entries carry `exitReason`.
   - `list_runners` adds `"exitReason"` only for GitLab records whose mapped state is `"exited"` and whose recorded exit code is in the table.
3. **Base image script** `scripts/prepare-tart-gitlab-runner-image.sh`, modelled on `scripts/prepare-tart-bitbucket-runner-image.sh` (same argument validation style, `tart clone`, boot, wait for `tart exec`, cleanup trap, final summary line): installs a **pinned** `gitlab-runner` release (version and the per-architecture SHA-256 checked in the script; download over HTTPS with the same curl hardening flags; verify the checksum before installing; `chmod +x`; `gitlab-runner --version` printed) into the guest at the default binary path. It checks the guest has `git` (shell executor jobs need it) without installing packages implicitly. It never takes a token. Add it to whatever shell-lint coverage the repo already runs for the other `scripts/prepare-tart-*.sh` (check CI and `scripts/test-*.sh`). **Checked at brief time (review NV-5): there is none.** `.github/workflows/ci.yml` runs `sh scripts/test-build-info.sh` and `sh scripts/test-install.sh` and no `shellcheck` or `bash -n` step; `.github/workflows/tart-installer.yml` runs only `scripts/test-install-tart-agent.sh`; no `scripts/test-*.sh` references any `prepare-tart-*.sh`. So there is no existing coverage to extend, and this issue adds none for the image script; the `bash` + fakes test in Done-when covers the guest runner script.
4. **Docs**: a "GitLab" section in `docs/macos-runners.md` next to "Bitbucket Cloud workspaces": how to prepare the image with the new script, the pinned version, that GitLab macOS runners are ephemeral one-job VMs whose idle timeout is the pool's wait timeout (default 600 s), and that **the shell executor ignores a job's `image:`**, so macOS GitLab jobs run on the base VM's own toolchain. It also says which runner exits carry an `exitReason` (0, 75, 77) and that any other exit, including 78 for an image missing `gitlab-runner`, lists as `exited` with no `exitReason`. Mention the `macos` tag/labels only as far as existing code defines them; do not invent pool UI behaviour (#141/#139 own that).

## Out of scope

- The manager's `("gitlab", "docker")` launch, its validation arm and D15's `ManagedRunner.exit_reason` relay (#147).
- Choosing `CiPlatform::Gitlab`, filling `gitlab`/`gitlabRunnerToken`, runner registration/deletion at GitLab, and mapping `exitReason` to outcomes (#139).
- Starting, stopping or modifying real Tart VMs on the development Mac. Tests must not run `tart`; use the crate's existing fake/stub patterns (e.g. the scripted fake binaries used by the slot-wait tests) or pure functions.

## Done when

- `cargo test -p gridops-tart-agent`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` pass.
- Tart-agent tests cover:
  - **Launch**: a valid GitLab request passes validation and the generated script runs `gitlab-runner run-single` with `--executor shell --max-builds 1 --wait-timeout <s>` and the quoted URL; hostile URL/path values are single-quoted; each invalid request shape (missing/empty/non-`glrt-` token, missing bootstrap, zero timeout, bad URL, non-ephemeral mode, native runtime, and each of an extra `jitConfig`, `bitbucket` or `bitbucketOauthClientSecret`, one case per field) is refused with `BadRequest`, and no message contains the token.
  - **Token delivery**: the script reads the token from stdin and the token never appears in the script text, in the `tart exec` arguments or in an error/log message (assert on the generated script and argv; a test using a fake `tart` that records its argv and stdin is the strongest form).
  - **Exit mapping**: exit statuses 0/75/77 map to `completed`/`idle_expired`/`registration_refused`; another status maps to none; a GitLab record that exited with a recorded code lists `exitReason`; a running GitLab runner, a GitHub/Bitbucket runner, and a GitLab runner with no recorded/unknown code list no `exitReason` key; a record persisted with an exit code reloads with it, and an old record without the field still loads.
  - **Script exit codes (unconditional, review B-1)**: the generated GitLab script is run under `bash` (e.g. `bash -c <script>`, as `start_runner` runs it through `/bin/bash -lc`) with the token written to its stdin, a temp directory of fake executables prepended to `PATH` (a fake `curl` for the verify call, if the script uses one) and the configured runner binary path pointing at a fake `gitlab-runner`. No `tart` and no VM is involved. The fakes are scripted per case, and the test asserts the exact exit status in each:
    - token refused (fake verify answers 403, and/or fake `gitlab-runner` prints the refused output, whichever mechanism the script uses; every mechanism the script implements gets a case) → exactly `77`;
    - fake `gitlab-runner run-single` behaves as an idle wait-timeout with no job → exactly `75`;
    - fake `gitlab-runner run-single` behaves as having run one job (cover both a passing and a failing job) → exactly `0`;
    - runner binary missing → `78`; another fake failure → its own status, not 0/75/77.
    The fakes also record their argv and environment so the test asserts `--executor shell --max-builds 1 --wait-timeout <s>` and the quoted URL reach `gitlab-runner`, the token reaches it only through `CI_SERVER_TOKEN`, and the token is in no fake's argv. This test is required, not optional.
- The pinning test `gitlab_exit_reasons_match_the_core_table` still passes unchanged.
- **Manual check — not run, deferred to #143**: a real `glrt-` token on a real Tart VM (runner online in GitLab, runs a macOS-tagged job, exits). No GitLab test project or token exists yet. The PR body says exactly this; do not fake it.

## Verification notes (brief review round 1)

### Verified on master (review NV-4)

Excerpts from `crates/gridops-tart-agent/src/main.rs` and `scripts/prepare-tart-bitbucket-runner-image.sh` at `a1987ff`, read for this amendment:

- `ProvisionRunner` (`#[serde(rename_all = "camelCase")]`, no `deny_unknown_fields`) ends with:

      #[serde(default)]
      #[allow(dead_code, reason = "the GitLab module of #142 reads it")]
      gitlab: Option<GitlabBootstrap>,
      ...
      #[serde(default)]
      #[allow(dead_code, reason = "the GitLab module of #142 reads it")]
      gitlab_runner_token: Option<String>,

  The token field is a plain `Option<String>` on the wire struct; the GitLab path must not log, format or `Debug`-print it (the struct derives `Debug`), and hands it to `start_runner` as the bootstrap secret the way the Bitbucket arm hands over its secret.
  It has no `registration_token` field, so a stray `registrationToken` key is ignored on deserialization rather than refused.
- `GitlabBootstrap`:

      #[derive(Debug, Deserialize)]
      #[serde(rename_all = "camelCase", deny_unknown_fields)]
      #[allow(dead_code, reason = "the GitLab module of #142 reads it")]
      struct GitlabBootstrap {
          url: String,
          wait_timeout_seconds: u32,
      }

- `GITLAB_EXIT_COMPLETED: i32 = 0`, `GITLAB_EXIT_IDLE_EXPIRED: i32 = 75`, `GITLAB_EXIT_REGISTRATION_REFUSED: i32 = 77` and `const GITLAB_EXIT_REASONS: [(i32, &str); 3]`, each with `#[allow(dead_code, reason = "the GitLab runner listing of #142 reads them")]`. Test `gitlab_exit_reasons_match_the_core_table` asserts `GITLAB_EXIT_REASONS == [(0, "completed"), (75, "idle_expired"), (77, "registration_refused")]`.
- `ProvisionRunner::validate` has, before the `_ => ... "Runner platform is invalid."` arm:

      ("gitlab", _) => {
          return Err(AgentError::BadRequest(
              "GitLab runners are not supported yet: Tart is #142.".into(),
          ));
      }

  and test `gitlab_runners_are_refused_by_name_until_142_lands` asserts that exact string. `validate` also already refuses a native runtime for any platform other than `github` (`"Native macOS runners support GitHub only; Bitbucket runners require a VM."`), before the platform `match`.
- Configuration: `env::var("GRIDOPS_TART_BITBUCKET_RUNNER_ROOT").unwrap_or_else(|_| "/Users/admin/atlassian-bitbucket-pipelines-runner".into())` followed by `anyhow::ensure!(valid_guest_path(&bitbucket_runner_root), "GRIDOPS_TART_BITBUCKET_RUNNER_ROOT is invalid")`. `valid_guest_path` requires a leading `/`, length at most 512, and only ASCII alphanumerics and `/ - _ .`.
- `start_runner(state, record, script, bootstrap_secret)` spawns `tart exec -i <vm_name> /bin/bash -lc <script>` with piped stdin, writes `bootstrap_secret.expose_secret()` plus `\n` to stdin and shuts it down, then in a spawned task does `let status = child.wait().await; tracing::info!(%vm_name, ?status, "macOS runner process exited");` and runs `tart stop <vm_name>`. The exit status is logged only, not stored.
- `bitbucket_runner_script` and `runner_script` both check for their runner binary and `exit 78` with a `printf '...' >&2` message if it is missing, then `IFS= read -r GRIDOPS_BOOTSTRAP_SECRET`.
- `list_runners` maps a VM's Tart state `"stopped"` to `"exited"` (`"running"` → `"running"`, `"suspended"` → `"paused"`, otherwise `"missing"`), and emits no `exitReason` today. `TartRecord` is `#[serde(rename_all = "camelCase")]` with `#[serde(default ...)]` on its later-added fields (`runtime`, `root`) and no platform field.
- The test fake `tart` (`FAKE_TART` in the tests module) answers `exec` with `if [ "$2" = "-i" ]; then cat > /dev/null; fi` then `exit 0`; the exit-capture tests need a fake that can exit with a scripted status.
- `scripts/prepare-tart-bitbucket-runner-image.sh`: `set -euo pipefail`; usage → exit 64; tart missing → exit 69; `GRIDOPS_TART_BITBUCKET_RUNNER_ROOT` absolute single-line check → exit 64; HTTPS-only URL check → exit 64; `tart clone`, `trap cleanup EXIT` (`tart stop`), `tart run --no-graphics ... &`, a 90 × 2 s loop on `tart exec <image> /usr/bin/true`; guest step checks `command -v git`; download flags `curl --fail --location --proto '=https' --tlsv1.2 --retry 5 --retry-all-errors --connect-timeout 15 --speed-limit 1000 --speed-time 30`; final `printf 'Prepared %s with ...'` line. It has no checksum step; the GitLab script adds one.

**Edge case found while verifying (added to scope):** `shell_single_quote` is `format!("'{}'", value.replace('\'', "'\\\"'\\\"'"))`, which replaces an embedded `'` with `'\"'\"'`. Run under `bash`, the quoted form of `a'b` (`'a'\"'\"'b'`) fails with "unexpected EOF while looking for matching `''". So an embedded single quote is **not** safely quoted today. The GitLab URL is attacker-shaped input from the request: either the GitLab validation refuses `'` in the URL, or the helper is fixed (it is in this crate) with a test; say which in the handoff. The hostile-value tests in Done-when include a URL containing `'`, and the `bash` + fakes test runs at least one such quoted value through `bash`.

### Not verified at brief time (review NV-1, NV-2, NV-3)

These are assumptions the design rests on. No source or documentation for them was read for this brief (`tart` 2.41.1's `tart exec --help` says nothing about exit status; `gitlab-runner` is not installed here and its source is not in this repository). The implementer verifies each from upstream source or documentation **before relying on it** and cites the evidence (file and line, or doc URL and quoted sentence) in the handoff; if one is false, stop and report rather than work around it:

- NV-1: `tart exec` exits with the guest command's exit status (needed for 0/75/77 to reach the agent).
- NV-2: `gitlab-runner run-single` accepts `--url`, `--executor shell`, `--max-builds 1` and `--wait-timeout <seconds>`, reads its token from `CI_SERVER_TOKEN`, and distinguishes an idle wait-timeout from a job run and from a refused token in a way the script can detect.
- NV-3: `POST <url>/api/v4/runners/verify` answers 403 for an invalid or revoked `glrt-` token (only if the script uses it).

## Source material read

- `gh issue view 142 --repo Performant-Labs/gridops` (issue body)
- epic #136, latest comment ("Contract deviations shipped by #137", D1–D16)
- docs/handoffs/137-brief.md (§ R2-1 items 1–4, § deviations table, NV-7, NV-8)
- crates/gridops-tart-agent/src/main.rs (`ProvisionRunner`, `GitlabBootstrap`, `GITLAB_EXIT_*`, `validate`, `list_runners`, `provision_runner`, `start_runner`, `bitbucket_runner_script`, `runner_script`, `TartRecord`, GitLab tests)
- scripts/prepare-tart-bitbucket-runner-image.sh
- docs/macos-runners.md (section list, "Bitbucket Cloud workspaces")
- Brief amendment, round 1 (additionally read):
  - crates/gridops-tart-agent/src/main.rs (`TartRecord`, `ProvisionRunner`, `GitlabBootstrap`, `GITLAB_EXIT_*`, `validate`, runner-root configuration, `list_runners`, `start_runner`, `bitbucket_runner_script`, `shell_single_quote`, `runner_script`, `valid_guest_path`, `FAKE_TART`, `install_script`, GitLab tests)
  - scripts/prepare-tart-bitbucket-runner-image.sh (in full)
  - .github/workflows/ci.yml (in full)
  - .github/workflows/tart-installer.yml, .github/workflows/release.yml, .github/workflows/fleet-smoke.yml (searched for shell-lint and `prepare-tart` references; none)
  - scripts/test-build-info.sh, scripts/test-install.sh, scripts/test-install-tart-agent.sh, scripts/test-release-preflight.sh (searched for `prepare-tart` references; none)
  - docs/handoffs/137-brief.md (R2-1 items 2–4, deviations D9–D15)
  - docs/macos-runners.md ("What a release needs from the macOS runner", "Bitbucket Cloud workspaces", "Workflow labels")
  - `tart exec --help` (tart 2.41.1, local)
