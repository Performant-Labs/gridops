//! Tests of the Tart agent's GitLab module (#142), written before it.
//!
//! Mounted from `gitlab.rs` with:
//!
//! ```ignore
//! #[cfg(test)]
//! #[path = "gitlab_tests.rs"]
//! mod tests;
//! ```
//!
//! The guest script is run under `/bin/bash` (the guest's shell, 3.2 on
//! macOS) with fake `gitlab-runner` and `curl` executables, so no Tart, VM or
//! GitLab is involved.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context as _, Result};
use tokio::{fs, io::AsyncWriteExt as _, process::Command};

use super::{DEFAULT_GITLAB_RUNNER_BINARY, exit_reason, runner_script};
use crate::tests::{gitlab_request, install_script};
use crate::{AgentError, BitbucketBootstrap, GitlabBootstrap, ProvisionRunner, valid_guest_path};

/// A runner token. Every token in these tests contains `LEAK_MARKER`, which
/// no message, argument or other environment variable may contain.
const TOKEN: &str = "glrt-T0KEN-must-never-leak-4f1c9a";
const LEAK_MARKER: &str = "must-never-leak";
const URL: &str = "https://gitlab.example.com";
/// Not the default 600, so a script that ignores the bootstrap is caught.
const WAIT_TIMEOUT: u32 = 37;

fn bootstrap(url: &str, wait_timeout_seconds: u32) -> GitlabBootstrap {
    GitlabBootstrap {
        url: url.into(),
        wait_timeout_seconds,
    }
}

/// One change to a GitLab request.
type Edit = fn(&mut ProvisionRunner);

/// The manager's GitLab request (`crate::tests::gitlab_request`) with this
/// module's token, then `edit`.
fn gitlab_input(edit: Edit) -> Result<ProvisionRunner> {
    let mut input: ProvisionRunner = serde_json::from_value(gitlab_request())?;
    input.gitlab_runner_token = Some(TOKEN.into());
    edit(&mut input);
    Ok(input)
}

// ---- Validation ----

#[test]
fn a_valid_gitlab_request_is_accepted() -> Result<()> {
    let accepted: [Edit; 3] = [
        |_| {},
        |input| input.gitlab = Some(bootstrap("http://gitlab.internal:8080/", 1)),
        |input| input.gitlab = Some(bootstrap("https://gitlab.example.com/sub/path", 600)),
    ];
    for edit in accepted {
        let input = gitlab_input(edit)?;
        let result = input.validate();
        assert!(result.is_ok(), "{:?}: {result:?}", input.gitlab);
    }
    // Documented deviation (brief, review round 1 W-1): the Tart agent's
    // request has no `registrationToken` field and no `deny_unknown_fields`,
    // so a stray one is ignored, not refused.
    let mut body = gitlab_request();
    body["gitlabRunnerToken"] = TOKEN.into();
    body["registrationToken"] = "glrt-stray-registration-token".into();
    let input: ProvisionRunner = serde_json::from_value(body)?;
    assert!(input.validate().is_ok());
    Ok(())
}

