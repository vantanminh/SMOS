#!/usr/bin/env bash
# SMOS one-command installer (Linux / Ubuntu VPS).
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/vantanminh/SMOS/main/scripts/install.sh | bash
#
# Env overrides (for automation / non-root tests):
#   SMOS_REF=main              git ref to clone
#   SMOS_REPO=https://github.com/vantanminh/SMOS.git
#   SMOS_PREFIX=/opt/smos      install prefix (binary + static)
#   SMOS_DATA_DIR=/var/lib/smos
#   SMOS_BIND=127.0.0.1:9090
#   SMOS_SKIP_DEPS=1           skip apt packages
#   SMOS_SKIP_SERVICE=1        skip systemd unit / enable
#   SMOS_SKIP_BUILD=0          set 1 only when SMOS_BIN_SRC points at a prebuilt binary
#   SMOS_BIN_SRC=              optional path to prebuilt smos binary
#   SMOS_SRC_DIR=              optional existing source tree (skip clone)
#   SMOS_NONINTERACTIVE=1      default for curl|bash

set -euo pipefail

SMOS_REF="${SMOS_REF:-main}"
SMOS_REPO="${SMOS_REPO:-https://github.com/vantanminh/SMOS.git}"
SMOS_PREFIX="${SMOS_PREFIX:-/opt/smos}"
SMOS_DATA_DIR="${SMOS_DATA_DIR:-/var/lib/smos}"
SMOS_BIND="${SMOS_BIND:-127.0.0.1:9090}"
SMOS_SKIP_DEPS="${SMOS_SKIP_DEPS:-0}"
SMOS_SKIP_SERVICE="${SMOS_SKIP_SERVICE:-0}"
SMOS_SKIP_BUILD="${SMOS_SKIP_BUILD:-0}"
SMOS_BIN_SRC="${SMOS_BIN_SRC:-}"
SMOS_SRC_DIR="${SMOS_SRC_DIR:-}"
SMOS_NONINTERACTIVE="${SMOS_NONINTERACTIVE:-1}"
SMOS_USER="${SMOS_USER:-smos}"
SMOS_BUILD_DIR="${SMOS_BUILD_DIR:-}"

log() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

is_root() { [ "$(id -u)" -eq 0 ]; }

run_as_root() {
  if is_root; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    die "root or sudo required for: $*"
  fi
}

detect_os() {
  case "$(uname -s)" in
    Linux) ;;
    *) die "this installer targets Linux (got $(uname -s)). See README for manual install." ;;
  esac
}

install_deps() {
  if [ "$SMOS_SKIP_DEPS" = "1" ]; then
    log "skipping system deps (SMOS_SKIP_DEPS=1)"
    return 0
  fi
  if command -v apt-get >/dev/null 2>&1; then
    log "installing build dependencies via apt"
    run_as_root apt-get update -y
    DEBIAN_FRONTEND=noninteractive run_as_root apt-get install -y \
      build-essential curl git pkg-config ca-certificates
  elif command -v dnf >/dev/null 2>&1; then
    log "installing build dependencies via dnf"
    run_as_root dnf install -y gcc gcc-c++ make curl git pkgconf-pkg-config ca-certificates
  elif command -v yum >/dev/null 2>&1; then
    log "installing build dependencies via yum"
    run_as_root yum install -y gcc gcc-c++ make curl git pkgconfig ca-certificates
  else
    log "no supported package manager; assuming curl git and a C toolchain exist"
  fi
  need_cmd curl
  need_cmd git
}

ensure_rust() {
  if command -v cargo >/dev/null 2>&1 && command -v rustc >/dev/null 2>&1; then
    log "rust already available: $(rustc --version)"
    return 0
  fi
  log "installing Rust via rustup"
  need_cmd curl
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck disable=SC1091
  if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck source=/dev/null
    . "$HOME/.cargo/env"
  fi
  export PATH="${HOME}/.cargo/bin:${PATH}"
  need_cmd cargo
  need_cmd rustc
  log "rust installed: $(rustc --version)"
}

