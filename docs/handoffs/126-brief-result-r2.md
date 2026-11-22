## Brief Review (Round 1)

### BLOCK findings
None.

### Resolved findings
**[R-1]** "Requirement 3 ('fail loudly') has no test requirement." — Item 5 now mandates: "Every failure path of requirement 3 / decision 5 has a test asserting the exact `(FailureCode, retryable)` pair, exercising the same pure functions / refactor as decision 10," enumerating each path (missing WorkingDir, missing `externals`, permission failure, fill errors with no staging dir taken, Start cache-missing in both JIT and persistent modes with no `docker create` issued) plus the logged-reason assertion.

### Needs-verification findings
**[NV-1]** Runner 2.338.2 maps host paths to job-container paths checking Work before Tools/Externals, first match wins (decision 1's premise). — The relevant excerpt of `ContainerInfo.cs` (path-mount resolution loop) from the actions/runner 2.338.2 source; the brief explicitly does not quote it ("third-party; handoff-A records them as read from the upstream source, and this brief does not quote them").
**[NV-2]** Tool-cache resolution order is `RUNNER_TOOL_CACHE`, then `AGENT_TOOLSDIRECTORY`, then `<work>/_tool` (brief facts and decision 9). — `HostContext.cs:409-415` of actions/runner 2.338.2, shown verbatim.
**[NV-3]** The image's `.env` is loaded by the runner listener over its environment and could override `RUNNER_TOOL_CACHE` (decision 9's "handle it if it can"). — `Program.cs:172-194` of actions/runner 2.338.2 plus the `.env` precedence semantics vs. process environment.
**[NV-4]** The runner derives `<root>/externals` from the parent of the running `bin/`, not an env var (brief facts, basis of the whole symlink design). — The runner's install-root/externals resolution code in actions/runner 2.338.2.
**[NV-5]** The host kernel follows a symlinked bind-mount source at mount time; and `--mount type=bind,source=…` errors on a missing source while `-v` auto-creates it (decision 6). — Docker/mount documentation or a kernel-level excerpt; a reproduced transcript.
**[NV-6]** The error `exec: "/__e/node24/bin/node": stat …` is caused by the daemon bind-mounting empty auto-created host directories. — Docker daemon behavior docs for missing `-v` sources, or a reproduced transcript showing empty root-owned dirs at `/actions-runner/externals` on the host.
**[NV-7]** The brief's citation of `provisioning.rs:64-69` for the 25 s Start budget — the excerpt shown here covers `:64-76` and matches ("must all finish within 25 s"), and `fleet.rs:51` (30 s) and `commands.rs:32` (5 s) are shown and match; but the Prepare 20-minute claim rests on `admission.rs:1151` `.checked_add(1_200_000)`, shown — 1,200,000 ms = 20 min, consistent. No finding; this NV is discharged by the shown excerpts.
**[NV-8]** The docker_runtime.rs:160-211 precedent (`!recorded.contains("/home/runner/run.sh")`) and work_folder.rs contents — cited but docker_runtime.rs excerpts are not shown in this prompt (excerpt cap reached; one cited file omitted). The citation's claims are plausible and consistent with the shown GitLab test's style, but remain unverified in-prompt.

### WARN findings
**[W-1]** The brief specifies sanitization for the cache-name function (`{{.Id}}` → single path component, unit-tested) but imposes no equivalent check on `<environment id>` used in `/var/lib/gridops-agent/runners/<environment id>` — a malformed or adversarial environment id would be joined into a host path unsanitized. — State whether environment ids are already constrained (e.g. by the id types) or add the same single-component guarantee for the runner root.
**[W-2]** Decision 8 (remove caches whose image no longer exists "locally" via the existing `missing()` check) is not covered by any test requirement in item 5 — the stale-cache sweep is a stated deliverable with no acceptance criterion. — Add a unit test (stand-in `docker` reporting a missing image) asserting stale caches and staging leftovers are removed and live caches kept.
**[W-3]** Requirement 1 says the per-image cache is "filled once" and decision 6 adds a Mutex/staging/rename, but the brief does not state what happens when two different images or a re-pull changes the id for the same tag while a runner is mid-start (decision 7 pins creating the container from the id, which mitigates it, but the invariant is unstated as a criterion). — A sentence pinning that a Start resolves the id itself and never reuses a tag-resolution from a previous step.

### NIT findings
**[NIT-1]** Item 5's permission-failure test "skipping when run as root" mirrors an existing pattern but the skip condition is not spelled out (skip when `geteuid() == 0` or an equivalent); name the mechanism so the diff gate can check it.

### Verdict
PASS — no BLOCK findings; implementation may proceed.