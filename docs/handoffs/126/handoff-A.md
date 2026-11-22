# Handoff-A: Phase 3 - Container jobs on the linux pool: runner externals and tool cache at host-valid paths (#126)  (up-front plan review, round 3: re-entry after F reported an architecture change)

**Date:** 2026-11-22
**Branch:** claude/fix-126-externals (head 3f00753; base 9e357e0)
**Brief reviewed:** docs/handoffs/126-brief.md (sha256 42204bd2…c949, the same file the round-2 PASS reviewed; unchanged since e10e8be)   **Reuse map:** there is no `survey.md`. The brief names the objects to extend (`DockerRuntime::prepare`, the socket branch of `DockerRuntime::start`, the shared-folder create/cleanup functions in `docker.rs`, `gridops_core::fleet::work_folder`); the analogous feature is `NativeRuntime`'s cached release (`native.rs`).   **Wireframe:** N/A (no UI)
**Verdict:** PASS

This replaces the round-2 handoff (PASS, committed in 25e34e8). The round-2 warns are re-checked below against the code F wrote.

## Why A ran again
- The task text says "before any code exists". That is the driver's fixed Phase-4 text. It does not describe this branch.
- Code exists: F's implementation is at a27bd95 and T-green passed at 3f00753.
- The outside diff gate (`docs/handoffs/126-diff-result-r1.md`, untracked) then BLOCKed with one finding, B-1: a missing failed-rename test.
- F's result reported `archChanged`. The driver therefore routed the BLOCK back to Phase 4 (A), "drift re-check if architecture changed" (`workflow-coding-pipeline.md` rework table). It did not classify the BLOCK, so no note sends B-1 to T.
- This round does two things:
  - it re-confirms the unchanged plan;
  - it checks that the architecture F built is still the one the plan and the round-2 warns describe.

## Summary
PASS: no block findings and 2 warn findings.
- **The plan is unchanged** (same brief hash as round 2), so the round-2 PASS on the plan stands.
- **F's architecture change stays inside the planned seams.**
  - Core gains `SHARED_RUNNERS_ROOT`, `EXTERNALS_CACHE_ROOT` and `shared_runner_root(PreparedEnvironmentId)` beside `SHARED_WORK_ROOT` / `shared_work_folder`, re-exported the same way.
  - Everything else is in `docker.rs`.
  - The server, the reconciler, the manager and core's `instrumented_runner_script` are untouched.
  - There are no wire, protocol or `FailureCode` changes.
- **Round-2 warns 1-9 are all followed**, each in the form round 2 suggested.
- **The 2 new warns are about consistency with neighbouring code.** Neither changes behaviour:
  - blocking tree removal inside async functions, where the analogous `NativeRuntime` uses `tokio::fs`;
  - a half-finished rename of the shared-folder functions.

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | Decision 6, "Fill like NativeRuntime"; decision 8, the sweep | pattern consistency; cross-cutting (async runtime) | **Blocking removal of large trees on the async runtime.**<br>- `remove_tree` (`docker.rs:1341`) is a synchronous `std::fs::remove_dir_all`, with a chmod-and-retry walk.<br>- It is called directly from async code: `fill_externals_cache` on the staging folder (`:1177`), `fill_staging` (`:1208`) and `sweep_externals_caches` (`:1325`). The sweep deletes whole stale caches, about 600 MB and 9,122 files each by F's measurement.<br>- `fill_externals_cache` also uses `std::fs::rename` (`:1169`).<br>- `NativeRuntime`'s fill, which decision 6 names as the model, does these with `tokio::fs::remove_dir_all` / `tokio::fs::rename` (`native.rs:486`, `:528-529`) and copies its tree with `spawn_blocking` (`native.rs:551`).<br>- F's own code is split: `link_out_of` runs under `spawn_blocking` (`docker.rs:1267`), but `remove_tree` does not.<br>- This is not a block. The agent runs the multi-threaded `#[tokio::main]` runtime (`main.rs:45`) and one command at a time (`supervisor.rs:491`), so the cost is one worker thread held for the length of a delete, not a stall. | Run `remove_tree` under `tokio::task::spawn_blocking` at the call sites, or add one async wrapper beside it. Use `tokio::fs::rename` for the cache rename. No behaviour or test change is needed. |
| 2 | warn | Round-2 warn 9 (renames covering both folders) | naming | **The rename stopped halfway.**<br>- F renamed the cleanup trio to `shared_folders_leftover`, `parse_shared_folders_leftover` and `remove_shared_folder(s)`.<br>- The creator keeps its old name, `create_shared_work_folder` (`docker.rs:752`), although Start now also creates the runner root with it (`:1085`). Its doc comment was updated, which round 2 allowed.<br>- The test module also keeps its old name, `shared_work_folder_tests` (`:1462`), although it now pins the runner-root parsing.<br>- So the "shared folder" family now has two vocabularies. | Rename `create_shared_work_folder` to `create_shared_folder` and the test module to `shared_folder_tests`. This is a pure rename and can ride along with the next F pass, or be left: the doc comment already says it covers both roots. |

**Checked and consistent (no finding):**
- **Round-2 warn 1 (setup placement), followed.**
  - `SHARED_RUNNER_ROOT_SETUP` (`:984`) is spliced in by `github_runner_script` (`:1003`) right after `IFS= read -r S`. In persistent mode that is before `args=(…)` and `./config.sh`.
  - Core's `instrumented_runner_script` is unchanged.
  - The variable is `GRIDOPS_RUNNER_EXTERNALS`, and `GRIDOPS_RUNNER_ROOT` is set, so the live-log forwarder reads the runner root's `_diag/pages`.
