## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** The runner derives the `/__e` source from its install root (`<root>/externals`) rather than an env var — this shapes the whole design. Evidence: actions/runner source (e.g. `src/Runner.Worker/Container/ContainerSetup.cs` or equivalent where `__e` mount source is computed), the release tarball for 2.338.2, or a runner documentation page.
- **[NV-2]** Tool-cache resolution order `RUNNER_TOOL_CACHE` → `AGENT_TOOLSDIRECTORY` → `<work>/_tool`. Evidence: actions/runner source excerpt showing tool-cache path resolution.
- **[NV-3]** The agent's docker branch is at `crates/gridops-host-agent/src/docker.rs` around line 180 in `start` for the `request.docker_socket` case, and cleanup is via `shared_work_leftover` / `remove_shared_work_folder` in `gridops_core::fleet::work_folder`. Evidence: those file:line excerpts from the repo.
- **[NV-4]** The host Docker daemon auto-creates empty root-owned directories for nonexistent bind-mount sources and leaks `/actions-runner/externals` and `/opt/hostedtoolcache` on hosts. Evidence: Docker documentation on bind mounts (behavior when source doesn't exist) plus a `docker inspect`/`ls -ld` of the leaked directories on a fleet host (out of scope per constraints — the brief may assert this from prior read-only observation, but no excerpt is shown).
- **[NV-5]** "Many runners start per host" — the copy-cost concern depends on host runner density. Evidence: fleet data on concurrent runners per host.

### WARN findings
- **[W-1]** Item 1 leaves the cache shape to "architect decides" while item 5 demands specific tests of it; the acceptance criteria are therefore only testable after that decision. The brief is arguably not fully self-contained on its own acceptance criteria — recommend the brief name one concrete preferred shape as the baseline (it does describe one: per-image cache under StateDirectory + per-env runner root) and state that deviations must be recorded. Minor, since a concrete shape is described.
- **[W-2]** Concurrency requirement "concurrent starts of the same image must not race or see a half-filled cache" needs an explicit acceptance criterion: e.g. N parallel agent starts of the same image all succeed and the cache directory is complete. As written it is testable in spirit but no test is listed in item 5 for concurrency beyond "atomic cache fill on a temp dir" — recommend item 5 explicitly include a concurrency test.
- **[W-3]** Missing error-path criterion for the tool cache: item 2 says set `RUNNER_TOOL_CACHE`/`AGENT_TOOLSDIRECTORY` to `<work>/_tool`, but if `<work>/_tool` cannot be created the runner presumably still starts and falls back to the bad behavior. Recommend specifying creation failure handling (or stating it's covered by existing work-folder setup failure paths).
- **[W-4]** Stale cache eviction rule is left open ("A bounded, simple rule is enough") with no acceptance criterion. Recommend requiring the plan state the rule and a unit test for it; otherwise item 5's "cache-path/cleanup parsing" tests don't cover eviction.
- **[W-5]** Edge case unaddressed: two different image tags with the same image id, and image id reuse after the image is retagged/updated — cache keying by image id is sound but the brief should note digest-vs-id choice. Low risk.

### NIT findings
- **[NIT-1]** The brief says the reconciler/server is unchanged "unless the plan finds otherwise" — slightly open-ended scope; fine, but the result should state explicitly that it was checked.
- **[NIT-2]** Item 3 says "pick the right `FailureCode` and retryability" — good; also pick the log target/channel so the failure is visible to operators, since the constraint says hosts can't be inspected live.

### Verdict
PASS — no BLOCK findings; implementation may proceed.