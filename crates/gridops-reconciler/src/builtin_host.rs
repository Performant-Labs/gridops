//! What the reconciler does when the built-in runner host is off: it never
//! starts a runner on the machine `GridOps` runs on, it winds down the idle
//! runners already there, and it treats a pool that has nowhere else to run as
//! unable to run rather than leaving it to fail and retry.
//!
//! The decisions are plain functions of the setting so they can be checked
//! without a manager. The runner manager enforces the same setting
//! independently when asked to start anything.

use gridops_core::fleet::{ExecutionOs, targets::Placement};

use super::Runner;

/// The placement the pool's platform really has. With the host off, a Linux
/// platform (which the host would run in Docker) goes to machines only,
/// whatever was saved: the saved choice is kept and applies again if the host
/// is turned back on. Other platforms never ran on this host's Docker.
pub(super) const fn effective_placement(
    builtin_enabled: bool,
    os: ExecutionOs,
    configured: Placement,
) -> Placement {
    if builtin_enabled || !matches!(os, ExecutionOs::Linux) {
        configured
    } else {
        Placement::Machines
    }
}

/// Whether the host may stand in for a machine for this platform.
pub(super) const fn host_may_serve(builtin_enabled: bool, os: ExecutionOs) -> bool {
    builtin_enabled || !matches!(os, ExecutionOs::Linux)
}

/// The providers the manager may be asked to start runners with. Docker is
/// this host; Tart runs on a separate Mac and is not affected.
pub(super) fn host_providers(builtin_enabled: bool, providers: Vec<String>) -> Vec<String> {
    if builtin_enabled {
        providers
    } else {
        providers
            .into_iter()
            .filter(|provider| provider != "docker")
            .collect()
    }
}

/// A runner on this host that is not working, to remove now the host is off.
/// A busy one finishes its job first, and ephemeral ones then go on their own.
pub(super) fn retire_when_off(builtin_enabled: bool, runner: &Runner) -> bool {
    !builtin_enabled
        && !runner.fleet
        && !runner.busy
        && runner.provider == "docker"
        && runner.container_id.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner() -> Runner {
        Runner {
            id: "runner-1".into(),
            installation_id: 1,
            name: "linux-1234".into(),
            container_id: Some("0123456789abcdef".into()),
            github_runner_id: None,
            ci_platform: "github".into(),
            bitbucket_connection_id: None,
            bitbucket_runner_uuid: None,
            target_repository_id: None,
            repository_owner: None,
            repository_name: None,
            provider: "docker".into(),
            status: "online".into(),
            busy: false,
            last_job_id: None,
            configuration_version: 1,
            updated_at: 1_000,
            fleet: false,
        }
    }

    #[test]
    fn linux_goes_to_machines_only_while_the_host_is_off() {
        for configured in [
            Placement::Machines,
            Placement::MachinesFirst,
            Placement::HostFirst,
        ] {
            assert_eq!(
                effective_placement(false, ExecutionOs::Linux, configured),
                Placement::Machines
            );
            assert_eq!(
                effective_placement(true, ExecutionOs::Linux, configured),
                configured,
                "on keeps what was saved"
            );
        }
        // Tart and other platforms do not run in this host's Docker.
        assert_eq!(
            effective_placement(false, ExecutionOs::Macos, Placement::HostFirst),
            Placement::HostFirst
        );
        assert!(!host_may_serve(false, ExecutionOs::Linux));
        assert!(host_may_serve(false, ExecutionOs::Macos));
        assert!(host_may_serve(true, ExecutionOs::Linux));
    }

    #[test]
    fn docker_is_dropped_from_the_providers_while_the_host_is_off() {
        let both = vec!["docker".to_owned(), "tart".to_owned()];
        assert_eq!(host_providers(false, both.clone()), ["tart"]);
        assert_eq!(host_providers(true, both), ["docker", "tart"]);
        assert!(host_providers(false, vec!["docker".into()]).is_empty());
    }

    #[test]
    fn idle_runners_on_the_host_are_retired_and_everything_else_is_left() {
        assert!(retire_when_off(false, &runner()));
        assert!(!retire_when_off(true, &runner()), "host on");
        assert!(
            !retire_when_off(
                false,
                &Runner {
                    busy: true,
                    ..runner()
                }
            ),
            "finishes its job"
        );
        assert!(
            !retire_when_off(
                false,
                &Runner {
                    fleet: true,
                    ..runner()
                }
            ),
            "on a machine"
        );
        assert!(
            !retire_when_off(
                false,
                &Runner {
                    container_id: None,
                    ..runner()
                }
            ),
            "no container here"
        );
        assert!(
            !retire_when_off(
                false,
                &Runner {
                    provider: "tart".into(),
                    ..runner()
                }
            ),
            "Tart is elsewhere"
        );
    }
}
