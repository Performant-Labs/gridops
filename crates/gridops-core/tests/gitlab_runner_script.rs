//! The GitLab runner script and its inputs (#147), which the fleet host
//! agent's Docker runtime and the runner manager both launch: one script in
//! `gridops_core::provisioning`, fed the token on stdin and every other input
//! through fixed-name environment variables.
//!
//! The script runs here for real, in `bash`, against a local HTTP server
//! standing in for GitLab's `POST /api/v4/runners/verify` (the real `curl`
//! talks to it, through a wrapper that records its arguments and
//! environment) and a stand-in `gitlab-runner` that records how it was
//! called and then behaves like `run-single` does at the end of a run
//! (`commands/single.go`, `checkFinishedConditions`).

use std::{
    collections::BTreeSet,
    fs,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context as _, Result, ensure};
use gridops_core::provisioning::{
    GITLAB_DEFAULT_JOB_IMAGE, GITLAB_EXIT_COMPLETED, GITLAB_EXIT_IDLE_EXPIRED, GITLAB_EXIT_REASONS,
    GITLAB_EXIT_REGISTRATION_REFUSED, GITLAB_EXIT_VERIFY_UNAVAILABLE, GITLAB_JOB_RECEIVED_SIGNAL,
    GITLAB_READY_MARKER, GITLAB_RUNNER_IMAGE, GITLAB_START_READY_TIMEOUT_SECS,
    GITLAB_VERIFY_MAX_TIME_SECS, gitlab_runner_env, gitlab_runner_script, valid_gitlab_url,
};

const TOKEN: &str = "glrt-script-test-token-0001";
const WAIT_TIMEOUT: u32 = 600;
/// What `run-single` logs when `--wait-timeout` passes without a job
/// (gitlab-runner `commands/single.go`, `checkFinishedConditions`).
const IDLE_LINE: &str = "This runner has not received a job in 600 seconds, so now exiting";

// ---------------------------------------------------------------------------
// Pinned values
// ---------------------------------------------------------------------------

/// B-1: both images are pinned releases, never a moving tag. The runner
/// image is the official one, Ubuntu-based (`vX.Y.Z` or `ubuntu-vX.Y.Z`; the
/// `alpine*` and `ubi-fips` variants lack the script's tools or differ).
#[test]
fn the_gitlab_images_are_pinned_releases() -> Result<()> {
    let tag = GITLAB_RUNNER_IMAGE
        .strip_prefix("gitlab/gitlab-runner:")
        .context(format!("not the official image: {GITLAB_RUNNER_IMAGE}"))?;
    let version = tag.strip_prefix("ubuntu-").unwrap_or(tag);
    let numbers = version
        .strip_prefix('v')
        .context(format!("not a release tag: {GITLAB_RUNNER_IMAGE}"))?
        .split('.')
        .collect::<Vec<_>>();
    ensure!(
        numbers.len() == 3
            && numbers
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())),
        "not an exact Ubuntu-based release tag: {GITLAB_RUNNER_IMAGE}"
    );

    let (name, tag) = GITLAB_DEFAULT_JOB_IMAGE.rsplit_once(':').context(format!(
        "the job image has no tag: {GITLAB_DEFAULT_JOB_IMAGE}"
    ))?;
    ensure!(
        !name.is_empty()
            && !tag.contains('/')
            && tag != "latest"
            && tag.bytes().any(|byte| byte.is_ascii_digit()),
        "the job image must be pinned to a version tag: {GITLAB_DEFAULT_JOB_IMAGE}"
    );
    Ok(())
}

/// R2 B-2: create, the stdin feed and the readiness wait must finish inside
/// the reconciler's 30 s start deadline less its 5 s delivery margin, and the
/// verify call must give up before the wait does, so a slow GitLab ends the
/// Start with the script's own exit rather than the agent's timeout.
#[test]
fn the_gitlab_start_waits_fit_inside_the_start_deadline() {
    assert_eq!(GITLAB_START_READY_TIMEOUT_SECS, 15);
    assert_eq!(GITLAB_VERIFY_MAX_TIME_SECS, 10);
    assert!(u64::from(GITLAB_VERIFY_MAX_TIME_SECS) < GITLAB_START_READY_TIMEOUT_SECS);
}

