#!/usr/bin/env bash
# Prepares a Tart macOS base image for GitLab runners (#142): installs a
# pinned gitlab-runner release, checksum-verified, at the guest path the Tart
# agent runs (GRIDOPS_TART_GITLAB_RUNNER_BINARY on the agent; the default
# below is the agent's default). It takes no token: each runner's token
# reaches its VM at start, on stdin.
set -euo pipefail

# The pinned gitlab-runner release and the SHA-256 GitLab publishes for each
# macOS binary (https://gitlab-runner-downloads.s3.amazonaws.com/v19.5.3/release.sha256).
gitlab_runner_version=19.5.3
gitlab_runner_sha256_darwin_arm64=43c7ddb8c0f35e4b15fe3282f9577353f6b5da605b7cd13a31490b690ebb7a10
gitlab_runner_sha256_darwin_amd64=5d2b4514fe3538771da7906c4746598a2674a0c0e68577b8cdd1b9b5d6cd659c

if [[ $# -ne 2 ]]; then
  cat >&2 <<'USAGE'
Usage: prepare-tart-gitlab-runner-image.sh <source-image> <target-image>

Clones <source-image> to <target-image> and installs the pinned gitlab-runner
release in it. The guest must already have a working git (the shell executor
runs jobs on the VM's own toolchain); this script installs no packages.
USAGE
  exit 64
fi

source_image=$1
target_image=$2
tart_binary=${GRIDOPS_TART_BINARY:-/opt/homebrew/bin/tart}
runner_binary=${GRIDOPS_TART_GITLAB_RUNNER_BINARY:-/Users/admin/gitlab-runner/bin/gitlab-runner}

if [[ ! -x "$tart_binary" ]]; then
  printf 'Tart is unavailable at %s\n' "$tart_binary" >&2
  exit 69
fi
# The same rule the Tart agent applies to GRIDOPS_TART_GITLAB_RUNNER_BINARY.
if [[ ! $runner_binary =~ ^/[A-Za-z0-9/._-]+$ || ${#runner_binary} -gt 512 ]]; then
  printf 'GRIDOPS_TART_GITLAB_RUNNER_BINARY must be an absolute guest path of letters, digits, /, ., _ and -.\n' >&2
  exit 64
fi

"$tart_binary" clone "$source_image" "$target_image"
cleanup() {
  "$tart_binary" stop "$target_image" >/dev/null 2>&1 || true
}
trap cleanup EXIT
"$tart_binary" run --no-graphics "$target_image" >/dev/null &

for _ in $(seq 1 90); do
  if "$tart_binary" exec "$target_image" /usr/bin/true >/dev/null 2>&1; then
    break
  fi
  sleep 2
done
"$tart_binary" exec "$target_image" /usr/bin/true >/dev/null

"$tart_binary" exec -i "$target_image" /bin/bash -s -- \
  "$runner_binary" "$gitlab_runner_version" \
  "$gitlab_runner_sha256_darwin_arm64" "$gitlab_runner_sha256_darwin_amd64" <<'GUEST_SCRIPT'
set -euo pipefail
runner_binary=$1
version=$2
case "$(uname -m)" in
  arm64) arch=arm64; sha256=$3 ;;
  x86_64) arch=amd64; sha256=$4 ;;
  *) printf 'Unsupported guest architecture %s.\n' "$(uname -m)" >&2; exit 69 ;;
esac
if ! git --version >/dev/null 2>&1; then
  printf 'The guest has no working git, which shell executor jobs need. Install it in the source image first.\n' >&2
  exit 69
fi
git --version >&2
download=$(mktemp /tmp/gridops-gitlab-runner.XXXXXX)
trap 'rm -f "$download"' EXIT
curl --fail --location --proto '=https' --tlsv1.2 --retry 5 --retry-all-errors --connect-timeout 15 --speed-limit 1000 --speed-time 30 \
  "https://gitlab-runner-downloads.s3.amazonaws.com/v${version}/binaries/gitlab-runner-darwin-${arch}" --output "$download"
printf '%s  %s\n' "$sha256" "$download" | shasum -a 256 --check
mkdir -p "$(dirname "$runner_binary")"
mv "$download" "$runner_binary"
chmod 755 "$runner_binary"
"$runner_binary" --version
GUEST_SCRIPT

printf 'Prepared %s with gitlab-runner %s at %s.\n' "$target_image" "$gitlab_runner_version" "$runner_binary"
