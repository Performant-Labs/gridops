## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** `owned_queued_jobs` keeps only GitHub jobs via `KeyedJobOwner::github`, and its doc reads "GitLab jobs it owns are left out…" — the actual source of `crates/gridops-core/src/autoscaling.rs` around lines 245 and 920. If that doc/behavior is not as quoted, the root cause itself changes.

**[NV-2]** `provision` returns `ProvisionAttempt::NoDemand` when `!behind && active >= min_count` (`crates/gridops-reconciler/src/main.rs` ~2440-2453), and `choose_platform` returns `(profile, behind)` (`crates/gridops-core/src/fleet/targets.rs` ~1466). The exact code at those lines settles the mechanism; not shown here.

**[NV-3]** GitLab pending jobs do raise `desired_count` through `assigned_queued_jobs` and are owned via `queued_job_owners` + `gitlab_job_candidates` (~line 637). If GitLab jobs are not actually owned/counted, the "same ownership, one source of truth" scope item is under-specified.

**[NV-4]** The deficit logic uses `is_none_or` treating `target_repository_id == NULL` GitLab runners as covering any job. `Option::is_none_or` was only recently stabilized in Rust; the workspace toolchain version (`rust-toolchain`/CI image) must support it — evidence: toolchain file plus the existing call site compiling.

**[NV-5]** `maybe_scale_up` already sets desired count capped at `max_count` (scope item 2 asserts this). Evidence: the `maybe_scale_up` source in `crates/gridops-core/src/autoscaling.rs`.

**[NV-6]** `targets::ensure_target` refuses Bitbucket targets, and no Bitbucket signal exists anywhere in the queued-job pipeline. Evidence: `crates/gridops-core/src/fleet/targets.rs` `ensure_target` body and a search for Bitbucket references in `autoscaling.rs`. The brief's "do not change Bitbucket" scope carve-out depends on this.

### WARN findings
**[W-1]** The regression test asserts "a second pass with that runner starting (idle, covering the job) places no second runner", but the "up to max" test requires several pending jobs to keep placing runners. Whether one idle runner covers one pending job or only "the" job depends on the deficit arithmetic not shown here. When writing the test, pin down the exact rule (idle runner absorbs N pending jobs? or 1:1?) so the two tests can't both pass with a wrong implementation that, e.g., never places a second runner for 2 pending jobs plus 1 starting runner.

**[W-2]** Scope item 1 says the fix must not change `NoDemand`, and item 3 demands byte-identical GitHub results, but attribution data structures change (separate source tag or `Option` repository). If `platform_jobs`/`repository_for_platform` signatures must change to express `Option`, "identical results" should be read as behavioral, not signature-level — the brief should say which, or the diff gate will argue about it.

### NIT findings
**[NIT-1]** Prod narrative says desired capacity rose to 2 with 3 pending jobs and max 2 — worth a one-line note in the test section that desired is capped at `max_count`, so "one runner per pass up to 2" is the expected observable, to avoid someone "fixing" the pass count to 2 per pass.

### Verdict
PASS — no BLOCK findings; implementation may proceed.