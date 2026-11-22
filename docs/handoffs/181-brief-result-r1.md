## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** The core causal claim: `owned_queued_jobs` (autoscaling.rs ~245) keeps GitHub jobs only via `KeyedJobOwner::github`, and its doc reads "GitLab jobs it owns are left out: every view built from this is keyed by GitHub job and repository ids." — settle by showing `crates/gridops-core/src/autoscaling.rs` around lines 240-250 (function body + doc comment) and the `KeyedJobOwner` definition.
- **[NV-2]** `provision`'s `NoDemand` guard at main.rs ~2440-2453 triggers on `active >= min_count` when `!behind` — settle by showing `crates/gridops-reconciler/src/main.rs` lines 2400-2460.
- **[NV-3]** `attribute_queued_jobs` (targets.rs ~1373) is built on `queued_job_assignments`, so GitLab jobs never reach `choose_platform` — settle by showing `crates/gridops-core/src/fleet/targets.rs` lines 1350-1500.
- **[NV-4]** The default value of `queue_scale_factor`. If the default (or a common config) is greater than 1, then with one pending GitLab job the desired count could be ≥2, and the required regression test's "places exactly one runner" assertion could be wrong for factor >1 pools. — settle by showing where `queue_scale_factor` is read/defaulted (autoscaling.rs / config) and what the test fixtures set.
- **[NV-5]** The Bitbucket finding: no Bitbucket candidates in `queued_job_owners`, `maybe_scale_down` keeps Bitbucket runners, and `targets::ensure_target` refuses Bitbucket targets — settle by showing `queued_job_owners` (~autoscaling.rs 637 area) and the refusal text in `targets::ensure_target`.
- **[NV-6]** The seam claim that #148's A-B2 (`docs/handoffs/148-brief.md:72-77`) says only "fleet placement is untouched" and the rest still holds — settle by showing that file's lines 72-77.
- **[NV-7]** `fleet::provision` checks for a GitLab connection first (fleet.rs ~163-170), which is the safety argument for a `None` spare runner meaning "GitHub org or GitLab runner" — settle by showing `crates/gridops-reconciler/src/fleet.rs` lines 155-180.
- **[NV-8]** Test-fixture references: `Fleet` fixture at gitlab_lifecycle_tests.rs ~1713-1800 with `min_count=1`, `autoscaling_enabled=0`; pending-row seeding at gitlab_poll_tests.rs ~190-196; the `#[cfg(test)] #[path = …] mod` wiring pattern for `autoscaling_gitlab_tests.rs` — settle by showing those file sections so the "extend, don't duplicate" instructions are actionable as written.
- **[NV-9]** `autoscaling_gitlab_tests.rs` lines 1-5 currently read "Only `assigned_queued_jobs` (room and scale) sees GitLab jobs" — settle by showing the file's header.

### WARN findings
- **[W-1]** The regression test pins "exactly one runner" for one pending job, but the scaling path multiplies demand by `queue_scale_factor`; the brief never states the factor the test environment uses. If the fixture must set the factor explicitly to 1 for the assertion to hold, say so in the test spec, otherwise the criterion is underspecified relative to scope item 2.
- **[W-2]** "For a GitHub-only pool every result stays byte-identical" is not directly testable as written (byte-identity of internal results). Recommend restating as observable criteria: same `choose_platform`/`platform_jobs` outputs on representative fixtures, which the listed existing test suites already encode — the brief already implies this, but the acceptance wording should point at the suites rather than "byte-identical".
- **[W-3]** Missing error path in the test plan: a fleet GitLab pool whose GitLab connection is removed/absent while pending GitLab jobs exist. Scope item 1's safety note implies `fleet::provision` bails, but no test covers "GitLab job owned, no GitLab connection → no placement / no panic in the new accessor".
- **[W-4]** No acceptance criterion for the doc-comment updates listed in scope item 1 (queued_job_owners, choose_platform, platform_jobs, module doc). Either add a checklist item or mark them as review-verifiable only; as written they are scope without a check.

### NIT findings
- **[NIT-1]** "Deploying to Mars" in Out of scope is noise; trim to the real operational exclusions (prod writes).
- **[NIT-2]** The migration fallback ("next free number after 0047") invites drift if another PR lands a migration first; better phrased as "next free number at implementation time".

### Verdict
PASS — no BLOCK findings; implementation may proceed, with the needs-verification items carried into the diff gate.