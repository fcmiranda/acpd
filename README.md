# acpd — Agent Client Protocol Daemon

A headless Rust daemon that bridges Agent Client Protocol (ACP) events to desktop UI surfaces: Tmux pane state, Waybar status files, and desktop notifications.

## What it does

AI coding agents (Claude Code, Goose, custom hooks) currently manage their own Tmux spinner logic, window title cleanup, and UI state. `acpd` centralizes that into a single long-running daemon so clients become thin HTTP callers:

```text
AI agent  ──HTTP──▶  acpd  ──▶  Tmux / Waybar / Notifications
```

## Features

- **ACP JSON-RPC endpoint** at `POST /rpc` (`agentState/update`)
- **Simple REST endpoint** at `POST /api/status` for custom bash/JS hooks
- **Health and readiness probes** at `GET /health` and `GET /ready`
- **systemd-notify integration** for `Type=notify` services
- **Graceful shutdown** on `SIGTERM`/`SIGINT`, config reload hook on `SIGHUP`
- **Modular output adapters** — currently includes a `TmuxAdapter` scaffold

## Quick start

Requires Rust 1.95+.

```bash
# Build
cargo build --release

# Run with the reference config
cargo run --release -- --config config/default.toml
```

The daemon listens on `127.0.0.1:4040` by default.

## Configuration

Create a TOML config file:

```toml
listen_addr = "127.0.0.1"
port = 4040
pid_file = "/run/acpd/acpd.pid"
shutdown_timeout_secs = 30
log_level = "info"
```

`acpd` reads the config path from the second CLI argument, defaulting to `/etc/acpd/config.toml`.

## API

### Health

```bash
curl http://127.0.0.1:4040/health
curl http://127.0.0.1:4040/ready
```

### REST status update

```bash
curl -X POST http://127.0.0.1:4040/api/status \
  -H "Content-Type: application/json" \
  -d '{"agent":"antigravity","pane_id":"%2","state":"working","message":"Needs file system access"}'
```

Valid states: `idle`, `working`, `awaiting_input`, `error`.

### ACP JSON-RPC update

```bash
curl -X POST http://127.0.0.1:4040/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"agentState/update","params":{"state":"working","pane_id":"%2"},"id":1}'
```

## Deploy with systemd

```bash
sudo install -Dm755 target/release/acpd /usr/local/bin/acpd
sudo install -Dm644 config/default.toml /etc/acpd/config.toml
sudo install -Dm644 systemd/acpd.service /etc/systemd/system/acpd.service
sudo systemctl daemon-reload
sudo systemctl enable --now acpd
```

View logs:

```bash
journalctl -u acpd -f
```

## Development

```bash
# Build
cargo build

# Run tests
cargo test

# Format and lint
cargo fmt
cargo clippy -- -D warnings
```

See `AGENTS.md` for detailed agent-focused instructions, `CONTRIBUTING.md` for contribution guidelines and commit conventions, and `ARCHITECTURE.md` for the original design and migration plan.
