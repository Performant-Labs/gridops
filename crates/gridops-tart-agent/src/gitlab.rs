//! GitLab runners on Tart pools (#142): one ephemeral macOS VM per runner,
//! running `gitlab-runner run-single` with the shell executor for at most one
//! job.
//!
//! The request arrives as a GitLab `POST /v1/runners` forwarded by the manager
//! (#137 R2-1, D10). This module holds what is GitLab's own: the request's
//! validation, the guest runner script, and the mapping from that script's
//! exit status to the `exitReason` the runner listing reports. `main.rs`
//! wires them into `validate`, `provision_runner` and `list_runners`.
//!
//! The runner token is a secret handled like a GitHub JIT configuration: it
//! is never logged, never in an error, never on a command line, and reaches
//! the guest only on `tart exec -i`'s stdin.

use crate::{
    AgentError, GITLAB_EXIT_REASONS, GITLAB_RUNNER_TOKEN_PREFIX, GitlabBootstrap, ProvisionRunner,
    shell_single_quote,
};

/// Where `scripts/prepare-tart-gitlab-runner-image.sh` installs `gitlab-runner`
/// in the guest, and so the agent's default for
/// `GRIDOPS_TART_GITLAB_RUNNER_BINARY`. The image script and
/// `docs/macos-runners.md` name the same path.
pub(crate) const DEFAULT_GITLAB_RUNNER_BINARY: &str =
    "/Users/admin/gitlab-runner/bin/gitlab-runner";

/// The longest GitLab URL accepted, as the agent's other validators cap
/// theirs.
const MAX_URL_LEN: usize = 512;

/// The longest runner token accepted; GitLab's are well under 100 bytes.
const MAX_TOKEN_LEN: usize = 1_024;

/// Checks the GitLab parts of an ephemeral GitLab request. `validate` has
/// already refused a native runtime and a non-ephemeral mode, and checks the
/// CPU and memory every platform shares after this.
///
/// No message names or echoes the token.
pub(crate) fn validate(input: &ProvisionRunner) -> Result<(), AgentError> {
    if input.jit_config.is_some()
        || input.bitbucket.is_some()
        || input.bitbucket_oauth_client_secret.is_some()
    {
        return Err(AgentError::BadRequest(
            "GitLab runners take no GitHub JIT configuration or Bitbucket registration.".into(),
        ));
    }
    let Some(bootstrap) = input.gitlab.as_ref() else {
        return Err(AgentError::BadRequest(
            "GitLab runner bootstrap is required.".into(),
        ));
    };
    if !valid_url(&bootstrap.url) {
        return Err(AgentError::BadRequest(
            "GitLab URL must be an http or https URL of at most 512 bytes with no whitespace or \
             control characters."
                .into(),
        ));
    }
    if bootstrap.wait_timeout_seconds == 0 {
        return Err(AgentError::BadRequest(
            "GitLab runner wait timeout must be positive.".into(),
        ));
    }
    if !input
        .gitlab_runner_token
        .as_deref()
        .is_some_and(valid_token)
    {
        return Err(AgentError::BadRequest(
            "GitLab runner token is missing or invalid.".into(),
        ));
    }
    Ok(())
}

/// An `http`/`https` URL with something after the scheme, on one line, with
/// no whitespace or control characters. The guest script single-quotes it,
/// so any other character is inert.
fn valid_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    rest.is_some_and(|rest| !rest.is_empty())
        && url.len() <= MAX_URL_LEN
        && !url
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

/// A `glrt-` token of printable ASCII other than `"` and `\`. The guest
/// script writes it into a JSON string for the verify call without escaping,
/// and GitLab's tokens never contain either.
fn valid_token(token: &str) -> bool {
    token.len() > GITLAB_RUNNER_TOKEN_PREFIX.len()
        && token.len() <= MAX_TOKEN_LEN
        && token.starts_with(GITLAB_RUNNER_TOKEN_PREFIX)
        && token
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\\'))
}

/// The `exitReason` the listing reports for a GitLab runner script's exit
/// status, or none for a status outside the table.
pub(crate) fn exit_reason(code: i32) -> Option<&'static str> {
    GITLAB_EXIT_REASONS
        .iter()
        .find(|(status, _)| *status == code)
        .map(|(_, reason)| *reason)
}

