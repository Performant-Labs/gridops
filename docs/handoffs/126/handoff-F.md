# Handoff-F: Phase 6 (round 2, rework after diff gate r1) - Container jobs on the linux pool: runner externals and tool cache at host-valid paths

**Date:** 2026-11-22
**Branch:** claude/fix-126-externals (on 5535839, T's round-2 RED)
**Issue:** #126

Round 1 of this handoff (the full implementation) is kept below, after the divider.

## What was done
- `crates/gridops-host-agent/src/docker.rs`:
  - **A round-3 warn 1 (blocking tree removal on the async runtime).**
    - `remove_tree` is now an `async fn` that runs the old body, renamed `remove_tree_blocking`, under `tokio::task::spawn_blocking`.
    - All three callers await it: the staging removal in `fill_externals_cache`, the staging reset in `fill_staging`, and `sweep_externals_caches`.
    - The cache rename is now `tokio::fs::rename`.
    - `fill_staging` creates the cache root and the staging folder with `tokio::fs::create_dir_all` / `tokio::fs::create_dir` in one `async` block. The `PermissionDenied` → `Configuration, false` and other → `Runtime, true` mapping reads the same `io::Error` as before.
  - **A round-3 warn 2 (the half-finished rename).** `create_shared_work_folder` is now `create_shared_folder`, with its parameter `work` renamed `folder`. The inline test module `shared_work_folder_tests` is now `shared_folder_tests`. Its `use` line follows the rename; no test body or assertion changed.
  - **Diff gate W-1 (glob misses `..name`).** The runner-root copy loop gets a third arm, `"$gridops_image_dir"/..?*`, behind the same `-e`/`-L` existence guard. The doc comment says every entry is copied, dot-entries included.
  - **Diff gate NIT-1.** The `link_out_of` doc comment says links to folders are checked, not traversed.
- `docs/fleet-agent.md` (**diff gate NV-6**): the per-start and per-cache sizes are now marked as estimates from the official runner 2.338.2 release tarball, not the pool image, as brief item 6 requires ("otherwise estimated, saying which"). The text also says an image that adds files copies more.

## Design decisions
- **One async wrapper, not `spawn_blocking` at each call site.** A offered both. One wrapper keeps the three call sites one-liners and means no future caller can run the walk on a worker by mistake. The sync body stays a plain function because its chmod-and-retry walk is all blocking calls.
- **`spawn_blocking(...).await?` turns a `JoinError` into an `io::Error`** through tokio's `From` impl. That is the pattern `NativeRuntime::install` already uses (`native.rs:551`, see the evidence entry). A panicked removal therefore reaches the callers as an IO error, and they already handle that (staging: ignored; prepare: `Runtime, true` unless permission; sweep: a warn).
- **Diff gate W-2 (sweep only after a new fill): not changed, on purpose.** Brief decision 8 says the sweep runs "after a new fill", and A's round-2 warn 8 and round-3 review accept that. A `.partial` can outlive a crash only for an image that is never prepared again: the next fill of the same image removes its own staging first (`fill_staging`), and the next fill of any image sweeps all of them. Changing the warm-cache path would deviate from the brief and need a T pass for a new test, for a leak bounded by one crashed fill per image.
- **Diff gate NIT-2: not changed.** A's round 3 showed that every `warn!` in `docker.rs` on master is a formatted message with no named fields, so the finding's premise does not hold.
- **Diff gate NV-1 to NV-5: unchanged.** They need a real daemon or the third-party runner and Docker sources, which this phase does not have. They stay under Known issues, as in round 1.

## Reuse / extend-vs-new
- No new objects. The async `remove_tree` follows `NativeRuntime`'s staged fill (`native.rs:527-551`: `tokio::fs::rename`, and a tree walk under `spawn_blocking`), the analogue decision 6 names.
- `create_shared_folder` is the same function as before, renamed. No second path was added.

## Architecture notes for A
- **None that change structure.** All edits are inside `docker.rs` private items and the docs.
- `remove_tree` changed from sync to async. It is private, with three callers, all already async. No public interface, module boundary or dependency direction changed, and no crate was added.

## Deviations from spec / wireframe
None. W-2 is left as the brief's decision 8 states it (see Design decisions). No wireframe (no UI).

## Tier 1 self-check (incl. tests now GREEN)
All runs used `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, as a non-root user on a machine with no `/var/lib/gridops-agent`, so the Prepare wiring test ran in full rather than skipping.

    cargo fmt --all -- --check                         -> clean
    cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl
                                                       -> Finished, no warnings
    cargo test -p gridops-host-agent
      lib            28 passed (externals_tests 19, shared_folder_tests 6, 3 others)
      main            0
      dispatch        8 passed
      docker_gitlab  10 passed
      docker_runtime  1 + 1 + 2 passed
      version         1 passed
    cargo test -p gridops-core --lib                   -> 313 passed; 0 failed

T's two round-2 tests, run by name, are green:

    docker::externals_tests::a_failed_rename_into_the_cache_is_retryable_and_leaves_no_cache_or_staging ... ok
    docker::externals_tests::a_github_prepare_sharing_the_socket_fails_when_it_cannot_fill_the_cache ... ok

The failed-rename test runs exactly the lines warn 1 changed: the `tokio::fs::rename` and the awaited `remove_tree` of the staging folder.

**The W-1 glob, checked directly** under `/bin/bash` 3.2.57 (the oldest bash the script meets). The loop ran over an image directory holding `a`, `.b`, `..c`, `.env`, `bin` and `externals`. The runner root received `..c .b .env a bin`, with `externals` skipped. The old two arms miss `..c`.

No server applies: the host agent's Docker paths need a daemon and a fleet host (brief constraint).

## Evidence appendix
`docs/handoffs/126/evidence.md` (one entry added this round: the `spawn_blocking(...).await?` / `tokio::fs::rename` pattern in `native.rs`).

## Tests that look wrong (for T)
None.
- The only test-code text touched is A warn 2's mechanical rename. The inline module's name and its `use` line changed so that it still compiles against `create_shared_folder`; A assigned this rename to F.
- No test body, assertion or fixture changed, and `externals_tests.rs` and `tests/*.rs` are untouched.

## Known issues
Unchanged from round 1:
- **Not exercised:**
  - a real Docker daemon (`docker cp -L`, `--mount … readonly`, and the host resolving the runner root's `externals` link; diff gate NV-1, NV-2 and NV-5);
  - the runner and Docker source claims (NV-3 and NV-4);
  - Start's success path, which writes `/var/lib/gridops-agent`;
  - any fleet host.
- **The sizes are estimates** from the release tarball, because the pool image could not be pulled ("unauthorized"). The docs now say so.

## Files changed
- `crates/gridops-host-agent/src/docker.rs`
- `docs/fleet-agent.md`

---

# Round 1 - Handoff-F: Phase 6 - Container jobs on the linux pool: runner externals and tool cache at host-valid paths

**Date:** 2026-11-22
**Branch:** claude/fix-126-externals (on 55ce0ed, the T-red commit)
**Issue:** #126

## What was done
- `crates/gridops-core/src/fleet/work_folder.rs`:
  - adds `SHARED_RUNNERS_ROOT` (`/var/lib/gridops-agent/runners`), `EXTERNALS_CACHE_ROOT` (`/var/lib/gridops-agent/externals`) and `shared_runner_root(PreparedEnvironmentId)`, which takes the typed id rather than `impl Display` (A warn 5);
  - the module doc now covers externals and the tool cache.
- `crates/gridops-core/src/fleet/mod.rs`: re-exports the three new items beside `SHARED_WORK_ROOT`.
- `crates/gridops-host-agent/src/docker.rs`:
  - **Prepare** fills the externals cache after the pull when `uses_externals_cache` holds (decision 2).
  - **Start**'s GitHub path builds its script with `github_runner_script(mode, shared)`. The socket branch goes through `shared_runner(socket, image, &SharedPaths)`:
    1. resolve the id;
    2. check the cache;
    3. create the work folder, then the runner root;
    4. add `shared_socket_args`.

    The container is then created from the resolved image id. A failed `docker create` or `feed_secret` removes both folders.
  - **Cleanup** removes every folder that `parse_shared_folders_leftover` finds (the work folder and the runner root, never the cache).
  - **New private items:** `Refusal`, `uses_externals_cache`, `externals_cache_name`, `externals_cache`, `shared_socket_args`, `SHARED_RUNNER_ROOT_SETUP`, `github_runner_script`, `SharedPaths`, `SharedRunner`, `shared_runner`, `DockerRuntime::fill_externals_cache` (with the field `externals_lock`), `fill_staging`, `link_out_of`, `sweep_externals_caches` and `remove_tree`.
  - **Renames (A warn 9):** `shared_work_leftover` became `shared_folders_leftover`, `parse_shared_work_leftover` became `parse_shared_folders_leftover` (returning `(image, folders)`), and `remove_shared_work_folder` became `remove_shared_folder`, with a `remove_shared_folders` loop. The log lines are now neutral.
- `docs/fleet-agent.md`: "Docker actions and container jobs" now covers:
  - the externals cache and the runner root;
  - the tool cache;
  - the disk used, with measured sizes;
  - the failure modes;
  - the Upgrading note.
- `CHANGELOG.md`: an `## Unreleased` → `### Upgrading` item saying hosts need the new agent build, and a `### Fixed` entry.
- `docs/handoffs/126/evidence.md` (new) and `docs/handoffs/126/decisions.md` (appended).

## Design decisions
- **A symlinked `externals` is followed, not refused.** The fill runs `docker cp -L <fill>:<workdir>/externals <staging>/externals`. Links inside it are kept as links. A link that is absolute, or that does not resolve inside the copied tree, refuses the image with `Configuration, false`, so the cache never holds a dangling link. The alternative was to refuse every symlinked `externals`. Following it costs nothing and serves images that relocate `externals`. The doc comment on `fill_externals_cache` says this. **T: narrow `a_symlinked_externals_is_followed_or_refused_never_cached_dangling` to the `Ok` branch.**
- **One inspect gives the image id and the working directory:** `{{.Id}} {{json .Config}}`. Decision 3's literal `{{.Config.WorkingDir}}` is a template error on Docker 29 when the image has no WorkingDir (T's note). The JSON form answers on both Docker versions. A missing, empty or relative WorkingDir gives `Configuration, false`.
- **Fill order:**
  1. inspect;
  2. cache name (none gives `Runtime, true`);
  3. existing cache (return it without copying);
  4. working directory (`Configuration`);
  5. create the cache root and `<hex>.partial` (`PermissionDenied` gives `Configuration, false`, with no move-aside, per A warn 2; any other IO error gives `Runtime, true`);
  6. `rm -f gridops-externals-<hex>`, which removes a crash leftover;
  7. `create --name gridops-externals-<hex> --entrypoint "" <id> true`, with no label;
  8. `cp -L`;
  9. `rm -f` (always run after a successful `create`);
  10. check the copy is a folder and check its links;
  11. rename `<hex>.partial/externals` to `<hex>`;
  12. remove the staging folder;
  13. sweep.

  The staging folder is created before any Docker call, so the 0555 case is told apart from a Docker error (T's note).
- **`cp` error mapping:** "Could not find the file" gives `Configuration, false`, naming `<workdir>/externals`. Any other `cp`, `create` or `rm` error gives `Runtime, true`, naming the staging path (which contains the cache root and the hex).
- **Sweep (decision 8, A warn 8):** it runs only after a *new* fill, while the fill still holds `externals_lock`.
  - Every `*.partial` folder is removed.
  - A hex-named cache is removed only when plain `docker inspect sha256:<hex>` fails with an error `missing()` recognises ("no such object"). I kept `missing()` unchanged rather than adding "No such image" to it, because it is shared with Inspect and Cleanup.
  - Any other answer keeps the cache, as do names that are not hex.
- **`remove_tree`:** when `remove_dir_all` fails, it makes the folders writable and tries again. Without this, an image that ships a read-only folder would leave a cache or staging folder that the agent can never remove.
- **The runner script.** The setup runs right after `IFS= read -r S` in the main shell (A warn 1); in persistent mode that is before `args=(…)` and `./config.sh`. It:
  1. `cp -a`s every entry of `$PWD` except `externals` into `$GRIDOPS_RUNNER_ROOT`;
  2. `ln -sfn "$GRIDOPS_RUNNER_EXTERNALS" "$GRIDOPS_RUNNER_ROOT/externals"`;
  3. removes the `RUNNER_TOOL_CACHE=` and `AGENT_TOOLSDIRECTORY=` lines from the root's `.env` and appends both set to `$RUNNER_TOOL_CACHE`;
  4. `cd`s into the root.

  It uses only `cp`, `ln`, `grep`, `mv` and `printf`, which behave the same on GNU and BSD. With `shared == false` the script is byte-identical to before (item 4). `instrumented_runner_script` in core is unchanged.
- **Tool cache:** both `RUNNER_TOOL_CACHE` and `AGENT_TOOLSDIRECTORY` are set with `-e` to `<work>/_tool`. Setting the second as well overrides the pool image's ENV (`AGENT_TOOLSDIRECTORY=/opt/hostedtoolcache`), so the runner's `env.sh` cannot copy that value into `.env`. In the 2.338.2 release, `env.sh` does not list either variable. The `.env` rewrite covers an image that ships its own `.env`.
- **Mounts:**
  - the runner root uses `-v root:root`, like the work folder; the agent creates both first;
  - the cache uses `--mount type=bind,source=C,target=C,readonly` (decision 6).
- **`remove_shared_folder` tries `std::fs::remove_dir` first.** An empty folder (a failed `docker create`) is then removed without starting a container inside Start's 25 s budget. A non-empty folder goes through the existing throwaway-container path as before. This is A warn 3, done through the existing removal path, not a second one.
- **`parse_shared_folders_leftover` also rejects `.`** as the child name. Before, `/var/lib/gridops-agent/work/.` would have passed, and `remove_dir` would then have targeted the work root.
- **Per-start copy cost and cache size, measured from the runner 2.338.2 release tarball**, not the pool image (see Known issues):
  - **per start:** 272 files, 84,797,525 bytes (`bin` 83,244 KiB on disk plus `config.sh`, `env.sh`, `run.sh`, `safe_sleep.sh` and two `run-helper` templates), plus one link;
  - **one-time cache fill per image:** 9,122 files, 599,417,734 bytes (`externals` 606,736 KiB on disk), with 6 relative links (node20 and node24 `npm`, `npx`, `corepack`). All 6 resolve inside the tree, so the real release passes the link check.

## Reuse / extend-vs-new
- Extended the objects the brief names:
  - `DockerRuntime::prepare`;
  - the `request.docker_socket` branch of `DockerRuntime::start`, now `uses_externals_cache`;
  - `create_shared_work_folder`, which already took its root and is now also called for the runner root;
  - the cleanup trio, renamed and extended to both roots;
  - `gridops_core::fleet::work_folder`.
- The fill follows `NativeRuntime::release`/`download` (`native.rs`): a `tokio::sync::Mutex` field, a `.partial` staging folder and a rename.
- The new objects (`Refusal`, the pure builders and `SharedPaths`) are the ones that A warns 6 and 7 and decision 10 call for. No parallel path was added.

## Architecture notes for A
- **Layers:** the host agent's `docker.rs` and core's `fleet::work_folder`. The server, the reconciler, the manager and `provisioning.rs` are unchanged.
- **Public surface:** core gains 2 `pub const` and 1 `pub fn`, re-exported from `fleet`. There are no wire, schema or protocol changes, and no new `FailureCode` or `DiagnosticCode` variants (decision 5). There are no new crate dependencies.
- **Host footprint:** `/var/lib/gridops-agent/externals/<hex>` (shared, removed by the sweep) and `/var/lib/gridops-agent/runners/<env>` (per runner, removed by Cleanup).
- **Seam for T:** Start's socket branch is `shared_runner(socket, image, &SharedPaths)`, and `SharedPaths`'s fields are plain `PathBuf`s. T's "Not covered" item (the success path of the socket branch) can now be tested with the stand-in and temp roots.

## Deviations from spec / wireframe
- **Decision 3's inspect template** is `{{.Id}} {{json .Config}}`, not `{{.Id}} {{.Config.WorkingDir}}`, for Docker 29 (above). The behaviour is the one decision 3 asks for.
- **Extra refusal:** a link in `externals` that leads outside it gives `Configuration, false`. This enforces item 1's "never a dangling link".
- No wireframe (no UI).

## Tier 1 self-check (incl. tests now GREEN)
All runs used `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`. The machine has no `/var/lib/gridops-agent` and the tests ran as a non-root user, so the two "agent host" stand-in tests and the 0555 test ran in full rather than skipping.
```
cargo test -p gridops-core --lib work_folder      -> 4 passed (incl. the 2 new #126 tests)
cargo test -p gridops-core --lib                  -> 313 passed; 0 failed
cargo test -p gridops-host-agent  (run twice, same result both times)
  lib            27 passed  (18 docker::externals_tests + 6 shared_work_folder_tests + 3 others)
  dispatch        8 passed
  docker_gitlab  10 passed
  docker_runtime  2 passed
  other bins      1 + 1 passed
cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl  -> Finished, no warnings
cargo fmt --all -- --check                        -> clean
```
The T tests named in handoff-T-red that were RED are now GREEN, among them:
- `the_cache_is_filled_once_per_image_from_the_images_working_directory`
- `a_shared_start_without_a_cache_fails_retryably_and_creates_nothing`
- `cleanup_removes_the_work_folder_and_the_runner_root_but_never_the_cache`
- `the_runner_resolves_its_tool_cache_to_the_work_folder_whatever_the_image_sets`
- `finds_the_work_folder_and_the_runner_root_but_never_the_externals_cache`

No server applies: the host agent's Docker paths need a daemon and a fleet host, and none was touched (brief constraint).

## Evidence appendix
`docs/handoffs/126/evidence.md`

## Tests that look wrong (for T)
None. One narrowing is due: `a_symlinked_externals_is_followed_or_refused_never_cached_dangling` can now require the follow branch (`Ok`, real files, no dangling link), as the `fill_externals_cache` doc comment documents.

## Known issues
- **Copy sizes are a proxy measurement.**
  - The pool image `harbor.performantlabs.com/performantlabs/pl-runner:1.70.4` could not be pulled ("unauthorized"; no Harbor credentials were provided to this phase).
  - `ghcr.io/actions/actions-runner:2.338.2` failed mid-pull.
  - So the per-start and cache figures come from the official `actions-runner-linux-x64-2.338.2.tar.gz`, the same runner release the image installs. If the pool image adds files to `/actions-runner` beyond the release, the per-start copy grows by that amount.
- **Not exercised:**
  - a real Docker daemon: `docker cp -L` and `--mount … readonly` on a real engine, and a host bind source resolved through the runner root's `externals` link (the brief's premise, A's round-1 hedge);
  - Start's success path, which needs a writable `/var/lib/gridops-agent`;
  - any fleet host.
- **Exercised:** T's stand-in `docker` (no daemon), and the generated scripts under bash 3.2 on macOS.

## Files changed
- `crates/gridops-core/src/fleet/work_folder.rs`
- `crates/gridops-core/src/fleet/mod.rs`
- `crates/gridops-host-agent/src/docker.rs`
- `docs/fleet-agent.md`
- `CHANGELOG.md`
