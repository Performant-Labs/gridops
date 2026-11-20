## Brief Review (Round 1)

### BLOCK findings
**[B-1]** Missing test for two explicitly required invariants — the scope demands "Never overwrite a newer webhook-written status with an older one (use `status_changed_at`), and never resurrect a terminal row," but the "Done when" poll-test list contains no test for either behavior (no webhook-vs-poll status conflict, no poll attempting to revive a terminal row). A stated behavior with no acceptance test is a defect in the brief's own terms. Add both cases to the poll tests (e.g. a `FakeGitLab` job whose reported status is older than a webhook-written one, and a terminal `gitlab_jobs` row GitLab still reports active).

### Needs-verification findings
**[NV-1]** "the Bitbucket 'never scale down' guard in `maybe_scale_down` (`crates/gridops-reconciler/src/main.rs`) … keys on `runner.ci_platform == "bitbucket"`" — a source excerpt of `maybe_scale_down` in `crates/gridops-reconciler/src/main.rs` showing the guard's actual predicate would settle whether a GitLab pool genuinely falls through it.
**[NV-2]** "`group_projects` exists on `GitLabApi`" (D4) — the `GitLabApi` trait definition from the #137 merge (file:line) would settle that the poll can call it with `&GitLabCredentials`.
**[NV-3]** "a group connection, list `group_projects` and poll each" scales a pool — nothing in the brief shows the pool-to-project linkage used to route a group-polled job to the right pool's candidates; the `runner_pool_gitlab_connections` / `scope_id` schema excerpt (from `0047_gitlab.sql`) would settle how a project job found via a group connection is matched to pools.
**[NV-4]** "`FakeGitLab` supports `with_job`, `with_group_project`, `fail_next`" — the `FakeGitLab` source would settle that these helpers exist and can express a 429 and a group project.

### WARN findings
**[W-1]** Two load-bearing design decisions are deferred to the implementer mid-brief: whether `running` counts as demand, and whether GitLab tags may match the GitHub catch-all labels (`self-hosted`, OS/arch). Both are labeled "decide and document," yet the acceptance tests (tag matching, exclusive pool) don't pin either outcome, so a test suite could pass with either choice and the brief's "Done when" wouldn't catch a wrong one. Pick both now in the brief and add a test for each.
**[W-2]** The rate-limit backoff has no specified persistence or default: "skip that connection until the retry-after (or a documented default) has elapsed" leaves both where the skip-until timestamp lives (memory across passes? in-process only?) and the default value unnamed. The test only checks "skips on the next pass." Specify the mechanism and the default constant.
**[W-3]** The `i64` job-key collision fix is stated as an open choice ("Choose a key … keep the existing public API stable … or update every caller"). Two acceptable implementations with materially different blast radius is weak bounding for a parallel-wave change; the brief should name the key scheme (e.g. a tagged/NamespacedId or composite) so parallel sessions (#138, #139, #140) can code against it.
**[W-4]** The `NotFound → canceled` conclusion path is distinguished in scope ("a `NotFound` concludes it as `canceled` (or another documented terminal status)") but the acceptance test ("a stale recorded job is concluded via `job`") doesn't distinguish which API outcome it exercises. Add an explicit NotFound test and pick the terminal status in the brief.
**[W-5]** No acceptance test that the demand scan reads only the D8 active status set (the stated reason the partial index is used). Even a cheap assertion that a `manual`/`scheduled` or terminal job produces no candidates would cover it.

### NIT findings
**[NIT-N]** None.

### Verdict
BLOCK — 1 blocking finding; must resolve before implementation starts.