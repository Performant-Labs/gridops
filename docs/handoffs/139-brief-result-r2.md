## Brief Review (Round 1)

### BLOCK findings
None.

### Resolved findings
**[R-1]** [B-1, previous round] "Contradiction between 'Stub fidelity' (Definitions, brief-gate R1) and W-1 … on how fixture values are written." The Definitions entry now reads "never typed as string literals in the test" and explicitly records the amendment: "(Amended at brief-gate round 3, finding R3-B-1: this sentence previously said 'written as literals in the fixtures', which contradicted W-1.)" W-1 (constants/typed values) now governs unambiguously.

### Needs-verification findings
**[NV-1]** D14's dependency: that #138 will make `starts_belong_to_reconciler` true for GitLab pools before #139 depends on it — the excerpt confirms master (`resources.rs:4166-4168`) is still Tart-only (`providers.iter().any(|provider| provider == "tart")`), so the GitLab behaviour exists only on the unmerged `claude/gitlab-138-api-44d3a8` branch whose diff is not shown here; evidence: that branch's `resources.rs` diff or its brief's final text.
**[NV-2]** The #147/#140 manager/agent behaviours (exit-77 late refusal, `exitReason` production, `busy` webhook) are not on master; the brief bounds this with the stub-fidelity rule, so it needs only landing-time confirmation — evidence: the merged #147/#140 diffs.
**[NV-3]** D17's comment URL (issuecomment-7053069685) is quoted as the source of the `list_runners` contract; the comment itself is not in this prompt — evidence: that issue comment, if the exact signature ever matters to a later gate.
**[NV-4]** The prior NV-2/NV-3/NV-5 claims (sync call sites at `targets.rs:165/540`, profile readability, placement signatures) are now backed by quoted excerpts in the brief's round-2/round-3 amendment sections and are consistent with the attached `fleet.rs` excerpt (`provision` at `:152` receives `&Profile`, matching R3-W-3 and R3-NV-3); no further verification needed at this gate.

### WARN findings
**[W-1]** The brief still carries three "Done when" sections plus round amendments; the consolidated one is declared authoritative and item 4's master-state dependency is now conditioned on a PR-description record (R3-W-1 incorporated), which is acceptable but makes the eventual diff gate responsible for checking that record exists and matches the tests actually written.
**[W-2]** Scope breadth (carried from previous W-2, unaddressed and acceptable-by-design): #139 owns D17 core trait/fake additions, admission arms, pool-target sync, retry column copy, image substitution, sweep, and a two-path failure circuit across shared core files. Individually testable, but the diff gate should verify each seam's tests landed rather than sampling.

### NIT findings
**[NIT-1]** Previous NIT-2 (id types) is resolved by R3-NIT-2's explicit type statements (`runners.id` text UUID, `gitlab_runner_id` integer, `RunnerSummary.id: u64` is the GitLab id). No new nits.

### Verdict
PASS — no BLOCK findings; implementation may proceed.