# Fleet API contract

This page lists the version-one contracts that the fleet routes and the host
agent share. The Rust types live in `crates/gridops-core/src/fleet/protocol`;
the browser parsers in `src/features/fleet` follow the same fixtures. Examples
tagged `fleet-error` and `fleet-limits` are parsed by
`crates/gridops-core/tests/fleet_contracts.rs`, so they cannot drift.

## Routes

Browser routes authenticate with the normal session cookie, or — where the
table says so — with a scoped API token: `Authorization: Bearer gat_...`.
A request carrying both a cookie and a bearer token is refused. Agent
routes (`/api/v1/fleet/agent/...`) use exactly one
`Authorization: Bearer <host credential>` header and cannot reach browser
routes; a token is never a host credential and vice versa. A bearer token
sent to a route that does not take tokens is refused with
`403 token_not_accepted`, never answered as a signed-out caller. The table
below lists the fleet routes; [Who may call what](#who-may-call-what) lists
every route GridOps serves, including the runner-pool routes that take a
token. See [the automation guide](fleet-automation.md) for tokens, scopes and
bot-driven automation.

| Method | Path | Caller | Purpose |
|---|---|---|---|
| GET | `/api/v1/fleet/hosts` | browser, `fleet:read` | Paginated host list |
| GET | `/api/v1/fleet/hosts/{host_id}` | browser, `fleet:read` | One host |
| DELETE | `/api/v1/fleet/hosts/{host_id}` | browser, `fleet:operate` | Delete a retired machine (see [Deleting a machine](#deleting-a-machine)) |
| GET | `/api/v1/fleet/hosts/{host_id}/backends[/{backend_id}]` | browser, `fleet:read` | Execution backends |
| GET | `/api/v1/fleet/hosts/{host_id}/domains[/{domain_id}]` | browser, `fleet:read` | Resource domains (physical access only) |
| GET | `/api/v1/fleet/hosts/{host_id}/samples` | browser, `fleet:read` | Resource samples (physical access only) |
| GET | `/api/v1/fleet/hosts/{host_id}/events` | browser, `fleet:read` | Host event history |
| GET | `/api/v1/fleet/hosts/{host_id}/grants` | browser, `fleet:read` | Target grants |
| GET | `/api/v1/fleet/targets` | browser, `fleet:read` | CI targets |
| GET | `/api/v1/fleet/workload` | browser, `fleet:read` | Workload overview |
| GET | `/api/v1/fleet/pools/{pool_id}/profile` | browser, `fleet:read` | Pool fleet execution profile |
| GET | `/api/v1/fleet/pools/{pool_id}/queue` | browser, `fleet:read` | Queued runners and blocked hosts |
| GET | `/api/v1/fleet/pools/{pool_id}/capacity` | browser, `fleet:read` | The pool's minimum, desired and maximum runner counts |
| POST | `/api/v1/fleet/enrollments` | browser, `fleet:enroll` | Issue a single-use enrollment code |
| GET | `/api/v1/fleet/enrollments/{enrollment_id}` | browser, `fleet:enroll` | Enrollment status |
| POST | `/api/v1/fleet/enrollments/{enrollment_id}/revoke` | browser, `fleet:enroll` | Revoke an unused code |
| POST | `/api/v1/fleet/hosts/{host_id}/approve` | browser, `fleet:approve` | Approve a pending host |
| POST | `/api/v1/fleet/hosts/{host_id}/retire` | browser, `fleet:retire` (guard rails) | Retire a host (see [Retiring a machine](#retiring-a-machine)) |
| POST | `/api/v1/fleet/hosts/{host_id}/lifecycle` | browser, `fleet:operate` | Set host intent (active, paused, draining, maintenance) |
| PUT | `/api/v1/fleet/hosts/{host_id}/budget` | browser, `fleet:operate` | Set the machine's physical budget (CPU, memory, disk), up to what it reports |
| POST | `/api/v1/fleet/hosts/{host_id}/backends/{backend_id}/enabled` | browser, `fleet:operate` | Turn a runtime on or off |
| POST | `/api/v1/fleet/hosts/{host_id}/grants/{target_id}` | browser, `fleet:operate` | Set a target grant |
| PUT | `/api/v1/fleet/pools/{pool_id}/profile` | browser, `fleet:operate` | Save the pool's fleet execution profile |
| PUT | `/api/v1/fleet/pools/{pool_id}/capacity` | browser, `fleet:operate` | Set the pool's runner counts; runners above the new count finish their job and are not replaced |
| POST | `/api/v1/fleet/api-tokens` | browser (admin, cookie only) | Create an API token |
| GET | `/api/v1/fleet/api-tokens` | browser (cookie only) | List own API tokens |
| POST | `/api/v1/fleet/api-tokens/{token_id}/revoke` | browser (cookie only) | Revoke an API token |
| POST | `/api/v1/fleet/hosts/{host_id}/recover` | browser (cookie only) | Approve explicit host recovery |
| POST | `/api/v1/fleet/hosts/{host_id}/credentials/rotate` | browser (cookie only) | Request credential rotation |
| GET | `/api/v1/operations/{operation_id}` | browser, `fleet:read` | Durable operation status |
| POST | `/api/v1/fleet/agent/enroll` | agent | Consume an enrollment code |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/handshake` | agent | Open a session and negotiate protocol |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/inventory` | agent | Submit inventory |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/heartbeat` | agent | Heartbeat and samples |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/credentials/rotate` | agent | Exchange a rotated credential |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/credentials/ack` | agent | Acknowledge the new credential |

Any other path under `/api/v1/fleet` or `/api/v1/operations` returns the
fleet `not_found` error, not the SPA.

Mutating browser requests are replay-safe, either through an
`Idempotency-Key` header or through `expectedRevision` compare-and-swap;
the table under "Idempotency and retries" says which route uses which.
Replaying a key
with the same body returns the original result; a different body returns
`idempotency_conflict`. Writes that change an existing record carry the
expected revision and fail with `revision_conflict` when it has moved.

## Who may call what

This table is authoritative for every route the server registers, fleet or
not. `crates/gridops-api/tests/fleet_http/token_access.rs` reads it and checks
each row against the running server, and a unit test checks that it lists
exactly the routes the router registers and exactly the token-capable routes
of the OpenAPI document.

- **Session**: `required` means a signed-in browser session (the
  `gridops_session` cookie) is needed when no token is used; `optional` means
  a caller without one gets a `200` signed-out body (`"authenticated": false`
  or empty data), which the web app relies on. `—`: the route never reads a
  session (public, the GitHub OAuth and webhook endpoints, the host agent).
- **Token scope**: the scope a bearer token must hold. `—`: the route takes
  no API token.
- **Origin check** applies to cookie requests only. `exact`: a cookie write
  needs exactly one `Origin` header equal to the configured base URL (fleet
  routes). `foreign refused`: a cookie request whose `Origin` names another
  site is refused with `403`; a request without `Origin` passes (the legacy
  web-app routes). A token request carries no ambient credential and is never
  origin-checked.
- **A bearer token gets** what any `Authorization: Bearer …` header gets on
  the route, valid or not:
  - `accepted`: authenticated as the token's owner, if the token is live,
    holds the scope, and its owner is still an administrator; otherwise
    `401 unauthenticated` (unknown, malformed, expired or revoked token, or a
    cookie sent with it) or `403 forbidden` (missing scope, named in
    `details.requiredScope`, or an owner who is no longer an administrator).
  - `403 token_not_accepted`: refused before the handler runs, whether the
    token is valid or not and whether a cookie is also present. The route
    never answers a token as if the caller were signed out. An
    `Authorization` header with another scheme (for example `Basic` from a
    proxy in front of GridOps) is ignored by the web-app routes, as before,
    and refused as `401 unauthenticated` by the fleet routes, as before.
  - `ignored`: the route does not authenticate users at all; the header has
    no effect.
  - `401 unauthenticated`: a host-agent route; it takes only a host
    credential, never an API token.

Token management, recovering a machine, rotating its credential, repairing
unconfirmed starts, reading agent logs, pool actions (pause, resume, scale,
retry, reconcile), the audit log, settings and platform connections stay
cookie-only on purpose. Paths not listed here answer `404`.

| Method | Path | Session | Token scope | Origin check | A bearer token gets |
|---|---|---|---|---|---|
| GET | `/api/health` | — | — | — | ignored |
| GET | `/api/v1/openapi.json` | — | — | — | ignored |
| GET | `/install.sh` | — | — | — | ignored |
| GET | `/api/v1/auth/github` | — | — | — | ignored |
| GET | `/api/v1/auth/github/callback` | — | — | — | ignored |
| GET | `/auth/github` | — | — | — | ignored |
| GET | `/auth/github/callback` | — | — | — | ignored |
| GET | `/auth/github-app/manifest/callback` | — | — | — | ignored |
| POST | `/api/v1/webhooks/github` | — | — | — | ignored |
| POST | `/api/webhooks/github` | — | — | — | ignored |
| GET | `/api/v1/auth/session` | optional | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/auth/me` | required | — | — | 403 `token_not_accepted` |
| POST | `/api/v1/auth/logout` | required | — | foreign refused | 403 `token_not_accepted` |
| POST | `/auth/logout` | required | — | foreign refused | 403 `token_not_accepted` |
| PUT | `/api/v1/users/{user_id}/role` | required | — | foreign refused | 403 `token_not_accepted` |
| GET | `/api/v1/overview` | optional | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/overview/capacity` | optional | `pools:read` | — | accepted |
| GET | `/api/v1/overview/tart-host` | optional | `pools:read` | — | accepted |
| GET | `/api/v1/overview/queued-jobs` | optional | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/search` | required | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/repositories` | optional | — | — | 403 `token_not_accepted` |
| POST | `/api/v1/repositories/sync` | required | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/runner-pools` | optional | `pools:read` | — | accepted |
| POST | `/api/v1/runner-pools` | required | `pools:manage` | foreign refused | accepted |
| GET | `/api/v1/runner-pools/options` | optional | `pools:read` | — | accepted |
| GET | `/api/v1/runner-pools/repositories` | required | `pools:read` | — | accepted |
| GET | `/api/v1/installations/{installation_id}/runner-groups` | required | `pools:read` | — | accepted |
| GET | `/api/v1/installations/{installation_id}/repositories` | required | `pools:read` | — | accepted |
| GET | `/api/v1/runner-pools/{pool_id}` | required | `pools:read` | — | accepted |
| PUT | `/api/v1/runner-pools/{pool_id}` | required | `pools:manage` | foreign refused | accepted |
| DELETE | `/api/v1/runner-pools/{pool_id}` | required | `pools:manage` | foreign refused | accepted |
| GET | `/api/v1/runner-pools/{pool_id}/events` | required | `pools:read` | — | accepted |
| POST | `/api/v1/runner-pools/{pool_id}/action` | required | — | foreign refused | 403 `token_not_accepted` |
| GET | `/api/v1/runners` | optional | — | — | 403 `token_not_accepted` |
| POST | `/api/v1/runners/{runner_id}/action` | required | — | foreign refused | 403 `token_not_accepted` |
| GET | `/api/v1/runners/{runner_id}/logs` | required | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/runners/{runner_id}/logs/stream` | required | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/workflow-runs` | optional | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/workflow-runs/{run_id}` | required | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/workflow-runs/{run_id}/logs` | required | — | — | 403 `token_not_accepted` |
| POST | `/api/v1/workflow-runs/{run_id}/action` | required | — | foreign refused | 403 `token_not_accepted` |
| GET | `/api/v1/workflow-jobs/{job_id}/logs` | required | — | — | 403 `token_not_accepted` |
| POST | `/api/v1/workflow-jobs/{job_id}/agent-runs` | required | — | foreign refused | 403 `token_not_accepted` |
| GET | `/api/v1/agent-runs` | required | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/agent-runs/{run_id}` | required | — | — | 403 `token_not_accepted` |
| POST | `/api/v1/agent-runs/{run_id}/cancel` | required | — | foreign refused | 403 `token_not_accepted` |
| GET | `/api/v1/settings/ai` | required | — | — | 403 `token_not_accepted` |
| POST | `/api/v1/settings/ai/api-key` | required | — | foreign refused | 403 `token_not_accepted` |
| POST | `/api/v1/settings/ai/oauth/start` | required | — | foreign refused | 403 `token_not_accepted` |
| POST | `/api/v1/settings/ai/oauth/complete` | required | — | foreign refused | 403 `token_not_accepted` |
| DELETE | `/api/v1/settings/ai/connections/{connection_id}` | required | — | foreign refused | 403 `token_not_accepted` |
| GET | `/api/v1/settings/ai/connections/{connection_id}/models` | required | — | — | 403 `token_not_accepted` |
| PUT | `/api/v1/settings/ai/agent` | required | — | foreign refused | 403 `token_not_accepted` |
| GET | `/api/workflow-runs/{run_id}/logs` | required | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/webhooks` | optional | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/webhooks/{delivery_id}` | required | — | — | 403 `token_not_accepted` |
| POST | `/api/v1/webhooks/{delivery_id}/retry` | required | — | foreign refused | 403 `token_not_accepted` |
| POST | `/api/v1/webhooks/dismiss-failed` | required | — | foreign refused | 403 `token_not_accepted` |
| GET | `/api/v1/audit` | optional | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/log-streams` | optional | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/log-streams/{stream_id}/logs` | required | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/settings` | optional | — | — | 403 `token_not_accepted` |
| PUT | `/api/v1/settings` | required | — | foreign refused | 403 `token_not_accepted` |
| PUT | `/api/v1/settings/runner-host` | required | — | foreign refused | 403 `token_not_accepted` |
| PUT | `/api/v1/settings/runner-host/budget` | required | — | foreign refused | 403 `token_not_accepted` |
| GET | `/api/v1/platform-connections/bitbucket` | required | — | — | 403 `token_not_accepted` |
| POST | `/api/v1/platform-connections/bitbucket` | required | — | foreign refused | 403 `token_not_accepted` |
| PUT | `/api/v1/platform-connections/bitbucket/{connection_id}` | required | — | foreign refused | 403 `token_not_accepted` |
| DELETE | `/api/v1/platform-connections/bitbucket/{connection_id}` | required | — | foreign refused | 403 `token_not_accepted` |
| POST | `/api/v1/github-app/manifest` | required | — | foreign refused | 403 `token_not_accepted` |
| GET | `/api/v1/backups/database` | required | — | — | 403 `token_not_accepted` |
| GET | `/api/backups/database` | required | — | — | 403 `token_not_accepted` |
| GET | `/api/v1/fleet/hosts` | required | `fleet:read` | — | accepted |
| GET | `/api/v1/fleet/hosts/{host_id}` | required | `fleet:read` | — | accepted |
| DELETE | `/api/v1/fleet/hosts/{host_id}` | required | `fleet:operate` | exact | accepted |
| GET | `/api/v1/fleet/hosts/{host_id}/backends` | required | `fleet:read` | — | accepted |
| GET | `/api/v1/fleet/hosts/{host_id}/backends/{backend_id}` | required | `fleet:read` | — | accepted |
| POST | `/api/v1/fleet/hosts/{host_id}/backends/{backend_id}/enabled` | required | `fleet:operate` | exact | accepted |
| GET | `/api/v1/fleet/hosts/{host_id}/domains` | required | `fleet:read` | — | accepted |
| GET | `/api/v1/fleet/hosts/{host_id}/domains/{domain_id}` | required | `fleet:read` | — | accepted |
| GET | `/api/v1/fleet/hosts/{host_id}/samples` | required | `fleet:read` | — | accepted |
| GET | `/api/v1/fleet/hosts/{host_id}/events` | required | `fleet:read` | — | accepted |
| GET | `/api/v1/fleet/hosts/{host_id}/grants` | required | `fleet:read` | — | accepted |
| POST | `/api/v1/fleet/hosts/{host_id}/grants/{target_id}` | required | `fleet:operate` | exact | accepted |
| GET | `/api/v1/fleet/hosts/{host_id}/unconfirmed` | required | `fleet:read` | — | accepted |
| POST | `/api/v1/fleet/hosts/{host_id}/unconfirmed/repair` | required | — | exact | 403 `token_not_accepted` |
| GET | `/api/v1/fleet/hosts/{host_id}/logs` | required | — | — | 403 `token_not_accepted` |
| PUT | `/api/v1/fleet/hosts/{host_id}/budget` | required | `fleet:operate` | exact | accepted |
| POST | `/api/v1/fleet/hosts/{host_id}/lifecycle` | required | `fleet:operate` | exact | accepted |
| POST | `/api/v1/fleet/hosts/{host_id}/approve` | required | `fleet:approve` | exact | accepted |
| POST | `/api/v1/fleet/hosts/{host_id}/retire` | required | `fleet:retire` | exact | accepted |
| POST | `/api/v1/fleet/hosts/{host_id}/recover` | required | — | exact | 403 `token_not_accepted` |
| POST | `/api/v1/fleet/hosts/{host_id}/credentials/rotate` | required | — | exact | 403 `token_not_accepted` |
| GET | `/api/v1/fleet/targets` | required | `fleet:read` | — | accepted |
| GET | `/api/v1/fleet/workload` | required | `fleet:read` | — | accepted |
| GET | `/api/v1/fleet/pools/{pool_id}/profile` | required | `fleet:read` | — | accepted |
| PUT | `/api/v1/fleet/pools/{pool_id}/profile` | required | `fleet:operate` | exact | accepted |
| GET | `/api/v1/fleet/pools/{pool_id}/queue` | required | `fleet:read` | — | accepted |
| GET | `/api/v1/fleet/pools/{pool_id}/capacity` | required | `fleet:read` | — | accepted |
| PUT | `/api/v1/fleet/pools/{pool_id}/capacity` | required | `fleet:operate` | exact | accepted |
| POST | `/api/v1/fleet/enrollments` | required | `fleet:enroll` | exact | accepted |
| GET | `/api/v1/fleet/enrollments/{enrollment_id}` | required | `fleet:enroll` | — | accepted |
| POST | `/api/v1/fleet/enrollments/{enrollment_id}/revoke` | required | `fleet:enroll` | exact | accepted |
| POST | `/api/v1/fleet/api-tokens` | required | — | exact | 403 `token_not_accepted` |
| GET | `/api/v1/fleet/api-tokens` | required | — | — | 403 `token_not_accepted` |
| POST | `/api/v1/fleet/api-tokens/{token_id}/revoke` | required | — | exact | 403 `token_not_accepted` |
| GET | `/api/v1/operations/{operation_id}` | required | `fleet:read` | — | accepted |
| POST | `/api/v1/fleet/agent/enroll` | — | — | — | ignored |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/handshake` | — | — | — | 401 `unauthenticated` |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/inventory` | — | — | — | 401 `unauthenticated` |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/heartbeat` | — | — | — | 401 `unauthenticated` |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/logs` | — | — | — | 401 `unauthenticated` |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/output` | — | — | — | 401 `unauthenticated` |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/bootstrap` | — | — | — | 401 `unauthenticated` |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/commands/poll` | — | — | — | 401 `unauthenticated` |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/commands/result` | — | — | — | 401 `unauthenticated` |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/credentials/rotate` | — | — | — | 401 `unauthenticated` |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/credentials/ack` | — | — | — | 401 `unauthenticated` |

## API tokens

Administrators manage tokens in the web app under **Settings → API tokens**,
which calls the three cookie-only routes above: it lists the caller's own
tokens (name, display prefix, scopes, created, last used, expires, and
active/expired/revoked state), creates a token with a fresh
`Idempotency-Key` per submission (reused when the same submission is
retried), shows the `tokenSecret` from the `201` exactly once in a dialog
and discards it when the dialog closes, and revokes with a confirmation.
Members who are not administrators see an explanation instead.

The routes remain available to scripts that hold a cookie session:

- `POST /api/v1/fleet/api-tokens` with `Idempotency-Key` and
  `{ "name": string, "scopes": string[], "expiresInDays"?: number }`
  (1–365, default 90). `201` returns the metadata plus `tokenSecret`; a
  replay of the key returns `200 { "token": ... }` without the secret.
- `GET /api/v1/fleet/api-tokens` returns `{ "items": [...] }`, newest
  first, with no secret or verifier.
- `POST /api/v1/fleet/api-tokens/{token_id}/revoke` returns
  `{ "token": ... }` and is idempotent by state.

Token metadata is `id`, `name`, `displayPrefix` (the first eight characters
after `gat_`), `scopes`, `createdAt`, `expiresAt`, `lastUsedAt` and
`revokedAt`.

## Retiring a machine

`POST /api/v1/fleet/hosts/{host_id}/retire` with `{ "expectedRevision": n }`
revokes the machine's credentials and stops it taking work; its history
stays, and it can then be [deleted](#deleting-a-machine). Retiring cannot be
undone: the computer comes back only by enrolling again, as a new record.

**From a browser session** an administrator retires under the original
rules: the current revision, and no placement on the machine that is not
`cleaned` or `failed` (`400 invalid_request` otherwise). Nothing below
changes that.

**With an API token** the token needs `fleet:retire`, which no other scope
(`fleet:operate` included) implies, and its owner must still be an
administrator. The machine must also be one that cannot hurt a healthy
fleet. These guard rails are checked in the same database transaction as the
retire, so nothing can change between the check and the change:

| Reason | Refused while |
|---|---|
| `lifecycle_active` | The machine's lifecycle is `active`. Pause it, drain it or put it in maintenance first (`POST .../lifecycle`, `fleet:operate`). |
| `recently_seen` | It reported in (heartbeat or inventory) within the quiet period: 15 minutes, or `GRIDOPS_TOKEN_RETIRE_MIN_OFFLINE_MINUTES` (a whole number from 2 to 10080; the API refuses to start on anything else). A machine that never reported in counts as quiet. |
| `live_placement` | A placement is not `cleaned`, `failed` or `uncertain`, or capacity it holds is not released. |
| `unconfirmed_start` | A placement is `uncertain`: its start could not be confirmed. Repair it on the machine page. |
| `live_runner` | A runner placed on the machine has not finished (its status is not `deleted` or `failed`). |
| `pending_command` | A command for the machine is still waiting or running within its deadline. |

A refusal is `409 host_not_retirable` with `details.kind = "retire_blocked"`
and **every** reason that applies in `details.reasons`, in the order above:

```json fleet-error
{
  "code": "host_not_retirable",
  "message": "Machine cannot be retired with an API token yet",
  "requestId": "0192f0c4-7a1e-7c3b-9f00-3d1c2b4a5e72",
  "details": { "kind": "retire_blocked", "reasons": ["lifecycle_active", "recently_seen"] }
}
```

So a leaked `fleet:retire` token can retire only a machine that is already
out of service, silent and idle: one that was taking no work anyway. A
machine that is alive and paused, or drained but still reporting in, is
refused. Retiring a machine that is already retired is a no-op (`200`), for
a token and a browser alike. A stale `expectedRevision` is
`409 revision_conflict`. The audit row and fleet event `fleet.host.retired`
name the actor and, for a token, `"<login> (token: <name>)"` with the
token's id and name in `viaApiToken`.

There is no second approver and no separate expiry rule for this scope: the
guard rails above are what make a leaked token harmless to a working fleet.
Give a token `fleet:retire` only for the clean-up it is made for, and keep
its lifetime short (see [the release operator
token](fleet-automation.md#the-release-operator-token)). Operators should
review the guard rails and the quiet period for their fleet.

## Runner pools with a token

The runner-pool routes listed in [Who may call what](#who-may-call-what)
accept a token with `pools:read` (list and read pools, their events, the
capacity history and the pool form's options) or `pools:manage` (create,
edit and remove pools). Neither implies the other: a bot that creates pools
and then lists them needs both. The token acts as its owner, who must still
be a system administrator: the handlers are the web app's own, with the
form's validation (a `400 { "error": "..." }` with the form's message) and
the owner's GitHub installation access, so a token can see and change
exactly the pools its owner can, and nothing on an installation the owner
cannot reach. Responses carry no provider credentials or tokens. Every
create, edit and remove is audited (`runner_pool.created`,
`runner_pool.updated`, `runner_pool.deleted`) with the actor
`"<login> (token: <name>)"` and `viaApiToken` in the metadata. Token-request
refusals (`401 unauthenticated`, `403 forbidden` with or without
`details.requiredScope`) use the fleet error envelope; a browser session
gets the same answers as before. Pool actions (pause, resume, scale, retry,
reconcile) stay cookie-only.

## Deleting a machine

`DELETE /api/v1/fleet/hosts/{host_id}` removes a machine that can never take
work again from the machine list, the machine counts, the Fleet and Overview
pages, and capacity. It needs an administrator, or a token with
`fleet:operate`; a member, or a token without the scope, gets `403`. It takes
no body and returns `200 { "hostId": "..." }`.

A machine can be deleted when it is **retired**, or when it **never finished
enrolling** (pending or revoked) and has not reported in for 90 seconds. An
approved machine is retired first (from its page, or with a `fleet:retire`
token within its guard rails; see [Retiring a machine](#retiring-a-machine)),
which also revokes its credential. Anything else is refused with
`409 host_not_deletable` and `details.kind = "delete_blocked"`; the refusal
lists **every** reason that applies in `details.reasons`:

```json fleet-error
{
  "code": "host_not_deletable",
  "message": "Machine cannot be deleted yet",
  "requestId": "0192f0c4-7a1e-7c3b-9f00-3d1c2b4a5e71",
  "details": { "kind": "delete_blocked", "reasons": ["live_placement", "pending_command"] }
}
```

| Reason | Meaning |
|---|---|
| `not_retired` | The machine is approved and not retired. Retire it first. |
| `still_heartbeating` | It reported in within the last 90 seconds (not checked for a retired machine, whose credential is already revoked). |
| `active_credential` | An approved machine still holds an unrevoked credential. |
| `live_placement` | A placement is not `cleaned` or `failed`, or capacity it holds is not released. |
| `unconfirmed_start` | A placement is `uncertain`: its start could not be confirmed. Repair it on the machine page. |
| `live_runner` | A runner placed on the machine has not finished (its status is not `deleted` or `failed`). |
| `pending_command` | A command for the machine is still waiting or running within its deadline. |

The delete is **soft**. The record stays, hidden everywhere, because
placements, commands, events, samples and grants reference it and the runs
list, the audit log and the event history keep showing them. A delete:

- keeps the host record, its resource domains and budget, placements, intents,
  commands and results, events, samples, inventory, agent logs and consumed
  enrollments;
- revokes its credentials and open credential rotations, its unused recovery
  codes, and its target and user grants, and turns its backends off;
- leaves the machine `retired` with enrollment `revoked`, so every credential,
  recovery and approval check that refuses a retired machine refuses it;
- leaves pool profile host selectors alone, because removing the last one could
  widen a profile that selects "all authorized hosts";
- writes the audit row and fleet event `fleet.host.deleted`, naming the actor
  (and the API token name when a token was used), the machine's name and id,
  and its last heartbeat (`lastHeartbeatAt`, milliseconds).

A deleted machine is final. An agent that keeps talking with its credential is
rejected as `unauthenticated` on every agent route and cannot recreate the
record (only a valid enrollment code creates a machine). Recovery, rotation
and approval of a deleted machine are refused, and the database rejects
changing it back. Enrolling the same computer again with a new code creates a
new, clean, pending record.

## Responses and pagination

Query responses carry `schemaVersion: 1`, `readAt`, and either `item` or
`items` plus `nextCursor`. Cursors are opaque, at most 4,096 bytes, and bound
to the route, filters and reader. A malformed or foreign cursor is
`invalid_request`. An expired cursor is `cursor_expired`, and one older than
the retained history is `cursor_gap` (both 410). Pages default to 25 items, at most 100.

Host and backend items have `projection: "host"` or `"target"`. Readers with
physical host access see telemetry and budgets. Target-only readers see
limited identity and target-scoped counts, with no budgets, measurements,
local paths or credentials.

Integers sent to browsers stay within 2^53 − 1. Unknown fields are rejected
in both directions.

## Errors

Every fleet error has the same shape:

```json fleet-error
{
  "code": "revision_conflict",
  "message": "The host changed since it was read.",
  "requestId": "0192f0c4-7a1e-7c3b-9f00-3d1c2b4a5e6f",
  "details": { "kind": "revision", "currentRevision": 7 }
}
```

A bearer token sent to a route that does not accept tokens is refused before
anything else, whether the token is valid or not:

```json fleet-error
{
  "code": "token_not_accepted",
  "message": "This route does not accept API tokens",
  "requestId": "0192f0c4-7a1e-7c3b-9f00-3d1c2b4a5e73",
  "details": null
}
```

A token that authenticated but lacks the route's scope is refused with the
missing scope named, and nothing else is revealed:

```json fleet-error
{
  "code": "forbidden",
  "message": "Fleet action is not authorized",
  "requestId": "0192f0c4-7a1e-7c3b-9f00-3d1c2b4a5e70",
  "details": { "kind": "scope", "requiredScope": "fleet:approve" }
}
```

| Code | HTTP | Retryable | A bot should |
|---|---|---|---|
| `invalid_request`, `contract_range` | 400 | No | Fix the request; a resend will not improve on its own. |
| `unauthenticated` | 401 | No | Check the token. Unknown, expired, revoked and malformed tokens are deliberately indistinguishable. Mint a new token if the old one expired or was revoked. |
| `forbidden` | 403 | No | The token lacks the route's scope (`details.kind = "scope"` names it), or the owner's role no longer permits the action. Cannot be fixed by retrying. |
| `token_not_accepted` | 403 | No | The route does not take API tokens at all (see [Who may call what](#who-may-call-what)); no token or scope will change that. Use a browser session, or a route that takes a token. |
| `not_found` | 404 | No | The id is wrong or the record was deleted. |
| `host_not_deletable` | 409 | After cleanup | `details.reasons` lists everything keeping the machine: finish or clean up what each reason names, then delete again. See [Deleting a machine](#deleting-a-machine). |
| `host_not_retirable` | 409 | After the machine is out of service, silent and idle | `details.reasons` lists every guard rail the machine fails for a token retire. See [Retiring a machine](#retiring-a-machine). |
| `revision_conflict` | 409 | Yes | Re-read the resource, recompute from the fresh `revision` (or approval evidence), resend. On approve, `enrollment: "approved"` means the call already succeeded. |
| `idempotency_conflict` | 409 | No | The same `Idempotency-Key` was used with a different body. Do not retry with that key. |
| `enrollment_expired`, `grant_expired`, `cursor_expired`, `cursor_gap` | 410 | Re-issue | The one-time artifact is gone. Issue a new code (or re-read page one for cursors). |
| `payload_too_large` | 413 | No | Send a smaller body. |
| `rate_limited` | 429 | Yes | Wait `details.retryAfterMs`, then resend the same request. |
| `dependency_unavailable`, `not_ready` | 503 | Yes | Back off and retry; the control plane is busy or starting. |

`details` is optional and is one of `revision`, `invalid_field` (with
`field`), `limit` (with `maximum`), `retry` (with `retryAfterMs`),
`cursor_gap`, `readiness` (with a placement rejection reason), `scope`
(with `requiredScope`, returned only when a bearer token authenticated but
lacks the route's scope), `delete_blocked` (with `reasons`, on
`host_not_deletable`), or `retire_blocked` (with `reasons`, on
`host_not_retirable`). A host with no compatible capacity is a queued
placement with an explanation, not an error.

## Idempotency and retries

Mutating browser routes protect replays in one of two ways:

| Route | Mechanism | A replay returns |
|---|---|---|
| `POST /fleet/enrollments` | `Idempotency-Key` (required) | The original result body (`created` once, then `replay` metadata without a second code) |
| `POST /fleet/api-tokens` | `Idempotency-Key` (required) | `201` with the plaintext once, then the token metadata without the secret; a different body with the same key is `idempotency_conflict` |
| `POST /fleet/hosts/{id}/recover` | `Idempotency-Key` (required) | The original result |
| `POST /fleet/hosts/{id}/credentials/rotate` | `Idempotency-Key` (required) | The original result |
| `POST /fleet/hosts/{id}/approve` | `expectedRevision` | `revision_conflict` with `details.currentRevision`; if the host reads back `enrollment: "approved"`, the call already succeeded |
| `POST /fleet/hosts/{id}/lifecycle` | `expectedRevision` | as above, with `lifecycle` as the readback |
| `DELETE /fleet/hosts/{id}` | idempotent by state | `404 not_found`: the machine is already gone |
| `POST /fleet/hosts/{id}/backends/{backend_id}/enabled` | last-write-wins | the new state |
| `PUT /fleet/hosts/{id}/budget` | last-write-wins | the budget in force, returned in the response |
| `POST /fleet/hosts/{id}/grants/{target_id}` | last-write-wins | the new state |
| `POST /fleet/enrollments/{id}/revoke` | idempotent by state | `200` again |
| `POST /fleet/api-tokens/{id}/revoke` | idempotent by state | `200` again |
| `PUT /fleet/pools/{id}/profile` | last-write-wins | the saved profile |
| `PUT /fleet/pools/{id}/capacity` | last-write-wins | the saved `minCount`, `desiredCount`, `maxCount` (`0 <= min <= desired <= max <= 256`; a repository pool's max must cover its repositories) |

Routes not listed (`retire`, `repair_unconfirmed`) take `expectedRevision`
in their body and fail replays with `revision_conflict`.

## Machine-readable contract

The fleet and operations routes, the runner-pool routes that take a token,
their scopes and the error shapes are published as an OpenAPI 3.1
document, served unauthenticated at
[`GET /api/v1/openapi.json`](/api/v1/openapi.json) and checked into the
repository at `docs/openapi/gridops-v1.json`. A drift test fails the build
when the spec and the token-scope table disagree, and the examples in the
spec are parsed by the same wire types as the fixtures in this document.

## Protocol versions

The server and agent negotiate during the handshake. Version 1 is the only
version. An agent whose advertised range excludes the server's gets
`incompatible` and stays in maintenance until it's upgraded; it never falls
back to an unversioned transport.

## Limits

Defaults, as accepted by `ProtocolLimits`. Operators may lower any value but
can't exceed the hard cap. Heartbeat must be below stale, stale below
offline, and the start grant can't exceed local authority.

```json fleet-limits
{
  "json_bytes": 1048576,
  "event_batch_items": 100,
  "event_batch_bytes": 262144,
  "chunk_bytes": 262144,
  "poll_ms": 25000,
  "pending_commands": 100,
  "page_size": 25,
  "command_attempts": 10,
  "log_spool_bytes": 268435456,
  "enrollment_ms": 600000,
  "autoapproval_ms": 600000,
  "preparation_ms": 1200000,
  "start_ms": 60000,
  "authority_ms": 90000,
  "heartbeat_ms": 15000,
  "stale_ms": 45000,
  "offline_ms": 90000
}
```

| Limit | Default | Hard cap |
|---|---|---|
| JSON request body | 1 MiB | 2 MiB |
| Event batch | 100 items / 256 KiB | same |
| Log or artifact chunk | 256 KiB | same |
| Command long poll | 25 s | 25 s |
| Pending commands per host | 100 | 1,000 |
| Page size | 25 | 100 |
| Local log spool | 256 MiB | 2 GiB |
| Enrollment code lifetime | 10 min | 1 h (at least 1 min) |
| Start grant | 60 s | 60 s |
| Local authority without contact | 90 s | 90 s |
| Heartbeat / stale / offline | 15 s / 45 s / 90 s | same |

## Domain relationships

- A **host** is one physical or virtual machine running the agent. It has an
  epoch that changes on re-enrollment or recovery; commands from an older
  epoch are refused.
- A new enrollment always starts `pending`. If another approved, unretired
  host reports the same hardware fingerprint, the host read sets
  `duplicateHardware: true` for the approver, and `duplicateHardwareOf` lists
  up to five of those other hosts the reader may see (`id` and `name`), so the
  stale record is easy to tell from the live one. It's a hint only: a
  fingerprint never proves identity, and replacing a host goes through
  explicit recovery. To drop the stale record, retire it and then delete it.
- A host exposes **backends** (Docker, Tart, native) and **resource
  domains**. Domains form a tree under one root domain, so Docker, Tart and
  native work on the same machine share one physical budget.
- **Samples** are inclusive measurements of a domain. A sample is "current"
  only for the live session and when received less than 45 s ago.
- **Events** are append-only. Host retirement, deletion and credential
  rotation keep history and foreign keys intact.
- A placement that fails records a `placement_failed` event on its host. Its
  detail holds a stable `reason`, a short `message` and the numbers behind it
  (such as `elapsed_ms` and `allowed_ms`), and its runner's `failure_reason`
  carries the same message. The reasons are `agent_reported`,
  `agent_outcome_unknown`, `prepare_deadline_expired`, `start_deadline_expired`,
  `agent_session_lost`, `host_epoch_changed`, `runner_removed_before_start`,
  `operator_released` and `unknown`. The host detail read summarises them as
  `failedPlacementsLastHour` and `recentPlacementFailures` (the last 24 hours,
  grouped by reason). Only failures after the release that added this are
  recorded; earlier failed placements have no event.
- A runner whose placement failed to start is started again as a new runner
  (new name, new registration), placed with the machines its earlier attempts
  failed on tried last. That is only done once nothing of the failed
  placement can run on its machine: it is `failed` (never `uncertain`), its
  local absence and delayed-start exclusion are proven, its capacity is
  released and its prepare and start commands are settled. A runner gets at
  most three placements, and only failures the machine could have caused are
  retried (not `runner_removed_before_start`, `operator_released`, `unknown`,
  or an agent-reported `configuration`, `unsupported` or `cancelled` code).
  Each retry records a `placement_retried` event on the machine where the
  start failed; its detail holds `attempt`, `max_attempts`, `failure_reason`,
  `retry_runner_id`, `retry_operation_id`, `avoided_host_ids` and a `message`.
- A machine that failed three starts in the last 15 minutes, with no
  successful start since, takes no new runners. The host read carries this as
  `startGate` (`null` when the machine takes new runners) with `failures`,
  `windowMinutes`, `since` (the oldest failure counted), `until` (when it is
  reconsidered if nothing else changes) and `message`, for example "Not
  accepting new runners: 3 start failures in the last 15 minutes." A start
  that succeeds on the machine lifts it at once. It only leaves the machine
  out of new placements: its lifecycle, its running jobs and everything else
  stay as they are. A runner that waits because every machine that could
  take it is set aside this way shows the queue reason
  `machines_failing_starts`. For system administrators, the overview
  (`GET /api/v1/overview`) also lists one `warning` alert in `slo.alerts` per
  approved, unretired machine set aside this way, titled with the machine's
  name, with `detail` set to the gate's `message`, `until` as above and
  `href` `/fleet/machines/<id>`; the alert goes once the gate lifts.
