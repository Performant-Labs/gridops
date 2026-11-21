#!/bin/sh
# Tests the GitLab and the Bitbucket Tart image preparation scripts
# (scripts/prepare-tart-gitlab-runner-image.sh and
# scripts/prepare-tart-bitbucket-runner-image.sh, #173). It does not cover
# the GitHub one, scripts/prepare-tart-runner-image.sh.
#
# Each script runs under /bin/bash (bash 3.2 on macOS, whatever bash is
# first on PATH) against a fake `tart` (GRIDOPS_TART_BINARY) that boots
# nothing. The fake records every call in a log the script's own output is
# appended to, so the log is one timeline. It keeps each VM it is asked to
# clone as a file, a backgrounded `tart run` lasts until `tart stop` (and a
# moment after it, as a real VM's shutdown does), and `tart exec` answers for
# the target image (agent probe, install) and the verification clone (agent
# probe, binary check) as each case says. Nothing outside a temp directory is
# touched.
#
#   sh scripts/test-prepare-tart-runner-images.sh
#
# Checks are strings run by eval, so their variables expand when they run.
# The fake's VM names are this test's own, so listing them with ls is safe.
# shellcheck disable=SC2016,SC2034,SC2012
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# ---- The fake tart ----------------------------------------------------------
#
# FAKE_TART_STATE   its state directory: log, vms/, running/, stopping/, exec.*
# FAKE_TART_TARGET  the target image's name (the script's second argument)
# FAKE_TART_CLONE   present (default) | missing: whether the verification
#                   clone has the runner (its check exits 0, or 1)
# FAKE_TART_AGENT   answers (default) | never: whether the clone's guest
#                   agent ever answers `tart exec <clone> /usr/bin/true`
# FAKE_TART_HANG    install | check: that exec blocks (writing its pid to
#                   hang.pid) until the test interrupts it or its VM stops
#
# The verification clone is any VM cloned from the target. Log lines start
# with "FAKE: "; everything else in the log is the script's own output.
mkdir -p "$tmp/bin"
cat > "$tmp/bin/tart" <<'FAKE'
#!/bin/sh
state=$FAKE_TART_STATE
mkdir -p "$state/vms" "$state/running" "$state/stopping"
log() { printf 'FAKE: %s\n' "$*" >> "$state/log"; }
last() { for arg in "$@"; do :; done; printf '%s' "$arg"; }
is_clone() { [ "$(cat "$state/clone" 2>/dev/null)" = "$1" ]; }
hang() {
  echo $$ > "$state/hang.pid"
  i=0
  while [ $i -lt 300 ] && [ ! -e "$state/stopping/$1" ]; do
    /bin/sleep 0.1
    i=$((i + 1))
  done
  exit 1
}
command=$1
shift
case "$command" in
  clone)
    log "clone $1 $2"
    if [ -e "$state/vms/$2" ]; then echo "VM $2 already exists" >&2; exit 1; fi
    if [ "$1" = "$FAKE_TART_TARGET" ]; then
      if [ ! -e "$state/vms/$1" ]; then echo "no VM $1" >&2; exit 1; fi
      printf '%s' "$2" > "$state/clone"
    fi
    : > "$state/vms/$2"
    ;;
  run)
    vm=$(last "$@")
    log "run $vm"
    if [ ! -e "$state/vms/$vm" ]; then echo "no VM $vm" >&2; exit 1; fi
    if [ -n "$(ls "$state/running")" ]; then log "two VMs running: $(ls "$state/running") and $vm"; fi
    rm -f "$state/stopping/$vm"
    echo $$ > "$state/running/$vm"
    i=0
    while [ $i -lt 600 ] && [ ! -e "$state/stopping/$vm" ]; do
      /bin/sleep 0.1
      i=$((i + 1))
    done
    # A VM takes a moment to shut down after `tart stop` returns.
    /bin/sleep 0.3
    log "run-exit $vm"
    rm -f "$state/running/$vm"
    ;;
  stop)
    vm=$(last "$@")
    log "stop $vm"
    if [ ! -e "$state/running/$vm" ]; then echo "VM $vm is not running" >&2; exit 1; fi
    : > "$state/stopping/$vm"
    ;;
  delete)
    vm=$(last "$@")
    log "delete $vm"
    if [ -e "$state/running/$vm" ]; then echo "VM $vm is running" >&2; exit 1; fi
    if [ ! -e "$state/vms/$vm" ]; then echo "no VM $vm" >&2; exit 1; fi
    rm -f "$state/vms/$vm"
    ;;
  exec)
    interactive=''
    while [ $# -gt 0 ]; do
      case "$1" in
        -i) interactive=1; shift ;;
        -*) shift ;;
        *) break ;;
      esac
    done
    vm=$1
    shift
    if [ "$*" = /usr/bin/true ]; then kind=probe; else kind=command; fi
    log "exec $$ $vm $kind"
    # Every argument, then stdin: what the guest was asked to run.
    { printf '%s ' "$@"; echo; } > "$state/exec.$$.text"
    printf '%s' "$(last "$@")" > "$state/exec.$$.last"
    if [ -n "$interactive" ]; then cat > "$state/exec.$$.stdin"; cat "$state/exec.$$.stdin" >> "$state/exec.$$.text"; fi
    if [ ! -e "$state/running/$vm" ] || [ -e "$state/stopping/$vm" ]; then
      echo "VM $vm is not running" >&2
      exit 1
    fi
    if [ "$kind" = probe ]; then
      if is_clone "$vm" && [ "${FAKE_TART_AGENT:-answers}" = never ]; then exit 1; fi
      exit 0
    fi
    if is_clone "$vm"; then
      if [ "${FAKE_TART_HANG:-}" = check ]; then hang "$vm"; fi
      if [ "${FAKE_TART_CLONE:-present}" = missing ]; then exit 1; fi
      echo "Version:      19.5.3"
      exit 0
    fi
    if [ "${FAKE_TART_HANG:-}" = install ]; then hang "$vm"; fi
    ;;
  *)
    log "$command $*"
    ;;
