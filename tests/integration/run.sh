#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
FIXTURES_DIR="$ROOT_DIR/tests/integration/fixtures"
ARTIFACTS_ROOT="$ROOT_DIR/artifacts/integration"

usage() {
  cat <<'EOF'
Usage: run.sh [--case NAME]

Environment:
  GIFTWRAP_INTEGRATION=1   required safety switch
  GIFTWRAP_BIN             path to giftwrap binary (optional)
  GW_IMAGE                 base image (default: debian:bookworm-slim)
  GW_IMAGE_ALT             override image (default: debian:bookworm)
  RUN_ID                   artifacts run id (default: UTC timestamp)
EOF
}

if [[ "${GIFTWRAP_INTEGRATION:-}" != "1" ]]; then
  echo "Refusing to run: set GIFTWRAP_INTEGRATION=1" >&2
  exit 1
fi

if ! command -v podman >/dev/null 2>&1; then
  echo "podman not found; install podman before running integration tests" >&2
  exit 1
fi

GW_IMAGE=${GW_IMAGE:-docker.io/library/debian:bookworm-slim}
GW_IMAGE_ALT=${GW_IMAGE_ALT:-docker.io/library/debian:bookworm}

strip_tag() {
  local image="$1"
  local tail="${image##*/}"
  if [[ "$tail" == *:* ]]; then
    echo "${image%:*}"
  else
    echo "$image"
  fi
}

GW_IMAGE_BASE=$(strip_tag "$GW_IMAGE")

RUN_ID=${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}
ARTIFACTS_DIR="$ARTIFACTS_ROOT/$RUN_ID"
mkdir -p "$ARTIFACTS_DIR"

