# SMOS — Server Management OS

Rust service for VPS host management: **metrics**, **processes**, **logs**, **configuration**, **audit journal**, and a single-binary **WebUI dashboard**.

## Features

| Area | Capability |
| --- | --- |
| Performance | Live CPU, memory, disk (and load average on Unix) via `sysinfo` |
| History | Persist metrics samples + daily service logs; **default 30-day retention** (configurable) |
| Processes | List running processes; terminate / kill (self-process blocked) |
| Logs | Tail SMOS service log, daily history files, + allowlisted extra paths |
| Config | Read/update validated config; persisted to `config.json` |
| Audit | Append-only JSONL journal for mutating actions |
| WebUI | Hash-routed SPA (`#/overview`, `#/metrics`, `#/processes`, `#/logs`, `#/config`, `#/audit`) |

## Quick start

```bash
# Build
cargo build --release

# Run (defaults to 127.0.0.1:9090)
./target/release/smos

# Or with options
./target/release/smos \
  --bind 0.0.0.0:9090 \
  --data-dir /var/lib/smos \
  --host-label my-vps \
  --auth-token 'change-me'
```

Open the dashboard: **http://127.0.0.1:9090/** (or your bind address).

### Environment variables

| Variable | Meaning |
| --- | --- |
| `SMOS_BIND` | Bind address `host:port` |
| `SMOS_DATA_DIR` | Data directory (config, audit, service log) |
| `SMOS_AUTH_TOKEN` | Optional shared secret for **mutating** API calls |
| `SMOS_HOST_LABEL` | Dashboard host label |
| `RUST_LOG` | Tracing filter (default `info`) |

### Auth posture

- **GET** routes are open (health, metrics, process list, logs, config view, audit).
- When `--auth-token` / `SMOS_AUTH_TOKEN` is set, **PUT/POST** require either:
  - `Authorization: Bearer <token>`, or
  - `X-SMOS-Token: <token>`
- Default bind is **localhost**. For a public VPS, use a firewall, reverse proxy TLS, and set an auth token. Process kill without auth on `0.0.0.0` is unsafe.

## Install (one command)

On a Linux VPS, install the **prebuilt** binary from **GitHub Releases** (no compile on the server):

```bash
curl -fsSL https://raw.githubusercontent.com/vantanminh/SMOS/main/scripts/install.sh | bash
```

What it does:

