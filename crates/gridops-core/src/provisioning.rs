//! Shared provisioning decisions and lifecycle obligations.
//!
//! The API and reconciler own their I/O adapters, credentials, and persistence
//! details. This module owns the decisions that must not drift between them.

use crate::fleet::CiPlatform;

/// The `--wait-timeout` a GitLab runner gets when its pool sets none (#139).
pub const GITLAB_DEFAULT_WAIT_TIMEOUT_SECS: u32 = 600;

/// The prefix of a GitLab runner authentication token, a secret.
pub const GITLAB_RUNNER_TOKEN_PREFIX: &str = "glrt-";

/// The GitLab runner script ran its one job.
pub const GITLAB_EXIT_COMPLETED: i32 = 0;
/// The GitLab runner script hit `--wait-timeout` without a job.
pub const GITLAB_EXIT_IDLE_EXPIRED: i32 = 75;
/// GitLab rejected the runner's token.
pub const GITLAB_EXIT_REGISTRATION_REFUSED: i32 = 77;
/// The runner's image has no `gitlab-runner` (`EX_CONFIG` from `sysexits.h`,
/// #173). Only the Tart agent's GitLab guest script exits with it, when the
/// Tart image has no executable at `GRIDOPS_TART_GITLAB_RUNNER_BINARY`; the
/// pool's image must be prepared again. [`gitlab_runner_script`] never
/// exits with it: a `run-single` that itself exits 78 ends that script with
/// a status outside the table.
pub const GITLAB_EXIT_RUNNER_MISSING: i32 = 78;

/// A GitLab runner script's exit status and the `exitReason` the runner
/// listing (`GET /v1/runners`) reports for it. `gridops-tart-agent` holds a
/// copy with the same names and values.
pub const GITLAB_EXIT_REASONS: [(i32, &str); 4] = [
    (GITLAB_EXIT_COMPLETED, "completed"),
    (GITLAB_EXIT_IDLE_EXPIRED, "idle_expired"),
    (GITLAB_EXIT_REGISTRATION_REFUSED, "registration_refused"),
    (GITLAB_EXIT_RUNNER_MISSING, "runner_missing"),
];

/// The official GitLab Runner image, Ubuntu-based (the script needs `bash`
/// and `curl`), pinned to a release. A GitLab pool's `docker_image` names the
/// runner image; the server, not a runtime, falls back to this one when a
/// pool sets none (D147-1).
pub const GITLAB_RUNNER_IMAGE: &str = "gitlab/gitlab-runner:ubuntu-v19.6.0";

/// `run-single --docker-image`: the image a job runs in when its
/// `.gitlab-ci.yml` names none.
pub const GITLAB_DEFAULT_JOB_IMAGE: &str = "alpine:3.24.4";

/// The line the GitLab runner script prints on stdout once GitLab has
/// accepted the token, just before it starts `gitlab-runner`. A fleet Start
/// waits for it.
pub const GITLAB_READY_MARKER: &str = "[GRIDOPS GITLAB RUNNER READY]";

/// What `gitlab-runner run-single` logs when it exits after running its
/// `--max-builds 1` job (`commands/single.go`, `checkFinishedConditions`).
/// It exits 0 after a job and after `--wait-timeout` alike, so this line is
/// how the script tells a completed runner from an idle one.
pub const GITLAB_JOB_RECEIVED_SIGNAL: &str =
    "This runner has processed its build limit, so now exiting";

/// The start of what `run-single` logs when `--wait-timeout` passes without
/// a job (`commands/single.go`, `checkFinishedConditions`).
const GITLAB_IDLE_SIGNAL: &str = "This runner has not received a job in";

/// How long a fleet Start waits for [`GITLAB_READY_MARKER`] or the runner's
/// exit. The reconciler gives a start command `START_COMMAND_DEADLINE_MILLIS`
/// (30 s, `gridops-reconciler/src/fleet.rs`), less the command registry's
/// `DELIVERY_MARGIN_MILLIS` (5 s, `fleet/registry/commands.rs`), so creating
/// the container, feeding the token and this wait must all finish within
/// 25 s.
pub const GITLAB_START_READY_TIMEOUT_SECS: u64 = 15;