/// R2 B-2: a GitLab that can't be asked is a retryable runtime failure, so
/// its exit status must not read as any ending the listing reports, nor as
/// the status any failing command under `set -e` gives (1, 2), nor as one
/// the shell reserves (126 and up).
#[test]
fn verify_unavailable_has_its_own_exit_status() {
    let reported = GITLAB_EXIT_REASONS.map(|(status, _)| status);
    assert!(
        !reported.contains(&GITLAB_EXIT_VERIFY_UNAVAILABLE),
        "{GITLAB_EXIT_VERIFY_UNAVAILABLE} is a reported ending"
    );
    assert!(
        (3..=125).contains(&GITLAB_EXIT_VERIFY_UNAVAILABLE),
        "{GITLAB_EXIT_VERIFY_UNAVAILABLE} is not a distinct script exit status"
    );
}

/// D10 and brief review W-2: one check, shared by the manager and the fleet
/// Start, that the GitLab URL is an http(s) URL.
#[test]
fn only_http_and_https_gitlab_urls_are_valid() {
    for url in [
        "https://gitlab.com",
        "https://gitlab.example.com/",
        "http://gitlab.internal:8080",
        "https://example.com/gitlab",
    ] {
        assert!(valid_gitlab_url(url), "{url} should be valid");
    }
    for url in [
        "",
        "gitlab.example.com",
        "ftp://gitlab.example.com",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "https://",
        "not a url",
    ] {
        assert!(!valid_gitlab_url(url), "{url:?} should be refused");
    }
}

/// R2 W-3 and A's warning 2: the script's non-secret inputs are built in one
/// place, so neither caller can misspell a name the script reads. Every value
/// the script needs is there, and the script reads every name it is given.
#[test]
fn the_runner_env_carries_the_script_inputs_and_the_script_reads_them() -> Result<()> {
    let url = "https://gitlab.example.com";
    let env = gitlab_runner_env(url, WAIT_TIMEOUT);
    let script = gitlab_runner_script();
    let mut names = BTreeSet::new();
    let mut values = BTreeSet::new();
    for variable in &env {
        let (name, value) = variable
            .split_once('=')
            .context(format!("not NAME=value: {variable}"))?;
        ensure!(names.insert(name), "{name} is set twice: {env:?}");
        values.insert(value);
        ensure!(
            script.contains(&format!("${name}")) || script.contains(&format!("${{{name}")),
            "the script never reads {name}"
        );
    }
    for needed in [url, "600", GITLAB_DEFAULT_JOB_IMAGE] {
        ensure!(values.contains(needed), "{needed} is missing from {env:?}");
    }
    // The script text itself is constant: nothing is interpolated into it.
    ensure!(!script.contains(url) && !script.contains(TOKEN));
    Ok(())
}

// ---------------------------------------------------------------------------
// The script, run
// ---------------------------------------------------------------------------

/// A verified token: the script says it is ready, then runs `run-single`
/// with the pinned flags, the token reaching the runner but no other
/// process's arguments or environment, and exits 0 once the job ran.
#[test]
fn a_verified_runner_says_it_is_ready_and_runs_one_job() -> Result<()> {
    let run = run_script(&Scenario::verified(
        &format!("Checking for jobs... received\nJob succeeded\n{GITLAB_JOB_RECEIVED_SIGNAL}\n"),
        0,
    ))?;
    ensure!(
        run.status == Some(GITLAB_EXIT_COMPLETED),
        "completed run exited {:?}\n{run}",
        run.status
    );
    ensure!(
        run.stdout.lines().any(|line| line == GITLAB_READY_MARKER),
        "no readiness marker line on stdout\n{run}"
    );
    // gitlab-runner's own output reaches the container's output.
    ensure!(
        run.output().contains("Job succeeded"),
        "the runner's output was swallowed\n{run}"
    );

    // NV-4: the token went to GitLab's verify endpoint as form data.
    let [verify] = run.verify_requests.as_slice() else {
        anyhow::bail!("expected one verify request\n{run}");
    };
    ensure!(verify.method == "POST" && verify.path == "/api/v4/runners/verify");
    ensure!(
        form_value(&verify.body, "token").as_deref() == Some(TOKEN),
        "verify body {:?}",
        verify.body
    );
    // ... from stdin: curl got it in neither its arguments nor environment,
    // so the script never exported it, and it is bounded by the pinned
    // `--max-time`.
    let [curl] = run.curl_calls.as_slice() else {
        anyhow::bail!("expected one curl call\n{run}");
    };
    ensure!(
        !curl.argv.iter().any(|arg| arg.contains(TOKEN)) && !curl.env.contains(TOKEN),
        "the token reached curl's arguments or environment: {curl:?}"
    );
    let max_time = GITLAB_VERIFY_MAX_TIME_SECS.to_string();
    ensure!(
        flag_value(&curl.argv, &["--max-time", "-m"]).as_deref() == Some(max_time.as_str()),
        "curl is not bounded by --max-time {max_time}: {:?}",
        curl.argv
    );

    // NV-1: `run-single` with the brief's flags.
    let [runner] = run.runner_calls.as_slice() else {
        anyhow::bail!("expected one gitlab-runner call\n{run}");
    };
    ensure!(
        runner.argv.first().map(String::as_str) == Some("run-single"),
        "{:?}",
        runner.argv
    );
    for (flag, value) in [
        ("--url", run.url.as_str()),
        ("--executor", "docker"),
        ("--docker-image", GITLAB_DEFAULT_JOB_IMAGE),
        ("--max-builds", "1"),
        ("--wait-timeout", "600"),
    ] {
        ensure!(
            flag_value(&runner.argv, &[flag]).as_deref() == Some(value),
            "run-single lacks {flag} {value}: {:?}",
            runner.argv
        );
    }
    ensure!(
        runner
            .argv
            .iter()
            .any(|arg| arg == "--docker-disable-cache"),
        "run-single lacks --docker-disable-cache: {:?}",
        runner.argv
    );
    // NV-2: the token reached the runner by whatever mechanism the script
    // chose (an argument, an environment variable or a config file).
    ensure!(
        runner.seen.contains(TOKEN),
        "the token never reached gitlab-runner: {runner:?}"
    );
    Ok(())
}

