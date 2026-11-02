#!/bin/sh
# Tests scripts/build-info.sh against throwaway git repositories:
#   sh scripts/test-build-info.sh
set -u

here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
failures=0

pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; failures=$((failures + 1)); }

# Stray values from the caller's shell must not leak into the cases.
unset GRIDOPS_BUILD_NUMBER GRIDOPS_COMMIT

# A repository with three commits and the script under test.
repo="$work/repo"
mkdir -p "$repo/scripts"
cp "$here/build-info.sh" "$repo/scripts/build-info.sh"
(
  cd "$repo" || exit 1
  git init -q -b master .
  git config user.email test@example.invalid
  git config user.name test
  git config commit.gpgsign false
  for n in 1 2 3; do
    echo "$n" > file
    git add file scripts
    git commit -q -m "commit $n"
  done
) || { echo "could not build the fixture repository"; exit 1; }
sha="$(git -C "$repo" rev-parse --short=7 HEAD)"

run() { (cd "$1" && shift && "$@") ; }

# 1. Numbers come from git: the commit count and the 7-character sha.
out="$(run "$repo" sh scripts/build-info.sh 2>&1)"
expected="GRIDOPS_BUILD_NUMBER=3
GRIDOPS_COMMIT=$sha"
[ "$out" = "$expected" ] && pass "prints the commit count and short sha" || fail "prints the commit count and short sha (got: $out)"

# 2. The number rises with every commit.
(cd "$repo" && echo 4 > file && git add file && git commit -q -m "commit 4")
out="$(run "$repo" sh scripts/build-info.sh 2>&1)"
case "$out" in GRIDOPS_BUILD_NUMBER=4*) pass "the number rises with a new commit" ;; *) fail "the number rises with a new commit (got: $out)" ;; esac
sha="$(git -C "$repo" rev-parse --short=7 HEAD)"

# 3. It works from any directory: it looks at its own checkout.
out="$(cd "$work" && sh "$repo/scripts/build-info.sh" 2>&1)"
case "$out" in GRIDOPS_BUILD_NUMBER=4*) pass "works from another directory" ;; *) fail "works from another directory (got: $out)" ;; esac

# 4. --export prefixes each line so eval exports the variables.
out="$(run "$repo" sh scripts/build-info.sh --export 2>&1)"
expected="export GRIDOPS_BUILD_NUMBER=4
export GRIDOPS_COMMIT=$sha"
[ "$out" = "$expected" ] && pass "--export prints export lines" || fail "--export prints export lines (got: $out)"
exported="$(cd "$repo" && eval "$(sh scripts/build-info.sh --export)" && sh -c 'echo "$GRIDOPS_BUILD_NUMBER/$GRIDOPS_COMMIT"')"
[ "$exported" = "4/$sha" ] && pass "eval of --export reaches child processes" || fail "eval of --export reaches child processes (got: $exported)"

# 5. Variables already set win, and are checked.
out="$(cd "$repo" && GRIDOPS_BUILD_NUMBER=900 GRIDOPS_COMMIT=ABCDEF0123456 sh scripts/build-info.sh 2>&1)"
expected="GRIDOPS_BUILD_NUMBER=900
GRIDOPS_COMMIT=abcdef0"
[ "$out" = "$expected" ] && pass "honours env, lower-cases and shortens the commit" || fail "honours env (got: $out)"

out="$(cd "$repo" && GRIDOPS_BUILD_NUMBER=900 sh scripts/build-info.sh 2>&1)"
expected="GRIDOPS_BUILD_NUMBER=900
GRIDOPS_COMMIT=$sha"
[ "$out" = "$expected" ] && pass "takes the missing half from git" || fail "takes the missing half from git (got: $out)"

if out="$(cd "$repo" && GRIDOPS_BUILD_NUMBER=dev sh scripts/build-info.sh 2>&1)"; then fail "rejects a non-numeric build number"; else
  case "$out" in *"GRIDOPS_BUILD_NUMBER"*"positive integer"*) pass "rejects a non-numeric build number" ;; *) fail "rejects a non-numeric build number (message: $out)" ;; esac
