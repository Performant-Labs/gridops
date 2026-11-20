## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
**[NV-1]** Claim that `failure_cleanup(true, true, …)` releases the capacity lease, marks the row failed and lets the GitLab arm drive `delete_runner` at the point where the Bitbucket arm registers — evidence: the actual `start_admitted_runner` / `failure_cleanup` source in `crates/gridops-reconciler/src/main.rs` (around the Bitbucket registration call and its error path).

**[NV-2]** Claim that a non-Bitbucket runner currently falls into the GitHub branch of `delete_runner` dispatch "at ~`main.rs:2811`" — evidence: the dispatch code at that line in `main.rs`.

**[NV-3]** Claim that `placement::store_bootstrap` stores `github_registration` material in `crates/gridops-reconciler/src/fleet.rs` and can store the new typed `GitlabRunnerMaterial` identically — evidence: `fleet.rs::register_and_start` and `crates/gridops-core/src/fleet/placement.rs::store_bootstrap`.

**[NV-4]** Claim that `ExternalId::try_from` exists and accepts the `u64` GitLab runner id D17 specifies — evidence: the `ExternalId` definition and its `TryFrom` impl in `gridops-core`.

**[NV-5]** Claim that `FakeGitLab` already has `fail_next` and a `calls()`/`FakeMethod` recording scheme that a new `list_runners` variant slots into without breaking existing fake tests — evidence: `crates/gridops-core/src/gitlab/fake.rs`.

**[NV-6]** Claim that migrations 0047 already contains `runners_gitlab_connection_idx` and `runners_gitlab_runner_id_idx` and the `runner_pool_gitlab_connections` / `runners.gitlab_connection_id` / `gitlab_runner_id` columns (D7, D16) — evidence: `migrations/0047_gitlab.sql`.

**[NV-7]** Claim that `ensure_target` already has GitLab arms (D6) while `ensure_target_identity_for_pool` is `today _ => false` (D11) — evidence: `crates/gridops-core/src/fleet/targets.rs` and `fleet/service.rs` at the cited sites.

**[NV-8]** Claim that `pool_docker_image` (fleet) and the manager-path `ProvisioningPlan.image` fallback currently fall back to `STOCK_DOCKER_IMAGE` for an empty `docker_image` — evidence: `fleet/targets.rs::pool_docker_image` and the `ProvisioningPlan` image resolution in `main.rs`.

**[NV-9]** Claim that the reconciler already has a retention/cleanup cadence the sweep can reuse — evidence: the reconciler main loop / schedule code in `main.rs`.

### WARN findings
**[W-1]** Sweep identifier ambiguity: the sweep keys liveness off the description marker (`gridops-runner:<uuid>` → `runners` row lookup), but D17 says it converts `RunnerSummary.id` (`u64`) via `ExternalId::try_from` and skips non-converting ids. It is unstated which identifier `delete_runner` requires and what the `ExternalId` conversion is for when deletion keys on the parsed uuid. Recommend the brief state explicitly: "delete_runner is called with the GitLab runner id converted to ExternalId; the marker uuid is used only for liveness lookup."

**[W-2]** `gitlab_runner_image(pool_docker_image: &str) -> String` is specified to "return the pool image" *and* "error with a readable start failure" when empty — a `String` return cannot error. Recommend fixing the spec to `-> Result<String, …>` (or "returns Option; caller maps None to the refusal") so the acceptance criterion "errors with a readable 'GitLab pool has no runner image' start failure" is implementable as written.

**[W-3]** "Token never in logs or row" (amended Done-when) is the only criterion asserted without a stated test mechanism. Recommend naming how it's checked (e.g. assert the token string appears in no captured log output and in no `runners` row column in the test), otherwise it risks being "verified" by eyeball.

**[W-4]** The two "Done when" sections coexist (issue text says tests "in the `start_tests.rs` style"; amendments say fixtures-only in `start_tests.rs` plus a new `gitlab_lifecycle_tests.rs`). The Clarifications preamble says amendments replace the issue text where they differ, so this is resolvable, but the brief should mark the original "Done when" as superseded to avoid a reviewer/judge reading the weaker original as the gate.

### NIT findings
**[NIT-1]** "A description that does not parse is never deleted" and "never touch a runner without the marker" are stated twice (D17 and Done-when); harmless duplication but worth collapsing.

**[NIT-2]** Sweep cadence "not every pass; e.g. reuse the existing retention/cleanup cadence" leaves the exact cadence to the implementer; fine, but a one-line "log at sweep start/end" would make the criterion "its errors are logged, never fail the pass" easier to observe in tests.

### Verdict
PASS — no BLOCK findings; implementation may proceed.