obtain_source() {
  if [ -n "$SMOS_SRC_DIR" ]; then
    [ -d "$SMOS_SRC_DIR" ] || die "SMOS_SRC_DIR not a directory: $SMOS_SRC_DIR"
    [ -f "$SMOS_SRC_DIR/Cargo.toml" ] || die "SMOS_SRC_DIR missing Cargo.toml: $SMOS_SRC_DIR"
    BUILD_SRC="$SMOS_SRC_DIR"
    log "using existing source at $BUILD_SRC"
    return 0
  fi
  if [ -n "$SMOS_BUILD_DIR" ]; then
    BUILD_SRC="$SMOS_BUILD_DIR"
  else
    BUILD_SRC="$(mktemp -d "${TMPDIR:-/tmp}/smos-src.XXXXXX")"
    CLEANUP_SRC=1
  fi
  log "cloning ${SMOS_REPO}@${SMOS_REF} into $BUILD_SRC"
  if [ -d "$BUILD_SRC/.git" ]; then
    git -C "$BUILD_SRC" fetch --depth 1 origin "$SMOS_REF"
    git -C "$BUILD_SRC" checkout -q FETCH_HEAD || git -C "$BUILD_SRC" checkout -q "$SMOS_REF"
  else
    mkdir -p "$BUILD_SRC"
    git clone --depth 1 --branch "$SMOS_REF" "$SMOS_REPO" "$BUILD_SRC" \
      || git clone --depth 1 "$SMOS_REPO" "$BUILD_SRC"
    if [ ! -f "$BUILD_SRC/Cargo.toml" ]; then
      die "clone did not produce Cargo.toml in $BUILD_SRC"
    fi
  fi
  [ -f "$BUILD_SRC/Cargo.toml" ] || die "source tree incomplete: $BUILD_SRC"
}

build_binary() {
  if [ "$SMOS_SKIP_BUILD" = "1" ]; then
    [ -n "$SMOS_BIN_SRC" ] || die "SMOS_SKIP_BUILD=1 requires SMOS_BIN_SRC"
    [ -f "$SMOS_BIN_SRC" ] || die "SMOS_BIN_SRC not found: $SMOS_BIN_SRC"
    BUILT_BIN="$SMOS_BIN_SRC"
    log "using prebuilt binary $BUILT_BIN"
    return 0
  fi
  need_cmd cargo
  log "building release binary (this may take a few minutes)"
  (
    cd "$BUILD_SRC"
    # Prefer cargo from rustup path
    export PATH="${HOME}/.cargo/bin:${PATH}"
    cargo build --release
  )
  BUILT_BIN="$BUILD_SRC/target/release/smos"
  [ -x "$BUILT_BIN" ] || die "build failed: missing $BUILT_BIN"
  log "built $BUILT_BIN"
}

install_files() {
  log "installing to $SMOS_PREFIX (data: $SMOS_DATA_DIR)"
  run_as_root mkdir -p "$SMOS_PREFIX" "$SMOS_DATA_DIR"
  run_as_root cp "$BUILT_BIN" "$SMOS_PREFIX/smos"
  run_as_root chmod 755 "$SMOS_PREFIX/smos"

  STATIC_SRC=""
  if [ -d "$BUILD_SRC/static" ]; then
    STATIC_SRC="$BUILD_SRC/static"
  elif [ -n "${SMOS_STATIC_SRC:-}" ] && [ -d "$SMOS_STATIC_SRC" ]; then
    STATIC_SRC="$SMOS_STATIC_SRC"
  fi
  if [ -n "$STATIC_SRC" ]; then
    run_as_root rm -rf "$SMOS_PREFIX/static"
    run_as_root cp -a "$STATIC_SRC" "$SMOS_PREFIX/static"
    log "installed WebUI assets from $STATIC_SRC"
  else
    die "static WebUI assets not found next to source"
  fi

  # Optional service user (best-effort)
  if ! id -u "$SMOS_USER" >/dev/null 2>&1; then
    if command -v useradd >/dev/null 2>&1; then
      run_as_root useradd -r -s /usr/sbin/nologin "$SMOS_USER" 2>/dev/null \
        || run_as_root useradd -r -s /bin/false "$SMOS_USER" 2>/dev/null \
        || log "could not create user $SMOS_USER; service may run as root"
    fi
  fi
  if id -u "$SMOS_USER" >/dev/null 2>&1; then
    run_as_root chown -R "${SMOS_USER}:${SMOS_USER}" "$SMOS_DATA_DIR" 2>/dev/null \
      || run_as_root chown -R "$SMOS_USER" "$SMOS_DATA_DIR" || true
  fi
}

write_env_file() {
  local env_file="${SMOS_ENV_FILE:-/etc/smos.env}"
  if [ -f "$env_file" ]; then
    log "keeping existing env file $env_file"
    return 0
  fi
  local token
  token="$(tr -dc 'A-Za-z0-9' </dev/urandom 2>/dev/null | head -c 32 || true)"
  if [ -z "$token" ]; then
    token="change-me-$(date +%s)"
  fi
  log "writing $env_file"
  run_as_root tee "$env_file" >/dev/null <<EOF
SMOS_AUTH_TOKEN=${token}
RUST_LOG=info
EOF
  run_as_root chmod 600 "$env_file"
}