/// The script `start_runner` runs in the VM with `/bin/bash -lc`, the token on
/// its stdin. Every interpolated value is single-quoted.
///
/// It exits 78 (`runner_missing`, `EX_CONFIG`) if the image has no
/// executable `gitlab-runner` at `runner_binary`. Otherwise it verifies the
/// token with GitLab (`POST /api/v4/runners/verify`, the body piped to `curl`
/// by the `printf` builtin, so the token is in no argv; 403 exits 77), then
/// runs `gitlab-runner run-single` for one job with the token only in its
/// `CI_SERVER_TOKEN`. `run-single` exits 0 both after its job and after the
/// wait timeout, so the script tells them apart by the lines upstream logs
/// (`commands/single.go`, `network/gitlab.go`): a job received or the build
/// limit reached exits 0; otherwise "Checking for jobs... forbidden" (the
/// token was refused after the verify passed) exits 77; otherwise the wait
/// timeout's line exits 75. Anything else keeps a status outside the table:
/// `run-single`'s own non-zero status (70 if that is itself 75, 77 or 78, so a
/// status in the table is always the script's own verdict), 69 for
/// a verify call GitLab did not answer with 200 or 403, and 70 for a
/// `run-single` that exited 0 with none of those lines (stopped by a signal).
pub(crate) fn runner_script(runner_binary: &str, bootstrap: &GitlabBootstrap) -> String {
    format!(
        r#"set -euo pipefail
runner_binary={runner_binary}
gitlab_url={gitlab_url}
if [ ! -x "$runner_binary" ]; then
  printf 'GridOps macOS runner image is missing %s. Prepare the GitLab runner in this Tart base image first.\n' "$runner_binary" >&2
  exit 78
fi
IFS= read -r GRIDOPS_BOOTSTRAP_SECRET
[ -n "$GRIDOPS_BOOTSTRAP_SECRET" ]
work_root="$HOME/gridops-gitlab-runner"
mkdir -p "$work_root/builds" "$work_root/cache"
cd "$work_root"
verify_url="${{gitlab_url%/}}/api/v4/runners/verify"
verify_status=$(printf '{{"token":"%s"}}' "$GRIDOPS_BOOTSTRAP_SECRET" | curl --silent --show-error --globoff --proto '=http,https' --retry 3 --connect-timeout 15 --max-time 60 --request POST --header 'Content-Type: application/json' --data-binary @- --output /dev/null --write-out '%{{http_code}}' "$verify_url") || true
case "$verify_status" in
  200) ;;
  403)
    printf 'GitLab refused the runner token (HTTP 403 from %s).\n' "$verify_url" >&2
    exit 77 ;;
  *)
    printf 'GitLab did not verify the runner token (HTTP %s from %s).\n' "${{verify_status:-none}}" "$verify_url" >&2
    exit 69 ;;
esac
runner_output="$work_root/run-single.log"
set +e
CI_SERVER_TOKEN="$GRIDOPS_BOOTSTRAP_SECRET" "$runner_binary" run-single --url "$gitlab_url" --executor shell --max-builds 1 --wait-timeout {wait_timeout} --builds-dir "$work_root/builds" --cache-dir "$work_root/cache" 2>&1 | tee "$runner_output"
runner_status=${{PIPESTATUS[0]}}
set -e
unset GRIDOPS_BOOTSTRAP_SECRET
if [ "$runner_status" -ne 0 ]; then
  case "$runner_status" in
    75|77|78)
      printf 'gitlab-runner exited with status %s.\n' "$runner_status" >&2
      exit 70 ;;
  esac
  exit "$runner_status"
fi
if grep -qF -e 'Checking for jobs... received' -e 'This runner has processed its build limit' "$runner_output"; then
  exit 0
fi
if grep -qF 'Checking for jobs... forbidden' "$runner_output"; then
  printf 'GitLab refused the runner token.\n' >&2
  exit 77
fi
if grep -qF 'This runner has not received a job in' "$runner_output"; then
  exit 75
fi
printf 'gitlab-runner exited without taking a job or reaching its wait timeout.\n' >&2
exit 70"#,
        runner_binary = shell_single_quote(runner_binary),
        gitlab_url = shell_single_quote(&bootstrap.url),
        wait_timeout = bootstrap.wait_timeout_seconds,
    )
}

#[cfg(test)]
#[path = "gitlab_tests.rs"]
mod tests;
