#!/usr/bin/env bash
set -euo pipefail

if [[ -n "${BASH_VERSION:-}" ]]; then
  shopt -s inherit_errexit 2>/dev/null || true
fi

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PLATFORM="$(uname -s)"
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
RELEASE_BIN=""
LAUNCHER_DIR=""
LAUNCHER_PATH=""
INSTALLED_APP_DIR=""
INSTALLED_CLI=""
INSTALLED_HOST_LIB=""
NATIVE_DOCTOR_JSON=""

if [[ -t 1 ]]; then
  C_RESET=$'\033[0m'
  C_BOLD=$'\033[1m'
  C_BLUE=$'\033[34m'
  C_GREEN=$'\033[32m'
  C_YELLOW=$'\033[33m'
  C_RED=$'\033[31m'
else
  C_RESET=""
  C_BOLD=""
  C_BLUE=""
  C_GREEN=""
  C_YELLOW=""
  C_RED=""
fi

CURRENT_STEP=""

log_section() {
  printf '\n%s%s%s\n' "$C_BOLD$C_BLUE" "$1" "$C_RESET"
}

log_info() {
  printf '%s•%s %s\n' "$C_BLUE" "$C_RESET" "$1"
}

log_success() {
  printf '%s✓%s %s\n' "$C_GREEN" "$C_RESET" "$1"
}

log_warn() {
  printf '%s!%s %s\n' "$C_YELLOW" "$C_RESET" "$1"
}

log_error() {
  printf '%s✗%s %s\n' "$C_RED" "$C_RESET" "$1" >&2
}

fail() {
  log_error "$1"
  exit 1
}

on_error() {
  local exit_code=$?
  if [[ -n "$CURRENT_STEP" ]]; then
    log_error "Install failed while: $CURRENT_STEP"
  else
    log_error "Install failed."
  fi
  log_error "If you need more detail, rerun this script and watch the step immediately above the failure."
  exit "$exit_code"
}

trap on_error ERR

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "Missing required command: $1"
}

resolve_release_bin() {
  local -a candidates=(
    "$REPO_ROOT/target/release/aegis"
    "$REPO_ROOT/target/$HOST_TRIPLE/release/aegis"
  )
  local candidate
  for candidate in "${candidates[@]}"; do
    if [[ -x "$candidate" ]]; then
      RELEASE_BIN="$candidate"
      return 0
    fi
  done
  return 1
}

run_step() {
  CURRENT_STEP="$1"
  log_info "$CURRENT_STEP"
  shift
  "$@"
}

run_quiet_step() {
  CURRENT_STEP="$1"
  log_info "$CURRENT_STEP"
  shift
  local temp_log
  temp_log="$(mktemp)"
  if "$@" >"$temp_log" 2>&1; then
    rm -f "$temp_log"
    return 0
  fi
  cat "$temp_log" >&2
  rm -f "$temp_log"
  return 1
}

path_contains_dir() {
  local dir="$1"
  local entry
  IFS=':' read -r -a entries <<< "${PATH:-}"
  for entry in "${entries[@]}"; do
    if [[ "$entry" == "$dir" ]]; then
      return 0
    fi
  done
  return 1
}

