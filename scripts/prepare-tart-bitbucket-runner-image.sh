#!/usr/bin/env bash
# Prepares a Tart macOS base image for Bitbucket runners: installs the
# Bitbucket Pipelines runner from the archive URL Bitbucket Cloud gives, at the
# guest path the Tart agent runs (GRIDOPS_TART_BITBUCKET_RUNNER_ROOT).
#
# After the install the guest flushes its disk, the target VM is stopped and
# its `tart run` waited for, and the image is checked in a fresh clone of it
# (#173): only then is it reported prepared. A clone without the runner's
# bin/start.sh deletes the target, so no broken image keeps its name. Only one
# VM runs at a time.
set -euo pipefail

if [[ $# -ne 3 ]]; then
  cat >&2 <<'USAGE'
Usage: prepare-tart-bitbucket-runner-image.sh <source-image> <target-image> <runner-archive-url>

The archive URL comes from Bitbucket Cloud's macOS runner setup dialog. It is
not a credential and should point to the Atlassian runner tar.gz archive.
USAGE
  exit 64
fi

source_image=$1
target_image=$2
archive_url=$3
tart_binary=${GRIDOPS_TART_BINARY:-/opt/homebrew/bin/tart}
runner_root=${GRIDOPS_TART_BITBUCKET_RUNNER_ROOT:-/Users/admin/atlassian-bitbucket-pipelines-runner}

if [[ ! -x "$tart_binary" ]]; then
  printf 'Tart is unavailable at %s\n' "$tart_binary" >&2
  exit 69
fi
if [[ $runner_root != /* || $runner_root == *$'\n'* || $runner_root == *$'\r'* ]]; then
  printf 'GRIDOPS_TART_BITBUCKET_RUNNER_ROOT must be an absolute single-line guest path.\n' >&2
  exit 64
fi
if [[ $archive_url != https://* ]]; then
  printf 'The Bitbucket runner archive URL must use HTTPS.\n' >&2
  exit 64
fi

# The VMs this run started, and the verification clone once it exists. One
# cleanup stops whichever VM runs, waits for its `tart run`, and deletes the
# clone. It never deletes the target: an interrupted run keeps it, stopped.
target_pid=""
check_image=""
check_pid=""
stop_vm() {
  "$tart_binary" stop "$1" >/dev/null 2>&1 || true
  wait "$2" 2>/dev/null || true
}
cleanup() {
  if [[ -n $target_pid ]]; then
    stop_vm "$target_image" "$target_pid"
    target_pid=""
  fi
  if [[ -n $check_pid ]]; then
    stop_vm "$check_image" "$check_pid"
    check_pid=""
  fi
  if [[ -n $check_image ]]; then
    "$tart_binary" delete "$check_image" >/dev/null 2>&1 || true
    check_image=""
  fi
}
# An interrupt ends the run (and so runs cleanup); a returning trap would
# resume the script after the interrupted command.
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# wait_for_agent <vm>: waits for the VM's Tart guest agent to answer.
wait_for_agent() {
  for _ in $(seq 1 90); do
    if "$tart_binary" exec "$1" /usr/bin/true >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  if ! "$tart_binary" exec "$1" /usr/bin/true >/dev/null; then
    printf 'The Tart guest agent of %s did not answer.\n' "$1" >&2
    return 1
  fi
}

"$tart_binary" clone "$source_image" "$target_image"
"$tart_binary" run --no-graphics "$target_image" >/dev/null &
target_pid=$!
wait_for_agent "$target_image"

quoted_root=$(printf '%q' "$runner_root")
quoted_url=$(printf '%q' "$archive_url")
"$tart_binary" exec "$target_image" /bin/bash -lc "
set -euo pipefail
command -v java >/dev/null
java -version >&2
command -v git >/dev/null
rm -rf ${quoted_root}
mkdir -p ${quoted_root}
curl --fail --location --proto '=https' --tlsv1.2 --retry 5 --retry-all-errors --connect-timeout 15 --speed-limit 1000 --speed-time 30 ${quoted_url} --output /tmp/gridops-bitbucket-runner.tar.gz
tar -xzf /tmp/gridops-bitbucket-runner.tar.gz -C ${quoted_root}
rm -f /tmp/gridops-bitbucket-runner.tar.gz
test -x ${quoted_root}/bin/start.sh
sync
"

# Stop the target and wait for its `tart run` to end, so the install is on
# disk before the image is cloned.
"$tart_binary" stop "$target_image" >/dev/null
wait "$target_pid" || true
target_pid=""

# Check the image as a runner VM will see it: in a fresh clone.
start_script="$runner_root/bin/start.sh"
check_name="${target_image}-gridops-check-$$"
"$tart_binary" clone "$target_image" "$check_name"
check_image=$check_name
"$tart_binary" run --no-graphics "$check_image" >/dev/null &
check_pid=$!
wait_for_agent "$check_image"
# shellcheck disable=SC2016 # the guest's shell expands $1
if ! "$tart_binary" exec "$check_image" /bin/sh -c 'test -x "$1"' sh "$start_script"; then
  # Only an answer from a running guest says the runner is missing; a guest
  # agent that stopped answering says nothing about the image.
  if ! "$tart_binary" exec "$check_image" /usr/bin/true >/dev/null 2>&1; then
    printf 'Could not check %s in a fresh clone: its guest agent stopped answering. %s is kept.\n' "$start_script" "$target_image" >&2
    exit 1
  fi
  printf 'A fresh clone of %s has no executable Bitbucket runner at %s. Deleting %s.\n' "$target_image" "$start_script" "$target_image" >&2
  cleanup
  if ! "$tart_binary" delete "$target_image" >/dev/null; then
    printf 'Could not delete %s; delete it with: tart delete %s\n' "$target_image" "$target_image" >&2
  fi
  exit 1
fi
cleanup

printf 'Prepared %s with the Bitbucket runner at %s.\n' "$target_image" "$runner_root"
