# Release checklist (template)

**This is the reusable template.** Copy its content into a new GitHub issue
titled `Release checklist: vX.Y.Z` each time a release is cut, fill in and check
off the boxes on that copy, never here. The copy command is in [`docs/releasing.md`](https://github.com/Performant-Labs/gridops/blob/master/docs/releasing.md#release-checklist);
the reasoning behind the steps is in the same file.

Placeholders:

- `X.Y.Z` is the version being released, and `vX.Y.Z` its tag. The copy
  command writes the real version wherever the template says so; do not type it
  by hand.
- `<previous version>` (the last released version, without the `v`),
  `<deployment>`, `<merge commit sha>` and the other `<angle brackets>` are
  filled in by hand when you reach that step.
- `___` is a blank for a result: a link, a number, a command's output.

This repository is public: write no private hostnames, internal URLs or machine
names into the issue. Say `<deployment>` and keep the real address elsewhere.

## Pre-flight

- [ ] 1. **Identify the release commit** (the tip of `master` once the version bump is merged)
  - Commit SHA: `___`
- [ ] 2. **Confirm CI is green** on that exact commit
  - Both jobs of `ci.yml` (Web, and Rust) passed: `gh run list --repo Performant-Labs/gridops --branch master --workflow ci.yml --limit 3`
  - Link the specific run, not "CI passes in general": `___`
- [ ] 3. **No ready work left unmerged**
  - `gh pr list --repo Performant-Labs/gridops --state open`
  - Anything open that belongs in this release is merged, or named in step 7 as left out
- [ ] 4. **Decide what the release builds** from the repository variables (`Settings → Secrets and variables → Actions → Variables`, or `gh variable list --repo Performant-Labs/gridops`)
  - Linux x64 agent and `install.sh`: always
  - Linux arm64 agent: only when `RELEASE_LINUX_ARM64` is `true` (needs an `ARM64` runner in a pool)
  - macOS agents, control plane images and `compose.release.yaml`: only when `RELEASE_MACOS` is `true` (needs the self-hosted Mac runner)
  - This release builds (Linux arm64 / macOS): `___`
  - Windows is not a target; nothing is built for it

## Version

- [ ] 5. **Bump the version to `X.Y.Z`**
  - `Cargo.toml`: `[workspace.package] version = "X.Y.Z"`
  - `Cargo.lock`: refresh with `cargo check` and commit the result (the release builds with `--locked`)
  - Version examples in `README.md` and `docs/` still read as examples, not as the current version: `git grep -n '<previous version>' -- Cargo.toml Cargo.lock README.md docs`; what is left must be history or an example
  - In the same pull request, add the release's section to `CHANGELOG.md` (step 7) and review the README's `## Known issues` (step 8). The release commit has to contain both, because the tag points at it
  - Open the pull request, wait for CI, merge it to `master`. The merge commit is the release commit of step 1
- [ ] 6. **The version and the build number are what you expect**
  - `cargo metadata --no-deps --format-version 1 | grep -o '"version":"X.Y.Z"' | head -1` prints a match
  - The build number is the commit count of the release commit (`scripts/build-info.sh` in a full clone). It is never put in the tag, and the version is not bumped per build ([`docs/releasing.md`](https://github.com/Performant-Labs/gridops/blob/master/docs/releasing.md#version-and-build-number))
  - Release commit's build number: `___`

## Release notes and docs

- [ ] 7. **Write the changelog and review what the release will say**
  - `CHANGELOG.md` has a `## X.Y.Z - <date of the tag>` section, newest first, written from the merged pull requests (`git log --oneline --merges v<previous version>..<merge commit sha>`) under **Upgrading** (what an operator must do or will notice: upgrade commands, removed things, changed defaults), **Added**, **Changed** and **Fixed**. One line per change, with its pull request number. Only what was checked: do not describe behaviour from a title alone
  - Changelog section is in the release commit: `git show <merge commit sha>:CHANGELOG.md | head -40`. Result: `___`
  - The workflow publishes GitHub's generated notes (`generate_release_notes: true`): the merged pull requests since the previous tag. Read that list against the changelog; they must not disagree
  - Anything user-visible that needs a sentence (a changed setting, a removed flag, a migration, an agent that must be upgraded): `___`
  - Left out on purpose: `___`
- [ ] 8. **Known issues and docs match this release**
  - README `## Known issues`: every entry is still true for this release, fixed ones are removed, new user-visible problems are added, each with a link to its issue (file one if there is none), and the "Last reviewed for" line names X.Y.Z. This is part of the pull request in step 5
  - Known issues now: `___`
  - `docs/fleet-agent.md`, `docs/releasing.md` and `README.md` describe what the release does (installer, upgrade path, settings, compose file names)
  - Skip explicitly ("nothing changed") rather than leaving the box unchecked: `___`

## Dry run

- [ ] 9. **Run the release workflow by hand on the release commit, before tagging**
  - `gh workflow run release.yml --repo Performant-Labs/gridops --ref master`, then `gh run watch` on it (find the id with `gh run list --repo Performant-Labs/gridops --workflow release.yml --limit 1`)
  - A run started by hand builds everything and publishes nothing. Every job that step 4 expects is green and none is skipped by mistake: `___`
  - The dry run is on the release commit, not an older `master`. Run link: `___`
- [ ] 10. **Smoke test the fleet**
  - Run `fleet-smoke.yml` from the Actions tab, or `gh workflow run fleet-smoke.yml --repo Performant-Labs/gridops`, and confirm it runs a job end to end on a fleet machine
  - Run link: `___`

## Tag and publish

> **Order matters.** Publish the release before deploying the new version. A
> deployment serves `/install.sh` pinned to its own version, so one running
> X.Y.Z sends machines to the `vX.Y.Z` release; until it exists, installs fail.

- [ ] 11. **Tag the release commit and push the tag**
  - `git tag vX.Y.Z <merge commit sha>` (the exact commit of step 1, never "whatever HEAD is")
  - `git push origin vX.Y.Z`
  - `release.yml` refuses a tag that does not match the workspace version. A refusal means step 5 was incomplete: fix it on `master` and delete the tag (`git push origin :refs/tags/vX.Y.Z`, then `git tag -d vX.Y.Z`) before tagging again
- [ ] 12. **The release workflow publishes**
  - `gh run list --repo Performant-Labs/gridops --workflow release.yml --limit 1` shows the run for the tag, and it finishes green. Run link: `___`
  - A cancelled or failed job is re-run (`gh run rerun <run id> --failed`), not re-tagged
- [ ] 13. **Check what was published**
  - `gh release view vX.Y.Z --repo Performant-Labs/gridops --json assets,isDraft,isPrerelease` lists `install.sh`, `SHA256SUMS`, `gridops-host-agent-linux-x64`, plus what step 4 expects (`-linux-arm64`; `-macos-arm64`, `-macos-x64`, `gridops-tart-agent-macos-arm64`, `compose.release.yaml`, `env.example`), and the release is not a draft
  - Download them into a clean directory (`gh release download vX.Y.Z --repo Performant-Labs/gridops --dir <directory>`) and run `sha256sum -c SHA256SUMS` there: every file prints `OK`
  - With `RELEASE_MACOS`: `docker buildx imagetools inspect ghcr.io/performant-labs/gridops-api:X.Y.Z` (and `gridops-manager`, `gridops-reconciler`) shows `linux/amd64` and `linux/arm64`, and `:latest` points at it
  - The notes render correctly on the release page
  - Result: `___`

## Deploy the control plane

Only after step 13. Do this per deployment, and name it as `<deployment>`.

- [ ] 14. **Back up the database first**
  - A backup from Settings, or a consistent copy of the SQLite database inside the data volume, as the deployment normally does it. Verify it: `sqlite3 <backup file> 'PRAGMA integrity_check;'` prints `ok` ([`docs/disaster-recovery.md`](https://github.com/Performant-Labs/gridops/blob/master/docs/disaster-recovery.md))
  - Backup file and where it is kept (no path to a private host): `___`
- [ ] 15. **Keep a rollback image tag** for each service that changes
  - Note the running images (`docker compose images`) and tag each as `<image>:rollback-<previous version>` before the new ones replace them. Previous version running now: `___`
- [ ] 16. **Deploy only the changed services**
  - From images: pull the release's `compose.release.yaml` and `env.example`, keep the deployment's own `.env`, then `docker compose -f compose.release.yaml up -d --no-deps api manager reconciler`
  - From source: `eval "$(scripts/build-info.sh --export)"`, `docker compose build api manager reconciler`, then `docker compose up -d --no-deps api manager reconciler`
  - Recreate only the services this release changed; `--no-deps` leaves the rest alone
- [ ] 17. **Check health and migrations**
  - `curl -fsS https://<deployment>/api/health` returns `version` `X.Y.Z`, the expected `buildNumber` and `commit`
  - The API log from startup shows the migrations applied without error: `docker compose logs api`
  - The web UI's top bar reads `vX.Y.Z · build <number> · <commit>`
  - `curl -fsS https://<deployment>/install.sh` is pinned to `X.Y.Z`
  - Result: `___`

## Upgrade the host agents

- [ ] 18. **Linux machines that run the agent as a system service**
  - `curl -fsSL https://<deployment>/install.sh | sudo sh` on each. It replaces the binary, keeps the old one as `gridops-host-agent.previous`, restarts the service and checks that it is active
  - If the service does not come back, the installer prints the rename that restores the previous binary; use it, and record it in this issue
- [ ] 19. **Machines with a per-user agent** (macOS LaunchAgent, Linux systemd user unit)
  - Run the same installer without `sudo`: `curl -fsSL https://<deployment>/install.sh | sh`. It upgrades in place and keeps the settings
- [ ] 20. **Check the fleet**
  - The Machines pages show the new agent version, `X.Y.Z+build.<number>`, for every upgraded machine, and each machine is connected
  - Machines left on the old version on purpose, and why: `___`

## Post-release

- [ ] 21. **Smoke test the deployed release**
  - Run a job end to end on a fleet machine of the deployment (step 10's workflow again, now against the upgraded fleet). Run link: `___`
- [ ] 22. **Know the way back before you need it**
  - Control plane: `docker compose up -d --no-deps <service>` with the `rollback-<previous version>` images of step 15. The database was backed up in step 14; restore it only if the new version migrated it, following [`docs/disaster-recovery.md`](https://github.com/Performant-Labs/gridops/blob/master/docs/disaster-recovery.md)
  - Agent: rename `gridops-host-agent.previous` back over the binary and restart the service
  - Did anything have to be rolled back? `___`
- [ ] 23. **Close out**
  - Close the issues this release fixed, and the milestone if one was used
  - Link this checklist from the GitHub release (edit its notes)
  - A problem found after the release goes into the README's `## Known issues` (with an issue link) in a pull request, and is noted below. Put a fix in the next release's changelog
  - Tick every box above, then close this issue
