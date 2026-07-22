use crate::adapters::{AgentState, AgentUpdate, OutputAdapter};
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct ApiState {
    pub adapters: Arc<Vec<Box<dyn OutputAdapter>>>,
    pub pending_idles:
        Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::task::JoinHandle<()>>>>,
    pub idle_debounce_ms: u64,
}

use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    pub id: u64,
}

#[derive(Debug, Deserialize)]
pub struct StateUpdateParams {
    pub state: String,
    pub pane_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SplitPaneParams {
    pub command: Option<String>,
    pub target_pane: Option<String>,
    pub vertical: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct NewWindowParams {
    pub name: Option<String>,
    pub target: Option<String>,
    pub command: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NewSessionParams {
    pub name: String,
    pub directory: Option<String>,
    pub command: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TargetParams {
    pub target: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum RpcResponse {
    Success {
        jsonrpc: String,
        result: Value,
        id: u64,
    },
    Error {
        jsonrpc: String,
        error: RpcError,
        id: u64,
    },
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
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

async fn dispatch_update(state: &ApiState, update: AgentUpdate) {
    if update.state == AgentState::Idle {
        let mut pending = state.pending_idles.lock().await;
        if let Some(task) = pending.remove(&update.pane_id) {
            task.abort();
        }
        let adapters = state.adapters.clone();
        let update_clone = update.clone();
        let pane_id = update.pane_id.clone();
        let pending_map = state.pending_idles.clone();
        let delay_ms = state.idle_debounce_ms;

        let task = tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
            for adapter in adapters.iter() {
                if let Err(e) = adapter.update(&update_clone).await {
                    tracing::error!("Adapter error: {}", e);
                }
            }
            let mut pending = pending_map.lock().await;
            pending.remove(&pane_id);
        });
        pending.insert(update.pane_id.clone(), task);
    } else {
        {
            let mut pending = state.pending_idles.lock().await;
            if let Some(task) = pending.remove(&update.pane_id) {
                task.abort();
            }
        }
        for adapter in state.adapters.iter() {
            if let Err(e) = adapter.update(&update).await {
                tracing::error!("Adapter error: {}", e);
            }
        }
    }
}

async fn handle_rpc(
    State(adapters): State<ApiState>,
    Json(payload): Json<RpcRequest>,
) -> Json<RpcResponse> {
    tracing::info!("Received RPC: method={} id={}", payload.method, payload.id);

    match payload.method.as_str() {
        "agentState/update" => match serde_json::from_value::<StateUpdateParams>(payload.params) {
            Ok(params) => {
                let state = AgentState::from(params.state.as_str());
                let update = AgentUpdate {
                    pane_id: params.pane_id,
                    state,
                    message: None,
                };
                dispatch_update(&adapters, update).await;
                Json(RpcResponse::Success {
                    jsonrpc: "2.0".into(),
                    result: serde_json::json!("ok"),
                    id: payload.id,
                })
            }
            Err(e) => Json(RpcResponse::Error {
                jsonrpc: "2.0".into(),
                error: RpcError {
                    code: -32602,
                    message: format!("Invalid params: {}", e),
                },
                id: payload.id,
            }),
        },
        "tmux.split_pane" => {
            match serde_json::from_value::<SplitPaneParams>(payload.params) {
                Ok(params) => {
                    let mut cmd = tokio::process::Command::new("tmux");
                    cmd.arg("split-window");

                    if params.vertical.unwrap_or(false) {
                        cmd.arg("-h"); // tmux -h is vertical split
                    } else {
                        cmd.arg("-v"); // tmux -v is horizontal split
                    }

                    if let Some(target) = params.target_pane {
                        cmd.arg("-t").arg(target);
                    }

                    if let Some(command) = params.command {
                        cmd.arg(&command);
                    }

                    execute_tmux_cmd(cmd, payload.id, "pane created").await
                }
                Err(e) => Json(RpcResponse::Error {
                    jsonrpc: "2.0".into(),
                    error: RpcError {
                        code: -32602,
                        message: format!("Invalid params: {}", e),
                    },
                    id: payload.id,
                }),
            }
        }
        "tmux.new_window" => match serde_json::from_value::<NewWindowParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("new-window");
                if let Some(name) = params.name {
                    cmd.arg("-n").arg(name);
                }
                if let Some(target) = params.target {
                    cmd.arg("-t").arg(target);
                }
                if let Some(command) = params.command {
                    cmd.arg(&command);
                }
                execute_tmux_cmd(cmd, payload.id, "window created").await
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.new_session" => match serde_json::from_value::<NewSessionParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("new-session").arg("-d").arg("-s").arg(params.name);
                if let Some(dir) = params.directory {
                    cmd.arg("-c").arg(dir);
                }
                if let Some(command) = params.command {
                    cmd.arg(&command);
                }
                execute_tmux_cmd(cmd, payload.id, "session created").await
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.kill_pane" => match serde_json::from_value::<TargetParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("kill-pane").arg("-t").arg(params.target);
                execute_tmux_cmd(cmd, payload.id, "pane killed").await
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.kill_window" => match serde_json::from_value::<TargetParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("kill-window").arg("-t").arg(params.target);
                execute_tmux_cmd(cmd, payload.id, "window killed").await
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.kill_session" => match serde_json::from_value::<TargetParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("kill-session").arg("-t").arg(params.target);
                execute_tmux_cmd(cmd, payload.id, "session killed").await
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        _ => Json(RpcResponse::Error {
            jsonrpc: "2.0".into(),
            error: RpcError {
                code: -32601,
                message: "Method not found".into(),
            },
            id: payload.id,
        }),
    }
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
    dispatch_update(&adapters, update).await;
    Json(StatusResponse { success: true })
}

pub fn api_router(adapters: ApiState) -> Router {
    Router::new()
        .route("/rpc", post(handle_rpc))
        .route("/api/status", post(handle_status))
        .with_state(adapters)
}

async fn execute_tmux_cmd(
    mut cmd: tokio::process::Command,
    id: u64,
    success_msg: &str,
) -> Json<RpcResponse> {
    match cmd.output().await {
        Ok(output) if output.status.success() => Json(RpcResponse::Success {
            jsonrpc: "2.0".into(),
            result: serde_json::json!(success_msg),
            id,
        }),
        Ok(output) => {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            Json(RpcResponse::Error {
                jsonrpc: "2.0".into(),
                error: RpcError {
                    code: -32000,
                    message: format!("Tmux error: {}", err_msg),
                },
                id,
            })
        }
        Err(e) => Json(RpcResponse::Error {
            jsonrpc: "2.0".into(),
            error: RpcError {
                code: -32000,
                message: format!("Failed to execute tmux: {}", e),
            },
            id,
        }),
    }
}

fn json_invalid_params(e: serde_json::Error, id: u64) -> Json<RpcResponse> {
    Json(RpcResponse::Error {
        jsonrpc: "2.0".into(),
        error: RpcError {
            code: -32602,
            message: format!("Invalid params: {}", e),
        },
        id,
    })
}
