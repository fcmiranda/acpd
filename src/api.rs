use crate::adapters::{AgentState, AgentUpdate, OutputAdapter};
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PaneStateInfo {
    pub last_timestamp: u64,
    pub seq_id: u64,
}

#[derive(Clone)]
pub struct ApiState {
    pub adapters: Arc<Vec<Box<dyn OutputAdapter>>>,
    pub pending_idles:
        Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::task::JoinHandle<()>>>>,
    pub idle_debounce_ms: u64,
    pub pane_states: Arc<tokio::sync::Mutex<std::collections::HashMap<String, PaneStateInfo>>>,
    pub next_seq: Arc<std::sync::atomic::AtomicU64>,
}

impl ApiState {
    pub fn new(adapters: Vec<Box<dyn OutputAdapter>>, idle_debounce_ms: u64) -> Self {
        Self {
            adapters: Arc::new(adapters),
            pending_idles: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            idle_debounce_ms,
            pane_states: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            next_seq: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }
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
    pub timestamp: Option<u64>,
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
    pub timestamp: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub success: bool,
}

#[derive(Debug, Clone)]
pub struct IncomingUpdate {
    pub pane_id: String,
    pub state: AgentState,
    pub message: Option<String>,
    pub timestamp: Option<u64>,
}

async fn dispatch_update(state: &ApiState, update: IncomingUpdate) {
    let seq = state
        .next_seq
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    {
        let mut states = state.pane_states.lock().await;
        if let Some(existing) = states.get(&update.pane_id) {
            if matches!(update.timestamp, Some(ts) if ts < existing.last_timestamp) {
                tracing::warn!(
                    "Discarding stale state update for pane {}: incoming_ts {:?} < last_ts {}",
                    update.pane_id,
                    update.timestamp,
                    existing.last_timestamp
                );
                return;
            }
        }
        states.insert(
            update.pane_id.clone(),
            PaneStateInfo {
                last_timestamp: update.timestamp.unwrap_or(0),
                seq_id: seq,
            },
        );
    }

    if update.state == AgentState::Idle {
        let mut pending = state.pending_idles.lock().await;
        if let Some(task) = pending.remove(&update.pane_id) {
            task.abort();
        }
        let adapters = state.adapters.clone();
        let update_clone = AgentUpdate {
            pane_id: update.pane_id.clone(),
            state: update.state.clone(),
            message: update.message.clone(),
        };
        let pane_id = update.pane_id.clone();
        let pending_map = state.pending_idles.clone();
        let pane_states = state.pane_states.clone();
        let delay_ms = state.idle_debounce_ms;

        let task = tokio::spawn(async move {
            if delay_ms > 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
            }
            {
                let states = pane_states.lock().await;
                if states.get(&pane_id).is_some_and(|c| c.seq_id != seq) {
                    return;
                }
            }
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
        let agent_update = AgentUpdate {
            pane_id: update.pane_id,
            state: update.state,
            message: update.message,
        };
        for adapter in state.adapters.iter() {
            if let Err(e) = adapter.update(&agent_update).await {
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
                let update = IncomingUpdate {
                    pane_id: params.pane_id,
                    state,
                    message: None,
                    timestamp: params.timestamp,
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
    let update = IncomingUpdate {
        pane_id: payload.pane_id.clone(),
        state,
        message: payload.message.clone(),
        timestamp: payload.timestamp,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::AgentState;
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;

    struct TestAdapter {
        updates: Arc<StdMutex<Vec<AgentUpdate>>>,
    }

    #[async_trait]
    impl OutputAdapter for TestAdapter {
        async fn update(&self, update: &AgentUpdate) -> anyhow::Result<()> {
            self.updates.lock().unwrap().push(update.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_stale_update_discarded() {
        let updates = Arc::new(StdMutex::new(Vec::new()));
        let adapter = TestAdapter {
            updates: updates.clone(),
        };
        let state = ApiState::new(vec![Box::new(adapter)], 0);

        // Newer update at t=2000
        dispatch_update(
            &state,
            IncomingUpdate {
                pane_id: "%1".into(),
                state: AgentState::Working,
                message: None,
                timestamp: Some(2000),
            },
        )
        .await;

        // Stale update at t=1000
        dispatch_update(
            &state,
            IncomingUpdate {
                pane_id: "%1".into(),
                state: AgentState::Idle,
                message: None,
                timestamp: Some(1000),
            },
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let recorded = updates.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].state, AgentState::Working);
    }

    #[tokio::test]
    async fn test_idle_superseded_by_working() {
        let updates = Arc::new(StdMutex::new(Vec::new()));
        let adapter = TestAdapter {
            updates: updates.clone(),
        };
        let state = ApiState::new(vec![Box::new(adapter)], 100);

        // Idle update at t=1000 with 100ms debounce
        dispatch_update(
            &state,
            IncomingUpdate {
                pane_id: "%1".into(),
                state: AgentState::Idle,
                message: None,
                timestamp: Some(1000),
            },
        )
        .await;

        // Working update at t=1050 arrives before 100ms debounce finishes
        dispatch_update(
            &state,
            IncomingUpdate {
                pane_id: "%1".into(),
                state: AgentState::Working,
                message: None,
                timestamp: Some(1050),
            },
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let recorded = updates.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].state, AgentState::Working);
    }
}
