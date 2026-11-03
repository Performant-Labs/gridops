#!/bin/zsh
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  print -u2 "gridops-tart-agent requires an Apple Silicon macOS host."
  exit 1
fi

script_dir="${0:A:h}"
repo_root="${script_dir:h}"
plist_source="${repo_root}/deploy/launchd/dev.gridops.tart-agent.plist"
plist_target="${HOME}/Library/LaunchAgents/dev.gridops.tart-agent.plist"

# Re-running the installer must not change a setting the operator did not
# pass. When an agent is already installed, every setting below that is not
# set (or is empty) in the environment keeps the value in the installed
# launchd agent, and the installer prints what it kept and what it changed.
# A first install uses the defaults. The token is not a setting here: it is
# always passed and always rewritten.
settings=(
  GRIDOPS_TART_AGENT_HOME
  GRIDOPS_TART_AGENT_INSTALL_DIR
  GRIDOPS_TART_AGENT_BIND
  GRIDOPS_TART_BINARY
  GRIDOPS_TART_RUNNER_ROOT
  GRIDOPS_TART_NETWORK_MODE
  GRIDOPS_TART_CPU_BUDGET
  GRIDOPS_TART_MEMORY_BUDGET_MB
  GRIDOPS_TART_MAX_RUNNERS
  GRIDOPS_TART_MIN_FREE_DISK_MB
  GRIDOPS_TART_VM_SLOT_WAIT_SECONDS
)
typeset -A existing defaults
typeset -a kept
existing_install=0
if [[ -e "${plist_target}" || -L "${plist_target}" ]]; then
  existing_install=1
  if ! command -v python3 >/dev/null 2>&1; then
    print -u2 "An agent is already installed (${plist_target}), and reading its settings needs python3. Install the Xcode Command Line Tools: xcode-select --install"
    exit 1
  fi
  # Prints NAME=value for each environment variable asked for that the plist
  # sets to a non-empty string, and ProgramArguments=<program>. Anything else
  # it cannot read is an error, printed with exit status 1, not a guess.
  if ! existing_text="$(python3 -I -c '
import plistlib, sys

def fail(why):
    print(why)
    sys.exit(1)

try:
    with open(sys.argv[1], "rb") as f:
        plist = plistlib.load(f)
except Exception as error:
    fail(f"{type(error).__name__}: {error}")
if not isinstance(plist, dict):
    fail("it is not a dictionary")
env = plist.get("EnvironmentVariables")
args = plist.get("ProgramArguments")
if not isinstance(env, dict):
    fail("it has no EnvironmentVariables dictionary")
if not isinstance(args, list) or not args or not isinstance(args[0], str) or not args[0]:
    fail("it has no program in ProgramArguments")
values = {"ProgramArguments": args[0]}
for name in sys.argv[2:]:
    value = env.get(name)
    if value is None or value == "":
        continue
    if not isinstance(value, str):
        fail(f"{name} is not a string")
    values[name] = value
for name, value in values.items():
    if "\n" in value or "\r" in value or "\0" in value:
        fail(f"{name} contains a line break")
    print(f"{name}={value}")
' "${plist_target}" "${settings[@]}" PATH)"; then
    print -u2 "Cannot read the existing launchd agent ${plist_target}: ${existing_text:-python3 failed}"
    print -u2 "The installer keeps an installed agent's settings and will not guess them. Fix the file, or remove it to install with the defaults and the settings you pass:"
    print -u2 "  launchctl bootout gui/${UID}/dev.gridops.tart-agent; rm '${plist_target}'"
    exit 1
  fi
  for line in "${(@f)existing_text}"; do
    [[ -n "${line}" ]] && existing[${line%%=*}]="${line#*=}"
  done
  # The install directory is where the program is. It counts as a setting of
  # its own only if it was not the default, the home's bin directory; that
  # one follows the home.
  existing_program="${existing[ProgramArguments]}"
  unset 'existing[ProgramArguments]'
  existing_install_dir="${existing_program:h}"
  if [[ "${existing_install_dir}" != "${existing[GRIDOPS_TART_AGENT_HOME]:-}/bin" ]]; then
    existing[GRIDOPS_TART_AGENT_INSTALL_DIR]="${existing_install_dir}"
  fi
