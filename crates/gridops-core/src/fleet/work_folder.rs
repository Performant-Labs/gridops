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

    #[test]
    fn every_other_runner_keeps_the_relative_default() {
        assert_eq!(job_work_folder(&RuntimeKind::Docker, false, "x"), "_work");
        for other in [RuntimeKind::TartVm, RuntimeKind::NativeProcess] {
            assert_eq!(job_work_folder(&other, true, "x"), "_work");
            assert_eq!(job_work_folder(&other, false, "x"), "_work");
        }
    }
}
