//! #126: a GitHub runner that shares the host's Docker socket gets its `externals` and its tool
//! cache at paths that exist on the host, so the daemon can bind-mount them into job containers.
//!
//! The pure parts (the cache name, the Start check, the `docker create` arguments, the runner
//! scripts, the shared-folder predicate) are tested directly; the scripts run under `bash` in a
//! temp dir. The Prepare fill, the stale-cache sweep, Start's refusal and Cleanup run against a
//! stand-in `docker` CLI (the `tests/docker_runtime.rs` pattern: the test reruns itself with the
//! stand-in first on `PATH`). No test needs a Docker daemon.
//!
//! The stand-in answers as Docker 29 does where it matters: `docker image inspect` of a missing
//! image says "No such image", plain `docker inspect` says "no such object", `docker rm -f` of a
//! missing container succeeds, `docker cp` of a missing path says "Could not find the file … in
//! container …", and `{{.Config.WorkingDir}}` on an image without one is a template error (an
//! older Docker printed an empty string; both are tested).

use std::{
    fs,
    io::Write as _,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context as _, Result, bail, ensure};
use gridops_core::fleet::{
    Architecture, CiTarget, ExecutionOs, ExternalId, GitlabConnectionId, ResourceRequest,
    RuntimeKind,
    ids::{BootstrapReference, PreparedEnvironmentId, ProfileId, Revision, UpstreamText},
    protocol::{
        agent::{
            AgentOperation, ExecutionConfiguration, LocalResourceIdentity, PrepareEnvironment,
            PreparedWorkload, RunnerMode, StartRunner,
        },
        outcomes::FailureCode,
        primitives::{ImageReference, WireTimestamp},
    },
};

use super::{
    DockerRuntime, Refusal, externals_cache, externals_cache_name, github_runner_script,
    shared_socket_args, uses_externals_cache,
};
use crate::helper::{Bootstrap, HelperRequest, HelperResponse, RuntimeHelper as _};

/// The pool image's tag, and the id `docker image inspect` gives for it.
const TAG: &str = "harbor.example/performantlabs/pl-runner:1.70.4";
const HEX: &str = "2f529cfa0d48433f3bd3f5c0723cb7bf34de2096d7dbec805c10f81048b19491";
/// Other images: `HEX_A` is still present locally, `HEX_B` is gone, `HEX_C` a crashed fill.
const HEX_A: &str = "aaaa0000000000000000000000000000000000000000000000000000000000aa";
const HEX_B: &str = "bbbb0000000000000000000000000000000000000000000000000000000000bb";
const HEX_C: &str = "cccc0000000000000000000000000000000000000000000000000000000000cc";
const SOCKET: &str = "/var/run/docker.sock";

// ---------------------------------------------------------------------------------------------
// The cache name and the Start check
// ---------------------------------------------------------------------------------------------

/// Item 1 and A warn 5: the cache folder is named from `{{.Id}}` without `sha256:`, and the name
/// is always one plain path component. Anything else is no name at all.
#[test]
fn the_cache_name_is_the_image_ids_hex_and_always_one_path_component() {
    assert_eq!(
        externals_cache_name(&format!("sha256:{HEX}")).as_deref(),
        Some(HEX)
    );
    for bad in [
        "",
        "sha256:",
        "sha256:.",
        "sha256:..",
        "sha256:../etc",
        "sha256:ab/cd",
        "sha256:2F529CFA",
        "sha256:2f52 9cfa",
        "sha256:2f529cfa\n",
        &format!("sha256:{HEX}.partial"),
        &format!("sha256:{HEX}/.."),
        "/",
        ".",
        "..",
    ] {
        assert_eq!(externals_cache_name(bad), None, "{bad:?}");
    }
    for id in [
        format!("sha256:{HEX}"),
        format!("sha256:{HEX_A}"),
        "sha256:abc".to_owned(),
        "sha256:a/b".to_owned(),
        "sha256:..".to_owned(),
    ] {
        if let Some(name) = externals_cache_name(&id) {
            let mut components = Path::new(&name).components();
            assert!(
                matches!(components.next(), Some(std::path::Component::Normal(_)))
                    && components.next().is_none()
                    && !name.contains('/')
                    && !name.is_empty(),
                "{id:?} gave {name:?}"
            );
        }
    }
}

/// Item 3 / decision 5: Start finds the cache for the image id it resolved, or refuses with
/// `Runtime, true`, naming the image id and the path. A leftover staging folder is no cache, and an
/// id that gives no name is a Docker error.
#[test]
fn the_start_check_finds_the_cache_or_refuses_retryably_naming_the_id_and_path() -> Result<()> {
    let root = scratch("start-check")?;
    fs::create_dir_all(root.join(HEX))?;
    fs::create_dir_all(root.join(format!("{HEX_B}.partial")))?;

    let found = externals_cache(&root, &format!("sha256:{HEX}"));
    ensure!(
        matches!(&found, Ok(path) if *path == root.join(HEX)),
        "{}",
        outcome(&found)
    );

    let missing = externals_cache(&root, &format!("sha256:{HEX_B}"));
    let Err(refusal) = &missing else {
        bail!("a missing cache was found: {}", outcome(&missing));
    };
    ensure!(
        refusal.code == FailureCode::Runtime && refusal.retryable,
        "{}",
        describe(refusal)
    );
    let path = root.join(HEX_B).display().to_string();
    ensure!(
        refusal.reason.contains(HEX_B) && refusal.reason.contains(&path),
        "the reason must name the image id and {path}: {}",
        refusal.reason
    );

    let bad = externals_cache(&root, "sha256:NOT/HEX");
    let Err(refusal) = &bad else {
        bail!("an id with no cache name was accepted: {}", outcome(&bad));
    };
    ensure!(
        refusal.code == FailureCode::Runtime
            && refusal.retryable
            && refusal.reason.contains("sha256:NOT/HEX"),
        "{}",
        describe(refusal)
    );
    fs::remove_dir_all(&root)?;
    Ok(())
}