/// The verify call's `curl --max-time`. Shorter than
/// [`GITLAB_START_READY_TIMEOUT_SECS`], so a slow GitLab ends the Start with
/// [`GITLAB_EXIT_VERIFY_UNAVAILABLE`] inside the same 30 s − 5 s budget
/// rather than with the agent's own timeout.
pub const GITLAB_VERIFY_MAX_TIME_SECS: u32 = 10;

/// GitLab could not be asked whether it accepts the token (no answer in
/// time, no connection, or neither 200 nor 403). A retryable failure, so not
/// in [`GITLAB_EXIT_REASONS`] (`EX_UNAVAILABLE` from `sysexits.h`, beside
/// 75's `EX_TEMPFAIL` and 77's `EX_NOPERM`).
pub const GITLAB_EXIT_VERIFY_UNAVAILABLE: i32 = 69;

/// Whether `url` can be a GitLab runner's `--url`: an absolute http(s) URL
/// with a host, no credentials and no whitespace or control characters. The
/// manager and the fleet host agent both check a GitLab URL with this.
#[must_use]
pub fn valid_gitlab_url(url: &str) -> bool {
    url.len() <= 2_048
        && !url
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        && url::Url::parse(url).is_ok_and(|parsed| {
            matches!(parsed.scheme(), "http" | "https")
                && parsed.host_str().is_some_and(|host| !host.is_empty())
                && parsed.username().is_empty()
                && parsed.password().is_none()
        })
}

/// The GitLab runner script's non-secret inputs, as the `NAME=value`
/// environment of its container. The token is never one of them: the
/// script reads it from stdin.
#[must_use]
pub fn gitlab_runner_env(url: &str, wait_timeout_seconds: u32) -> Vec<String> {
    vec![
        format!("GRIDOPS_GITLAB_URL={url}"),
        format!("GRIDOPS_GITLAB_WAIT_TIMEOUT={wait_timeout_seconds}"),
        format!("GRIDOPS_GITLAB_JOB_IMAGE={GITLAB_DEFAULT_JOB_IMAGE}"),
    ]
}

/// The script a GitLab runner container runs with `bash -c`, on the fleet
/// host agent and the runner manager alike. Its text is constant: its inputs
/// arrive as the environment [`gitlab_runner_env`] builds, and the `glrt-`
/// token as the one line on its stdin.
///
/// The token lives only in the non-exported shell variable `S`. The verify
/// call reads it from a pipe fed by the `printf` builtin, so no process has
/// it in its arguments, and `gitlab-runner` alone gets it, as
/// `CI_SERVER_TOKEN` in its own environment (`common/config.go`,
/// `RunnerCredentials.Token`), never in the container's metadata or the
/// environment of the container's main process.
///
/// Exit status: [`GITLAB_EXIT_REGISTRATION_REFUSED`] when GitLab answers 403
/// to `POST /api/v4/runners/verify`, [`GITLAB_EXIT_VERIFY_UNAVAILABLE`] when
/// it can't be asked, [`GITLAB_EXIT_COMPLETED`] after the one job ran,
/// [`GITLAB_EXIT_IDLE_EXPIRED`] after `--wait-timeout` passed without a
/// job, and another non-zero status for any other failure. A `run-single`
/// that itself exits with a status in [`GITLAB_EXIT_REASONS`] or with
/// [`GITLAB_EXIT_VERIFY_UNAVAILABLE`] ends the script with 1, so a status in
/// the table is always the script's own verdict.
#[must_use]
pub fn gitlab_runner_script() -> String {
    GITLAB_RUNNER_SCRIPT
        .replace("@READY_MARKER@", GITLAB_READY_MARKER)
        .replace("@JOB_SIGNAL@", GITLAB_JOB_RECEIVED_SIGNAL)
        .replace("@IDLE_SIGNAL@", GITLAB_IDLE_SIGNAL)
        .replace(
            "@VERIFY_MAX_TIME@",
            &GITLAB_VERIFY_MAX_TIME_SECS.to_string(),
        )
        .replace("@EXIT_COMPLETED@", &GITLAB_EXIT_COMPLETED.to_string())
        .replace("@EXIT_IDLE@", &GITLAB_EXIT_IDLE_EXPIRED.to_string())
        .replace(
            "@EXIT_REFUSED@",
            &GITLAB_EXIT_REGISTRATION_REFUSED.to_string(),
        )
        .replace(
            "@EXIT_UNAVAILABLE@",
            &GITLAB_EXIT_VERIFY_UNAVAILABLE.to_string(),
        )
        .replace(
            "@EXIT_RUNNER_MISSING@",
            &GITLAB_EXIT_RUNNER_MISSING.to_string(),
        )
}

