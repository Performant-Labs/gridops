#!/bin/sh
# Tests the launchd plist that install-tart-agent.sh writes, and what a re-run
# keeps from an existing one and what it changes, on any Unix with
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
out1="$out"
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

# plist_program <home>: the program launchd runs.
plist_program() {
  python3 -I -c 'import plistlib, sys
with open(sys.argv[1], "rb") as f:
    print(plistlib.load(f)["ProgramArguments"][0])' \
    "$1/Library/LaunchAgents/dev.gridops.tart-agent.plist"
}

# has <text>: the last installer output contains <text>, as written.
has() { printf '%s\n' "$out" | grep -qF -- "$1"; }

# 7. A first install has nothing to keep and says nothing about it.
check "a first install keeps nothing" '! printf "%s\n" "$out1" | grep -q -e kept -e "existing install"'

# 8. Re-running the installer without a setting keeps the value the existing
# install has, for every setting the installer writes, and says so; the
# defaults that changed in 0.2.0 do not silently replace an operator's 1 and
# 180.
fake_binary "$tmp/own tart/tart"
old_home="$tmp/home8"
out="$(install "$old_home" "$brew" \
  GRIDOPS_TART_AGENT_HOME="$tmp/agent8" \
  GRIDOPS_TART_AGENT_INSTALL_DIR="$tmp/bin8" \
  GRIDOPS_TART_AGENT_BIND=0.0.0.0:8790 \
  GRIDOPS_TART_BINARY="$tmp/own tart/tart" \
  GRIDOPS_TART_RUNNER_ROOT=/Users/runner/actions-runner \
  GRIDOPS_TART_NETWORK_MODE=nat \
  GRIDOPS_TART_CPU_BUDGET=6 \
  GRIDOPS_TART_MEMORY_BUDGET_MB=8192 \
  GRIDOPS_TART_MAX_RUNNERS=1 \
  GRIDOPS_TART_MIN_FREE_DISK_MB=20480 \
  GRIDOPS_TART_VM_SLOT_WAIT_SECONDS=180 2>&1)" || { echo "$out"; exit 1; }
out="$(install "$old_home" "$brew" 2>&1)" || { echo "$out"; exit 1; }
for kept in \
  "GRIDOPS_TART_AGENT_HOME=$tmp/agent8" \
  "GRIDOPS_TART_AGENT_INSTALL_DIR=$tmp/bin8" \
  "GRIDOPS_TART_AGENT_BIND=0.0.0.0:8790" \
  "GRIDOPS_TART_BINARY=$tmp/own tart/tart" \
  "GRIDOPS_TART_RUNNER_ROOT=/Users/runner/actions-runner" \
  "GRIDOPS_TART_NETWORK_MODE=nat" \
  "GRIDOPS_TART_CPU_BUDGET=6" \
  "GRIDOPS_TART_MEMORY_BUDGET_MB=8192" \
  "GRIDOPS_TART_MAX_RUNNERS=1" \
  "GRIDOPS_TART_MIN_FREE_DISK_MB=20480" \
  "GRIDOPS_TART_VM_SLOT_WAIT_SECONDS=180"; do
  name="${kept%%=*}"
  value="${kept#*=}"
  check "a re-run keeps $name" \
    'has "kept $kept from the existing install" && { [ $name = GRIDOPS_TART_AGENT_INSTALL_DIR ] || [ "$(plist_env "$old_home" $name)" = "$value" ]; }'
done
check "a re-run keeps the agent's binary in its install directory" \
  '[ "$(plist_program "$old_home")" = "$tmp/bin8/gridops-tart-agent" ] && [ -x "$tmp/bin8/gridops-tart-agent" ]'
check "a re-run keeps the token in the agent's home" \
  '[ "$(plist_env "$old_home" GRIDOPS_TART_AGENT_TOKEN_FILE)" = "$tmp/agent8/token" ] && [ -s "$tmp/agent8/token" ]'
check "a kept value that is not today's default says what the default is" \
  'has "kept GRIDOPS_TART_MAX_RUNNERS=1 from the existing install (the default is 2)"'
check "a re-run without changes reports no change" '! has "set GRIDOPS_"'

# 9. A setting passed on a re-run replaces the kept value and the installer
# prints the change; the settings not passed are still kept.
out="$(install "$old_home" "$brew" GRIDOPS_TART_VM_SLOT_WAIT_SECONDS=60 GRIDOPS_TART_CPU_BUDGET=6 2>&1)" || { echo "$out"; exit 1; }
check "a setting passed on a re-run is written" \
  '[ "$(plist_env "$old_home" GRIDOPS_TART_VM_SLOT_WAIT_SECONDS)" = 60 ]'
check "the change is printed" 'has "set GRIDOPS_TART_VM_SLOT_WAIT_SECONDS 180 -> 60"'
check "a setting passed with its existing value is neither kept nor changed" \
  '! has "GRIDOPS_TART_CPU_BUDGET"'
check "the settings not passed are still kept" \
  '[ "$(plist_env "$old_home" GRIDOPS_TART_MAX_RUNNERS)" = 1 ] && has "kept GRIDOPS_TART_MAX_RUNNERS=1 from the existing install"'

