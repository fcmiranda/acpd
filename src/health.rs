use axum::{routing::get, Router, Json};
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use axum::extract::State;

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    uptime_secs: u64,
}

async fn health_check(
    State(start_time): State<Arc<Instant>>,
) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy",
        uptime_secs: start_time.elapsed().as_secs(),
    })
}

pub fn health_router(start_time: Arc<Instant>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(|| async { "ok" }))
        .with_state(start_time)
}
