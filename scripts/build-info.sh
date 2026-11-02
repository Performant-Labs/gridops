#!/bin/sh
# Prints the build number and commit of the checkout this script lives in. This is the one place a
# GridOps build gets its number; Docker, compose, the release workflow and the Rust and web build scripts
# all take it from here (or from the same two environment variables, which win when set).
#
#   GRIDOPS_BUILD_NUMBER=412     the number of commits in the history of HEAD (`git rev-list --count HEAD`)
#   GRIDOPS_COMMIT=9d92a33       the short commit
#
# Usage:
#   scripts/build-info.sh                       print KEY=value lines
#   scripts/build-info.sh --export              print `export KEY=value` lines, for eval
#   eval "$(scripts/build-info.sh --export)"    export both into the current shell
#   env $(scripts/build-info.sh) docker compose build
#
# It fails, with a message on stderr, rather than print a made-up number: outside a git checkout, in a
# shallow clone (the count would be too low), or when GRIDOPS_BUILD_NUMBER is not a positive integer.
set -eu

prefix=""
case "${1:-}" in
  "") ;;
  --export) prefix="export " ;;
  *) echo "usage: build-info.sh [--export]" >&2; exit 2 ;;
esac

die() { echo "build-info.sh: $1" >&2; exit 1; }

number="${GRIDOPS_BUILD_NUMBER:-}"
commit="${GRIDOPS_COMMIT:-}"

if [ -n "$number" ]; then
  case "$number" in
    *[!0-9]* | 0*) die "GRIDOPS_BUILD_NUMBER must be a positive integer without leading zeros, got '$number'" ;;
  esac
fi
if [ -n "$commit" ]; then
  case "$commit" in
    *[!0-9a-fA-F]*) die "GRIDOPS_COMMIT must be a hexadecimal commit id (7 to 40 characters), got '$commit'" ;;
  esac
  [ "${#commit}" -ge 7 ] && [ "${#commit}" -le 40 ] || die "GRIDOPS_COMMIT must be a hexadecimal commit id (7 to 40 characters), got '$commit'"
  commit="$(printf '%s' "$commit" | tr 'A-F' 'a-f' | cut -c1-7)"
fi

if [ -z "$number" ] || [ -z "$commit" ]; then
  root="$(cd "$(dirname "$0")/.." && pwd)"
  hint="Run this in a full git checkout, or set GRIDOPS_BUILD_NUMBER and GRIDOPS_COMMIT."
  command -v git >/dev/null 2>&1 || die "git is not installed. $hint"
  [ "$(git -C "$root" rev-parse --is-inside-work-tree 2>/dev/null || true)" = true ] \
    || die "$root is not a git checkout, so there is no build number to read. $hint"
  if [ -z "$number" ]; then
    [ "$(git -C "$root" rev-parse --is-shallow-repository 2>/dev/null || true)" != true ] \
      || die "$root is a shallow clone, which would give a wrong, low build number. Run 'git fetch --unshallow' (or check out with full history), or set GRIDOPS_BUILD_NUMBER."
    number="$(git -C "$root" rev-list --count HEAD 2>/dev/null)" || die "could not count the commits of HEAD. $hint"
  fi
  if [ -z "$commit" ]; then
    commit="$(git -C "$root" rev-parse --short=7 HEAD 2>/dev/null)" || die "could not read the commit of HEAD. $hint"
  fi
fi

printf '%sGRIDOPS_BUILD_NUMBER=%s\n' "$prefix" "$number"
printf '%sGRIDOPS_COMMIT=%s\n' "$prefix" "$commit"
