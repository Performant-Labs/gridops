# Handoff-S: Phase 8 - GitLab: run gitlab-runner in Linux Docker runners (#147)

**Date:** 2026-11-20
**Branch:** claude/gitlab-147-hostagent-e4a01d (head 6fc3cc5; implementation diff `b22c451..6fc3cc5`)
**Issue:** #147
**Brief audited against:** `docs/handoffs/147-brief.md`, all amendment sections (architecture review R1 and R2, brief review R1)
**Handoff-T reviewed:** `docs/handoffs/147/handoff-T-green.md` (and `handoff-T-red.md`)
**Handoff-A reviewed:** `docs/handoffs/147/handoff-A.md` (round 3, PASS) and `handoff-A-dup.md` (PASS)
**Handoff-F reviewed:** `docs/handoffs/147/handoff-F.md`
**Also read:** `docs/handoffs/147-diff-result-r1.md` (the diff review, PASS with 3 warnings), `decisions.md`, `evidence.md`
**Operator-facing report:** N/A. No visual surface (Rust back end, docs, a shell script), so no HTML report.

## A precondition
Confirmed: A returned PASS in round 3 (two warnings, both acted on by F) and PASS in the anti-duplication gate (`handoff-A-dup.md`, two non-blocking warnings).

## T precondition
Confirmed: T reported zero blocking issues (`handoff-T-green.md`, "Blocking issues: None").

## Visual-diff-tool precondition
N/A. No UI surface. The Browser-tool and Visual-diff-tool preconditions apply to visual / web projects only. No Playwright suite is touched.

## Visual diff results
N/A (no rendered output).

## Preview / spec sanity check
Checked the brief and issue against the project's conventions for a protocol and runner back end. No source-of-truth defect found that justifies ADVISORY-HOLD:
- The issue text ("default image ... overridable by the pool's `docker_image`") conflicts with how the server chooses images today. The brief resolves it explicitly (D147-1) rather than leaving it open, and F implements the resolution.
- The `DiagnosticCode::RegistrationRefused` doc comment contradicted the Inspect behaviour the brief needs. The brief records this (D147-2) and F edits only that comment.
- One upstream property worth knowing, not a spec defect: `gitlab-runner run-single` logs the first 8 characters of the token body as its "short description" on its first line (`helpers/shorten_token.go`, `single.go` `Execute`). It is on stdout, which the brief requires to reach `docker logs`. The brief's "never in env, args or `docker inspect`" is met; see Advisory notes.

## What I verified myself (not only T's and F's reports)