fi

# keep_existing <NAME> <variable>: unless NAME was passed, gives <variable>
# the existing install's value of NAME, when it has one.
keep_existing() {
  local name="$1" variable="$2"
  [[ -z "${(P)name:-}" && -n "${existing[${name}]:-}" ]] || return 0
  defaults[${name}]="${(P)variable}"
  kept+=("${name}")
  typeset -g "${variable}=${existing[${name}]}"
}

# from_existing <NAME>: notes that a value came from the existing install.
from_existing() {
  (( ${kept[(Ie)$1]} )) && print -nr -- " (from the existing install)"
  return 0
}

agent_home="${GRIDOPS_TART_AGENT_HOME:-${HOME}/.gridops/tart-agent}"
keep_existing GRIDOPS_TART_AGENT_HOME agent_home
install_dir="${GRIDOPS_TART_AGENT_INSTALL_DIR:-${agent_home}/bin}"
keep_existing GRIDOPS_TART_AGENT_INSTALL_DIR install_dir
binary="${install_dir}/gridops-tart-agent"
token_file="${agent_home}/token"
tart_binary="${GRIDOPS_TART_BINARY:-$(command -v tart || true)}"
keep_existing GRIDOPS_TART_BINARY tart_binary
bind="${GRIDOPS_TART_AGENT_BIND:-127.0.0.1:8790}"
keep_existing GRIDOPS_TART_AGENT_BIND bind
network_mode="${GRIDOPS_TART_NETWORK_MODE:-softnet}"
keep_existing GRIDOPS_TART_NETWORK_MODE network_mode
runner_root="${GRIDOPS_TART_RUNNER_ROOT:-/Users/admin/actions-runner}"
keep_existing GRIDOPS_TART_RUNNER_ROOT runner_root
cpu_budget="${GRIDOPS_TART_CPU_BUDGET:-4}"
keep_existing GRIDOPS_TART_CPU_BUDGET cpu_budget
memory_budget="${GRIDOPS_TART_MEMORY_BUDGET_MB:-6144}"
keep_existing GRIDOPS_TART_MEMORY_BUDGET_MB memory_budget
max_runners="${GRIDOPS_TART_MAX_RUNNERS:-2}"
keep_existing GRIDOPS_TART_MAX_RUNNERS max_runners
min_free_disk="${GRIDOPS_TART_MIN_FREE_DISK_MB:-40960}"
keep_existing GRIDOPS_TART_MIN_FREE_DISK_MB min_free_disk
vm_slot_wait="${GRIDOPS_TART_VM_SLOT_WAIT_SECONDS:-60}"
keep_existing GRIDOPS_TART_VM_SLOT_WAIT_SECONDS vm_slot_wait
agent_token="${GRIDOPS_TART_AGENT_TOKEN:-}"

if (( ${kept[(Ie)GRIDOPS_TART_BINARY]} )) && [[ ! -x "${tart_binary}" ]]; then
  print -u2 "GRIDOPS_TART_BINARY is ${tart_binary}$(from_existing GRIDOPS_TART_BINARY), which is not an executable. Pass GRIDOPS_TART_BINARY with the path of tart (command -v tart)."
  exit 1
fi
if [[ -z "${tart_binary}" || ! -x "${tart_binary}" ]]; then
  print -u2 "Tart is not installed. Install it with: brew install openai/tools/tart"
  exit 1
fi
if [[ "${network_mode}" != "nat" && "${network_mode}" != "softnet" ]]; then
  print -u2 "GRIDOPS_TART_NETWORK_MODE is ${network_mode}$(from_existing GRIDOPS_TART_NETWORK_MODE), but it must be nat or softnet."
  exit 1
