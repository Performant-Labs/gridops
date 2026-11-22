## Brief Review (Round 1)

### BLOCK findings
**[B-1]** The brief's acceptance criteria are entirely external and not reproduced here. Every "Done when" list, the stub table, the file-ownership table, and the spec itself live in GitHub issues (#197, #198, #190–#195, #199–#204) whose text is not in this prompt. This reviewer cannot evaluate completeness, testability, or consistency of criteria it cannot read, and the brief even anticipates conflicts between those lists ("If any wave-1 issue's 'Owns' list conflicts with the table in #197...") without stating the conflict-resolution rule concretely. The brief as a standalone document is vacuous: its only internal testable criteria are "migration applies in under a minute" and "two guard-check PRs fail." — Fix: inline the Done-when lists (or the contract-test checklist derived from them) and the canonical ownership table into the brief, so the brief gate can actually judge testability, coverage, and boundedness. The plan cannot be validated against invisible sources.

### Needs-verification findings
**[NV-1]** Claim: `0049_fleet_inventory_snapshot_retention.sql` already exists on master, so the new migration is `0050` — evidence: `ls migrations/` output on the current branch/master.
**[NV-2]** Claim: wave-1 issues can be implemented without touching `main.rs`, `mod.rs`/`lib.rs`, the API router, `src/lib/navigation.ts`, or `page.tsx` composition — evidence: the #197 stub table plus the current contents of those files (do they expose registration points — e.g. a router registry, plugin list, nav data array — that this PR can pre-wire?).
**[NV-3]** Claim: `VACUUM INTO` on the Mars DB copy takes ~93 s and full migration application is "under a minute" — evidence: the recorded wall time from the actual run; nothing in this prompt supports either number.
**[NV-4]** Claim: `CARGO_BUILD_JOBS=2` / `RUST_TEST_THREADS=2` are set in `.claude/settings.local.json` and the shared target dir works via untracked `.cargo/config.toml` — evidence: contents of those files on the machine.
**[NV-5]** Claim: the project memory's throwaway-container recipe (`nginx:alpine` + `apk add sqlite`) yields a working read-only copy path — evidence: the project memory entry and a dry-run; a failure mode here silently blocks the timing criterion.

### WARN findings
**[W-1]** "Under a minute" migration timing is measured once, on one machine, against one snapshot — it is a sample, not a testable acceptance criterion, and has no defined pass condition if the machine is loaded. Recommendation: record the number as informational and set a soft threshold (e.g. 2× budget) rather than a hard gate, or state what happens on failure.
**[W-2]** "If Mars is unreachable, say so in the result rather than skipping silently" leaves the timing criterion unsatisfiable with no fallback (e.g. a synthetic DB of equivalent size). Recommendation: define a fallback dataset and its acceptance basis now, or explicitly mark the timing check as best-effort in the Done-when.
**[W-3]** The brief says "when in doubt, wire it here" for shared files but the actual arbiter of "shared" is the ownership table in #197 plus ten per-issue "Owns" lists — the single point of failure this whole wave depends on. Recommendation: generate `docs/design/fleet-health-owners.json` mechanically from the union of the lists and add a duplicate/missing-file check as an explicit contract test.
**[W-4]** The guard-check PRs are created "from branches off this story's branch" and then closed/deleted; if the story branch is force-pushed or rebased before merge, the guard demonstration no longer corresponds to merged code. Recommendation: run the guard checks against the final pre-merge SHA, or re-run them after any rebase.

### NIT findings
**[NIT-1]** "Use the next free number (`0050` today; re-check with `ls migrations | tail -1`)" — `tail -1` on lexicographic names breaks past `0099` and misorders if any non-padded name exists; `ls migrations | sort | tail -1` is safer.
**[NIT-2]** The brief says to record the timing result but doesn't say where — recommend naming the destination (e.g. a section in `docs/design/fleet-health.md`) so it isn't lost in the PR description.

### Verdict
BLOCK — 1 blocking finding; must resolve before implementation starts.