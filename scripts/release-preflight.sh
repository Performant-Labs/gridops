#!/bin/sh
# Checks, before step 1 of a release checklist, that whoever cuts the release
# (a person or an agent) holds what the release needs. Prints one line per
# check, PASS, FAIL or SKIP, runs every check even after a failure, and exits
# 1 with the list of failures at the end (0 when nothing failed).
#
#   GRIDOPS_URL=https://<deployment> GRIDOPS_TOKEN=gat_... \
#     PREFLIGHT_UPGRADE_HOSTS="<ssh target> <ssh target>" \
#     PREFLIGHT_DATA_PATH=/var/lib/docker PREFLIGHT_DEPLOY_HOST=<ssh target> \
#     sh scripts/release-preflight.sh
#
# What it checks (docs/release-checklist-template.md, "Prerequisites"):
#   control-plane       GET /api/health answers
#   token-read          the token is valid and holds fleet:read (it lists machines)
#   token-operate       the token holds fleet:operate, proved with a request
#                       the server refuses before it can change anything
#   token-expiry        only when PREFLIGHT_TOKEN_EXPIRES_AT is set: the API
#                       does not let a token read its own expiry or scopes
#   heartbeats          every approved, non-retired machine has a fresh heartbeat
#   start-gate          no machine is set aside for failing runner starts
#   placement-failures  no machine had failed placements in the last hour
#   macos-machine       a macOS ARM64 machine is approved, active, not set aside
#                       and has a fresh heartbeat: every release builds the
#                       macOS agent on one (skipped only with
#                       PREFLIGHT_RELEASE_MACOS=false)
#   upgrade-sudo <h>    each listed system-service machine lets this user run
#                       the upgrade command with sudo and no password
#   disk                free space where the deployment keeps its data
#
# Settings (environment only; the token is never taken from arguments and
# never printed):
#   GRIDOPS_URL                     the deployment's address (https://)
#   GRIDOPS_TOKEN                   the release operator token (GRIDOPS_API_TOKEN
#                                   is read when GRIDOPS_TOKEN is unset)
#   GRIDOPS_ALLOW_INSECURE=1        allow an http:// URL (loopback development only)
#   PREFLIGHT_TOKEN_EXPIRES_AT      the token's expiry from Settings -> API tokens,
#                                   YYYY-MM-DD or YYYY-MM-DDTHH:MM:SSZ (UTC)
#   PREFLIGHT_MIN_TOKEN_HOURS       it must outlive the release by this much (6)
#   PREFLIGHT_MAX_TOKEN_DAYS        and expire within this many days (7)
#   PREFLIGHT_IGNORE_HOSTS          machine names or ids left out on purpose
#   PREFLIGHT_MAX_FAILED_PLACEMENTS failures per machine in the last hour allowed (0)
#   PREFLIGHT_RELEASE_MACOS=false   only when the repository variable RELEASE_MACOS
#                                   is false: the release then publishes no macOS
#                                   agent, which its checklist must say
#   PREFLIGHT_UPGRADE_HOSTS         space-separated ssh targets of the machines
#                                   that run the agent as a system service
#   PREFLIGHT_UPGRADE_COMMAND       the root-owned upgrade wrapper the sudo rule
#                                   allows (/usr/local/sbin/gridops-agent-upgrade)
#   PREFLIGHT_DATA_PATH             path whose free space is checked, for example
#                                   Docker's root directory on the deploy host
#   PREFLIGHT_DEPLOY_HOST           ssh target of the deploy host; without it
#                                   the disk is checked on this machine
#   PREFLIGHT_MIN_FREE_GB           free space needed at PREFLIGHT_DATA_PATH (10)
#
# Needs curl and jq; ssh only for PREFLIGHT_UPGRADE_HOSTS or PREFLIGHT_DEPLOY_HOST.
set -eu
# Host lists are split on spaces; never expand them as file patterns.
set -f

