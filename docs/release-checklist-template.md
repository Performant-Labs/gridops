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

## Prerequisites

Check these before step 1. Whoever cuts the release, a person or an agent,
needs all of them. With them in place an agent can run steps 1 to 23 itself;
what still needs a person is listed in [`docs/releasing.md`](https://github.com/Performant-Labs/gridops/blob/master/docs/releasing.md#running-a-release-with-an-agent).
Keep the real addresses, user names, machine names and the token out of this
issue: write the placeholders, and a result such as "all passed".

- [ ] P1. **GitHub rights on `Performant-Labs/gridops`**: push a tag, run workflows, create and edit releases, create, edit and close issues (the Write role does all of it)
  - `gh auth status` names the account, and `gh api repos/Performant-Labs/gridops --jq .permissions.push` prints `true`
  - A fine-grained token needs Contents, Actions and Issues read and write, plus Variables read (step 4); a classic token needs `repo`
  - If a ruleset protects `v*` tags, the account is on its bypass list
- [ ] P2. **A release operator token for `<deployment>`** with the scopes `fleet:read` and `fleet:operate` and a short expiry (one to three days); [how to create it, keep it and what it can do](https://github.com/Performant-Labs/gridops/blob/master/docs/fleet-automation.md#the-release-operator-token)
  - An administrator creates it under **Settings → API tokens**. The secret is shown once: put it in a secrets manager or the agent's secret store, never in the repository, an issue, a pull request or a chat
  - The preflight (P9) proves both scopes. The API does not let a token read its own expiry, so read it on the Settings page and pass it as `PREFLIGHT_TOKEN_EXPIRES_AT`
  - Revoke it in step 23
- [ ] P3. **A way to reach the deploy host**: a login on `<deploy host>` that can run `docker` (root, or a member of the `docker` group, which is equivalent to root), in the deployment's compose project directory with its `.env`, and that can read the data volume for the backup of step 14
  - `ssh <deploy host> docker compose ls` lists the project, and `docker compose -p <project> ps` in its directory shows `api`, `manager` and `reconciler`
  - `docker volume inspect <data volume>` finds the volume that holds the SQLite database
- [ ] P4. **Free disk on the deploy host**, for the new images and a database backup: at least 10 GB free where Docker keeps its data
  - `ssh <deploy host> 'df -h "$(docker info --format "{{.DockerRootDir}}")"'`, or the preflight's `disk` check with `PREFLIGHT_DEPLOY_HOST` and `PREFLIGHT_DATA_PATH`
- [ ] P5. **Root on each machine that runs the agent as a system service** (step 18), given as a narrow sudo rule that lets only the upgrade command run without a password. An agent cannot type a sudo password and must not look for one. Someone with root sets it up once per machine:
  - A root-owned wrapper, `/usr/local/sbin/gridops-agent-upgrade`, mode `0755`, that downloads `/install.sh` from the deployment and runs it. Write the deployment's real address in place of `<deployment>` on the machine (not here): it is fixed in the wrapper, so the caller cannot point it elsewhere. The wrapper refuses to run on a machine without the system service, where the installer would otherwise set up a second, per-user agent for root:

    ```sh
    #!/bin/sh
    # Upgrades the GridOps agent that runs as this machine's system service, from
    # the deployment that serves it. Owned by root, mode 0755, takes no arguments.
    set -eu
    PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
    case "$(systemctl show gridops-host-agent -p FragmentPath --value 2>/dev/null)" in
      /etc/systemd/system/* | /usr/lib/systemd/system/* | /lib/systemd/system/*) ;;
      *) echo "gridops-agent-upgrade: no gridops-host-agent system service on this machine" >&2; exit 1 ;;
    esac
    tmp="$(mktemp)"
    trap 'rm -f "$tmp"' EXIT
    curl -fsSL --retry 5 --connect-timeout 15 --max-time 300 -o "$tmp" https://<deployment>/install.sh
    GRIDOPS_NO_ENROLL=1 sh "$tmp"
    ```

  - A sudoers file that allows exactly that command, with no arguments (`""`), for the releaser's login, checked with `visudo -cf` before it is installed. The file name in `/etc/sudoers.d` must have no `.` in it, or sudo ignores it:

    ```sh
    sudo install -o root -g root -m 0755 gridops-agent-upgrade /usr/local/sbin/gridops-agent-upgrade
    printf '%s\n' '<releaser> ALL=(root) NOPASSWD: /usr/local/sbin/gridops-agent-upgrade ""' > gridops-agent-upgrade.sudoers
    sudo visudo -cf gridops-agent-upgrade.sudoers &&
      sudo install -o root -g root -m 0440 gridops-agent-upgrade.sudoers /etc/sudoers.d/gridops-agent-upgrade
    sudo visudo -c
    ```

  - Give the rule no `SETENV` and no `env_keep` for `GRIDOPS_*`: sudo then drops the caller's environment, so the installer always installs the version the deployment pins. Anyone who can change what `https://<deployment>/install.sh` serves can run code as root on these machines through this rule, exactly as with the manual `curl | sudo sh`
  - Check: `ssh <machine> sudo -n -l /usr/local/sbin/gridops-agent-upgrade` prints the path, and `sudo -n -l` shows it under `NOPASSWD:`. The preflight checks both for every machine in `PREFLIGHT_UPGRADE_HOSTS`
  - Machines with a per-user agent (step 19) need only an ssh login as the user the agent runs as
- [ ] P6. **Machines healthy**: every approved machine that is not retired has a fresh heartbeat, none is set aside for failing runner starts (the machine read's `startGate`), and none had failed placements in the last hour. Fix a machine, or pause it, before the dry run of step 9, which otherwise can land on it
  - The preflight's `heartbeats`, `start-gate` and `placement-failures` checks. A machine left out on purpose goes in `PREFLIGHT_IGNORE_HOSTS` and is named in step 20
- [ ] P7. **A macOS ARM64 machine ready for the macOS agent build.** Every release builds the macOS agent on a self-hosted macOS ARM64 runner (a Tart VM on the Mac), and the release is not published without it ([what the runner needs](https://github.com/Performant-Labs/gridops/blob/master/docs/macos-runners.md#what-a-release-needs-from-the-macos-runner)). In GridOps a macOS ARM64 machine is approved, `active`, has a fresh heartbeat and is not set aside for failing starts
  - The preflight's `macos-machine` check. Listing the Mac in `PREFLIGHT_IGNORE_HOSTS` does not excuse it
  - Only in an emergency, with the repository variable `RELEASE_MACOS` set to `false`: the release is **published without a macOS agent**, and the installer of this version cannot install or upgrade the agent on a Mac. Run the preflight with `PREFLIGHT_RELEASE_MACOS=false`, and record the opt-out and why in step 4
- [ ] P8. **Expect to share runners with CI.** Release and dry-run jobs run on the same pools as CI, so they can queue behind it for minutes. Start the dry run when `gh run list --repo Performant-Labs/gridops --status queued` is short, and do not cancel other people's CI to make room. A job that hangs on one machine is cancelled and re-run after that machine is paused
- [ ] P9. **Run the preflight** from a checkout of the repository, and fix every `FAIL` before step 1. It needs `curl` and `jq`, and `ssh` for the machine and deploy host checks

  ```sh
  read -r GRIDOPS_TOKEN && export GRIDOPS_TOKEN   # paste it; an agent loads it from its secret store
  GRIDOPS_URL=https://<deployment> \
  PREFLIGHT_TOKEN_EXPIRES_AT=<expiry from Settings, YYYY-MM-DDTHH:MM:SSZ in UTC> \
  PREFLIGHT_UPGRADE_HOSTS="<machine> <machine>" \
  PREFLIGHT_DEPLOY_HOST=<deploy host> PREFLIGHT_DATA_PATH=<Docker root directory> \
    sh scripts/release-preflight.sh
  # add PREFLIGHT_RELEASE_MACOS=false only with the RELEASE_MACOS=false opt-out of P7
  ```

  - One `PASS`, `FAIL` or `SKIP` line per check; it runs them all, then exits non-zero with the list of failures. `SKIP` means a check was not configured, not that it passed. The settings are described at the top of the script
  - Its output names machines and the deployment, so record only the summary here: `___`

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
  - Linux x64 agent, `install.sh` and `SHA256SUMS`: always
  - macOS agents (`gridops-host-agent-macos-arm64`, `-macos-x64`, `gridops-tart-agent-macos-arm64`): always, on the macOS runner of P7. The release is not published without them unless `RELEASE_MACOS` is exactly `false`, an emergency opt-out that publishes it **without a macOS agent**
  - Control plane images, `compose.release.yaml` and `env.example`: only when `RELEASE_IMAGES` is `true` (built on a macOS runner with Docker for now; unset, deployments build from source)
  - Linux arm64 agent: only when `RELEASE_LINUX_ARM64` is `true` (needs an `ARM64` runner in a pool)
  - This release builds (macOS agent / images / Linux arm64): `___`
  - If `RELEASE_MACOS` is `false`: why, and that this release has no macOS agent: `___`
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
  - A run started by hand on a branch builds what the tag will and publishes nothing: the **Publish release** job assembles the assets, lists them in the run summary and skips its last step. Every job that step 4 expects is green and none is skipped by mistake: **Agent (macOS)** and **macOS runner watchdog** ran (skipped only with `RELEASE_MACOS=false`), **Control plane images** ran only with `RELEASE_IMAGES`. Do not use the `skip_macos_agent` input here: `___`
  - If no macOS runner started the macOS job within 60 minutes, the watchdog cancelled the run and its summary says so; bring the runner online (P7) and run it again
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
  - A cancelled or failed job is re-run (`gh run rerun <run id> --failed`), not re-tagged. A run the macOS runner watchdog cancelled is re-run whole (`gh run rerun <run id>`) once the macOS runner is online
  - **Publish release** fails rather than publish without a build step 4 needs; its summary names the build
- [ ] 13. **Check what was published**
  - `gh release view vX.Y.Z --repo Performant-Labs/gridops --json assets,isDraft,isPrerelease` lists, and the release is not a draft:
    - always: `install.sh`, `SHA256SUMS`, `gridops-host-agent-linux-x64`
    - always, unless step 4 records the `RELEASE_MACOS=false` opt-out: `gridops-host-agent-macos-arm64`, `gridops-host-agent-macos-x64`, `gridops-tart-agent-macos-arm64`
    - with `RELEASE_IMAGES`: `compose.release.yaml`, `env.example`
    - with `RELEASE_LINUX_ARM64`: `gridops-host-agent-linux-arm64`
  - Download them into a clean directory (`gh release download vX.Y.Z --repo Performant-Labs/gridops --dir <directory>`) and run `sha256sum -c SHA256SUMS` there: every file prints `OK`, and `SHA256SUMS` has a line for each binary above, `gridops-host-agent-macos-arm64` included (`grep macos-arm64 SHA256SUMS`)
  - With `RELEASE_IMAGES`: `docker buildx imagetools inspect ghcr.io/performant-labs/gridops-api:X.Y.Z` (and `gridops-manager`, `gridops-reconciler`) shows `linux/amd64` and `linux/arm64`, and `:latest` points at it
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
  - With the sudo rule of P5: `ssh <machine> sudo -n /usr/local/sbin/gridops-agent-upgrade` on each, which an agent can run. Without it, a person runs `curl -fsSL https://<deployment>/install.sh | sudo sh` there. Either replaces the binary, keeps the old one as `gridops-host-agent.previous`, restarts the service and checks that it is active
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
  - Revoke the release operator token of P2 under **Settings → API tokens**
  - Link this checklist from the GitHub release (edit its notes)
  - A problem found after the release goes into the README's `## Known issues` (with an issue link) in a pull request, and is noted below. Put a fix in the next release's changelog
  - Tick every box above, then close this issue
