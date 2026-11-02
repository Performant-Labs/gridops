#!/bin/sh
# Tests release-preflight.sh with stand-ins for curl, ssh and df. Needs no
# network, no GridOps and no root, and touches nothing outside a temp
# directory.
#
#   sh scripts/test-release-preflight.sh
#
# Checks are strings run by eval after each run, as in test-install.sh.
# shellcheck disable=SC2016,SC2034
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
stubs="$tmp/stubs"
mkdir -p "$stubs"

token="gat_TestTokenThatMustNeverBePrinted0123456789"

# curl: a tiny GridOps. It reads the Authorization header from its config
# on stdin (-K -), the way the script sends it, and answers by route.
#   FAKE_SCOPES         scopes the token holds (default "fleet:read fleet:operate")
#   FAKE_HOSTS          file with the first page of GET /hosts
#   FAKE_HOSTS_PAGE2    file with the page after cursor "page-2"
#   FAKE_HEALTH_DOWN=1  the control plane does not answer
#   FAKE_OLD_CURL=1     a curl without --retry-all-errors
cat > "$stubs/curl" <<STUB
#!/bin/sh
case "\$1" in
  --help) [ -n "\${FAKE_OLD_CURL:-}" ] || echo "     --retry-all-errors  Retry all errors"; exit 0 ;;
esac
echo "curl \$*" >> "$tmp/curl-calls"
method=GET; url=""; config=""; body=""; out=""; fmt=""
while [ \$# -gt 0 ]; do
  case "\$1" in
    -X) method="\$2"; shift ;;
    -K) [ "\$2" = - ] && config="\$(cat)"; shift ;;
    -d|--data|--data-urlencode) body="\$body \$2"; shift ;;
    -o) out="\$2"; shift ;;
    -w) fmt="\$2"; shift ;;
    -H|--connect-timeout|--max-time|--retry) shift ;;
    -*) ;;
    *) url="\$1" ;;
  esac
  shift
done
printf '%s\n' "\$config" >> "$tmp/curl-config"
scopes="\${FAKE_SCOPES-fleet:read fleet:operate}"
# reply <status> <body>: like curl -o <file> -w '%{http_code}'.
reply() {
  if [ -n "\$out" ]; then printf '%s' "\$2" > "\$out"; else printf '%s' "\$2"; fi
  [ -z "\$fmt" ] || printf '%s' "\$1"
  exit 0
}
case "\$url" in
  */api/health)
    [ -z "\${FAKE_HEALTH_DOWN:-}" ] || { echo "curl: (7) Failed to connect" >&2; exit 7; }
    reply 200 '{"status":"ok","version":"9.9.9","buildNumber":412,"commit":"abc1234"}' ;;
esac
case "\$config" in
  *"Authorization: Bearer $token"*) ;;
  *) reply 401 '{"code":"unauthenticated","message":"Authentication required","requestId":"r"}' ;;