#[test]
fn each_invalid_gitlab_request_is_refused_without_its_token() -> Result<()> {
    // The control: every case below breaks this accepted request one way.
    let control = gitlab_input(|_| {})?.validate();
    assert!(
        control.is_ok(),
        "the valid request was refused: {control:?}"
    );

    let refused: [(&str, Edit); 22] = [
        ("no token", |input| input.gitlab_runner_token = None),
        ("an empty token", |input| {
            input.gitlab_runner_token = Some(String::new());
        }),
        ("a token that is not a runner token", |input| {
            input.gitlab_runner_token = Some("glpat-must-never-leak-personal-token".into());
        }),
        ("a token on two lines", |input| {
            input.gitlab_runner_token = Some(format!("{TOKEN}\nsecond-line"));
        }),
        ("a token with a carriage return", |input| {
            input.gitlab_runner_token = Some(format!("{TOKEN}\r"));
        }),
        ("no bootstrap", |input| input.gitlab = None),
        ("a zero wait timeout", |input| {
            input.gitlab = Some(bootstrap(URL, 0));
        }),
        ("an empty URL", |input| {
            input.gitlab = Some(bootstrap("", 600));
        }),
        ("a URL without a scheme", |input| {
            input.gitlab = Some(bootstrap("gitlab.example.com", 600));
        }),
        ("an ftp URL", |input| {
            input.gitlab = Some(bootstrap("ftp://gitlab.example.com", 600));
        }),
        ("a URL with a space", |input| {
            input.gitlab = Some(bootstrap("https://gitlab.example.com/a b", 600));
        }),
        ("a URL on two lines", |input| {
            input.gitlab = Some(bootstrap(
                "https://gitlab.example.com\nhttps://evil.example.com",
                600,
            ));
        }),
        ("a URL with a tab", |input| {
            input.gitlab = Some(bootstrap("https://gitlab.example.com\t", 600));
        }),
        ("a URL with a control character", |input| {
            input.gitlab = Some(bootstrap("https://gitlab.example.com/\u{1b}[2J", 600));
        }),
        ("a URL over 512 bytes", |input| {
            input.gitlab = Some(bootstrap(
                &format!("https://gitlab.example.com/{}", "a".repeat(512)),
                600,
            ));
        }),
        ("a persistent mode", |input| {
            input.mode = "persistent".into();
        }),
        ("the native runtime", |input| {
            input.runtime = "native".into();
        }),
        ("an extra jitConfig", |input| {
            input.jit_config = Some("j".repeat(64));
        }),
        ("an extra bitbucket bootstrap", |input| {
            input.bitbucket = Some(BitbucketBootstrap {
                account_uuid: "{account}".into(),
                repository_uuid: None,
                runner_uuid: "{runner}".into(),
                oauth_client_id: "client".into(),
            });
        }),
        ("an extra bitbucketOauthClientSecret", |input| {
            input.bitbucket_oauth_client_secret = Some("s".repeat(32));
        }),
        // The checks every platform shares still apply after the GitLab ones.
        ("a fractional CPU limit", |input| input.cpu_limit = 2.5),
        ("no memory", |input| input.memory_limit_mb = 0),
    ];
    for (case, edit) in refused {
        let result = gitlab_input(edit)?.validate();
        let Err(AgentError::BadRequest(message)) = &result else {
            anyhow::bail!("{case}: expected BadRequest, got {result:?}")
        };
        assert!(!message.contains(LEAK_MARKER), "{case}: {message}");
        assert!(!format!("{result:?}").contains(LEAK_MARKER), "{case}");
        match case {
            "a persistent mode" => assert!(message.contains("ephemeral"), "{case}: {message}"),
            "the native runtime" => assert!(message.contains("GitLab"), "{case}: {message}"),
            _ => {}
        }
    }
    Ok(())
}

// ---- Exit reasons and the runner binary ----

#[test]
fn exit_statuses_map_to_the_exit_reason_table() {
    assert_eq!(exit_reason(0), Some("completed"));
    assert_eq!(exit_reason(75), Some("idle_expired"));
    assert_eq!(exit_reason(77), Some("registration_refused"));
    for other in [-1, 1, 2, 74, 76, 78, 143, 255] {
        assert_eq!(exit_reason(other), None, "{other}");
    }
}

#[test]
fn the_default_runner_binary_is_a_guest_path_the_image_script_and_docs_name() {
    assert!(
        valid_guest_path(DEFAULT_GITLAB_RUNNER_BINARY),
        "{DEFAULT_GITLAB_RUNNER_BINARY}"
    );
    let image_script = include_str!("../../../scripts/prepare-tart-gitlab-runner-image.sh");
    assert!(image_script.contains(DEFAULT_GITLAB_RUNNER_BINARY));
    assert!(image_script.contains("GRIDOPS_TART_GITLAB_RUNNER_BINARY"));
    let docs = include_str!("../../../docs/macos-runners.md");
    assert!(docs.contains(DEFAULT_GITLAB_RUNNER_BINARY));
}

// ---- The guest script, under bash with fakes ----

