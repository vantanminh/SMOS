# SMOS — Server Management OS

Rust service for VPS host management: **metrics**, **processes**, **logs**, **configuration**, **audit journal**, and a single-binary **WebUI dashboard**.

## Features

| Area | Capability |
| --- | --- |
| Performance | Live CPU, memory, disk (and load average on Unix) via `sysinfo` |
| Processes | List running processes; terminate / kill (self-process blocked) |
| Logs | Tail SMOS service log + allowlisted extra paths |
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

## Install on Ubuntu server

Copy-paste install for **Ubuntu 22.04 / 24.04** (and similar Debian-based VPS images).

```bash
# 1) System packages
sudo apt-get update
sudo apt-get install -y build-essential curl git pkg-config

# 2) Rust toolchain (rustup)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustc --version

# 3) Clone and build SMOS
git clone https://github.com/<your-org>/SMOS.git smos
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
git clone <your-repo> smos && cd smos
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
| GET | `/api/processes` | Process list |
| POST | `/api/processes/{pid}/action` | Body: `{"action":"terminate"\|"kill"}` |
| GET | `/api/logs` | Log sources |
| GET | `/api/logs/{source_id}?lines=N` | Tail log source (`smos-service` built-in) |
| GET | `/api/config` | Public config (token redacted) |
| PUT | `/api/config` | Partial update (validated + audited) |
| GET | `/api/audit?limit=N` | Audit journal (newest first) |
| GET | `/` | WebUI dashboard |

### Example

```bash
curl -s http://127.0.0.1:9090/api/health | jq
curl -s http://127.0.0.1:9090/api/metrics | jq '.cpu,.memory.usage_percent'
curl -s -X PUT http://127.0.0.1:9090/api/config \
  -H 'Content-Type: application/json' \
  -d '{"host_label":"edge-1","log_tail_lines":300}'
curl -s http://127.0.0.1:9090/api/audit?limit=5 | jq
```

## Data directory layout

```
smos-data/           # or --data-dir
  config.json        # persisted configuration
  audit.jsonl        # append-only audit journal
  smos.log           # service log (also browsable in UI)
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
