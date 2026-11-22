//! Where a Docker runner keeps its job files when its jobs share the host's Docker daemon.
//!
//! A job that uses a Docker action (or a `container:` / `services:` entry) makes the runner ask the
//! daemon behind `/var/run/docker.sock` to bind-mount paths from the runner's work folder into
//! another container. The daemon resolves those paths on the *host*. With the default relative
//! `_work`, the runner's work folder is `/actions-runner/_work` inside the runner container, a
//! path that does not exist on the host, so the action container starts with empty mounts and fails
//! (for example `FileNotFoundError: /github/workflow/event.json`).
//!
//! The fix is a work folder with the same absolute path in both places. The server puts that path in
//! the runner's just-in-time configuration; the host agent creates the directory and bind-mounts it at
//! the same path. This module is the one place both sides take the path from.

use std::fmt::Display;

use super::RuntimeKind;

/// Parent of every shared work folder, on the host and inside the runner container alike.
/// It sits in the host agent's systemd `StateDirectory`, which is writable by the agent's user.
pub const SHARED_WORK_ROOT: &str = "/var/lib/gridops-agent/work";

/// The runner's own default, relative to its install directory. Used whenever the folder is not shared.
pub const DEFAULT_WORK_FOLDER: &str = "_work";

/// The absolute work folder for one prepared environment.
pub fn shared_work_folder(environment: impl Display) -> String {
    format!("{SHARED_WORK_ROOT}/{environment}")
}

/// The `work_folder` to put in a runner's registration for a profile.
///
/// Only a Docker runner whose profile shares the host Docker socket can run jobs that make the daemon
/// mount the work folder, so every other runner keeps `_work` and behaves as before.
pub fn job_work_folder(
    runtime: &RuntimeKind,
    shares_docker_socket: bool,
    environment: impl Display,
) -> String {
    if *runtime == RuntimeKind::Docker && shares_docker_socket {
        shared_work_folder(environment)
    } else {
        DEFAULT_WORK_FOLDER.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_docker_runner_sharing_the_socket_gets_one_absolute_folder_per_environment() {
        let folder = job_work_folder(&RuntimeKind::Docker, true, "0a1b2c");
        assert_eq!(folder, "/var/lib/gridops-agent/work/0a1b2c");
        assert!(folder.starts_with('/'));
        assert_ne!(
            folder,
            job_work_folder(&RuntimeKind::Docker, true, "0a1b2d")
        );
    }

    /// #126 decision 1: a runner's root is one folder per environment, directly under its own
    /// root, built from the typed environment id.
    #[test]
    fn a_runner_root_is_a_direct_child_of_the_runners_root()
    -> Result<(), crate::fleet::ids::NilFleetUuid> {
        let environment = crate::fleet::ids::PreparedEnvironmentId::try_new(
            uuid::Uuid::from_u128(0x0a1b_2c3d_4e5f_4061_8293_a4b5_c6d7_e8f9),
        )?;
        let root = shared_runner_root(environment);
        assert_eq!(
            root,
            "/var/lib/gridops-agent/runners/0a1b2c3d-4e5f-4061-8293-a4b5c6d7e8f9"
        );
        let path = std::path::Path::new(&root);
        assert_eq!(
            path.parent(),
            Some(std::path::Path::new(SHARED_RUNNERS_ROOT))
        );
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(environment.to_string().as_str())
        );
        Ok(())
    }

    /// #126 decision 1 and item 5: the runner's root must not sit inside a work folder (runner
    /// 2.338.2 maps Work before Externals, so a root there turns the node path into `/__w/...`),
    /// and the externals cache must not sit under the work root (whose children Cleanup removes).
    /// All three roots are in the agent's `StateDirectory`, and none contains another.
    #[test]
    fn the_runner_root_and_the_externals_cache_are_never_under_a_work_folder()
    -> Result<(), crate::fleet::ids::NilFleetUuid> {
        use std::path::Path;
        let environment =
            crate::fleet::ids::PreparedEnvironmentId::try_new(uuid::Uuid::from_u128(7))?;
        assert_eq!(EXTERNALS_CACHE_ROOT, "/var/lib/gridops-agent/externals");
        let runner_root = shared_runner_root(environment);
        let work = shared_work_folder(environment);
        assert!(
            !Path::new(&runner_root).starts_with(&work),
            "{runner_root} is under {work}"
        );
        assert!(!Path::new(&runner_root).starts_with(SHARED_WORK_ROOT));
        assert!(!Path::new(EXTERNALS_CACHE_ROOT).starts_with(SHARED_WORK_ROOT));
        assert!(!Path::new(EXTERNALS_CACHE_ROOT).starts_with(SHARED_RUNNERS_ROOT));
        let roots = [SHARED_WORK_ROOT, SHARED_RUNNERS_ROOT, EXTERNALS_CACHE_ROOT];
        for root in roots {
            assert!(
                Path::new(root).starts_with("/var/lib/gridops-agent"),
                "{root}"
            );
            for other in roots {
                assert!(
                    root == other || !Path::new(root).starts_with(other),
                    "{root} is under {other}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn every_other_runner_keeps_the_relative_default() {
        assert_eq!(job_work_folder(&RuntimeKind::Docker, false, "x"), "_work");
        for other in [RuntimeKind::TartVm, RuntimeKind::NativeProcess] {
            assert_eq!(job_work_folder(&other, true, "x"), "_work");
            assert_eq!(job_work_folder(&other, false, "x"), "_work");
        }
    }
}
