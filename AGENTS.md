# AGENTS.md

## Project Overview

`acpd` (Agent Client Protocol Daemon) is a headless Rust daemon that bridges Agent Client Protocol (ACP) events to desktop UI surfaces: Tmux pane state, Waybar status files, and desktop notifications.

- **Language:** Rust (edition 2024)
- **Runtime:** Tokio async runtime
- **Web framework:** Axum
- **Configuration:** TOML file
- **Deployment:** systemd service with `Type=notify`

The daemon exposes two HTTP interfaces:

- `POST /rpc` — JSON-RPC ACP endpoint (`agentState/update`)
- `POST /api/status` — convenience REST endpoint for quick integrations
- `GET /health` and `GET /ready` — health/readiness probes

## Setup Commands

Install the Rust toolchain (tested with Rust 1.95):

```bash
rustup update
```

Clone and build:

```bash
git clone <repo-url>
cd acpd
cargo build
```

Install runtime dependencies for full functionality:

```bash
# Tmux integration requires tmux server running
# systemd/journald integration is optional but expected in production
```

## Development Workflow

Run the daemon locally with the reference config:

```bash
cargo run -- --config config/default.toml
```

Run in release mode:

```bash
cargo run --release -- --config config/default.toml
```

The binary usage is:

```bash
acpd --config <path-to-config.toml>
```

Default config path when no argument is provided: `/etc/acpd/config.toml`.

### Logging

Set log level via config (`log_level`) or environment:

```bash
RUST_LOG=debug cargo run -- --config config/default.toml
```

In production with systemd, logs go to the journal:

```bash
journalctl -u acpd -f
```

## Testing Instructions

Run the test suite:

```bash
cargo test
```

> Current baseline: the project has no unit tests yet. Add tests for new logic, especially state parsing, config loading, and adapter behavior.

Run a specific test:

```bash
cargo test <test_name>
```

## Code Style Guidelines

Format code:

```bash
cargo fmt
```

Check formatting:

```bash
cargo fmt -- --check
```

Run the linter:

```bash
cargo clippy -- -D warnings
```

> Note: as of the current codebase, `cargo clippy -- -D warnings` and `cargo fmt --check` report pre-existing issues. Do not introduce new warnings, and fix existing ones when touching related files.

### Conventions

- Use `anyhow::Result` for fallible operations.
- Keep modules focused: `api.rs` for HTTP routes, `daemon.rs` for lifecycle, `adapters.rs` for output sinks, `signals.rs` for signal handling.
- Prefer structured logging via `tracing` over `println!`.
- Deserialize config with `serde` + `toml`.

## Commit Best Practices

> **Always prefer small, focused commits.** Each commit should represent a single logical change — one feature, one fix, one refactor, or one documentation update. Avoid bundling unrelated changes into a single commit.
>
> Why: small commits make reviews faster, history easier to bisect, rollbacks safer, and blame more useful.

Guidelines for every commit:

- **Atomic commits:** a commit should be complete and self-contained. Tests and formatting for that change should pass after the commit.
- **Clear, imperative messages:** write `Add health endpoint`, not `Added health endpoint` or `Adding health endpoint`.
- **Subject line under 72 characters** when possible.
- **Body when helpful:** explain *why* the change was made if the reason is not obvious from the diff.
- **No unrelated changes:** do not mix formatting fixes, refactors, and feature work in the same commit.

Example good commits:

```text
Add JSON-RPC agentState/update handler

Implements the ACP standard method so Claude Code and Goose
can update pane state directly.
```

```text
Fix PID file cleanup on stale process detection
```

## Build and Deployment

Build release binary:

```bash
cargo build --release
```

The binary is produced at `target/release/acpd`.

Install system-wide:

```bash
sudo install -Dm755 target/release/acpd /usr/local/bin/acpd
sudo install -Dm644 config/default.toml /etc/acpd/config.toml
sudo install -Dm644 systemd/acpd.service /etc/systemd/system/acpd.service
sudo mkdir -p /run/acpd
```

Start and enable the service:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now acpd
```

Check service status:

```bash
sudo systemctl status acpd
```

## Pull Request Guidelines

- **Small commits required:** every PR must consist of small, focused commits. See [Commit Best Practices](#commit-best-practices) above for the full rule and examples.
- **Before submitting:**
  1. `cargo build`
  2. `cargo test`
  3. `cargo fmt`
  4. `cargo clippy -- -D warnings`
- **Review process:** at least one review approval is expected before merging.
- Keep PR descriptions concise and reference related issues when applicable.

## Debugging and Troubleshooting

Health check:

```bash
curl http://127.0.0.1:4040/health
curl http://127.0.0.1:4040/ready
```

Send a test status update:

```bash
curl -X POST http://127.0.0.1:4040/api/status \
  -H "Content-Type: application/json" \
  -d '{"agent":"test","pane_id":"%1","state":"working","message":"testing"}'
```

Send an ACP JSON-RPC update:

```bash
curl -X POST http://127.0.0.1:4040/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"agentState/update","params":{"state":"working","pane_id":"%1"},"id":1}'
```

Common issues:

- **Port already in use:** change `port` in the config or stop the existing `acpd` process.
- **Permission denied on PID file:** ensure `/run/acpd` is writable by the service user (systemd creates this via `RuntimeDirectory=acpd`).
- **Tmux adapter not visible yet:** the `TmuxAdapter` currently logs state changes but does not yet execute tmux commands.

## Additional Notes

- The project uses APM for skill management. Run `apm install` after pulling changes to `apm.yml`.
- `ARCHITECTURE.md` contains the original design document and migration plan.
- `CONTRIBUTING.md` contains the full contribution guidelines, commit conventions, and pull request process.
