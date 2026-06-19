use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Idle,
    Working,
    AwaitingInput,
    Permission,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentUpdate {
    pub pane_id: String,
    pub state: AgentState,
    pub message: Option<String>,
}

#[async_trait]
pub trait OutputAdapter: Send + Sync {
    async fn update(&self, update: &AgentUpdate) -> anyhow::Result<()>;
}

// ==========================================
// WAYBAR ADAPTER
// ==========================================
pub struct WaybarAdapter;

impl WaybarAdapter {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl OutputAdapter for WaybarAdapter {
    async fn update(&self, update: &AgentUpdate) -> anyhow::Result<()> {
        let raw_state = match update.state {
            AgentState::Idle => "idle",
            AgentState::Working => "busy",
            AgentState::AwaitingInput => "question",
            AgentState::Permission => "permission",
            AgentState::Error => "error",
        };

        // Write to /tmp/ai-agent-waybar-state
        let _ = tokio::fs::write("/tmp/ai-agent-waybar-state", raw_state).await;
        
        // Pkill waybar to trigger custom module update
        let _ = Command::new("pkill")
            .args(["-RTMIN+13", "waybar"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();

        tracing::info!("WaybarAdapter updated to: {}", raw_state);
        Ok(())
    }
}

// ==========================================
// TMUX ADAPTER
// ==========================================
pub struct TmuxAdapter {
    // Store active spinner tasks per pane so we can cancel them
    spinners: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
}

impl TmuxAdapter {
    pub fn new() -> Self {
        Self {
            spinners: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn stop_spinner(&self, pane_id: &str) {
        let mut spinners = self.spinners.lock().await;
        if let Some(task) = spinners.remove(pane_id) {
            task.abort();
        }
    }

    async fn start_spinner(&self, pane_id: String) {
        self.stop_spinner(&pane_id).await;

        let pane_clone = pane_id.clone();
        let task = tokio::spawn(async move {
            let frames = ["◜", "◠", "◝", "◞", "◡", "◟"];
            let mut i = 0;
            // Yellow fallback, in the future read from omarchy
            let color = "yellow"; 
            
            loop {
                let frame = frames[i % frames.len()];
                let formatted = format!("#[fg={}]{} #[fg=default]", color, frame);
                
                let _ = Command::new("tmux")
                    .args(["set-option", "-w", "-t", &pane_clone, "@ai_agent_state", &formatted])
                    .output().await;
                    
                let _ = Command::new("tmux")
                    .args(["set-option", "-w", "-t", &pane_clone, "@ai_agent_state_raw", "busy"])
                    .output().await;
                
                let _ = Command::new("tmux").args(["refresh-client", "-S"]).output().await;

                i += 1;
                tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
            }
        });

        self.spinners.lock().await.insert(pane_id, task);
    }

    async fn trigger_bell(&self, pane_id: &str, action: &str, force: bool) {
        // Fetch session and window properties for the pane
        let output = Command::new("tmux")
            .args(["display-message", "-t", pane_id, "-p", "#S|#{window_id}|#I|#W"])
            .output().await;
            
        if let Ok(out) = output {
            let props = String::from_utf8_lossy(&out.stdout);
            let parts: Vec<&str> = props.trim().split('|').collect();
            if parts.len() == 4 {
                let session = parts[0];
                let window_id = parts[1];
                let window_idx = parts[2];
                let window_name = parts[3];

                // Check if any other client is looking elsewhere
                let clients_out = Command::new("tmux")
                    .args(["list-clients", "-F", "#{client_session} #{window_id}"])
                    .output().await;
                
                let mut notify = true;
                if !force {
                    if let Ok(cout) = clients_out {
                        let clients_str = String::from_utf8_lossy(&cout.stdout);
                        let any_other_client = clients_str.lines().any(|line| {
                            let mut iter = line.split_whitespace();
                            if let (Some(c_sess), Some(c_wid)) = (iter.next(), iter.next()) {
                                !(c_sess == session && c_wid == window_id)
                            } else {
                                false
                            }
                        });
                        
                        if !any_other_client {
                            notify = false;
                        }
                    }
                }
                
                if notify {
                    let _ = Command::new("tmux").args(["set", "-g", "@ai_agent_last_bell", pane_id]).output().await;
                    let msg = format!("  #[fg=cyan]{}:{} › {} #[fg=yellow](i)#[fg=default]", window_idx, window_name, action);
                    let _ = Command::new("tmux").args(["set", "-g", "@ai_agent_bell", &msg]).output().await;
                    let _ = Command::new("tmux").args(["refresh-client", "-S"]).output().await;
                    
                    // Clear after 7 seconds
                    tokio::spawn(async move {
                        tokio::time::sleep(tokio::time::Duration::from_secs(7)).await;
                        let _ = Command::new("tmux").args(["set", "-g", "@ai_agent_bell", ""]).output().await;
                        let _ = Command::new("tmux").args(["refresh-client", "-S"]).output().await;
                    });
                }
            }
        }
    }
}

#[async_trait]
impl OutputAdapter for TmuxAdapter {
    async fn update(&self, update: &AgentUpdate) -> anyhow::Result<()> {
        match &update.state {
            AgentState::Working => {
                self.start_spinner(update.pane_id.clone()).await;
            }
            state => {
                self.stop_spinner(&update.pane_id).await;

                // Static states
                let (icon, color, raw) = match state {
                    AgentState::Idle => ("󱥂", "#94e2d5", "idle"),
                    AgentState::AwaitingInput => ("󱜻", "#cba6f7", "question"),
                    AgentState::Permission => ("󱅭", "#f38ba8", "permission"),
                    AgentState::Error => ("!", "red", "error"),
                    _ => unreachable!(),
                };

                let formatted = format!("#[fg={}]{} #[fg=default]", color, icon);

                let _ = Command::new("tmux")
                    .args(["set-option", "-w", "-t", &update.pane_id, "@ai_agent_state", &formatted])
                    .output().await;
                    
                let _ = Command::new("tmux")
                    .args(["set-option", "-w", "-t", &update.pane_id, "@ai_agent_state_raw", raw])
                    .output().await;

                let _ = Command::new("tmux").args(["refresh-client", "-S"]).output().await;

                // Trigger a bell based on state
                let (action_msg, force_bell) = match state {
                    AgentState::Idle => (Some("󱥂 finished"), false),
                    AgentState::AwaitingInput => (Some("󱜻 question"), true),
                    AgentState::Permission => (Some("󱅭 permission"), true),
                    _ => (None, false),
                };

                if let Some(msg) = action_msg {
                    self.trigger_bell(&update.pane_id, msg, force_bell).await;
                }
            }
        }
        
        tracing::info!("TmuxAdapter: Updated pane {} to {:?}", update.pane_id, update.state);
        Ok(())
    }
}