select_launcher_dir() {
  local candidate
  local -a preferred_dirs=("$HOME/.cargo/bin" "$HOME/.local/bin" "$HOME/bin")
  local entry

  IFS=':' read -r -a entries <<< "${PATH:-}"
  for entry in "${entries[@]}"; do
    for candidate in "${preferred_dirs[@]}"; do
      if [[ "$entry" == "$candidate" ]]; then
        printf '%s\n' "$candidate"
        return 0
      fi
    done
  done

  for candidate in "${preferred_dirs[@]}"; do
    if [[ -d "$candidate" || -w "$(dirname "$candidate")" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done

  printf '%s\n' "$HOME/.local/bin"
}

resolve_path_command() {
  type -P "$1" 2>/dev/null || true
}

doctor_json_field() {
  local field_name="$1"
  AEGIS_NATIVE_DOCTOR_JSON="${NATIVE_DOCTOR_JSON}" python3 - "$field_name" <<'PY'
import json
import os
import sys

field = sys.argv[1]
data = json.loads(os.environ["AEGIS_NATIVE_DOCTOR_JSON"])
value = data.get(field)
if value is None:
    sys.exit(1)
if not isinstance(value, str):
    raise SystemExit(f"{field} is not a string field")
print(value)
PY
}

install_launcher() {
  mkdir -p "$LAUNCHER_DIR"
  cat >"$LAUNCHER_PATH" <<EOF
#!/usr/bin/env bash
set -euo pipefail

INSTALLED_CLI="$INSTALLED_CLI"
INSTALLED_HOST_LIB="$INSTALLED_HOST_LIB"

resolve_path() {
  python3 - "\$1" <<'PY'
import os
import sys

path = sys.argv[1]
print(os.path.realpath(path) if os.path.exists(path) else os.path.abspath(path))
PY
}

requested_host_lib=""
args=("\$@")
for ((i=0; i<\${#args[@]}; i++)); do
  case "\${args[\$i]}" in
    --host-lib=*)
      requested_host_lib="\${args[\$i]#--host-lib=}"
      ;;
    --host-lib)
      ((i+=1))
      if (( i >= \${#args[@]} )); then
        printf 'Missing value for --host-lib\n' >&2
        exit 64
      fi
      requested_host_lib="\${args[\$i]}"
      ;;
  esac
done

if [[ ! -x "\$INSTALLED_CLI" ]]; then
  printf 'Aegis is not installed at %s\n' "\$INSTALLED_CLI" >&2
  printf 'Run ./install.sh from the Aegis repo to refresh the canonical local release.\n' >&2
  exit 1
fi

if [[ -n "\$requested_host_lib" ]]; then
  requested_host_lib="\$(resolve_path "\$requested_host_lib")"
  installed_host_lib="\$(resolve_path "\$INSTALLED_HOST_LIB")"
  if [[ "\$requested_host_lib" != "\$installed_host_lib" ]]; then
    printf 'The canonical aegis launcher only supports the installed production host library.\n' >&2
    printf 'Requested: %s\n' "\$requested_host_lib" >&2
    printf 'Installed: %s\n' "\$installed_host_lib" >&2
    printf 'Use cargo run -- ... or the workspace binary directly for non-production host overrides.\n' >&2
    exit 2
  fi
fi

exec "\$INSTALLED_CLI" "\$@"
EOF
  chmod 0755 "$LAUNCHER_PATH"
}

print_summary() {
  local resolved_aegis
  resolved_aegis="$(resolve_path_command aegis)"
  log_section "Install Complete"
  log_success "Installed app dir: $INSTALLED_APP_DIR"
  log_success "Installed CLI: $INSTALLED_CLI"
  log_success "Canonical launcher: $LAUNCHER_PATH"
  log_success "Canonical state root: ${AEGIS_HOME:-$HOME/.aegis}"
  printf '\n'
  log_info "Open the browser with: aegis"
  log_info "Open explicitly with: aegis open"
  log_info "Run the automation server with: aegis serve --addr 127.0.0.1:7878"
  log_info "Inspect config with: aegis config get agent"
  log_info "Inspect secrets with: aegis config secrets-get --profile default"
  if ! path_contains_dir "$LAUNCHER_DIR"; then
    log_warn "Add $LAUNCHER_DIR to your PATH so 'aegis' resolves to the canonical installed launcher."
  elif [[ "$resolved_aegis" != "$LAUNCHER_PATH" ]]; then
    log_warn "'aegis' currently resolves to ${resolved_aegis:-another path}. Put $LAUNCHER_DIR earlier on PATH to prefer the installed release."
  fi
}

log_section "Aegis Installer"
log_info "Repo: $REPO_ROOT"
log_info "Platform: $PLATFORM"
log_info "This will build the release binary, install the local app, and bootstrap ~/.aegis."

CURRENT_STEP="checking local prerequisites"
require_command cargo
require_command cmake
require_command python3

if [[ "$PLATFORM" == "Darwin" ]]; then
  require_command codesign
fi

if [[ ! -d "$REPO_ROOT/third_party/cef" ]]; then
  fail "CEF SDK is missing under $REPO_ROOT/third_party/cef. Install the platform CEF bundle before running install."
fi

cd "$REPO_ROOT"
CURRENT_STEP="checking native preflight readiness"
NATIVE_DOCTOR_JSON="$(cargo run --quiet -- native doctor)"
INSTALLED_APP_DIR="$(doctor_json_field canonical_install_dir)" || fail "Unable to resolve canonical install dir from `aegis native doctor`."
INSTALLED_CLI="$(doctor_json_field canonical_install_cli)" || fail "Unable to resolve canonical install CLI from `aegis native doctor`."
INSTALLED_HOST_LIB="$(doctor_json_field canonical_install_host_library)" || fail "Unable to resolve canonical install host library from `aegis native doctor`."
LAUNCHER_DIR="$(select_launcher_dir)"
LAUNCHER_PATH="$LAUNCHER_DIR/aegis"

log_section "Build"
run_step "building release Rust binary" cargo build --release

if ! resolve_release_bin; then
  fail "Expected release binary at $REPO_ROOT/target/release/aegis or $REPO_ROOT/target/$HOST_TRIPLE/release/aegis after build, but it was not found."
fi

log_section "Install"
run_quiet_step "installing the local Aegis app" "$RELEASE_BIN" native install

if [[ ! -x "$INSTALLED_CLI" ]]; then
  fail "Expected installed CLI at $INSTALLED_CLI, but it was not found."
fi

run_step "installing the canonical shell launcher" install_launcher

log_section "Bootstrap State"
run_quiet_step "bootstrapping canonical Aegis config" "$INSTALLED_CLI" config get agent
run_quiet_step "bootstrapping canonical Aegis runtime config" "$INSTALLED_CLI" config get runtime
run_quiet_step "bootstrapping canonical Aegis secrets store" "$INSTALLED_CLI" config secrets-get --profile default

log_section "Verify"
run_quiet_step "verifying installed CLI surface" "$INSTALLED_CLI" config --help
run_quiet_step "verifying installed native paths" "$INSTALLED_CLI" native paths
run_quiet_step "verifying canonical launcher" "$LAUNCHER_PATH" usage

if [[ ! -d "${AEGIS_HOME:-$HOME/.aegis}" ]]; then
  fail "Expected canonical Aegis state directory at ${AEGIS_HOME:-$HOME/.aegis}, but it was not created."
fi

print_summary
