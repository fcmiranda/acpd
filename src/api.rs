use crate::adapters::{AgentState, AgentUpdate, OutputAdapter};
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub type ApiState = Arc<Vec<Box<dyn OutputAdapter>>>;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: RpcParams,
    pub id: u64,
}

#[derive(Debug, Deserialize)]
pub struct RpcParams {
    pub state: String,
    pub pane_id: String,
}

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    pub result: String,
    pub id: u64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct StatusRequest {
    pub agent: Option<String>,
    pub pane_id: String,
    pub state: String,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub success: bool,
}

async fn handle_rpc(
    State(adapters): State<ApiState>,
    Json(payload): Json<RpcRequest>,
) -> Json<RpcResponse> {
    tracing::info!("Received RPC: {:?}", payload);
    if payload.method == "agentState/update" {
        let state = AgentState::from(payload.params.state.as_str());
        let update = AgentUpdate {
            pane_id: payload.params.pane_id.clone(),
            state,
            message: None,
        };
        for adapter in adapters.iter() {
            if let Err(e) = adapter.update(&update).await {
                tracing::error!("Adapter error: {}", e);
            }
        }
    }
    Json(RpcResponse {
        jsonrpc: "2.0".into(),
        result: "ok".into(),
        id: payload.id,
    })
}

async fn handle_status(
    State(adapters): State<ApiState>,
    Json(payload): Json<StatusRequest>,
) -> Json<StatusResponse> {
    tracing::info!("Received REST status: {:?}", payload);
    let state = AgentState::from(payload.state.as_str());
    let update = AgentUpdate {
        pane_id: payload.pane_id.clone(),
        state,
        message: payload.message.clone(),
    };
    for adapter in adapters.iter() {
        if let Err(e) = adapter.update(&update).await {
            tracing::error!("Adapter error: {}", e);
        }
    }
    Json(StatusResponse { success: true })
}

pub fn api_router(adapters: ApiState) -> Router {
    Router::new()
        .route("/rpc", post(handle_rpc))
        .route("/api/status", post(handle_status))
        .with_state(adapters)
}