fi
if out="$(cd "$repo" && GRIDOPS_BUILD_NUMBER=0 sh scripts/build-info.sh 2>&1)"; then fail "rejects build number 0"; else pass "rejects build number 0"; fi
if out="$(cd "$repo" && GRIDOPS_COMMIT=nothex sh scripts/build-info.sh 2>&1)"; then fail "rejects a bad commit"; else
  case "$out" in *"GRIDOPS_COMMIT"*) pass "rejects a bad commit" ;; *) fail "rejects a bad commit (message: $out)" ;; esac
fi

# 6. Outside a git checkout it fails with a message, unless both are set.
bare="$work/bare"
mkdir -p "$bare/scripts"
cp "$here/build-info.sh" "$bare/scripts/build-info.sh"
if out="$(cd "$bare" && sh scripts/build-info.sh 2>&1)"; then fail "fails outside a git checkout"; else
  case "$out" in *"not a git checkout"*"GRIDOPS_BUILD_NUMBER"*) pass "fails outside a git checkout, naming the variables" ;; *) fail "fails outside a git checkout (message: $out)" ;; esac
fi
out="$(cd "$bare" && GRIDOPS_BUILD_NUMBER=7 GRIDOPS_COMMIT=1234567 sh scripts/build-info.sh 2>&1)"
expected="GRIDOPS_BUILD_NUMBER=7
GRIDOPS_COMMIT=1234567"
[ "$out" = "$expected" ] && pass "needs no git when both variables are set" || fail "needs no git when both are set (got: $out)"
if (cd "$bare" && GRIDOPS_BUILD_NUMBER=7 sh scripts/build-info.sh >/dev/null 2>&1); then fail "fails outside git with only the number"; else pass "fails outside git with only the number"; fi

# 7. A shallow clone would report a wrong, low count, so it refuses.
shallow="$work/shallow"
git clone -q --depth 1 "file://$repo" "$shallow" 2>/dev/null
if out="$(cd "$shallow" && sh scripts/build-info.sh 2>&1)"; then fail "refuses a shallow clone"; else
  case "$out" in *shallow*) pass "refuses a shallow clone" ;; *) fail "refuses a shallow clone (message: $out)" ;; esac
fi
out="$(cd "$shallow" && GRIDOPS_BUILD_NUMBER=4 GRIDOPS_COMMIT=$sha sh scripts/build-info.sh 2>&1)"
[ "$out" = "GRIDOPS_BUILD_NUMBER=4
GRIDOPS_COMMIT=$sha" ] && pass "a shallow clone is fine when both are set" || fail "a shallow clone is fine when both are set (got: $out)"

# 8. The Docker build's gate (scripts/docker-build-info.sh) refuses a build without a number.
gate="$here/docker-build-info.sh"
env_file="$work/build-info.env"
if out="$(sh "$gate" "$env_file" 2>&1)"; then fail "the docker gate refuses a missing number"; else
  case "$out" in *"GRIDOPS_BUILD_NUMBER is not set"*"scripts/build-info.sh"*) pass "the docker gate refuses a missing number and points at build-info.sh" ;; *) fail "the docker gate message (got: $out)" ;; esac
fi
if GRIDOPS_BUILD_NUMBER=5 sh "$gate" "$env_file" >/dev/null 2>&1; then fail "the docker gate refuses a missing commit"; else pass "the docker gate refuses a missing commit"; fi
if GRIDOPS_BUILD_NUMBER=dev GRIDOPS_COMMIT=abcdef0 sh "$gate" "$env_file" >/dev/null 2>&1; then fail "the docker gate refuses a non-numeric number"; else pass "the docker gate refuses a non-numeric number"; fi
if GRIDOPS_BUILD_NUMBER=5 GRIDOPS_COMMIT=abc sh "$gate" "$env_file" >/dev/null 2>&1; then fail "the docker gate refuses a short commit"; else pass "the docker gate refuses a short commit"; fi
GRIDOPS_BUILD_NUMBER=412 GRIDOPS_COMMIT=9D92A33ffe sh "$gate" "$env_file" >/dev/null 2>&1
[ "$(cat "$env_file" 2>/dev/null)" = "GRIDOPS_BUILD_NUMBER=412
GRIDOPS_COMMIT=9d92a33" ] && pass "the docker gate writes the checked values" || fail "the docker gate writes the checked values (got: $(cat "$env_file" 2>/dev/null))"

if [ "$failures" -ne 0 ]; then
  printf '%s test(s) failed\n' "$failures"
  exit 1
fi
echo "all build-info tests passed"
