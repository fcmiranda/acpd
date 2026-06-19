mod adapters;

use axum::{
    extract::State,
    routing::post,
    Json, Router,
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::adapters::{AgentUpdate, OutputAdapter, TmuxAdapter, WaybarAdapter};

/// The shared state injected into our HTTP handlers.
struct AppState {
    adapters: Vec<Box<dyn OutputAdapter>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Initialize logging (tracing)
    // By default, it will show INFO level logs for our app.
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "acpd=info,axum=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting ACP Daemon (acpd)...");

    // 2. Initialize the Output Adapters
    let tmux_adapter = Box::new(TmuxAdapter::new());
    let waybar_adapter = Box::new(WaybarAdapter::new());
    
    let shared_state = Arc::new(AppState {
        adapters: vec![tmux_adapter, waybar_adapter],
    });

    // 3. Build the Axum Router
    let app = Router::new()
        // The REST fast-path for legacy scripts
        .route("/api/status", post(handle_status))
        // (Future) The official JSON-RPC ACP endpoint could be mapped here too
        // .route("/rpc", post(handle_rpc))
        .with_state(shared_state);

    // 4. Start the HTTP Server on port 4040
    let listener = TcpListener::bind("127.0.0.1:4040").await?;
    tracing::info!("Listening for ACP events on {}", listener.local_addr()?);
    
    axum::serve(listener, app).await?;

    Ok(())
}

/// Handler for the `/api/status` endpoint
async fn handle_status(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AgentUpdate>,
) -> Json<&'static str> {
    tracing::info!("Received REST update: {:?}", payload);

    // Broadcast the update to all registered adapters (currently just Tmux)
    for adapter in &state.adapters {
        if let Err(e) = adapter.update(&payload).await {
            tracing::error!("Adapter update failed: {}", e);
        }
    }

    Json("ok")
}
