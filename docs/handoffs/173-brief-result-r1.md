## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** Claim that `left_service` does nothing for `Exit::Unknown` and that `RegistrationRefused` writes a `failure_reason`, an error event, and calls `count_failure` — evidence: `crates/gridops-reconciler/src/gitlab_lifecycle.rs` (the `left_service` arm and the `RegistrationRefused` handling).
- **[NV-2]** Claim that exit 78 is currently unmapped and produced by `runner_script` in `crates/gridops-tart-agent/src/gitlab.rs`, and that `GITLAB_EXIT_REASONS` in both `crates/gridops-core/src/provisioning.rs` and `crates/gridops-tart-agent/src/main.rs` contains only 0/75/77 — evidence: the relevant table/function bodies in those files.
- **[NV-3]** Claim that `sync` inside the guest actually fixes the lost write and that `prepare-tart-runner-image.sh` (GitHub) already runs it — evidence: `scripts/prepare-tart-runner-image.sh` guest-script section; the live-rerun log from issue #143 if retained.
- **[NV-4]** Claim that `crates/gridops-manager` relays `exitReason` unchanged and needs no change beyond tests — evidence: the manager's runner-listing re-parse code and its test.
- **[NV-5]** Claim that no fleet runtime can observe exit 78 (Docker runner image always ships `gitlab-runner`) — evidence: the fleet path in `gitlab_lifecycle.rs` (`Exit::from_inspect`, `fleet_start_failed`) and the Docker runner image definition.
- **[NV-6]** "The same event and runner-row paths; match what the pool page and API surface for `registration_refused`" — evidence: the pool API response shape and pool-page rendering of `registration_refused`, so the matching criterion is checkable.

### WARN findings
- **[W-1]** Deleting the target image on verification failure is destructive and does not distinguish "the image is genuinely broken" from "the verification clone booted flakily / guest agent timed out". A transient clone-boot failure destroys the (possibly long) image build. Recommend: only delete the target when the clone boots and the guest agent responds but the binary check fails; on clone-boot failure, exit non-zero but keep the target.
- **[W-2]** The criterion "Make the reason as visible on the pool as `registration_refused`'s is" is relative to unspecified existing behavior; if `registration_refused` turns out to be barely visible, the new reason inherits that. Recommend restating as concrete assertions (error system event present; `failure_reason` populated on the runner row) rather than "match X".
- **[W-3]** The shared-helper decision ("if that stays simple") leaves two acceptable shapes; the Done-when tests must pass under either. Acceptable, but the handoff should state which shape was chosen so the diff gate can check for divergence between the two scripts.
- **[W-4]** "counted once across repeated reconcile passes" is testable, but the brief doesn't say which idempotency mechanism is expected (reconcile-state key, seen-runner set, etc.); if `count_failure` is naturally idempotent per runner lifetime, fine — otherwise the implementer must pick one. Flag for the diff gate.

### NIT findings
- **[NIT-1]** "The test runs in CI … or a script test wired into `ci.yml`/`tart-installer.yml`" leaves the CI wiring point ambiguous; pick one so a missing-CI-test failure is detectable at the diff gate.
- **[NIT-2]** The changelog/docs done-when items are not machine-checkable; fine as prose, but the diff gate should grep the named files.

### Verdict
PASS — no BLOCK findings; implementation may proceed.