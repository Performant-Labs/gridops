# Fleet API contract

This page lists the version-one contracts that the fleet routes and the host
agent share. The Rust types live in `crates/gridops-core/src/fleet/protocol`;
the browser parsers in `src/features/fleet` follow the same fixtures. Examples
tagged `fleet-error` and `fleet-limits` are parsed by
`crates/gridops-core/tests/fleet_contracts.rs`, so they cannot drift.

## Routes

Browser routes use the normal session cookie and reject an `Authorization`
header. Agent routes (`/api/v1/fleet/agent/...`) use exactly one
`Authorization: Bearer <host credential>` header and cannot reach browser
routes.

| Method | Path | Caller | Purpose |
|---|---|---|---|
| GET | `/api/v1/fleet/hosts` | browser | Paginated host list |
| GET | `/api/v1/fleet/hosts/{host_id}` | browser | One host |
| GET | `/api/v1/fleet/hosts/{host_id}/backends[/{backend_id}]` | browser | Execution backends |
| GET | `/api/v1/fleet/hosts/{host_id}/domains[/{domain_id}]` | browser | Resource domains (physical access only) |
| GET | `/api/v1/fleet/hosts/{host_id}/samples` | browser | Resource samples (physical access only) |
| GET | `/api/v1/fleet/hosts/{host_id}/events` | browser | Host event history |
| POST | `/api/v1/fleet/enrollments` | browser | Issue a single-use enrollment code |
| GET | `/api/v1/fleet/enrollments/{enrollment_id}` | browser | Enrollment status |
| POST | `/api/v1/fleet/enrollments/{enrollment_id}/revoke` | browser | Revoke an unused code |
| POST | `/api/v1/fleet/hosts/{host_id}/approve` | browser | Approve a pending host |
| POST | `/api/v1/fleet/hosts/{host_id}/recover` | browser | Approve explicit host recovery |
| POST | `/api/v1/fleet/hosts/{host_id}/credentials/rotate` | browser | Request credential rotation |
| GET | `/api/v1/operations/{operation_id}` | browser | Durable operation status |
| POST | `/api/v1/fleet/agent/enroll` | agent | Consume an enrollment code |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/handshake` | agent | Open a session and negotiate protocol |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/inventory` | agent | Submit inventory |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/heartbeat` | agent | Heartbeat and samples |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/credentials/rotate` | agent | Exchange a rotated credential |
| POST | `/api/v1/fleet/agent/hosts/{host_id}/credentials/ack` | agent | Acknowledge the new credential |

Any other path under `/api/v1/fleet` or `/api/v1/operations` returns the
fleet `not_found` error, not the SPA.

Mutating browser requests take an `Idempotency-Key` header. Replaying a key
with the same body returns the original result; a different body returns
`idempotency_conflict`. Writes that change an existing record carry the
expected revision and fail with `revision_conflict` when it has moved.

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

| Code | HTTP |
|---|---|
| `invalid_request`, `contract_range` | 400 |
| `unauthenticated` | 401 |
| `forbidden` | 403 |
| `not_found` | 404 |
| `revision_conflict`, `idempotency_conflict` | 409 |
| `enrollment_expired`, `grant_expired`, `cursor_expired`, `cursor_gap` | 410 |
| `payload_too_large` | 413 |
| `rate_limited` | 429 |
| `dependency_unavailable`, `not_ready` | 503 |

`details` is optional and is one of `revision`, `invalid_field`, `limit`
(with `maximum`), `retry` (with `retryAfterMs`), `cursor_gap`, or `readiness`
(with a placement rejection reason). A host with no compatible capacity is a
queued placement with an explanation, not an error.

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
- A new enrollment always starts `pending`. If another approved host reports
  the same hardware fingerprint, the host read sets `duplicateHardware: true`
  for the approver. It's a hint only: a fingerprint never proves identity, and
  replacing a host goes through explicit recovery.
- A host exposes **backends** (Docker, Tart, native) and **resource
  domains**. Domains form a tree under one root domain, so Docker, Tart and
  native work on the same machine share one physical budget.
- **Samples** are inclusive measurements of a domain. A sample is "current"
  only for the live session and when received less than 45 s ago.
- **Events** are append-only. Host retirement and credential rotation keep
  history and foreign keys intact.
