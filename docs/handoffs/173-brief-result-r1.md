## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** `scripts/prepare-tart-runner-image.sh` (GitHub) already runs `sync` in its guest script and has the flush/stop shape the new code should copy — the excerpt of that script's guest script and trap (file:lines showing the `sync` line and `tart stop`/`wait` handling) would settle it. The brief asks to "confirm and leave it" but provides no excerpt.

**[NV-2]** Exit 78 is currently outside `GITLAB_EXIT_REASONS`, `Exit::from_exit_reason` returns `Unknown` for it, and `left_service` no-ops on `Unknown` — the relevant excerpt of `crates/gridops-core/src/provisioning.rs` (the table and its doc) and `crates/gridops-reconciler/src/gitlab_lifecycle.rs` (`from_exit_reason`, `left_service`) would settle it.

**[NV-3]** The Tart agent's `runner_script` has the `75|77) ... exit 70` collision guard at `crates/gridops-tart-agent/src/gitlab.rs` ~160-163, and core's `GITLAB_RUNNER_SCRIPT` has the matching placeholder substitution at `crates/gridops-core/src/provisioning.rs` ~198 — excerpts of both would settle the exact shape the extension must fit.

**[NV-4]** The manager maps Docker container exit codes through core's table at `crates/gridops-manager/src/main.rs` ~1626-1639 and relays `exitReason` from the Tart agent unchanged — those excerpts (plus the manager doc ~627-629) would settle whether "no change beyond tests" holds.

**[NV-5]** No fleet runtime runs Tart (`crates/gridops-host-agent/src/runtime.rs` ~51-58 returns `Unsupported`), justifying leaving `from_inspect`/`fleet_start_failed` alone — the runtime.rs excerpt would settle it.

**[NV-6]** The reconciler's counted-once guarantee lives in `runner_exited`'s guarded UPDATE (~359-372), and the Tart agent does not depend on `gridops-core` (hence the duplicated table) — excerpts of `runner_exited` and the tart-agent `Cargo.toml` would settle both.

**[NV-7]** 78 is the correct `EX_CONFIG` sysexits code, matching the doc convention used for 69 — a sysexits.h / documentation excerpt (`EX_CONFIG 78`) would settle it.

**[NV-8]** The live-run observations (VM every ~minute, exit 78 each time, "Provisioned 1" events, `provisionFailureCount` staying 0, runner registered/deleted per attempt) — CI logs or pool event records from run #143 would settle them; not checkable from this prompt, and nothing in the brief depends on their exact details beyond motivating the change.

### WARN findings
**[W-1]** Destructive failure path: if the fresh-clone verification fails for an unrelated reason (boot flake, guest-agent timeout, disk pressure), the script deletes the target image. The brief distinguishes check-failure (delete target) from interruption (keep target), but not "clone booted but agent never answered" from "binary genuinely missing." Recommend the deletion trigger be narrowed to the binary check failing, with agent-timeout/boot failure treated as an ordinary non-zero exit that keeps the target (and still cleans the clone).

**[W-2]** Verification mechanism unspecified: the fresh-clone check ("binary exists, is executable, `--version` runs") runs inside the guest, but the brief never says how the script executes it (guest-agent command via `tart run`? `tart exec`?). The test criterion says the fake `tart` "can be told the clone has no binary," which presupposes a mechanism that routes through the `tart` binary. State the mechanism (e.g., which `tart` subcommand carries the check) so the fake's contract and the criterion are concretely testable.

**[W-3]** Conflicting guidance on sharing code: the Scope section says "Share the clone-boot-check-cleanup logic between the two scripts if that stays simple … otherwise keep each script self-contained," while the later "Shared code (review W3)" note says "Prefer one boot-and-wait function per script." These point in opposite directions for the same decision. Pick one as the default and say in handoff-F which was chosen — as written, an implementer can justify either and a reviewer cannot tell deviation from compliance.

**[W-4]** "Confirm and leave it" for `prepare-tart-runner-image.sh` is a verification task with no Done-when item: nothing records the confirmation outcome. Either add a handoff note requirement stating the sync line was observed (with line number), or drop the instruction — an unverifiable instruction in a brief with testable acceptance criteria is dead weight.

**[W-5]** Coexistence with #171 in `gitlab_lifecycle.rs` is bounded by file-region ("touch only `Exit`, the reason helpers, `left_service` and the Outcomes doc"), but the brief cannot guarantee #171 does not also touch `Exit` or the reason helpers (it names `deregister_runner`, `fail_unstarted`, the sweep — plausibly reason helpers are shared). Rebase-conflict risk is real; recommend the brief name the merge order or a conflict-resolution owner for overlapping symbols.

### NIT findings
**[NIT-1]** "Delete the target image so no broken image is left under the target name" — deleting and *re-preparing* vs just deleting leaves the pool with no image at all; the pool's next provisioning will then fail differently (image missing). Worth one sentence on whether that downstream state is acceptable or should also get an event.

**[NIT-2]** The Done-when item "the manager test shows it survives the re-parse" names no test; other criteria name specific tests. Name the manager test (or say a new one is added) so the criterion is checkable.

**[NIT-3]** "or a better name you justify" followed immediately by W8 fixing the name — the optionality is already resolved; delete the hedge.

### Verdict
PASS — no BLOCK findings; implementation may proceed.