install_systemd() {
  if [ "$SMOS_SKIP_SERVICE" = "1" ]; then
    log "skipping systemd (SMOS_SKIP_SERVICE=1)"
    return 0
  fi
  if ! command -v systemctl >/dev/null 2>&1; then
    log "systemctl not found; skip service install"
    return 0
  fi
  write_env_file
  local unit=/etc/systemd/system/smos.service
  local run_user=root
  if id -u "$SMOS_USER" >/dev/null 2>&1; then
    run_user="$SMOS_USER"
  fi
  log "installing systemd unit $unit (User=$run_user)"
  run_as_root tee "$unit" >/dev/null <<EOF
[Unit]
Description=SMOS Server Management OS
After=network.target

[Service]
Type=simple
User=${run_user}
WorkingDirectory=${SMOS_PREFIX}
EnvironmentFile=-/etc/smos.env
Environment=RUST_LOG=info
ExecStart=${SMOS_PREFIX}/smos --bind ${SMOS_BIND} --data-dir ${SMOS_DATA_DIR} --host-label %H
Restart=on-failure

[Install]
WantedBy=multi-user.target
EOF
  run_as_root systemctl daemon-reload
  run_as_root systemctl enable smos
  run_as_root systemctl restart smos || run_as_root systemctl start smos
  log "systemd service smos enabled and started"
}

smoke_check() {
  local bin="$SMOS_PREFIX/smos"
  [ -x "$bin" ] || die "installed binary missing: $bin"
  if "$bin" --help >/dev/null 2>&1 || "$bin" -h >/dev/null 2>&1; then
    log "smoke: $bin --help ok"
  else
    # Binary may not support --help; try version via running health if service up
    log "smoke: binary present at $bin"
  fi
  if [ "$SMOS_SKIP_SERVICE" != "1" ] && command -v curl >/dev/null 2>&1; then
    local host port
    host="${SMOS_BIND%:*}"
    port="${SMOS_BIND##*:}"
    # localhost bind may use 127.0.0.1
    sleep 1
    if curl -fsS --max-time 3 "http://${host}:${port}/api/health" >/dev/null 2>&1; then
      log "smoke: health endpoint ok at http://${host}:${port}/api/health"
    else
      log "smoke: health not reachable yet (service may need a moment); try: curl http://${host}:${port}/api/health"
    fi
  fi
}

print_done() {
  cat <<EOF

SMOS installed.

  Binary:   ${SMOS_PREFIX}/smos
  WebUI:    ${SMOS_PREFIX}/static
  Data:     ${SMOS_DATA_DIR}
  Bind:     ${SMOS_BIND}

  Dashboard: http://${SMOS_BIND}/
  Health:    http://${SMOS_BIND}/api/health

  Service:   systemctl status smos
  Logs:      journalctl -u smos -f

One-liner reinstall:
  curl -fsSL https://raw.githubusercontent.com/vantanminh/SMOS/${SMOS_REF}/scripts/install.sh | bash

EOF
}

cleanup() {
  if [ "${CLEANUP_SRC:-0}" = "1" ] && [ -n "${BUILD_SRC:-}" ] && [ -d "$BUILD_SRC" ]; then
    # Keep build dir on failure for debugging if SMOS_KEEP_SRC=1
    if [ "${SMOS_KEEP_SRC:-0}" != "1" ]; then
      rm -rf "$BUILD_SRC" || true
    fi
  fi
}

main() {
  CLEANUP_SRC=0
  BUILD_SRC=""
  BUILT_BIN=""
  trap cleanup EXIT

  log "SMOS installer starting (ref=${SMOS_REF})"
  detect_os
  install_deps
  if [ "$SMOS_SKIP_BUILD" != "1" ]; then
    ensure_rust
    obtain_source
  else
    # still need static assets
    if [ -z "$SMOS_SRC_DIR" ] && [ -z "${SMOS_STATIC_SRC:-}" ]; then
      obtain_source
    elif [ -n "$SMOS_SRC_DIR" ]; then
      BUILD_SRC="$SMOS_SRC_DIR"
    fi
  fi
  build_binary
  install_files
  install_systemd
  smoke_check
  print_done
}

main "$@"
