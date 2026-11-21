## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** The demand scan's premise that `runner_pools.labels` exists and holds the pool's configured labels (A-B4, used for tag matching and `run_untagged`) — the `runner_pools` schema (a `labels` column and its format: JSON array? comma list?) is not excerpted here. Evidence: the `runner_pools` table definition in the migrations on master, plus how `autoscaling.rs`'s GitHub path parses it today.
- **[NV-2]** A2-B1 models the conclusion bound on `confirm_jobs`/`unconfirmed_jobs` at `main.rs:2030-2220` — those functions are not excerpted; the claim that the proposed `updated_at`-before-request mechanism "works like `workflow_job_sync.checked_at`" cannot be checked. Evidence: those lines of `crates/gridops-reconciler/src/main.rs`.
- **[NV-3]** A-B1's claim that the reconcile interval (`reconcileIntervalSeconds`) is readable from a worker the way `run_passes` reads it — no excerpt shows where that setting lives or how it's accessed. Evidence: the `run_passes` scheduling code in `main.rs` and the settings/config reader.
- **[NV-4]** A-W2's claim that a GitLab connection's access token is unsealed "the same way Bitbucket connections' tokens are" via `runtime_secret` — the Bitbucket credentials loader and `runtime_secret` are not excerpted. Evidence: the Bitbucket token-unsealing call site in the reconciler.
- **[NV-5]** A-W1 cites `main.rs` near line 4825 and `start_tests.rs` near line 425 as `Reconciler { .. }` test literals that must gain a `gitlab` field — not excerpted; exact edit surface unverifiable from this prompt. Evidence: those regions.
- **[NV-6]** The exclusive-pool claim (A-B4) that #134's semantics make "treat an exclusive pool exactly like any other pool for GitLab" correct — migration 0046 is claimed "read in full" but not excerpted. Evidence: `migrations/0046_runner_pool_exclusive_labels.sql` and the exclusive-label enforcement code path.

### WARN findings
- **[W-1]** Internal contradiction on where `gitlab_credentials` lives: A-W2 says `gitlab_poll.rs` defines it; A2-W7 says it lives in `main.rs` "next to `runtime_secret`". Round 2 is binding, but A-W2's text was left unamended — an implementer reading top-down could build the A-W2 version and break the "#139 never depends on the poll module" rule. Recommendation: strike or annotate A-W2 as superseded.
- **[W-2]** A2-B1 asserts "Every writer sets `updated_at`", but round-3 D18 says `updated_at` moves "whenever anything changed; `Ignored` means no write". The precedence rule resolves this on paper, but the A2 Done-when criterion "a row whose `job` answer did not change is not asked about again" depends only on the re-check mark, so the A2-B1 sentence should be corrected to match D18, not left standing.
- **[W-3]** Test-setup tension in the duplicate-row Done-when: the dedup test needs a project connection and its parent group connection sharing a `base_url` (dedup keys on `base_url`), while FakeGitLab keys all state by `base_url` alone — the group poll will see the project's jobs too, and any `fail_next` on that URL hits both connections. The test is still constructible (the connections have different ids), but the brief should note the fake's shared-URL aliasing so the test isn't misread as isolation.
- **[W-4]** The Done-when still says a `NotFound` conclusion writes "`canceled` (or the other terminal status the implementation documents)", while round-3 D18 fixes it as `canceled`. Amended text should win; trim the parenthetical so the acceptance criterion is single-valued.
- **[W-5]** Edge case not covered: A2-W6 says rows of a project whose `pending_jobs` failed are not concluded "in that poll", but for a **group** connection a per-project skip interacts with the D18 duplicate rows — the same job may exist under a group-connection row (from the group poll) and a project-connection row; if only one poll failed, one row can be concluded while its duplicate is skipped, and the next poll's dedup then counts a job whose other row is terminal. Worth an explicit rule or a Done-when test.
- **[W-6]** No error path specified for `gitlab_credentials` returning `None` (connection/secret gone): should the poll skip silently, log, or clear the connection's backoff? One sentence would close it.

### NIT findings
- **[NIT-1]** A-B1's fallback interval constant `GITLAB_POLL_INTERVAL` (30 s) is conditional on an unverifiable readability judgement (see NV-3); name the primary mechanism explicitly in the Done-when so the test doesn't depend on the fallback.
- **[NIT-2]** The brief cites the D8 index as `gitlab_jobs_active_created_idx` while its `WHERE` clause covers five statuses — the name is contract-fixed; fine, but a reader may look for a "created"-only index.

### Verdict
PASS — no BLOCK findings; implementation may proceed.