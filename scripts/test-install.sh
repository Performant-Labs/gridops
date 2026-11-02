#!/bin/sh
# Tests install.sh's upgrade of an agent that runs as a system service, with
# stand-ins for systemctl, id and curl. Needs no root and touches nothing
# outside a temp directory.
#
#   sh scripts/test-install.sh
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
stubs="$tmp/stubs"
mkdir -p "$stubs" "$tmp/usr/bin" "$tmp/release"

# The "release": an agent that reports a version, and its checksums.
printf '#!/bin/sh\necho "gridops-host-agent 9.9.9"\n' > "$tmp/release/gridops-host-agent-linux-x64"
printf '#!/bin/sh\necho "gridops-host-agent 9.9.9"\n' > "$tmp/release/gridops-host-agent-linux-arm64"
(cd "$tmp/release" && sha256sum gridops-host-agent-linux-* > SHA256SUMS)

cat > "$stubs/curl" <<STUB
#!/bin/sh
# curl <options> -o <dest> <url>; "curl --help all" lists the options this curl has.
case "\$1" in
  --help) [ -n "\${FAKE_OLD_CURL:-}" ] || echo "     --retry-all-errors  Retry all errors"; exit 0 ;;
esac
echo "curl \$*" >> "$tmp/curl-calls"
while [ \$# -gt 0 ]; do [ "\$1" = -o ] && dest="\$2"; url="\$1"; shift; done
cp "$tmp/release/\$(basename "\$url")" "\$dest"
STUB
cat > "$stubs/systemctl" <<STUB
#!/bin/sh
echo "systemctl \$*" >> "$tmp/calls"
case "\$*" in
  *"-p FragmentPath"*) echo "\${FAKE_UNIT:-}" ;;
  *"-p ExecStart"*) echo "{ path=$tmp/usr/bin/gridops-host-agent ; argv[]=$tmp/usr/bin/gridops-host-agent run ; ignore_errors=no }" ;;
  "is-active "*) echo "\${FAKE_ACTIVE:-active}" ;;
esac
STUB
cat > "$stubs/id" <<STUB
#!/bin/sh
echo "\${FAKE_UID:-0}"
STUB
cat > "$stubs/sleep" <<STUB
#!/bin/sh
STUB
chmod +x "$stubs"/*

printf '#!/bin/sh\necho "gridops-host-agent 0.1.3"\n' > "$tmp/usr/bin/gridops-host-agent"
chmod 755 "$tmp/usr/bin/gridops-host-agent"

failures=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; failures=$((failures + 1)); fi; }
run() { env PATH="$stubs:$PATH" HOME="$tmp/home" GRIDOPS_VERSION=9.9.9 "$@" sh "$here/install.sh"; }

# 1. A system install, run as root, is upgraded in place and restarted.
: > "$tmp/calls"
out="$(run FAKE_UNIT=/etc/systemd/system/gridops-host-agent.service 2>&1)" || { echo "$out"; false; }
check "replaces the binary at the unit's path" '"$tmp/usr/bin/gridops-host-agent" --version | grep -q 9.9.9'
check "keeps the old binary to roll back to" '"$tmp/usr/bin/gridops-host-agent.previous" --version | grep -q 0.1.3'
check "restarts the service" 'grep -q "systemctl restart gridops-host-agent" "$tmp/calls"'
check "leaves no per-user install behind" '[ ! -e "$tmp/home" ]'

# 2. Not root: refuses, and changes nothing.
: > "$tmp/calls"
printf '#!/bin/sh\necho "gridops-host-agent 0.1.3"\n' > "$tmp/usr/bin/gridops-host-agent"
if out="$(run FAKE_UNIT=/etc/systemd/system/gridops-host-agent.service FAKE_UID=1001 2>&1)"; then ok=0; else ok=1; fi
check "refuses without root" '[ $ok = 1 ] && echo "$out" | grep -q "as root"'
check "leaves the binary alone without root" '"$tmp/usr/bin/gridops-host-agent" --version | grep -q 0.1.3'
check "does not restart without root" '! grep -q restart "$tmp/calls"'

# 3. The service doesn't come back: fails and says how to roll back.
if out="$(run FAKE_UNIT=/etc/systemd/system/gridops-host-agent.service FAKE_ACTIVE=failed 2>&1)"; then ok=0; else ok=1; fi
check "fails when the service does not come back" '[ $ok = 1 ] && echo "$out" | grep -q "roll back"'

# 4. NO_SERVICE replaces the binary only.
: > "$tmp/calls"
run FAKE_UNIT=/etc/systemd/system/gridops-host-agent.service GRIDOPS_NO_SERVICE=1 >/dev/null 2>&1
check "GRIDOPS_NO_SERVICE does not restart" '! grep -q restart "$tmp/calls"'

# 5. No system unit: the usual per-user install runs (binary only here).
rm -f "$tmp/usr/bin/gridops-host-agent.previous"
run FAKE_UNIT= GRIDOPS_NO_ENROLL=1 GRIDOPS_NO_SERVICE=1 GRIDOPS_INSTALL_DIR="$tmp/user-bin" >/dev/null 2>&1
check "installs per user when there is no system unit" '"$tmp/user-bin/gridops-host-agent" --version | grep -q 9.9.9'
check "does not touch the system binary then" '! [ -e "$tmp/usr/bin/gridops-host-agent.previous" ]'

# 6. The download survives a flaky network: it retries every error and bounds
# a stalled connection or transfer. A curl without --retry-all-errors (older
# than 7.71) still works, just without that flag.
: > "$tmp/curl-calls"
run FAKE_UNIT= GRIDOPS_NO_ENROLL=1 GRIDOPS_NO_SERVICE=1 GRIDOPS_INSTALL_DIR="$tmp/user-bin2" >/dev/null 2>&1
check "curl retries every error" 'grep -q -e "--retry 5 --retry-all-errors" "$tmp/curl-calls"'
check "curl bounds a connection attempt" 'grep -q -e "--connect-timeout 15" "$tmp/curl-calls"'
check "curl aborts a stalled transfer" 'grep -q -e "--speed-limit 1000 --speed-time 30" "$tmp/curl-calls"'
: > "$tmp/curl-calls"
run FAKE_OLD_CURL=1 FAKE_UNIT= GRIDOPS_NO_ENROLL=1 GRIDOPS_NO_SERVICE=1 GRIDOPS_INSTALL_DIR="$tmp/user-bin3" >/dev/null 2>&1
check "an older curl still installs" '"$tmp/user-bin3/gridops-host-agent" --version | grep -q 9.9.9'
check "an older curl is not given --retry-all-errors" '[ -s "$tmp/curl-calls" ] && ! grep -q -e "--retry-all-errors" "$tmp/curl-calls" && grep -q -e "--retry 5" "$tmp/curl-calls"'

[ "$failures" = 0 ] || { echo "$failures failed"; exit 1; }
echo "all passed"
