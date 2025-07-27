//! Heartbeat handling for multi-step agent reasoning
//!
//! Based on Letta/MemGPT's heartbeat concept, this allows agents to request
//! additional turns without waiting for external input.

use serde_json::Value;
use tokio::sync::mpsc;

use crate::AgentId;

/// A heartbeat request from an agent
#[derive(Debug, Clone)]
pub struct HeartbeatRequest {
    pub agent_id: AgentId,
    pub tool_name: String,
    pub tool_call_id: String,
}

/// Channel for sending heartbeat requests
pub type HeartbeatSender = mpsc::Sender<HeartbeatRequest>;
pub type HeartbeatReceiver = mpsc::Receiver<HeartbeatRequest>;

/// Create a new heartbeat channel with reasonable buffer size
pub fn heartbeat_channel() -> (HeartbeatSender, HeartbeatReceiver) {
    mpsc::channel(100)
}

/// Check if a tool's arguments contain a heartbeat request
pub fn check_heartbeat_request(fn_arguments: &Value) -> bool {
    fn_arguments
        .get("request_heartbeat")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}
