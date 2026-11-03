#!/bin/sh
# Tests the launchd plist that install-tart-agent.sh writes, on any Unix with
# zsh and python3: uname, stat, cargo, plutil and launchctl are stand-ins, so
# nothing is built, installed or started, and nothing outside a temp
# directory is touched. What it cannot show is launchd itself running the
# agent, which needs a Mac.
#
#   sh scripts/test-install-tart-agent.sh
#
# Checks are strings run by eval, so their variables expand when they run.
# shellcheck disable=SC2016,SC2034
set -eu

if ! command -v zsh >/dev/null 2>&1 || ! command -v python3 >/dev/null 2>&1; then
  echo "skipped: needs zsh and python3"
  exit 0
fi

here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# The installer finds the repository from its own path, so give it a copy.
repo="$tmp/repo"
mkdir -p "$repo/scripts" "$repo/deploy/launchd"
cp "$here/install-tart-agent.sh" "$repo/scripts/"
cp "$here/../deploy/launchd/dev.gridops.tart-agent.plist" "$repo/deploy/launchd/"

stubs="$tmp/stubs"
mkdir -p "$stubs"
cat > "$stubs/uname" <<'STUB'
#!/bin/sh
case "$1" in -s) echo Darwin ;; -m) echo arm64 ;; esac
STUB
# stat -f %u: every file is root's, as an installed softnet must be.
cat > "$stubs/stat" <<'STUB'
#!/bin/sh
echo 0
STUB
cat > "$stubs/cargo" <<'STUB'
#!/bin/sh
mkdir -p target/release && printf '#!/bin/sh\n' > target/release/gridops-tart-agent
STUB
# plutil -lint: the plist must parse.
cat > "$stubs/plutil" <<'STUB'
#!/bin/sh
shift
exec python3 -I -c 'import plistlib, sys; plistlib.load(open(sys.argv[1], "rb"))' "$1"
STUB
cat > "$stubs/launchctl" <<STUB
#!/bin/sh
echo "launchctl \$*" >> "$tmp/launchctl-calls"
STUB
chmod +x "$stubs"/*

# Executables at the given paths; softnet ones are root-owned SUID in the
# installer's eyes (stat is stubbed; the SUID bit is real).
fake_binary() {
  mkdir -p "$(dirname "$1")"
  printf '#!/bin/sh\n' > "$1"
  chmod 755 "$1"
  case "$1" in */softnet) chmod u+s "$1" ;; esac
}

failures=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; failures=$((failures + 1)); fi; }

# install <home> <PATH> [VAR=value ...]: runs the installer in a clean
# environment; the plist lands in <home>/Library/LaunchAgents.
install() {
  home="$1"
  search="$2"
  shift 2
  env -i HOME="$home" PATH="$stubs:$search:/usr/bin:/bin" \
    GRIDOPS_TART_AGENT_TOKEN="$(printf 't%.0s' $(seq 32))" "$@" \
    zsh "$repo/scripts/install-tart-agent.sh"
}

# plist_env <home> <key>: the value launchd will set for <key>.
plist_env() {
  python3 -I -c 'import plistlib, sys
with open(sys.argv[1], "rb") as f:
    print(plistlib.load(f)["EnvironmentVariables"].get(sys.argv[2], "<unset>"))' \
    "$1/Library/LaunchAgents/dev.gridops.tart-agent.plist" "$2"
}

# 1. The usual Homebrew install: Tart and Softnet in one directory, listed
# once, ahead of launchd's own directories.
brew="$tmp/homebrew/bin"
fake_binary "$brew/tart"
fake_binary "$brew/softnet"
out="$(install "$tmp/home1" "$brew" 2>&1)" || { echo "$out"; exit 1; }
check "the agent's PATH starts with Tart's directory, once" \
  '[ "$(plist_env "$tmp/home1" PATH)" = "$brew:/usr/bin:/bin:/usr/sbin:/sbin" ]'
check "the plist still names the Tart binary" \
  '[ "$(plist_env "$tmp/home1" GRIDOPS_TART_BINARY)" = "$brew/tart" ]'
