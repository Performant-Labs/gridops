# Handoff-T-red: Phase 4 (round 2, rework after diff gate r1) - Container jobs on the linux pool: runner externals and tool cache at host-valid paths (#126)

**Date:** 2026-11-22
**Branch:** claude/fix-126-externals (head 7f963e4, A's round-3 PASS)
**Brief / wireframe reviewed:** docs/handoffs/126-brief.md (unchanged since e10e8be), docs/handoffs/126-diff-result-r1.md, docs/handoffs/126/handoff-A.md (round 3, "Notes for O"), docs/handoffs/126/decisions.md. Wireframe: N/A (no UI).

Round 1 of this handoff (the full contract and the original RED) is kept below, after the divider.

## A precondition
Confirmed: A returned PASS (round 3, 7f963e4), with 0 blocks and 2 warns.
- A's "Notes for O" route two diff-gate findings to T this cycle:
  - **B-1:** add the failed-rename fill test.
  - **W-3:** pin the expected pair in the Prepare wiring test.
- A's warns 1 and 2 are F's work for this pass:
  - warn 1: `spawn_blocking` for `remove_tree`, and `tokio::fs::rename`;
  - warn 2: finish the `shared_folder` rename.
- Neither warn needs a test change.

## Why this is not a classic RED
- F's implementation is already on the branch (a27bd95), so the behaviour these tests pin already exists. Both tests pass against the current code.
- The tests close coverage holes the diff gate found. They do not specify new behaviour.
- RED validity is therefore shown by mutation instead. Each test fails for the right reason when the behaviour it pins is removed from `docker.rs` (4 mutations, all reverted; see below).
- For F's pass this round, these tests are the guard. The failed-rename test exercises exactly the line A warn 1 changes (`std::fs::rename` to `tokio::fs::rename`, and `remove_tree` of the staging folder under `spawn_blocking`).

## Tests authored
Both are in `crates/gridops-host-agent/src/docker/externals_tests.rs`. It is the only file changed; no production file was touched.

1. **`a_failed_rename_into_the_cache_is_retryable_and_leaves_no_cache_or_staging`** (new). Pins brief item 5 / decision 5: "a failed rename → `Runtime, true`, and no staging directory is taken for a cache" (diff gate B-1).
   - **Setup:** a regular file is pre-created at `<cache_root>/<hex>`. `is_real_dir` is false for a file, so the fill runs, copies, and then cannot rename the staged folder onto the file (`ENOTDIR`).
   - **Assertions:**
     - the answer is `(Runtime, true)`;
     - the reason names the image tag and the cache path;
     - exactly one `docker cp` ran, which proves the failure is the rename and not an earlier step;
     - `<hex>.partial` is gone (only `<hex>` remains in the cache root);
     - the file at `<hex>` is unchanged (still a regular file with its content);
     - Start's `externals_cache` check still refuses;
     - no fill container is left behind.
   - **Tier:** a unit test against the stand-in `docker` (the module's existing rerun pattern). This is the cheapest tier that reaches the rename: it needs the fill's real filesystem steps but no Docker daemon.
   - **Why a separate test:** it is not a sixth case in `a_docker_error_during_the_fill_is_retryable_and_leaves_no_cache`. That loop asserts `!cache_root.join(HEX).exists()`, which cannot hold when a file must stand at that path. This test also asserts things specific to a rename failure: staging removed after a complete copy, the occupant untouched, and the cp count.
2. **`a_github_prepare_sharing_the_socket_fails_when_it_cannot_fill_the_cache`** (changed). Pins decision 2's wiring and decision 5's mapping (diff gate W-3).
   - **Before:** it accepted either `(Configuration, false)` or `(Runtime, true)`, so a wrong pair for the failure that actually occurred passed.
   - **Now:** a new helper, `cache_root_creation_answer()`, finds the nearest existing ancestor of `EXTERNALS_CACHE_ROOT` and tries to create a probe folder there. That gives the pair decision 5 assigns on this machine:
     - `PermissionDenied` → `(Configuration, false)`;
     - any other error → `(Runtime, true)`;
     - success → remove the probe and skip with a reason, because the Prepare would write outside the test. The old test had no guard against this.
   - The test then asserts that the Prepare answers exactly that pair.
   - **Tier:** unchanged. It must go through `DockerRuntime::call` to cover the wiring.

## RED confirmation
- **Command:** `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-host-agent --lib <test>`.
- **Mutations:** each was applied to `crates/gridops-host-agent/src/docker.rs` and reverted with a backup copy. `git status` afterwards shows only the test file modified.
- **Results against unmutated code:**
  - both tests pass;
  - the externals module has 19 tests and all pass;
  - every `gridops-host-agent` test binary passes: lib 28, dispatch 8, docker_gitlab 10, docker_runtime 1+1+2, version 1.

| # | Mutation (behaviour removed) | Test | Result and the failing line |
|---|---|---|---|
| M1 | The rename error maps to `Refusal::configuration` | failed-rename | FAILED: `Error: Configuration, retryable false: Could not move the externals of the image harbor.example/performantlabs/pl-runner:1.70.4 into …/state/externals/2f529cfa…9491: Not a directory (os error 20)` |
| M2 | Staging is removed only after `placed?`, so a failed rename leaves it | failed-rename | FAILED: `Error: staging left behind: ["2f529cfa…9491", "2f529cfa…9491.partial"]` |
| M3 | Whatever stands at the cache path is removed before the rename | failed-rename | FAILED: `Error: Ok(…/state/externals/2f529cfa…9491)` (the file was replaced by a cache) |
| M4 | `PermissionDenied` while creating the cache root maps to `Runtime` | Prepare wiring | FAILED: `Error: a Prepare that could not fill the externals cache answered Failed { code: Runtime, retryable: true, diagnostic: None }, expected Configuration, retryable false` |

- **What M4 also shows:**
  - The Prepare test really runs on this machine; it does not skip. It runs as a non-root user, `/var/lib` is `root:wheel 755`, and there is no `/var/lib/gridops-agent`.
  - The old either-pair assertion would have passed M4. That is the W-3 gap, now closed.
- **Hygiene:**
  - `cargo fmt --all -- --check` is clean (after `rustfmt` on the test file).
  - `cargo +1.98.1 clippy -p gridops-host-agent --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` is clean.

## Not covered (unchanged from round 1, plus the diff gate's NV items)
The following are not exercised by any test:
- the sweep under the lock (A round-2 warn 8);
- Start's success path, which writes `/var/lib/gridops-agent`;
- a real daemon's `docker cp -L`, `--mount …,readonly`, and the host's resolution of the runner root's `externals` link (NV-1, NV-2, NV-5).

The diff gate's W-2 (no sweep of `.partial` on the warm-cache path) is a production choice within the brief's decision 8. It has no test this round. If F changes it, the existing sweep test's shape covers a warm-cache variant only after a T pass.

## Ready for F
RED is valid as a rework RED: both tests pin behaviour the diff gate found untested, and each fails for the right reason when that behaviour is removed (M1-M4). F may proceed with A's round-3 warns 1 and 2. These tests and the rest of the suite must stay green through that refactor.

---

# Round 1 - Handoff-T-red: Phase 4 - Container jobs on the linux pool: runner externals and tool cache at host-valid paths (#126)

**Date:** 2026-11-22
**Branch:** claude/fix-126-externals (head 25e34e8, the Phase 3 PASS commit)
**Brief / wireframe reviewed:** docs/handoffs/126-brief.md (as amended in 92aa2ce and e10e8be), docs/handoffs/126/handoff-A.md (round 2), docs/handoffs/126/decisions.md. Wireframe: N/A (no UI).

## A precondition
Confirmed: A returned PASS on the plan (Phase 3, round 2), with 0 blocks and 9 warns. The tests follow these warns:
- **Warn 1:** the copy, the link and the `cd` come before `config.sh`; the variable is `GRIDOPS_RUNNER_EXTERNALS`.
- **Warn 2:** Prepare's 0555 case gives `Configuration, false` with no move-aside.
- **Warn 3:** the Start cache check comes before any folder is created.
- **Warn 4:** the fill container is named `gridops-externals-<hex>`, carries no runner label, is removed on every exit path, and a crash leftover does not block it.
- **Warn 5:** the core names, the runner root built from `PreparedEnvironmentId`, the hex-only cache name and the `.partial` staging.
- **Warn 6:** Prepare and Start share one predicate.
- **Warn 7:** one `Refusal` type.
- **Warn 9:** the cleanup rename.

Warn 8 (the sweep runs under the fill's lock) has no test. See "Not covered".

## Contract the tests fix (F must provide these names)
Until all of these exist, the core lib tests, the host-agent lib tests and the `docker_gitlab` binary do not compile. In Rust, a RED over new items always works this way (the #140, #146, #148 and #179 precedents).

**Core, `crates/gridops-core/src/fleet/work_folder.rs`, re-exported from `fleet/mod.rs` as `SHARED_WORK_ROOT` is:**
- `pub const SHARED_RUNNERS_ROOT: &str` = `"/var/lib/gridops-agent/runners"`
- `pub const EXTERNALS_CACHE_ROOT: &str` = `"/var/lib/gridops-agent/externals"`
- `pub fn shared_runner_root(environment: PreparedEnvironmentId) -> String`

**Host agent, private items in `crates/gridops-host-agent/src/docker.rs`.** The tests live in the child module `docker::externals_tests` (`src/docker/externals_tests.rs`, declared `#[cfg(test)] mod externals_tests;` in `docker.rs`), so the items can stay private:
- `struct Refusal { code: FailureCode, retryable: bool, reason: String }` (A warn 7).
- `fn externals_cache_name(image_id: &str) -> Option<String>`: drops `sha256:` and accepts only non-empty lowercase hex.
- `fn externals_cache(cache_root: &Path, image_id: &str) -> Result<PathBuf, Refusal>`: Start's check.
- `fn uses_externals_cache(request: &HelperRequest, prepared: &PrepareEnvironment) -> bool` (A warn 6).
- `fn shared_socket_args(socket: &str, work: &Path, runner_root: &Path, externals: &Path) -> Vec<String>`: everything the socket branch adds to `docker create`, meaning the `-v`, `--mount` and `-e` arguments.
- `fn github_runner_script(mode: RunnerMode, shared: bool) -> String`: the whole script for `/bin/bash -lc`.
- `impl DockerRuntime { async fn fill_externals_cache(&self, cache_root: &Path, image: &str) -> Result<PathBuf, Refusal> }`: inspect, fill, rename and sweep, under the new mutex field.
- `fn parse_shared_folders_leftover(inspect: &str) -> Option<(String, Vec<String>)>`: returns the image, then the folders. It replaces `parse_shared_work_leftover` (A warn 9).

## Tests authored

### Core: `crates/gridops-core/src/fleet/work_folder.rs` (`mod tests`, 2 new)
| Test | Pins | Tier |
|---|---|---|
| `a_runner_root_is_a_direct_child_of_the_runners_root` | Item 1 and decision 1. The root is exactly `/var/lib/gridops-agent/runners/<uuid>`, its parent is `SHARED_RUNNERS_ROOT`, and its last component is the typed id. | Unit (pure) |
| `the_runner_root_and_the_externals_cache_are_never_under_a_work_folder` | Item 5 and decision 1. The runner root is under neither `<work>` nor `SHARED_WORK_ROOT`. The cache root is not under `SHARED_WORK_ROOT`. `EXTERNALS_CACHE_ROOT` is the brief's literal path. The three roots are under `/var/lib/gridops-agent`, and none contains another. | Unit (pure) |

### Host agent: `crates/gridops-host-agent/src/docker/externals_tests.rs` (new, 18 tests)
**Pure functions and bash, no stand-in:**

| Test | Pins | Tier |
|---|---|---|
| `the_cache_name_is_the_image_ids_hex_and_always_one_path_component` | Item 1 and A warn 5. `sha256:<hex>` gives `<hex>`. Empty, `.`, `..`, `/`, uppercase, spaces, a newline, `.partial` and a missing prefix give `None`. Any `Some` is one `Normal` component. | Unit |
| `the_start_check_finds_the_cache_or_refuses_retryably_naming_the_id_and_path` | Item 3 and decision 5. The cache is found by its id. A missing cache, even with a `<hex>.partial` beside it, gives `Runtime, true` with the reason naming the id and the full path. An id that yields no cache name gives `Runtime, true` naming the id. | Unit (temp dir) |
| `only_a_github_runner_that_shares_the_socket_uses_the_externals_cache` | A warn 6 truth table. GitHub repo, GitHub org and fix sandbox with the socket give `true`. GitHub without the socket, and GitLab project or group with or without the socket, give `false`. | Unit |
| `a_shared_runner_gets_its_folders_at_host_paths_and_the_cache_read_only` | Items 1 and 2, decisions 4, 6 and 9. See the list below. | Unit |
| `a_shared_jit_runner_runs_from_its_runner_root_with_externals_linked_to_the_cache` | Decision 4 and A warn 1, JIT. The generated script runs under `bash`, from the image directory, with the secret on stdin and the `-e` variables from `shared_socket_args`. See the list below. | Unit (bash, temp dir; the #126 brief's precedent `job_console_pages_are_forwarded_to_runner_stdout`) |
| `a_shared_persistent_runner_is_configured_and_run_from_its_runner_root` | Decision 4 and A warn 1, persistent. `config.sh` runs with `$PWD` set to the runner root and receives `--token <secret>` and `--work <shared work>`. `run.sh` then runs there, configured. The image directory gets no `.runner`. | Unit (bash) |
| `a_runner_that_does_not_share_the_socket_still_runs_in_the_images_directory` | Item 4, both modes. `run.sh` (and `config.sh` with `--work _work`) run in the image directory, and no runner root is written. **Guard, green by design.** | Unit (bash) |
| `the_runner_resolves_its_tool_cache_to_the_work_folder_whatever_the_image_sets` | Item 2 and decision 9. See the note below. | Unit (bash) |

The checks behind `a_shared_runner_gets_its_folders_at_host_paths_and_the_cache_read_only`:
- The socket, `-v work:work`, and the runner root at its own path, writable (as `-v` or `--mount`).
- The cache as `--mount type=bind,source=C,target=C,readonly`, and never in a `-v`.
- `-e GRIDOPS_RUNNER_WORK`, `GRIDOPS_RUNNER_ROOT`, `GRIDOPS_RUNNER_EXTERNALS` and `RUNNER_TOOL_CACHE=<work>/_tool`.
- If `AGENT_TOOLSDIRECTORY` is set, it is `<work>/_tool`.
- No `/actions-runner`, `/opt/hostedtoolcache`, `/__e` or `/__t` anywhere.

The checks behind `a_shared_jit_runner_runs_from_its_runner_root_with_externals_linked_to_the_cache`:
- `run.sh` runs with `$PWD` set to the runner root and receives `--jitconfig <secret>`.
- The root holds a real `bin` (copied, not linked) and `run.sh`.
- `externals` is a symlink that resolves to the cache.
- The image directory is untouched.
- The job log the runner writes under the root's `_diag/pages` is forwarded to stdout.

How `the_runner_resolves_its_tool_cache_to_the_work_folder_whatever_the_image_sets` works:
- The container env carries the pool image's `AGENT_TOOLSDIRECTORY=/opt/hostedtoolcache` (the brief's facts).
- The image ships a `.env` with `RUNNER_TOOL_CACHE=` and `AGENT_TOOLSDIRECTORY=/opt/hostedtoolcache`.
- The test resolves the tool directory the way the runner does: `RUNNER_TOOL_CACHE`, else `AGENT_TOOLSDIRECTORY`, else `<work>/_tool`. Before resolving, it applies the `.env` of the directory `run.sh` ran in, with later lines winning.
- The result must be `<work>/_tool`.
- Any fix passes: stripping the lines, appending overrides, or unsetting both.

**Against a stand-in `docker`** (the `tests/docker_runtime.rs` re-exec pattern, in-crate per decision 10, no daemon):

| Test | Pins | Tier |
|---|---|---|
| `the_cache_is_filled_once_per_image_from_the_images_working_directory` | Items 1 and 3, decisions 3, 6 and 7, A warn 4. See the list below. | Integration (stand-in) |
| `a_symlinked_externals_is_followed_or_refused_never_cached_dangling` | Item 1. `externals` is a relative symlink to `../opt/runner-externals`. The test accepts either (a) `Ok`, with a real cache directory holding the real files and no dangling link, or (b) `Configuration, false`, with a reason naming the image and `/actions-runner/externals` and no cache left. Either way, no container is left. **Phase 6 narrows this to the choice F documents.** | Integration |
| `an_image_without_a_working_directory_or_externals_is_a_configuration_failure` | Decision 5, 3 cases: no `WorkingDir` on Docker 29 (template error), no `WorkingDir` on older Docker (empty), and a working directory without `externals`. Each gives `Configuration, false`, a reason naming the image (tag or hex) plus `/actions-runner/externals` in case 3, no cache and no container left. | Integration |
| `an_unwritable_cache_root_is_a_configuration_failure_and_is_not_moved_aside` | Decision 5 and A warn 2. A 0555 cache root gives `Configuration, false`, with a reason naming the image and the cache root. The root keeps mode 0555 at the same path, with no `.not-ours-` sibling. The test skips when it runs as root, using the 0555 probe from `docker.rs:977-980`. | Integration |
| `a_docker_error_during_the_fill_is_retryable_and_leaves_no_cache` | Decision 5 and item 5. See the list below. | Integration |
| `a_fill_sweeps_caches_whose_image_is_gone_and_leftover_staging` | Decision 8. See the note below. | Integration |
| `a_github_prepare_sharing_the_socket_fails_when_it_cannot_fill_the_cache` | Decision 2 wiring through `DockerRuntime::call`. A GitHub Prepare with the socket, on a machine without `/var/lib/gridops-agent`, answers `Failed` (`Configuration, false` or `Runtime, true`), not `Succeeded`. It skips when it runs as root or when `/var/lib/gridops-agent` exists. | Integration |
| `a_shared_start_without_a_cache_fails_retryably_and_creates_nothing` | Item 3, decisions 2 and 7, A warn 3, for JIT and persistent. A Start with the socket and no cache for the resolved id answers exactly `Failed { Runtime, true, None }`, with no `docker create` and no `docker run`. The same skip as above applies. | Integration |
| `cleanup_removes_the_work_folder_and_the_runner_root_but_never_the_cache` | Item 1 and A warn 9, through `Cleanup`. A runner whose mounts include the socket, the work folder, the runner root and the cache gets a removal `docker run` for the work folder and the root, and none for the cache. | Integration |
| `the_stand_in_answers_like_docker_29` | Guards the stand-in itself. See the list below. **Guard.** | Unit |

The checks behind `the_cache_is_filled_once_per_image_from_the_images_working_directory`:
- The working directory is `/home/runner`, not `/actions-runner`.
- The cache is `<root>/<hex>` and is a real directory.
- It holds the image's node and an internal relative link that resolves.
- No staging is left.
- Exactly one `create`, with `--name gridops-externals-<hex>` and no `io.gridops.fleet.environment` label.
- A crash leftover container with that name does not block the fill.
- No container is left afterwards.
- A second fill does no new `create` and no new `cp`.

The cases in `a_docker_error_during_the_fill_is_retryable_and_leaves_no_cache`:
- `create` fails.
- `cp` fails.
- `rm` fails.
- The image does not answer.
- The id is `sha256:2F52-NOT-HEX`.

Each case gives `Runtime, true` and no `<root>/<hex>`. The reason names the hex and the cache root, or the tag, or the bad id. The fill container is removed even after a failed `cp`.

The two cases in `a_fill_sweeps_caches_whose_image_is_gone_and_leftover_staging`:
1. **Missing image.** The cache holds a live cache (image present), a stale cache (Docker answers "no such object") and `<hex>.partial`. After a new fill, only `{new, live}` remain.
2. **Docker does not answer** for the other images (a "not missing" error). The stale cache is kept.

The checks behind `the_stand_in_answers_like_docker_29` (each answer matches real Docker 29.8.2 output, below):
- `{{.Config.WorkingDir}}` on an image without one is a "map has no entry for key" error.
- `image inspect` of a missing image says "No such image".
- `inspect` says "no such object".
- `rm -f` of a missing container succeeds.
- `cp` says "Could not find the file … in container …".

### Host agent: existing tests changed
- **`docker.rs` `shared_work_folder_tests`, renamed to `parse_shared_folders_leftover` and its `(image, folders)` shape.**
  - `finds_the_image_and_the_shared_folder` is the same case in the new shape.
  - New **RED**: `finds_the_work_folder_and_the_runner_root_but_never_the_externals_cache`. The parse test the brief asks for, with the cache among the mount sources.
  - `a_runner_without_a_shared_folder_has_nothing_to_remove` adds the case of a container whose only GridOps mount is the cache.
  - `only_a_direct_child_of_the_work_root_is_ever_removed` becomes `…_of_the_work_or_runners_root_…`. It adds the `runners` variants (the root itself, `/`, `a/b`, `..`, `runnersx/a`), the cache, and `/var/lib/gridops-agent`. It is a **guard**: nothing is accepted today.
- **`tests/docker_runtime.rs`** `a_runner_container_is_created_with_the_flags_its_image_needs` (item 4). A runner without the socket gets:
  - no `/var/lib/gridops-agent`;
  - no `--mount`;
  - no `-e RUNNER_TOOL_CACHE=`, `AGENT_TOOLSDIRECTORY=`, `GRIDOPS_RUNNER_ROOT=`, `_EXTERNALS=` or `_WORK=`.

  Only `-e` values count, because the shared script itself mentions `GRIDOPS_RUNNER_ROOT`. **Guard.**
- **`tests/docker_gitlab.rs`** `a_verified_gitlab_runner_starts_with_its_token_only_on_stdin` (decision 10). A GitLab runner gets:
  - no arg containing `SHARED_RUNNERS_ROOT` or `EXTERNALS_CACHE_ROOT`;
  - no `-e RUNNER_TOOL_CACHE=`, `GRIDOPS_RUNNER_ROOT=` or `GRIDOPS_RUNNER_EXTERNALS=`;
  - no `--mount`.

  **Guard** (compile RED only, through the core constants).

### Not covered by tests, and why
- **The success path of `start` with the socket.** It is not reachable as a non-root user: it writes `/var/lib/gridops-agent/{work,runners}`. Three things are therefore pinned only through the pure functions that `start` must call:
  - that `start` uses `shared_socket_args` and `github_runner_script(mode, true)`;
  - that it creates the runner container **from the resolved id, not the tag** (decision 7);
  - A warn 3's removal of both folders when `docker create` or `feed_secret` fails.

  The diff gate and S must check this wiring by reading. If F routes the socket branch through a helper that takes the roots as `&Path`, I will add a stand-in test of it in Phase 6.
- **A warn 8 (the sweep runs under the fill's lock).** A test would rely on timing, and the supervisor already runs one command at a time.
- **A failed `rename` in the fill.** There is no reliable way to make it fail without depending on F's order of checks. The other four Docker and IO failures are covered.
- **The per-start copy cost** (bytes and files), the docs and the CHANGELOG. These belong to F's handoff and S. The JIT test checks that `externals` in the root is a link. It does not check that nothing was copied and then deleted.
- **The order of the secret read before the copy** (A warn 1's 500 ms `feed_secret` window). A test would rely on timing. The persistent test does pin the other half of warn 1: the copy and the `cd` come before `config.sh`.

## RED confirmation
Run with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` on 25e34e8 plus the test files only. `cargo fmt --all -- --check` is clean.

**1. Compile RED (the real state of the branch): only the contract items are missing.**
```
cargo test -p gridops-core --lib --no-run
  error[E0425]: cannot find value `SHARED_RUNNERS_ROOT` in this scope        (x3)
  error[E0425]: cannot find value `EXTERNALS_CACHE_ROOT` in this scope       (x4)
  error[E0425]: cannot find function `shared_runner_root` in this scope      (x2)
cargo test -p gridops-host-agent --lib --no-run
  error[E0432]: unresolved imports `super::Refusal`, `super::externals_cache`, `super::externals_cache_name`,
                `super::github_runner_script`, `super::shared_socket_args`, `super::uses_externals_cache`
  error[E0432]: unresolved import `super::parse_shared_folders_leftover`
  error[E0599]: no method named `fill_externals_cache` found for struct `DockerRuntime`   (x7)
  error[E0425]: cannot find function `shared_runner_root` in module `gridops_core::fleet`
  error[E0425]: cannot find value `EXTERNALS_CACHE_ROOT` in module `gridops_core::fleet`
cargo test -p gridops-host-agent --test docker_gitlab --no-run
  error[E0425]: cannot find value `SHARED_RUNNERS_ROOT` in module `gridops_core::fleet`
  error[E0425]: cannot find value `EXTERNALS_CACHE_ROOT` in module `gridops_core::fleet`
cargo test -p gridops-host-agent --test docker_runtime      -> 2 passed (needs no new symbol; guard green)
```

**2. Runtime RED behind the compile RED.**

This run used throwaway stubs that reproduce today's behaviour under the new names:
- the core constants and `shared_runner_root` with their brief values;
- `externals_cache_name` returning `None`;
- `externals_cache` and `fill_externals_cache` returning `Ok(root.join(input))` without checking or copying;
- `uses_externals_cache` returning `false`;
- `shared_socket_args` giving today's socket and work `-v` plus `GRIDOPS_RUNNER_WORK`;
- `github_runner_script` giving today's two scripts verbatim;
- `parse_shared_folders_leftover` wrapping today's parse.

`start`, `prepare` and Cleanup were unchanged. The stubs were reverted and never staged. Result for `cargo test -p gridops-host-agent --lib -- docker::`: 7 passed, 17 failed.
```
the_cache_name_is_the_image_ids_hex_and_always_one_path_component     left: None  right: Some("2f529cfa…")
the_start_check_finds_the_cache_or_refuses_…                          Ok(<root>/sha256:2f529cfa…)   (no cache by name)
only_a_github_runner_that_shares_the_socket_uses_the_externals_cache  github, socket: expected true
a_shared_runner_gets_its_folders_at_host_paths_and_the_cache_read_only  the runner root is not mounted writable at its own path: [-v sock, -v work:work, -e GRIDOPS_RUNNER_WORK=…]
a_shared_jit_runner_runs_from_its_runner_root_…                        run.sh did not run in the runner root:  run pwd=…/image
a_shared_persistent_runner_is_configured_and_run_from_its_runner_root config.sh did not run in the runner root:  config pwd=…/image
the_runner_resolves_its_tool_cache_to_the_work_folder_whatever_the_image_sets  the runner in …/image would use /opt/hostedtoolcache as its tool cache, not …/work/env/_tool
the_cache_is_filled_once_per_image_from_the_images_working_directory  <root>/harbor.example/…/pl-runner:1.70.4   (path != <root>/<hex>)
a_symlinked_externals_is_followed_or_refused_never_cached_dangling   <root>/harbor.example/…   (no cache, no refusal)
an_image_without_a_working_directory_or_externals_is_a_configuration_failure  no working directory, Docker 29: Ok(…)
an_unwritable_cache_root_is_a_configuration_failure_and_is_not_moved_aside   Ok(…)
a_docker_error_during_the_fill_is_retryable_and_leaves_no_cache      create fails: Ok(…)
a_fill_sweeps_caches_whose_image_is_gone_and_leftover_staging        image gone: left [aaaa…, bbbb…, cccc….partial], expected [2f52…, aaaa…]
a_github_prepare_sharing_the_socket_fails_when_it_cannot_fill_the_cache  answered Succeeded { PreparedEnvironment … }
a_shared_start_without_a_cache_fails_retryably_and_creates_nothing   Ephemeral: answered Failed { code: Configuration, retryable: false }   (today's work-folder permission error, not the cache check)
cleanup_removes_the_work_folder_and_the_runner_root_but_never_the_cache  Cleanup did not empty /var/lib/gridops-agent/runners/<id>: [one `run` for the work folder only]
finds_the_work_folder_and_the_runner_root_but_never_the_externals_cache  left: (img, [work])  right: (img, [runners/0a1b, work/0a1b])
-- green by design: a_runner_that_does_not_share_the_socket_… (guard), the_stand_in_answers_like_docker_29 (guard),
   and the 5 shared_work_folder_tests guards/renames.
```
`docker_gitlab` (10) and `docker_runtime` (2) pass with the stubs, as guards. The core tests' RED is the compile RED only: they pin new constants and a new path function, and today has neither.

**My own correction.** My first draft of the tool-cache test passed against today's script, which made it an invalid RED. It read `.env` from the runner root, not from the directory the runner actually ran in, and it did not model the pool image's `AGENT_TOOLSDIRECTORY` ENV. It now reads the run directory's `.env` and sets the image ENV, and it fails for the right reason (shown above).

**3. The tests can pass, and they catch the defects they target.** I wrote a throwaway reference implementation of the whole contract, wired into `prepare`, the socket branch of `start` and Cleanup. With it, the whole host-agent crate passed: lib 27, dispatch 8, docker_gitlab 10, docker_runtime 2. Three repeated runs were stable, and the core `work_folder` tests passed 4/4. Then, one change at a time:

| Mutation | Caught by |
|---|---|
| Sweep asks `docker image inspect` (Docker says "No such image", which `missing()` does not recognise) | the sweep test: the stale cache is kept |
| Fill from a hardcoded `/actions-runner` | the fill-once test: Configuration, no `/actions-runner/externals` in an image whose WorkingDir is `/home/runner` |
| Copy and `cd` placed after `config.sh` (A warn 1's trap) | the persistent test: `config.sh` did not run in the runner root |
| Cache mounted with `-v …:ro` | the args test |
| Fill container not removed after a failed `cp` | the Docker-error test and the configuration test: containers left behind |
| Start creates the work folder before checking the cache (A warn 3) | the Start test: `Configuration, false` instead of `Runtime, true` |
| A non-socket runner gets `-e RUNNER_TOOL_CACHE` | the `docker_runtime` guard |
| A GitLab runner gets `-e GRIDOPS_RUNNER_ROOT=/var/lib/gridops-agent/runners/x` | the `docker_gitlab` guard |
| Core: runner root at `<work>/runner` (the round-1 design) | both core tests |

`cargo +1.98.1 clippy -p gridops-core -p gridops-host-agent --all-targets --locked -- -D warnings -A dead_code -A clippy::unused_async_trait_impl` (CI's toolchain and flags) exits 0 with the reference in place. `-A dead_code` was needed only for my unwired reference, not for the tests.

**4. A flake fixed before handoff.** `instrumented_runner_script` keeps its forwarder offsets in a fixed `/tmp/gridops-forwarded-job-state` (`provisioning.rs`). In production there is one per container, but in one test process the parallel bash tests read each other's offsets, and the JIT test once lost its forwarded log. The four bash tests now take a static lock around each script run.

**Real-Docker facts behind the stand-in.** Measured read-only on this Mac's Docker 29.8.2, using local images and one throwaway container (`gridops-t126-probe`, removed):
- `docker image inspect -f '{{.Id}} {{.Config.WorkingDir}}'` on an image without a WorkingDir exits 1 with `template parsing error: … map has no entry for key "WorkingDir"`. `{{json .Config}}` works, and so does `{{index .Config "WorkingDir"}}`, which prints `<no value>`.
- For a missing image, `docker image inspect` and `docker inspect --type image` give `Error response from daemon: No such image: X`, while plain `docker inspect X` gives `error: no such object: X`.
- `docker rm -f` of a missing container exits 0.
- `docker cp c:/missing dst` gives `Error response from daemon: Could not find the file /missing in container c`.
- `docker cp` of a symlink without `-L` copies the link.
- `docker cp c:/dir existing-dir` nests the copy as `existing-dir/dir`.

## Notes for F
- **Decision 3's literal template breaks on Docker 29.** `{{.Id}} {{.Config.WorkingDir}}` fails outright for an image with no WorkingDir (see above). Both the template error and the old empty answer must give `Configuration, false`. `{{json .Config}}` handles both.
- **The sweep's "missing" check.** Use plain `docker inspect sha256:<hex>`, whose "no such object" `missing()` already recognises, or extend `missing()` to "no such image". The stand-in gives each form its real message.
- **"No `externals`" arrives as `docker cp`'s "Could not find the file … in container …"**, an error response from the daemon. Map that text to `Configuration, false`, and any other `cp` error to `Runtime, true`.
- **Create the staging folder before `docker cp`.** Otherwise the 0555 case surfaces as a `cp` (Docker) error, which is `Runtime`, not `Configuration, false`.
- **What the reasons must name:**
  - the image, as the tag or the hex id;
  - for Docker or IO errors after the id is known, also the cache root's path (any path under it, such as `<root>/<hex>` or the staging folder, contains it);
  - for the missing-`externals` case, `<workdir>/externals`.
- **The scripts must run under `bash` on macOS too** (BSD userland), because the tests run them on the dev Mac. That means `cp -R` or `cp -a`, `ln -s`, `sed -i.bak` and `tar`; no `cp --no-target-directory`, `find -printf` or suffix-less `sed -i`. The reference used a loop over `"$PWD"/* "$PWD"/.[!.]*` that skips `externals` and runs `cp -R`, then `ln -sfn`, then `cd`.
- **The stand-in answers these `inspect` templates:**
  - `{{.Id}}`;
  - `{{.Config.WorkingDir}}`, `{{json .Config.WorkingDir}}`, `{{index .Config "WorkingDir"}}` and `{{json .Config}}`;
  - `{{"\n"}}`;
  - the existing Cleanup template, the label template and `{{.State.Running}}`.

  It also answers `create`, `cp` (flags `-L`, `-a`, `-q`), `rm` and `run`. Anything else is reported under "docker calls the stand-in could not answer". Tell me, and I will extend the stand-in (test code is mine).
- **The symlinked-`externals` test accepts follow or refuse.** Say which you chose in the doc comment, and I will narrow the test to that choice in Phase 6.

## Ready for F
RED is valid. Each RED test fails on the assertion the feature will satisfy. The only other failures are the compile errors for the contract items listed above. The guards are labelled and are green by design:
- the unshared script;
- the stand-in's fidelity;
- the `docker_runtime` and `docker_gitlab` extensions;
- the extended direct-child parse test;
- the renamed parse test.

F may implement against these tests.
