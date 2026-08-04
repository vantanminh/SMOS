#!/usr/bin/env bash
# SMOS one-command installer — downloads a prebuilt GitHub Release (no compile on server).
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/vantanminh/SMOS/main/scripts/install.sh | bash
#
# Env overrides:
#   SMOS_VERSION=latest|v0.1.0   release tag (default: latest)
#   SMOS_REPO=vantanminh/SMOS    GitHub owner/repo
#   SMOS_PREFIX=/opt/smos
#   SMOS_DATA_DIR=/var/lib/smos
#   SMOS_BIND=127.0.0.1:9090
#   SMOS_SKIP_DEPS=1             skip apt curl/ca-certificates
#   SMOS_SKIP_SERVICE=1          skip systemd
#   SMOS_FROM_SOURCE=1           fallback: clone + cargo build (slow; needs rustc)
#   SMOS_ASSET_URL=              override direct tarball URL
#   SMOS_USER=smos

set -euo pipefail

SMOS_VERSION="${SMOS_VERSION:-latest}"
SMOS_REPO="${SMOS_REPO:-vantanminh/SMOS}"
SMOS_PREFIX="${SMOS_PREFIX:-/opt/smos}"
SMOS_DATA_DIR="${SMOS_DATA_DIR:-/var/lib/smos}"
SMOS_BIND="${SMOS_BIND:-127.0.0.1:9090}"
SMOS_SKIP_DEPS="${SMOS_SKIP_DEPS:-0}"
SMOS_SKIP_SERVICE="${SMOS_SKIP_SERVICE:-0}"
SMOS_FROM_SOURCE="${SMOS_FROM_SOURCE:-0}"
SMOS_ASSET_URL="${SMOS_ASSET_URL:-}"
SMOS_USER="${SMOS_USER:-smos}"
SMOS_GITHUB_API="${SMOS_GITHUB_API:-https://api.github.com}"
SMOS_GITHUB_RAW="${SMOS_GITHUB_RAW:-https://github.com}"

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
    *) die "this installer targets Linux (got $(uname -s))." ;;
  esac
}

detect_target() {
  local arch
  arch="$(uname -m)"
  case "$arch" in
    x86_64|amd64) SMOS_TARGET="x86_64-unknown-linux-gnu" ;;
    aarch64|arm64) SMOS_TARGET="aarch64-unknown-linux-gnu" ;;
    *) die "unsupported architecture: $arch (need x86_64 or aarch64)" ;;
  esac
  log "detected target ${SMOS_TARGET}"
}

install_runtime_deps() {
  if [ "$SMOS_SKIP_DEPS" = "1" ]; then
    log "skipping runtime deps (SMOS_SKIP_DEPS=1)"
    return 0
  fi
  if command -v apt-get >/dev/null 2>&1; then
    log "installing runtime packages (curl ca-certificates tar)"
    run_as_root apt-get update -y
    DEBIAN_FRONTEND=noninteractive run_as_root apt-get install -y curl ca-certificates tar gzip
  elif command -v dnf >/dev/null 2>&1; then
    run_as_root dnf install -y curl ca-certificates tar gzip
  elif command -v yum >/dev/null 2>&1; then
    run_as_root yum install -y curl ca-certificates tar gzip
  else
    log "no apt/dnf/yum; assuming curl and tar exist"
  fi
  need_cmd curl
  need_cmd tar
}