# 10. Moving the agent's home moves the install directory that followed it.
out="$(install "$tmp/home5" "$brew" GRIDOPS_TART_AGENT_HOME="$tmp/agent10" 2>&1)" || { echo "$out"; exit 1; }
check "a new home is written and its change printed" \
  '[ "$(plist_env "$tmp/home5" GRIDOPS_TART_AGENT_HOME)" = "$tmp/agent10" ] && has "set GRIDOPS_TART_AGENT_HOME $tmp/home5/.gridops/tart-agent -> $tmp/agent10"'
check "an install directory that followed the home moves with it" \
  '[ "$(plist_program "$tmp/home5")" = "$tmp/agent10/bin/gridops-tart-agent" ] && has "set GRIDOPS_TART_AGENT_INSTALL_DIR $tmp/home5/.gridops/tart-agent/bin -> $tmp/agent10/bin"'
check "the earlier values of home5 are kept" \
  'has "kept GRIDOPS_TART_MAX_RUNNERS=1 from the existing install" && has "kept GRIDOPS_TART_VM_SLOT_WAIT_SECONDS=600 from the existing install"'

# 11. Values with characters special to sed and XML survive being read back.
out="$(install "$tmp/home2" "$softnet_dir" 2>&1)" || { echo "$out"; exit 1; }
check "a kept Tart path is written back exactly" \
  '[ "$(plist_env "$tmp/home2" GRIDOPS_TART_BINARY)" = "$odd/tart" ] && has "kept GRIDOPS_TART_BINARY=$odd/tart from the existing install"'
check "the PATH is unchanged" \
  '[ "$(plist_env "$tmp/home2" PATH)" = "$odd:$softnet_dir:/usr/bin:/bin:/usr/sbin:/sbin" ] && ! has "set PATH"'

# 12. A setting an older install did not write gets today's default, and the
# installer says so.
python3 -I -c 'import plistlib, sys
with open(sys.argv[1], "rb") as f:
    p = plistlib.load(f)
del p["EnvironmentVariables"]["GRIDOPS_TART_VM_SLOT_WAIT_SECONDS"]
with open(sys.argv[1], "wb") as f:
    plistlib.dump(p, f)' "$tmp/home1/Library/LaunchAgents/dev.gridops.tart-agent.plist"
out="$(install "$tmp/home1" "$brew" 2>&1)" || { echo "$out"; exit 1; }
check "a setting the existing install lacks gets the default" \
  '[ "$(plist_env "$tmp/home1" GRIDOPS_TART_VM_SLOT_WAIT_SECONDS)" = 60 ] && has "set GRIDOPS_TART_VM_SLOT_WAIT_SECONDS to 60 (not in the existing install)"'

# 13. An existing plist that cannot be read stops the installer, which says
# so and leaves the file alone instead of guessing.
plist13="$tmp/home13/Library/LaunchAgents/dev.gridops.tart-agent.plist"
mkdir -p "$(dirname "$plist13")"
printf 'not a plist\n' > "$plist13"
if out="$(install "$tmp/home13" "$brew" 2>&1)"; then ok=0; else ok=1; fi
check "an unreadable existing plist is refused and left alone" \
  '[ $ok = 1 ] && has "Cannot read the existing launchd agent $plist13" && [ "$(cat "$plist13")" = "not a plist" ] && [ ! -e "$tmp/home13/.gridops" ]'

# 14. A kept value is checked like a passed one, and the error says where it
# came from: here a wait an install from before the 600-second limit wrote.
python3 -I -c 'import plistlib, sys
with open(sys.argv[1], "rb") as f:
    p = plistlib.load(f)
p["EnvironmentVariables"]["GRIDOPS_TART_VM_SLOT_WAIT_SECONDS"] = "900"
with open(sys.argv[1], "wb") as f:
    plistlib.dump(p, f)' "$tmp/home1/Library/LaunchAgents/dev.gridops.tart-agent.plist"
if out="$(install "$tmp/home1" "$brew" 2>&1)"; then ok=0; else ok=1; fi
check "a kept slot wait above 600 seconds is refused, naming the existing install" \
  '[ $ok = 1 ] && has "GRIDOPS_TART_VM_SLOT_WAIT_SECONDS is 900 (from the existing install)" && [ "$(plist_env "$tmp/home1" GRIDOPS_TART_VM_SLOT_WAIT_SECONDS)" = 900 ]'
out="$(install "$tmp/home1" "$brew" GRIDOPS_TART_VM_SLOT_WAIT_SECONDS=60 2>&1)" || { echo "$out"; exit 1; }
check "passing the setting fixes it" \
  '[ "$(plist_env "$tmp/home1" GRIDOPS_TART_VM_SLOT_WAIT_SECONDS)" = 60 ] && has "set GRIDOPS_TART_VM_SLOT_WAIT_SECONDS 900 -> 60"'

# 15. A kept Tart binary that is gone is refused, not replaced by another.
rm "$tmp/own tart/tart"
if out="$(install "$old_home" "$brew" 2>&1)"; then ok=0; else ok=1; fi
check "a kept Tart binary that is gone is refused" \
  '[ $ok = 1 ] && has "GRIDOPS_TART_BINARY is $tmp/own tart/tart (from the existing install)" && [ "$(plist_env "$old_home" GRIDOPS_TART_BINARY)" = "$tmp/own tart/tart" ]'

[ "$failures" = 0 ] || { echo "$failures failed"; exit 1; }
echo "all passed"
