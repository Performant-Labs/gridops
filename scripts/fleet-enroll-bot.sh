#!/bin/sh
# Bot-driven host enrollment for GridOps, using a scoped API token.
#
#   GRIDOPS_URL=https://gridops.example.com GRIDOPS_API_TOKEN=gat_... \
#     fleet-enroll-bot.sh issue <target-id>...        # prints code + enrollment id
#   GRIDOPS_URL=... GRIDOPS_API_TOKEN=... \
#     fleet-enroll-bot.sh approve <enrollment-id>     # waits, approves, prints host id
#   GRIDOPS_URL=... GRIDOPS_API_TOKEN=... \
#     fleet-enroll-bot.sh activate <host-id> [--pool <pool-id>]
#
# The token needs fleet:read, fleet:enroll, fleet:approve and fleet:operate.
# Reads GRIDOPS_URL and GRIDOPS_API_TOKEN from the environment, never from
# arguments; refuses non-HTTPS URLs unless GRIDOPS_ALLOW_INSECURE=1. On
# failure exits 1 and prints the fleet error code on stderr. The token is
# never echoed; the enrollment code goes to stdout only.
#
# Settings:
#   GRIDOPS_URL               GridOps address (required, HTTPS)
#   GRIDOPS_API_TOKEN         a gat_ token (required)
#   GRIDOPS_POLL_SECONDS      poll interval for waiting steps (default 2)
#   GRIDOPS_TIMEOUT_SECONDS   give up waiting after (default 300)
#   GRIDOPS_ALLOW_INSECURE=1  allow http URLs (loopback development only)
set -eu

URL="${GRIDOPS_URL:-}"
TOKEN="${GRIDOPS_API_TOKEN:-}"
POLL_SECONDS="${GRIDOPS_POLL_SECONDS:-2}"
TIMEOUT_SECONDS="${GRIDOPS_TIMEOUT_SECONDS:-300}"

say() { printf '%s\n' "$*" >&2; }
die() { printf 'fleet-enroll-bot: %s\n' "$*" >&2; exit 1; }

[ -n "$URL" ] || die "GRIDOPS_URL is required"
[ -n "$TOKEN" ] || die "GRIDOPS_API_TOKEN is required"
case "$URL" in
  https://*) ;;
  http://*)
    [ "${GRIDOPS_ALLOW_INSECURE:-0}" = "1" ] || die "GRIDOPS_URL must be HTTPS (set GRIDOPS_ALLOW_INSECURE=1 only for loopback development)"
    ;;
  *) die "GRIDOPS_URL must start with https:// (or http:// with GRIDOPS_ALLOW_INSECURE=1)" ;;
esac

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v jq >/dev/null 2>&1 || die "jq is required"

API="$URL/api/v1/fleet"

# request <method> <path> [idempotency-key] [json-body]
# Prints the response body on stdout when 2xx; on any other status prints
# the fleet error code on stderr and exits 1.
request() {
  method=$1
  path=$2
  key=${3:-}
  body=${4:-}
  set -- curl -sS -X "$method" "$API$path" \
    -H "Authorization: Bearer $TOKEN" \
    -H 'Content-Type: application/json' \
    --max-time 30 \
    -w '\n%{http_code}'
  [ -z "$key" ] || set -- "$@" -H "Idempotency-Key: $key"
  [ -z "$body" ] || set -- "$@" -d "$body"
  response=$("$@" 2>&1) || die "request to $path failed"
  status=$(printf '%s' "$response" | tail -n1)
  payload=$(printf '%s' "$response" | sed '$d')
  case "$status" in
    2*) printf '%s' "$payload" ;;
    *)
      code=$(printf '%s' "$payload" | jq -r '.code // "http_'$status'"' 2>/dev/null || true)
      [ -n "$code" ] || code="http_$status"
      die "$method $path -> $status $code"
      ;;
  esac
}

deadline() { echo $(( $(date +%s) + TIMEOUT_SECONDS )); }

# Polls GET /enrollments/{id} until the one-time code is consumed.
wait_for_host() {
  enrollment_id=$1
  end=$(deadline)
  while :; do
    host=$(request GET "/enrollments/$enrollment_id" | jq -r '.consumedHostId // empty')
    [ -n "$host" ] && { printf '%s' "$host"; return 0; }
    [ "$(date +%s)" -ge "$end" ] && die "timed out waiting for enrollment consumption"
    sleep "$POLL_SECONDS"
  done
}