find_giftwrap_bin() {
  if [[ -n "${GIFTWRAP_BIN:-}" ]]; then
    echo "$GIFTWRAP_BIN"
    return 0
  fi

  local candidate
  for candidate in \
    "$ROOT_DIR"/target/*-unknown-linux-musl/release/giftwrap \
    "$ROOT_DIR"/target/*-unknown-linux-musl/debug/giftwrap \
    "$ROOT_DIR"/target/release/giftwrap \
    "$ROOT_DIR"/target/debug/giftwrap; do
    if [[ -x "$candidate" ]]; then
      echo "$candidate"
      return 0
    fi
  done

  if command -v giftwrap >/dev/null 2>&1; then
    command -v giftwrap
    return 0
  fi
  return 1
}

find_musl_bin() {
  local candidate
  for candidate in \
    "$ROOT_DIR"/target/*-unknown-linux-musl/release/giftwrap \
    "$ROOT_DIR"/target/*-unknown-linux-musl/debug/giftwrap; do
    if [[ -x "$candidate" ]]; then
      echo "$candidate"
      return 0
    fi
  done
  return 1
}

build_musl_bin() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found; cannot build musl giftwrap" >&2
    return 1
  fi
  echo "Building musl giftwrap agent..." >&2
  (cd "$ROOT_DIR" && cargo build --release --target x86_64-unknown-linux-musl)
}

is_static_binary() {
  local bin="$1"
  if ! command -v ldd >/dev/null 2>&1; then
    return 1
  fi
  local output
  output=$(ldd "$bin" 2>&1 || true)
  case "$output" in
    *"not a dynamic executable"*|*"statically linked"*)
      return 0
      ;;
  esac
  return 1
}

GIFTWRAP_BIN=$(find_giftwrap_bin || true)
if [[ -z "${GIFTWRAP_BIN:-}" ]]; then
  echo "giftwrap binary not found; build with cargo or set GIFTWRAP_BIN" >&2
  exit 1
fi
if [[ ! -x "$GIFTWRAP_BIN" ]]; then
  echo "giftwrap binary is not executable: $GIFTWRAP_BIN" >&2
  exit 1
fi

AGENT_BIN=${GW_AGENT_BIN:-}
if [[ -z "${AGENT_BIN:-}" ]]; then
  if is_static_binary "$GIFTWRAP_BIN"; then
    AGENT_BIN="$GIFTWRAP_BIN"
  fi
fi
if [[ -z "${AGENT_BIN:-}" ]]; then
  AGENT_BIN=$(find_musl_bin || true)
fi
if [[ -z "${AGENT_BIN:-}" && -z "${GW_AGENT_BIN:-}" ]]; then
  build_musl_bin || true
  AGENT_BIN=$(find_musl_bin || true)
fi
if [[ -z "${AGENT_BIN:-}" ]]; then
  cat <<'EOF' >&2
No container-compatible giftwrap agent found.
Build a static musl binary (recommended):
  cargo build --release --target x86_64-unknown-linux-musl
Or set GW_AGENT_BIN to a compatible giftwrap binary path.
EOF
  exit 1
fi
if command -v realpath >/dev/null 2>&1; then
  AGENT_BIN=$(realpath "$AGENT_BIN")
elif command -v readlink >/dev/null 2>&1; then
  AGENT_BIN=$(readlink -f "$AGENT_BIN")
fi
if [[ ! -x "$AGENT_BIN" ]]; then
  echo "agent binary is not executable: $AGENT_BIN" >&2
  exit 1
fi

read_lines() {
  local file="$1"
  local -n out="$2"
  out=()
  while IFS= read -r line || [[ -n "$line" ]]; do
    line=${line%$'\r'}
    if [[ -z "$line" ]]; then
      continue
    fi
    if [[ "${line:0:1}" == "#" ]]; then
      continue
    fi
    out+=("$line")
  done < "$file"
}

resolve_git_dir_path() {
  local raw
  raw=$(git -C "$ROOT_DIR" rev-parse --git-common-dir 2>/dev/null) || return 1
  if [[ "$raw" == /* ]]; then
    echo "$raw"
    return 0
  fi
  if command -v realpath >/dev/null 2>&1; then
    realpath "$ROOT_DIR/$raw"
  else
    readlink -f "$ROOT_DIR/$raw"
  fi
}

render_arg() {
  local arg="$1"
  arg=${arg//\{\{GW_IMAGE\}\}/$GW_IMAGE}
  arg=${arg//\{\{GW_IMAGE_ALT\}\}/$GW_IMAGE_ALT}
  arg=${arg//\{\{GW_IMAGE_BASE\}\}/$GW_IMAGE_BASE}
  arg=${arg//\{\{CTX_SHA\}\}/$CTX_SHA}
  arg=${arg//\{\{GIT_DIR_PATH\}\}/$GIT_DIR_PATH}
  printf '%s' "$arg"
}

render_env_entry() {
  local entry="$1"
  if [[ "$entry" == *"="* ]]; then
    local key="${entry%%=*}"
    local value="${entry#*=}"
    printf '%s=%s' "$key" "$(render_arg "$value")"
  else
    render_arg "$entry"
  fi
}

write_cmd_file() {
  local file="$1"
  shift
  local -a env_vars=()
  while [[ "$1" != "--" ]]; do
    env_vars+=("$1")
    shift
  done
  shift
  local -a args=("$@")

  {
    printf 'env'
    for env in "${env_vars[@]}"; do
      if [[ -n "$env" ]]; then
        printf ' %q' "$env"
      fi
    done
    printf ' %q' "$GIFTWRAP_BIN"
    for arg in "${args[@]}"; do
      printf ' %q' "$arg"
    done
    printf '\n'
  } > "$file"
}

run_with_env() {
  local out_stdout="$1"
  local out_stderr="$2"
  local out_exit="$3"
  shift 3
  local -a env_vars=()
  while [[ "$1" != "--" ]]; do
    env_vars+=("$1")
    shift
  done
  shift
  local -a args=("$@")

  set +e
  (cd "$CASE_DIR" && env "${env_vars[@]}" "$GIFTWRAP_BIN" "${args[@]}") \
    >"$out_stdout" 2>"$out_stderr"
  local status=$?
  set -e
  printf '%s\n' "$status" > "$out_exit"
  return 0
}

run_step() {
  local step_dir="$1"
  local step_out="$2"

  local -a raw_args
  read_lines "$step_dir/args.txt" raw_args
  if [[ ${#raw_args[@]} -eq 0 ]]; then
    echo "No args in $step_dir/args.txt" >&2
    exit 1
  fi

  local -a env_vars=()
  local -a step_env_raw=()
  if [[ -f "$step_dir/env.txt" ]]; then
    read_lines "$step_dir/env.txt" step_env_raw
  fi

  local has_container_override="false"
  local has_agent_override="false"
  local env_entry
  for env_entry in "${env_vars[@]}"; do
    if [[ "$env_entry" == GW_USER_OPT_SET_gw_container=* ]]; then
      has_container_override="true"
    fi
    if [[ "$env_entry" == GW_USER_OPT_SET_gw_agent=* ]]; then
      has_agent_override="true"
    fi
  done
  if [[ -z "${GW_USER_OPT_SET_gw_container:-}" && "$has_container_override" == "false" ]]; then
    env_vars+=("GW_USER_OPT_SET_gw_container=$GW_IMAGE")
  fi
  if [[ -z "${GW_USER_OPT_SET_gw_agent:-}" && "$has_agent_override" == "false" ]]; then
    env_vars+=("GW_USER_OPT_SET_gw_agent=$AGENT_BIN")
  fi

  CTX_SHA=""
  GIT_DIR_PATH=""

  local needs_ctx="false"
  local needs_git="false"
  for arg in "${raw_args[@]}"; do
    if [[ "$arg" == *"{{CTX_SHA}}"* ]]; then
      needs_ctx="true"
    fi
    if [[ "$arg" == *"{{GIT_DIR_PATH}}"* ]]; then
      needs_git="true"
    fi
  done
  local env_entry
  for env_entry in "${step_env_raw[@]}"; do
    if [[ "$env_entry" == *"{{CTX_SHA}}"* ]]; then
      needs_ctx="true"
    fi
    if [[ "$env_entry" == *"{{GIT_DIR_PATH}}"* ]]; then
      needs_git="true"
    fi
  done

  if [[ "$needs_ctx" == "true" ]]; then
    local ctx_out="$step_out/ctx-sha.txt"
    local ctx_err="$step_out/ctx-sha.stderr.txt"
    set +e
    (cd "$CASE_DIR" && env "${env_vars[@]}" "$GIFTWRAP_BIN" --gw-ctx) \
      >"$ctx_out" 2>"$ctx_err"
    local ctx_status=$?
    set -e
    if [[ $ctx_status -ne 0 ]]; then
      printf '%s\n' "$ctx_status" > "$step_out/ctx-sha.exit-code"
      echo "Context sha failed for $CASE_NAME (see $ctx_err)" >&2
      exit 1
    fi
    read -r CTX_SHA < "$ctx_out"
    CTX_SHA=${CTX_SHA%$'\r'}
  fi

  if [[ "$needs_git" == "true" ]]; then
    GIT_DIR_PATH=$(resolve_git_dir_path || true)
    if [[ -z "$GIT_DIR_PATH" ]]; then
      echo "Failed to resolve git dir for $CASE_NAME" >&2
      exit 1
    fi
  fi

  for env_entry in "${step_env_raw[@]}"; do
    env_vars+=("$(render_env_entry "$env_entry")")
  done

  local -a args=()
  local arg
  for arg in "${raw_args[@]}"; do
    args+=("$(render_arg "$arg")")
  done

  local print_out="$step_out/runtime-args.txt"
  local print_err="$step_out/runtime-args.stderr.txt"
  local print_exit="$step_out/runtime-args.exit-code"

  local -a print_args=("--gw-print" "${args[@]}")
  run_with_env "$print_out" "$print_err" "$print_exit" "${env_vars[@]}" -- "${print_args[@]}"

  local cfg_out="$step_out/config.json"
  local cfg_err="$step_out/config.stderr.txt"
  local cfg_exit="$step_out/config.exit-code"
  run_with_env "$cfg_out" "$cfg_err" "$cfg_exit" "${env_vars[@]}" -- "--gw-show-config"

  write_cmd_file "$step_out/cmd.txt" "${env_vars[@]}" -- "${args[@]}"
  run_with_env "$step_out/stdout.txt" "$step_out/stderr.txt" "$step_out/exit-code" \
    "${env_vars[@]}" -- "${args[@]}"
}

run_case() {
  CASE_NAME="$1"
  CASE_DIR="$FIXTURES_DIR/$CASE_NAME"

  if [[ ! -d "$CASE_DIR" ]]; then
    echo "Unknown case: $CASE_NAME" >&2
    exit 1
  fi

  local case_out="$ARTIFACTS_DIR/$CASE_NAME"
  mkdir -p "$case_out"

  if [[ -f "$CASE_DIR/reset.txt" ]]; then
    local -a reset_paths
    read_lines "$CASE_DIR/reset.txt" reset_paths
    local rel
    for rel in "${reset_paths[@]}"; do
      rm -f "$CASE_DIR/$rel"
    done
  fi

  if [[ ! -d "$CASE_DIR/steps" ]]; then
    echo "Missing steps/ in $CASE_DIR" >&2
    exit 1
  fi

  local step_dir
  shopt -s nullglob
  local step_dirs=("$CASE_DIR"/steps/*)
  shopt -u nullglob
  if [[ ${#step_dirs[@]} -eq 0 ]]; then
    echo "No step directories found in $CASE_DIR/steps" >&2
    exit 1
  fi
  for step_dir in "${step_dirs[@]}"; do
    if [[ ! -d "$step_dir" ]]; then
      continue
    fi
    local step_name
    step_name=$(basename "$step_dir")
    local step_out="$case_out/$step_name"
    mkdir -p "$step_out"
    run_step "$step_dir" "$step_out"
  done
}

CASES=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --case)
      shift
      if [[ $# -eq 0 ]]; then
        echo "--case requires a name" >&2
        exit 1
      fi
      CASES+=("$1")
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

if [[ ${#CASES[@]} -eq 0 ]]; then
  for dir in "$FIXTURES_DIR"/*; do
    if [[ -d "$dir" ]]; then
      CASES+=("$(basename "$dir")")
    fi
  done
fi

for case in "${CASES[@]}"; do
  run_case "$case"
done

echo "Artifacts written to $ARTIFACTS_DIR"
find  "$ARTIFACTS_DIR" -name exit-code | xargs snail -m '"{$src}: {$text.strip()}"'