URL="${GRIDOPS_URL:-}"
URL="${URL%/}"
TOKEN="${GRIDOPS_TOKEN:-${GRIDOPS_API_TOKEN:-}}"
MIN_TOKEN_HOURS="${PREFLIGHT_MIN_TOKEN_HOURS:-6}"
MAX_TOKEN_DAYS="${PREFLIGHT_MAX_TOKEN_DAYS:-7}"
MAX_FAILED="${PREFLIGHT_MAX_FAILED_PLACEMENTS:-0}"
UPGRADE_HOSTS="${PREFLIGHT_UPGRADE_HOSTS:-}"
UPGRADE_COMMAND="${PREFLIGHT_UPGRADE_COMMAND:-/usr/local/sbin/gridops-agent-upgrade}"
DATA_PATH="${PREFLIGHT_DATA_PATH:-}"
DEPLOY_HOST="${PREFLIGHT_DEPLOY_HOST:-}"
MIN_FREE_GB="${PREFLIGHT_MIN_FREE_GB:-10}"
IGNORE_HOSTS="${PREFLIGHT_IGNORE_HOSTS:-}"
RELEASE_MACOS="${PREFLIGHT_RELEASE_MACOS:-true}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

failed=""
pass() { printf 'PASS  %s: %s\n' "$1" "$2"; }
skip() { printf 'SKIP  %s: %s\n' "$1" "$2"; }
fail() {
  printf 'FAIL  %s: %s\n' "$1" "$2"
  failed="${failed}  - $1: $2
"
}
is_number() { case "$1" in '' | *[!0-9]*) return 1 ;; *) return 0 ;; esac; }
# A path or ssh target that is safe to put on a remote command line.
is_plain() { case "$1" in '' | *[!A-Za-z0-9/._@:-]*) return 1 ;; *) return 0 ;; esac; }
plural() { if [ "$1" = 1 ]; then echo "1 $2"; else echo "$1 ${2}s"; fi; }

# --- Settings ---------------------------------------------------------------

api_ready=1
if [ -z "$URL" ]; then
  fail config "GRIDOPS_URL is not set (the deployment's https:// address)"
  api_ready=0
else
  case "$URL" in
    https://*) ;;
    http://*)
      if [ "${GRIDOPS_ALLOW_INSECURE:-0}" != 1 ]; then
        fail config "GRIDOPS_URL must use https:// (GRIDOPS_ALLOW_INSECURE=1 only for loopback development)"
        api_ready=0
      fi
      ;;
    *) fail config "GRIDOPS_URL must start with https://"; api_ready=0 ;;
  esac
fi
if [ -z "$TOKEN" ]; then
  fail config "GRIDOPS_TOKEN is not set (the release operator token, see docs/fleet-automation.md)"
  api_ready=0
fi
for setting in "PREFLIGHT_MIN_TOKEN_HOURS=$MIN_TOKEN_HOURS" "PREFLIGHT_MAX_TOKEN_DAYS=$MAX_TOKEN_DAYS" \
  "PREFLIGHT_MAX_FAILED_PLACEMENTS=$MAX_FAILED" "PREFLIGHT_MIN_FREE_GB=$MIN_FREE_GB"; do
  is_number "${setting#*=}" || fail config "${setting%%=*} must be a whole number, not '${setting#*=}'"
done
is_number "$MAX_FAILED" || MAX_FAILED=0
is_number "$MIN_FREE_GB" || MIN_FREE_GB=""
is_number "$MIN_TOKEN_HOURS" || MIN_TOKEN_HOURS=6
is_number "$MAX_TOKEN_DAYS" || MAX_TOKEN_DAYS=7
case "$RELEASE_MACOS" in
  true | false) ;;
  *) fail config "PREFLIGHT_RELEASE_MACOS must be true or false, not '$RELEASE_MACOS'"; RELEASE_MACOS=true ;;
esac
upgrade_ready=1
if [ -n "$UPGRADE_HOSTS" ] && ! is_plain "$UPGRADE_COMMAND"; then
  fail config "PREFLIGHT_UPGRADE_COMMAND must be a plain path (letters, digits, / . _ -)"
  upgrade_ready=0
fi
if [ "$api_ready" = 1 ]; then
  for tool in curl jq; do
    if ! command -v "$tool" >/dev/null 2>&1; then
      fail config "$tool is required"
      api_ready=0
    fi
  done
fi

# --- HTTP -------------------------------------------------------------------

# curl retries timeouts, 429 and 5xx; --retry-all-errors (curl 7.71+, probed
# so an older curl still works) also retries a dropped connection. The
# token goes in through curl's config on stdin, so it is never on a command
# line where another user could read it.
retry_all=""
if [ "$api_ready" = 1 ] && curl --help all 2>/dev/null | grep -q -e --retry-all-errors; then
  retry_all="--retry-all-errors"
