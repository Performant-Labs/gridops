## Implementation Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** docker.rs (fill_staging) — the claim that `docker cp -L <container>:<path> <dest>` on a real engine follows a symlinked top-level `externals` and preserves inner symlinks while making the files agent-owned — the evidence that would settle it is a real-daemon transcript of `docker create`/`docker cp -L`/`docker rm` against an image with a symlinked `externals`, or the Docker CLI source for `cp -L`.

**[NV-2]** docker.rs (shared_socket_args) — the claim that `--mount type=bind,…,readonly` fails (rather than silently creating) when the source is missing, and that the host daemon resolves the runner root's `externals` symlink at job-container mount time (the brief's own premise) — evidence: Docker daemon bind-mount source-resolution code / a live-daemon test mounting through a symlinked path.

**[NV-3]** The runner-behaviour claims this design rests on: that runner 2.338.2 derives `/__e` from `<install root>/externals` (root = parent of running `bin/`), maps Work before Externals in ContainerInfo.cs, resolves the tool cache as `RUNNER_TOOL_CACHE` → `AGENT_TOOLSDIRECTORY` → `<work>/_tool` (HostContext.cs), and applies the root's `.env` over its environment (Program.cs) — all third-party; the actions/runner 2.338.2 source for ContainerInfo.cs, HostContext.cs and Program.cs would settle them. The handoff itself lists these as not exercised.

**[NV-4]** docker.rs (SHARED_RUNNER_ROOT_SETUP) / round-1 design notes — the claim that the 2.338.2 release `env.sh` lists neither `RUNNER_TOOL_CACHE` nor `AGENT_TOOLSDIRECTORY`, so the image ENV cannot smuggle `/opt/hostedtoolcache` back into `.env` — evidence: the runner release's `env.sh` excerpt (decision 9 explicitly required this check; the `.env` rewrite mitigates but the env.sh claim itself is unverified here).

**[NV-5]** externals_tests.rs (STAND_IN) — the claim that Docker 29.8 emits exactly "Could not find the file … in container …", "No such image", "no such object", and the `{{.Config.WorkingDir}}` "map has no entry for key" template error, which the `Configuration`/`Runtime` classification (`cp` error-text sniffing, `missing()`) depends on — evidence: real-daemon transcripts (the test comment says they were "checked against a real daemon while writing these tests", but no transcript is in this prompt).

**[NV-6]** docs/fleet-agent.md — the per-start (272 files / ~85 MB) and per-cache (9,122 files / ~599 MB) sizes, measured from the release tarball rather than the pool image — evidence: `docker image inspect` + a directory count/du of the pool image `pl-runner:1.70.4`, or at minimum the tarball listing the figures were computed from. The docs do correctly label these as estimates, per brief item 1.

**[NV-7]** docker.rs (remove_tree) — the claim that `spawn_blocking(...).await?` converts a `JoinError` into an `io::Error` via tokio's `From` impl, as native.rs:551 (`.await??`) uses — evidence: the tokio source for `impl From<JoinError> for io::Error`. The shown native.rs excerpt does not include :551, so the precedent is cited but not shown.

### WARN findings
**[W-1]** externals_tests.rs (`a_fill_sweeps_caches_whose_image_is_gone_and_leftover_staging`, "Docker does not answer" case) — the test filters `.partial` names out of `left` when `fail == "inspect-other"`, so in that case the assertion is vacuous about whether the staging leftover was removed; only the first case actually pins staging removal. — Recommendation: assert staging removal explicitly in both cases (the code does sweep it unconditionally; the test just doesn't check it there).

**[W-2]** externals_tests.rs (`a_docker_error_during_the_fill_is_retryable_and_leaves_no_cache`) — the brief asks the test to assert "no staging directory is taken for a cache"; this test only asserts `!cache_root.join(HEX).exists()` and does not assert the `<hex>.partial` staging folder was cleaned up on the `create`/`cp` failure paths (only the failed-rename test checks `entries(&cache_root)`). The code does clean it (`let _ = remove_tree(&staging)`), but the failure-path test doesn't pin it. — Recommendation: assert `entries(&cache_root)` is empty in those cases too.

**[W-3]** docker.rs (`remove_shared_folder`, `create_shared_folder`) — `std::fs::remove_dir` and the blocking `create_dir_all`/`rename` in `create_shared_folder` run directly on the async runtime; round 3 moved only `remove_tree` off it. These are single-syscall or small-tree operations, so the risk is low, but it is inconsistent with the round-3 rationale. — Recommendation: note the discrepancy or wrap the move-aside path if roots can be large.

### NIT findings
**[NIT-1]** externals_tests.rs (`the_cache_is_filled_once_per_image…`) — `ensure!("{:?}", dangling_links(path)?)` evaluates the (empty) vector twice; cosmetic.

**[NIT-2]** docker.rs (`sweep_externals_caches`) — a non-hex, non-`.partial` entry under the cache root is silently kept forever with no log; a `tracing::debug!`/`warn!` naming it would make an unexpected leftover diagnosable.

### Verdict
PASS — no BLOCK findings; testing may proceed.