fi
if [[ "${network_mode}" == "softnet" ]] && ! command -v softnet >/dev/null 2>&1; then
  print -u2 "Softnet mode was requested but softnet is not installed in PATH."
  print -u2 "Install and privilege openai/softnet, or explicitly use GRIDOPS_TART_NETWORK_MODE=nat."
  exit 1
fi
softnet_command="$(command -v softnet || true)"
if [[ "${network_mode}" == "softnet" ]]; then
  softnet_binary="$(realpath "${softnet_command}")"
  if [[ "$(stat -f '%u' "${softnet_binary}")" != "0" || ! -u "${softnet_binary}" ]]; then
    print -u2 "Softnet is installed but is not owned by root with its SUID bit set."
    print -u2 "Run: sudo chown root:wheel ${softnet_binary}"
    print -u2 "Then: sudo chmod u+s ${softnet_binary}"
    exit 1
  fi
fi

# Tart runs softnet by looking it up on its own PATH, which it inherits from
# the agent, and launchd gives a job that sets no PATH only /usr/bin:/bin:
# /usr/sbin:/sbin. So the agent's PATH starts with the directories Tart and
# Softnet were found in (the directory of the link Homebrew makes, not of its
# versioned target, so it survives upgrades), each once.
agent_path=""
path_dirs=("${tart_binary:a:h}")
if [[ -n "${softnet_command}" ]]; then
  path_dirs+=("${softnet_command:a:h}")
fi
for dir in "${path_dirs[@]}" /usr/bin /bin /usr/sbin /sbin; do
  if [[ "${dir}" == *:* ]]; then
    print -u2 "Warning: ${dir} contains a colon and cannot be put on the agent's PATH."
    continue
  fi
  if [[ ":${agent_path}:" != *":${dir}:"* ]]; then
    agent_path="${agent_path:+${agent_path}:}${dir}"
  fi
done

# The runner manager waits 900 seconds for a macOS runner start; after the wait
# the agent needs up to 300 more to boot the VM and start the runner. The agent
# refuses a longer wait at startup too.
if [[ ! "${vm_slot_wait}" =~ '^[0-9]{1,4}$' ]] || (( vm_slot_wait > 600 )); then
  print -u2 "GRIDOPS_TART_VM_SLOT_WAIT_SECONDS is ${vm_slot_wait}$(from_existing GRIDOPS_TART_VM_SLOT_WAIT_SECONDS), but it must be a whole number of seconds from 0 to 600: the runner manager waits 900 seconds for a macOS runner start, and after the wait the agent needs up to 300 seconds to boot the VM and start the runner."
  exit 1
fi
if [[ ! "${bind}" =~ '^[A-Za-z0-9.:-]+$' ]]; then
  print -u2 "GRIDOPS_TART_AGENT_BIND$(from_existing GRIDOPS_TART_AGENT_BIND) contains unsupported characters."
  exit 1