# Resolve release tarball URL from GitHub Releases API.
resolve_asset_url() {
  if [ -n "$SMOS_ASSET_URL" ]; then
    ASSET_URL="$SMOS_ASSET_URL"
    log "using SMOS_ASSET_URL=${ASSET_URL}"
    return 0
  fi

  need_cmd curl
  local api_url json
  if [ "$SMOS_VERSION" = "latest" ]; then
    api_url="${SMOS_GITHUB_API}/repos/${SMOS_REPO}/releases/latest"
  else
    api_url="${SMOS_GITHUB_API}/repos/${SMOS_REPO}/releases/tags/${SMOS_VERSION}"
  fi

  log "fetching release metadata: ${api_url}"
  http_code=0
  json="$(curl -fsSL -w '\n%{http_code}' \
    -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    "${api_url}" 2>/dev/null)" || true
  # split body / code if -w appended
  if printf '%s' "$json" | tail -n1 | grep -Eq '^[0-9]{3}$'; then
    http_code="$(printf '%s' "$json" | tail -n1)"
    json="$(printf '%s' "$json" | sed '$d')"
  fi
  if [ -z "$json" ] || [ "$http_code" = "404" ]; then
    die "no GitHub Release found for ${SMOS_REPO} (${SMOS_VERSION}, HTTP ${http_code:-unknown}).
  CI creates a release automatically on every push to main (tag like v0.1.0-build.N).
  Check: https://github.com/${SMOS_REPO}/releases
  Actions: https://github.com/${SMOS_REPO}/actions
  After the Release workflow is green, re-run this installer.
  Or pin: SMOS_VERSION=v0.1.0-build.1
  Or build on server: SMOS_FROM_SOURCE=1"
  fi

  # Prefer python for JSON if present; else grep/sed best-effort.
  ASSET_URL=""
  ASSET_NAME=""
  RELEASE_TAG=""
  if command -v python3 >/dev/null 2>&1; then
    # Export target for the parser process
    parsed="$(
      SMOS_TARGET="$SMOS_TARGET" printf '%s' "$json" | SMOS_TARGET="$SMOS_TARGET" python3 -c '
import json, sys, os
target = os.environ.get("SMOS_TARGET", "")
data = json.load(sys.stdin)
print("RELEASE_TAG=" + repr(data.get("tag_name") or ""))
want = "-" + target + ".tar.gz"
url = ""
name = ""
for a in data.get("assets") or []:
    n = a.get("name") or ""
    if n.endswith(want) or (target and target in n and n.endswith(".tar.gz") and not n.endswith(".sha256")):
        url = a.get("browser_download_url") or ""
        name = n
        break
if not url:
    for a in data.get("assets") or []:
        n = a.get("name") or ""
        if n.startswith("smos-") and n.endswith(".tar.gz") and not n.endswith(".sha256"):
            url = a.get("browser_download_url") or ""
            name = n
            break
print("ASSET_URL=" + repr(url))
print("ASSET_NAME=" + repr(name))
'
    )"
    eval "$parsed"
  else
    RELEASE_TAG="$(printf '%s' "$json" | sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
    ASSET_URL="$(printf '%s' "$json" | grep -oE 'https://github.com/[^"]+/releases/download/[^"]+\.tar\.gz' | grep "${SMOS_TARGET}" | grep -v sha256 | head -n1 || true)"
    if [ -z "$ASSET_URL" ]; then
      ASSET_URL="$(printf '%s' "$json" | grep -oE 'https://github.com/[^"]+/releases/download/[^"]+\.tar\.gz' | grep -v sha256 | head -n1 || true)"
    fi
    ASSET_NAME="$(basename "$ASSET_URL" 2>/dev/null || true)"
  fi

  [ -n "$ASSET_URL" ] || die "no release tarball found for target ${SMOS_TARGET} in ${SMOS_REPO} (${SMOS_VERSION}). Publish a release first (git tag vX.Y.Z && git push --tags)."
  log "release=${RELEASE_TAG:-$SMOS_VERSION} asset=${ASSET_NAME:-unknown}"
  log "download ${ASSET_URL}"
}

download_and_extract() {
  WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/smos-install.XXXXXX")"
  local tar_path="${WORK_DIR}/smos.tgz"
  log "downloading to ${tar_path}"
  curl -fsSL -L -o "$tar_path" "$ASSET_URL" || die "download failed"

  log "extracting"
  tar -C "$WORK_DIR" -xzf "$tar_path"

  # Find binary: either staged dir smos-VER-target/smos or flat smos
  EXTRACTED_BIN=""
  EXTRACTED_STATIC=""
  if [ -x "${WORK_DIR}/smos" ] && [ ! -d "${WORK_DIR}/smos" ]; then
    EXTRACTED_BIN="${WORK_DIR}/smos"
  fi
  # shellcheck disable=SC2044
  for f in $(find "$WORK_DIR" -type f -name smos 2>/dev/null); do
    if [ -x "$f" ] || [ -f "$f" ]; then
      EXTRACTED_BIN="$f"
      break
    fi
  done
  [ -n "$EXTRACTED_BIN" ] || die "tarball missing smos binary"
  chmod +x "$EXTRACTED_BIN" || true

  local parent
  parent="$(dirname "$EXTRACTED_BIN")"
  if [ -d "${parent}/static" ]; then
    EXTRACTED_STATIC="${parent}/static"
  elif [ -d "${WORK_DIR}/static" ]; then
    EXTRACTED_STATIC="${WORK_DIR}/static"
  else
    # search
    EXTRACTED_STATIC="$(find "$WORK_DIR" -type d -name static | head -n1 || true)"
  fi
  [ -n "$EXTRACTED_STATIC" ] && [ -d "$EXTRACTED_STATIC" ] || die "tarball missing static/ WebUI assets"
  log "binary=${EXTRACTED_BIN}"
  log "static=${EXTRACTED_STATIC}"
}

# Stop running service so upgrades can replace the binary cleanly.
# (Linux returns "Text file busy" if you cp over an executing ELF.)
stop_smos_if_running() {
  if [ "$SMOS_SKIP_SERVICE" = "1" ]; then
    return 0
  fi
  if ! command -v systemctl >/dev/null 2>&1; then
    return 0
  fi
  if systemctl is-active --quiet smos 2>/dev/null; then
    log "stopping smos service for upgrade"
    run_as_root systemctl stop smos || log "warning: could not stop smos (will try atomic replace)"
  fi
}

