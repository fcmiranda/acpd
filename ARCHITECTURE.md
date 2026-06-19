# Unified AI Terminal State Engine: Architecture Plan

Currently, your AI CLIs (Antigravity and OpenCode) independently manage complex UI logic: spawning detached processes, calculating frame intervals, querying `tmux` properties, and cleaning up window titles. This creates duplicate code and fragile edge cases. 

By extracting this into a centralized **State Daemon**, AI CLIs are reduced to "thin clients" that simply fire-and-forget standard events, while the daemon handles the heavy lifting of terminal UI rendering.

## 1. Architecture Overview

```mermaid
flowchart LR
    A[Antigravity Hook] -->|HTTP REST| D(acpd: Agent Client Protocol Daemon)
    B[OpenCode Plugin] -->|HTTP REST| D
    C[Claude Code / Goose] -->|HTTP JSON-RPC (ACP)| D
    W[SQLite/Log Watcher] -.->|Passive Polling| D
    
    D -->|Updates| T[Tmux Server]
    D -->|Writes| Y[Waybar State File]
    D -->|Triggers| N[Desktop Notifications]
```

* **The Daemon (`acpd`)**: A persistent background process that exposes a local HTTP server. It maintains an internal state machine for each `pane_id` using official ACP states (`working`, `idle`, `awaiting_input`, etc).
* **Generic Output Adapters**: Internally, the daemon uses a generic `OutputAdapter` trait. This decouples the state engine from Tmux. You can have multiple sinks (`TmuxAdapter`, `WaybarAdapter`, `NotificationAdapter`) reacting to state changes.
* **The Clients**: Lightweight scripts or direct CLI configurations that make a fast `curl` or `fetch` request to the daemon's local port and immediately exit.

## 2. The Contract (ACP & Local HTTP)

To ensure future-proofing with emerging AI agents while keeping current custom scripts simple to maintain, the daemon implements a **"Two-Door" HTTP Server** strategy instead of raw Unix Sockets:

### Door 1: The Standard (ACP via JSON-RPC)
Modern tools like Claude Code and Goose natively speak the Agent Client Protocol. You can point them to `POST http://127.0.0.1:4040/rpc` and they will automatically update your Tmux UI with official payloads:

```json
{
  "jsonrpc": "2.0",
  "method": "agentState/update",
  "params": { 
    "state": "working", // "idle", "working", "awaiting_input", "error"
    "pane_id": "%2" 
  },
  "id": 1
}
```

### Door 2: The Fast-Path (REST API)
For your current custom Bash/MJS hooks (Antigravity, OpenCode), dealing with JSON-RPC wrappers is overkill. They can hit a convenience REST endpoint `POST http://127.0.0.1:4040/api/status`:

```json
{
  "agent": "antigravity",
  "pane_id": "%2",
  "state": "working",
  "message": "Needs file system access"
}
```
The daemon internally translates this into the ACP state machine.

## 3. Language Recommendations

Since this is a persistent background daemon, resource usage and concurrency are the primary factors.

### 🌟 Top Recommendation: Rust
* **Why**: You are already building terminal tools in Rust (`matchmaker`), making this a perfect fit for your ecosystem. Rust is incredibly fast, memory-safe, and compiles to a single, lightweight binary with zero runtime dependencies.
* **Tech Stack**: `axum` for the local HTTP routing, `serde_json` for ACP/REST parsing, and standard `std::process::Command` for interfacing with `tmux`. 

### Alternative 1: Go (Golang)
* **Why**: Go is the undisputed king of writing simple, highly concurrent daemons and CLI tooling. It also compiles to a single static binary and makes Unix socket communication trivially easy.
* **Tech Stack**: `net.Listen("unix", "...")` and `goroutines` to manage isolated spinner intervals per pane.

### Alternative 2: TypeScript (Bun / Deno)
* **Why**: You already have the logic written in JS/TS. Using a modern runtime like Bun makes spinning up a fast local socket server very easy.
* **Drawback**: Running a JS engine persistently in the background consumes more baseline memory (~30-50MB) compared to a compiled Rust/Go binary (~2-5MB).

## 4. Execution Plan (Phased Migration)

1. **Phase 1: Build the Daemon (`acpd`)**
   * Create the `axum` HTTP server.
   * Migrate `getTomlColor`, `startSpinner`, and `setStaticState` from `tmux-hook.mjs` into the daemon.
   * Implement a state registry to track `setInterval` animations per `pane_id`.
2. **Phase 2: Refactor Antigravity**
   * Replace the complex `tmux-hook.mjs` with a tiny `fetch()` or `curl` call pointing to the `/api/status` endpoint.
3. **Phase 3: Refactor OpenCode**
   * Strip all UI and `tmux` logic from `hooker.ts`. Update the plugin events to simply forward payloads via HTTP.
4. **Phase 4: Future-Proofing (Watchers & ACP)**
   * Formally implement the `agentState/update` JSON-RPC method for Claude Code integration.
   * (Optional) Implement passive file watchers to eliminate hooks entirely.
