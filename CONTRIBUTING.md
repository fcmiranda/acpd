# Contributing to acpd

Thank you for considering a contribution to `acpd`! This guide covers how to get started and what we expect from pull requests.

## Development setup

You need Rust 1.95 or later.

```bash
# Clone the repository
git clone <repo-url>
cd acpd

# Build the project
cargo build

# Run the daemon locally
cargo run -- --config config/default.toml
```

See `AGENTS.md` for detailed agent-focused instructions and `README.md` for a quick-start overview.

## Commit best practices

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

## Before submitting a pull request

Run these checks locally:

```bash
cargo build
cargo test
cargo fmt
cargo clippy -- -D warnings
```

> Note: the current codebase has pre-existing `cargo fmt` and `cargo clippy` warnings. Do not introduce new warnings, and fix existing ones when you touch related files.

## Pull request process

1. **Small commits required:** every PR must consist of small, focused commits. See the [Commit best practices](#commit-best-practices) section above.
2. **One logical change per PR:** avoid combining unrelated features, refactors, and formatting fixes.
3. **All checks must pass:** `cargo build`, `cargo test`, `cargo fmt`, and `cargo clippy -- -D warnings`.
4. **At least one review approval** is expected before merging.
5. Keep the PR description concise and reference related issues when applicable.

## Code style

- Use `anyhow::Result` for fallible operations.
- Keep modules focused: `api.rs` for HTTP routes, `daemon.rs` for lifecycle, `adapters.rs` for output sinks, `signals.rs` for signal handling.
- Prefer structured logging via `tracing` over `println!`.
- Deserialize config with `serde` + `toml`.

## Adding tests

The project currently has no unit tests. New logic — especially state parsing, config loading, and adapter behavior — should include tests.

```bash
# Run the test suite
cargo test

# Run a specific test
cargo test <test_name>
```

## Getting help

- Open an issue for bugs or feature requests.
- See `AGENTS.md` for agent-focused project details.
- See `ARCHITECTURE.md` for the original design and migration plan.
