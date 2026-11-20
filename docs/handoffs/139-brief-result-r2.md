## Brief Review (Round 1)

### BLOCK findings
None.

### Resolved findings
**[R-1]** Earlier [B-1]: "Missing acceptance test for D6 (GitLab pool-target sync)… the amended Done-when tests only 'admission…' — it never requires a test that a pool with GitLab connections actually receives `gitlab_project`/`gitlab_group` targets." Resolved: the round-1 amendments add "Done-when adds (B-1, D6 pool-target sync)" covering targets created per connection, linking to profiles, no GitLab target without connections, and idempotence.

**[R-2]** Earlier [B-2]: "Missing acceptance tests for the round-2 block 2 error paths… neither the `save_pool_fleet` `InvalidRequest` refusal nor the pre-`create_runner` manager-path refusal has a test." Resolved: "Done-when adds (B-2, round-2 block 2 refusals)" specifies (a) `save_pool_fleet` returning `FleetError::InvalidRequest` with the exact message and saving nothing, (b) the manager-path refusal asserted via no `FakeMethod::CreateRunner`, no manager POST, no row left `starting`, and (c) non-empty image used as given.

**[R-3]** Earlier [B-3]: "Missing acceptance test for the `requires_docker_socket` refusal… No Done-when entry covers either half." Resolved: "Done-when adds (B-3, `requires_docker_socket`)" covers both the pre-`create_runner` fleet refusal (no CreateRunner call, no Prepare, readable reason) and the once-per-runner counted Prepare refusal with GitLab cleanup.

Also resolved (WARN/NIT, no longer present as issues): **[R-4]** W-1 — the "Stub fixtures mirror the published contract (W-1)" amendment requires `GITLAB_EXIT_REASONS`, real field names, and real `FailureCode`/`DiagnosticCode` types. **[R-5]** W-2 — "Readable diagnostic / readable reason (W-2)" defines the criterion (non-empty, names the connection, names the outcome/cause, no `glrt-` token). **[R-6]** W-3 — "Observable behaviour with the placeholder client (W-3)" makes "not wedged" concrete (counted once, circuit opens at the threshold, no `starting` row, leases/placements released, pass returns without error and reconciles other pools). **[R-7]** NIT-1 — the amended Done-when now says "token never in the `runners` row or reconciler logs; the fleet path's sealed bootstrap material is the one place it is stored". **[R-8]** NIT-2 — D7 now reads "already shipped in 0047 by #137; #139 adds none".

### Needs-verification findings
**[NV-1]** The "Checked against master (NV-1 to NV-6)" section asserts the reviewer read and confirmed these facts, but the confirmations themselves are not shown in this prompt: `main.rs:2766` reset, `main.rs:2811` GitHub-branch dispatch, `fleet.rs:263-267` `finish` re-visit, `service.rs:805` `_ => false`, `Scope::target(connection_id)`, FakeGitLab surface, cascade behavior of `delete_pool`/0047 FKs, `retry_failed_start` INSERT…SELECT, `record_provision_failure`/`provision_failure_policy`/`set_pool_wait_reason`. Settled by showing the cited file:line excerpts themselves.
**[NV-2]** D5/host-agent-facts outcome shapes (Inspect `IdleExpired`/late `RegistrationRefused` as `Failed { ResourceUnavailable, retryable: false, diagnostic }`; Start refusal as `Failed { Configuration, … }`; completed = `ResourceUnavailable` with no diagnostic) describe unmerged #147 behavior — settled when that PR's diff exists; the W-1 fixture rule mitigates drift.
**[NV-3]** That #140's webhook sets the manager-path busy status the round-3 reset reads (~`main.rs:807`) — settled by #140's diff when it exists.
**[NV-4]** That #146's client and #148's `gitlab_lifecycle` field will actually match the assumed shapes (same `gitlab` field name/type; `list_runners` on `GitLabClient` with the D17 signature) — settled by those PRs' diffs; the brief's rebase rule mitigates but cannot guarantee it.

### WARN findings
**[W-1]** The round-3 manager-path reset ("may also reset when the status loop sees the GitLab runner busy (set by #140's webhook)") depends on #140 landing; if it does not, a working manager-path GitLab pool only resets on exit. The brief accepts the analogous fleet-path gap but does not state the manager-path consequence if #140 slips. Recommend recording that dependency in Done-when or accepting the exit-only reset explicitly.
**[W-2]** The placeholder-client Done-when (W-3) says "every GitLab start attempt fails at `create_runner` and counts once" while round 3 ties counting to the guarded UPDATE — with the circuit open after 3 attempts, later attempts are presumably not even started; the wording "every attempt counts" is slightly loose and could be read as requiring attempts past circuit-open. Recommend wording: "each attempt that reaches `create_runner` before the circuit opens counts exactly once."

### NIT findings
**[NIT-1]** `runner_pools.labels` is cited from `0001_initial.sql` in the checked-against-master list but no excerpt is shown; if labels were added later, the sweep/tags code is unaffected but the NV list is slightly over-broad.

### Verdict
PASS — no BLOCK findings; implementation may proceed.