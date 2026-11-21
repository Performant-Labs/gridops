# GitLab runners

GridOps can run GitLab CI jobs as well as GitHub Actions jobs. You connect a
GitLab project or group, add the connection to a runner pool, and GridOps
creates a runner in GitLab for each runner it starts. Each runner takes one job
and is then deleted from GitLab. Linux jobs run in Docker, on the built-in
runner host or an enrolled machine. macOS jobs run in Tart virtual machines.

This page covers setup and how jobs reach a pool. The macOS image is in
[macOS runners](macos-runners.md#gitlab), and the Docker side of Linux runners
is in [the fleet agent](fleet-agent.md#gitlab-runners).

## What GridOps needs from GitLab

A connection is one GitLab project or group on one GitLab instance
(`https://gitlab.com` by default, or a self-managed URL). It holds one access
token: a personal, project or group access token.

**Scopes.** Give the token:

| Scope | Used for |
|---|---|
| `create_runner` | Creating each runner (`POST /user/runners`). |
| `manage_runner` | Deleting each runner when it leaves service, and the orphan sweep (`DELETE /runners/:id`). GitLab 17.1 or later. |
| `read_api` | Resolving the project or group, the pending-jobs poll, reading single jobs, listing the scope's runners for the sweep, and checking the token. |

`api` covers all three. GridOps checks all three (or `api`) when you save the
connection, when you replace its token and when you choose **Verify**, and
refuses a token that lacks one with "The GitLab access token is missing the
`<scope>` scope." If GitLab still refuses to delete a runner at runtime, the
pool's events show a **GitLab runner not deleted** warning naming the
connection, the GitLab runner and the reason, and the orphan sweep retries the
delete.

**Role.** The token's user needs one of these:

- the **Maintainer** role or higher on a project connection;
- the **Owner** role on a group connection;
- GitLab administrator rights.

GridOps checks the role with GitLab's members API when you save the
connection, when you replace its token, and when you choose **Verify**. A
project or group access token counts as its bot user, so give the token the
role directly.

The token is encrypted at rest in GridOps. No API response, log line or audit
event contains it. A rejected or revoked token is reported as "GitLab rejected
the access token.", a missing scope as "The GitLab access token is missing the
`<scope>` scope.", and a missing role names the role.

## Connect a project or group

Only a GridOps system administrator can manage GitLab connections.

1. Open **Settings → Connections**, choose **Add connection**, then **GitLab**.
2. Enter a name and the GitLab address. Leave the address as
   `https://gitlab.com` unless you run your own GitLab; then use its address,
   for example `https://git.example.com`. The address must be `http` or
   `https` with no query or fragment.
3. Choose **Project** or **Group**. A project connection creates project
   runners. A group connection creates group runners, which every project in
   the group and its subgroups can use.
4. Enter the path from the project's or group's URL, such as
   `my-group/my-project`, without a leading or trailing slash.
5. Paste the access token and save.

GridOps resolves the path, checks the token's scopes and role, and saves the
connection only if both pass. Each project or group can be connected once per
GitLab address. To replace a token, edit the connection; the new token is
checked against the same project or group before it is stored.

The same calls are available in the API under
`/api/v1/platform-connections/gitlab` (see the OpenAPI document in
`docs/openapi/gridops-v1.json`).

## The webhook

Saving a connection shows a webhook URL and a webhook secret once. The URL is
`<GRIDOPS_BASE_URL>/api/v1/webhooks/gitlab`.

1. In GitLab, open the project's or group's **Settings → Webhooks** and choose
   **Add new webhook**.
2. Paste the URL, and paste the secret into **Secret token**.
3. Under **Trigger**, tick **Job events** only.

GitLab sends the secret in the `X-Gitlab-Token` header. GridOps matches each
delivery to the connection for its project (or the group that contains it) by
comparing that header with each candidate's secret. A delivery that matches no
connection is answered with 401 and recorded on the **Webhooks** page as
rejected. Matched deliveries are listed there with a GitLab
badge and can be retried.

The secret is shown only when the connection is created or the secret is
rotated. If you lose it, choose **Rotate webhook secret** on the connection
and paste the new one into GitLab. The old secret stops working at once; the
URL does not change.

GitLab must be able to reach `GRIDOPS_BASE_URL`. gitlab.com cannot deliver to
a loopback or private address. **The webhook is optional.** The reconciler also
polls each connected project's pending jobs on the reconcile interval (the
`reconcileIntervalSeconds` setting, 30 seconds by default), and asks GitLab
about any recorded job it has not heard about for five minutes. Without a
webhook, jobs are still picked up, only later.

## Pools, labels and GitLab tags

To run GitLab jobs, create a runner pool with GitLab as its target and choose
one or more GitLab connections. A pool with GitLab targets can't also target
GitHub repositories, organizations or Bitbucket. GridOps starts the runners of
such a pool from the reconciler, not from the request that created it.

**Pool labels become the runner's GitLab tags.** Every pool has its own name
as a label, plus the labels you add (at most 20 in all). A GitLab tag can't
contain a comma and is at most 64 characters. GridOps adds no other tags: no
`self-hosted`, no OS and no architecture tag. If your jobs use
`tags: [macos]`, add a `macos` label to the macOS pool.

GridOps uses the same rule as GitLab to decide which pool a job needs. A
**tagged** job belongs to a pool when every one of the job's tags is one of
the pool's labels, compared case-insensitively. The pool may have more labels
than the job asks for.

**`run_untagged`.** A job with no tags belongs only to a pool with **Run
untagged jobs** on. It is off by default, and GitLab is told the same when the
runner is created. With it off, untagged jobs never scale the pool and its
runners never take them.

Only a job in GitLab's `pending` status counts as demand. When a pool's
autoscaling is on, a pending job that belongs to it scales it up, within its
`max_count`. A pool keeps `min_count` runners waiting. A waiting runner exits
after 600 seconds without a job, is deleted from GitLab, and is replaced, so
an idle pool with a `min_count` above zero creates a new GitLab runner every
ten minutes.

GitLab hands a pending job to whichever matching runner asks first. If the
project or group also has GitLab's instance runners enabled, they can take
untagged jobs and jobs whose tags they carry before a GridOps runner starts.
Turn instance runners off for the project or group if GridOps should run
those jobs.

## Runner lifecycle

For each runner it starts, the reconciler creates a runner in GitLab on one of
the pool's connections, with the pool's labels as tags, the pool's
`run_untagged`, and the description `gridops-runner:<deployment>:<runner id>`.
The runner's `glrt-` token goes only to the runner itself, never to the
database, an event or a log.

Each runner runs `gitlab-runner run-single` for at most one job:

- It exits after its job and is deleted from GitLab.
- It exits after 600 seconds with no job (`idle_expired`) and is deleted from
  GitLab.
- If GitLab refuses its token (`registration_refused`), it is deleted and the
  pool's provisioning failure count goes up, as for any failed start.

A delete that fails is logged and retried by the orphan sweep. When GitLab
refuses it (for example, the token lacks `manage_runner`), the runner's pool
also gets a **GitLab runner not deleted** warning in its events. The sweep's
own refused deletes warn every pool linked to the connection, at most once an
hour per pool, connection and reason. Once every
`gitlabRunnerSweepIntervalSeconds` (600 by default, between 60 and 3,600), the
sweep lists each connection's project or group runners and deletes every one
whose description carries this deployment's marker but whose runner is no
longer live in GridOps. It never touches runners GridOps didn't create.

## Linux runners need the Docker socket

A Linux GitLab runner is a container of the official `gitlab-runner` image
(`gitlab/gitlab-runner:ubuntu-v19.6.0` when the pool sets no image) using
GitLab's Docker executor. That executor starts each job's containers on the
host's Docker daemon, so the runner needs the host's Docker socket:

- **On the built-in runner host**, set `GRIDOPS_RUNNER_DOCKER_SOCKET` for the
  runner manager. Without it the manager refuses GitLab runners. A manager
  started with `compose.no-docker-socket.yaml` starts no runners at all.
- **On an enrolled machine**, the agent needs `GRIDOPS_AGENT_DOCKER_SOCKET`,
  the machine's grant must allow the Docker socket, and the pool's profile
  must share it. Without the profile's grant the agent refuses the runner with
  a configuration error.

A job with access to the Docker socket controls that host's Docker daemon.
Run GitLab pools only for projects you trust, on hosts that run nothing else
you need to protect. See
[the fleet agent](fleet-agent.md#gitlab-runners) for the details.

Each job runs in the image its `.gitlab-ci.yml` names with `image:`, or in
`alpine:3.24.4` when it names none.

## macOS runners

A GitLab pool whose provider is Tart runs each runner in a fresh macOS VM with
the shell executor. The shell executor ignores `image:`, so install the tools
the jobs need in the base image. Prepare the image with
`scripts/prepare-tart-gitlab-runner-image.sh`; see
[macOS runners](macos-runners.md#gitlab).

## Known limits

- **No instance runners.** GridOps creates project and group runners only. It
  can't create instance-wide (shared) runners, even with an administrator's
  token.
- **No job logs from GitLab.** GridOps doesn't fetch GitLab job logs. A job's
  log is in GitLab. A runner's own output, including `gitlab-runner`'s log, is
  in the runner's logs in GridOps.
- **GitLab jobs aren't in the queued-jobs views.** They scale pools, but the
  queued-jobs list and queue diagnosis show GitHub jobs only.
- **Poll errors are only logged.** A failed pending-jobs poll (for example
  after a token is revoked) appears in the reconciler log, not in the UI. Use
  **Verify** on the connection to see the error.
- **A GitHub App installation is still required.** Creating a pool with
  GitLab targets requires a GitHub installation you administer, even though
  the pool uses no GitHub repository.