/// Stands in for `gitlab-runner`, modelled on gitlab-runner's
/// `commands/single.go` and `network/gitlab.go`: `run-single` logs to stderr
/// and exits 0 both after its one job (whatever the job's result) and when
/// the wait timeout passes with no job. A refused token is logged as
/// "Checking for jobs... forbidden" and then also waits out the timeout.
/// It can also exit with a status of its own (75 or 77, colliding with the
/// table) or exit 0 having logged none of those lines, as when it is stopped
/// by a signal. It records each invocation's arguments and its whole
/// environment.
const FAKE_GITLAB_RUNNER: &str = r#"#!/bin/sh
dir=$(dirname "$0")
{ printf '%s\n' "$@"; echo '--end--'; } >> "$dir/runner-argv"
env > "$dir/runner-env"
url=''; wait=''; previous=''
for arg in "$@"; do
  case "$previous" in --url) url=$arg ;; --wait-timeout) wait=$arg ;; esac
  case "$arg" in --url=*) url=${arg#*=} ;; --wait-timeout=*) wait=${arg#*=} ;; esac
  previous=$arg
done
echo "Starting runner for $url with token T0KEN-ab ..." >&2
case "$(cat "$dir/runner-behaviour" 2>/dev/null)" in
  job-passed)
    echo 'Checking for jobs... received                     job=1 repo_url=https://gitlab.example.com/group/project.git runner=T0KEN-ab' >&2
    echo 'Job succeeded                                     duration_s=1.2 job=1 project=1 runner=T0KEN-ab' >&2
    echo 'This runner has processed its build limit, so now exiting' >&2
    exit 0 ;;
  job-failed)
    echo 'Checking for jobs... received                     job=1 repo_url=https://gitlab.example.com/group/project.git runner=T0KEN-ab' >&2
    echo 'ERROR: Job failed: exit status 1                  duration_s=1.2 job=1 project=1 runner=T0KEN-ab' >&2
    echo 'ERROR: Failed to process build                    error=exit status 1' >&2
    echo 'This runner has processed its build limit, so now exiting' >&2
    exit 0 ;;
  idle)
    echo "This runner has not received a job in $wait seconds, so now exiting" >&2
    exit 0 ;;
  refused)
    echo 'ERROR: Checking for jobs... forbidden              runner=T0KEN-ab status=403 Forbidden' >&2
    echo 'Runner is not healthy!' >&2
    echo 'ERROR: Checking for jobs... forbidden              runner=T0KEN-ab status=403 Forbidden' >&2
    echo 'Runner is not healthy!' >&2
    echo "This runner has not received a job in $wait seconds, so now exiting" >&2
    exit 0 ;;
  crash)
    echo 'FATAL: the fake runner crashed' >&2
    exit 3 ;;
  exit-75)
    echo 'FATAL: the fake runner stopped' >&2
    exit 75 ;;
  exit-77)
    echo 'FATAL: the fake runner stopped' >&2
    exit 77 ;;
  silent)
    exit 0 ;;
esac
echo 'fake gitlab-runner: no behaviour scripted' >&2
exit 99
"#;

/// Stands in for `curl` on the verify call (`POST <url>/api/v4/runners/verify`,
/// 200 for a valid token, 403 for a refused one). It answers with the status
/// in `verify-status`, honouring `-o`/`--output`, `-w`/`--write-out`
/// (`%{http_code}`) and `--fail`, and records its arguments and its stdin.
const FAKE_CURL: &str = r#"#!/bin/sh
dir=$(dirname "$0")
{ printf '%s\n' "$@"; echo '--end--'; } >> "$dir/curl-argv"
cat >> "$dir/curl-stdin"
code=$(cat "$dir/verify-status" 2>/dev/null || echo 200)
output=''; write_out=''; fail=''
while [ $# -gt 0 ]; do
  case "$1" in
    -o|--output) output=$2; shift ;;
    --output=*) output=${1#*=} ;;
    -w|--write-out) write_out=$2; shift ;;
    --write-out=*) write_out=${1#*=} ;;
    --fail|--fail-with-body) fail=1 ;;
    --*) ;;
    -*f*) fail=1 ;;
  esac
  shift