/// No job within `--wait-timeout`: `run-single` exits 0 all the same, and
/// only its output tells. The script exits idle-expired.
#[test]
fn a_runner_that_never_got_a_job_exits_idle_expired() -> Result<()> {
    let run = run_script(&Scenario::verified(&format!("{IDLE_LINE}\n"), 0))?;
    ensure!(
        run.status == Some(GITLAB_EXIT_IDLE_EXPIRED),
        "exited {:?}\n{run}",
        run.status
    );
    ensure!(
        run.stdout.lines().any(|line| line == GITLAB_READY_MARKER),
        "{run}"
    );
    Ok(())
}

/// A runner that fails on its own (here `run-single`'s fatal exit on a bad
/// configuration) is neither completed nor idle nor refused.
#[test]
fn a_runner_that_fails_by_itself_reports_no_runner_ending() -> Result<()> {
    let run = run_script(&Scenario::verified("FATAL: Missing Executor\n", 1))?;
    ensure!(
        run.status
            .is_some_and(|status| status != 0 && !is_reported(status)),
        "exited {:?}\n{run}",
        run.status
    );
    Ok(())
}

/// `run-single` exiting with a status the script itself gives a meaning to
/// (69, 75, 77, or 78, which the Tart agent's script exits with for an
/// image without `gitlab-runner`, #173) must not pass for that meaning: the
/// script exits with another status, outside `GITLAB_EXIT_REASONS`. Without
/// that, the manager, which maps a Docker container's exit status through
/// the same table, would list such a runner as idle, refused or missing.
#[test]
fn a_runner_exiting_with_a_status_of_the_scripts_own_leaves_the_table() -> Result<()> {
    for own in [69, 75, 77, 78] {
        let run = run_script(&Scenario::verified("FATAL: the fake runner stopped\n", own))?;
        ensure!(
            run.status
                .is_some_and(|status| status != own && status != 0 && !is_reported(status)),
            "run-single exiting {own}: the script exited {:?}\n{run}",
            run.status
        );
    }
    Ok(())
}

/// GitLab refuses the token (403): registration refused, before anything
/// says ready and without starting the runner.
#[test]
fn a_refused_token_exits_registration_refused_without_running() -> Result<()> {
    let run = run_script(&Scenario::answered(403))?;
    ensure!(
        run.status == Some(GITLAB_EXIT_REGISTRATION_REFUSED),
        "exited {:?}\n{run}",
        run.status
    );
    ensure!(!run.stdout.contains(GITLAB_READY_MARKER), "{run}");
    ensure!(run.runner_calls.is_empty(), "{run}");
    Ok(())
}

/// GitLab answers neither yes nor no (an error status, or nothing listens):
/// the distinct verify-unavailable status, never ready, never run.
#[test]
fn a_gitlab_that_cannot_answer_exits_verify_unavailable() -> Result<()> {
    for scenario in [Scenario::answered(500), Scenario::unreachable()] {
        let run = run_script(&scenario)?;
        ensure!(
            run.status == Some(GITLAB_EXIT_VERIFY_UNAVAILABLE),
            "{}: exited {:?}\n{run}",
            scenario.name,
            run.status
        );
        ensure!(
            !run.stdout.contains(GITLAB_READY_MARKER),
            "{}\n{run}",
            scenario.name
        );
        ensure!(run.runner_calls.is_empty(), "{}\n{run}", scenario.name);
    }
    Ok(())
}

