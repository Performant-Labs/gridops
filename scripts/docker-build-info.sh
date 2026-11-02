#!/bin/sh
# Run by the Dockerfile's build-info stage, where the build number arrives as the build arguments GRIDOPS_BUILD_NUMBER and
# GRIDOPS_COMMIT (the image has no .git to read them from). Stops the build with a message when either is missing or malformed,
# otherwise writes the checked values to the file named by $1 (default /build-info.env) for the later stages to source.
# It lives in a file, not inline in the Dockerfile, so a failed build shows this message, not a wall of script.
set -eu

out="${1:-/build-info.env}"
number="${GRIDOPS_BUILD_NUMBER:-}"
commit="${GRIDOPS_COMMIT:-}"

fail() {
  {
    echo ""
    echo "ERROR: $1"
    echo "Every GridOps build carries a build number. Build through scripts/build-info.sh:"
    echo '    env $(scripts/build-info.sh) docker compose build'
    echo "    docker build --build-arg GRIDOPS_BUILD_NUMBER=<n> --build-arg GRIDOPS_COMMIT=<sha> ..."
    echo "See docs/releasing.md."
    echo ""
  } >&2
  exit 1
}

case "$number" in
  "") fail "GRIDOPS_BUILD_NUMBER is not set, so this build would have no build number." ;;
  *[!0-9]* | 0*) fail "GRIDOPS_BUILD_NUMBER must be a positive integer, got '$number'." ;;
esac
case "$commit" in
  "") fail "GRIDOPS_COMMIT is not set, so this build would not say which commit it is." ;;
  *[!0-9a-fA-F]*) fail "GRIDOPS_COMMIT must be a hexadecimal commit id, got '$commit'." ;;
esac
[ "${#commit}" -ge 7 ] && [ "${#commit}" -le 40 ] || fail "GRIDOPS_COMMIT must be 7 to 40 hexadecimal characters, got '$commit'."

printf 'GRIDOPS_BUILD_NUMBER=%s\nGRIDOPS_COMMIT=%s\n' "$number" "$(printf '%s' "$commit" | cut -c1-7 | tr 'A-F' 'a-f')" > "$out"
