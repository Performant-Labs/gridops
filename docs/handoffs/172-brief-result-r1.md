## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** The brief asserts `validate_pool_configuration` returns "Docker image (dockerImage) is required for a pool that uses Linux · Docker." for Docker+empty image, and ends with the runner-group rule — the actual source text of `crates/gridops-core/src/models.rs` (~443-457) is not shown here. Evidence: excerpt of that function.
- **[NV-2]** The claim that `runner_group_id` is currently required/non-defaulted on `CreateRunnerPool`/`UpdateRunnerPool` and that an absent field yields 422 (serde) vs 0-val 400 — evidence: the struct definitions and the serde version's deny-unknown/missing-field behavior, plus a live API response for a payload without `runnerGroupId`.
- **[NV-3]** The claim that the new-pool form prefills `options.defaults.dockerImage` (the GitHub image) and that the Docker image `<Input>` is `required` in both form files at the cited lines — evidence: excerpts of `runner-pools.new.tsx` ~231 and `runner-pools.$poolId.settings.tsx` ~172, and where `options.defaults` is populated.
- **[NV-4]** That `docker_runner_image("", gitlab = true)` resolves to `GITLAB_RUNNER_IMAGE` and that `GITLAB_RUNNER_IMAGE` is `gitlab/gitlab-runner:ubuntu-v19.6.0` in `provisioning.rs` — evidence: excerpt of `fleet/targets.rs` ~503 and the constant definition.
- **[NV-5]** That the update handler's `input.validate()` is at `crates/gridops-api/src/resources.rs` ~1603 and that `UpdateRunnerPool::validate` truly has no scope field to branch on — evidence: excerpt of the handler and struct.

### WARN findings
- **[W-1]** The create-side runner-group criterion contains an escape hatch: "confirm the current behaviour for an absent field and keep GitHub's outcome identical, or document the difference if today it is a 422 from serde." This leaves GitHub's absent-field outcome unpinned (400 vs 422), and the Done-when section does not test it. Recommendation: pin the GitHub absent-`runnerGroupId` outcome (status code and body) explicitly in Done-when after confirming current behavior, so the "GitHub and Bitbucket pools validate exactly as before" constraint is testable.
- **[W-2]** Missing edge case: an update that changes `provider` to Docker on a GitLab pool, or (if scope is updatable) changes scope between GitHub and GitLab — which rule set applies mid-update is only implied ("the stored scope"). Recommendation: state explicitly that validation uses the stored scope/provider and add a test if scope/provider mutation is possible via update.
- **[W-3]** Frontend criterion "Switching the selection back to GitHub/Bitbucket… without clobbering an image the user typed" has no corresponding Done-when Vitest case. Recommendation: add a test asserting the user-typed image survives a GitLab→GitHub switch.
- **[W-4]** The headless walkthrough requires "a stubbed or seeded GitLab connection," which is out of scope of the listed files and may collide with the parallel #171/#174 work on `gitlab.tsx`. Recommendation: name the stub mechanism (mock, seeded fixture) so it doesn't require touching `src/features/platform-connections/gitlab.tsx` fields/text owned by the other sessions.

### NIT findings
- **[NIT-1]** "An update of that GitLab pool with an empty image and no runner group succeeds" — "no runner group" is ambiguous between absent and `runnerGroupId: 0`; the create tests cover both, so say which the update test sends (ideally both).
- **[NIT-2]** "The GitLab descriptor's `toCreatePayload` stops sending `runnerGroupId: 1` (or keep it only if removal breaks something; say which in the handoff)" — decide now; an either/or deliverable weakens Done-when. If kept, note why in the handoff regardless.

### Verdict
PASS — no BLOCK findings; implementation may proceed.