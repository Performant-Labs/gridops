# Bot-driven fleet automation

How a script, CI job or agent operates the fleet with a scoped API token —
no browser session, no direct database writes. The
[API contract](fleet-api.md) lists every route; this page is the recipe.

## Create a token

An administrator creates a token once, in **Settings → API tokens**
(or `POST /api/v1/fleet/api-tokens` with a cookie session). The plaintext
`gat_...` is shown exactly once; only its SHA-256 verifier is stored.

For full host onboarding a bot needs all four v1 scopes:

| Scope | Lets the bot |
|---|---|
| `fleet:read` | Read hosts, backends, grants, events, targets, queues, operations |
| `fleet:enroll` | Issue, check and revoke enrollment codes |
| `fleet:approve` | Approve a pending host |
| `fleet:operate` | Turn runtimes on/off, set lifecycle intent, set grants, save a pool's fleet execution profile |

Recovering, retiring or rotating a host's credentials, reading agent logs,
and managing tokens stay cookie-only. A token belongs to its creator: their
role is re-checked on every request, so demoting or deleting the owner
neuters the token without touching it. Tokens expire (default 90 days, at
most 365) and are revocable by their owner or any system admin.

Keep the token out of shell history and CI logs:

```sh
export GRIDOPS_URL=https://gridops.example.com
read -r GRIDOPS_API_TOKEN   # paste, then Enter; never on the command line
export GRIDOPS_API_TOKEN
```

In CI, load it from a masked secret. Revoke it the moment it is no longer
needed: `POST /api/v1/fleet/api-tokens/{token_id}/revoke`.

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