/// [`gitlab_runner_script`] before its constants are filled in. Written for
/// bash 3.2 and up, with only POSIX tools beside `curl` and `gitlab-runner`.
const GITLAB_RUNNER_SCRIPT: &str = r#"set -u
IFS= read -r S || true
if [ -z "${S:-}" ]; then
  echo 'GridOps: no GitLab runner token arrived on stdin.' >&2
  exit 1
fi
url=${GRIDOPS_GITLAB_URL%/}

# Ask GitLab whether it accepts the token. It goes to curl on a pipe from the
# printf builtin, never as an argument.
answer=$(printf '%s' "$S" | curl --silent --show-error --output /dev/null   --write-out '%{http_code}' --max-time @VERIFY_MAX_TIME@   --data-urlencode 'token@-' "$url/api/v4/runners/verify") || true
case "$answer" in
  200) ;;
  403)
    echo 'GridOps: GitLab refused the runner token (HTTP 403).' >&2
    exit @EXIT_REFUSED@ ;;
  *)
    echo "GridOps: could not ask GitLab to verify the runner token (HTTP ${answer:-none})." >&2
    exit @EXIT_UNAVAILABLE@ ;;
esac
printf '%s
' '@READY_MARKER@'

# gitlab-runner's output reaches the container's stdout through tee, and a
# copy is kept to tell how run-single ended: it exits 0 both after its job
# and after --wait-timeout.
work=$(mktemp -d "${TMPDIR:-/tmp}/gridops-gitlab.XXXXXX") || exit 1
mkfifo "$work/output" || exit 1
tee "$work/runner.log" < "$work/output" &
tee_pid=$!
CI_SERVER_TOKEN="$S" gitlab-runner run-single   --url "$url"   --executor docker   --docker-image "$GRIDOPS_GITLAB_JOB_IMAGE"   --docker-disable-cache   --max-builds 1   --wait-timeout "$GRIDOPS_GITLAB_WAIT_TIMEOUT"   > "$work/output" 2>&1 &
runner_pid=$!
unset S
# A stop reaches this shell (Docker's init passes it on): hand it to the runner
# and wait for it to finish.
trap 'kill -TERM "$runner_pid" 2>/dev/null' TERM INT
wait "$runner_pid"
runner_status=$?
while kill -0 "$runner_pid" 2>/dev/null; do
  wait "$runner_pid"
  runner_status=$?
done
trap - TERM INT
wait "$tee_pid" 2>/dev/null

finish() {
  rm -rf "$work"
  exit "$1"
}
if [ "$runner_status" -ne 0 ]; then
  echo "GridOps: gitlab-runner exited with status $runner_status." >&2
  case "$runner_status" in
    @EXIT_IDLE@|@EXIT_REFUSED@|@EXIT_UNAVAILABLE@|@EXIT_RUNNER_MISSING@) finish 1 ;;
  esac
  finish "$runner_status"
