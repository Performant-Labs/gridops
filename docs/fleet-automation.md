# Bot-driven fleet automation

How a script, CI job or agent operates the fleet and its runner pools with a
scoped API token — no browser session, no direct database writes. The
[API contract](fleet-api.md) lists every route
([Who may call what](fleet-api.md#who-may-call-what) says which take a token);
this page is the recipe.

## Create a token

An administrator creates a token in the web app under
**Settings → API tokens** (in the Account group of the settings sidebar, or
"Manage API tokens" in the command menu). The form takes a name, one or more
scopes and a lifetime in days (default 90, at most 365). The plaintext
`gat_...` is then shown exactly once, in a dialog with a copy button that
closes only when you confirm you have stored it; the page forgets it as soon
as the dialog closes, and only its SHA-256 verifier is stored on the server.
If the create request fails (for example the network drops), submitting the
same form again reuses the same `Idempotency-Key`, so it can never mint a
second token; if the first attempt had in fact succeeded, the page says so
and asks you to revoke that token and create a new one, because the secret
cannot be shown twice.

The same page lists your own tokens, newest first: name, the first eight
characters of the token (`gat_AbCdEfGh…`, for recognising it, not a
secret), scopes, when it was created and last used, when it expires, and
whether it is active, expired or revoked. **Revoke** asks for confirmation
and takes effect immediately. Members who are not administrators see an
explanation instead of the form.

Scripts that already hold a cookie session can still call the API directly:
`POST /api/v1/fleet/api-tokens` with an `Idempotency-Key` header, `GET` the
same path to list, and `POST /api/v1/fleet/api-tokens/{token_id}/revoke`
(see [the API contract](fleet-api.md#api-tokens)).

Give a token only the scopes its job needs. No scope implies another:
`fleet:operate` does not include `fleet:retire`, and `pools:manage` does not
include `pools:read`.

| Scope | Lets the bot |
|---|---|
| `fleet:read` | Read hosts, backends, grants, events, targets, queues, operations |
| `fleet:enroll` | Issue, check and revoke enrollment codes |
| `fleet:approve` | Approve a pending host |
| `fleet:operate` | Turn runtimes on/off, set lifecycle intent, set grants, set a machine's budget, save a pool's fleet execution profile, set a pool's runner counts, delete a retired machine (`DELETE /api/v1/fleet/hosts/{host_id}`, see [Deleting a machine](fleet-api.md#deleting-a-machine)) |
| `fleet:retire` | Retire a machine that is already out of service (not `active`), has been silent for the quiet period (15 minutes by default) and runs nothing; refused with every reason otherwise (see [Retiring a machine](fleet-api.md#retiring-a-machine)) |
| `pools:read` | List and read runner pools, their events and capacity history, and the pool form's options (installations, Bitbucket workspaces, defaults) |
| `pools:manage` | Create, edit and remove runner pools, with exactly the web form's validation (see [Runner pools with a token](fleet-api.md#runner-pools-with-a-token)) |

Full host onboarding needs `fleet:read`, `fleet:enroll`, `fleet:approve`
and `fleet:operate`; adding a pool for the new machine also needs
`pools:read` and `pools:manage`.

### What a bot can and cannot do

A token can do only what its scopes name, only on the routes that
[Who may call what](fleet-api.md#who-may-call-what) marks with that scope,
and only what its owner may do. Everything else stays with a person in the
browser, on purpose:

- **Token management** (create, list, revoke): cookie-only, so a leaked token
  cannot mint, extend or find other tokens.
- **Recovering a machine, rotating its credential, repairing unconfirmed
  starts, reading agent logs**: cookie-only.
- **Retiring a machine that is still alive or still running work**: only a
  browser session can; a token is held to the guard rails.
- **Pool actions** (pause, resume, scale, retry, reconcile), runners,
  workflow runs, logs, the audit log, settings (the built-in runner host and
  its budget included), platform connections, AI settings, users and
  backups: cookie-only.

Every one of those routes answers a bearer token with
`403 token_not_accepted`, whether the token is valid or not, so a script
never mistakes a refusal for an empty answer. A token does nothing on GitHub
beyond what the pool form itself does for its owner (listing the owner's
installations, repositories and runner groups, and starting a new pool's
runners), and nothing on the deploy host.

A token belongs to its creator: their role is re-checked on every request,
so demoting or deleting the owner neuters the token without touching it (a
demoted owner's token is refused even on pool routes where the owner, as a
member with installation access, could still act in the browser). Tokens
expire (default 90 days, at most 365) and are revocable by their owner or
any system admin.

Keep the token out of shell history and CI logs:

```sh
export GRIDOPS_URL=https://gridops.example.com
read -r GRIDOPS_API_TOKEN   # paste, then Enter; never on the command line
export GRIDOPS_API_TOKEN
```

In CI, load it from a masked secret. Revoke it the moment it is no longer
needed, under **Settings → API tokens** or with
`POST /api/v1/fleet/api-tokens/{token_id}/revoke` from a cookie session.

## The release operator token

A release can need machines paused, drained or activated through the API: a
machine that keeps failing starts is paused before the dry run, and a
machine may be drained and activated again around its upgrade. Whoever cuts the
release, a person or an agent, holds a token made for that release alone
(prerequisite P2 of the
[release checklist](release-checklist-template.md#prerequisites)):

| | |
|---|---|
| **Scopes** | `fleet:read` and `fleet:operate`; add `pools:read` and `pools:manage` only for a release that creates or changes a runner pool, and `fleet:retire` only for one that retires a machine |
| **Lifetime** | one to three days: long enough for one release, gone soon after. `scripts/release-preflight.sh` fails a token that expires within 6 hours or lives longer than 7 days |
| **Owner** | an administrator; the token can do only what its owner may do, checked on every request |
| **Name** | the release it is for, for example `release-v0.2.0`, so the audit trail says why a machine changed |

**Create it** as an administrator under **Settings → API tokens**: the name,
the scopes and a lifetime of 1 to 3 days. Copy the `gat_...` secret from
the dialog; it is never shown again.

**Keep it** in a secrets manager, or the secret store of the agent or CI job
that runs the release, and hand it over through the environment
(`GRIDOPS_TOKEN`, or `GRIDOPS_API_TOKEN` as above). Never put it in the
repository, an issue (the checklist issue included), a pull request, a chat,
a log or a command line. If it leaks, revoke it at once and create another.
Revoke it when the release is closed out (checklist step 23).

**What it can do:** every `fleet:read` route (machines, backends, events,
grants, targets, queues, pool profiles and capacity, operations) and every
`fleet:operate` route: set a machine's lifecycle (active, paused, draining,
maintenance), turn runtimes on or off, set target grants and budgets, save a
pool's fleet profile and runner counts, and delete a retired machine.
`fleet:operate` is broad, which is why the token lives for days, not months.
With the extra scopes it can also create, edit and remove runner pools
(`pools:manage`, with the form's validation) and read them (`pools:read`),
and retire a machine that is out of service, silent and idle
(`fleet:retire`).

**What it cannot do:** enroll or approve machines (`fleet:enroll`,
`fleet:approve`); recover a machine, rotate its credentials, read agent
logs, pause or scale a pool through the pool actions, change settings, or
create, list or revoke tokens (so it cannot extend itself), all of which
stay cookie-only and answer `403 token_not_accepted`; retire a machine that
is still active, reporting in or running work, which only a browser can;
and, without the extra scopes, anything on runner pools or retirement
(`403 forbidden` naming the missing scope). It works only on the routes
[the API contract](fleet-api.md#routes) marks with a scope, and it cannot
upgrade an agent: GridOps has no command
that makes an agent update itself yet, so system-service machines are
upgraded through the narrow sudo rule in the checklist's prerequisites. It
grants nothing on GitHub or on the deploy host.

**What can be checked with the token itself.** `scripts/release-preflight.sh`
proves both scopes: it lists the machines (`fleet:read`), and it sends a
`POST /api/v1/fleet/hosts/{host_id}/lifecycle` with an empty body for a host
id that matches no machine. The scope is checked before the body is read, so
a token with `fleet:operate` gets `400 invalid_request` and one without it
`403` naming `fleet:operate`; nothing changes either way (the refusal is
audited as `api_token.denied`, as any scope miss is). The token's expiry
and scopes are not readable with the token: the list of tokens is
cookie-only. Read the expiry on the Settings page and pass it to the
preflight as `PREFLIGHT_TOKEN_EXPIRES_AT`. Nor can a request that changes
nothing tell whether the owner is still an administrator: that shows up as a
`403 forbidden` without a scope on the first real change.

**Which release steps need what.** With `fleet:read` and `fleet:operate` a
release pauses, drains and activates machines and changes pool fleet
profiles and runner counts. Creating or editing a runner pool (for example
for a new macOS machine) needs `pools:read` and `pools:manage`; retiring the
machine a new one replaces needs `fleet:retire`, after it is paused or
drained, shut down for the quiet period and idle. Enrolling and approving a
machine need `fleet:enroll` and `fleet:approve`, which the release token
does not carry: a person does that, or a separate onboarding token. To check
that a token holds a pool scope before relying on it,
`GET /api/v1/runner-pools` answers `200` with `pools:read` and `403` naming
`pools:read` without it.

## Onboard a machine

1. **Issue a code** for the repositories the machine may serve:

   ```sh
   curl -sS -X POST "$GRIDOPS_URL/api/v1/fleet/targets" \
     -H "Authorization: Bearer $GRIDOPS_API_TOKEN" | jq .   # pick target IDs
   key="enroll-$(date +%s)"
   curl -sS -X POST "$GRIDOPS_URL/api/v1/fleet/enrollments" \
     -H "Authorization: Bearer $GRIDOPS_API_TOKEN" \
     -H "Idempotency-Key: $key" \
     -H 'Content-Type: application/json' \
     -d '{"targetIds":["<target-id>"]}'
   # → { "code": "…43 chars…", "enrollmentId": "…", "expiresAt": "…" }
   ```

   The code works once and expires in 10 minutes. To retry the *same*
   issue (for example after a network failure), reuse the same
   `Idempotency-Key`: you get the replay body instead of a second code.

2. **Enroll the agent** on the new machine with the code from step 1 —
   exactly as [the fleet-agent guide](fleet-agent.md) describes:

   ```sh
   GRIDOPS_FLEET_URL=https://gridops.example.com \
   GRIDOPS_ENROLLMENT_CODE=<code> \
   gridops-host-agent enroll
   ```

3. **Find the host** by polling the enrollment — `consumedHostId` is the
   correlation between the code and the host. Do not match on name:

   ```sh
   curl -sS "$GRIDOPS_URL/api/v1/fleet/enrollments/$ENROLLMENT_ID" \
     -H "Authorization: Bearer $GRIDOPS_API_TOKEN" | jq .consumedHostId
   ```

4. **Approve** once the host's approval evidence is available. The evidence
   carries a lease, so approve promptly after reading it:

   ```sh
   curl -sS "$GRIDOPS_URL/api/v1/fleet/hosts/$HOST_ID" \
     -H "Authorization: Bearer $GRIDOPS_API_TOKEN" | jq .item.approvalEvidence
   # state "available" →
   curl -sS -X POST "$GRIDOPS_URL/api/v1/fleet/hosts/$HOST_ID/approve" \
     -H "Authorization: Bearer $GRIDOPS_API_TOKEN" \
     -H 'Content-Type: application/json' \
     -d '{"expectedRevision":…,"expectedInventoryRevision":…,
          "expectedInventoryDigest":"…","targetIds":[…]}'
   ```

   When `state` is `unavailable`, `reason` says what to wait for
   (`pending_host`, `missing_observation`, `stale_inventory`,
   `observation_expired`, …) — poll with backoff. `requires_system_admin`
   means the token's owner lost admin: fix the owner, not the request. On
   `revision_conflict`, re-read the host and retry; if it reads back
   `enrollment: "approved"`, the approve already succeeded.

5. **Activate.** Approval leaves the host `paused` with no runtimes, so
   enable what it should run and set the intent to active:

   ```sh
   curl -sS "$GRIDOPS_URL/api/v1/fleet/hosts/$HOST_ID/backends" \
     -H "Authorization: Bearer $GRIDOPS_API_TOKEN" | jq .items   # backend ids
   curl -sS -X POST "$GRIDOPS_URL/api/v1/fleet/hosts/$HOST_ID/backends/$BACKEND_ID/enabled" \
     -H "Authorization: Bearer $GRIDOPS_API_TOKEN" \
     -H 'Content-Type: application/json' -d '{"enabled":true}'
   curl -sS -X POST "$GRIDOPS_URL/api/v1/fleet/hosts/$HOST_ID/lifecycle" \
     -H "Authorization: Bearer $GRIDOPS_API_TOKEN" \
     -H 'Content-Type: application/json' \
     -d '{"lifecycle":"active","expectedRevision":<fresh revision>}'
   ```

   Optionally widen a grant (`allow_native`, `allow_dockerSocket`) per
   target with `POST /hosts/{id}/grants/{target_id}`. For the pool to
   actually use fleet machines, its fleet execution profile must be
   enabled: `PUT /pools/{pool_id}/profile` with `enabled: true` (read the
   current profile first and flip the flag).

6. **Verify** the host reads `enrollment: "approved"`, `lifecycle:
   "active"` and `heartbeatFreshness: "fresh"` — then run a workflow.

## New-host checklist: the three layers that make a host take work

Approving a host is not the end. A freshly enrolled machine receives **no jobs**
until three independent switches line up, and a placement that cannot find them
fails with a different queue reason for each one still missing. Work through
this list for every new host (all of it is API-driven, with a `fleet:operate`
token):

1. **Enable a runtime backend.** New hosts report their backends (docker,
   native) but they start **disabled**. Pick the runtimes the machine should
   run and enable them:

   ```sh
   curl -sS -X POST "$GRIDOPS_URL/api/v1/fleet/hosts/$HOST_ID/backends/$BACKEND_ID/enabled" \
     -H "Authorization: Bearer $GRIDOPS_API_TOKEN" \
     -H 'Content-Type: application/json' -d '{"enabled":true}'
   ```

2. **Set the target grant's socket flag when the pool shares the host Docker
   socket.** Approval grants `allowSchedule` for the enrollment targets, but
   `allowDockerSocket` starts `false` — by design, it hands jobs root-equivalent
   access to the host's Docker daemon. If the pool's fleet profile has
   `requiresDockerSocket: true` (any pool whose jobs run `docker` inside the
   job — starting databases, containers, builds), grant it explicitly:

   ```sh
   curl -sS -X POST "$GRIDOPS_URL/api/v1/fleet/hosts/$HOST_ID/grants/$TARGET_ID" \
     -H "Authorization: Bearer $GRIDOPS_API_TOKEN" \
     -H 'Content-Type: application/json' \
     -d '{"allowSchedule":true,"allowNative":false,"allowDockerSocket":true}'
   ```

3. **Point the agent at the socket it should share.** The agent only
   advertises the Docker-socket capability when `GRIDOPS_AGENT_DOCKER_SOCKET`
   is set in its environment — Linux systems use
   `/var/run/docker.sock`, OrbStack on macOS uses
   `~/.orbstack/run/docker.sock`. Add it to the service unit
   (`Environment=GRIDOPS_AGENT_DOCKER_SOCKET=/var/run/docker.sock` in the
   systemd unit, or the launchd plist's `EnvironmentVariables`), reload, and
   restart the agent. A host missing this layer reports its backend as ready
   but cannot back a socket-requiring platform.

4. **Activate the host** (`activate` in the enrollment script, or
   `POST /hosts/{id}/lifecycle` with `active`).

5. **Verify with a job that uses Docker**, not just a placement: dispatch a
   workflow whose steps run `docker version` and a throwaway container, and
   confirm the runner container on the new host carries the socket mount:

   ```sh
   docker inspect <runner-container> --format '{{range .Mounts}}{{.Source}}:{{.Destination}} {{end}}'
   ```

### Reading the queue when a host will not take work

The pool queue (`GET /api/v1/fleet/pools/{pool_id}/queue`) names the exact
reason a runner is waiting; each maps to one missing layer:

| `reason` | Missing layer |
|---|---|
| `backend_disabled` | Layer 1 — the backend is disabled, or the host is paused/draining |
| `no_eligible_host` | Layer 2 — no host's grant satisfies the platform (usually `allowDockerSocket`) |
| `machines_failing_starts` | Not a host layer: every machine that could take it failed three runner starts in the last 15 minutes and is set aside until a start succeeds there or the failures age out (the machine page says why) |
| `capability_unverified` | Layer 3 — the agent does not advertise the socket capability (`GRIDOPS_AGENT_DOCKER_SOCKET` unset) |
| `forbidden` | Not a host layer: the intent is pinned to a replaced fleet profile (known issue; cancel and re-dispatch the run) |
| `physical_resource_exhausted` | The platform reservation exceeds every host's budget — check disk/cpu/memory reservations against reported budgets, and raise a budget with `PUT /api/v1/fleet/hosts/{id}/budget` (`{"cpuMillis":…,"memoryMib":…,"diskBytes":…}`, any subset, never above what the machine reports) |

## Manage runner pools

With `pools:read` and `pools:manage`. The request bodies are the web form's;
to edit, read the pool first and send it back with your change:

```sh
curl -sS "$GRIDOPS_URL/api/v1/runner-pools/options" \
  -H "Authorization: Bearer $GRIDOPS_API_TOKEN" | jq '.installations, .defaults'
curl -sS -X POST "$GRIDOPS_URL/api/v1/runner-pools" \
  -H "Authorization: Bearer $GRIDOPS_API_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"installationId":<id>,"name":"macos-release","scope":"organization",
       "mode":"ephemeral","provider":"tart","labels":[],"image":"<tart image>",
       "desiredCount":0,"minCount":0,"maxCount":2,"autoscalingEnabled":true,
       "queueScaleFactor":1,"idleTimeoutMinutes":10,"cpuLimit":4,
       "memoryLimitMb":8192,"runnerGroupId":1}'
# → 201 { "id": "<pool id>", "provisioned": [...], "cannotRun": ... }
curl -sS "$GRIDOPS_URL/api/v1/runner-pools/$POOL_ID" \
  -H "Authorization: Bearer $GRIDOPS_API_TOKEN"
curl -sS -X PUT "$GRIDOPS_URL/api/v1/runner-pools/$POOL_ID" \
  -H "Authorization: Bearer $GRIDOPS_API_TOKEN" \
  -H 'Content-Type: application/json' -d @pool.json
curl -sS -X DELETE "$GRIDOPS_URL/api/v1/runner-pools/$POOL_ID" \
  -H "Authorization: Bearer $GRIDOPS_API_TOKEN"
```

For a pool that includes Tart, `tartImage` is the Tart base VM each runner
clones (a local VM name on the Mac); with Tart as the only or first provider
`image` may carry it instead. When both `image` and the primary provider's
`tartImage` or `dockerImage` are sent they must be equal, on create and on
edit, so a body built from `.defaults` must replace `tartImage` too, not only
`image`, and an edit that changes the primary provider's image changes both
(or leaves the provider-specific field out).

A pool stores an image only for the providers it uses: `dockerImage` or
`tartImage` sent for a provider not in `providers` is ignored, and reads back
as `""`. To add a provider to a pool, send its image with the edit: adding Tart
without `tartImage` is refused with `400` (`Tart base VM (tartImage) is required
for a pool that uses macOS · Tart virtual machines.`), and adding Docker without
`dockerImage` likewise. Nothing is filled in from the server's defaults. Tart
with `"macosRuntime":"native"` clones no VM and needs no `tartImage`.

A validation error is `400 {"error": "..."}` with the form's message, a
duplicate name `409`, and removing a pool that still has runners on fleet
machines `409` until they stop. Creating a pool starts its `desiredCount`
runners, as the form does. The pool must be on an installation the token's
owner administers, and records the owner as its creator. It runs on fleet
machines once its fleet profile is enabled
(`PUT /api/v1/fleet/pools/{pool_id}/profile`, `fleet:operate`).

## Retire a machine

With `fleet:retire` (and `fleet:read` to read the revision). Take the
machine out of service first and let it go quiet:

```sh
curl -sS -X POST "$GRIDOPS_URL/api/v1/fleet/hosts/$HOST_ID/lifecycle" \
  -H "Authorization: Bearer $GRIDOPS_API_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"lifecycle":"draining","expectedRevision":<revision>}'   # fleet:operate
# wait for its work to finish, stop the agent, wait out the quiet period
curl -sS -X POST "$GRIDOPS_URL/api/v1/fleet/hosts/$HOST_ID/retire" \
  -H "Authorization: Bearer $GRIDOPS_API_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"expectedRevision":<fresh revision>}'
```

A `409 host_not_retirable` lists every reason in `details.reasons`
(`lifecycle_active`, `recently_seen`, `live_placement`, `unconfirmed_start`,
`live_runner`, `pending_command`); wait for or fix what each names, and try
again. On `revision_conflict`, re-read the host. A retired machine can then
be deleted with `fleet:operate`.

## The script

[`scripts/fleet-enroll-bot.sh`](../scripts/fleet-enroll-bot.sh) does the
server-side steps (POSIX sh, `curl` and `jq` only):

```sh
eval "$(scripts/fleet-enroll-bot.sh issue <target-id>...)"   # → code=, enrollment_id=
# run the agent on the machine with $code, then:
eval "$(scripts/fleet-enroll-bot.sh approve "$enrollment_id")"  # → host_id=
scripts/fleet-enroll-bot.sh activate "$host_id" [--pool <pool-id>]
```

It reads `GRIDOPS_URL` and `GRIDOPS_API_TOKEN` from the environment only,
refuses non-HTTPS URLs unless `GRIDOPS_ALLOW_INSECURE=1` (loopback
development), prints the one-time code to stdout only, never echoes the
token, generates a fresh `Idempotency-Key` per `issue` (set
`GRIDOPS_IDEMPOTENCY_KEY` to replay one), retries `approve` on
`revision_conflict`, and exits non-zero with the fleet error code on
failure.

## Audit

Every token-authenticated mutation is audited in the same transaction as
the change, with `"<login> (token: <name>)"` as the actor and the token's
id and name in the metadata, so a bot's actions are attributable after the
fact. Reads are not audited per request, but each successful call bumps the
token's `lastUsedAt` (rate-limited to one write per minute). A scope denial
writes an `api_token.denied` row — also rate-limited — naming the route.
Runner-pool changes are audited the same way (`runner_pool.created`,
`runner_pool.updated`, `runner_pool.deleted`), as are retirements
(`fleet.host.retired`). A `token_not_accepted` refusal writes nothing: the
route never looked at the token.