esac
exit 0
FAKE
chmod +x "$tmp/bin/tart"

# The scripts wait for a guest agent with `sleep 2` between tries; here no
# wait is needed, so a clone whose agent never answers fails in moments.
mkdir -p "$tmp/stubs"
printf '#!/bin/sh\nexit 0\n' > "$tmp/stubs/sleep"
chmod +x "$tmp/stubs/sleep"

failures=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; failures=$((failures + 1)); fi; }

# ---- Running a script -------------------------------------------------------

# start <case> <script> <args...>: starts the script in the background, in a
# clean environment, with its output appended to the case's log. Sets
# $state, $log and $pid. Variables named in $extra (VAR=value words) are
# added to the environment.
start() {
  state="$tmp/$1"
  log="$state/log"
  script="$2"
  shift 2
  mkdir -p "$state"
  : > "$log"
  # shellcheck disable=SC2086
  env -i HOME="$tmp" PATH="$tmp/stubs:/usr/bin:/bin" \
    GRIDOPS_TART_BINARY="$tmp/bin/tart" FAKE_TART_STATE="$state" FAKE_TART_TARGET="$target" \
    $extra /bin/bash "$here/$script" "$@" >> "$log" 2>&1 < /dev/null &
  pid=$!
}

# finish: waits for the script started last; sets $status.
finish() {
  if wait "$pid" 2>/dev/null; then status=0; else status=$?; fi
}

# interrupt: waits (up to 10 s) for the blocking exec, then interrupts the
# script and that exec together, as Ctrl-C reaches both. Sets $hung to yes
# when the blocking exec was reached, no when the script ended (or never got
# there) first.
interrupt() {
  hung=no
  i=0
  while [ $i -lt 200 ] && kill -0 "$pid" 2>/dev/null; do
    if [ -s "$state/hang.pid" ]; then hung=yes; break; fi
    sleep 0.05
    i=$((i + 1))
  done
  if [ $hung = yes ]; then
    kill -TERM "$pid" "$(cat "$state/hang.pid")" 2>/dev/null || true
  else
    kill -TERM "$pid" 2>/dev/null || true
  fi
  finish
}

