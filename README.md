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

### Tmux Orchestration (RPC)

`acpd` can dynamically orchestrate Tmux windows and panes via standard JSON-RPC 2.0 calls, allowing AI agents to control the terminal seamlessly.

**1. Create a new Tmux window:**
```bash
curl -X POST http://127.0.0.1:4040/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"tmux.new_window","params":{"name":"✨ Agent","command":"echo Hello && sleep 10"},"id":1}'
```

**2. Split the current pane vertically:**
```bash
curl -X POST http://127.0.0.1:4040/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"tmux.split_pane","params":{"vertical":true,"command":"npm run dev"},"id":1}'
```

## Deploy with systemd (User Level)

Instead of running globally as root, `acpd` is designed to run as a user-level background service, allowing easy development and updates without `sudo`.

```bash
# 1. Setup default configuration
mkdir -p ~/.config/acpd
cp config/default.toml ~/.config/acpd/config.toml

# 2. Link the systemd service to your user configuration
mkdir -p ~/.config/systemd/user
ln -sf $(pwd)/systemd/acpd.service ~/.config/systemd/user/acpd.service

# 3. Reload and enable the daemon on boot
systemctl --user daemon-reload
systemctl --user enable --now acpd
```

View logs:
```bash
journalctl --user -u acpd -f
```

**Developer Workflow:** 
If you modify the source code, run `./scripts/update-daemon.sh`. It will automatically rebuild the release binary and restart the service in the background.

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