| Check | Result |
|---|---|
| `TMPDIR=/tmp cargo test -p gridops-core -p gridops-host-agent -p gridops-manager -p gridops-tart-agent -p gridops-reconciler` | 30 result lines, 610 passed, 0 failed. Matches F's and T's figure (script 11/11, `docker_gitlab` 10/10, dispatch 8/8, manager 111/111). |
| `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (CI's command) | Clean. |
| `cargo fmt --all -- --check` | Clean. |
| `fleet_http release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` | Reproduced. The output shows `PASS macos-machine: macOS ARM64 machine ready ...` for a fixture that is meant to be Linux, because the native backend reads the OS from `cfg!(target_os)` on this Mac. Unrelated to #147 (its only change to `native.rs` is `diagnostic: None`). Environmental; CI runs on Linux. |
| Only owned files changed (`git diff b22c451..HEAD --stat`, excluding `docs/handoffs`) | 28 files, every one in the brief's NIT-2 list or a test file for it. No edit to `crates/gridops-tart-agent`, the reconciler, API `src`, `fleet/targets.rs`, `agent.rs`, `Cargo.lock` or any shared contract type. `outcomes.rs` is a doc-comment change only. |
| Raw bytes of the script's token line | `CI_SERVER_TOKEN="$S" gitlab-runner run-single ...` (confirmed with `od -c`; tool output had masked it). |
| No TODO, FIXME, `dbg!`, `println!`, `unwrap()`, `expect(` or `#[allow` added in the diff | None found. |
| External fact NV-1 / NV-3 (gitlab-runner v19.6.0, `commands/single.go`) | Fetched from gitlab.com. `checkFinishedConditions` logs "This runner has processed its build limit, so now exiting" and "This runner has not received a job in N seconds, so now exiting"; `postBuild` runs only after a build; `Execute` returns normally either way. Matches `GITLAB_JOB_RECEIVED_SIGNAL` and the private idle prefix. |
| External fact NV-2 (`common/config.go`) | Line 1466: `Token ... long:"token" env:"CI_SERVER_TOKEN"`; line 324: `disable-cache`. `HandleArgs` refuses to run without a token. |
| External fact NV-4 (GitLab `lib/api/ci/runner.rb`, `lib/api/ci/helpers/runner.rb`, master) | `POST /runners/verify` requires `token`, answers 200 or `forbidden!`. `system_id` is declared `optional` and `current_runner_manager` falls back to `LEGACY_SYSTEM_XID`, so F's decision to omit it holds. |
| Pinned images on Docker Hub | `gitlab/gitlab-runner:ubuntu-v19.6.0` is `active` (updated 2026-11-19, linux amd64/arm64/ppc64le/riscv64/s390x). `alpine:3.24.4` is `active` (updated 2026-11-08). |
| Shared notes for #139 and #142 | `/Users/andreangelantoni/ClaudeAangel100/side-session-notes/local_e1c0b548-f83a-4d0b-8ba3-069dc9dfc38b/147-hostagent-notes.md` carries D147-1, the Start/Inspect outcome branching, the `requires_docker_socket` requirement and the merge-order hook. |

## Spec compliance

| Requirement | Compliant | Evidence |
|---|---|---|
| Issue: `run-single --url --executor docker --docker-image <job image> --max-builds 1 --wait-timeout`, plus `--docker-disable-cache` (R2 W-5) | YES | `GITLAB_RUNNER_SCRIPT` in `provisioning.rs`; test `a_verified_runner_says_it_is_ready_and_runs_one_job` checks every flag. Flags confirmed at v19.6.0 (NV-1). |
| Issue: token read from stdin, never in env, args or `docker inspect` | YES | Non-exported `S`; verify via `printf '%s' "$S" \| curl --data-urlencode 'token@-'`; runner gets `CI_SERVER_TOKEN` on the child only. Tests: `a_verified_gitlab_runner_starts_with_its_token_only_on_stdin` (no create arg holds it, stdin is exactly `token\n`), the script test (curl argv and env clean, stdin only), manager `a_gitlab_runner_starts_labelled_with_its_token_only_on_stdin` (create body clean). `/proc/1/environ` of a `--init` container is the container's `Config.Env`, which holds no token (see Advisory note 1). |
| Done when: each exit path maps to the right outcome | YES | Script: 0 completed, 75 idle, 77 refused, 69 verify-unavailable, other non-zero passed on. Fleet Start: 77 gives `Configuration, false, RegistrationRefused`; anything else `Runtime, true`; the container is removed first (A warning 1). Fleet Inspect: `ResourceUnavailable, false` with `IdleExpired` (75), `RegistrationRefused` (77), else `None`. Manager: `exitReason` from `GITLAB_EXIT_REASONS`. |
| Done when: manual check with a real `glrt-` token | NOT RUN, as the brief requires | Deferred to #143. Not faked. The criterion #147 can meet is that the PR says so explicitly: that is an O item at PR time (see PR obligations). |
| Docker access and docs | YES | Prepare refuses a GitLab profile without `requires_docker_socket` (`Configuration, false`, before any pull). Start re-checks. `docs/fleet-agent.md` "GitLab runners" names `GRIDOPS_AGENT_DOCKER_SOCKET` with the grant and profile, and `GRIDOPS_RUNNER_DOCKER_SOCKET`. The header of `compose.no-docker-socket.yaml` and the existing paragraph say the no-socket compose turns off GitLab runners too. |
| D1 / W-4: parse the core `GitlabRunnerMaterial`; refuse Persistent and unparseable material | YES | `start_gitlab` parses the core type; no private copy in `docker.rs`. Test `a_gitlab_start_with_unusable_material_creates_nothing`. |
| Brief review W-2: fleet URL check | YES | `valid_gitlab_url` (core) is used by Start and the manager; refuses ftp, unparseable, credentials, whitespace. |
| D5 outcomes, B-2, B-3 | YES | As above. `HelperResponse::Failed.diagnostic` is `skip_serializing_if` none (byte-for-byte wire test), keeps `deny_unknown_fields`; `dispatch::outcome` passes it through; a GitHub runner exiting 75 or 77 gets no diagnostic (test). |
| D6 advertisement | YES | `docker_backend` adds `gitlab_project` and `gitlab_group`, ephemeral; test pins the exact set of six. |
| D10 manager arm | YES | Placeholder replaced; validation (fields present, ephemeral, `glrt-` prefix, http(s) URL, no other platform's secret) on both providers; Tart is forwarded before `provision_runner_inner` (checked: `start_runner` branches on provider first), so the socket refusal cannot hit a Tart GitLab start; `runner_command` and `take_bootstrap_secret` branch on platform; `io.gridops.platform=gitlab` label; 400 without `shared_docker_socket`; `exitReason` via `inspect_container`; token never in an error message (test). |
| D15 `ManagedRunner.exit_reason` | YES | `skip_serializing_if`; test pins survive-and-absent. |
| D12 | YES | `wait_timeout_seconds` reaches `--wait-timeout` on both paths via `GRIDOPS_GITLAB_WAIT_TIMEOUT`. |
| B-1 / D147-1: no image constant or fallback in `docker.rs` or the manager | YES | Both constants live in `provisioning.rs`; the agent and manager run the prepared or requested image. The tests pin both values and refuse `:latest`. `GITLAB_RUNNER_IMAGE` is not referenced from `docker.rs` or `main.rs`. |
| R2 B-2 deadline: 15 s wait, 10 s verify, 30 s less 5 s | YES | Constants pinned with doc comments citing both deadlines; tests `the_gitlab_start_waits_fit_inside_the_start_deadline` and `a_runner_that_never_gets_ready_is_given_up_within_the_deadline`. `START_COMMAND_DEADLINE_MILLIS` is unchanged. |
| R2 W-2 compile fallout | YES | All 15 `HelperResponse::Failed` sites are covered; the API crate edit is the 4 `diagnostic: None` lines in `placement.rs`, test-only. |
| R2 W-3 env vars; A warning 2 | YES | The script text is constant; `gitlab_runner_env` and `valid_gitlab_url` live with the script in core; a test checks the script reads every name the builder sets and holds no interpolated value. |
| R2 W-4 / D147-2 doc comment | YES | Only the `RegistrationRefused` doc text changes. |
| R2 W-5 leftovers | YES | `--docker-disable-cache`; orphaned job containers after a mid-job kill are documented as a known gap for #143 in `docs/fleet-agent.md`. |
| W-8 separate script; skips the GitHub shared work folder | YES | `gitlab_runner_script()` is its own function; `start_gitlab` mounts only the socket; a test asserts nothing under `SHARED_WORK_ROOT` is mounted. |
| Boundaries: no change to shared contract types, Tart agent, reconciler, API, targets | YES | Verified from the diff stat above. |

## Quality audit

| Area | Result | Notes |
|---|---|---|
| API consistency | PASS | `exitReason` is optional and absent unless a GitLab runner exited with a table status; `HelperResponse::Failed` is wire-compatible when `diagnostic` is `None`. |
| Error handling | PASS | Every failed Start removes its container after reading its exit status; refusals are non-retryable, unreachable-GitLab and unknown exits are retryable; manager messages never echo the token. |
| UI/UX match to spec | N/A | No UI. |
| Accessibility | N/A | No UI. |
| Architecture gate | PASS | A round 3 PASS and the anti-duplication PASS stand. Nothing I found conflicts with either. The `create_args` extraction keeps the GitHub flags byte-for-byte (`docker_runtime` passes). |
| Code organization | PASS | One shared script, env builder and URL check in core; the GitLab branch is a separate `start_gitlab` plus `start_gitlab_runner`; no dead code, TODO or debug output. |
| Security | PASS | Token on stdin only. Not in create args, labels, container env, curl argv or environment, the main process environment, or any error text. Set as `CI_SERVER_TOKEN` on the `gitlab-runner` child only (a process's environment is readable only by its uid or root, unlike argv). URL validated before use; the verify call sends no `system_id` (deliberate, documented). The Docker socket grants the runner host-level reach; that is the brief's accepted design and is documented. |
| Performance | PASS | The Start wait polls `docker inspect` and `docker logs` every 200 ms for at most 15 s; see Advisory note 3. |
| Visual regression | N/A | No UI. |
| Naming consistency | PASS | Constants and wire names follow the `GITLAB_*` family and the brief's terms. |
| Test quality (`testing/test-quality.md` section 7) | PASS | See below. |

### Test-quality audit (section 7, applied to every test in the diff)

About 40 new tests for 10 production files; proportionate for a feature story with three layers (script, fleet runtime, manager).
- **Tier and shape.** Pure logic and pinned values sit at unit tier; the script runs for real in `bash` with the real `curl` against a local server; the fleet runtime runs against a stand-in `docker` CLI that pins outcomes, not exact `docker` calls; the manager runs through its real router. No test re-proves another tier's branch. No e2e is needed.
- **Fails for the right reason.** T's RED phase recorded 21 behaviour mutations that each failed the named test, and T's GREEN phase re-ran one (idle-diagnostic to `None`). The code I read is consistent with that evidence: the exit status is read first, `rm -f` runs next, and only exit 77 takes the refusal arm of `start_gitlab`'s `match` (every other exit, 69 included, takes the `_` arm).
- **Behaviour, not structure.** The assertions on `docker create` arguments are the only observable at the stand-in tier for "the token is not in `docker inspect`", and they assert effects (no argument holds the token, the environment entries and mounts the runner needs), not call counts.
- **Smells (section 6).** None of: assertion-free, unreachable outcome, snapshot, padding, flaky-by-design. The pinned-value tests (`assert_eq!` on the 15 s and 10 s constants, the pinned images) are required by name in the brief (B-1, R2 B-2). Two guard tests are green on the placeholder by design and are paired with RED tests (T-red documents this).
- **Merge candidates (non-blocking, see Advisory note 4).** `a_token_refused_at_once...` and `a_token_refused_during_the_wait...`, and `an_unanswered_verify...` and `any_other_exit...`, exercise one `match` arm of `start_gitlab` each pair. They are separate tests because the brief names each timing and each exit code, and they cost 0 to 3 s in parallel. No "delete" finding; a table-driven merge would be tidier.

## Scope check
F delivered the brief's scope: the GitLab branch of `docker.rs` and its helper and dispatch plumbing, the shared script and constants in `provisioning.rs`, the manager arm (validation, launch, label, `exitReason`, `exit_reason`), the docs, and the mechanical fallout. No over-delivery: the one behaviour change outside GitLab is that the manager no longer sets `RUNNER_ALLOW_RUNASROOT` on a GitLab container (GitHub still gets it), which the brief's separate GitLab launch implies. No under-delivery apart from the manual check the brief itself defers to #143.

## Verdict

**PASS** - all acceptance criteria that #147 can meet are met, the implementation matches the amended brief, tests are green on an independent run, and quality is acceptable. Ready for O to commit.

## Advisory notes
Non-blocking. None needs a rework cycle.

1. **"Main process `/proc/<pid>/environ`" is shown by mechanism, not by a real daemon.** With `--init`, PID 1 is Docker's init and its environment is the container's `Config.Env`, which holds no token. The script keeps the token in a non-exported variable; the script test shows curl, a child of the main shell, inherits none. The `gitlab-runner` child does hold it as `CI_SERVER_TOKEN` (not PID 1; A's round-3 note and the brief's B-2 accept this). A test against a real daemon and the real image is #143's.
2. **`run-single` prints the first 8 characters of the token body** in its startup line (`Starting runner for <url> with token <short> ...`) to stdout, which reaches `docker logs` and the fleet's live log. This is upstream behaviour and the brief requires the runner's output to be visible; it is not env, args or `docker inspect`. #143 can confirm on the real runner what the line shows. Worth a sentence in the PR.
3. **Diff-review warnings W-1 to W-3 were not acted on; I agree none blocks.**
   - W-1, the retried-start shortcut (`docker.rs:318`): it returns `started` for any container already named `gridops-<environment>` without reading its state. This is exactly the GitHub path's behaviour (`docker.rs:139`). A created-but-never-fed or already-exited GitLab container is reported started, then Inspect reports it `ResourceUnavailable` (with `RegistrationRefused` for 77), so it is cleaned up. Eventually consistent, so not a regression.
   - W-2, no direct test of the script's own environment: covered by mechanism (note 1).
   - W-3, `docker logs` polled every 200 ms for at most 15 s (`docker.rs:685`): about 75 small calls on a slow start; the log holds a few lines before the marker. Acceptable.
   - A related pre-existing property: `docker()` has a 10-minute per-call timeout (`docker.rs:826`), so a hung daemon could outlast the 15 s wait. The GitHub Start has the same exposure; the brief accepts the 30 s bound (A round 3).
4. **Optional test tidy:** merge the four failed-start tests into one table-driven test (exit 77 at 0 s and 3 s, 69, 1) if the suite is ever trimmed; the brief names each case, so they stay for now.
5. **`wait_timeout_seconds = 0` makes `run-single` wait forever** (`--wait-timeout 0` disables the idle exit). Neither path rejects it; the contract allows it and #139 sends 600. Already in the notes for #139. A zero from a future caller would hold a placement indefinitely, so #139 should keep sending a positive value.
6. **Docs nit:** `docs/fleet-agent.md:311` is one 139-character line (the paragraph was not re-wrapped after the token sentence was added). Cosmetic.
7. **Untested against reality (by design):** the real `gitlab/gitlab-runner:ubuntu-v19.6.0` image, a real GitLab and a real Docker daemon. NV-1 to NV-4 and NV-6 rest on cited sources; I independently confirmed NV-1 to NV-4 and the two image tags above. NV-6 (Ubuntu base with `bash` and `curl`) rests on F's manifest reading and is confirmed end to end only by #143.

## PR obligations for O (Phase 11/12; the PR does not exist yet and #136 has no D147 comment yet)

The PR description must state, because the brief's remaining Done-when criterion depends on it:
1. **The manual check was not run in #147 and is deferred to #143** (no test project or token exists). Not faked.
2. **D147-1** (the pool's `docker_image` is the gitlab-runner image on both paths; the server, #139, applies `GITLAB_RUNNER_IMAGE` when a GitLab pool has none) and **D147-2** (late exit 77 is Inspect `ResourceUnavailable` with `RegistrationRefused`; the doc comment is updated). Both also go on epic #136.
3. **The env-var choice** (brief review W-1): URL, wait timeout and job image reach the script as `GRIDOPS_GITLAB_URL`, `GRIDOPS_GITLAB_WAIT_TIMEOUT` and `GRIDOPS_GITLAB_JOB_IMAGE` on both paths, within D1's permission, so #139 and #142 do not diverge.
4. **The exact pinned values and their registry checks** (brief review W-3): `GITLAB_RUNNER_IMAGE = gitlab/gitlab-runner:ubuntu-v19.6.0` and `GITLAB_DEFAULT_JOB_IMAGE = alpine:3.24.4`, each checked with `curl https://hub.docker.com/v2/repositories/<repo>/tags/<tag>` (answer: `active`, with the update dates above).
5. **The `system_id` note:** verify sends the token only; GitLab's docs say `system_id` is required for `glrt-` tokens but its source makes it optional (legacy-manager fallback). If GitLab enforces the documented rule later, the effect is a retryable verify-unavailable, not a silent failure.
6. **Known gaps:** job containers orphaned by a mid-job stop (#143); the one environmental `fleet_http` failure on macOS workstations; Done-when's `/proc/<pid>/environ` is shown by mechanism (note 1).
