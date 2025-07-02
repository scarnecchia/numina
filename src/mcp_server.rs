use crate::{agent::UserId, PatternService};
use rmcp::{model::ServerInfo, schemars, serde_json, tool, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

#[derive(Debug, Clone)]
pub struct PatternServer {
    #[allow(dead_code)]
    service: Arc<PatternService>,
}

impl PatternServer {
    pub fn new(service: PatternService) -> Self {
        Self {
            service: Arc::new(service),
        }
    }
}

impl ServerHandler for PatternServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: rmcp::model::Implementation {
                name: "pattern".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some("Pattern MCP server for Letta agent management with calendar scheduling and activity monitoring".to_string()),
            ..Default::default()
        }
    }
}

#[tool(tool_box)]
impl PatternServer {
    #[tool(description = "Send a message to a Letta agent and get a response")]
    async fn chat_with_agent(
        &self,
        #[tool(param)] user_id: i64,
        #[tool(param)] message: String,
    ) -> String {
        info!(user_id, "Sending message to agent");
        debug!(%message, "Message content");

        let agent_manager = match self.service.agent_manager() {
            Some(manager) => manager,
            None => {
                return "Error: Agent manager not initialized. Please configure Letta client."
                    .to_string()
            }
        };

        match agent_manager.send_message(UserId(user_id), &message).await {
            Ok(response) => {
                info!("Agent responded successfully");
                response
            }
            Err(e) => {
                info!("Agent error: {}", e);
                format!("Error communicating with agent: {}", e)
            }
        }
    }

    #[tool(description = "Get the current memory state of a user's agent")]
    async fn get_agent_memory(&self, #[tool(param)] user_id: i64) -> String {
        info!(user_id, "Getting agent memory");

        let agent_manager = match self.service.agent_manager() {
            Some(manager) => manager,
            None => {
                return "Error: Agent manager not initialized. Please configure Letta client."
                    .to_string()
            }
        };

        match agent_manager.get_agent_memory(UserId(user_id)).await {
            Ok(memory) => {
                info!("Retrieved agent memory successfully");
                serde_json::to_string_pretty(&memory)
                    .unwrap_or_else(|e| format!("Error serializing memory: {}", e))
            }
            Err(e) => {
                info!("Error getting agent memory: {}", e);
                format!("Error retrieving agent memory: {}", e)
            }
        }
    }

    #[tool(description = "Update the memory blocks of a user's agent")]
    async fn update_agent_memory(
        &self,
        #[tool(param)] user_id: i64,
        #[tool(param)] memory_json: String,
    ) -> String {
        info!(user_id, "Updating agent memory");

        let agent_manager = match self.service.agent_manager() {
            Some(manager) => manager,
            None => {
                return "Error: Agent manager not initialized. Please configure Letta client."
                    .to_string()
            }
        };

        // Parse the memory JSON
        let memory: letta::types::AgentMemory = match serde_json::from_str(&memory_json) {
            Ok(m) => m,
            Err(e) => return format!("Error parsing memory JSON: {}", e),
        };

        match agent_manager
            .update_agent_memory(UserId(user_id), memory)
            .await
        {
            Ok(()) => {
                info!("Agent memory updated successfully");
                "Memory updated successfully".to_string()
            }
            Err(e) => {
                info!("Error updating agent memory: {}", e);
                format!("Error updating agent memory: {}", e)
            }
        }
    }

    #[tool(description = "Schedule an event with smart time estimation")]
    async fn schedule_event(&self, #[tool(aggr)] req: ScheduleEventRequest) -> String {
        info!("Scheduling event: {}", req.title);
        debug!(?req, "Full event request");

        // TODO: Implement actual scheduling logic with database
        let response = format!(
            "Event '{}' scheduled for {} minutes",
            req.title, req.duration_minutes
        );

        info!("Event scheduled successfully");
        response
    }

    #[tool(description = "Send message to user via Discord")]
    async fn send_message(
        &self,
        #[tool(param)] channel_id: u64,
        #[tool(param)] message: String,
    ) -> String {
        info!(channel_id, "Sending message to Discord channel");
        debug!(%message, "Message content");

        // TODO: Implement Discord integration
        let response = format!("Message sent to channel {}: {}", channel_id, message);

        info!("Message sent successfully");
        response
    }

    #[tool(description = "Check activity state for interruption timing")]
    fn check_activity_state(&self) -> String {
        debug!("Checking activity state");

        // TODO: Implement platform-specific activity monitoring
        let state = ActivityState::default();
        let json = serde_json::to_string(&state).expect("ActivityState should serialize");

        debug!(?state, "Current activity state");
        json
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScheduleEventRequest {
    pub title: String,
    pub duration_minutes: u32,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ActivityState {
    pub interruptibility: InterruptibilityScore,
    pub current_app: Option<String>,
    pub idle_minutes: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum InterruptibilityScore {
    Low,
    Medium,
    High,
}

impl Default for ActivityState {
    fn default() -> Self {
        Self {
            interruptibility: InterruptibilityScore::Medium,
            current_app: None,
            idle_minutes: 0.0,
        }
    }
}
