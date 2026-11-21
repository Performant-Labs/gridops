## Brief Review (Round 1)

### BLOCK findings
None.

### Resolved findings
**[R-1]** Earlier W-1: "the round-3 manager-path reset … depends on #140 landing; if it does not, a working manager-path GitLab pool only resets on exit. … Recommend recording that dependency in Done-when or accepting the exit-only reset explicitly." Resolved: the round-2 amendment "Dependency on #140 for the manager-path busy reset (W-1)" states the consequence ("until #140 is on master, a working manager-path GitLab pool resets its failure count only on exit"), routes the Done-when "count is reset" test through the exit path, and commits to a busy-reset test once #140 lands.

**[R-2]** Earlier W-2: "'every attempt counts' is slightly loose and could be read as requiring attempts past circuit-open." Resolved: W-3 now reads "each GitLab start attempt that reaches `create_runner` before the circuit opens fails there and counts exactly once (no attempt is started while the circuit is open)".

**[R-3]** Earlier NV-1: "the confirmations themselves are not shown in this prompt." Resolved: the "Excerpts for the checked-against-master facts" section quotes the cited lines directly (`main.rs:2766-2767`, `main.rs:2810`, `fleet.rs:264-267`, `service.rs:805`, `gitlab/mod.rs:88`, `targets.rs:741`, `retry.rs:226-228`, `provision_failure_policy`'s `if attempts >= 3`, and the 0047/0001 schema lines).

**[R-4]** Earlier NIT-1: "`runner_pools.labels` is cited … but no excerpt is shown." Resolved: `` `labels` text DEFAULT '[]' NOT NULL, `` at `0001_initial.sql:104` is now quoted, with the migration-grep note ruling out redefinition (0022's `labels_json` is a fleet table; 0046 is a separate table).

### Needs-verification findings
**[NV-1]** The quoted excerpts settle the previously unverifiable master facts, but the excerpt block still quotes no text for `GITLAB_DEFAULT_WAIT_TIMEOUT_SECS = 600` or `GITLAB_EXIT_REASONS` in `crates/gridops-core/src/provisioning.rs` (only "lines 1-35" were read) — settled by quoting those definitions; if the constant or the `idle_expired`/`completed`/`registration_refused` spellings differ, the W-1 fixture rule and D12 inherit the error.

**[NV-2]** D5 / host-agent-facts outcome shapes and #140's webhook busy status remain descriptions of unmerged #147/#140 behavior — settled when those diffs exist; the "Stub fixtures mirror the published contract" rule mitigates drift by routing through the real types/constants.

**[NV-3]** That #146 and #148 land shapes matching the D17 signature and the shared `gitlab` field name/type — settled by those PRs' diffs; the rebase rule mitigates.

### WARN findings
**[W-1]** Round-3's fleet-path counting rule branches on `rows_affected() == 1` of a guarded UPDATE in `follow`/`finish`, but the quoted excerpts do not show the current guarded UPDATE exists or that `finish`/`follow` can observe `rows_affected` (only the `finish` re-visit loop at `fleet.rs:264-267` is quoted). If the guarded update must be introduced, that is a `fleet.rs` edit the seam list ("`provision`, `register_and_start`, `follow`, `finish` gain GitLab arms") may or may not cover. Recommend naming the guarded UPDATE's introduction explicitly as in-scope so the diff gate can recognize it.

### NIT findings
**[NIT-1]** `FakeGitLab` line numbers ("`pub enum FakeMethod {` (21)", `with_scope` (88), etc.) are cited as evidence locations without the surrounding text; the symbols' existence is plausible but the numbering itself proves nothing and will rot — drop line numbers from the record or quote the signatures.

### Verdict
PASS — no BLOCK findings; implementation may proceed.