1. Installs only runtime tools (`curl`, `tar`, CA certs)
2. Detects CPU arch (`x86_64` / `aarch64`)
3. Downloads the matching tarball from [GitHub Releases](https://github.com/vantanminh/SMOS/releases)
4. Installs binary + WebUI to `/opt/smos`, data to `/var/lib/smos`
5. Enables and starts systemd unit `smos` on `127.0.0.1:9090`

```bash
curl -sS http://127.0.0.1:9090/api/health
systemctl status smos
# Dashboard: http://127.0.0.1:9090/
```

| Variable | Default | Meaning |
| --- | --- | --- |
| `SMOS_VERSION` | `latest` | Release tag, e.g. `v0.1.0` |
| `SMOS_BIND` | `127.0.0.1:9090` | HTTP bind |
| `SMOS_PREFIX` | `/opt/smos` | Install prefix |
| `SMOS_DATA_DIR` | `/var/lib/smos` | Config / audit / logs |
| `SMOS_SKIP_SERVICE` | `0` | `1` = skip systemd |
| `SMOS_FROM_SOURCE` | `0` | `1` = clone + cargo build (slow fallback) |

Pin a version:

```bash
curl -fsSL https://raw.githubusercontent.com/vantanminh/SMOS/main/scripts/install.sh | SMOS_VERSION=v0.1.0 bash
```

### Releases are automatic (CI)

You do **not** need to create tags by hand for installs to work.

| Trigger | What happens |
| --- | --- |
| **Push to `main`** | Workflow **Release** builds Linux tarball and publishes a GitHub Release (`v<cargo-version>-build.<run>`, marked **latest**) |
| **Manual tag** `v1.2.3` | Same workflow publishes that version (skipped for auto tags `*-build.*` to avoid loops) |
| **Actions → Release → Run workflow** | Optional; empty tag = auto version |

```bash
git push origin main
# wait until https://github.com/vantanminh/SMOS/actions  → Release is green
# then on the server:
curl -fsSL https://raw.githubusercontent.com/vantanminh/SMOS/main/scripts/install.sh | bash
```

If install prints **404 / no GitHub Release**, the Release workflow has not finished (or failed) yet — open the Actions tab, wait for green, retry. Asset pattern: `smos-v…-x86_64-unknown-linux-gnu.tar.gz`.

> **Trust note:** `curl | bash` runs remote code. Review [`scripts/install.sh`](scripts/install.sh) or pin `SMOS_VERSION`.

## Install on Ubuntu server (manual steps)

Copy-paste install for **Ubuntu 22.04 / 24.04** (and similar Debian-based VPS images). Prefer the [one-command installer](#install-one-command) above when possible.

```bash
# 1) System packages
sudo apt-get update
sudo apt-get install -y build-essential curl git pkg-config

# 2) Rust toolchain (rustup)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustc --version

# 3) Clone and build SMOS
git clone https://github.com/vantanminh/SMOS.git smos
cd smos
cargo build --release

# 4) Install binary + WebUI assets
sudo useradd -r -s /usr/sbin/nologin smos 2>/dev/null || true
sudo mkdir -p /opt/smos /var/lib/smos
sudo cp target/release/smos /opt/smos/
sudo cp -r static /opt/smos/
sudo chown -R smos:smos /var/lib/smos
sudo chmod 755 /opt/smos/smos

# 5) Auth token (required for mutating APIs if you expose beyond localhost)
echo 'SMOS_AUTH_TOKEN=change-me-to-a-long-secret' | sudo tee /etc/smos.env
sudo chmod 600 /etc/smos.env

# 6) systemd unit
sudo tee /etc/systemd/system/smos.service >/dev/null <<'EOF'
[Unit]
Description=SMOS Server Management OS
After=network.target

[Service]
Type=simple
User=smos
WorkingDirectory=/opt/smos
EnvironmentFile=/etc/smos.env
Environment=RUST_LOG=info
ExecStart=/opt/smos/smos --bind 127.0.0.1:9090 --data-dir /var/lib/smos --host-label %H
Restart=on-failure

[Install]
WantedBy=multi-user.target
EOF

# 7) Enable and start
sudo systemctl daemon-reload
sudo systemctl enable --now smos
sudo systemctl status smos --no-pager

# 8) Smoke-check (on the server)
curl -sS http://127.0.0.1:9090/api/health
```

Open the dashboard via SSH tunnel or reverse proxy:

```bash
# From your laptop
ssh -L 9090:127.0.0.1:9090 user@your-ubuntu-server
# then browse http://127.0.0.1:9090/
```

Optional: put nginx/Caddy in front of `127.0.0.1:9090` for TLS. Do not bind `0.0.0.0` without a strong `SMOS_AUTH_TOKEN` and firewall.

### Upgrade on Ubuntu

```bash
cd smos
git pull
cargo build --release
sudo systemctl stop smos
sudo cp target/release/smos /opt/smos/
sudo cp -r static /opt/smos/
sudo systemctl start smos
```

## CI

GitHub Actions runs on every **push** and **pull_request**: installs stable Rust on `ubuntu-latest`, then `cargo build` and `cargo test` (see `.github/workflows/ci.yml`).

## VPS deploy (Linux, generic)

Same runtime layout as the Ubuntu section (`/opt/smos`, `/var/lib/smos`, systemd). Use the Ubuntu install commands above on Ubuntu servers; on other distros install a C toolchain + rustup, then:

```bash
git clone https://github.com/vantanminh/SMOS.git smos && cd smos
cargo build --release
sudo useradd -r -s /usr/sbin/nologin smos || true
sudo mkdir -p /var/lib/smos /opt/smos
sudo cp target/release/smos /opt/smos/
sudo cp -r static /opt/smos/
sudo chown -R smos:smos /var/lib/smos
```

## API routes

| Method | Path | Description |
| --- | --- | --- |
| GET | `/api/health` | Liveness + version + host label |
| GET | `/api/metrics` | Live host metrics snapshot |
| GET | `/api/metrics/history?hours=N&limit=M` | Stored metrics samples (default last 24h) |
| GET | `/api/history` | History storage status (sizes, counts, log files) |
| GET | `/api/processes` | Process list |
| POST | `/api/processes/{pid}/action` | Body: `{"action":"terminate"\|"kill"}` |
| GET | `/api/logs` | Log sources (live + daily history files) |
| GET | `/api/logs/{source_id}?lines=N` | Tail log source (`smos-service` or `history:smos.log.YYYY-MM-DD`) |
| GET | `/api/config` | Public config (token redacted) |
| PUT | `/api/config` | Partial update (validated + audited) |
| GET | `/api/audit?limit=N` | Audit journal (newest first) |
| GET | `/` | WebUI dashboard |

### History retention

| Config field | Default | Meaning |
| --- | --- | --- |
| `history_retention_days` | **30** | Keep metrics samples and rotated daily logs for this many days (1–3650) |
| `metrics_history_interval_secs` | **60** | How often the service appends a metrics sample to disk (10–3600) |

Change either field in the **Config** page or via `PUT /api/config`. Prune runs on startup and about once per hour.

### Example

```bash
curl -s http://127.0.0.1:9090/api/health | jq
curl -s http://127.0.0.1:9090/api/metrics | jq '.cpu,.memory.usage_percent'
curl -s 'http://127.0.0.1:9090/api/metrics/history?hours=24&limit=200' | jq '.count, .retention_days'
curl -s http://127.0.0.1:9090/api/history | jq
curl -s -X PUT http://127.0.0.1:9090/api/config \
  -H 'Content-Type: application/json' \
  -d '{"host_label":"edge-1","history_retention_days":90}'
curl -s http://127.0.0.1:9090/api/audit?limit=5 | jq
```

## Data directory layout

```
smos-data/                    # or --data-dir
  config.json                 # persisted configuration
  audit.jsonl                 # append-only audit journal
  smos.log.YYYY-MM-DD         # daily rotated service logs (browsable in UI)
  history/
    metrics.jsonl             # compact metrics samples (pruned by retention)
```

## Development

```bash
cargo build
cargo test
cargo run -- --bind 127.0.0.1:9090 --data-dir ./smos-data
```

Tests cover real host collectors (`sysinfo`), config persistence, audit append/list, log tail, process list/self-guard/kill child, and in-process HTTP integration (health, metrics, processes, config, audit, logs, dashboard HTML, live TCP bind).

## Architecture (short)

- **Binary:** `smos` (Axum + Tokio)
- **Domain modules:** `metrics`, `processes`, `logs`, `config`, `audit` — same functions used by HTTP handlers and unit tests
- **WebUI:** static SPA under `static/` (hash router; no separate Node runtime required in production)
- **Primary target:** Linux VPS; Windows supported best-effort for local development

## License

MIT
