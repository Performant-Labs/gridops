#!/bin/sh
# Installs the GridOps host agent on Linux or macOS, enrolls the machine and
# keeps the agent running as a user service.
#
#   curl -fsSL https://gridops.example.com/install.sh | sh
#
# GridOps serves this script with its own address and version filled in, so
# the agent always matches the control plane. Run from a GitHub release it
# installs that release and asks for the address.
#
# Settings (environment variables):
#   GRIDOPS_URL               GridOps address the agent connects to
#   GRIDOPS_VERSION           agent version to install, e.g. 0.1.0 (default: latest)
#   GRIDOPS_INSTALL_DIR       where the binary goes (default: ~/.local/bin)
#   GRIDOPS_ENROLLMENT_CODE   enroll without a prompt
#   GRIDOPS_NO_ENROLL=1       install only
#   GRIDOPS_NO_SERVICE=1      don't install or start the user service
set -eu

REPOSITORY="iamngoni/gridops"
SERVED_URL="__GRIDOPS_URL__"
SERVED_VERSION="__GRIDOPS_VERSION__"
case "$SERVED_URL" in __*) SERVED_URL="" ;; esac
case "$SERVED_VERSION" in __*) SERVED_VERSION="" ;; esac

URL="${GRIDOPS_URL:-$SERVED_URL}"
VERSION="${GRIDOPS_VERSION:-$SERVED_VERSION}"
VERSION="${VERSION#v}"
INSTALL_DIR="${GRIDOPS_INSTALL_DIR:-$HOME/.local/bin}"
STATE_DIR="${GRIDOPS_AGENT_STATE_DIR:-$HOME/.gridops/host-agent}"
LABEL="dev.gridops.host-agent"

say() { printf '%s\n' "$*"; }
fail() { printf 'gridops: %s\n' "$*" >&2; exit 1; }

case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  *) fail "unsupported system $(uname -s); on Windows use install.ps1" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x64 ;;
  aarch64 | arm64) arch=arm64 ;;
  *) fail "unsupported architecture $(uname -m)" ;;
esac
# A shell under Rosetta reports x86_64 on Apple silicon.
if [ "$os" = macos ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = 1 ]; then
  arch=arm64
fi

if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL --retry 3 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -q -O "$2" "$1"; }
else
  fail "curl or wget is required"
fi
if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
else
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

if [ -n "$VERSION" ]; then
  base="https://github.com/$REPOSITORY/releases/download/v$VERSION"
else
  base="https://github.com/$REPOSITORY/releases/latest/download"
fi
asset="gridops-host-agent-$os-$arch"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

say "Downloading $asset${VERSION:+ $VERSION}..."
fetch "$base/$asset" "$work/$asset" || fail "couldn't download $base/$asset"
fetch "$base/SHA256SUMS" "$work/SHA256SUMS" || fail "couldn't download $base/SHA256SUMS"
expected="$(awk -v name="$asset" '$2 == name || $2 == "*" name { print $1 }' "$work/SHA256SUMS")"
[ -n "$expected" ] || fail "SHA256SUMS has no entry for $asset"
[ "$(sha256 "$work/$asset")" = "$expected" ] || fail "checksum mismatch for $asset"

mkdir -p "$INSTALL_DIR"
bin="$INSTALL_DIR/gridops-host-agent"
cp "$work/$asset" "$bin.new"
chmod 755 "$bin.new"
mv -f "$bin.new" "$bin"
say "Installed $bin ($("$bin" --version 2>/dev/null || echo "$asset"))."
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say "Add $INSTALL_DIR to your PATH to run gridops-host-agent directly." ;;
esac

enrolled() { [ -s "$STATE_DIR/agent.json" ] && grep -q fleet_url "$STATE_DIR/agent.json"; }

