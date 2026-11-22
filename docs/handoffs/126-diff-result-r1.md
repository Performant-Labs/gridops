## Implementation Review (Round 1)

### BLOCK findings

**[B-1]** crates/gridops-host-agent/src/docker/externals_tests.rs (test set) — The brief's item 5 enumerates the required failure-path tests for a Docker/IO error during the fill: "stand-in `docker` failing `create`, `cp` or `rm`, **or a failed rename** → `Runtime, true`, and no staging directory is taken for a cache". The implemented test `a_docker_error_during_the_fill_is_retryable_and_leaves_no_cache` covers `create`, `cp`, `rm`, `inspect-tag`, and a bad id — the failed-rename case (`std::fs::rename` of `<hex>.partial/externals` to `<hex>` failing, e.g. a read-only cache root created after the staging fill, or a pre-existing target of another type) has no test — why it blocks — it is an explicitly enumerated acceptance criterion of the brief, and the rename path is the one step that makes the fill atomic; its error mapping (`Refusal::runtime` in `fill_externals_cache`) and its "no staging taken for a cache" guarantee are untested — remediation — add a case (e.g. pre-create a non-directory at `<cache_root>/<hex>` or make the cache root unwritable after staging, mirroring the 0555-probe skip pattern) asserting `(Runtime, true)` and that no `<hex>` cache and no leftover staging remain.

### Needs-verification findings

**[NV-1]** crates/gridops-host-agent/src/docker.rs (`shared_runner` / the runner-root `externals` symlink) — The premise that the host kernel/Docker daemon follows the runner root's `externals` symlink when resolving the bind-mount source for `/__e` at job-container creation time — evidence that would settle it: a real-engine run (a daemon bind-mounting a path whose last component is a symlink to a read-only-mounted cache), or the Docker daemon's source-mount resolution code; the handoff itself lists this as "not exercised".

**[NV-2]** crates/gridops-host-agent/src/docker.rs (`{{.Id}} {{json .Config}}` inspect) — The claim that `{{.Config.WorkingDir}}` is a template error on Docker 29 for images without a WorkingDir, and that `{{json .Config}}` works on both Docker versions — the stand-in models this behaviour, it does not prove it; evidence: `docker image inspect -f …` against real Docker 29 and an older daemon, or the CLI's template engine source.

**[NV-3]** crates/gridops-host-agent/src/docker.rs (tool-cache precedence, `.env` override) — Claims that the runner resolves the tool cache as `RUNNER_TOOL_CACHE` → `AGENT_TOOLSDIRECTORY` → `<work>/_tool` and applies its root's `.env` over its environment, and that 2.338.2's `env.sh` lists neither variable — evidence: HostContext.cs:409-415 and Program.cs:172-194 of actions/runner 2.338.2 (not in this prompt, not in this repository).

**[NV-4]** crates/gridops-host-agent/src/docker.rs (runner 2.338.2 mount mapping) — The claim (decision 1) that the runner maps host paths checking Work before Externals, so a runner root inside `<work>` would break — evidence: ContainerInfo.cs of actions/runner 2.338.2; third-party, not shown here.

**[NV-5]** crates/gridops-host-agent/src/docker.rs (`create --entrypoint "" <id> true`) — The claim that a fill container can be created this way even for an image with no `Cmd` — evidence: Docker daemon docs/source for entrypoint/cmd defaulting with an empty entrypoint override.

**[NV-6]** docs/fleet-agent.md (measured sizes) — The per-start copy (272 files / ~85 MB) and cache (~9,122 files / ~600 MB) figures come from the upstream release tarball, not the pool image (which could not be pulled); the docs present them as "for runner 2.338.2" without the estimate caveat the brief asks for ("measured from the pool image when the implementer can pull it locally, otherwise estimated, saying which") — evidence: a pull of `harbor.performantlabs.com/performantlabs/pl-runner:1.70.4` with credentials, and `du`/`find` on its `/actions-runner`.

### WARN findings

**[W-1]** crates/gridops-host-agent/src/docker.rs (`SHARED_RUNNER_ROOT_SETUP`) — The copy loop's second glob, `"$gridops_image_dir"/.[!.]*`, misses dot-entries beginning with two dots (e.g. `..bashrc`-style names) and any `..foo` entry, so such files in an image's runner directory would not reach the runner root — recommendation — use `find` or add a `..?*` glob arm (with the same `-e/-L` existence guard), or state the limitation in the doc comment.

**[W-2]** crates/gridops-host-agent/src/docker.rs (`fill_externals_cache`) — The early return when the cache already exists (`is_real_dir(&target)`) skips the stale-cache sweep entirely, so a host whose every Prepare finds a warm cache never sweeps `*.partial` leftovers or caches of removed images — decision 8 says "after a new fill", so this is within spec, but a long-lived host with one pinned image accumulates staging leftovers from crashed fills until some new image arrives — recommendation — also sweep `.partial` folders (cheap, no Docker calls) on the early-return path, or note the accumulation in docs.

**[W-3]** crates/gridops-host-agent/src/docker/externals_tests.rs (`a_github_prepare_sharing_the_socket_fails_when_it_cannot_fill_the_cache`) — The assertion accepts either `(Configuration, false)` or `(Runtime, true)`, so it cannot detect a regression that returns the wrong pair for the specific failure that occurred (permission vs. daemon) — recommendation — pin the expected code per environment (or assert the reason text, as the other tests do).

### NIT findings

**[NIT-1]** crates/gridops-host-agent/src/docker.rs (`link_out_of`) — Traversal does not follow symlinked directories, so a link inside a symlinked subdirectory whose target lies inside the tree but outside that subdirectory's own path is accepted only because the top-level canonicalize covers it; fine, but the doc comment could say links to directories are checked, not traversed.

**[NIT-2]** crates/gridops-host-agent/src/docker.rs (`Refusal::answer`) — Logging `self.reason` with no structured fields or operation context; every other warn in the file uses named fields.

### Verdict
BLOCK — 1 blocking finding (B-1, the missing failed-rename fill test the brief enumerates); must resolve before testing starts.