## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** That `docker_runner_image` in `crates/gridops-core/src/fleet/targets.rs` (~503) currently returns `GITLAB_RUNNER_IMAGE` for empty image when `gitlab` — excerpt of `fleet/targets.rs:495-510` and `provisioning.rs`'s `GITLAB_RUNNER_IMAGE` definition would settle it.
- **[NV-2]** That `validate_pool_configuration` in `crates/gridops-core/src/models.rs` (~443-457) enforces the Docker-image-required and runner-group-positive rules exactly as quoted — excerpt of `models.rs:430-470`.
- **[NV-3]** That `runner_group_id` is currently non-defaulted on `CreateRunnerPool`/`UpdateRunnerPool`, so omitting it is a serde 422 today — excerpt of the struct definitions in `models.rs`.
- **[NV-4]** That the update handler currently calls `input.validate()` at `resources.rs:1603` before `pool_access`, and the store-1 normalisation exists at ~1737-1741 and ~2670-2674 — excerpt of those `resources.rs` regions.
- **[NV-5]** That `validateProvider` (`schemas.ts` ~90), the `required` inputs in `runner-pools.new.tsx` (~231) / `runner-pools.$poolId.settings.tsx` (~172), and the `options.defaults.dockerImage` prefill behave as described — excerpts of those lines.
- **[NV-6]** That `schema.ts`'s import graph (`gitlab.tsx`/`bitbucket.tsx` import from it) would actually create the claimed cycle if the registry were imported — excerpt of the import blocks of `schemas.ts`, `gitlab.tsx`, `bitbucket.tsx`.
- **[NV-7]** That the fleet execution editor's GitHub fallback is `ghcr.io/actions/actions-runner:latest` as the "starts from" claim implies — excerpt of `fleet-profile-section.tsx` / the settings route's `dockerImage` prop passing.

### WARN findings
- **[W-1]** The `UpdateRunnerPool::validate(gitlab: bool)` signature change makes validation depend on the *stored* scope while the update body could in principle change scope; the brief assumes scope is immutable on update. Recommend stating explicitly that scope cannot be changed via update (or pinning that behaviour), since the `gitlab` flag otherwise validates against a scope the row may no longer have.
- **[W-2]** Done-when says "a GitHub create body with no `runnerGroupId` returns the 400 … for an `organization` pool" only; the parallel case for a `repository`-scoped GitHub pool (also subject to the runner-group rule per the normalisation comment `repository|gitlab` store 1) is untested. Add a pin for repository scope too, or state why organization covers it.
- **[W-3]** The 404-precedence acceptance criterion is one-directional ("invalid body to a missing pool returns 404"); the mirror case — a *valid* GitLab update body to a GitLab pool still returning success — is covered, but a GitLab update body sent to a *GitHub* pool (or vice versa) is unspecified. Recommend a criterion for cross-scope validation precedence.

### NIT findings
- **[NIT-1]** The brief says the vitest reads `provisioning.rs` via `readFileSync` to pin `GITLAB_RUNNER_IMAGE`; this couples a frontend test to Rust source layout by regex — brittle if the constant's formatting changes (e.g. `const GITLAB_RUNNER_IMAGE: &str = "…"` split across lines). Worth noting the match strategy.
- **[NIT-2]** "The headless UI walkthrough … (with a stubbed or seeded GitLab connection)" leaves the stubbing mechanism open; name the fixture/stub approach so the criterion is unambiguous at the diff gate.

### Verdict
PASS — no BLOCK findings; implementation may proceed.