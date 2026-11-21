## Brief Review (Round 1)

### BLOCK findings
None.

### Resolved findings
**[R-1]** "Missing test for two explicitly required invariants — the scope demands 'Never overwrite a newer webhook-written status with an older one (use `status_changed_at`), and never resurrect a terminal row,' but the 'Done when' poll-test list contains no test for either behavior." — resolved: "Done when" now specifies both tests in full, including seeding mechanics ("No older status over a newer webhook status" and "No resurrection of a terminal row", with candidate-absence assertions).

### Needs-verification findings
**[NV-1]** The excerpted `maybe_scale_down` (main.rs:1396-1411) is consistent with the brief's claim, but it ends at line 1419 and the idle-timer behavior after `assigned_queued_jobs` is not shown — a fuller excerpt would settle that an idle GitLab pool with zero queued jobs actually reaches the scale-down write.
**[NV-2]** The brief asserts the access token is unsealed from `access_token_key` "the same way Bitbucket connections' tokens are" — a `main.rs`/core excerpt of the Bitbucket unsealing path would settle the mechanism exists and is reusable for `gitlab_connections.access_token_key`.
**[NV-3]** The brief asserts a scale-down "candidate filter" excludes nothing GitLab-specific; the excerpt at main.rs:1384 shows `runner.ci_platform != "bitbucket"` in the candidate filter — an excerpt of the surrounding filter (lines ~1370-1384) would settle GitLab runners are scale-down candidates at all.

### WARN findings
**[W-1]** Rate-limit backoff remains under-specified: "until the retry-after (or a documented default) has elapsed" leaves both the storage mechanism (in-memory across passes? per-restart?) and the default constant unnamed; the test only proves "skips on the next pass." Name the mechanism and the default so the test can be deterministic under `retry_after: None`.
**[W-2]** The job-key collision fix is still an open choice ("Choose a key … or update every caller in the same change"), which is weak bounding given six parallel sessions touching `gridops-api`, the reconciler and webhooks; #138/#139/#140 sessions cannot code against an unnamed key type. The brief should name the key scheme.

### NIT findings
**[NIT-1]** "Never overwrite a newer webhook-written status" is tested via a seeded `status_changed_at` after the pass's start, because `FakeGitLab` carries no timestamps; the test is honest about this workaround but it pins the implementation to comparing against the pass's start time rather than the observation time of that job — worth one sentence of tolerance in the criterion.

### Verdict
PASS — no BLOCK findings; implementation may proceed.