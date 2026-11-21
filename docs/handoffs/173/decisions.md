## Phase 3 Architecture (A, up-front plan review) - 2026-11-21T08:48:27-08:00
- **Decided:** BLOCK, with 1 block and 7 warn findings (docs/handoffs/173/handoff-A.md).
  - The block is about the runner scripts. Both keep `run-single`'s own exit status out of `GITLAB_EXIT_REASONS`:
    - Tart `runner_script` turns 75 or 77 into 70 (`gitlab.rs:160-163`);
    - core `GITLAB_RUNNER_SCRIPT` turns 75, 77 or 69 into 1 (`provisioning.rs:198`).
  - The brief adds 78 to both tables but extends neither guard. So:
    - a `run-single` that itself exits 78 would list as `runner_missing` on Tart;
    - a Docker GitLab container exiting 78 would too, because the manager maps Docker exit codes through core's table (`gridops-manager/src/main.rs:1626-1639`).
  - The warns:
    - (2) the script test is a `scripts/test-*.sh` driver run in `tart-installer.yml` on real macOS bash 3.2, not in `gitlab_tests.rs`;
    - (3) no sourced-helper precedent exists, so whichever shape is chosen follows set rules;
    - (4) reuse the GitHub script's `sync` placement and its `vm_pid` stop-then-`wait`, with one `cleanup`;
    - (5) one `left_service` arm for both failing exits, and counting stays inside `left_service`;
    - (6) the fleet condition is already resolved: no Tart fleet runtime; do not map `ImageUnavailable`; a Docker follow-up for custom images without `gitlab-runner`;
    - (7) shared files with #171 and #172: `gitlab_lifecycle.rs`, `docs/gitlab-runners.md`, `CHANGELOG.md`;
    - (8) the `EX_CONFIG` doc, literal pins in place of a cross-crate dev-dependency, and the full doc/test sweep.
- **Assumed:** with no `survey.md`, the analogues the brief names (the `RegistrationRefused` arm, the two tables, and `prepare-tart-runner-image.sh`'s `sync`) serve as the Reuse map. "The trap cleans up both VMs" is read as: stop and keep the target on interruption, and delete it only when verification fails.
- **Hedged:** gitlab-runner probably never exits 78 itself. The block rests on the repo's own pinned and documented invariant, which already covers the equally unlikely 75 and 77, not on observed behaviour.
- **Evidence:** I read:
  - the brief and the brief gate result;
  - all three prep scripts and `scripts/test-install-tart-agent.sh`, `test-release-preflight.sh`, `ci.yml` and `tart-installer.yml`;
  - tart agent `gitlab.rs`, `gitlab_tests.rs:180-600` and `main.rs` (the table copy, the listing, the 78 exits, the parity and listing tests);
  - core `provisioning.rs` (the table, the Docker script and its guard, the pins) and `tests/gitlab_runner_script.rs`;
  - manager `main.rs` (`ManagedRunner`, `gitlab_exit_reason`) and `gitlab_runner_tests.rs`;
  - reconciler `gitlab_lifecycle.rs` in full, `main.rs:186-240` and `:895-930`, and `gitlab_lifecycle_tests.rs` (`assert_readable`, the refusal tests);
  - host-agent `runtime.rs` and `docker.rs` (the exit diagnostics and the inspect path), and `DiagnosticCode`;
  - `docs/macos-runners.md` and `docs/gitlab-runners.md`, the exit sections;
  - the #171 and #172 briefs on their branches, whose diffs touch only `docs/handoffs`.

  `git diff origin/master HEAD -- crates scripts src tests .github` is empty.