# Install binary via temp file + rename. On Linux, `cp` onto a running
# executable fails with ETXTBSY; `mv` replaces the path atomically.
install_binary() {
  local dest="${SMOS_PREFIX}/smos"
  local tmp="${dest}.new.$$"
  run_as_root cp "$EXTRACTED_BIN" "$tmp"
  run_as_root chmod 755 "$tmp"
  # Prefer install(1) when available; fall back to mv -f.
  if command -v install >/dev/null 2>&1; then
    # install still overwrites in-place — use mv for the final step.
    run_as_root mv -f "$tmp" "$dest"
  else
    run_as_root mv -f "$tmp" "$dest"
  fi
  run_as_root chmod 755 "$dest"
}

install_files() {
  log "installing to ${SMOS_PREFIX} (data: ${SMOS_DATA_DIR})"
  stop_smos_if_running
  run_as_root mkdir -p "$SMOS_PREFIX" "$SMOS_DATA_DIR"
  install_binary
  run_as_root rm -rf "${SMOS_PREFIX}/static"
  run_as_root cp -a "$EXTRACTED_STATIC" "${SMOS_PREFIX}/static"

  if ! id -u "$SMOS_USER" >/dev/null 2>&1; then
    if command -v useradd >/dev/null 2>&1; then
      run_as_root useradd -r -s /usr/sbin/nologin "$SMOS_USER" 2>/dev/null \
        || run_as_root useradd -r -s /bin/false "$SMOS_USER" 2>/dev/null \
        || log "could not create user ${SMOS_USER}"
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
    log "keeping existing env file ${env_file}"
    return 0
  fi
  local token
  token="$(tr -dc 'A-Za-z0-9' </dev/urandom 2>/dev/null | head -c 32 || true)"
  if [ -z "$token" ]; then
    token="change-me-$(date +%s)"
  fi
  log "writing ${env_file}"
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
  log "installing systemd unit ${unit} (User=${run_user})"
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
  local bin="${SMOS_PREFIX}/smos"
  [ -x "$bin" ] || die "installed binary missing: ${bin}"
  log "smoke: binary present at ${bin}"
  if [ "$SMOS_SKIP_SERVICE" != "1" ] && command -v curl >/dev/null 2>&1; then
    local host port
    host="${SMOS_BIND%:*}"
    port="${SMOS_BIND##*:}"
    sleep 1
    if curl -fsS --max-time 3 "http://${host}:${port}/api/health" >/dev/null 2>&1; then
      log "smoke: health ok http://${host}:${port}/api/health"
    else
      log "smoke: health not ready yet — try: curl http://${host}:${port}/api/health"
    fi
  fi
}

print_done() {
  cat <<EOF

SMOS installed from GitHub Release (prebuilt).

  Binary:   ${SMOS_PREFIX}/smos
  WebUI:    ${SMOS_PREFIX}/static
  Data:     ${SMOS_DATA_DIR}
  Bind:     ${SMOS_BIND}
  Version:  ${RELEASE_TAG:-$SMOS_VERSION}

  Dashboard: http://${SMOS_BIND}/
  Health:    http://${SMOS_BIND}/api/health
  Service:   systemctl status smos

Reinstall / upgrade (safe while service is running — stops, replaces binary, restarts):
  curl -fsSL https://raw.githubusercontent.com/vantanminh/SMOS/main/scripts/install.sh | bash

Pin a version:
  curl -fsSL https://raw.githubusercontent.com/vantanminh/SMOS/main/scripts/install.sh | SMOS_VERSION=v0.1.0 bash

EOF
}

# Optional slow path if no release exists and operator opts in.
install_from_source() {
  log "SMOS_FROM_SOURCE=1 — building on this machine (requires rustc/cargo)"
  need_cmd git
  if ! command -v cargo >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    # shellcheck source=/dev/null
    [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
    export PATH="${HOME}/.cargo/bin:${PATH}"
  fi
  need_cmd cargo
  WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/smos-src.XXXXXX")"
  git clone --depth 1 "https://github.com/${SMOS_REPO}.git" "$WORK_DIR"
  (cd "$WORK_DIR" && cargo build --release)
  EXTRACTED_BIN="${WORK_DIR}/target/release/smos"
  EXTRACTED_STATIC="${WORK_DIR}/static"
  [ -x "$EXTRACTED_BIN" ] || die "source build failed"
}

cleanup() {
  if [ -n "${WORK_DIR:-}" ] && [ -d "${WORK_DIR:-}" ]; then
    rm -rf "$WORK_DIR" || true
  fi
}

main() {
  WORK_DIR=""
  ASSET_URL=""
  ASSET_NAME=""
  RELEASE_TAG=""
  EXTRACTED_BIN=""
  EXTRACTED_STATIC=""
  trap cleanup EXIT

  log "SMOS installer (release download) starting"
  detect_os
  detect_target
  install_runtime_deps

  if [ "$SMOS_FROM_SOURCE" = "1" ]; then
    install_from_source
  else
    resolve_asset_url
    download_and_extract
  fi

  install_files
  install_systemd
  smoke_check
  print_done
}

main "$@"