done
if [ "$code" = 200 ]; then body='{"id":1,"token_expires_at":null}'; else body='{"message":"403 Forbidden"}'; fi
if [ -z "$output" ]; then printf '%s' "$body"
elif [ "$output" != /dev/null ]; then printf '%s' "$body" > "$output"; fi
if [ -n "$write_out" ]; then
  printf '%b' "$(printf '%s' "$write_out" | sed "s/%{http_code}/$code/g; s/%{response_code}/$code/g")"
fi
if [ -n "$fail" ] && [ "$code" -ge 400 ]; then exit 22; fi
exit 0
"#;

/// A scratch guest: `bin` holds the fakes (first on PATH), `home` is `$HOME`
/// and `work` the working directory.
struct Guest {
    dir: PathBuf,
}

struct GuestRun {
    status: Option<i32>,
    output: String,
}

impl Guest {
    async fn new() -> Result<Self> {
        let dir =
            std::env::temp_dir().join(format!("gridops-gitlab-guest-{}", uuid::Uuid::new_v4()));
        for path in ["bin", "home", "work"] {
            fs::create_dir_all(dir.join(path)).await?;
        }
        let guest = Self { dir };
        install_script(&guest.bin().join("gitlab-runner"), FAKE_GITLAB_RUNNER).await?;
        install_script(&guest.bin().join("curl"), FAKE_CURL).await?;
        Ok(guest)
    }

    fn bin(&self) -> PathBuf {
        self.dir.join("bin")
    }

    fn home(&self) -> PathBuf {
        self.dir.join("home")
    }

    fn runner_binary(&self) -> String {
        self.bin()
            .join("gitlab-runner")
            .to_string_lossy()
            .into_owned()
    }

    async fn flag(&self, name: &str, contents: &str) -> Result<()> {
        Ok(fs::write(self.bin().join(name), contents).await?)
    }

    async fn read(&self, name: &str) -> String {
        fs::read_to_string(self.bin().join(name))
            .await
            .unwrap_or_default()
    }

    /// Each recorded invocation's arguments.
    async fn invocations(&self, name: &str) -> Vec<Vec<String>> {
        self.read(name)
            .await
            .split_terminator("--end--\n")
            .map(|invocation| invocation.lines().map(ToOwned::to_owned).collect())
            .collect()
    }

    /// Runs `script` as `start_runner` does in the VM (`/bin/bash`, the
    /// token and a newline on stdin), with a clean environment.
    async fn run(&self, script: &str) -> Result<GuestRun> {
        let path = format!("{}:/usr/bin:/bin", self.bin().display());
        let mut child = Command::new("/bin/bash")
            .arg("-c")
            .arg(script)
            .current_dir(self.dir.join("work"))
            .env_clear()
            .env("PATH", path)
            .env("HOME", self.home())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let mut stdin = child.stdin.take().context("script stdin")?;
        stdin.write_all(format!("{TOKEN}\n").as_bytes()).await?;
        drop(stdin);
        let output = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
            .await
            .context("the guest script did not finish within 30 seconds")??;
        Ok(GuestRun {
            status: output.status.code(),
            output: format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        })
    }

    /// Files the fakes did not create anywhere in the guest whose name
    /// starts with `pwned`: proof a hostile value ran as shell.
    async fn pwned(&self) -> Result<Vec<PathBuf>> {
        let mut found = Vec::new();
        for path in ["bin", "home", "work"] {
            let mut entries = fs::read_dir(self.dir.join(path)).await?;
            while let Some(entry) = entries.next_entry().await? {
                if entry.file_name().to_string_lossy().starts_with("pwned") {
                    found.push(entry.path());
                }
            }
        }
        Ok(found)
    }
}

impl Drop for Guest {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The value of `flag` in `argv`, given as `--flag value` or `--flag=value`.
fn flag_value<'a>(argv: &'a [String], flag: &str) -> Option<&'a str> {
    argv.iter().enumerate().find_map(|(index, arg)| {
        if arg == flag {
            argv.get(index + 1).map(String::as_str)
        } else {
            arg.strip_prefix(flag)
                .and_then(|rest| rest.strip_prefix('='))
        }
    })
}

