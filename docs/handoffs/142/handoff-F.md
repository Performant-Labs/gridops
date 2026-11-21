# Handoff-F: Phase 5 - GitLab macOS runners on Tart pools, Tart agent side (#142)

**Date:** 2026-11-20
**Branch:** claude/gitlab-142-tart-13d6d4 (on top of 48c5c0e, T's RED)
**Issue:** #142

## What was done

- `crates/gridops-tart-agent/src/gitlab.rs` (new): GitLab's own code. It holds:
  - `validate` (called from `ProvisionRunner::validate`);
  - `runner_script`, the guest script;
  - `exit_reason` (the `GITLAB_EXIT_REASONS` lookup);
  - `DEFAULT_GITLAB_RUNNER_BINARY`;
  - the mount for T's `gitlab_tests.rs`.
- `crates/gridops-tart-agent/src/main.rs` (production code only; T's test changes were already committed):
  - **Module:** `mod gitlab;`.
  - **`AgentState.gitlab_runner_binary`:** configured in `main()` from `GRIDOPS_TART_GITLAB_RUNNER_BINARY`, defaulting to the constant, and checked with `valid_guest_path`.
  - **`TartRecord`:** two new `#[serde(default)]` fields, `platform: Option<String>` and `exit_code: Option<i32>`. Both constructors write `platform` verbatim from the request.
  - **Constants:** a local `GITLAB_RUNNER_TOKEN_PREFIX` copy. The `#[allow(dead_code)]` attributes on the #137 items are removed.
  - **`validate`:** `("gitlab", "ephemeral") => gitlab::validate(self)?`, then `("gitlab", _)` gives "GitLab Tart runners must be ephemeral.", then the shared CPU and memory checks. The native-runtime message now names GitLab.
  - **`provision_runner`:** a `"gitlab"` arm that builds the script and hands the token over with `.take()` and `SecretString::from`.
  - **`start_runner`'s waiter:** records the exit code through the new `record_exit_code` before `tart stop`.
  - **`list_runners`:** adds `exitReason` for exited GitLab records whose code is in the table.
  - **`shell_single_quote`:** fixed in place (`'\''`).
- `scripts/prepare-tart-gitlab-runner-image.sh` (new, mode 755): prepares the base image with the pinned gitlab-runner **19.5.3**. Each architecture's binary is checked against its SHA-256 with `shasum -a 256 --check`. The script also checks that `git` works and prints the runner version.
- `docs/macos-runners.md`: a new "GitLab" section after "Bitbucket Cloud workspaces".
- `docs/handoffs/142/evidence.md`: F entries appended (NV-1/2/3 citations, plus unchanged-code facts).

## Design decisions

- **Default binary path (W-2): `/Users/admin/gitlab-runner/bin/gitlab-runner`.**
  - It is defined once, as `gitlab::DEFAULT_GITLAB_RUNNER_BINARY`.
  - The image script installs to the same literal path and accepts `GRIDOPS_TART_GITLAB_RUNNER_BINARY`. The docs state the path.
  - The image script applies the agent's `valid_guest_path` rule to the override, so it cannot install somewhere the agent would refuse.
- **How a refused token is detected: both mechanisms.**
  - **Verify first.** `POST <url>/api/v4/runners/verify`. The JSON body is written by the `printf` builtin into a pipe to `curl --data-binary @-`, and `-w '%{http_code}'` gives the status:
    - 200 continues;
    - 403 exits 77;
    - anything else exits 69 (not in the table).
    - This follows upstream's own `VerifyRunner`: 200 is valid, 403 is not valid, anything else is an error (`evidence.md`).
  - **Then `run-single`'s output.** Its output goes through `tee` to the runtime log and to a file, and is classified when it exits 0:
    - a "Checking for jobs... received" or "processed its build limit" line exits **0**;
    - otherwise "Checking for jobs... forbidden" exits **77**;
    - otherwise "has not received a job in" exits **75**;
    - otherwise it exits **70**. This is a run-single stopped by a signal, which would otherwise be mislabelled "completed".
  - Because the script also reads `run-single`'s refusal, **there is no W-1 time-of-check gap**. A token revoked after the verify call still exits 77.
  - **Latency caveat, from upstream source:** after a 403 on the job request, `run-single` sleeps `NotHealthyCheckInterval` (300 s) before it checks its wait timeout. So a mid-run refusal surfaces as 77 after about 300 s (or after the wait timeout, if that is longer), not immediately.
- **Statuses that would collide with the table.** If `run-single` itself exits non-zero with 75 or 77, the script exits 70 with a message. This keeps the promise that 0, 75 and 77 come only from the three named situations. Every other non-zero status passes through unchanged (78 stays 78).
- **The token has two homes in the guest, and nowhere else:**
  - **`CI_SERVER_TOKEN`:** a prefix assignment on the one `gitlab-runner` command, not exported.
  - **The verify request body.**
  - It is in no argv: the real `gitlab-runner` process's argv was checked with `pgrep -fl` during the real-binary run below.
- **Token character set.** Validation requires:
  - the `glrt-` prefix (`GITLAB_RUNNER_TOKEN_PREFIX`);
  - a length from 6 to 1024 bytes;
  - printable ASCII other than `"` and `\`.

  This makes it single-line with no whitespace, and safe to put in the JSON body without escaping in bash 3.2. GitLab's tokens never contain either excluded character.
- **URL validation.** Hand-rolled, as the neighbouring validators are:
  - an `http://` or `https://` prefix with something after it;
  - at most 512 bytes;
  - no whitespace and no control characters.

  `'` is **accepted**, and the quoting helper is fixed instead (A finding 4, the preferred option). `gitlab.rs` adds no quoting helper of its own. The verify call passes `--globoff`, so `{}[]` in a URL is not curl-globbed.
- **The exit code is recorded for every VM runner (A finding 1).** `start_runner` stays platform-agnostic, and `list_runners` decides by `platform == "gitlab"`.
  - `record_exit_code` takes `records.write()`, updates only through `get_mut`, and never inserts. It persists while still holding the lock, then the waiter runs `tart stop`.
  - A persist failure is logged with the runner id and the error only.
  - A signal or a wait error (`code() == None`) records nothing.
  - The crash window (W-2) is stated in a code comment at the call site.
- **Builds and cache.** Both directories are under `$HOME/gridops-gitlab-runner/` (`--builds-dir`, `--cache-dir`), and the script `cd`s there.

## Reuse / extend-vs-new

There is no `survey.md`. The baseline was the analogues the brief names, and each was extended:

- `ProvisionRunner::validate`: new tuple arms.
- `provision_runner`'s platform `match`: a new arm.
- `start_runner`'s stdin secret channel: reused unchanged. Its waiter was extended, following `start_native_runner`'s update-in-place pattern.
- `TartRecord`: serde-default fields, like `root`.
- `list_runners`: a conditional key.
- `shell_single_quote`: fixed in place, so there is no second quoting seam.
- Configuration: copied from the `GRIDOPS_TART_BITBUCKET_RUNNER_ROOT` pattern.
- The image script: modelled on the Bitbucket script (argument checks, `clone`, `trap cleanup`, the boot loop, the summary line), with the GitHub script's `shasum --check` step.

The one new object is the `gitlab.rs` module. The brief requires it (Scope 1), and A accepted it as the workspace's sibling-module convention.

## Architecture notes for A

- **Layers touched:** the Tart agent only. Nothing in `gridops-manager`, `gridops-host-agent` or `gridops-core` changed. There are no new crate dependencies.
- **Persisted schema:** `TartRecord` gains `platform` and `exitCode`, both optional and serde-defaulted. Pre-#142 records load with both `None` (tested).
- **Wire contract:** `GET /v1/runners` entries can carry `exitReason`, but only an exited GitLab entry with a code in the table (D15, as specified). The request contract is unchanged.
- **New configuration:** `GRIDOPS_TART_GITLAB_RUNNER_BINARY`. Like Bitbucket's root, it is not added to the installer or the plist (A finding 6).
- **New background writer to the record store:** the runner waiter (A finding 1), update-in-place only.
- **Contract deviation for the epic #136 table (brief W-1):** the Tart agent's `ProvisionRunner` has no `registration_token` field and no `deny_unknown_fields`. A stray `registrationToken` on a GitLab request is therefore **silently ignored, not refused**. T's `a_valid_gitlab_request_is_accepted` pins this. #139 and #147 should not expect a refusal.
- **Reachability (A finding 7):** #142 is not reachable end to end until #147 does two things in the manager:
  - lets `("gitlab", "ephemeral")` with `provider == "tart"` through its `validate`, which today refuses every GitLab request;
  - adds D15's `exit_reason` relay.

  #143's manual check should wait for both.

## Deviations from spec / wireframe

None. Wireframe: N/A.

One addition beyond the brief's minimum: the 69/70 statuses for a verify call GitLab did not answer and for an unclassifiable `run-single` exit. Both are outside the table, as "any other failure keeps its own non-table status" requires.

## Tier 1 self-check (incl. tests now GREEN)

    $ cargo test -p gridops-tart-agent
    running 51 tests
    test result: ok. 51 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.59s
    (repeated: 22 of 24 full runs fully green; the 2 failing runs were the pre-existing flakes below, which the untouched baseline shows too)

    $ cargo fmt --all -- --check          -> clean
    $ cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl
        Finished `dev` profile [unoptimized + debuginfo] target(s)      (no warnings; CI's pinned toolchain and flags)
    $ bash -n scripts/prepare-tart-gitlab-runner-image.sh && shellcheck scripts/prepare-tart-gitlab-runner-image.sh   -> clean

**Real-path checks, beyond the mocked boundary.** All headless, with no Tart VM:

- **The guest script against the real gitlab-runner 19.5.3 binary and real curl.** The binary is the pinned one, checksum-verified. The rendered script ran under `/bin/bash` with `env -i`, the token on stdin, and a local stub GitLab:
  - verify 403: exit **77** (0 s);
  - verify 200 and job request 204, with `--wait-timeout 3`: exit **75** (7 s);
  - verify 200 and job request 403: exit **77** (300 s, the upstream not-healthy interval).
  - In every case the token was in no output. The stub received it only in the verify JSON body, and `pgrep -fl` showed the real `run-single` argv without it.
  - Not exercised for real: the "job received" → 0 path. A stub would need a full job payload, so that path is covered only by T's upstream-modelled fake.
- **The image script with a fake `tart`** whose `exec -i` runs the guest step locally:
  - usage: 64; missing tart: 69; bad or `'`-containing binary path: 64;
  - the full run downloaded 19.5.3 from GitLab's bucket, `shasum --check` passed, the binary was installed 755, `--version` printed 19.5.3, and `tart stop` ran on exit.
  - Separately, both macOS binaries were downloaded and their SHA-256 matched the script's pinned values.

## Evidence appendix

`docs/handoffs/142/evidence.md`, section "F (Phase 5, implementation)":

- NV-1: tart 2.41.1 `Exec.swift` and `Root.swift`.
- NV-2: gitlab-runner v19.5.3's `run-single` flags, `CI_SERVER_TOKEN`, the log lines and `NotHealthyCheckInterval`.
- NV-3: `VerifyRunner`'s 200/403 handling and the `/api/v4/` base.
- In this repository: the delete order, the atomic `persist_record`, the stdin secret channel, the `?error` logging, and the `stopped` → `exited` mapping.

## Tests that look wrong (for T)

None of T's #142 tests looks wrong. Two **pre-existing** tests are flaky under full-suite load. They reproduce on the untouched base `229081c` (2 failed runs in 15) as well as on this branch:

- `tests::a_runner_waiting_for_a_slot_is_listed_as_waiting` fails with "the runner did not start once the slot was free" at about 30 s.
  - **Likely cause:** a lost-update race in the fake. `FAKE_TART`'s `run` branch reads `busy` and writes `busy - 1` back, while the test writes `busy=0` between that read and write. The 999 then wins, and the start waits out the 30 s slot wait.
- `tests::every_tart_command_gets_a_path_starting_with_tarts_directory` panics at its PATH assertion, as T already noted.

Neither touches #142 code. I left them alone because they are test-fixture code.

## Known issues

- A token revoked while `run-single` is already polling exits 77 only after gitlab-runner's 300-second not-healthy sleep (or after the wait timeout, if that is longer). This is upstream behaviour, and the script cannot shorten it without killing `run-single` on the first "forbidden" line. Detecting it this way still beats the brief's accepted W-1 gap, which never reported such a token at all.
- The "job received" → 0 path was verified only against T's fake, not the real binary (see Tier 1).
- **Manual check, not run, deferred to #143:** a real `glrt-` token on a real Tart VM (runner online in GitLab, runs a macOS-tagged job, exits). No GitLab test project or token exists yet.

## Files changed

- crates/gridops-tart-agent/src/gitlab.rs (new)
- crates/gridops-tart-agent/src/main.rs
- scripts/prepare-tart-gitlab-runner-image.sh (new)
- docs/macos-runners.md