- **Round-2 warn 2 (split folder creation), followed.**
  - Prepare creates the cache root and staging folder plainly; `PermissionDenied` gives `Configuration, false` and any other IO error gives `Runtime, true` (`:1206-1220`).
  - Start creates the runner root with `create_shared_work_folder` (move-aside kept).
- **Round-2 warn 3 (order and leftovers), followed.**
  - `shared_runner` (`:1066`) resolves the id, checks the cache, then creates the work folder and the runner root.
  - A failed `docker create` or `feed_secret` removes both folders through the existing `remove_shared_folder(s)`. Its new `remove_dir`-first step only shortcuts the empty case; it is not a second removal path.
- **Round-2 warn 4 (the fill container), followed.** It is named `gridops-externals-<hex>`, removed with `rm -f` before `create`, created with `--entrypoint ""` and the command `true`, carries no `io.gridops.fleet.environment` label, and is removed after `cp` whether or not the copy succeeded.
- **Round-2 warn 5 (names and contract), followed.**
  - `shared_runner_root` takes `PreparedEnvironmentId`, not `impl Display`.
  - The cache name is lowercase hex without `sha256:`.
  - Staging is `<hex>.partial`, which the name function can never produce.
- **Round-2 warn 6 (one predicate), followed.** `uses_externals_cache` (`:897`) is the only condition, used by Prepare (`:118`) and Start (`:183`). In Start's GitHub path it is equivalent to the old `request.docker_socket` test, because GitLab returns earlier.
- **Round-2 warn 7 (one failure type), followed.** One private `Refusal` (`:862`) carries the code, the retryability and the reason; `answer()` logs and answers.
  - Its `tracing::warn!("{}", reason)` matches the file's existing log style: every warn on master is a formatted message with no named fields (`docker.rs@9e357e0:193`, `:512`, `:756`, `:820`).
  - So the diff gate's NIT-2 ("every other warn … uses named fields") does not hold.
- **Round-2 warn 8 (sweep under the lock), followed.** `_guard` (`:1132`) is held across the fill and `sweep_externals_caches`.
  - The sweep reuses the existing `missing()` through a plain `docker inspect`, so the shared helper is unchanged.
- **Round-2 warn 9 (renames):** done for the cleanup trio and its log line. The remainder is warn 2 above.
- **Decision 7 (container from the resolved id).**
  - The runner container is created from `sha256:<hex>`.
  - The only code that reads a container's image back is Cleanup's `{{.Config.Image}}` (`:781`; repo-wide grep). It passes the id to the throwaway `docker run`, which works because Cleanup reads it while the container still exists, so the image cannot have been pruned.
- **Decision 6 (mounts).** The cache is mounted with `--mount type=bind,…,readonly`. The runner root uses `-v` like the work folder, which is safe because the agent creates it first.
- **F's deviations are within the plan's contract.**
  - The `{{.Id}} {{json .Config}}` inspect gives decision 3's behaviour on Docker 29.
  - The link-escape refusal is `Configuration, false`, the class decision 5 gives to image content.
- **File layout.** `docker.rs` with the child test file `docker/externals_tests.rs` matches existing precedent (`gridops-api/src/webhooks.rs` with `webhooks/gitlab_tests.rs`; `resources.rs` with `resources/gitlab_tests.rs`).
- **Coexistence.**
  - origin/master has moved 3 commits (#186, #152, #130). Of the files this branch touches, they share only `CHANGELOG.md`, an ordinary text merge at land.
  - #130's new `PlacementFailureReason::RegistrationTimedOut` is server-side and does not affect decision 5's agent codes.
  - The one other worktree (#124) changes none of these files.

## Notes for O
The verdict is PASS, so nothing must change in the brief. One routing gap needs attention: **the diff gate's B-1 has no route to T.**
- **What B-1 asks for:** a failed-rename test for the fill (`docs/handoffs/126-diff-result-r1.md`).
- **Why it has no route:** `archChanged` sent the BLOCK to A without classifying it, so no `tNote` carries it.
- **It is test-only.** The failure can be reached through the existing `cache_root: &Path` seam with no production change: pre-create a regular file at `<cache_root>/<hex>`. `is_real_dir` is false for it, so the fill runs, and renaming the staged `externals` folder onto a file fails. The test asserts:
  - `(Runtime, true)`;
  - the reason text;
  - no `<hex>.partial` left behind;
  - no cache folder at `<hex>`.
- **For T (next phase):** add this test in this cycle and read the diff result's W-3 (pin the expected pair per environment in `a_github_prepare_sharing_the_socket_fails_when_it_cannot_fill_the_cache`).
- **For F:** warns 1 and 2 above can ride along with the next pass; neither needs a test change.

## Patterns referenced
- `crates/gridops-host-agent/src/docker.rs`, at 3f00753 (the production parts in full) and at 9e357e0 (the log-style and socket-branch baseline).
- `crates/gridops-host-agent/src/native.rs:480-560`: the analogous staged fill (`tokio::fs`, `spawn_blocking`).
- `crates/gridops-core/src/fleet/work_folder.rs` and `fleet/mod.rs`, as changed.
- `docs/handoffs/126/handoff-A.md` at 25e34e8 (the round-2 warns), `handoff-F.md`, `decisions.md`, `docs/handoffs/126-diff-result-r1.md`.
- The release's `workflow/coding-pipeline.workflow.mjs` (Phase 4 / 7 routing, `archChanged`) and `workflow-coding-pipeline.md` (rework table).