/// What reached `gitlab-runner` and `curl` in one run: the documented
/// `run-single` flags, the token only through `CI_SERVER_TOKEN`, a builds
/// directory under `$HOME`, and the token in no argument.
async fn assert_runner_invocation(guest: &Guest, case: &str, url: &str) -> Result<()> {
    let runs = guest.invocations("runner-argv").await;
    let [argv] = runs.as_slice() else {
        anyhow::bail!("{case}: expected one gitlab-runner run, got {runs:?}")
    };
    assert_eq!(
        argv.first().map(String::as_str),
        Some("run-single"),
        "{case}: {argv:?}"
    );
    assert_eq!(flag_value(argv, "--url"), Some(url), "{case}: {argv:?}");
    assert_eq!(
        flag_value(argv, "--executor"),
        Some("shell"),
        "{case}: {argv:?}"
    );
    assert_eq!(
        flag_value(argv, "--max-builds"),
        Some("1"),
        "{case}: {argv:?}"
    );
    let wait = WAIT_TIMEOUT.to_string();
    assert_eq!(
        flag_value(argv, "--wait-timeout"),
        Some(wait.as_str()),
        "{case}: {argv:?}"
    );
    assert!(
        argv.iter().all(|arg| !arg.contains(LEAK_MARKER)),
        "{case}: token in argv"
    );

    let env = guest.read("runner-env").await;
    let token_lines = env
        .lines()
        .filter(|line| line.contains(LEAK_MARKER))
        .collect::<Vec<_>>();
    assert_eq!(
        token_lines,
        [format!("CI_SERVER_TOKEN={TOKEN}")],
        "{case}: the token must reach gitlab-runner only as CI_SERVER_TOKEN"
    );
    let builds_dir = flag_value(argv, "--builds-dir")
        .map(ToOwned::to_owned)
        .or_else(|| {
            env.lines()
                .find_map(|line| line.strip_prefix("RUNNER_BUILDS_DIR="))
                .map(ToOwned::to_owned)
        })
        .with_context(|| {
            format!("{case}: no builds directory (--builds-dir or RUNNER_BUILDS_DIR)")
        })?;
    let home = format!("{}/", guest.home().display());
    assert!(
        builds_dir.starts_with(&home),
        "{case}: {builds_dir} is not under {home}"
    );
    Ok(())
}

async fn assert_verify_call(guest: &Guest, case: &str, url: &str) -> Result<()> {
    let calls = guest.invocations("curl-argv").await;
    anyhow::ensure!(!calls.is_empty(), "{case}: the token was not verified");
    let endpoint = format!("{url}/api/v4/runners/verify");
    for argv in &calls {
        assert!(
            argv.iter().all(|arg| !arg.contains(LEAK_MARKER)),
            "{case}: token in curl argv"
        );
        assert!(argv.contains(&endpoint), "{case}: {argv:?}");
    }
    assert!(
        guest.read("curl-stdin").await.contains(TOKEN),
        "{case}: the verify request body did not carry the token on stdin"
    );
    Ok(())
}

