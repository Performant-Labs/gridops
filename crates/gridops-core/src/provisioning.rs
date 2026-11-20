//! Shared provisioning decisions and lifecycle obligations.
//!
//! The API and reconciler own their I/O adapters, credentials, and persistence
//! details. This module owns the decisions that must not drift between them.

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
    pub fn for_selected_provider(
        provider: &str,
        bitbucket_connection_selected: bool,
        pool_mode: &str,
        pool_ephemeral: bool,
        macos_runtime: &str,
        docker_image: &str,
        tart_image: &str,
    ) -> Self {
        let platform = if bitbucket_connection_selected {
            "bitbucket"
        } else {
            "github"
        };
        let mode = if bitbucket_connection_selected {
            "persistent".to_owned()
        } else {
            pool_mode.to_owned()
        };
        let ephemeral = platform == "github" && pool_ephemeral;
        let runtime = if provider == "tart" && platform == "github" {
            macos_runtime.to_owned()
        } else {
            "vm".to_owned()
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
            platform,
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
    /// (the Tart agent keeps a copy, pinned by the same literals).
    #[test]
    fn gitlab_exit_reasons_map_each_exit_status_once() {
        assert_eq!(
            GITLAB_EXIT_REASONS,
            [
                (0, "completed"),
                (75, "idle_expired"),
                (77, "registration_refused")
            ]
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
