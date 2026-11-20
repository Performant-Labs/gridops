## Brief Review (Round 1)

### BLOCK findings

**[B-1]** The `fleet_ready.sql` acceptance criterion in "Done-when, extended for A-1" is self-contradictory: it claims `fleet_ready.sql` has "no `runners` rows" (verified in the Clarifications section), yet the same criterion says to test that "the `fleet_ready.sql`'s Bitbucket and GitHub targets survive" and that "that fixture carries the whole fleet graph that references targets." The fixture cannot simultaneously contain no `runners` rows and carry a complete fleet graph (which includes `workload_placements` requiring `runners`). This makes the acceptance test vacuous for the `runners` rebuild hazard: the test cannot exercise the cascade/set-null foreign key behavior on `runner_events` and `log_streams` if those tables are empty. The criterion must specify a fixture that actually contains `runners` rows with inbound references, or the test must be explicitly acknowledged as not covering the `runners` rebuild hazard.

### Needs-verification findings

**[NV-1]** The claim that `fleet_ready.sql` contains "no `runners` rows" — the evidence would be the actual fixture content, which is listed in "Source material read" as reviewed but its text is not shown in this prompt. Name: `crates/gridops-core/tests/fixtures/fleet_ready.sql`.

**[NV-2]** The claim that `fleet_ready.sql` "inserts the fleet graph (hosts, domains, backends, `fleet_ci_targets` including a `bitbucket_workspace` target, placements, allocations, commands)" — the evidence would be the fixture's INSERT statements, not shown here.

**[NV-3]** The claim that `fleet_legacy.sql` contains `runner_events` row `legacy-event` and `log_streams` row `legacy-log` — the fixture text is not shown, only asserted.

### WARN findings

**[W-1]** The brief states `fleet_ready.sql` has no `runners` rows (for the legacy upgrade test), yet the extended done-when criteria imply it should test the full fleet graph including `runners`. This suggests the test plan may need a third fixture or an amendment to `fleet_ready.sql` to include `runners` data, otherwise the `runners` rebuild hazard on inbound FKs is only tested on `fleet_legacy.sql`, not on a realistic post-0046 production graph.

### NIT findings

**[NIT-1]** The phrase "the `fleet_ready.sql`'s Bitbucket and GitHub targets survive" in the extended done-when is grammatically awkward and could be misinterpreted; clarify that it means the fixture's existing Bitbucket and GitHub target rows remain intact after migration 0046.

### Verdict

BLOCK — 1 blocking finding(s); must resolve before implementation starts.