# Polls the host until fresh approval evidence is available, then prints it.
wait_for_evidence() {
  host=$1
  end=$(deadline)
  while :; do
    evidence=$(request GET "/hosts/$host" | jq -c '
      select(.item.approvalEvidence.state == "available") | .item |
      {expectedRevision: .approvalEvidence.expectedRevision,
       expectedInventoryRevision: .approvalEvidence.expectedInventoryRevision,
       expectedInventoryDigest: .approvalEvidence.expectedInventoryDigest,
       targetIds: .enrollmentTargetIds}')
    [ -n "$evidence" ] && { printf '%s' "$evidence"; return 0; }
    [ "$(date +%s)" -ge "$end" ] && die "timed out waiting for approval evidence"
    sleep "$POLL_SECONDS"
  done
}

cmd_issue() {
  # Zero target ids enrolls a host with an empty scope; approval then
  # grants nothing by default, which the admin widens later.
  if [ "$#" -gt 0 ]; then
    targets=$(printf '%s\n' "$@" | jq -R . | jq -s .)
  else
    targets='[]'
  fi
  # A fresh key per issue call; GRIDOPS_IDEMPOTENCY_KEY lets a retry
  # replay the same call instead of minting a second code.
  key="${GRIDOPS_IDEMPOTENCY_KEY:-enroll-$(uuid_guess)}"
  body=$(jq -n --argjson targetIds "$targets" '{targetIds: $targetIds}')
  response=$(request POST /enrollments "$key" "$body")
  code=$(printf '%s' "$response" | jq -r '.code')
  enrollment_id=$(printf '%s' "$response" | jq -r '.enrollmentId')
  [ -n "$code" ] && [ "$code" != "null" ] || {
    say "issue: no code in response; if this was a replay, reuse the enrollment id below"
  }
  # The one-time code, stdout only: pipe it straight to the agent.
  printf 'code=%s\nenrollment_id=%s\n' "$code" "$enrollment_id"
}

uuid_guess() {
  if command -v uuidgen >/dev/null 2>&1; then
    uuidgen
  else
    # POSIX sh has no $RANDOM; read a few bytes from the kernel instead.
    printf 'bot-%s' "$(date +%s)-$$-$(od -An -N4 -tu4 /dev/urandom | tr -d ' \n')"
  fi
}

cmd_approve() {
  [ "$#" -eq 1 ] || die "usage: approve <enrollment-id>"
  enrollment_id=$1
  host=$(wait_for_host "$enrollment_id")
  say "enrolled host: $host"
  evidence=$(wait_for_evidence "$host")
  body=$(printf '%s' "$evidence" | jq -c '{expectedRevision, expectedInventoryRevision, expectedInventoryDigest, targetIds}')
  # approve relies on expectedRevision compare-and-swap: on
  # revision_conflict re-read and retry per the docs.
  attempt=0
  while :; do
    attempt=$((attempt + 1))
    if request POST "/hosts/$host/approve" '' "$body" >/dev/null 2>&1; then
      break
    fi
    [ "$attempt" -ge 5 ] && die "approve kept conflicting after $attempt attempts"
    say "approve conflicted (attempt $attempt); re-reading host"
    sleep "$POLL_SECONDS"
    evidence=$(request GET "/hosts/$host" | jq -c '
      select(.item.approvalEvidence.state == "available") | .item |
      {expectedRevision: .approvalEvidence.expectedRevision,
       expectedInventoryRevision: .approvalEvidence.expectedInventoryRevision,
       expectedInventoryDigest: .approvalEvidence.expectedInventoryDigest,
       targetIds: .enrollmentTargetIds}')
    [ -n "$evidence" ] || die "approval evidence disappeared"
    body=$(printf '%s' "$evidence" | jq -c '{expectedRevision, expectedInventoryRevision, expectedInventoryDigest, targetIds}')
  done
  say "approved: $host"
  printf 'host_id=%s\n' "$host"
}

cmd_activate() {
  pool=''
  host=''
  while [ "$#" -gt 0 ]; do
    case $1 in
      --pool) pool=$2; shift 2 ;;
      *) host=$1; shift ;;
    esac
  done
  [ -n "$host" ] || die "usage: activate <host-id> [--pool <pool-id>]"
  backends=$(request GET "/hosts/$host/backends" | jq -r '.items[].id')
  [ -n "$backends" ] || die "host reports no runtimes to enable"
  for backend in $backends; do
    request POST "/hosts/$host/backends/$backend/enabled" '' '{"enabled":true}' >/dev/null
    say "runtime enabled: $backend"
  done
  # Backends bumps the host revision, so read it after the toggles. The
  # host detail exposes the row revision as policyRevision.
  revision=$(request GET "/hosts/$host" | jq -r '.item.policyRevision')
  request POST "/hosts/$host/lifecycle" '' "{\"lifecycle\":\"active\",\"expectedRevision\":$revision}" >/dev/null
  say "host active: $host"
  if [ -n "$pool" ]; then
    current=$(request GET "/pools/$pool/profile" | jq -c 'if .item then .item else empty end')
    [ -n "$current" ] || die "pool $pool has no fleet profile yet; configure its first platform once, then re-run"
    body=$(printf '%s' "$current" | jq -c '.enabled = true | {enabled, placement, mode, cpuMillis, memoryMiB, diskBytes, platforms: [.platforms[] | {runtimeKind, executionOs, architecture, image, requiresDockerSocket}]}')
    request PUT "/pools/$pool/profile" '' "$body" >/dev/null
    say "pool fleet execution enabled: $pool"
  fi
  printf 'host_id=%s\n' "$host"
}

case "${1:-}" in
  issue) shift; cmd_issue "$@" ;;
  approve) shift; cmd_approve "$@" ;;
  activate) shift; cmd_activate "$@" ;;
  *)
    die "usage: $0 issue <target-id>... | approve <enrollment-id> | activate <host-id> [--pool <pool-id>]"
    ;;
esac