esac
need() {
  case " \$scopes " in
    *" \$1 "*) ;;
    *) reply 403 '{"code":"forbidden","message":"Fleet action is not authorized","requestId":"r","details":{"kind":"scope","requiredScope":"'"\$1"'"}}' ;;
  esac
}
case "\$method \$url" in
  "GET "*/api/v1/fleet/hosts*)
    need fleet:read
    case "\$body" in
      *cursor=page-2*) reply 200 "\$(cat "\$FAKE_HOSTS_PAGE2")" ;;
      *) reply 200 "\$(cat "\$FAKE_HOSTS")" ;;
    esac ;;
  "POST "*/api/v1/fleet/hosts/*/lifecycle)
    need fleet:operate
    # The body is never a valid lifecycle change: the route only gets as far
    # as reading it, so a token with the scope gets invalid_request.
    case "\$body" in
      *lifecycle*) echo "MUTATION ATTEMPTED" >> "$tmp/mutations"; reply 200 '{}' ;;
    esac
    reply 400 '{"code":"invalid_request","message":"Invalid request","requestId":"r"}' ;;
esac
reply 404 '{"code":"not_found","message":"Not found","requestId":"r"}'
STUB

# ssh: the target names how its sudo is set up.
cat > "$stubs/ssh" <<'STUB'
#!/bin/sh
echo "ssh $*" >> "$TEST_TMP/ssh-calls"
while [ $# -gt 0 ]; do
  case "$1" in
    -o) shift ;;
    -*) ;;
    *) target="$1"; shift; break ;;
  esac
  shift
done
remote="$*"
cmd=/usr/local/sbin/gridops-agent-upgrade
listing() {
  echo "Matching Defaults entries for releaser on $target:"
  echo "    env_reset, secure_path=/usr/sbin\\:/usr/bin\\:/sbin\\:/bin"
  echo
  echo "User releaser may run the following commands on $target:"
  echo "$1"
}
case "$remote" in
  "df -Pk "*)
    echo "Filesystem 1024-blocks Used Available Capacity Mounted on"
    echo "/dev/sda1 104857600 52428800 ${FAKE_REMOTE_FREE_KB:-52428800} 50% /"
    exit 0 ;;
esac
case "$target" in
  ok.example)
    case "$remote" in *"sudo -n -l $cmd"*) ;; *) exit 1 ;; esac
    echo "$cmd"; listing "    (root) NOPASSWD: $cmd \"\"" ;;
  all.example)
    echo "$cmd"; listing "    (ALL : ALL) NOPASSWD: ALL" ;;
  password.example)
    echo "sudo: a password is required" >&2; exit 1 ;;
  other-rule.example)
    # Allowed, but only with a password: the NOPASSWD rule is for something
    # else, which is enough for "sudo -n -l <command>" to answer.
    echo "$cmd"; listing "    (ALL : ALL) ALL
    (root) NOPASSWD: /usr/bin/systemctl restart nginx" ;;
  mixed.example)
    echo "$cmd"; listing "    (root) NOPASSWD: /usr/bin/true, PASSWD: $cmd" ;;
  lookalike.example)
    echo "$cmd"; listing "    (root) NOPASSWD: $cmd-old" ;;
  down.example)
    echo "ssh: connect to host down.example port 22: Connection refused" >&2; exit 255 ;;
  *) exit 255 ;;
esac
STUB

# df -Pk <path>: FAKE_FREE_KB available, or fail for a missing path.
cat > "$stubs/df" <<'STUB'
#!/bin/sh
echo "df $*" >> "$TEST_TMP/df-calls"
for last in "$@"; do :; done
[ "$last" != /missing ] || { echo "df: /missing: No such file or directory" >&2; exit 1; }
echo "Filesystem 1024-blocks Used Available Capacity Mounted on"
echo "/dev/sda1 104857600 52428800 ${FAKE_FREE_KB:-52428800} 50% /var/lib/docker"
STUB
chmod +x "$stubs"/*

# Machines, as GET /api/v1/fleet/hosts lists them (only the fields the
# preflight reads, plus a few it must ignore).
host() { # id name enrollment lifecycle freshness failed_last_hour start_gate_json [os] [arch]
  printf '{"projection":"host","id":"%s","name":"%s","hostOs":"%s","architecture":"%s","enrollment":"%s","lifecycle":"%s","heartbeatFreshness":"%s","failedPlacementsLastHour":%s,"recentPlacementFailures":%s,"startGate":%s,"agentVersion":"0.1.4"}' \
    "$1" "$2" "${9:-linux}" "${10:-x64}" "$3" "$4" "$5" "$6" "$7" "$8"
}
page() { # next_cursor items...
  next=$1; shift
  items=""
  for item in "$@"; do items="${items:+$items,}$item"; done
  if [ "$next" = null ]; then next=null; else next="\"$next\""; fi
  printf '{"schemaVersion":1,"readAt":"2026-11-02T20:00:00.000Z","items":[%s],"nextCursor":%s}' "$items" "$next"
}
no_failures='[]'
healthy_a=$(host h-a build-a approved active fresh 0 "$no_failures" null)
healthy_b=$(host h-b build-b approved active fresh 0 "$no_failures" null)
retired=$(host h-r old-box approved retired offline 0 "$no_failures" null)
pending=$(host h-p new-box pending paused unknown 0 "$no_failures" null)
stale=$(host h-s flaky-box approved active stale 0 "$no_failures" null)
gated=$(host h-g gated-box approved active fresh 3 '[{"reason":"start_deadline_expired","message":"Runner did not start in time","count":3,"latestAt":"2026-11-02T19:58:00.000Z","latestPlacementId":"p"}]' '{"failures":3,"windowMinutes":15,"since":"2026-11-02T19:45:00.000Z","until":"2026-11-02T20:00:00.000Z","message":"Not accepting new runners: 3 start failures in the last 15 minutes."}')
failing=$(host h-f failing-box approved active fresh 2 '[{"reason":"agent_reported","message":"not_registered","count":2,"latestAt":"2026-11-02T19:58:00.000Z","latestPlacementId":"p"}]' null)
# The macOS ARM64 machine the release builds the macOS agent on, and ones
# that cannot: paused, not reporting, set aside, waiting for approval, x64.
mac=$(host h-m mac-box approved active fresh 0 "$no_failures" null macos arm64)
mac_paused=$(host h-mp mac-paused approved paused fresh 0 "$no_failures" null macos arm64)
mac_stale=$(host h-ms mac-stale approved active stale 0 "$no_failures" null macos arm64)
mac_gated=$(host h-mg mac-gated approved active fresh 0 "$no_failures" '{"failures":3,"windowMinutes":15,"since":"2026-11-02T19:45:00.000Z","until":"2026-11-02T20:00:00.000Z","message":"Not accepting new runners: 3 start failures in the last 15 minutes."}' macos arm64)
mac_pending=$(host h-mn mac-new pending paused unknown 0 "$no_failures" null macos arm64)
mac_retired=$(host h-mr mac-old approved retired offline 0 "$no_failures" null macos arm64)
mac_intel=$(host h-mi mac-intel approved active fresh 0 "$no_failures" null macos x64)
linux_arm=$(host h-la linux-arm approved active fresh 0 "$no_failures" null linux arm64)

page null "$healthy_a" "$retired" "$pending" "$mac" > "$tmp/hosts-healthy"
page page-2 "$healthy_a" > "$tmp/hosts-page1"
page null "$stale" > "$tmp/hosts-page2"
page null "$healthy_a" "$stale" "$mac" > "$tmp/hosts-stale"
page null "$healthy_b" "$gated" > "$tmp/hosts-gated"
page null "$healthy_a" "$failing" "$mac" > "$tmp/hosts-failing"
page null "$healthy_a" > "$tmp/hosts-no-mac"
page null "$healthy_a" "$mac_paused" "$mac_stale" "$mac_gated" > "$tmp/hosts-mac-unready"
page null "$healthy_a" "$mac_pending" "$mac_retired" "$mac_intel" "$linux_arm" > "$tmp/hosts-mac-wrong"
page null "$healthy_a" "$mac_paused" "$mac" > "$tmp/hosts-mac-one-ready"
page page-2 "$healthy_a" > "$tmp/hosts-mac-page1"
page null "$mac" > "$tmp/hosts-mac-page2"
page null > "$tmp/hosts-none"

# An expiry <seconds> from now, as Settings -> API tokens shows it (UTC).
expiry() { date -u -d "@$(( $(date +%s) + $1 ))" +%Y-%m-%dT%H:%M:%SZ; }

failures=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; failures=$((failures + 1)); fi; }
# run VAR=value... : the preflight with the stubs first on PATH and only the
# settings given; everything else unset. Sets $out and $status.
run() {
  : > "$tmp/curl-calls"; : > "$tmp/ssh-calls"; : > "$tmp/df-calls"; : > "$tmp/curl-config"
  if out="$(env -i PATH="$stubs:$PATH" HOME="$tmp/home" TEST_TMP="$tmp" \
    FAKE_HOSTS="$tmp/hosts-healthy" \
    GRIDOPS_URL=https://gridops.example.com GRIDOPS_TOKEN="$token" \
    "$@" sh "$here/release-preflight.sh" 2>&1)"; then status=0; else status=$?; fi
}
line() { printf '%s\n' "$out" | grep -q -e "$1"; }
show() { printf '%s\n' "$out" | sed 's/^/     | /'; }

# 1. Everything in order: every check passes and it exits 0.
run PREFLIGHT_UPGRADE_HOSTS="ok.example all.example" PREFLIGHT_DATA_PATH=/var/lib/docker \
  PREFLIGHT_TOKEN_EXPIRES_AT="$(expiry 172800)"
check "all good: exits 0" '[ $status = 0 ]' || show
check "all good: control plane answers" 'line "^PASS  *control-plane"'
check "all good: token reads the fleet" 'line "^PASS  *token-read"'
check "all good: token can operate" 'line "^PASS  *token-operate"'
check "all good: token expiry in range" 'line "^PASS  *token-expiry"'
check "all good: heartbeats fresh" 'line "^PASS  *heartbeats"'
check "all good: no start gate" 'line "^PASS  *start-gate"'
check "all good: no placement failures" 'line "^PASS  *placement-failures"'
check "all good: a macOS ARM64 machine is ready" 'line "^PASS  *macos-machine.*mac-box"'
check "all good: sudo rule on ok.example" 'line "^PASS  *upgrade-sudo ok.example"'
check "all good: sudo rule on all.example" 'line "^PASS  *upgrade-sudo all.example"'
check "all good: disk" 'line "^PASS  *disk"'
check "all good: no FAIL line" '! line "^FAIL"'
check "retired and pending machines are not counted" 'line "2 machines" && ! line "old-box" && ! line "new-box"'
check "the token is never printed" '! printf "%s" "$out" | grep -q "$token"'
check "the token is never on a command line" '! grep -q "$token" "$tmp/curl-calls"'
check "the token reaches curl through its config on stdin" 'grep -q "Authorization: Bearer $token" "$tmp/curl-config"'
check "the operate probe changes nothing" '[ ! -e "$tmp/mutations" ]'
check "curl retries and is bounded in time" 'grep -q -e "--retry 3 --retry-all-errors" "$tmp/curl-calls" && grep -q -e "--connect-timeout 10" "$tmp/curl-calls" && grep -q -e "--max-time 30" "$tmp/curl-calls"'
check "ssh never prompts" 'grep -q "BatchMode=yes" "$tmp/ssh-calls" && grep -q "ConnectTimeout=" "$tmp/ssh-calls"'
check "ssh lists the upgrade command with sudo -n -l" 'grep -q "sudo -n -l /usr/local/sbin/gridops-agent-upgrade" "$tmp/ssh-calls"'

# 2. Nothing optional set: the optional checks say they were skipped, and
# the expiry says why the API cannot tell.
run
check "optional unset: exits 0" '[ $status = 0 ]' || show
check "optional unset: expiry skipped, saying the API cannot read it" 'line "^SKIP  *token-expiry.*cannot" && line "PREFLIGHT_TOKEN_EXPIRES_AT"'
check "optional unset: sudo check skipped" 'line "^SKIP  *upgrade-sudo"'
check "optional unset: disk skipped" 'line "^SKIP  *disk"'
check "optional unset: no ssh or df" '[ ! -s "$tmp/ssh-calls" ] && [ ! -s "$tmp/df-calls" ]'

# 3. Missing settings fail, and the rest still runs.
run GRIDOPS_URL= GRIDOPS_TOKEN= PREFLIGHT_DATA_PATH=/var/lib/docker
check "no URL or token: exits non-zero" '[ $status != 0 ]'
check "no URL: says so" 'line "^FAIL  *config.*GRIDOPS_URL"'
check "no token: says so" 'line "^FAIL  *config.*GRIDOPS_TOKEN"'
check "no URL: disk still checked" 'line "^PASS  *disk"'
run GRIDOPS_URL=http://gridops.example.com
check "plain http is refused" '[ $status != 0 ] && line "^FAIL  *config.*https"'
run GRIDOPS_TOKEN= GRIDOPS_API_TOKEN="$token"
check "GRIDOPS_API_TOKEN is accepted too" '[ $status = 0 ] && line "^PASS  *token-operate"' || show

# 4. Token problems.
run GRIDOPS_TOKEN=gat_wrong
check "bad token: exits non-zero" '[ $status != 0 ]'
check "bad token: read fails as unauthenticated" 'line "^FAIL  *token-read.*unauthenticated"'
check "bad token: operate fails" 'line "^FAIL  *token-operate"'
check "bad token: machine checks fail, not pass" 'line "^FAIL  *heartbeats" && ! line "^PASS  *heartbeats"'
run FAKE_SCOPES="fleet:read"
check "read-only token: exits non-zero" '[ $status != 0 ]'
check "read-only token: operate fails naming fleet:operate" 'line "^FAIL  *token-operate.*fleet:operate"'
check "read-only token: read still passes" 'line "^PASS  *token-read"'
check "read-only token: the probe changes nothing" '[ ! -e "$tmp/mutations" ]'
run FAKE_SCOPES="fleet:operate"
check "operate-only token: read fails naming fleet:read" '[ $status != 0 ] && line "^FAIL  *token-read.*fleet:read"'
run FAKE_HEALTH_DOWN=1
check "control plane down: fails" '[ $status != 0 ] && line "^FAIL  *control-plane"'

# 5. Expiry, when the operator supplies it.
run PREFLIGHT_TOKEN_EXPIRES_AT="$(expiry 7200)"
check "token about to expire: fails" '[ $status != 0 ] && line "^FAIL  *token-expiry"'
run PREFLIGHT_TOKEN_EXPIRES_AT="$(expiry -3600)"
check "token expired: fails" '[ $status != 0 ] && line "^FAIL  *token-expiry.*expired"'
run PREFLIGHT_TOKEN_EXPIRES_AT="$(expiry 7776000 | cut -c1-10)"
check "token living too long: fails" '[ $status != 0 ] && line "^FAIL  *token-expiry.*short"'
run PREFLIGHT_TOKEN_EXPIRES_AT="next tuesday-ish"
check "unreadable expiry: fails" '[ $status != 0 ] && line "^FAIL  *token-expiry"'

# 6. Machines.
run FAKE_HOSTS="$tmp/hosts-stale"
check "stale heartbeat: fails naming the machine" '[ $status != 0 ] && line "^FAIL  *heartbeats.*flaky-box (stale)"'
run FAKE_HOSTS="$tmp/hosts-stale" PREFLIGHT_IGNORE_HOSTS="flaky-box"
check "an ignored machine is skipped, not failed" '[ $status = 0 ] && line "^PASS  *heartbeats" && line "ignored.*flaky-box"' || show
run FAKE_HOSTS="$tmp/hosts-page1" FAKE_HOSTS_PAGE2="$tmp/hosts-page2"
check "reads every page of machines" '[ $status != 0 ] && line "^FAIL  *heartbeats.*flaky-box" && grep -q "cursor=page-2" "$tmp/curl-calls"'
run FAKE_HOSTS="$tmp/hosts-gated"
check "start gate: fails naming the machine and why" '[ $status != 0 ] && line "^FAIL  *start-gate.*gated-box.*3 start failures"'
check "start gate: the failures fail too" 'line "^FAIL  *placement-failures.*gated-box"'
check "start gate: the healthy machine is not named" '! line "^FAIL.*build-b"'
run FAKE_HOSTS="$tmp/hosts-failing"
check "placement failures: fails naming machine and reason" '[ $status != 0 ] && line "^FAIL  *placement-failures.*failing-box.*agent_reported"'
check "placement failures: start gate still passes" 'line "^PASS  *start-gate"'
run FAKE_HOSTS="$tmp/hosts-failing" PREFLIGHT_MAX_FAILED_PLACEMENTS=2
check "placement failures within the allowance pass" '[ $status = 0 ]' || show
run FAKE_HOSTS="$tmp/hosts-none" PREFLIGHT_RELEASE_MACOS=false
check "no machines: passes and says so" '[ $status = 0 ] && line "^PASS  *heartbeats.*0 machines"' || show
run FAKE_HOSTS="$tmp/hosts-none"
check "no machines: the macOS check fails" '[ $status != 0 ] && line "^FAIL  *macos-machine"' || show

# 7. The upgrade command on system-service machines.
run PREFLIGHT_UPGRADE_HOSTS="password.example"
check "password needed: fails" '[ $status != 0 ] && line "^FAIL  *upgrade-sudo password.example"'
run PREFLIGHT_UPGRADE_HOSTS="other-rule.example"
check "allowed only with a password: fails" '[ $status != 0 ] && line "^FAIL  *upgrade-sudo other-rule.example.*NOPASSWD"'
run PREFLIGHT_UPGRADE_HOSTS="mixed.example"
check "PASSWD after NOPASSWD: fails" '[ $status != 0 ] && line "^FAIL  *upgrade-sudo mixed.example"'
run PREFLIGHT_UPGRADE_HOSTS="lookalike.example"
check "a rule for a longer path does not count" '[ $status != 0 ] && line "^FAIL  *upgrade-sudo lookalike.example"'
run PREFLIGHT_UPGRADE_HOSTS="down.example"
check "unreachable machine: fails saying ssh" '[ $status != 0 ] && line "^FAIL  *upgrade-sudo down.example.*ssh"'
run PREFLIGHT_UPGRADE_HOSTS="ok.example down.example password.example"
check "each machine gets its own line" 'line "^PASS  *upgrade-sudo ok.example" && line "^FAIL  *upgrade-sudo down.example" && line "^FAIL  *upgrade-sudo password.example"'
run PREFLIGHT_UPGRADE_HOSTS="ok.example" PREFLIGHT_UPGRADE_COMMAND='/usr/local/sbin/x;reboot'
check "an unsafe upgrade command is refused before ssh" '[ $status != 0 ] && line "^FAIL  *config.*PREFLIGHT_UPGRADE_COMMAND" && [ ! -s "$tmp/ssh-calls" ]'

# 8. Free disk.
run PREFLIGHT_DATA_PATH=/var/lib/docker FAKE_FREE_KB=1048576
check "low disk: fails with the numbers" '[ $status != 0 ] && line "^FAIL  *disk.*1 GB free.*10 GB"'
run PREFLIGHT_DATA_PATH=/var/lib/docker FAKE_FREE_KB=1048576 PREFLIGHT_MIN_FREE_GB=1
check "a lower minimum is honoured" '[ $status = 0 ] && line "^PASS  *disk"' || show
run PREFLIGHT_DATA_PATH=/missing
check "missing path: fails" '[ $status != 0 ] && line "^FAIL  *disk"'
run PREFLIGHT_DATA_PATH=/var/lib/docker PREFLIGHT_DEPLOY_HOST=deploy.example FAKE_REMOTE_FREE_KB=2097152
check "disk on the deploy host over ssh" '[ $status != 0 ] && line "^FAIL  *disk.*2 GB free" && grep -q "deploy.example df -Pk /var/lib/docker" "$tmp/ssh-calls" && [ ! -s "$tmp/df-calls" ]'
run PREFLIGHT_DATA_PATH=/var/lib/docker PREFLIGHT_MIN_FREE_GB=ten
check "a non-number minimum is refused" '[ $status != 0 ] && line "^FAIL  *config.*PREFLIGHT_MIN_FREE_GB"'

# 7b. The macOS ARM64 machine every release builds the macOS agent on.
run FAKE_HOSTS="$tmp/hosts-no-mac"
check "no Mac: fails early" '[ $status != 0 ] && line "^FAIL  *macos-machine.*no macOS ARM64 machine"' || show
check "no Mac: says how to fix it or opt out" 'line "^FAIL  *macos-machine.*RELEASE_MACOS" && line "^FAIL  *macos-machine.*without a macOS agent"'
check "no Mac: the other checks still pass" 'line "^PASS  *heartbeats" && line "^PASS  *start-gate"'
check "no Mac: in the summary" 'line "^  - macos-machine"'
run FAKE_HOSTS="$tmp/hosts-mac-unready"
check "paused, stale or set-aside Macs: fails" '[ $status != 0 ] && line "^FAIL  *macos-machine"' || show
check "names why each Mac cannot build" 'line "^FAIL  *macos-machine.*mac-paused.*paused" && line "^FAIL  *macos-machine.*mac-stale.*stale" && line "^FAIL  *macos-machine.*mac-gated.*set aside"'
run FAKE_HOSTS="$tmp/hosts-mac-wrong"
check "pending, retired, Intel Macs and Linux ARM64 do not count" '[ $status != 0 ] && line "^FAIL  *macos-machine.*no macOS ARM64 machine"' || show
check "a Mac waiting for approval is pointed out" 'line "^FAIL  *macos-machine.*mac-new.*pending"'
run FAKE_HOSTS="$tmp/hosts-mac-one-ready"
check "one ready Mac is enough" 'line "^PASS  *macos-machine.*mac-box" && ! line "^FAIL  *macos-machine"' || show
run FAKE_HOSTS="$tmp/hosts-mac-page1" FAKE_HOSTS_PAGE2="$tmp/hosts-mac-page2"
check "a Mac on a later page is found" '[ $status = 0 ] && line "^PASS  *macos-machine.*mac-box"' || show
run PREFLIGHT_IGNORE_HOSTS="mac-box"
check "ignoring the Mac does not excuse it: the release still needs it" 'line "^PASS  *macos-machine.*mac-box"' || show
run FAKE_HOSTS="$tmp/hosts-no-mac" PREFLIGHT_RELEASE_MACOS=false
check "opt-out: skipped, passes, and says the release has no macOS agent" '[ $status = 0 ] && line "^SKIP  *macos-machine.*without a macOS agent" && line "^SKIP  *macos-machine.*checklist"' || show
run PREFLIGHT_RELEASE_MACOS=true
check "PREFLIGHT_RELEASE_MACOS=true is the default" '[ $status = 0 ] && line "^PASS  *macos-machine"' || show
run PREFLIGHT_RELEASE_MACOS=no
check "an unknown PREFLIGHT_RELEASE_MACOS is refused" '[ $status != 0 ] && line "^FAIL  *config.*PREFLIGHT_RELEASE_MACOS"'
run GRIDOPS_TOKEN=gat_wrong
check "machine list unreadable: the macOS check fails, not passes" 'line "^FAIL  *macos-machine.*not checked"'

# 9. The summary lists every failure, and nothing is cut short by set -e.
run GRIDOPS_TOKEN=gat_wrong PREFLIGHT_UPGRADE_HOSTS="password.example" PREFLIGHT_DATA_PATH=/missing
check "summary counts every failure" 'line "checks failed" && line "token-read" && line "upgrade-sudo password.example" && line "disk"'
check "summary is last" 'printf "%s\n" "$out" | tail -n 1 | grep -q -e "^  - disk"' || show

# 10. An older curl still works, without --retry-all-errors.
run FAKE_OLD_CURL=1
check "older curl: passes" '[ $status = 0 ]' || show
check "older curl: not given --retry-all-errors" '! grep -q -e "--retry-all-errors" "$tmp/curl-calls" && grep -q -e "--retry 3" "$tmp/curl-calls"'

[ "$failures" = 0 ] || { echo "$failures failed"; exit 1; }
echo "all passed"
