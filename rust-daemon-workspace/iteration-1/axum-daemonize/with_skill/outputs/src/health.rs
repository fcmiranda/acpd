use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

/// Shared state injected into health check handlers.
#[derive(Clone)]
pub struct HealthState {
    /// Timestamp when the daemon started — used to compute uptime.
    pub start_time: Instant,
}

/// JSON response body for the /health endpoint.
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub service: &'static str,
    pub version: &'static str,
    pub uptime_secs: u64,
    pub pid: u32,
}

/// Handler for GET /health
async fn health_check(State(state): State<Arc<HealthState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy",
        service: "api-gateway",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: state.start_time.elapsed().as_secs(),
        pid: std::process::id(),
    })
}

/// Handler for GET /ready — lightweight readiness probe.
async fn readiness_probe() -> &'static str {
    "ok"
}

/// Build an axum Router containing the health and readiness endpoints.
pub fn health_router(start_time: Instant) -> Router {
    let state = Arc::new(HealthState { start_time });

    Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_probe))
        .with_state(state)
}