# The script's own output (the log without the fake's lines).
output() { grep -v '^FAKE: ' "$log" || true; }
# line <text>: the number of the first log line that is exactly <text>, or 0.
line() { n=$(grep -n -x -F -- "$1" "$log" | head -n 1 | cut -d: -f1); echo "${n:-0}"; }
# The verification clone's name, if one was made.
clone() { cat "$state/clone" 2>/dev/null || true; }
# exec_lines <vm> <kind>: the log lines of the execs of that kind on <vm>.
exec_lines() { grep -n -E "^FAKE: exec [0-9]+ $1 $2\$" "$log" || true; }
# first_exec <vm> <kind>: the line number of the first such exec, or 0.
first_exec() { n=$(exec_lines "$1" "$2" | head -n 1 | cut -d: -f1); echo "${n:-0}"; }
# The text (arguments and stdin) of every command exec on <vm>.
exec_texts() {
  exec_lines "$1" command | sed -E 's/^[0-9]+:FAKE: exec ([0-9]+) .*/\1/' | while read -r p; do
    cat "$state/exec.$p.text"
  done
}
# The guest script of the first command exec on the target: its stdin, or
# its last argument (bash -lc "<script>").
install_script() {
  p=$(exec_lines "$target" command | head -n 1 | sed -E 's/^[0-9]+:FAKE: exec ([0-9]+) .*/\1/')
  [ -n "$p" ] || return 0
  if [ -s "$state/exec.$p.stdin" ]; then cat "$state/exec.$p.stdin"; else cat "$state/exec.$p.last"; fi
}
# The last line of the install script that is not blank, trimmed.
install_last_line() { install_script | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' | sed '/^$/d' | tail -n 1; }
# The VMs that exist, and those whose `tart run` has not ended, one line.
vms() { ls "$state/vms" 2>/dev/null | tr '\n' ' ' | sed 's/ $//'; }
running() { ls "$state/running" 2>/dev/null | tr '\n' ' ' | sed 's/ $//'; }
succeeded() { output | grep -q "^Prepared $target"; }

source_image=gridops-macos-tahoe-base
target=gridops-macos-tahoe-runner-under-test

# ---- The cases, for each script ---------------------------------------------
#
# suite <name> <script> <the path the check must find> <extra env> <args...>
suite() {
  name=$1
  script=$2
  wanted=$3
  base_extra=$4
  shift 4

  # 1. The image is good: install, flush, stop the target and wait for it,
  # then check a fresh clone of it, and only then say it is prepared.
  extra="$base_extra"
  start "$name-good" "$script" "$@"
  finish
  out="$(output)"
  c="$(clone)"
  check "$name: a good image exits 0" '[ $status = 0 ] || { printf "%s\n" "$out"; false; }'
  check "$name: the guest script's last line is sync" '[ "$(install_last_line)" = sync ]'
  check "$name: the target is stopped after the install" \
    '[ $(first_exec "$target" command) -gt 0 ] && [ $(line "FAKE: stop $target") -gt $(first_exec "$target" command) ]'
  check "$name: a verification clone of the target is made" '[ -n "$c" ] && [ "$c" != "$target" ] && [ $(line "FAKE: clone $target $c") -gt 0 ]'
  check "$name: the target's tart run has ended before the clone is made" \
    '[ $(line "FAKE: run-exit $target") -gt 0 ] && [ $(line "FAKE: run-exit $target") -lt $(line "FAKE: clone $target $c") ]'
  check "$name: the clone boots after that, and never beside another VM" \
    '[ $(line "FAKE: run $c") -gt $(line "FAKE: clone $target $c") ] && ! grep -q "^FAKE: two VMs running" "$log"'
  check "$name: the clone is checked for $wanted" \
    '[ $(first_exec "$c" command) -gt $(line "FAKE: run $c") ] && exec_texts "$c" | grep -qF -- "test -x" && exec_texts "$c" | grep -qF -- "$wanted"'
  check "$name: the clone is stopped and deleted, its tart run ended first" \
    '[ $(line "FAKE: run-exit $c") -gt 0 ] && [ $(line "FAKE: delete $c") -gt $(line "FAKE: run-exit $c") ]'
  check "$name: success is printed only after the check" \
    'succeeded && [ $(first_exec "$c" command) -gt 0 ] && [ $(grep -n "^Prepared $target" "$log" | head -n 1 | cut -d: -f1) -gt $(first_exec "$c" command) ]'
  check "$name: only the target is left, and nothing runs" '[ "$(vms)" = "$target" ] && [ -z "$(running)" ]'
  good_log="$log"

  # 2. The clone has no runner: no success, the missing path named, and
  # neither the target nor the clone left behind.
  extra="$base_extra FAKE_TART_CLONE=missing"
  start "$name-missing" "$script" "$@"
  finish
  out="$(output)"
  c="$(clone)"
  check "$name: a missing runner exits non-zero" '[ $status != 0 ]'
  check "$name: and names the missing path, $wanted" \
    'printf "%s\n" "$out" | grep -v "^Prepared " | grep -qF -- "$wanted"'
  check "$name: and prints no success line" '! succeeded'
  check "$name: and deletes the target and the clone" \
    '[ -n "$c" ] && [ $(line "FAKE: delete $target") -gt 0 ] && [ $(line "FAKE: delete $c") -gt 0 ] && [ -z "$(vms)" ]'
  check "$name: and leaves nothing running" '[ -z "$(running)" ]'

  # 3. Interrupted during the install: the target is stopped, its tart run
  # waited for, and kept; no clone is made.
  extra="$base_extra FAKE_TART_HANG=install"
  start "$name-interrupted-install" "$script" "$@"
  interrupt
  check "$name: interrupted during the install, the run reached the install" '[ $hung = yes ]'
  check "$name: it exits non-zero, with no success line" '[ $status != 0 ] && ! succeeded'
  check "$name: the target is stopped and its tart run has ended" \
    '[ $(line "FAKE: stop $target") -gt 0 ] && [ -z "$(running)" ]'
  check "$name: the target is kept, and no clone is made" \
    '[ $(line "FAKE: delete $target") = 0 ] && [ "$(vms)" = "$target" ] && [ -z "$(clone)" ]'

  # 4. Interrupted during the clone's check: the target is kept (an
  # interruption is not a missing runner), the clone is stopped and deleted.
  extra="$base_extra FAKE_TART_HANG=check"
  start "$name-interrupted-check" "$script" "$@"
  interrupt
  check "$name: interrupted during the check, the run reached the check" '[ $hung = yes ]'
  check "$name: it exits non-zero, with no success line" '[ $status != 0 ] && ! succeeded'
  check "$name: the target is kept" '[ $(line "FAKE: delete $target") = 0 ] && [ "$(vms)" = "$target" ]'
  check "$name: the clone is deleted and nothing runs" \
    '[ -n "$(clone)" ] && [ $(line "FAKE: delete $(clone)") -gt 0 ] && [ -z "$(running)" ]'

  # 5. The clone's guest agent never answers: that is not a missing runner,
  # so the target is kept; the clone is still cleaned up.
  extra="$base_extra FAKE_TART_AGENT=never"
  start "$name-no-agent" "$script" "$@"
  finish
  check "$name: a clone whose agent never answers exits non-zero, with no success line" \
    '[ $status != 0 ] && ! succeeded'
  check "$name: the target is kept, the clone deleted, nothing runs" \
    '[ -n "$(clone)" ] && [ $(line "FAKE: delete $target") = 0 ] && [ "$(vms)" = "$target" ] && [ -z "$(running)" ]'

  # 6. Bad input keeps its exit codes and touches no VM.
  extra="$base_extra"
  start "$name-usage" "$script" "$source_image"
  finish
  check "$name: a wrong argument count exits 64 and runs no tart" '[ $status = 64 ] && ! grep -q "^FAKE: " "$log"'
  extra="$base_extra"
  state="$tmp/$name-no-tart"
  mkdir -p "$state"
  log="$state/log"
  # shellcheck disable=SC2086
  if env -i HOME="$tmp" PATH="$tmp/stubs:/usr/bin:/bin" GRIDOPS_TART_BINARY="$tmp/no-such-tart" \
    $extra /bin/bash "$here/$script" "$@" > "$log" 2>&1 < /dev/null; then status=0; else status=$?; fi
  check "$name: a missing tart exits 69" '[ $status = 69 ]'

  if [ $failures -gt 0 ] && [ -n "${VERBOSE:-}" ]; then
    echo "---- $name, good image, log:"
    cat "$good_log"
  fi
}

gitlab_binary=/Users/admin/gridops-test/gitlab-runner/bin/gitlab-runner
suite gitlab prepare-tart-gitlab-runner-image.sh "$gitlab_binary" \
  "GRIDOPS_TART_GITLAB_RUNNER_BINARY=$gitlab_binary" \
  "$source_image" "$target"
# GitLab's check also runs the binary.
check "gitlab: the clone runs gitlab-runner --version" \
  'state="$tmp/gitlab-good"; log="$state/log"; exec_texts "$(clone)" | grep -qF -- "--version"'

bitbucket_root=/Users/admin/gridops-test/bitbucket-runner
suite bitbucket prepare-tart-bitbucket-runner-image.sh "$bitbucket_root/bin/start.sh" \
  "GRIDOPS_TART_BITBUCKET_RUNNER_ROOT=$bitbucket_root" \
  "$source_image" "$target" https://product-downloads.example.test/atlassian-bitbucket-pipelines-runner-1.0.0.tar.gz

if [ $failures -gt 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "all passed"
