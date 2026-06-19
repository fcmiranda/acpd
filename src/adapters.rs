use async_trait::async_trait;
use crate::state::AgentState;

#[async_trait]
pub trait OutputAdapter: Send + Sync {
    async fn update_state(
        &self,
        pane_id: &str,
        state: &AgentState,
        message: Option<&str>,
    ) -> anyhow::Result<()>;
}

pub struct TmuxAdapter;

#[async_trait]
impl OutputAdapter for TmuxAdapter {
    async fn update_state(
        &self,
        pane_id: &str,
        state: &AgentState,
        message: Option<&str>,
    ) -> anyhow::Result<()> {
        tracing::debug!("TmuxAdapter: pane={} state={:?} message={:?}", pane_id, state, message);
        // Here we will eventually run `tmux` commands like `tmux set-option -p -t {pane_id} @agent_state {state}`
        Ok(())
    }
}
