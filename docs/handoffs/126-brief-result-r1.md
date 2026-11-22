## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** Claim: the actions/runner derives the `/__e` mount source from its install root and maps Work (`/__w`) before Tools and Externals in ContainerInfo.cs — the evidence would be the actions/runner 2.338.2 source of ContainerInfo.cs (and HostContext.cs:409-415 for the `RUNNER_TOOL_CACHE` lookup order, Program.cs:172-194 for `.env` loading) — all third-party, explicitly not quoted in this brief. The entire fix design (runner root outside the work folder, decision 1; tool-cache env precedence, decisions 2/9) rests on these unshown behaviors.

**[NV-2]** Claim: Docker's daemon resolves bind-mount sources on the host and silently creates missing source directories (motivating the `--mount`-not-`-v` rule in decision 6) — a Docker documentation page or daemon source excerpt would settle it.

**[NV-3]** Claim: the fleet pool image `pl-runner:1.70.4` has `AGENT_TOOLSDIRECTORY=/opt/hostedtoolcache`, an empty tool cache, and 683 MB of `externals` — the `docker image inspect`/`du` output from `callisto` would settle it; only the brief's own field notes assert it.

**[NV-4]** Claim: the agent runs `docker create` for container jobs with exactly the three mounts `-v /…:/__w -v /actions-runner/externals:/__e -v /opt/hostedtoolcache:/__t` (issue-reported symptom) — an excerpt of the runner's job-container invocation or issue #126's log text would settle it; `docker.rs` is said to be read in full but no excerpt showing these mounts is attached.

**[NV-5]** Decision 9's own open question: whether the image's `.env` (Program.cs:172-194) can override `RUNNER_TOOL_CACHE` — requires the actions/runner Program.cs source plus the pool image's `.env` contents; the brief defers this ("check … and handle it if it can") without specifying what "handle" means, which risks an untestable criterion if the override turns out to be possible.

**[NV-6]** Claim: `moves_an_unwritable_root_aside_and_starts_a_fresh_one` at `docker.rs:977-980` contains the root-probe early-return pattern the new tests should copy — the `docker.rs` excerpt is not among the attached excerpts (the file was read but not shown); a verbatim excerpt of `docker.rs:845-852` (`missing()`) and `:970-995` would settle both this and the stale-sweep test premise that `missing()` recognises "no such object".

### WARN findings
**[W-1]** Requirement 5's log-assertion criterion ("the failing function returns an error whose text names the image id and the path, the caller logs that text") couples a unit-testable property (error text) to an untested behavior (the caller's `tracing::warn!`). The tests as specified can only assert the error text, not that it is logged; the criterion should say so to avoid a vacuous acceptance claim.

**[W-2]** Acceptance criterion "State what the copy costs per start … measured from the pool image when the implementer can pull it locally, otherwise estimated, saying which" is satisfiable but weakly testable — an estimate with no bound cannot fail review. Recommend requiring at least an order-of-magnitude sanity check against the stated 683 MB externals figure.

**[W-3]** Decision 2 fills the cache only when `request.docker_socket && !prepare.target.is_gitlab()`, but requirement 3's Start check is scoped to "the socket branch." If a placement's docker-socket flag could differ between Prepare and Start (retry after profile change), Start would demand a cache Prepare never filled. The brief should state the flag is stable across the placement's lifetime, or make the Start check conditional on the same predicate.

**[W-4]** Stale-cache sweep (decision 8) removes caches whose image "no longer exists locally," but a cache filled in Prepare for a placement that later re-pulls is keyed by id and preserved — good. However nothing specifies ordering between concurrent fills of different images on the same host and the sweep that deletes "leftover staging directories"; a just-created staging directory of an in-flight fill could be swept. NativeRuntime's Mutex only serialises within one NativeRuntime; clarify whether the new fill holds an equivalent lock across sweep+fill.

### NIT findings
**[NIT-1]** The citations section cites `docker.rs:977-980` for the probe while requirement 5 says `docker.rs:977-980` covers the early return — consistent, but `docker.rs` excerpts were omitted from the appendix despite being cited; include them next round for self-containment.

**[NIT-2]** Requirement 6 places the "hosts need the new agent build" statement in two places (CHANGELOG Upgrading and fleet-agent.md); fine, but say which sentence is authoritative if they drift.

### Verdict
PASS — no BLOCK findings; implementation may proceed.