/// A warn 6: Prepare's fill and Start's check use one predicate. GitHub runners (and fix
/// sandboxes, which start through the GitHub path) that share the socket use the cache; nothing
/// else does.
#[test]
fn only_a_github_runner_that_shares_the_socket_uses_the_externals_cache() -> Result<()> {
    let github = CiTarget::GithubRepository {
        installation_id: ExternalId::new(1)?,
        repository_id: ExternalId::new(1)?,
    };
    let organization = CiTarget::GithubOrganization {
        installation_id: ExternalId::new(1)?,
        organization_id: ExternalId::new(2)?,
    };
    let project = CiTarget::GitlabProject {
        connection_id: GitlabConnectionId::new("gitlab-connection-1")?,
        project_id: ExternalId::new(42)?,
    };
    let group = CiTarget::GitlabGroup {
        connection_id: GitlabConnectionId::new("gitlab-connection-1")?,
        group_id: ExternalId::new(43)?,
    };
    for (case, target, workload, socket, expected) in [
        (
            "github, socket",
            &github,
            PreparedWorkload::CiRunner,
            true,
            true,
        ),
        (
            "github org, socket",
            &organization,
            PreparedWorkload::CiRunner,
            true,
            true,
        ),
        (
            "fix sandbox, socket",
            &github,
            PreparedWorkload::FixSandbox,
            true,
            true,
        ),
        (
            "github, no socket",
            &github,
            PreparedWorkload::CiRunner,
            false,
            false,
        ),
        (
            "gitlab project, socket",
            &project,
            PreparedWorkload::CiRunner,
            true,
            false,
        ),
        (
            "gitlab group, socket",
            &group,
            PreparedWorkload::CiRunner,
            true,
            false,
        ),
        (
            "gitlab, no socket",
            &project,
            PreparedWorkload::CiRunner,
            false,
            false,
        ),
    ] {
        let mut prepared = prepare_environment(target.clone(), RunnerMode::Ephemeral, socket)?;
        prepared.workload = workload;
        let request = HelperRequest {
            environment_id: Some(PreparedEnvironmentId::try_new(uuid::Uuid::new_v4())?),
            docker_socket: socket,
            prepared: Some(prepared.clone()),
            bootstrap: None,
            operation: AgentOperation::PrepareEnvironment(prepared.clone()),
        };
        ensure!(
            uses_externals_cache(&request, &prepared) == expected,
            "{case}: expected {expected}"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The runner container's arguments and scripts
// ---------------------------------------------------------------------------------------------

/// Items 1 and 2, decisions 4, 6 and 9: the socket branch mounts the work folder and the runner
/// root at their own paths, the cache read-only with `--mount` (never `-v`, which would create a
/// missing source), and tells the scripts where each is. The tool cache is the work folder's
/// `_tool`, never the image's `/opt/hostedtoolcache`.
#[test]
fn a_shared_runner_gets_its_folders_at_host_paths_and_the_cache_read_only() -> Result<()> {
    let work = Path::new("/var/lib/gridops-agent/work/0a1b");
    let root = Path::new("/var/lib/gridops-agent/runners/0a1b");
    let cache = PathBuf::from(format!("/var/lib/gridops-agent/externals/{HEX}"));
    let args = shared_socket_args(SOCKET, work, root, &cache);
    let (work, root, cache) = (
        work.display().to_string(),
        root.display().to_string(),
        cache.display().to_string(),
    );

    ensure!(
        values(&args, &["-v", "--volume"]).contains(&format!("{SOCKET}:/var/run/docker.sock")),
        "no Docker socket: {args:?}"
    );
    ensure!(
        values(&args, &["-v", "--volume"]).contains(&format!("{work}:{work}")),
        "the work folder is not at its own path: {args:?}"
    );
    let root_mounted = values(&args, &["-v", "--volume"]).contains(&format!("{root}:{root}"))
        || mounts(&args)
            .iter()
            .any(|mount| mount.source == root && mount.target == root && !mount.readonly);
    ensure!(
        root_mounted,
        "the runner root is not mounted writable at its own path: {args:?}"
    );
    ensure!(
        mounts(&args).iter().any(|mount| mount.bind
            && mount.source == cache
            && mount.target == cache
            && mount.readonly),
        "the externals cache is not a read-only --mount at its own path: {args:?}"
    );
    ensure!(
        !values(&args, &["-v", "--volume"])
            .iter()
            .any(|volume| volume.contains(&cache)),
        "the cache is mounted with -v, which creates a missing source: {args:?}"
    );

    let env = env_of(&args);
    for (name, value) in [
        ("GRIDOPS_RUNNER_WORK", work.clone()),
        ("GRIDOPS_RUNNER_ROOT", root.clone()),
        ("GRIDOPS_RUNNER_EXTERNALS", cache.clone()),
        ("RUNNER_TOOL_CACHE", format!("{work}/_tool")),
    ] {
        ensure!(
            env.iter().any(|(key, set)| key == name && *set == value),
            "missing -e {name}={value}: {args:?}"
        );
    }
    ensure!(
        env.iter()
            .filter(|(key, _)| key == "AGENT_TOOLSDIRECTORY")
            .all(|(_, value)| *value == format!("{work}/_tool")),
        "AGENT_TOOLSDIRECTORY points elsewhere than the work folder's _tool: {args:?}"
    );
    for host_only in ["/actions-runner", "/opt/hostedtoolcache", "/__e", "/__t"] {
        ensure!(
            !args.iter().any(|arg| arg.contains(host_only)),
            "{host_only} is not a host path: {args:?}"
        );
    }
    Ok(())
}

/// Decision 4 / A warn 1, JIT: after reading the secret, the script makes the runner root a copy
/// of the image's runner directory (with `bin` copied, not linked) whose `externals` is a link to
/// the cache, and runs `run.sh` there; the live job log is read from there too. The image's own
/// directory is left alone.
#[test]
fn a_shared_jit_runner_runs_from_its_runner_root_with_externals_linked_to_the_cache() -> Result<()>
{
    let runner = RunnerDirs::new("jit")?;
    let output = runner.run(RunnerMode::Ephemeral, true, "jit-token\n", &[])?;
    let log = runner.log()?;
    ensure!(
        output.status.success(),
        "the script failed: {}\n{log}",
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(
        ran_in(&log, "run")? == canonical(&runner.root)?,
        "run.sh did not run in the runner root:\n{log}"
    );
    ensure!(
        log.contains("run args=--jitconfig jit-token"),
        "run.sh did not get the JIT config:\n{log}"
    );
    runner.root_is_a_runner_linked_to_the_cache()?;
    ensure!(
        !runner.image.join("_diag").exists() && runner.image_is_untouched()?,
        "the image's own runner directory was changed"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    ensure!(
        stdout.contains("GRIDOPS JOB LOG job.log") && stdout.contains("job output from"),
        "the job log in the runner root was not forwarded: {stdout}"
    );
    runner.remove()
}

/// Decision 4 / A warn 1, persistent: `config.sh` runs in the runner root (so `.runner` and
/// `.credentials` land where `run.sh` looks), with the shared work folder, then `run.sh` runs
/// there configured.
#[test]
fn a_shared_persistent_runner_is_configured_and_run_from_its_runner_root() -> Result<()> {
    let runner = RunnerDirs::new("persistent")?;
    let output = runner.run(RunnerMode::Persistent, true, "reg-token\n", &registration())?;
    let log = runner.log()?;
    ensure!(
        output.status.success(),
        "the script failed: {}\n{log}",
        String::from_utf8_lossy(&output.stderr)
    );
    let root = canonical(&runner.root)?;
    ensure!(
        ran_in(&log, "config")? == root,
        "config.sh did not run in the runner root:\n{log}"
    );
    let work = runner.work.display().to_string();
    ensure!(
        log.contains("--token reg-token") && log.contains(&format!("--work {work}")),
        "config.sh did not get the token and the shared work folder:\n{log}"
    );
    ensure!(
        ran_in(&log, "run")? == root && log.contains("run configured=yes"),
        "run.sh did not run configured in the runner root:\n{log}"
    );
    runner.root_is_a_runner_linked_to_the_cache()?;
    ensure!(
        !runner.image.join(".runner").exists() && runner.image_is_untouched()?,
        "the image's own runner directory was configured or changed"
    );
    runner.remove()
}

/// Item 4: a runner that does not share the socket runs as before, in the image's own directory,
/// in both modes.
#[test]
fn a_runner_that_does_not_share_the_socket_still_runs_in_the_images_directory() -> Result<()> {
    for (mode, secret, extra) in [
        (RunnerMode::Ephemeral, "jit-token\n", Vec::new()),
        (RunnerMode::Persistent, "reg-token\n", registration()),
    ] {
        let runner = RunnerDirs::new("unshared")?;
        let output = runner.run(mode, false, secret, &extra)?;
        let log = runner.log()?;
        ensure!(
            output.status.success(),
            "{mode:?}: the script failed: {}\n{log}",
            String::from_utf8_lossy(&output.stderr)
        );
        ensure!(
            ran_in(&log, "run")? == canonical(&runner.image)?,
            "{mode:?}: run.sh did not run in the image's directory:\n{log}"
        );
        if mode == RunnerMode::Persistent {
            ensure!(
                log.contains("--work _work"),
                "{mode:?}: the default work folder changed:\n{log}"
            );
        } else {
            ensure!(log.contains("run args=--jitconfig jit-token"), "{log}");
        }
        ensure!(
            entries(&runner.root)?.is_empty(),
            "{mode:?}: an unshared runner wrote a runner root"
        );
        runner.remove()?;
    }
    Ok(())
}

/// Item 2 and decision 9: the tool cache (`/__t`'s source) is the work folder's `_tool`, whatever
/// the image sets. The pool image's ENV has `AGENT_TOOLSDIRECTORY=/opt/hostedtoolcache`, and the
/// runner applies the `.env` of the directory it runs from over its environment, so an image
/// `.env` naming a tool directory must not win either. Checked as the runner resolves it:
/// `RUNNER_TOOL_CACHE`, else `AGENT_TOOLSDIRECTORY`, else `<work>/_tool`, after `.env` is applied
/// (later lines win).
#[test]
fn the_runner_resolves_its_tool_cache_to_the_work_folder_whatever_the_image_sets() -> Result<()> {
    let runner = RunnerDirs::new("tool-cache")?;
    fs::write(
        runner.image.join(".env"),
        "LANG=C.UTF-8\nRUNNER_TOOL_CACHE=/opt/hostedtoolcache\nAGENT_TOOLSDIRECTORY=/opt/hostedtoolcache\n",
    )?;
    let output = runner.run(RunnerMode::Ephemeral, true, "jit-token\n", &[])?;
    let log = runner.log()?;
    ensure!(
        output.status.success(),
        "the script failed: {}\n{log}",
        String::from_utf8_lossy(&output.stderr)
    );
    let seen = log
        .lines()
        .find_map(|line| line.strip_prefix("run tool="))
        .context(format!("run.sh did not run:\n{log}"))?;
    let (mut tool_cache, mut tools_directory) = seen
        .split_once('|')
        .map(|(cache, directory)| (cache.to_owned(), directory.to_owned()))
        .context("run.sh logged no tool variables")?;
    let ran_in = ran_in(&log, "run")?;
    let env_file = fs::read_to_string(ran_in.join(".env")).unwrap_or_default();
    for line in env_file.lines() {
        match line.split_once('=') {
            Some(("RUNNER_TOOL_CACHE", value)) => value.clone_into(&mut tool_cache),
            Some(("AGENT_TOOLSDIRECTORY", value)) => value.clone_into(&mut tools_directory),
            _ => {}
        }
    }
    let work_tool = format!("{}/_tool", runner.work.display());
    let effective = if !tool_cache.is_empty() {
        tool_cache
    } else if !tools_directory.is_empty() {
        tools_directory
    } else {
        work_tool.clone()
    };
    ensure!(
        effective == work_tool,
        "the runner in {} would use {effective} as its tool cache, not {work_tool}; its .env:\n{env_file}",
        ran_in.display()
    );
    runner.remove()
}

// ---------------------------------------------------------------------------------------------
// Prepare: the fill, its failures and the sweep (stand-in `docker`)
// ---------------------------------------------------------------------------------------------

/// Items 1 and 3, decisions 3, 6 and 7, A warn 4: Prepare fills the cache once per image, from the
/// image's own working directory (here GitHub's `/home/runner`, not `/actions-runner`), with the
/// real files, through a fill container named after the cache that carries no runner label and
/// is removed again. A fill container left by a crash does not block it. A second fill of the
/// same image copies nothing.
#[tokio::test]
async fn the_cache_is_filled_once_per_image_from_the_images_working_directory() -> Result<()> {
    let Some(directory) =
        stand_in("the_cache_is_filled_once_per_image_from_the_images_working_directory")?
    else {
        return Ok(());
    };
    let docker = StandIn::new(&directory, "/home/runner", Docker::V29)?;
    docker.externals("/home/runner")?;
    docker.crashed_fill_container()?;
    let cache_root = directory.join("state/externals");
    let runtime = DockerRuntime::new(None, None);

    let filled = runtime.fill_externals_cache(&cache_root, TAG).await;
    let Ok(path) = &filled else {
        bail!("{}", outcome(&filled));
    };
    ensure!(*path == cache_root.join(HEX), "{}", path.display());
    ensure!(
        fs::symlink_metadata(path)?.is_dir(),
        "the cache is not a real folder"
    );
    ensure!(
        fs::read_to_string(path.join("node24/bin/node"))? == "node24 from /home/runner"
            && fs::read_to_string(path.join("node24/bin/npm"))? == "npm",
        "the cache does not hold the image's externals"
    );
    ensure!(
        dangling_links(path)?.is_empty(),
        "{:?}",
        dangling_links(path)?
    );
    ensure!(
        entries(&cache_root)? == vec![HEX.to_owned()],
        "staging left behind: {:?}",
        entries(&cache_root)?
    );
    let creates = docker.creates()?;
    ensure!(creates.len() == 1, "{creates:?}");
    ensure!(
        values(&creates[0], &["--name"]) == vec![format!("gridops-externals-{HEX}")],
        "the fill container is not named after its cache: {:?}",
        creates[0]
    );
    ensure!(
        !creates[0]
            .iter()
            .any(|arg| arg.contains("io.gridops.fleet.environment")),
        "the fill container carries the runner label: {:?}",
        creates[0]
    );
    ensure!(
        docker.containers()?.is_empty(),
        "containers left behind: {:?}",
        docker.containers()?
    );

    let copies = docker.calls_of("cp")?;
    let again = runtime.fill_externals_cache(&cache_root, TAG).await;
    ensure!(
        matches!(&again, Ok(again) if again == path),
        "{}",
        outcome(&again)
    );
    ensure!(
        docker.creates()?.len() == 1 && docker.calls_of("cp")? == copies,
        "a second fill of the same image copied again: {:?}",
        docker.calls()?
    );
    Ok(())
}

/// Item 1: an image whose `externals` is itself a symlink is FOLLOWED (the choice documented on
/// `fill_externals_cache`): the cache holds the real files, never the link, and never a dangling
/// link; no fill container is left behind.
#[tokio::test]
async fn a_symlinked_externals_is_followed_and_the_cache_never_holds_a_dangling_link() -> Result<()>
{
    let Some(directory) =
        stand_in("a_symlinked_externals_is_followed_and_the_cache_never_holds_a_dangling_link")?
    else {
        return Ok(());
    };
    let docker = StandIn::new(&directory, "/actions-runner", Docker::V29)?;
    let real = docker.fs().join("opt/runner-externals/node24/bin");
    fs::create_dir_all(&real)?;
    fs::write(real.join("node"), "node24 behind a link")?;
    fs::create_dir_all(docker.fs().join("actions-runner/bin"))?;
    std::os::unix::fs::symlink(
        "../opt/runner-externals",
        docker.fs().join("actions-runner/externals"),
    )?;
    let cache_root = directory.join("state/externals");

    let filled = DockerRuntime::new(None, None)
        .fill_externals_cache(&cache_root, TAG)
        .await;
    let path = match &filled {
        Ok(path) => path,
        Err(refusal) => bail!(
            "a symlinked externals must be followed: {}",
            describe(refusal)
        ),
    };
    ensure!(*path == cache_root.join(HEX), "{}", path.display());
    ensure!(
        fs::symlink_metadata(path)?.is_dir()
            && fs::read_to_string(path.join("node24/bin/node"))? == "node24 behind a link",
        "followed, but the cache does not hold the real files"
    );
    ensure!(
        dangling_links(path)?.is_empty(),
        "{:?}",
        dangling_links(path)?
    );
    ensure!(
        docker.containers()?.is_empty(),
        "containers left behind: {:?}",
        docker.containers()?
    );
    Ok(())
}

/// Decision 5: an image with no working directory, or none with an `externals` in it, cannot be
/// served by any retry: `Configuration, false`, naming the image (and the missing path), with no
/// cache and no fill container left. Docker 29 answers `{{.Config.WorkingDir}}` on such an image
/// with a template error; older Docker printed nothing. Both are the image's fault.
#[tokio::test]
async fn an_image_without_a_working_directory_or_externals_is_a_configuration_failure() -> Result<()>
{
    let Some(directory) =
        stand_in("an_image_without_a_working_directory_or_externals_is_a_configuration_failure")?
    else {
        return Ok(());
    };
    for (case, workdir, docker_version, missing_path) in [
        ("no working directory, Docker 29", "", Docker::V29, None),
        (
            "no working directory, older Docker",
            "",
            Docker::Legacy,
            None,
        ),
        (
            "no externals in the working directory",
            "/actions-runner",
            Docker::V29,
            Some("/actions-runner/externals"),
        ),
    ] {
        let docker = StandIn::new(&directory, workdir, docker_version)?;
        fs::create_dir_all(docker.fs().join("actions-runner/bin"))?;
        let cache_root = directory.join(format!("state-{}/externals", uuid::Uuid::new_v4()));
        let filled = DockerRuntime::new(None, None)
            .fill_externals_cache(&cache_root, TAG)
            .await;
        let Err(refusal) = &filled else {
            bail!("{case}: {}", outcome(&filled));
        };
        ensure!(
            refusal.code == FailureCode::Configuration && !refusal.retryable,
            "{case}: {}",
            describe(refusal)
        );
        ensure!(
            (refusal.reason.contains(HEX) || refusal.reason.contains(TAG))
                && missing_path.is_none_or(|path| refusal.reason.contains(path)),
            "{case}: the reason must name the image and what it lacks: {}",
            refusal.reason
        );
        ensure!(
            !cache_root.join(HEX).exists(),
            "{case}: a refused image left a cache"
        );
        ensure!(
            docker.containers()?.is_empty(),
            "{case}: containers left behind: {:?}",
            docker.containers()?
        );
    }
    Ok(())
}

/// Decision 5, A warn 2: a cache folder the agent may not create is `Configuration, false`,
/// naming the path. Unlike the work folder, the cache root is not moved aside (it is mounted with
/// `--mount`, which never makes the daemon create it).
#[tokio::test]
async fn an_unwritable_cache_root_is_a_configuration_failure_and_is_not_moved_aside() -> Result<()>
{
    let Some(directory) =
        stand_in("an_unwritable_cache_root_is_a_configuration_failure_and_is_not_moved_aside")?
    else {
        return Ok(());
    };
    let docker = StandIn::new(&directory, "/actions-runner", Docker::V29)?;
    docker.externals("/actions-runner")?;
    let state = directory.join("state");
    let cache_root = state.join("externals");
    fs::create_dir_all(&cache_root)?;
    fs::set_permissions(&cache_root, fs::Permissions::from_mode(0o555))?;
    if fs::create_dir(cache_root.join("probe")).is_ok() {
        // Running as root: permissions do not apply, so there is nothing to refuse.
        eprintln!("skipped: permissions do not apply to this user");
        return Ok(());
    }

    let filled = DockerRuntime::new(None, None)
        .fill_externals_cache(&cache_root, TAG)
        .await;
    let Err(refusal) = &filled else {
        bail!("{}", outcome(&filled));
    };
    ensure!(
        refusal.code == FailureCode::Configuration && !refusal.retryable,
        "{}",
        describe(refusal)
    );
    let path = cache_root.display().to_string();
    ensure!(
        refusal.reason.contains(&path)
            && (refusal.reason.contains(HEX) || refusal.reason.contains(TAG)),
        "the reason must name the image and {path}: {}",
        refusal.reason
    );
    ensure!(
        fs::metadata(&cache_root)?.permissions().mode() & 0o777 == 0o555
            && entries(&state)? == vec!["externals".to_owned()],
        "the cache root was moved aside or replaced: {:?}",
        entries(&state)?
    );
    ensure!(
        docker.containers()?.is_empty(),
        "containers left behind: {:?}",
        docker.containers()?
    );
    Ok(())
}

/// Decision 5, item 5: a Docker error during the fill (`create`, `cp` or `rm` failing, the image
/// not answering) or an id that gives no cache name is `Runtime, true`, and no staging folder
/// becomes a cache. The fill container is removed even after a failed `cp`.
#[tokio::test]
async fn a_docker_error_during_the_fill_is_retryable_and_leaves_no_cache() -> Result<()> {
    let Some(directory) =
        stand_in("a_docker_error_during_the_fill_is_retryable_and_leaves_no_cache")?
    else {
        return Ok(());
    };
    let bad_id = "sha256:2F52-NOT-HEX";
    for (case, fail, id, named) in [
        ("create fails", "create", None, HEX),
        ("cp fails", "cp", None, HEX),
        ("rm fails", "rm", None, HEX),
        ("the image does not answer", "inspect-tag", None, TAG),
        ("the id gives no cache name", "", Some(bad_id), bad_id),
    ] {
        let docker = StandIn::new(&directory, "/actions-runner", Docker::V29)?;
        docker.externals("/actions-runner")?;
        docker.set("fail", fail)?;
        if let Some(id) = id {
            docker.set("id", id)?;
        }
        let cache_root = directory.join(format!("state-{}/externals", uuid::Uuid::new_v4()));
        let filled = DockerRuntime::new(None, None)
            .fill_externals_cache(&cache_root, TAG)
            .await;
        let Err(refusal) = &filled else {
            bail!("{case}: {}", outcome(&filled));
        };
        ensure!(
            refusal.code == FailureCode::Runtime && refusal.retryable,
            "{case}: {}",
            describe(refusal)
        );
        ensure!(
            refusal.reason.contains(named),
            "{case}: the reason must name {named}: {}",
            refusal.reason
        );
        if named == HEX {
            let path = cache_root.display().to_string();
            ensure!(
                refusal.reason.contains(&path),
                "{case}: the reason must name the path {path}: {}",
                refusal.reason
            );
        }
        ensure!(
            !cache_root.join(HEX).exists(),
            "{case}: a failed fill was taken for a cache"
        );
        if fail != "rm" {
            ensure!(
                docker.containers()?.is_empty(),
                "{case}: containers left behind: {:?}",
                docker.containers()?
            );
        }
    }
    Ok(())
}

/// Item 5 (diff gate r1, B-1): a failed rename of the complete staging copy into place is
/// `Runtime, true`, naming the cache path; the staging folder is removed, nothing becomes a cache
/// (Start's check still refuses), and whatever stood at the cache path is left as it was. The
/// rename fails here because a regular file already holds the cache's name, so the fill runs
/// (it is no folder) but the staged folder cannot replace it.
#[tokio::test]
async fn a_failed_rename_into_the_cache_is_retryable_and_leaves_no_cache_or_staging() -> Result<()>
{
    let Some(directory) =
        stand_in("a_failed_rename_into_the_cache_is_retryable_and_leaves_no_cache_or_staging")?
    else {
        return Ok(());
    };
    let docker = StandIn::new(&directory, "/actions-runner", Docker::V29)?;
    docker.externals("/actions-runner")?;
    let cache_root = directory.join("state/externals");
    fs::create_dir_all(&cache_root)?;
    let target = cache_root.join(HEX);
    fs::write(&target, "not a cache")?;

    let filled = DockerRuntime::new(None, None)
        .fill_externals_cache(&cache_root, TAG)
        .await;
    let Err(refusal) = &filled else {
        bail!("{}", outcome(&filled));
    };
    ensure!(
        refusal.code == FailureCode::Runtime && refusal.retryable,
        "{}",
        describe(refusal)
    );
    let path = target.display().to_string();
    ensure!(
        refusal.reason.contains(&path) && refusal.reason.contains(TAG),
        "the reason must name the image and the cache path {path}: {}",
        refusal.reason
    );
    ensure!(
        docker.calls_of("cp")?.len() == 1,
        "the fill did not reach the rename (no copy was made): {:?}",
        docker.calls()?
    );
    ensure!(
        entries(&cache_root)? == vec![HEX.to_owned()],
        "staging left behind: {:?}",
        entries(&cache_root)?
    );
    ensure!(
        fs::symlink_metadata(&target)?.is_file() && fs::read_to_string(&target)? == "not a cache",
        "what stood at the cache path was replaced or changed"
    );
    ensure!(
        externals_cache(&cache_root, &format!("sha256:{HEX}")).is_err(),
        "a failed fill was taken for a cache"
    );
    ensure!(
        docker.containers()?.is_empty(),
        "containers left behind: {:?}",
        docker.containers()?
    );
    Ok(())
}

/// Decision 8, A warn 8: after a new fill, caches whose image Docker says is gone, and staging
/// left by a crashed fill, are removed; caches of images still present stay. A cache whose image
/// Docker could not be asked about is kept.
#[tokio::test]
async fn a_fill_sweeps_caches_whose_image_is_gone_and_leftover_staging() -> Result<()> {
    let Some(directory) =
        stand_in("a_fill_sweeps_caches_whose_image_is_gone_and_leftover_staging")?
    else {
        return Ok(());
    };
    for (case, fail, expected) in [
        ("image gone", "", vec![HEX.to_owned(), HEX_A.to_owned()]),
        (
            "Docker does not answer for the other images",
            "inspect-other",
            vec![HEX.to_owned(), HEX_A.to_owned(), HEX_B.to_owned()],
        ),
    ] {
        let docker = StandIn::new(&directory, "/actions-runner", Docker::V29)?;
        docker.externals("/actions-runner")?;
        docker.set("present", &format!("{HEX_A}\n"))?;
        docker.set("fail", fail)?;
        let cache_root = directory.join(format!("state-{}/externals", uuid::Uuid::new_v4()));
        for name in [
            HEX_A.to_owned(),
            HEX_B.to_owned(),
            format!("{HEX_C}.partial"),
        ] {
            fs::create_dir_all(cache_root.join(&name).join("node24/bin"))?;
            fs::write(cache_root.join(&name).join("node24/bin/node"), &name)?;
        }

        let filled = DockerRuntime::new(None, None)
            .fill_externals_cache(&cache_root, TAG)
            .await;
        ensure!(filled.is_ok(), "{case}: {}", outcome(&filled));
        let left: Vec<String> = entries(&cache_root)?
            .into_iter()
            .filter(|name| !(fail == "inspect-other" && name.ends_with(".partial")))
            .collect();
        ensure!(
            left == expected,
            "{case}: left {left:?}, expected {expected:?}"
        );
        ensure!(
            fs::read_to_string(cache_root.join(HEX_A).join("node24/bin/node"))? == HEX_A,
            "{case}: the live cache was changed"
        );
    }
    Ok(())
}

/// Decision 2 wiring: a GitHub Prepare that shares the socket fills the cache under the agent's
/// `StateDirectory`; where it cannot (here, a machine without one), the Prepare fails instead of
/// reporting an environment its container jobs could not use. The answer is the one decision 5
/// gives the error this machine returns for creating the cache root (diff gate r1, W-3): a folder
/// the agent may not create is `Configuration, false`, any other IO error `Runtime, true`.
#[tokio::test]
async fn a_github_prepare_sharing_the_socket_fails_when_it_cannot_fill_the_cache() -> Result<()> {
    let Some(directory) =
        stand_in("a_github_prepare_sharing_the_socket_fails_when_it_cannot_fill_the_cache")?
    else {
        return Ok(());
    };
    if not_an_agent_host_or_skip()? {
        let Some((code, retryable)) = cache_root_creation_answer()? else {
            return Ok(());
        };
        let docker = StandIn::new(&directory, "/actions-runner", Docker::V29)?;
        docker.externals("/actions-runner")?;
        let prepared = prepare_environment(github()?, RunnerMode::Ephemeral, true)?;
        let response = DockerRuntime::new(None, Some(SOCKET.to_owned()))
            .call(HelperRequest {
                environment_id: Some(PreparedEnvironmentId::try_new(uuid::Uuid::new_v4())?),
                docker_socket: true,
                prepared: None,
                bootstrap: None,
                operation: AgentOperation::PrepareEnvironment(prepared),
            })
            .await;
        ensure!(
            matches!(
                response,
                HelperResponse::Failed {
                    code: answered,
                    retryable: answered_retryable,
                    ..
                } if answered == code && answered_retryable == retryable
            ),
            "a Prepare that could not fill the externals cache answered {response:?}, expected {code:?}, retryable {retryable}"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Start and Cleanup (stand-in `docker`)
// ---------------------------------------------------------------------------------------------

/// Item 3, decisions 2 and 7, A warn 3: Start resolves the image's id itself and, when that id
/// has no cache, fails `Runtime, true` before creating anything, in both modes.
#[tokio::test]
async fn a_shared_start_without_a_cache_fails_retryably_and_creates_nothing() -> Result<()> {
    let Some(directory) =
        stand_in("a_shared_start_without_a_cache_fails_retryably_and_creates_nothing")?
    else {
        return Ok(());
    };
    if !not_an_agent_host_or_skip()? {
        return Ok(());
    }
    for (mode, material) in [
        (
            RunnerMode::Ephemeral,
            serde_json::json!({"kind": "github_jit", "jit_config": "jit-token"}),
        ),
        (
            RunnerMode::Persistent,
            serde_json::json!({
                "kind": "github_registration",
                "token": "reg-token",
                "url": "https://github.com/example/repository",
                "name": "runner-1",
                "labels": ["linux"],
                "group": null,
            }),
        ),
    ] {
        let docker = StandIn::new(&directory, "/actions-runner", Docker::V29)?;
        let prepared = prepare_environment(github()?, mode, true)?;
        let environment = PreparedEnvironmentId::try_new(uuid::Uuid::new_v4())?;
        let start = StartRunner {
            prepared_environment_id: environment,
            execution_profile_id: prepared.execution_profile_id,
            profile_revision: prepared.profile_revision,
            bootstrap_reference: BootstrapReference::try_new(uuid::Uuid::new_v4())?,
            bootstrap_expires_at: WireTimestamp::from_millis(gridops_core::now_millis())?,
            resource_request: prepared.resource_request,
            allocation_ids: Vec::new(),
        };
        let response = DockerRuntime::new(None, Some(SOCKET.to_owned()))
            .call(HelperRequest {
                environment_id: Some(environment),
                docker_socket: true,
                prepared: Some(prepared),
                bootstrap: Some(Bootstrap(material.to_string())),
                operation: AgentOperation::StartRunner(start),
            })
            .await;
        ensure!(
            response
                == HelperResponse::Failed {
                    code: FailureCode::Runtime,
                    retryable: true,
                    diagnostic: None,
                },
            "{mode:?}: a Start with no externals cache answered {response:?}"
        );
        ensure!(
            docker.creates()?.is_empty() && docker.calls_of("run")?.is_empty(),
            "{mode:?}: a Start with no cache created something: {:?}",
            docker.calls()?
        );
    }
    Ok(())
}

/// Item 1, A warn 9: Cleanup removes both the runner's work folder and its runner root, and never
/// the externals cache the runner had mounted (other runners of the image share it).
#[tokio::test]
async fn cleanup_removes_the_work_folder_and_the_runner_root_but_never_the_cache() -> Result<()> {
    let Some(directory) =
        stand_in("cleanup_removes_the_work_folder_and_the_runner_root_but_never_the_cache")?
    else {
        return Ok(());
    };
    let docker = StandIn::new(&directory, "/actions-runner", Docker::V29)?;
    let environment = PreparedEnvironmentId::try_new(uuid::Uuid::new_v4())?;
    let work = gridops_core::fleet::shared_work_folder(environment);
    let root = gridops_core::fleet::shared_runner_root(environment);
    let cache = format!("{}/{HEX}", gridops_core::fleet::EXTERNALS_CACHE_ROOT);
    docker.set("containers/runner-1", "runner-1")?;
    docker.set("label", &environment.to_string())?;
    docker.set("container-image", TAG)?;
    docker.set("mounts", &format!("{SOCKET}\n{work}\n{root}\n{cache}\n"))?;

    let resource = LocalResourceIdentity::Docker {
        object_id: UpstreamText::new("runner-1".to_owned())?,
    };
    let response = DockerRuntime::new(None, Some(SOCKET.to_owned()))
        .call(HelperRequest {
            environment_id: Some(environment),
            docker_socket: true,
            prepared: None,
            bootstrap: None,
            operation: AgentOperation::Cleanup {
                expected_resource: resource,
            },
        })
        .await;
    ensure!(
        matches!(response, HelperResponse::Succeeded { .. }),
        "{response:?}"
    );
    let runs = docker.calls_of("run")?;
    ensure!(
        !runs.iter().any(|run| run.contains(&cache)),
        "Cleanup touched the shared externals cache: {runs:?}"
    );
    for folder in [&work, &root] {
        ensure!(
            runs.iter().any(|run| run.contains(&format!("{folder}:"))),
            "Cleanup did not empty {folder}: {runs:?}"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn outcome(result: &std::result::Result<PathBuf, Refusal>) -> String {
    match result {
        Ok(path) => format!("Ok({})", path.display()),
        Err(refusal) => format!("Err({})", describe(refusal)),
    }
}

fn describe(refusal: &Refusal) -> String {
    format!(
        "{:?}, retryable {}: {}",
        refusal.code, refusal.retryable, refusal.reason
    )
}

fn github() -> Result<CiTarget> {
    Ok(CiTarget::GithubRepository {
        installation_id: ExternalId::new(1)?,
        repository_id: ExternalId::new(1)?,
    })
}

fn prepare_environment(
    target: CiTarget,
    mode: RunnerMode,
    docker_socket: bool,
) -> Result<PrepareEnvironment> {
    Ok(PrepareEnvironment {
        execution_profile_id: ProfileId::try_new(uuid::Uuid::new_v4())?,
        profile_revision: Revision::new(1)?,
        workload: PreparedWorkload::CiRunner,
        target,
        runtime_kind: RuntimeKind::Docker,
        execution_os: ExecutionOs::Linux,
        architecture: Architecture::X64,
        mode,
        image: ImageReference::try_from(TAG.to_owned())?,
        labels: Vec::new(),
        configuration: ExecutionConfiguration {
            requires_docker_socket: docker_socket,
            ..ExecutionConfiguration::default()
        },
        resource_request: ResourceRequest::new(1_000, 1_024, 0)?,
        allocation_ids: Vec::new(),
    })
}

/// The registration variables Start sets for a persistent runner.
fn registration() -> Vec<(String, String)> {
    [
        (
            "GRIDOPS_REGISTRATION_URL",
            "https://github.com/example/repository",
        ),
        ("GRIDOPS_RUNNER_NAME", "runner-1"),
        ("GRIDOPS_RUNNER_LABELS", "linux"),
        ("GRIDOPS_RUNNER_GROUP", ""),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}

/// Every value given after any of `flags`, as `--flag value` or `--flag=value`.
fn values(args: &[String], flags: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if flags.contains(&arg.as_str()) {
            if let Some(value) = iter.next() {
                found.push(value.clone());
            }
        } else if let Some((flag, value)) = arg.split_once('=') {
            if flag.starts_with('-') && flags.contains(&flag) {
                found.push(value.to_owned());
            }
        }
    }
    found
}

/// The `-e` / `--env` variables of `args`.
fn env_of(args: &[String]) -> Vec<(String, String)> {
    values(args, &["-e", "--env"])
        .into_iter()
        .filter_map(|variable| {
            variable
                .split_once('=')
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
        })
        .collect()
}

struct Mount {
    bind: bool,
    source: String,
    target: String,
    readonly: bool,
}

fn mounts(args: &[String]) -> Vec<Mount> {
    values(args, &["--mount"])
        .into_iter()
        .map(|spec| {
            let mut mount = Mount {
                bind: false,
                source: String::new(),
                target: String::new(),
                readonly: false,
            };
            for field in spec.split(',') {
                match field.split_once('=') {
                    Some(("type", kind)) => mount.bind = kind == "bind",
                    Some(("source" | "src", source)) => source.clone_into(&mut mount.source),
                    Some(("target" | "destination" | "dst", target)) => {
                        target.clone_into(&mut mount.target);
                    }
                    Some(("readonly" | "ro", value)) => {
                        mount.readonly = matches!(value, "true" | "1");
                    }
                    None if field == "readonly" || field == "ro" => mount.readonly = true,
                    _ => {}
                }
            }
            mount
        })
        .collect()
}

/// The names in `directory`, sorted; none when it does not exist.
fn entries(directory: &Path) -> Result<Vec<String>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(directory)? {
        names.push(entry?.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}

/// Every symlink under `directory` (not followed) whose target does not resolve.
fn dangling_links(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut dangling = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next)? {
            let path = entry?.path();
            let kind = fs::symlink_metadata(&path)?.file_type();
            if kind.is_symlink() {
                if fs::metadata(&path).is_err() {
                    dangling.push(path);
                }
            } else if kind.is_dir() {
                pending.push(path);
            }
        }
    }
    Ok(dangling)
}

/// The directory the stub `stub` logged it ran in.
fn ran_in(log: &str, stub: &str) -> Result<PathBuf> {
    let prefix = format!("{stub} pwd=");
    let line = log
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .context(format!("{stub} did not run:\n{log}"))?;
    let directory = line.split(" args=").next().unwrap_or(line);
    canonical(Path::new(directory))
}

fn canonical(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).context(format!("{}", path.display()))
}

fn scratch(tag: &str) -> Result<PathBuf> {
    let directory =
        std::env::temp_dir().join(format!("gridops-externals-{tag}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&directory)?;
    Ok(directory)
}

/// Whether this machine is safe for a test that drives the real `StateDirectory` paths: not root,
/// and no host agent's `/var/lib/gridops-agent`. Says why when it is not.
fn not_an_agent_host_or_skip() -> Result<bool> {
    let uid = Command::new("id").arg("-u").output()?;
    if String::from_utf8_lossy(&uid.stdout).trim() == "0" {
        eprintln!("skipped: running as root could write to /var/lib/gridops-agent");
        return Ok(false);
    }
    if Path::new("/var/lib/gridops-agent").exists() {
        eprintln!("skipped: /var/lib/gridops-agent exists here (a host agent's StateDirectory)");
        return Ok(false);
    }
    Ok(true)
}

/// The failure decision 5 gives creating `EXTERNALS_CACHE_ROOT` on this machine, found by
/// creating a probe folder in its nearest existing ancestor: `PermissionDenied` is
/// `Configuration, false`, any other error `Runtime, true`. `None` (skip, saying why) when this
/// user could create it, as the Prepare would then write outside the test.
fn cache_root_creation_answer() -> Result<Option<(FailureCode, bool)>> {
    let mut ancestor = Path::new(gridops_core::fleet::EXTERNALS_CACHE_ROOT);
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .context("the cache root has no existing ancestor")?;
    }
    let probe = ancestor.join(format!("gridops-probe-{}", uuid::Uuid::new_v4()));
    match fs::create_dir(&probe) {
        Ok(()) => {
            fs::remove_dir(&probe)?;
            eprintln!(
                "skipped: this user may create folders in {}, so the Prepare could fill a real cache",
                ancestor.display()
            );
            Ok(None)
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            Ok(Some((FailureCode::Configuration, false)))
        }
        Err(_) => Ok(Some((FailureCode::Runtime, true))),
    }
}

/// Make everything under `directory` writable again, so it can be removed.
fn make_writable(directory: &Path) {
    let mut pending = vec![directory.to_path_buf()];
    while let Some(next) = pending.pop() {
        let _ = fs::set_permissions(&next, fs::Permissions::from_mode(0o755));
        if let Ok(read) = fs::read_dir(&next) {
            for entry in read.flatten() {
                let path = entry.path();
                if fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir()) {
                    pending.push(path);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The runner directories for the script tests
// ---------------------------------------------------------------------------------------------

/// An image's runner directory (stub `run.sh` / `config.sh`, `bin`, `externals`), a filled cache,
/// an empty runner root and a work folder, all under one temp dir. The stubs log to `log`.
struct RunnerDirs {
    base: PathBuf,
    image: PathBuf,
    cache: PathBuf,
    root: PathBuf,
    work: PathBuf,
}

static SCRIPT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const RUN_STUB: &str = r#"#!/usr/bin/env bash
{
  echo "run pwd=$PWD"
  echo "run args=$*"
  echo "run configured=$([ -f .runner ] && echo yes || echo no)"
  echo "run tool=${RUNNER_TOOL_CACHE:-}|${AGENT_TOOLSDIRECTORY:-}"
} >> "$GRIDOPS_TEST_LOG"
mkdir -p _diag/pages
printf 'job output from %s\n' "$PWD" > _diag/pages/job.log
sleep 0.3
"#;

const CONFIG_STUB: &str = r#"#!/usr/bin/env bash
echo "config pwd=$PWD args=$*" >> "$GRIDOPS_TEST_LOG"
echo '{}' > .runner
"#;

impl RunnerDirs {
    fn new(tag: &str) -> Result<Self> {
        let base = scratch(&format!("runner-{tag}"))?;
        let dirs = Self {
            image: base.join("image"),
            cache: base.join("externals").join(HEX),
            root: base.join("runners/env"),
            work: base.join("work/env"),
            base,
        };
        fs::create_dir_all(dirs.image.join("bin"))?;
        fs::write(dirs.image.join("bin/Runner.Listener"), "listener")?;
        fs::create_dir_all(dirs.image.join("externals/node24/bin"))?;
        fs::write(dirs.image.join("externals/node24/bin/node"), "image copy")?;
        for (name, stub) in [("run.sh", RUN_STUB), ("config.sh", CONFIG_STUB)] {
            fs::write(dirs.image.join(name), stub)?;
            fs::set_permissions(dirs.image.join(name), fs::Permissions::from_mode(0o755))?;
        }
        fs::create_dir_all(dirs.cache.join("node24/bin"))?;
        fs::write(dirs.cache.join("node24/bin/node"), "cache copy")?;
        // The agent creates both folders before the container starts.
        fs::create_dir_all(&dirs.root)?;
        fs::create_dir_all(&dirs.work)?;
        Ok(dirs)
    }

    /// Runs the generated script as the container would: from the image's directory, with the
    /// image's ENV and the secret on stdin. A shared runner also gets the `-e` variables of
    /// `shared_socket_args`.
    fn run(
        &self,
        mode: RunnerMode,
        shared: bool,
        secret: &str,
        extra: &[(String, String)],
    ) -> Result<std::process::Output> {
        // The shared runner script keeps its log-forwarding offsets in a fixed
        // `/tmp/gridops-forwarded-job-state` (one per container in production), so two scripts
        // running at once in this process would read each other's offsets.
        let _one_at_a_time = SCRIPT_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut command = Command::new("bash");
        command
            .args(["-c", &github_runner_script(mode, shared)])
            .current_dir(&self.image)
            .env("GRIDOPS_TEST_LOG", self.base.join("log"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for variable in [
            "GRIDOPS_RUNNER_ROOT",
            "GRIDOPS_RUNNER_EXTERNALS",
            "GRIDOPS_RUNNER_WORK",
            "RUNNER_TOOL_CACHE",
        ] {
            command.env_remove(variable);
        }
        // The pool image's own ENV (brief, "Facts established").
        command.env("AGENT_TOOLSDIRECTORY", "/opt/hostedtoolcache");
        if shared {
            for (key, value) in env_of(&shared_socket_args(
                SOCKET,
                &self.work,
                &self.root,
                &self.cache,
            )) {
                command.env(key, value);
            }
        }
        for (key, value) in extra {
            command.env(key, value);
        }
        let mut child = command.spawn()?;
        child
            .stdin
            .take()
            .context("no stdin")?
            .write_all(secret.as_bytes())?;
        Ok(child.wait_with_output()?)
    }

    fn log(&self) -> Result<String> {
        Ok(fs::read_to_string(self.base.join("log")).unwrap_or_default())
    }

    /// The root holds the runner (a real `bin`, its scripts) and an `externals` that is a link to
    /// the cache, not a copy of the image's.
    fn root_is_a_runner_linked_to_the_cache(&self) -> Result<()> {
        let bin = fs::symlink_metadata(self.root.join("bin"))?;
        ensure!(
            bin.is_dir()
                && fs::read_to_string(self.root.join("bin/Runner.Listener"))? == "listener",
            "bin was not copied into the runner root"
        );
        ensure!(
            self.root.join("run.sh").is_file(),
            "run.sh was not copied into the runner root"
        );
        let externals = self.root.join("externals");
        ensure!(
            fs::symlink_metadata(&externals)?.file_type().is_symlink(),
            "the runner root's externals is not a link"
        );
        ensure!(
            canonical(&externals)? == canonical(&self.cache)?
                && fs::read_to_string(externals.join("node24/bin/node"))? == "cache copy",
            "the runner root's externals does not lead to the cache"
        );
        Ok(())
    }

    fn image_is_untouched(&self) -> Result<bool> {
        Ok(fs::symlink_metadata(self.image.join("externals"))?.is_dir()
            && fs::read_to_string(self.image.join("externals/node24/bin/node"))? == "image copy")
    }

    fn remove(&self) -> Result<()> {
        make_writable(&self.base);
        fs::remove_dir_all(&self.base)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// The stand-in `docker`
// ---------------------------------------------------------------------------------------------

const STAND_IN_ENV: &str = "GRIDOPS_TEST_EXTERNALS_DOCKER";

/// A `docker` CLI whose state lives in files beside it:
/// - `tag`, `id`: the image the tests fill from, and its `{{.Id}}`;
/// - `workdir` (its `WorkingDir`, may be empty), `version` (`29` or `legacy`: how an image without
///   a `WorkingDir` answers the template);
/// - `fs/`: the image's file system, which `docker cp` copies from;
/// - `present`: other images' ids (hex, one per line) still in the local store;
/// - `fail`: `create`, `cp`, `rm`, `inspect-tag` (the image does not answer) or `inspect-other`
///   (no other image or object answers) fail as an unreachable daemon would;
/// - `containers/<name>`: each existing container, holding its id; `label`, `container-image`,
///   `mounts`: what `inspect` says about a runner container.
///
/// It records `calls` (one line per call), `creates/<n>` (each `create`'s arguments,
/// NUL-separated), `runs` and `unsupported`.
const STAND_IN: &str = r#"#!/usr/bin/env bash
d=$(cd "$(dirname "$0")" && pwd)
printf '%s\n' "$*" >> "$d/calls"
mkdir -p "$d/containers" "$d/creates"
fail=$(cat "$d/fail" 2>/dev/null)
tag=$(cat "$d/tag")
id=$(cat "$d/id")

daemon() { echo 'Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?' >&2; exit 1; }
unsupported() { printf '%s\n' "$*" >> "$d/unsupported"; echo "stand-in docker: unsupported: $*" >&2; exit 2; }
is_tag() { [ "$1" = "$tag" ] || [ "$1" = "$id" ] || [ "$1" = "${id#sha256:}" ]; }

# The name of the container a reference (name or id) names.
container() {
  [ -n "$1" ] || return 1
  if [ -f "$d/containers/$1" ]; then echo "$1"; return 0; fi
  for f in "$d"/containers/*; do
    if [ -f "$f" ] && [ "$(cat "$f")" = "$1" ]; then basename "$f"; return 0; fi
  done
  return 1
}

# The id of the present image a reference names.
image() {
  local hex=${1#sha256:}
  if is_tag "$1"; then echo "$id"; return 0; fi
  if [ -n "$hex" ] && grep -qx -- "$hex" "$d/present" 2>/dev/null; then echo "sha256:$hex"; return 0; fi
  return 1
}

image_template() { # template image-id
  local t=$1 iid=$2 wd=/actions-runner
  if [ "$iid" = "$id" ]; then wd=$(cat "$d/workdir"); fi
  t=${t//'{{.Id}}'/"$iid"}
  t=${t//'{{"\n"}}'/$'\n'}
  if [ -z "$wd" ] && [ "$(cat "$d/version")" != legacy ]; then
    case "$t" in *'.Config.WorkingDir'*)
      echo 'template parsing error: template: :1:17: executing "" at <.Config.WorkingDir>: map has no entry for key "WorkingDir"' >&2
      exit 1 ;;
    esac
    t=${t//'{{index .Config "WorkingDir"}}'/'<no value>'}
    t=${t//'{{json .Config}}'/'{"Env":["PATH=/usr/bin:/bin"]}'}
  else
    t=${t//'{{.Config.WorkingDir}}'/"$wd"}
    t=${t//'{{json .Config.WorkingDir}}'/"\"$wd\""}
    t=${t//'{{index .Config "WorkingDir"}}'/"$wd"}
    t=${t//'{{json .Config}}'/"{\"Env\":[\"PATH=/usr/bin:/bin\"],\"WorkingDir\":\"$wd\"}"}
  fi
  case "$t" in *'{{'*) unsupported "image template $1" ;; esac
  printf '%s\n' "$t"
}

container_template() { # template name
  local t=$1 cid mounts= m
  cid=$(cat "$d/containers/$2")
  while IFS= read -r m; do mounts+="$m"$'\n'; done < <(cat "$d/mounts" 2>/dev/null)
  t=${t//'{{range .Mounts}}{{.Source}}{{"\n"}}{{end}}'/"$mounts"}
  t=${t//'{{.Id}}'/"$cid"}
  t=${t//'{{index .Config.Labels "io.gridops.fleet.environment"}}'/"$(cat "$d/label" 2>/dev/null)"}
  t=${t//'{{.Config.Image}}'/"$(cat "$d/container-image" 2>/dev/null)"}
  t=${t//'{{.State.Running}}'/false}
  t=${t//'{{"\n"}}'/$'\n'}
  case "$t" in *'{{'*) unsupported "container template $1" ;; esac
  printf '%s\n' "$t"
}

inspect_cmd() { # kind args...
  local type=$1 fmt= refs=() ref iid name wd; shift
  while [ $# -gt 0 ]; do
    case "$1" in
      -f|--format) fmt=$2; shift 2 ;;
      --format=*) fmt=${1#--format=}; shift ;;
      --type) type=$2; shift 2 ;;
      --type=*) type=${1#--type=}; shift ;;
      -*) shift ;;
      *) refs+=("$1"); shift ;;
    esac
  done
  for ref in "${refs[@]}"; do
    if is_tag "$ref"; then
      [ "$fail" = inspect-tag ] && daemon
    elif ! container "$ref" >/dev/null; then
      [ "$fail" = inspect-other ] && daemon
    fi
    if [ "$type" != container ] && iid=$(image "$ref"); then
      if [ -n "$fmt" ]; then image_template "$fmt" "$iid"; continue; fi
      wd=/actions-runner; [ "$iid" = "$id" ] && wd=$(cat "$d/workdir")
      if [ -z "$wd" ] && [ "$(cat "$d/version")" != legacy ]; then
        echo "[{\"Id\":\"$iid\",\"Config\":{\"Env\":[\"PATH=/usr/bin:/bin\"]}}]"
      else
        echo "[{\"Id\":\"$iid\",\"Config\":{\"Env\":[\"PATH=/usr/bin:/bin\"],\"WorkingDir\":\"$wd\"}}]"
      fi
    elif [ "$type" != image ] && name=$(container "$ref"); then
      if [ -n "$fmt" ]; then container_template "$fmt" "$name"; continue; fi
      echo "[{\"Id\":\"$(cat "$d/containers/$name")\",\"Name\":\"/$name\"}]"
    elif [ "$type" = image ]; then
      echo "Error response from daemon: No such image: $ref" >&2; exit 1
    else
      echo "error: no such object: $ref" >&2; exit 1
    fi
  done
}

create_cmd() {
  [ "$fail" = create ] && daemon
  local name= prev= a n cid
  for a in "$@"; do
    [ "$prev" = --name ] && name=$a
    case "$a" in --name=*) name=${a#--name=} ;; esac
    prev=$a
  done
  if [ -n "$name" ] && [ -f "$d/containers/$name" ]; then
    echo "Error response from daemon: Conflict. The container name \"/$name\" is already in use by container \"$(cat "$d/containers/$name")\". You have to remove (or rename) that container to be able to reuse that name." >&2
    exit 1
  fi
  n=$(ls "$d/creates" | wc -l | tr -d ' ')
  printf '%s\0' "$@" > "$d/creates/$n"
  cid="f111c0ffee$n"
  echo "$cid" > "$d/containers/${name:-$cid}"
  echo "$cid"
}

# Copies a directory's contents, links kept as links.
copy_tree() { mkdir -p "$2" && (cd "$1" && tar cf - .) | (cd "$2" && tar xf -); }

cp_cmd() {
  [ "$fail" = cp ] && { echo 'Error response from daemon: write /var/lib/docker/tmp/docker-export: no space left on device' >&2; exit 1; }
  local follow=false args=() a from dest ref path src out
  for a in "$@"; do
    case "$a" in
      -L|--follow-link) follow=true ;;
      -a|--archive|-q|--quiet) ;;
      -*) unsupported "cp $*" ;;
      *) args+=("$a") ;;
    esac
  done
  [ ${#args[@]} -eq 2 ] || unsupported "cp $*"
  from=${args[0]} dest=${args[1]}
  case "$from" in *:*) ;; *) unsupported "cp into a container: $*" ;; esac
  ref=${from%%:*} path=${from#*:}
  container "$ref" >/dev/null || { echo "Error response from daemon: No such container: $ref" >&2; exit 1; }
  src="$d/fs$path"
  if [ ! -e "$src" ] && [ ! -L "$src" ]; then
    echo "Error response from daemon: Could not find the file $path in container $ref" >&2; exit 1
  fi
  case "$path" in
    */.|*/./) copy_tree "$src" "$dest" || exit 1; exit 0 ;;
  esac
  if [ -d "$dest" ] && [ ! -L "$dest" ]; then out="$dest/$(basename "$path")"; else out=$dest; fi
  [ -d "$(dirname "$out")" ] || { echo "Error: invalid output path: directory $(dirname "$out") does not exist" >&2; exit 1; }
  if [ -L "$src" ] && ! $follow; then
    ln -s "$(readlink "$src")" "$out" || exit 1
  elif [ -d "$src" ]; then
    copy_tree "$src" "$out" || exit 1
  else
    cp "$src" "$out" || exit 1
  fi
}

rm_cmd() {
  [ "$fail" = rm ] && daemon
  local force=false refs=() a ref name
  for a in "$@"; do
    case "$a" in
      --force) force=true ;;
      --volumes) ;;
      -*f*) force=true ;;
      -v) ;;
      -*) unsupported "rm $*" ;;
      *) refs+=("$a") ;;
    esac
  done
  for ref in "${refs[@]}"; do
    if name=$(container "$ref"); then
      rm -f "$d/containers/$name"; echo "$ref"
    elif ! $force; then
      echo "Error response from daemon: No such container: $ref" >&2; exit 1
    fi
  done
}

cmd=$1; shift
kind=
case "$cmd" in image|container) kind=$cmd; cmd=$1; shift ;; esac
case "$cmd" in
  inspect) inspect_cmd "$kind" "$@" ;;
  create) create_cmd "$@" ;;
  cp) cp_cmd "$@" ;;
  rm) rm_cmd "$@" ;;
  run) printf '%s\n' "$*" >> "$d/runs" ;;
  pull) ;;
  *) unsupported "$cmd $*" ;;
esac
"#;

/// How Docker answers `{{.Config.WorkingDir}}` for an image without one.
#[derive(Clone, Copy)]
enum Docker {
    /// A template error ("map has no entry for key").
    V29,
    /// An empty string.
    Legacy,
}

struct StandIn {
    directory: PathBuf,
}

impl StandIn {
    /// Resets the stand-in to one image, `TAG` with id `sha256:HEX` and `workdir`, an empty file
    /// system, no other image, no container and no failure.
    fn new(directory: &Path, workdir: &str, version: Docker) -> Result<Self> {
        for state in ["fs", "containers", "creates"] {
            let path = directory.join(state);
            if path.exists() {
                make_writable(&path);
                fs::remove_dir_all(&path)?;
            }
        }
        for state in [
            "calls",
            "runs",
            "present",
            "fail",
            "mounts",
            "label",
            "container-image",
        ] {
            let _ = fs::remove_file(directory.join(state));
        }
        let stand_in = Self {
            directory: directory.to_path_buf(),
        };
        stand_in.set("tag", TAG)?;
        stand_in.set("id", &format!("sha256:{HEX}"))?;
        stand_in.set("workdir", workdir)?;
        stand_in.set(
            "version",
            match version {
                Docker::V29 => "29",
                Docker::Legacy => "legacy",
            },
        )?;
        fs::create_dir_all(stand_in.fs())?;
        fs::create_dir_all(directory.join("containers"))?;
        Ok(stand_in)
    }

    fn set(&self, file: &str, content: &str) -> Result<()> {
        let path = self.directory.join(file);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        Ok(())
    }

    fn fs(&self) -> PathBuf {
        self.directory.join("fs")
    }

    /// The image's runner directory at `workdir`, with `externals` holding node and a relative
    /// link inside it.
    fn externals(&self, workdir: &str) -> Result<()> {
        let runner = self.fs().join(workdir.trim_start_matches('/'));
        fs::create_dir_all(runner.join("bin"))?;
        let externals = runner.join("externals");
        fs::create_dir_all(externals.join("node24/bin"))?;
        fs::create_dir_all(externals.join("node24/lib"))?;
        fs::write(
            externals.join("node24/bin/node"),
            format!("node24 from {workdir}"),
        )?;
        fs::write(externals.join("node24/lib/npm-cli.js"), "npm")?;
        std::os::unix::fs::symlink("../lib/npm-cli.js", externals.join("node24/bin/npm"))?;
        Ok(())
    }

    /// A fill container left behind by an agent that crashed mid-fill.
    fn crashed_fill_container(&self) -> Result<()> {
        self.set(&format!("containers/gridops-externals-{HEX}"), "deadbeef")
    }

    fn calls(&self) -> Result<Vec<String>> {
        Ok(fs::read_to_string(self.directory.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect())
    }

    /// The calls of `command` (with or without the `container` / `image` group).
    fn calls_of(&self, command: &str) -> Result<Vec<String>> {
        Ok(self
            .calls()?
            .into_iter()
            .filter(|call| {
                let mut words = call.split(' ');
                let first = words.next();
                first == Some(command)
                    || (matches!(first, Some("container" | "image"))
                        && words.next() == Some(command))
            })
            .collect())
    }

    fn creates(&self) -> Result<Vec<Vec<String>>> {
        let mut creates = Vec::new();
        for n in 0.. {
            let Ok(bytes) = fs::read(self.directory.join(format!("creates/{n}"))) else {
                break;
            };
            creates.push(
                bytes
                    .split(|byte| *byte == 0)
                    .filter(|arg| !arg.is_empty())
                    .map(|arg| String::from_utf8_lossy(arg).into_owned())
                    .collect(),
            );
        }
        Ok(creates)
    }

    fn containers(&self) -> Result<Vec<String>> {
        entries(&self.directory.join("containers"))
    }
}

/// In the rerun with the stand-in first on `PATH`: its directory. Otherwise reruns this binary's
/// test `test` that way and answers `None`.
fn stand_in(test: &str) -> Result<Option<PathBuf>> {
    if let Some(directory) = std::env::var_os(STAND_IN_ENV) {
        return Ok(Some(PathBuf::from(directory)));
    }
    // The runtime runs `docker` from PATH, which a test can't change for its own process, so it
    // reruns itself with the stand-in first.
    let directory = scratch("docker")?;
    let script = directory.join("docker");
    fs::write(&script, STAND_IN)?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
    let module = module_path!()
        .split_once("::")
        .map_or(module_path!(), |(_, path)| path);
    let output = Command::new(std::env::current_exe()?)
        .args(["--exact", &format!("{module}::{test}"), "--nocapture"])
        .env(
            "PATH",
            format!("{}:{}", directory.display(), std::env::var("PATH")?),
        )
        .env(STAND_IN_ENV, &directory)
        .output()?;
    let unsupported = fs::read_to_string(directory.join("unsupported")).unwrap_or_default();
    let calls = fs::read_to_string(directory.join("calls")).unwrap_or_default();
    make_writable(&directory);
    fs::remove_dir_all(&directory)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    ensure!(
        output.status.success() && stdout.contains("1 passed"),
        "{test} failed with the stand-in Docker\n--- stdout\n{stdout}\n--- stderr\n{}\n--- docker calls the stand-in could not answer\n{unsupported}\n--- docker calls\n{calls}",
        String::from_utf8_lossy(&output.stderr),
    );
    Ok(None)
}

/// Guards the stand-in itself: it answers with the messages Docker 29.8 gave for the same calls
/// (checked against a real daemon while writing these tests), which the runtime's error handling
/// reads.
#[test]
fn the_stand_in_answers_like_docker_29() -> Result<()> {
    let directory = scratch("stand-in-check")?;
    let script = directory.join("docker");
    fs::write(&script, STAND_IN)?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
    let stand_in = StandIn::new(&directory, "", Docker::V29)?;
    let run = |args: &[&str]| -> Result<(bool, String, String)> {
        let output = Command::new(&script).args(args).output()?;
        Ok((
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    };
    let (ok, _, error) = run(&[
        "image",
        "inspect",
        "-f",
        "{{.Id}} {{.Config.WorkingDir}}",
        TAG,
    ])?;
    ensure!(!ok && error.contains("map has no entry for key"), "{error}");
    let (ok, out, _) = run(&["image", "inspect", "-f", "{{.Id}}", TAG])?;
    ensure!(ok && out.trim() == format!("sha256:{HEX}"), "{out}");
    let (ok, _, error) = run(&["image", "inspect", HEX_B])?;
    ensure!(!ok && error.contains("No such image"), "{error}");
    let (ok, _, error) = run(&["inspect", HEX_B])?;
    ensure!(!ok && error.contains("no such object"), "{error}");
    let (ok, _, _) = run(&["rm", "-f", "nothing-here"])?;
    ensure!(ok, "docker rm -f of a missing container succeeds");
    stand_in.set("containers/fill", "f111")?;
    let (ok, _, error) = run(&["cp", "fill:/actions-runner/externals", "/nonexistent"])?;
    ensure!(
        !ok && error
            .contains("Could not find the file /actions-runner/externals in container fill"),
        "{error}"
    );
    make_writable(&directory);
    fs::remove_dir_all(&directory)?;
    Ok(())
}