enrolled_now=""
if [ "${GRIDOPS_NO_ENROLL:-}" != 1 ] && ! enrolled; then
  enrolled_now=1
  [ -n "$URL" ] && export GRIDOPS_FLEET_URL="$URL"
  if [ -n "${GRIDOPS_ENROLLMENT_CODE:-}" ]; then
    "$bin" enroll --no-start
  elif [ -r /dev/tty ] && [ -w /dev/tty ]; then
    # Piped into sh, stdin is this script: take the code from the terminal.
    "$bin" enroll --no-start </dev/tty >/dev/tty
  else
    fail "no terminal to ask for the enrollment code; set GRIDOPS_ENROLLMENT_CODE"
  fi
fi

if [ "${GRIDOPS_NO_SERVICE:-}" = 1 ]; then
  say "Start the agent with: $bin run"
  exit 0
fi
if ! enrolled; then
  say "Enroll with: $bin enroll   then start it with: $bin run"
  exit 0
fi

mkdir -p "$STATE_DIR"
# Native jobs inherit the agent's environment, so the service keeps the PATH
# of the shell running this installer, plus where Docker Desktop and OrbStack
# put the docker CLI. It runs unthrottled, like an agent started by hand.
service_path="$PATH:/usr/local/bin:/opt/homebrew/bin:$HOME/.orbstack/bin"
if [ "$os" = macos ]; then
  plist="$HOME/Library/LaunchAgents/$LABEL.plist"
  if [ ! -f "$plist" ] && pgrep -f "gridops-host-agent run" >/dev/null 2>&1; then
    fail "an agent is already running outside launchd; stop it, then run this again (or set GRIDOPS_NO_SERVICE=1)"
  fi
  # Keep the allowlist from an earlier install unless a new one is given.
  mkdir -p "$(dirname "$plist")"
  path="$(printf '%s' "$service_path" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g')"
  cat >"$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key><array><string>$bin</string><string>run</string></array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>GRIDOPS_AGENT_STATE_DIR</key><string>$STATE_DIR</string>
    <key>PATH</key><string>$path</string>
    <key>RUST_LOG</key><string>gridops_host_agent=info</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardOutPath</key><string>$STATE_DIR/agent.log</string>
  <key>StandardErrorPath</key><string>$STATE_DIR/agent.log</string>
  <key>ThrottleInterval</key><integer>10</integer>
</dict>
</plist>
EOF
  domain="gui/$(id -u)"
  launchctl bootout "$domain/$LABEL" >/dev/null 2>&1 || true
  launchctl bootstrap "$domain" "$plist" || fail "launchctl couldn't load $plist"
  say "The agent runs as the LaunchAgent $LABEL. Logs: $STATE_DIR/agent.log"
elif command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then
  unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
  unit="$unit_dir/gridops-host-agent.service"
  if [ ! -f "$unit" ] && pgrep -f "gridops-host-agent run" >/dev/null 2>&1; then
    fail "an agent is already running outside systemd; stop it, then run this again (or set GRIDOPS_NO_SERVICE=1)"
  fi
  mkdir -p "$unit_dir"
  cat >"$unit" <<EOF
[Unit]
Description=GridOps host agent
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=$bin run
Environment=GRIDOPS_AGENT_STATE_DIR=$STATE_DIR
Environment="PATH=$(printf '%s' "$service_path" | sed 's/%/%%/g')"
Environment=RUST_LOG=gridops_host_agent=info
Restart=always
RestartSec=10

[Install]
WantedBy=default.target
EOF
  systemctl --user daemon-reload
  systemctl --user enable gridops-host-agent >/dev/null
  systemctl --user restart gridops-host-agent
  say "The agent runs as the user service gridops-host-agent. Logs: journalctl --user -u gridops-host-agent"
  if [ "$(loginctl show-user "$(id -un)" -p Linger --value 2>/dev/null || echo no)" != yes ]; then
    say "To keep it running after you log out: sudo loginctl enable-linger $(id -un)"
  fi
else
  say "No user service manager found. Start the agent with: $bin run"
fi
[ -z "$enrolled_now" ] || say "Approve the machine in GridOps under Machines."