fi

# request <name> <method> <path> [curl args...]: the body goes to
# $tmp/<name>, the HTTP status to $status ("000" when nothing answered).
request() {
  name=$1 method=$2 path=$3
  shift 3
  status=$(printf 'header = "Authorization: Bearer %s"\n' "$TOKEN" |
    curl -sS -K - -X "$method" --retry 3 $retry_all --connect-timeout 10 --max-time 30 \
      -H 'Accept: application/json' -o "$tmp/$name" -w '%{http_code}' "$@" "$URL$path" \
      2>"$tmp/$name.err") || status="000"
  [ -s "$tmp/$name" ] || : > "$tmp/$name"
}

# The fleet error code and, on a scope refusal, the scope it named.
error_of() {
  jq -r '[.code // empty, (.details.requiredScope // empty)] | join(" ")' "$tmp/$1" 2>/dev/null || true
}

explain() { # <name>: why a request did not succeed
  if [ "$status" = 000 ]; then
    printf 'no answer from %s (%s)' "$URL" "$(head -n 1 "$tmp/$1.err" 2>/dev/null)"
    return
  fi
  code=$(error_of "$1")
  case "$code" in
    unauthenticated*) printf 'HTTP %s unauthenticated: the token is unknown, expired or revoked; create a new one' "$status" ;;
    "forbidden "*) printf 'HTTP %s forbidden: the token lacks %s' "$status" "${code#forbidden }" ;;
    forbidden) printf "HTTP %s forbidden: the token's owner may no longer be an administrator" "$status" ;;
    '') printf 'HTTP %s' "$status" ;;
    *) printf 'HTTP %s %s' "$status" "$code" ;;
  esac
}

# --- Control plane and token -------------------------------------------------

hosts_ready=0
if [ "$api_ready" = 1 ]; then
  # The health route needs no token: this tells a dead address from a bad token.
  status=$(curl -sS --retry 3 $retry_all --connect-timeout 10 --max-time 30 \
    -o "$tmp/health" -w '%{http_code}' "$URL/api/health" 2>"$tmp/health.err") || status="000"
  if [ "$status" = 200 ]; then
    version=$(jq -r '.version // "unknown"' "$tmp/health" 2>/dev/null || echo unknown)
    pass control-plane "$URL answers, version $version"
  else
    fail control-plane "$(explain health)"
  fi

  # fleet:read: list every machine, following the cursor.
  : > "$tmp/hosts.jsonl"
  cursor="" pages=0 read_ok=1
  while :; do
    if [ -n "$cursor" ]; then
      request hosts GET /api/v1/fleet/hosts -G --data-urlencode limit=100 --data-urlencode "cursor=$cursor"
    else
      request hosts GET /api/v1/fleet/hosts -G --data-urlencode limit=100
    fi
    if [ "$status" != 200 ]; then read_ok=0; break; fi
    jq -c '.items[]' "$tmp/hosts" >> "$tmp/hosts.jsonl" 2>/dev/null || { status="unreadable"; read_ok=0; break; }
    cursor=$(jq -r '.nextCursor // empty' "$tmp/hosts")
    pages=$((pages + 1))
    [ -n "$cursor" ] && [ "$pages" -lt 100 ] || break
  done
  if [ "$read_ok" = 1 ]; then
    pass token-read "the token is valid and holds fleet:read ($(plural "$(wc -l < "$tmp/hosts.jsonl" | tr -d ' ')" machine) listed)"
    hosts_ready=1
  elif [ "$status" = unreadable ]; then
    fail token-read "GET /api/v1/fleet/hosts answered with something that is not a machine list"
  else
    fail token-read "$(explain hosts)"
  fi

  # fleet:operate, without operating anything. The scope is checked before
  # the request body is read, and this body ({}) is never a valid lifecycle
  # change, for a host id that matches no machine: a token with the scope gets
  # 400 invalid_request, one without it 403 naming fleet:operate. The refusal
  # is recorded as an api_token.denied audit row, as any scope miss is.
  request operate POST /api/v1/fleet/hosts/00000000-0000-0000-0000-000000000000/lifecycle \
    -H 'Content-Type: application/json' -d '{}'
  if [ "$status" = 400 ] && [ "$(error_of operate)" = invalid_request ]; then
    pass token-operate "the token holds fleet:operate (drain and activate a machine)"
  else
    fail token-operate "$(explain operate)"
  fi