fi
if grep -aqF '@JOB_SIGNAL@' "$work/runner.log"; then
  finish @EXIT_COMPLETED@
fi
if grep -aqF '@IDLE_SIGNAL@' "$work/runner.log"; then
  finish @EXIT_IDLE@
fi
echo 'GridOps: gitlab-runner stopped without running a job.' >&2
finish 1"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisioningPlan {
    pub provider: String,
    pub platform: &'static str,
    pub mode: String,
    pub ephemeral: bool,
    pub runtime: String,
    pub image: String,
}

impl ProvisioningPlan {
    /// The runner shape for `platform` on `provider`. Bitbucket runners are
    /// persistent; GitLab runners are ephemeral (one job each); both run in a
    /// VM on Tart. Only GitHub follows the pool's mode and macOS runtime.
    pub fn for_selected_provider(
        provider: &str,
        platform: CiPlatform,
        pool_mode: &str,
        pool_ephemeral: bool,
        macos_runtime: &str,
        docker_image: &str,
        tart_image: &str,
    ) -> Self {
        let (mode, ephemeral, runtime) = match platform {
            CiPlatform::Github => (
                pool_mode.to_owned(),
                pool_ephemeral,
                if provider == "tart" {
                    macos_runtime.to_owned()
                } else {
                    "vm".to_owned()
                },
            ),
            CiPlatform::Bitbucket => ("persistent".to_owned(), false, "vm".to_owned()),
            CiPlatform::Gitlab => ("ephemeral".to_owned(), true, "vm".to_owned()),
        };
        let image = if provider == "tart" {
            if runtime == "native" {
                String::new()
            } else {
                tart_image.to_owned()
            }
        } else {
            docker_image.to_owned()
        };

        Self {
            provider: provider.to_owned(),
            platform: platform.as_str(),
            mode,
            ephemeral,
            runtime,
            image,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureCleanup {
    pub release_capacity: bool,
    pub remove_provider_runner: bool,
    pub remove_manager_runner: bool,
}

/// The common failure contract after capacity has been reserved. Adapters can
/// add their own persistence/event handling, but must release the lease and
/// clean up anything already registered outside the database.
pub fn failure_cleanup(
    capacity_reserved: bool,
    provider_runner_registered: bool,
    manager_runner_created: bool,
) -> FailureCleanup {
    FailureCleanup {
        release_capacity: capacity_reserved,
        remove_provider_runner: provider_runner_registered,
        remove_manager_runner: manager_runner_created,
    }
}

/// Wraps a runner launch so the job's console output reaches the container's
/// stdout. `setup` runs first (registration, reading the secret); `launch`
/// starts the runner. The runner's own diagnostics stay in a file, and each
/// step's secret-masked console page under `_diag/pages` is mirrored to stdout
/// while the job runs, which is what live job logs read.
#[must_use]
pub fn instrumented_runner_script(setup: &str, launch: &str) -> String {
    format!(
        r#"{setup}
# The runner listener writes verbose transport diagnostics to stdout/stderr even
# when its explicit diagnostic-output flag is absent. Keep those diagnostics inside the
# container; GridOps' user-facing stream is populated exclusively from the
# secret-masked job console pages below.
runner_diagnostics=/tmp/gridops-runner-diagnostics.log
{launch} >"$runner_diagnostics" 2>&1 &
runner_pid=$!
trap 'kill -TERM "$runner_pid" 2>/dev/null || true' TERM INT

# GitHub's runner writes the actual step console stream to rotating files under
# _diag/pages. Mirror every page to container stdout so GridOps can stream job
# output while it is still running, while retaining the runner's secret masking.
# The runner lives in the image's working directory: /home/runner in GitHub's
# image, /actions-runner in myoung34/github-runner and its derivatives.
runner_root=${{GRIDOPS_RUNNER_ROOT:-$PWD}}
forwarded_state=/tmp/gridops-forwarded-job-state
rm -rf "$forwarded_state"
mkdir -p "$forwarded_state"

flush_job_logs() {{
  for job_log in "$runner_root"/_diag/pages/*.log; do
    [ -f "$job_log" ] || continue
    state_file="$forwarded_state/$(basename "$job_log").offset"
    offset=0
    if [ -f "$state_file" ]; then read -r offset < "$state_file" || offset=0; fi
    size=$(wc -c < "$job_log" 2>/dev/null) || continue
    if [ "$size" -gt "$offset" ]; then
      if [ "$offset" -eq 0 ]; then
        printf '\n[GRIDOPS JOB LOG %s]\n' "$(basename "$job_log")"
      fi
      head -c "$size" "$job_log" 2>/dev/null | tail -c "+$((offset + 1))" || true
      printf '%s\n' "$size" > "$state_file"
    fi
  done
}}

(
  while kill -0 "$runner_pid" 2>/dev/null; do
    flush_job_logs
    sleep 0.1
  done
  flush_job_logs
) &
forwarder_pid=$!

set +e
wait "$runner_pid"
runner_status=$?
set -e
wait "$forwarder_pid" 2>/dev/null || true
# Runners start this in a login shell, which runs ~/.bash_logout on the way out.
# GitHub's image ships Ubuntu's, whose clear_console test fails; with errexit
# still on, that turns every exit into 1.
set +e
exit "$runner_status""#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::CiPlatform;

    fn plan(provider: &str, platform: CiPlatform) -> ProvisioningPlan {
        ProvisioningPlan::for_selected_provider(
            provider,
            platform,
            "ephemeral",
            true,
            "native",
            "docker:latest",
            "tart:latest",
        )
    }

    #[test]
    fn api_and_reconciler_inputs_produce_the_same_github_plan() {
        assert_eq!(
            plan("tart", CiPlatform::Github),
            ProvisioningPlan::for_selected_provider(
                "tart",
                CiPlatform::Github,
                "ephemeral",
                true,
                "native",
                "docker:latest",
                "tart:latest",
            )
        );
        assert_eq!(plan("tart", CiPlatform::Github).platform, "github");
        assert_eq!(plan("tart", CiPlatform::Github).runtime, "native");
        assert_eq!(plan("tart", CiPlatform::Github).image, "");
    }

    #[test]
    fn bitbucket_is_always_persistent_and_vm_hosted() {
        let selected = plan("tart", CiPlatform::Bitbucket);
        assert_eq!(selected.platform, "bitbucket");
        assert_eq!(selected.mode, "persistent");
        assert!(!selected.ephemeral);
        assert_eq!(selected.runtime, "vm");
        assert_eq!(selected.image, "tart:latest");
    }

    fn expected(
        provider: &str,
        platform: &'static str,
        mode: &str,
        ephemeral: bool,
        runtime: &str,
        image: &str,
    ) -> ProvisioningPlan {
        ProvisioningPlan {
            provider: provider.to_owned(),
            platform,
            mode: mode.to_owned(),
            ephemeral,
            runtime: runtime.to_owned(),
            image: image.to_owned(),
        }
    }

    /// Every platform on both providers, for two sets of pool inputs. The
    /// GitHub and Bitbucket rows are what the `bitbucket_connection_selected`
    /// flag gave before #137 (false and true); GitLab is always an ephemeral
    /// VM runner, whatever the pool says.
    #[test]
    fn each_platform_gets_its_plan_on_both_providers() {
        let pools = [("ephemeral", true, "native"), ("persistent", false, "vm")];
        let rows = [
            // (provider, platform, pool inputs, expected plan)
            (
                "docker",
                CiPlatform::Github,
                0,
                expected("docker", "github", "ephemeral", true, "vm", "docker:latest"),
            ),
            (
                "tart",
                CiPlatform::Github,
                0,
                expected("tart", "github", "ephemeral", true, "native", ""),
            ),
            (
                "docker",
                CiPlatform::Github,
                1,
                expected(
                    "docker",
                    "github",
                    "persistent",
                    false,
                    "vm",
                    "docker:latest",
                ),
            ),
            (
                "tart",
                CiPlatform::Github,
                1,
                expected("tart", "github", "persistent", false, "vm", "tart:latest"),
            ),
            (
                "docker",
                CiPlatform::Bitbucket,
                0,
                expected(
                    "docker",
                    "bitbucket",
                    "persistent",
                    false,
                    "vm",
                    "docker:latest",
                ),
            ),
            (
                "tart",
                CiPlatform::Bitbucket,
                0,
                expected(
                    "tart",
                    "bitbucket",
                    "persistent",
                    false,
                    "vm",
                    "tart:latest",
                ),
            ),
            (
                "docker",
                CiPlatform::Bitbucket,
                1,
                expected(
                    "docker",
                    "bitbucket",
                    "persistent",
                    false,
                    "vm",
                    "docker:latest",
                ),
            ),
            (
                "tart",
                CiPlatform::Bitbucket,
                1,
                expected(
                    "tart",
                    "bitbucket",
                    "persistent",
                    false,
                    "vm",
                    "tart:latest",
                ),
            ),
            (
                "docker",
                CiPlatform::Gitlab,
                0,
                expected("docker", "gitlab", "ephemeral", true, "vm", "docker:latest"),
            ),
            (
                "tart",
                CiPlatform::Gitlab,
                0,
                expected("tart", "gitlab", "ephemeral", true, "vm", "tart:latest"),
            ),
            (
                "docker",
                CiPlatform::Gitlab,
                1,
                expected("docker", "gitlab", "ephemeral", true, "vm", "docker:latest"),
            ),
            (
                "tart",
                CiPlatform::Gitlab,
                1,
                expected("tart", "gitlab", "ephemeral", true, "vm", "tart:latest"),
            ),
        ];
        for (provider, platform, pool, want) in rows {
            let (mode, ephemeral, macos_runtime) = pools[pool];
            assert_eq!(
                ProvisioningPlan::for_selected_provider(
                    provider,
                    platform,
                    mode,
                    ephemeral,
                    macos_runtime,
                    "docker:latest",
                    "tart:latest",
                ),
                want,
                "{provider} {platform:?} pool {pool}"
            );
        }
    }

    #[test]
    fn gitlab_runner_constants_are_pinned() {
        assert_eq!(GITLAB_DEFAULT_WAIT_TIMEOUT_SECS, 600);
        assert_eq!(GITLAB_RUNNER_TOKEN_PREFIX, "glrt-");
        assert_eq!(
            (
                GITLAB_EXIT_COMPLETED,
                GITLAB_EXIT_IDLE_EXPIRED,
                GITLAB_EXIT_REGISTRATION_REFUSED
            ),
            (0, 75, 77)
        );
    }

    /// The exit status to `exitReason` table both runner-manager listings use
    /// (the Tart agent keeps a copy, pinned by the same literals). 78
    /// (`EX_CONFIG`) is a Tart image without `gitlab-runner` (#173).
    #[test]
    fn gitlab_exit_reasons_map_each_exit_status_once() {
        assert_eq!(
            GITLAB_EXIT_REASONS.as_slice(),
            [
                (0, "completed"),
                (75, "idle_expired"),
                (77, "registration_refused"),
                (78, "runner_missing"),
            ]
            .as_slice()
        );
    }

    #[test]
    fn failure_contract_only_cleans_up_resources_that_exist() {
        assert_eq!(
            failure_cleanup(true, true, false),
            FailureCleanup {
                release_capacity: true,
                remove_provider_runner: true,
                remove_manager_runner: false,
            }
        );
        assert_eq!(
            failure_cleanup(true, false, true),
            FailureCleanup {
                release_capacity: true,
                remove_provider_runner: false,
                remove_manager_runner: true,
            }
        );
    }
}
