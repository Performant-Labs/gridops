# Handoff-T-red: Phase 4 - GitLab: run gitlab-runner in Linux Docker runners (#147)

**Date:** 2026-11-20
**Branch:** claude/gitlab-147-hostagent-e4a01d (head ecef67c)
**Brief / wireframe reviewed:** docs/handoffs/147-brief.md (all sections, including the three amendment sections); wireframe N/A (no UI surface)

## A precondition

Confirmed: A returned **PASS** in round 3 (`docs/handoffs/147/handoff-A.md`, commit ecef67c), with two warnings. The tests act on both:

- **Warning 1** (a failed Start must remove its container, including a refusal that lands inside `feed_secret`). Pinned by the four failed-start tests and the never-ready test in `docker_gitlab.rs`, which check the outcome and that the container is gone. Two of them are A's named tests: an immediate exit 77, and a readiness-wait timeout.
- **Warning 2** (the script's inputs live with the script in core). Pinned by `gitlab_runner_env` and `valid_gitlab_url` in `provisioning.rs`, which both callers use. A test checks that the script reads every name the env builder sets.

## Tests authored

Each test sits at the cheapest tier that can see the behaviour:

- serde and constants: unit or integration tests, no I/O;
- the script: real `bash` and real `curl` against a local HTTP server;
- the fleet runtime: a stand-in `docker` CLI;
- the manager: its real router in front of the existing stand-in Docker engine, extended.

No e2e: there is no UI.

### `crates/gridops-core/tests/gitlab_runner_script.rs` (new, integration, public API only)

| Test | What it pins |
|---|---|
| `the_gitlab_images_are_pinned_releases` | B-1. `GITLAB_RUNNER_IMAGE` is `gitlab/gitlab-runner:` plus a `vX.Y.Z` or `ubuntu-vX.Y.Z` tag, so not alpine, ubi or `latest`. `GITLAB_DEFAULT_JOB_IMAGE` has a versioned tag that is not `latest`. |
| `the_gitlab_start_waits_fit_inside_the_start_deadline` | R2 B-2: the ready wait is 15 s, the verify max-time is 10 s, and verify < wait. |
| `verify_unavailable_has_its_own_exit_status` | R2 B-2: `GITLAB_EXIT_VERIFY_UNAVAILABLE` is not in `GITLAB_EXIT_REASONS`, and is in 3..=125. That rules out `set -e`'s 1, bash's 2 and the shell-reserved 126+. |
| `only_http_and_https_gitlab_urls_are_valid` | D10 and brief review W-2: `valid_gitlab_url` accepts http(s) URLs and refuses an empty string, a missing scheme, `ftp:`, `file:`, `javascript:`, `https://` and plain text. |
| `the_runner_env_carries_the_script_inputs_and_the_script_reads_them` | R2 W-3 and A warning 2. `gitlab_runner_env(url, 600)` is a list of `NAME=value` entries, each name set once. The values include the URL, `600` and `GITLAB_DEFAULT_JOB_IMAGE`. The script reads every name. The script text holds no interpolated value. |
| `a_verified_runner_says_it_is_ready_and_runs_one_job` | After a 200 from verify:<br>- the script exits 0 and prints `GITLAB_READY_MARKER` as a whole stdout line;<br>- gitlab-runner's output appears in the script's output (W-8);<br>- NV-4: one `POST /api/v4/runners/verify` whose form field `token` is the token;<br>- the token is in neither curl's argv nor its environment, so it is never exported;<br>- curl gets `--max-time 10` (or `-m 10`);<br>- NV-1: `run-single` gets `--url`, `--executor docker`, `--docker-image <job image>`, `--docker-disable-cache`, `--max-builds 1` and `--wait-timeout 600`;<br>- NV-2: the token reaches gitlab-runner, by argv, environment or `--config` file. |
| `a_runner_that_never_got_a_job_exits_idle_expired` | Only the idle line appears, and `run-single` exits 0: the script exits 75 (and printed the marker). |
| `a_runner_that_fails_by_itself_reports_no_runner_ending` | `run-single` exits 1: the script exits non-zero and with no reported ending (not 0, 75 or 77). |
| `a_refused_token_exits_registration_refused_without_running` | A 403 gives 77, no marker, and gitlab-runner is never run. |
| `a_gitlab_that_cannot_answer_exits_verify_unavailable` | A 500, and a port nothing listens on, each give `GITLAB_EXIT_VERIFY_UNAVAILABLE`, no marker, and no runner. |
| `an_empty_token_runs_nothing` | An empty stdin gives a non-reported failure, and neither curl nor the runner runs. |

### `crates/gridops-host-agent/tests/docker_gitlab.rs` (new, integration, stand-in `docker` CLI)

The stand-in keeps one container's state in files. It fills in any Go template it is given for the `.Id`, `.Name`, `.State.Running`, `.State.ExitCode`, `.State.Status`, `json .State` and environment-label fields, and answers `create`, `start [-i]`, `logs [--follow]`, `wait`, `rm`, `kill`, `stop`, `pull`, `image inspect`, `version` and `info`. Anything it can't answer goes to `unsupported`, which is shown on failure. So the tests pin outcomes, not F's exact `docker` calls.

Each test reruns itself with the stand-in first on PATH, like the existing `docker_runtime.rs`.

| Test | What it pins |
|---|---|
| `a_gitlab_prepare_without_the_docker_socket_is_refused` | W-5: a GitLab Prepare without `requires_docker_socket` returns `Failed { Configuration, false, None }`, and nothing is pulled. With the socket it succeeds. A GitHub Prepare without the socket is unchanged. |
| `a_gitlab_start_with_unusable_material_creates_nothing` | W-4 and brief review W-2. These are each `Failed { Configuration, false, None }` with no `docker create`:<br>- GitHub material;<br>- GitLab material that doesn't parse;<br>- Persistent mode;<br>- an `ftp:` URL;<br>- an unparseable URL. |
| `a_verified_gitlab_runner_starts_with_its_token_only_on_stdin` | Done-when (token not in `docker inspect` or the main process environment), B-1, R2 W-3, W-7 and W-8. The ready container gives `RunnerStarted { Docker { object_id } }` in under 10 s, and the container is not removed. Stdin is exactly `token\n`. No `create` argument contains the token. The `create` arguments include:<br>- every `gitlab_runner_env` entry as `-e NAME=value`;<br>- `-v <socket>:/var/run/docker.sock`;<br>- `--init`.<br>Nothing under `SHARED_WORK_ROOT` is mounted. The image is the prepared (pool) image, and `GITLAB_RUNNER_IMAGE` is not used. After the image comes a `bash`, and the last argument is `gitlab_runner_script()`. |
| `a_token_refused_at_once_fails_the_start_as_refused` | A warning 1: an exit 77 inside `feed_secret` gives `Failed { Configuration, false, Some(RegistrationRefused) }`, the container is removed, and the Start ends in under 25 s. |
| `a_token_refused_during_the_wait_fails_the_start_as_refused` | Exit 77 about 3 s after start, inside the readiness wait: the same outcome. |
| `an_unanswered_verify_fails_the_start_as_retryable` | `GITLAB_EXIT_VERIFY_UNAVAILABLE` after 2 s gives `Failed { Runtime, true, None }`, and the container is removed. |
| `any_other_exit_fails_the_start_as_retryable` | Exit 1 at once gives `Failed { Runtime, true, None }`, and the container is removed. |
| `a_runner_that_never_gets_ready_is_given_up_within_the_deadline` | R2 B-2 and A warning 1: a container that runs with no marker gives `Failed { Runtime, true, None }` and is removed, after between 14 s and 25 s. |
| `an_exited_gitlab_runner_inspects_as_gone_saying_why` | B-2, B-3 and D147-2. A GitLab container that exited is `Failed { ResourceUnavailable, false, d }`, where `d` is:<br>- `IdleExpired` for exit 75;<br>- `RegistrationRefused` for exit 77;<br>- `None` for exit 0 and exit 3.<br>A running one inspects as `Succeeded`. A GitHub container that exited 75 or 77 gets `None`. |
| `the_docker_backend_offers_ephemeral_gitlab_runners` | D6: `supported_runners` is exactly the 4 GitHub entries plus `gitlab_project`/`ephemeral` and `gitlab_group`/`ephemeral`. |

### `crates/gridops-host-agent/tests/dispatch.rs` (extended)

| Test | What it pins |
|---|---|
| `helper_failures_keep_their_wire_form_and_may_carry_a_diagnostic` | B-3:<br>- `Failed { Runtime, true, None }` serializes to exactly `{"kind":"failed","code":"runtime","retryable":true}` and parses back;<br>- a failure with a `RegistrationRefused` diagnostic round-trips;<br>- an extra field is still refused. |
| `a_helper_diagnostic_reaches_the_command_outcome` | B-3: `Failed { Configuration, false, Some(diag) }` from the helper on Start reaches `CommandOutcome::Failed` with the identical diagnostic. |
| `Recorder` (test infra) | Gains `start_failure`. The three existing `HelperResponse::Failed` literals gain `diagnostic: None` (R2 W-2 mechanical fallout, done by me because these are test files). |

### Mechanical fallout in other test files (R2 W-2)

- `crates/gridops-host-agent/tests/docker_runtime.rs`: the `failed` closure gains `diagnostic: None`.
- `crates/gridops-api/tests/fleet_http/placement.rs`: the 4 `FakeDocker` literals gain `diagnostic: None`.

No assertion changes in either file.

### `crates/gridops-manager/src/main.rs` (tests module) and `src/gitlab_runner_tests.rs` (new)

| Test | What it pins |
|---|---|
| `valid_gitlab_requests_pass_validation_on_both_providers` (replaces `gitlab_runners_are_refused_by_name_until_their_owners_land`) | D10: a well-formed GitLab request passes `validate()` for `docker` and for `tart`. |
| `invalid_gitlab_requests_are_refused_without_echoing_the_token` | D10. On both providers, each of these is a `BadRequest` whose message contains neither the token nor the wrong-prefix value:<br>- no `gitlab`;<br>- no token;<br>- a token without `glrt-`;<br>- an empty token;<br>- persistent mode;<br>- an `ftp:` URL;<br>- a URL without a scheme;<br>- `jitConfig`;<br>- `registrationToken`;<br>- `bitbucket`;<br>- `bitbucketOauthClientSecret`. |
| `gitlab_runners_launch_the_shared_script_and_keep_the_token_for_stdin` | W-6. `take_bootstrap_secret` returns the `glrt-` token and clears it, even though mode is `ephemeral`. `runner_command`'s last argument is `gitlab_runner_script()`, run by a `bash`. Its env includes every `gitlab_runner_env(url, 600)` entry. Neither holds the token or `--jitconfig`. |
| `a_runners_exit_reason_survives_the_managers_reparse` | D15: a Tart entry with `exitReason` keeps it through `ManagedRunner` parse and serialize. Without one, there is no `exitReason` key. |
| `gitlab_runner_tests::a_gitlab_runner_starts_labelled_with_its_token_only_on_stdin` | D10 and W-6, through `POST /v1/runners` with the runner socket set: the answer is 201. The single create body:<br>- has the `io.gridops.platform=gitlab` label;<br>- has `gitlab_runner_script()` as the last `Cmd` element;<br>- has `Env` containing the builder's entries;<br>- has `HostConfig.Binds` containing the socket bind;<br>- does not contain the token anywhere.<br>The attached stdin is exactly `token\n`. |
| `gitlab_runner_tests::a_gitlab_runner_is_refused_without_a_runner_docker_socket` | W-6: without `shared_docker_socket` the answer is 400, the body does not contain the token, and no container is created. |
| `gitlab_runner_tests::exited_gitlab_runners_are_listed_with_why_they_ended` | D10 and W-6, `GET /v1/runners`. Exited GitLab containers with exit codes 0, 75 and 77 get `exitReason` `completed`, `idle_expired` and `registration_refused`. These get no `exitReason` key:<br>- exit code 3;<br>- a running GitLab container;<br>- a GitHub container that exited 75. |
| `stand_in_docker.rs` (test infra) | Gains `containers` (listed and inspected with state and exit code) and `recorded` (create bodies and attached stdin). Existing behaviour is unchanged when they are unset. |
| mount line in `main.rs` | `#[cfg(test)] mod gitlab_runner_tests;`. Test-only wiring, beside the other test modules. |

**Not tests I author.** These are Tier 1/3 items checked in Phase 6 or by S:

- the docs (W-7, W-5);
- the `outcomes.rs` doc comment (R2 W-4);
- the note for #139;
- the PR description items (D147-1, D147-2, the env-var choice, the deferred manual check, the pinned image evidence).

## RED confirmation

### Compile RED (against HEAD ecef67c; every production file unchanged)

Each error names a missing #147 item:

- `cargo test -p gridops-core --test gitlab_runner_script --no-run`
  - E0432 unresolved imports `GITLAB_DEFAULT_JOB_IMAGE`, `GITLAB_EXIT_VERIFY_UNAVAILABLE`, `GITLAB_JOB_RECEIVED_SIGNAL`, `GITLAB_READY_MARKER`, `GITLAB_RUNNER_IMAGE`, `GITLAB_START_READY_TIMEOUT_SECS`, `GITLAB_VERIFY_MAX_TIME_SECS`, `gitlab_runner_env`, `gitlab_runner_script` and `valid_gitlab_url`.
- `cargo test -p gridops-host-agent --test docker_gitlab --no-run`
  - E0432 for the same core items.
  - E0026: `HelperResponse::Failed` does not have a field named `diagnostic`.
  - 3 × E0277 at `docker_gitlab.rs:500-503`, a knock-on of the unresolved `gitlab_runner_env`.
- `--test dispatch --no-run`: 6 × E0559, `HelperResponse::Failed` has no field named `diagnostic`.
- `--test docker_runtime --no-run`: 1 × E0559, the same.
- `cargo test -p gridops-api --test fleet_http --no-run`: 4 × E0559, the same.
- `cargo test -p gridops-manager --no-run`
  - E0432 and E0425: `gridops_core::provisioning::gitlab_runner_env` and `gitlab_runner_script` not found.

**F should expect** that until each item exists, these test binaries don't compile, so the existing tests in them can't run: `dispatch`, `docker_runtime`, `fleet_http` and the manager's bin tests. That is inherent to RED over new items in Rust (the #137 precedent).

### Runtime RED once the core items exist

Measured with a throwaway core draft and today's manager code. `TMPDIR=/tmp cargo test -p gridops-manager -- gitlab exit_reason`:

    valid_gitlab_requests_pass_validation_on_both_providers   FAILED  docker: Err(BadRequest("GitLab runners are not supported yet: Docker is #147, Tart is #142."))
    gitlab_runners_launch_the_shared_script_...               FAILED  Error: BadRequest("GitLab runners are not supported yet: ...")
    a_runners_exit_reason_survives_the_managers_reparse        FAILED  left: Null  right: "idle_expired"
    a_gitlab_runner_starts_labelled_with_its_token_only_on_stdin FAILED  left: 400 right: 201 {"code":"bad_request","error":"GitLab runners are not supported yet: ..."}
    exited_gitlab_runners_are_listed_with_why_they_ended       FAILED  left: {all None}  right: {..01: "completed", ..02: "idle_expired", ..03: "registration_refused", others None}

Each fails on the feature's own assertion.

### Guards (green by design, not RED)

These pass today only because the placeholder refuses every GitLab request:

- `invalid_gitlab_requests_are_refused_without_echoing_the_token`;
- `a_gitlab_runner_is_refused_without_a_runner_docker_socket`.

They are paired with the RED tests above, which require the valid request to pass, and they are not vacuous. A mutation check (below) showed each one fails when a validator lets a forbidden field through, or when the socket check is missing.

### The harness checked against a throwaway draft (written into the source files, never staged, reverted)

The stand-ins are the most likely way for these tests to fail F for the wrong reason. So I wrote a minimal draft of each production change, ran the suite, ran mutations, then reverted all source files to HEAD with `git checkout` and the saved copy. Now only test files differ from HEAD. The draft is not a suggestion for F's code.

- **Draft GREEN.** Every affected suite passed:
  - `gitlab_runner_script` 11/11;
  - `gridops-host-agent` all targets (`docker_gitlab` 10/10 in about 23 s, `dispatch` 8/8, `docker_runtime` 2/2, lib 8/8);
  - `gridops-manager` 111/111;
  - `gridops-api --test fleet_http` compiles.
- **Mutations caught.** Each mutation below failed the named test:

| Mutation | Test that failed |
|---|---|
| Token exported, or put in curl's argv | `a_verified_runner_says_it_is_ready_and_runs_one_job` |
| Every run read as completed | `a_runner_that_never_got_a_job_exits_idle_expired` |
| The idle line taken as the job signal | `a_runner_that_never_got_a_job_exits_idle_expired` |
| The 403 code changed | `a_refused_token_exits_registration_refused_without_running` |
| Verify-unavailable exiting 1 | `a_gitlab_that_cannot_answer_exits_verify_unavailable` |
| The marker on stderr | `a_verified_runner_says_it_is_ready_and_runs_one_job` and the idle test |
| `--docker-disable-cache` dropped | `a_verified_runner_says_it_is_ready_and_runs_one_job` |
| Exit 77 treated as a runtime failure | both refusal tests |
| No `rm -f` after a failed Start | all four failed-start tests and the never-ready test |
| GitLab reasons given to GitHub exits | `an_exited_gitlab_runner_inspects_as_gone_saying_why` |
| Token passed as container env | `a_verified_gitlab_runner_starts_with_its_token_only_on_stdin` |
| A 5 s readiness wait | `a_runner_that_never_gets_ready_is_given_up_within_the_deadline` |
| No Prepare refusal | `a_gitlab_prepare_without_the_docker_socket_is_refused` |
| No URL check | `a_gitlab_start_with_unusable_material_creates_nothing` |
| `dispatch::outcome` dropping the diagnostic | `a_helper_diagnostic_reaches_the_command_outcome` |
| No platform label | `a_gitlab_runner_starts_labelled_...` |
| No socket check | `a_gitlab_runner_is_refused_without_a_runner_docker_socket` |
| `jitConfig` let through | `invalid_gitlab_requests_are_refused_...` |
| No `exitReason` | `exited_gitlab_runners_are_listed_with_why_they_ended` |

### Hygiene

- `cargo fmt --all -- --check` is clean.
- `cargo +1.98.1 clippy -p gridops-core -p gridops-host-agent -p gridops-manager -p gridops-api --all-targets` (with the drafts in place, so everything compiled) reports nothing in any test file. The only diagnostic was in the throwaway draft.
- Untouched suites still pass at HEAD:
  - `gridops-core --lib`: 261;
  - `gitlab_contract`: 14;
  - `fleet_agent_outcomes`: 8;
  - `gridops-host-agent --lib`: 8;
  - `--test version`: 1.
- **Environment, not #147.** The manager's route-level tests (the existing `provision_lock_tests` too) fail on this workstation with `path must be shorter than SUN_LEN`. This session's `TMPDIR` (`/var/folders/.../T/`) makes the stand-in Docker socket path too long. Run them with `TMPDIR=/tmp`. CI (Linux, `/tmp`) is unaffected.

## Ready for F

**RED is valid. F may implement against these tests.** Where the brief left a name or shape open, the tests take one reading. F implements to it, or tells T if one contradicts the brief:

1. **Core items in `gridops_core::provisioning`:**
   - `GITLAB_RUNNER_IMAGE: &str`, a `gitlab/gitlab-runner:vX.Y.Z` or `ubuntu-vX.Y.Z` tag;
   - `GITLAB_DEFAULT_JOB_IMAGE: &str`;
   - `GITLAB_READY_MARKER: &str`;
   - `GITLAB_JOB_RECEIVED_SIGNAL: &str`;
   - `GITLAB_START_READY_TIMEOUT_SECS: u64 = 15`;
   - `GITLAB_VERIFY_MAX_TIME_SECS: u32 = 10`;
   - `GITLAB_EXIT_VERIFY_UNAVAILABLE: i32`, in 3..=125 and not 75 or 77;
   - `pub fn gitlab_runner_script() -> String`, constant text;
   - `pub fn gitlab_runner_env(url: &str, wait_timeout_seconds: u32) -> Vec<String>`, `NAME=value` entries that fill in the job image themselves;
   - `pub fn valid_gitlab_url(url: &str) -> bool`.
2. **The script:**
   - calls `curl` and `gitlab-runner` by name from PATH, not `/usr/bin/...`, so the stand-ins are used;
   - reads one line of stdin;
   - prints the marker as a whole line on **stdout**;
   - lets gitlab-runner's stdout and stderr through;
   - verify posts to `$URL/api/v4/runners/verify` with form field `token`;
   - flags may be `--flag value` or `--flag=value`.
3. **NV-3 evidence I gathered, for F to confirm and cite.** gitlab-runner `commands/single.go` at v19.6.0 (fetched from gitlab.com raw), `checkFinishedConditions`, lines 167-175:
   - it logs "This runner has processed its build limit, so now exiting" once `--max-builds` is used up;
   - it logs "This runner has not received a job in N seconds, so now exiting" when `--wait-timeout` passes;
   - `Execute` returns normally either way, so the exit status is 0 in both cases and only the output tells them apart;
   - `postBuild` (lines 76-81) decrements `MaxBuilds` and resets `lastBuild` only after a build ran, so a long job does not also log the idle line;
   - a job that was received but never ran (`NewBuild` or `ProcessJob` failing, or `UpdateAbort`, lines 103-129) returns before `postBuild`, so it ends as idle.

   So the signal of a job that ran is the build-limit line.

   The token is `common/config.go:1466` `Token ... long:"token" env:"CI_SERVER_TOKEN"` (the URL is `:1462`, `--url`/`CI_SERVER_URL`). `HandleArgs` (single.go:192-195) refuses to start without it.

   The tests print `GITLAB_JOB_RECEIVED_SIGNAL` as its own line for a completed run, and the idle line verbatim for an idle one.
4. **Fleet Start:**
   - The Prepare refusal is in the helper (`docker.rs`), not only in dispatch.
   - A GitLab Start that fails after `create` reports exit 77 as `RegistrationRefused` even when `feed_secret` errors (A warning 1).
   - Every failed-Start outcome carries `diagnostic: None` except the refusal.
   - The image is `prepared.image`, followed by a `bash` (any path ending in `bash`), with the shared script as the last argument.
5. **Manager:**
   - the platform label is `io.gridops.platform=gitlab`;
   - the listing reads `State.ExitCode` via `inspect_container` (the stand-in answers that);
   - `exitReason` is omitted, not `null`, when absent;
   - the no-socket refusal is a 400.