else
  fail control-plane "not checked: GRIDOPS_URL or GRIDOPS_TOKEN is missing or unusable"
fi

# Expiry: GridOps keeps it, but only a cookie session can list tokens, so a
# token cannot read its own. The operator copies it from the Settings page.
to_epoch() {
  case "$1" in
    [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]) set -- "$1T00:00:00Z" ;;
    [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z) ;;
    *) return 1 ;;
  esac
  # GNU date, then BSD and macOS date, then BusyBox date (no T or Z).
  date -u -d "$1" +%s 2>/dev/null ||
    date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$1" +%s 2>/dev/null ||
    date -u -d "$(printf '%s' "$1" | sed 's/T/ /; s/Z$//')" +%s 2>/dev/null
}
expires="${PREFLIGHT_TOKEN_EXPIRES_AT:-}"
if [ -z "$expires" ]; then
  skip token-expiry "the API cannot tell a token its own expiry; read it under Settings -> API tokens and set PREFLIGHT_TOKEN_EXPIRES_AT to check it"
elif ! expires_at=$(to_epoch "$expires"); then
  fail token-expiry "cannot read PREFLIGHT_TOKEN_EXPIRES_AT '$expires' (use YYYY-MM-DD or YYYY-MM-DDTHH:MM:SSZ, UTC)"
else
  left=$((expires_at - $(date -u +%s)))
  if [ "$left" -le 0 ]; then
    fail token-expiry "the token expired at $expires; create a new one"
  elif [ "$left" -lt $((MIN_TOKEN_HOURS * 3600)) ]; then
    fail token-expiry "the token expires in $((left / 60)) minutes, less than the $MIN_TOKEN_HOURS hours a release needs; create a new one"
  elif [ "$left" -gt $((MAX_TOKEN_DAYS * 86400)) ]; then
    fail token-expiry "the token lives $((left / 86400)) more days; a release token should be short-lived (at most $MAX_TOKEN_DAYS days): revoke it and create one that expires sooner"
  else
    pass token-expiry "the token expires in $((left / 3600)) hours"
  fi
fi

# --- Machines -----------------------------------------------------------------