/// Each way a runner ends, run through the real script under bash, exits
/// with exactly the status `exit_reason` maps, or a status of its own outside
/// the table. Every case verifies the token first; when the verify call does
/// not answer 200, `run-single` never runs.
#[tokio::test]
async fn the_guest_script_exits_with_the_status_each_ending_maps_to() -> Result<()> {
    // (case, verify answer, run-single behaviour, expected exit status)
    let cases = [
        ("a job that passed", "200", "job-passed", 0),
        ("a job that failed", "200", "job-failed", 0),
        ("no job before the wait timeout", "200", "idle", 75),
        ("another runner failure", "200", "crash", 3),
        // Refused during run-single, after a verify that passed (W-1's
        // time-of-check gap): run-single logs "forbidden" and waits out the
        // timeout, which must not list as idle_expired.
        ("a token refused by run-single", "200", "refused", 77),
        (
            "a token refused by the verify call",
            "403",
            "job-passed",
            77,
        ),
        (
            "a verify call answered with neither 200 nor 403",
            "500",
            "job-passed",
            69,
        ),
        // run-single's own status colliding with the table must leave it.
        ("run-single itself exiting 75", "200", "exit-75", 70),
        ("run-single itself exiting 77", "200", "exit-77", 70),
        // Exit 0 with no job, no build limit and no timeout line (a signal
        // stop) must not list as completed.
        (
            "run-single exiting 0 without a recognised line",
            "200",
            "silent",
            70,
        ),
    ];
    for (case, verify, behaviour, expected) in cases {
        let guest = Guest::new().await?;
        guest.flag("verify-status", verify).await?;
        guest.flag("runner-behaviour", behaviour).await?;
        let script = runner_script(&guest.runner_binary(), &bootstrap(URL, WAIT_TIMEOUT));
        let run = guest.run(&script).await?;
        assert_eq!(run.status, Some(expected), "{case}:\n{}", run.output);
        assert!(!run.output.contains(LEAK_MARKER), "{case}: token in output");
        assert_verify_call(&guest, case, URL).await?;
        if verify == "200" {
            assert_runner_invocation(&guest, case, URL).await?;
        } else {
            assert_eq!(
                guest.invocations("runner-argv").await,
                Vec::<Vec<String>>::new(),
                "{case}: run-single ran after the verify call answered {verify}"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn a_guest_without_gitlab_runner_exits_78_and_says_where_it_looked() -> Result<()> {
    let guest = Guest::new().await?;
    let missing = format!("{}/no-such-gitlab-runner", guest.bin().display());
    assert!(valid_guest_path(&missing), "{missing}");
    let run = guest
        .run(&runner_script(&missing, &bootstrap(URL, WAIT_TIMEOUT)))
        .await?;
    assert_eq!(run.status, Some(78), "{}", run.output);
    assert!(run.output.contains(&missing), "{}", run.output);
    assert_eq!(
        guest.invocations("runner-argv").await,
        Vec::<Vec<String>>::new()
    );
    Ok(())
}

/// The URL comes from the request, so the script must hand it to
/// `gitlab-runner` as one inert word, whatever it contains.
#[tokio::test]
async fn hostile_urls_reach_gitlab_runner_verbatim_and_run_nothing() -> Result<()> {
    let hostile = [
        r#"https://gitlab.example.com/$(touch${IFS}pwned-substitution)"#,
        r#"https://gitlab.example.com/`touch${IFS}pwned-backtick`"#,
        r#"https://gitlab.example.com/;touch${IFS}pwned-semicolon;"x\"#,
        r#"https://gitlab.example.com/a&&touch${IFS}pwned-and||b|c>pwned-redirect"#,
        r#"https://gitlab.example.com/$HOME/${PATH}/*"#,
    ];
    for url in hostile {
        let guest = Guest::new().await?;
        guest.flag("runner-behaviour", "job-passed").await?;
        let run = guest
            .run(&runner_script(
                &guest.runner_binary(),
                &bootstrap(url, WAIT_TIMEOUT),
            ))
            .await?;
        assert_eq!(run.status, Some(0), "{url}:\n{}", run.output);
        assert_runner_invocation(&guest, url, url).await?;
        let pwned = guest.pwned().await?;
        assert!(pwned.is_empty(), "{url} ran as shell: {pwned:?}");
    }

    // A single quote: `shell_single_quote` mis-quotes it today (brief,
    // "Edge case found while verifying"). The validation may refuse it, or
    // the helper may be fixed (A finding 4, F's choice); either way it must
    // never reach bash broken.
    let quoted = "https://gitlab.example.com/it's";
    let mut input = gitlab_input(|_| {})?;
    input.gitlab = Some(bootstrap(quoted, WAIT_TIMEOUT));
    match input.validate() {
        Err(AgentError::BadRequest(message)) => {
            assert!(!message.contains(LEAK_MARKER), "{message}");
        }
        Ok(()) => {
            let guest = Guest::new().await?;
            guest.flag("runner-behaviour", "job-passed").await?;
            let run = guest
                .run(&runner_script(
                    &guest.runner_binary(),
                    &bootstrap(quoted, WAIT_TIMEOUT),
                ))
                .await?;
            assert_eq!(run.status, Some(0), "{}", run.output);
            assert_runner_invocation(&guest, quoted, quoted).await?;
        }
        Err(other) => anyhow::bail!("expected BadRequest or acceptance, got {other:?}"),
    }
    Ok(())
}