fi
if (( ${#agent_token} < 32 )); then
  print -u2 "Set GRIDOPS_TART_AGENT_TOKEN to an independently generated token of at least 32 characters."
  exit 1
fi

# What this run keeps and what it changes in the existing install.
if (( existing_install )); then
  typeset -A new_values=(
    GRIDOPS_TART_AGENT_HOME "${agent_home}"
    GRIDOPS_TART_AGENT_INSTALL_DIR "${install_dir}"
    GRIDOPS_TART_AGENT_BIND "${bind}"
    GRIDOPS_TART_BINARY "${tart_binary}"
    GRIDOPS_TART_RUNNER_ROOT "${runner_root}"
    GRIDOPS_TART_NETWORK_MODE "${network_mode}"
    GRIDOPS_TART_CPU_BUDGET "${cpu_budget}"
    GRIDOPS_TART_MEMORY_BUDGET_MB "${memory_budget}"
    GRIDOPS_TART_MAX_RUNNERS "${max_runners}"
    GRIDOPS_TART_MIN_FREE_DISK_MB "${min_free_disk}"
    GRIDOPS_TART_VM_SLOT_WAIT_SECONDS "${vm_slot_wait}"
    PATH "${agent_path}"
  )
  existing[GRIDOPS_TART_AGENT_INSTALL_DIR]="${existing_install_dir}"
  for name in "${settings[@]}" PATH; do
    value="${new_values[${name}]}"
    if (( ${kept[(Ie)${name}]} )); then
      note=""
      if [[ -n "${defaults[${name}]}" && "${defaults[${name}]}" != "${value}" ]]; then
        note=" (the default is ${defaults[${name}]})"
      fi
      print -r -- "kept ${name}=${value} from the existing install${note}"
    elif [[ -z "${existing[${name}]:-}" ]]; then
      print -r -- "set ${name} to ${value} (not in the existing install)"
    elif [[ "${existing[${name}]}" != "${value}" ]]; then
      print -r -- "set ${name} ${existing[${name}]} -> ${value}"
    fi
  done
fi

mkdir -p "${agent_home}/logs" "${install_dir}" "${HOME}/Library/LaunchAgents"
chmod 700 "${agent_home}"

print "Building the native Tart agent…"
(cd "${repo_root}" && cargo build --release --locked -p gridops-tart-agent)
/usr/bin/install -m 0755 "${repo_root}/target/release/gridops-tart-agent" "${binary}"
umask 077
print -r -- "${agent_token}" > "${token_file}"
chmod 600 "${token_file}"

escape_sed() {
  print -nr -- "$1" | sed 's/[&|\\]/\\&/g'
}

# A value for a plist <string>, escaped for XML and then for sed's replacement.
plist_value() {
  local value="$1"
  value="${value//&/&amp;}"
  value="${value//</&lt;}"
  value="${value//>/&gt;}"
  escape_sed "${value}"
}

sed \
  -e "s|__BINARY__|$(plist_value "${binary}")|g" \
  -e "s|__TOKEN_FILE__|$(plist_value "${token_file}")|g" \
  -e "s|__AGENT_HOME__|$(plist_value "${agent_home}")|g" \
  -e "s|__BIND__|$(plist_value "${bind}")|g" \
  -e "s|__TART_BINARY__|$(plist_value "${tart_binary}")|g" \
  -e "s|__PATH__|$(plist_value "${agent_path}")|g" \
  -e "s|__RUNNER_ROOT__|$(plist_value "${runner_root}")|g" \
  -e "s|__NETWORK_MODE__|$(plist_value "${network_mode}")|g" \
  -e "s|__CPU_BUDGET__|$(plist_value "${cpu_budget}")|g" \
  -e "s|__MEMORY_BUDGET__|$(plist_value "${memory_budget}")|g" \
  -e "s|__MAX_RUNNERS__|$(plist_value "${max_runners}")|g" \
  -e "s|__MIN_FREE_DISK__|$(plist_value "${min_free_disk}")|g" \
  -e "s|__VM_SLOT_WAIT__|$(plist_value "${vm_slot_wait}")|g" \
  -e "s|__STDOUT_LOG__|$(plist_value "${agent_home}/logs/agent.log")|g" \
  -e "s|__STDERR_LOG__|$(plist_value "${agent_home}/logs/agent.error.log")|g" \
  "${plist_source}" > "${plist_target}"
chmod 600 "${plist_target}"
plutil -lint "${plist_target}" >/dev/null

launchctl bootout "gui/${UID}/dev.gridops.tart-agent" 2>/dev/null || true
launchctl bootstrap "gui/${UID}" "${plist_target}"
launchctl kickstart -k "gui/${UID}/dev.gridops.tart-agent"

print "Installed gridops-tart-agent at ${binary}."
print "The bearer token is stored with mode 0600 at ${token_file}."