check "the agent is restarted" 'grep -q "kickstart -k" "$tmp/launchctl-calls"'
# Defaults: two runners (macOS runs two VMs and GridOps shares them out) and
# a 60-second safety-net wait for a VM slot (pools wait in GridOps's queue).
check "GRIDOPS_TART_MAX_RUNNERS defaults to 2" \
  '[ "$(plist_env "$tmp/home1" GRIDOPS_TART_MAX_RUNNERS)" = 2 ]'
check "GRIDOPS_TART_VM_SLOT_WAIT_SECONDS defaults to 60" \
  '[ "$(plist_env "$tmp/home1" GRIDOPS_TART_VM_SLOT_WAIT_SECONDS)" = 60 ]'

# 2. Tart and Softnet in separate directories whose names hold characters
# special to sed (| & \) and to XML (& < >), and a space.
odd="$tmp/Tart & Co|x\\y <1>"
softnet_dir="$tmp/soft net&|\\"
fake_binary "$odd/tart"
fake_binary "$softnet_dir/softnet"
out="$(install "$tmp/home2" "$softnet_dir" GRIDOPS_TART_BINARY="$odd/tart" 2>&1)" || { echo "$out"; exit 1; }
check "Tart's then Softnet's directory, written exactly" \
  '[ "$(plist_env "$tmp/home2" PATH)" = "$odd:$softnet_dir:/usr/bin:/bin:/usr/sbin:/sbin" ]'
check "the Tart binary path is written exactly" \
  '[ "$(plist_env "$tmp/home2" GRIDOPS_TART_BINARY)" = "$odd/tart" ]'

# 3. A Tart already in launchd's directories, and NAT mode without Softnet:
# no directory twice, nothing for Softnet.
out="$(install "$tmp/home3" "$tmp/empty" GRIDOPS_TART_BINARY=/bin/sh GRIDOPS_TART_NETWORK_MODE=nat 2>&1)" || { echo "$out"; exit 1; }
check "a Tart in /bin adds no directory" \
  '[ "$(plist_env "$tmp/home3" PATH)" = "/bin:/usr/bin:/usr/sbin:/sbin" ]'

# 4. Softnet mode still refuses a missing Softnet, and writes no plist.
if out="$(install "$tmp/home4" "$tmp/empty" GRIDOPS_TART_BINARY="$odd/tart" 2>&1)"; then ok=0; else ok=1; fi
check "softnet mode without softnet is refused" \
  '[ $ok = 1 ] && echo "$out" | grep -q "softnet is not installed" && [ ! -e "$tmp/home4/Library/LaunchAgents/dev.gridops.tart-agent.plist" ]'

# 5. Values set when installing replace the defaults, up to the 600-second
# maximum wait; a longer wait is refused and writes no plist.
out="$(install "$tmp/home5" "$brew" GRIDOPS_TART_MAX_RUNNERS=1 GRIDOPS_TART_VM_SLOT_WAIT_SECONDS=600 2>&1)" || { echo "$out"; exit 1; }
check "a runner cap set when installing is kept" \
  '[ "$(plist_env "$tmp/home5" GRIDOPS_TART_MAX_RUNNERS)" = 1 ]'
check "the longest slot wait, 600 seconds, is accepted" \
  '[ "$(plist_env "$tmp/home5" GRIDOPS_TART_VM_SLOT_WAIT_SECONDS)" = 600 ]'
if out="$(install "$tmp/home6" "$brew" GRIDOPS_TART_VM_SLOT_WAIT_SECONDS=601 2>&1)"; then ok=0; else ok=1; fi
check "a slot wait above 600 seconds is refused" \
  '[ $ok = 1 ] && echo "$out" | grep -q "from 0 to 600" && [ ! -e "$tmp/home6/Library/LaunchAgents/dev.gridops.tart-agent.plist" ]'

[ "$failures" = 0 ] || { echo "$failures failed"; exit 1; }
echo "all passed"
