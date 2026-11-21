# Handoff-T-green: Phase 7 - GitLab: run gitlab-runner in Linux Docker runners (#147)

**Date:** 2026-11-20
**Branch:** claude/gitlab-147-hostagent-e4a01d (head 735ed91)
**Issue:** #147
**Handoff-F reviewed:** docs/handoffs/147/handoff-F.md
**Handoff-T-red:** docs/handoffs/147/handoff-T-red.md

## GREEN confirmation

F reported "Tests that look wrong (for T): None". I re-ran everything myself; no test was changed in this phase.

    cargo test -p gridops-core --test gitlab_runner_script
      11 passed; 0 failed
    TMPDIR=/tmp cargo test -p gridops-core -p gridops-host-agent -p gridops-manager -p gridops-tart-agent -p gridops-reconciler
      every binary ok; none failed (docker_gitlab 10/10 in 15.3 s, dispatch 8/8, docker_runtime 2/2,
      manager 111/111 incl. gitlab_runner_tests, core script 11/11)
    TMPDIR=/tmp cargo test -p gridops-api --test fleet_http
      143 passed; 1 failed: release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine

The one failure is environmental and not #147's. The fixture's "Linux machine" runs the real native backend, which takes its OS from `cfg!(target_os)` (`crates/gridops-host-agent/src/native.rs:199-206`), so on this Mac the preflight sees macOS ARM64 and passes its machine check. #147's only change to `native.rs` is `diagnostic: None` in `failed()`. It reproduces F's report exactly. CI runs on Linux.

**Behavior spot-check (mutation).** I changed `GITLAB_EXIT_IDLE_EXPIRED => diagnostic(IdleExpired)` to `=> None` in `docker.rs`. `an_exited_gitlab_runner_inspects_as_gone_saying_why` failed. I reverted with `git checkout`; the tree is clean. The RED phase's mutation table (token exported or in argv, missing socket check, and others) already covered the script and manager paths.

## Tier 1 results

| Check | Command | Result |
|---|---|---|
| Build, lint | `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (CI's command) | PASS, no diagnostics |
| Format | `cargo fmt --all -- --check` | PASS |
| Existing tests still pass | the runs above | PASS (apart from the environmental `fleet_http` case) |
| Server starts / API smoke | the manager's real router is driven by `gitlab_runner_tests` (POST and GET `/v1/runners`); the fleet runtime is driven by `docker_gitlab` through a stand-in `docker` CLI | PASS |

## Tier 2 results

- **Test coverage per criterion:** PASS (see below).
- **Test quality (test-quality.md section 7):** PASS. Each test names a behavior and sits at the cheapest tier: serde/constants as unit, the script against real bash and curl, the runtime against a stand-in CLI, the manager through its router. Two tests are guards that pass by design on the placeholder and are paired with RED tests (T-red documents this); they fail under mutation. No redundant test found to delete or merge. The suite is proportionate: about 40 new tests for 10 production files.
- **Type safety:** PASS. No `any`-style escapes. Clippy is clean at `-D warnings`.
- **Error handling:** PASS. Refused (77), verify-unavailable (69), other exit, never-ready timeout, unparseable material, persistent mode, bad URL and missing socket are each tested. Each failed Start removes its container (A warning 1).
- **Data integrity:** PASS. A retried start finds its existing container. Only a successful Start keeps the container. The exit status is read before `rm -f`.
- **API contract:** PASS. `HelperResponse::Failed` keeps its old wire form when `diagnostic` is `None` (byte-for-byte test) and still refuses unknown fields. `exitReason` appears only for an exited GitLab runner. A Tart entry's `exitReason` survives the manager's re-parse. `outcomes.rs` changes a doc comment only.
- **Security:** PASS. The token is on stdin only: not in `docker create` args, container env, labels, curl argv or the main process environment (tests cover create args, curl argv/env, the stdin bytes, and the manager's create body). It is set as `CI_SERVER_TOKEN` on the `gitlab-runner` child only. Manager validation errors never echo the token. GitHub/Bitbucket secrets are refused on a GitLab request.
- **Migration safety:** N/A, no schema change.
- **Playwright:** N/A, no UI surface and no Playwright suite touched.
- **Evidence appendix:** `docs/handoffs/147/evidence.md` exists with F's entries (socket-flag derivation, existing Prepare `Permission` refusal, `reported_gone`, 30 s deadline and 5 s margin, `CiTarget::is_gitlab`, manager socket bind). Those cover the unchanged-code facts my tests rely on. No gap found.
- **Cross-check of F's results:** matches F's report, including the one environmental failure. Docs spot-check: `docs/fleet-agent.md` and `compose.no-docker-socket.yaml` name `GRIDOPS_RUNNER_DOCKER_SOCKET` and `GRIDOPS_AGENT_DOCKER_SOCKET`, and state that the no-socket compose turns off GitLab runners too (W-7). The `RegistrationRefused` doc comment states Start and Inspect carriage (R2 W-4).

## Acceptance criteria status

| Criterion | Status | Backing test |
|---|---|---|
| Token not in `docker inspect` or main-process environ | PASS | `a_verified_gitlab_runner_starts_with_its_token_only_on_stdin`, `a_verified_runner_says_it_is_ready_and_runs_one_job`, manager `a_gitlab_runner_starts_labelled_with_its_token_only_on_stdin` |
| Completed / idle expiry / refused map to the right outcome | PASS | script tests (0 / 75 / 77 / 69), `a_token_refused_*`, `an_exited_gitlab_runner_inspects_as_gone_saying_why`, `exited_gitlab_runners_are_listed_with_why_they_ended` |
| Pinned images, `:latest` refused | PASS | `the_gitlab_images_are_pinned_releases` |
| Start fits the 30 s deadline | PASS | `the_gitlab_start_waits_fit_inside_the_start_deadline`, `a_runner_that_never_gets_ready_is_given_up_within_the_deadline` |
| W-5 Prepare refuses without the socket | PASS | `a_gitlab_prepare_without_the_docker_socket_is_refused` |
| D6 advertisement | PASS | `the_docker_backend_offers_ephemeral_gitlab_runners` |
| B-3 diagnostic channel, GitHub wire form unchanged | PASS | `helper_failures_keep_their_wire_form_and_may_carry_a_diagnostic`, `a_helper_diagnostic_reaches_the_command_outcome` |
| D10 manager validation, launch, label, 400 without socket | PASS | manager validation tests and `gitlab_runner_tests` |
| D15 `exitReason` re-parse | PASS | `a_runners_exit_reason_survives_the_managers_reparse` |
| Manual check with a real `glrt-` token | NOT RUN, deferred to #143 per the brief. The PR description must say so (O's Phase 11/12 item). Not faked. |

## Blocking issues

None.

## Advisory notes

- Not exercised here: the real `gitlab/gitlab-runner` image, a real GitLab, and a real Docker daemon (the stand-in CLI and a stand-in runner were used). NV-1 to NV-6 rest on F's cited sources. This is the #143 gap the brief records; U does not apply (no UI).
- The `release_preflight` failure is macOS-workstation only; confirm green on CI.
- `wait_timeout_seconds = 0` makes `run-single` wait forever; neither path rejects it (recorded for #139).
- The PR must carry: D147-1, D147-2, the env-var choice (brief review W-1), the pinned image values with their registry checks, the `system_id` note, and the deferred manual check.