if [ "$hosts_ready" = 1 ]; then
  ignore_json=$(printf '%s\n' "$IGNORE_HOSTS" | tr -s ' \t' '\n' | jq -R . | jq -s -c 'map(select(length > 0))')
  # Machines in the fleet: approved and not retired. Pending and revoked
  # enrollments never take work.
  jq -c --argjson ignore "$ignore_json" '
    select(.enrollment == "approved" and .lifecycle != "retired")
    | select((.id as $id | .name as $n | $ignore | any(. == $id or . == $n)) | not)' \
    "$tmp/hosts.jsonl" > "$tmp/fleet.jsonl"
  ignored=$(jq -r --argjson ignore "$ignore_json" '
    select(.enrollment == "approved" and .lifecycle != "retired")
    | select(.id as $id | .name as $n | $ignore | any(. == $id or . == $n)) | .name' \
    "$tmp/hosts.jsonl" | paste -sd ' ' -)
  restricted=$(jq -r 'select(.projection != "host") | .name' "$tmp/hosts.jsonl" | paste -sd ' ' -)
  total=$(wc -l < "$tmp/fleet.jsonl" | tr -d ' ')
  note=""
  [ -z "$ignored" ] || note=" (ignored: $ignored)"

  if [ -n "$restricted" ]; then
    fail heartbeats "the token's owner sees only a restricted view of $restricted; the release token must belong to an administrator"
  else
    stale=$(jq -r 'select(.heartbeatFreshness != "fresh") | "\(.name) (\(.heartbeatFreshness))"' \
      "$tmp/fleet.jsonl" | paste -sd ',' - | sed 's/,/, /g')
    if [ -z "$stale" ]; then
      pass heartbeats "$(plural "$total" machine), all fresh$note"
    else
      fail heartbeats "not reporting in: $stale; bring them back, or retire them, or list them in PREFLIGHT_IGNORE_HOSTS and record why on the checklist$note"
    fi
  fi

  gated=$(jq -r 'select(.startGate != null) | "\(.name): \(.startGate.message)"' \
    "$tmp/fleet.jsonl" | paste -sd ';' - | sed 's/;/; /g')
  if [ -z "$gated" ]; then
    pass start-gate "no machine is set aside for failing runner starts"
  else
    fail start-gate "$gated (see the machine page; fix the machine or pause it before the dry run)"
  fi

  failing=$(jq -r --argjson max "$MAX_FAILED" '
    select((.failedPlacementsLastHour // 0) > $max)
    | "\(.name): \(.failedPlacementsLastHour) in the last hour (\(
        [(.recentPlacementFailures // [])[] | "\(.reason): \(.message)"] | join(", ")))"' \
    "$tmp/fleet.jsonl" | paste -sd ';' - | sed 's/;/; /g')
  if [ -z "$failing" ]; then
    pass placement-failures "no machine has more than $MAX_FAILED failed placements in the last hour"
  else
    fail placement-failures "$failing"
  fi
else
  for check in heartbeats start-gate placement-failures; do
    fail "$check" "not checked: the machine list could not be read"
  done
fi

# --- The macOS agent's machine ---------------------------------------------------

# Every release builds the macOS agent on a self-hosted macOS ARM64 runner
# (docs/macos-runners.md), and the release does not publish without it. A Mac
# that cannot take that job now fails here, before step 1, rather than as a
# release that waits for a runner and is cancelled. PREFLIGHT_IGNORE_HOSTS does
# not excuse it: the release needs the Mac whether or not it is ignored above.
macos_hint="the release builds its macOS agent on one (docs/macos-runners.md); bring one online, or opt out with the repository variable RELEASE_MACOS=false and PREFLIGHT_RELEASE_MACOS=false, which publishes the release without a macOS agent and must be written in its checklist"
if [ "$RELEASE_MACOS" = false ]; then
  skip macos-machine "PREFLIGHT_RELEASE_MACOS=false: this release publishes without a macOS agent; check that the repository variable RELEASE_MACOS is false and write it in the release's checklist"
elif [ "$hosts_ready" != 1 ]; then
  fail macos-machine "not checked: the machine list could not be read"
elif [ -n "$restricted" ]; then
  fail macos-machine "not checked: the token's owner sees only a restricted view of some machines"
else
  macs='select(.hostOs == "macos" and .architecture == "arm64")'
  ready=$(jq -r "$macs"' | select(.enrollment == "approved" and .lifecycle == "active"
      and .heartbeatFreshness == "fresh" and .startGate == null) | .name' \
    "$tmp/hosts.jsonl" | paste -sd ',' - | sed 's/,/, /g')
  # Why each other Mac cannot take the job; retired ones are left out.
  unready=$(jq -r "$macs"' | select(.lifecycle != "retired") | select(
        .enrollment != "approved" or .lifecycle != "active"
        or .heartbeatFreshness != "fresh" or .startGate != null)
      | "\(.name) (\([
          (if .enrollment != "approved" then "enrollment \(.enrollment)" else empty end),
          (if .lifecycle != "active" then .lifecycle else empty end),
          (if .heartbeatFreshness != "fresh" then "heartbeat \(.heartbeatFreshness)" else empty end),
          (if .startGate != null then "set aside for failing runner starts" else empty end)
        ] | join(", ")))"' "$tmp/hosts.jsonl" | paste -sd ';' - | sed 's/;/; /g')
  if [ -n "$ready" ]; then
    pass macos-machine "macOS ARM64 machine ready for the macOS agent build: $ready"
  elif [ -n "$unready" ]; then
    fail macos-machine "no macOS ARM64 machine is approved, active and reporting in: $unready; $macos_hint"
  else
    fail macos-machine "no macOS ARM64 machine is enrolled in GridOps; $macos_hint"
  fi
fi

# --- Upgrading system-service agents -------------------------------------------

# Run the upgrade command: sudo, no password, and nothing else. "sudo -n -l
# <command>" answers when the user may run it, but it also answers when that
# rule wants a password and another rule does not, so the listing must show
# NOPASSWD in force where the command appears.
nopasswd_for() { # <command> < listing
  awk -v cmd="$1" '
    {
      line = $0
      if (line ~ /NOPASSWD:[ \t]*ALL[ \t]*$/) { found = 1; next }
      start = 1
      while ((i = index(substr(line, start), cmd)) > 0) {
        at = start + i - 1
        after = substr(line, at + length(cmd), 1)
        if (after == "" || after == " " || after == ",") {
          before = substr(line, 1, at - 1)
          tag = ""
          while ((j = index(before, "PASSWD:")) > 0) {
            tag = (j > 2 && substr(before, j - 2, 2) == "NO") ? "NOPASSWD" : "PASSWD"
            before = substr(before, j + 7)
          }
          if (tag == "NOPASSWD") found = 1
        }
        start = at + length(cmd)
      }
    }
    END { exit found ? 0 : 1 }'
}

ssh_to() { # <target> <remote command>: never prompts, never reads stdin
  ssh -n -o BatchMode=yes -o ConnectTimeout=10 "$@"
}

if [ -z "$UPGRADE_HOSTS" ]; then
  skip upgrade-sudo "PREFLIGHT_UPGRADE_HOSTS is not set; list the ssh targets of machines that run the agent as a system service"
elif [ "$upgrade_ready" = 1 ]; then
  for target in $UPGRADE_HOSTS; do
    if ! is_plain "$target"; then
      fail "upgrade-sudo $target" "not a plain ssh target"
      continue
    fi
    if ssh_to "$target" "sudo -n -l $UPGRADE_COMMAND && sudo -n -l" > "$tmp/sudo" 2> "$tmp/sudo.err"; then
      if nopasswd_for "$UPGRADE_COMMAND" < "$tmp/sudo"; then
        pass "upgrade-sudo $target" "sudo runs $UPGRADE_COMMAND without a password"
      else
        fail "upgrade-sudo $target" "$UPGRADE_COMMAND is allowed only with a password; the sudo rule needs NOPASSWD"
      fi
    else
      code=$?
      if [ "$code" = 255 ]; then
        fail "upgrade-sudo $target" "ssh failed: $(head -n 1 "$tmp/sudo.err")"
      else
        fail "upgrade-sudo $target" "sudo will not run $UPGRADE_COMMAND without a password ($(head -n 1 "$tmp/sudo.err"))"
      fi
    fi
  done
fi

# --- Disk -----------------------------------------------------------------------

if [ -z "$DATA_PATH" ]; then
  skip disk "PREFLIGHT_DATA_PATH is not set; point it at Docker's root directory on the deploy host (docker info --format '{{.DockerRootDir}}')"
elif [ -z "$MIN_FREE_GB" ]; then
  :
elif ! is_plain "$DATA_PATH"; then
  fail disk "PREFLIGHT_DATA_PATH must be a plain path"
else
  where="$DATA_PATH"
  if [ -n "$DEPLOY_HOST" ]; then
    where="$DEPLOY_HOST:$DATA_PATH"
    if is_plain "$DEPLOY_HOST"; then
      ssh_to "$DEPLOY_HOST" "df -Pk $DATA_PATH" > "$tmp/df" 2> "$tmp/df.err" || : > "$tmp/df"
    else
      echo "PREFLIGHT_DEPLOY_HOST is not a plain ssh target" > "$tmp/df.err"
      : > "$tmp/df"
    fi
  else
    df -Pk "$DATA_PATH" > "$tmp/df" 2> "$tmp/df.err" || : > "$tmp/df"
  fi
  free_kb=$(awk 'NR == 2 { print $4 }' "$tmp/df")
  if ! is_number "$free_kb"; then
    fail disk "cannot read free space at $where ($(head -n 1 "$tmp/df.err"))"
  else
    free_gb=$((free_kb / 1048576))
    if [ "$free_kb" -lt $((MIN_FREE_GB * 1048576)) ]; then
      fail disk "$where has $free_gb GB free, needs at least $MIN_FREE_GB GB for the new images and a database backup"
    else
      pass disk "$where has $free_gb GB free (at least $MIN_FREE_GB GB needed)"
    fi
  fi
fi

# --- Summary --------------------------------------------------------------------

if [ -n "$failed" ]; then
  count=$(printf '%s' "$failed" | grep -c '^  - ')
  echo
  echo "$(plural "$count" check) failed; fix these before step 1:"
  printf '%s' "$failed"
  exit 1
fi
echo
echo "All checks passed."