/// No token on stdin: nothing is asked of GitLab and nothing runs.
#[test]
fn an_empty_token_runs_nothing() -> Result<()> {
    let mut scenario = Scenario::verified("", 0);
    scenario.stdin = String::new();
    let run = run_script(&scenario)?;
    ensure!(
        run.status
            .is_some_and(|status| status != 0 && !is_reported(status)),
        "exited {:?}\n{run}",
        run.status
    );
    ensure!(
        run.curl_calls.is_empty() && run.runner_calls.is_empty(),
        "{run}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn is_reported(status: i32) -> bool {
    GITLAB_EXIT_REASONS.iter().any(|(code, _)| *code == status)
}

/// `--flag value` or `--flag=value`, for the first of `flags` present.
fn flag_value(argv: &[String], flags: &[&str]) -> Option<String> {
    argv.iter().enumerate().find_map(|(index, arg)| {
        flags.iter().find_map(|flag| {
            if arg == flag {
                argv.get(index + 1).cloned()
            } else {
                arg.strip_prefix(&format!("{flag}=")).map(str::to_owned)
            }
        })
    })
}

/// The decoded value of `name` in an `application/x-www-form-urlencoded`
/// body.
fn form_value(body: &[u8], name: &str) -> Option<String> {
    let body = String::from_utf8_lossy(body);
    body.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (decode(key) == name).then(|| decode(value))
    })
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' {
            if let Some(decoded) = text
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                out.push(decoded);
                index += 3;
                continue;
            }
        }
        out.push(if byte == b'+' { b' ' } else { byte });
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

struct Scenario {
    name: &'static str,
    /// The verify endpoint's status; `None`: nothing listens.
    verify_status: Option<u16>,
    stdin: String,
    /// What the stand-in `gitlab-runner` logs (to stderr, as logrus does).
    runner_output: String,
    runner_exit: i32,
}

impl Scenario {
    fn verified(runner_output: &str, runner_exit: i32) -> Self {
        Self {
            name: "verified",
            verify_status: Some(200),
            stdin: format!("{TOKEN}\n"),
            runner_output: format!(
                "Runtime platform                                    arch=amd64 os=linux\n{runner_output}"
            ),
            runner_exit,
        }
    }

    fn answered(status: u16) -> Self {
        Self {
            name: if status == 403 {
                "refused"
            } else {
                "error status"
            },
            verify_status: Some(status),
            ..Self::verified("", 0)
        }
    }

    fn unreachable() -> Self {
        Self {
            name: "unreachable",
            verify_status: None,
            ..Self::verified("", 0)
        }
    }
}

#[derive(Debug)]
struct VerifyRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

/// One recorded call of a stand-in program.
#[derive(Debug, Default)]
struct Call {
    argv: Vec<String>,
    env: String,
    /// Everything the program could have read the token from: its arguments,
    /// environment, stdin and any `--config` file.
    seen: String,
}

struct Run {
    url: String,
    status: Option<i32>,
    stdout: String,
    stderr: String,
    verify_requests: Vec<VerifyRequest>,
    curl_calls: Vec<Call>,
    runner_calls: Vec<Call>,
}

impl Run {
    fn output(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

impl std::fmt::Display for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "--- stdout\n{}--- stderr\n{}--- verify {:?}\n--- curl {:?}\n--- gitlab-runner {:?}",
            self.stdout, self.stderr, self.verify_requests, self.curl_calls, self.runner_calls
        )
    }
}

/// Records each call: its argv one per line, then `--env` and its
/// environment, then whatever else it can see, then `--end`.
const RUNNER_STAND_IN: &str = r#"#!/bin/sh
d=$(dirname "$0")
{
  echo '--argv'
  for a in "$@"; do printf '%s\n' "$a"; done
  echo '--env'
  env
  echo '--seen'
  prev=
  for a in "$@"; do
    case "$prev" in -c|--config) cat "$a" 2>/dev/null ;; esac
    case "$a" in --config=*) cat "${a#--config=}" 2>/dev/null ;; esac
    prev=$a
  done
  cat
  echo
  echo '--end'
} >> "$d/gitlab-runner-calls"
cat "$d/runner-output" >&2
exit "$(cat "$d/runner-exit")"
"#;

/// Records the call, then runs the real curl with the same stdin.
fn curl_wrapper(real: &str) -> String {
    format!(
        r#"#!/bin/sh
d=$(dirname "$0")
{{
  echo '--argv'
  for a in "$@"; do printf '%s\n' "$a"; done
  echo '--env'
  env
  echo '--seen'
  echo '--end'
}} >> "$d/curl-calls"
exec '{real}' "$@"
"#
    )
}

fn parse_calls(path: &Path) -> Result<Vec<Call>> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    let mut calls = Vec::new();
    let mut call = Call::default();
    let mut section = "";
    for line in text.lines() {
        match line {
            "--argv" | "--env" | "--seen" => section = line,
            "--end" => {
                calls.push(std::mem::take(&mut call));
                section = "";
            }
            _ => match section {
                "--argv" => call.argv.push(line.to_owned()),
                "--env" => {
                    call.env.push_str(line);
                    call.env.push('\n');
                }
                _ => {
                    call.seen.push_str(line);
                    call.seen.push('\n');
                }
            },
        }
    }
    for call in &mut calls {
        call.seen = format!("{}\n{}\n{}", call.argv.join("\n"), call.env, call.seen);
    }
    Ok(calls)
}

/// A stand-in GitLab answering every request with `status`, recording each.
fn gitlab(status: u16) -> Result<(u16, Arc<Mutex<Vec<VerifyRequest>>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Ok(request) = answer(stream, status) {
                if let Ok(mut requests) = recorded.lock() {
                    requests.push(request);
                }
            }
        }
    });
    Ok((port, requests))
}

fn answer(mut stream: TcpStream, status: u16) -> Result<VerifyRequest> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        ensure!(
            stream.read(&mut byte)? == 1,
            "connection closed mid-request"
        );
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let mut lines = head.lines();
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or_default().to_owned();
    let path = request_line.next().unwrap_or_default().to_owned();
    let length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or_default();
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    let (reason, answer) = match status {
        200 => (
            "OK",
            r#"{"id":1,"token":"[REDACTED]","token_expires_at":null}"#,
        ),
        403 => ("Forbidden", r#"{"message":"403 Forbidden"}"#),
        _ => (
            "Internal Server Error",
            r#"{"message":"500 Internal Server Error"}"#,
        ),
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
        answer.len()
    )?;
    stream.flush()?;
    Ok(VerifyRequest { method, path, body })
}

struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn executable(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

/// Runs the shared script as a container would (`bash -c`, the inputs as
/// environment variables, the token on stdin), with stand-ins first on PATH.
fn run_script(scenario: &Scenario) -> Result<Run> {
    let dir = TempDir(
        std::env::temp_dir().join(format!("gridops-gitlab-script-{}", uuid::Uuid::new_v4())),
    );
    fs::create_dir_all(&dir.0)?;
    let real_curl = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v curl"])
            .output()?
            .stdout,
    )?
    .trim()
    .to_owned();
    ensure!(!real_curl.is_empty(), "these tests need curl on PATH");
    executable(&dir.0.join("curl"), &curl_wrapper(&real_curl))?;
    executable(&dir.0.join("gitlab-runner"), RUNNER_STAND_IN)?;
    fs::write(dir.0.join("runner-output"), &scenario.runner_output)?;
    fs::write(dir.0.join("runner-exit"), scenario.runner_exit.to_string())?;

    let (port, requests) = if let Some(status) = scenario.verify_status {
        gitlab(status)?
    } else {
        // The discard port, where nothing listens. A port bound and let go
        // could be taken by another process, which would then answer (#124).
        (9, Arc::default())
    };
    let url = format!("http://127.0.0.1:{port}");

    let mut command = Command::new("bash");
    command
        .args(["-c", &gitlab_runner_script()])
        .current_dir(&dir.0)
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", dir.0.display()))
        .env("HOME", &dir.0);
    for variable in gitlab_runner_env(&url, WAIT_TIMEOUT) {
        let (name, value) = variable
            .split_once('=')
            .context(format!("not NAME=value: {variable}"))?;
        command.env(name, value);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        // The script may exit before it reads (an empty token): ignore EPIPE.
        let _ = stdin.write_all(scenario.stdin.as_bytes());
    }
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(child.wait_with_output());
    });
    let output = finished
        .recv_timeout(Duration::from_secs(60))
        .context("the script ran for over a minute")??;
    let verify_requests =
        std::mem::take(&mut *requests.lock().map_err(|_| anyhow::anyhow!("poisoned"))?);
    Ok(Run {
        url,
        status: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        verify_requests,
        curl_calls: parse_calls(&dir.0.join("curl-calls"))?,
        runner_calls: parse_calls(&dir.0.join("gitlab-runner-calls"))?,
    })
}
