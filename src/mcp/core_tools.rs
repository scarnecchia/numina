//! Core MCP tools for agent interactions

use crate::agent::{constellation::MultiAgentSystem, UserId};
use rmcp::{
    handler::server::tool::Parameters,
    model::{CallToolResult, Content},
    Error as McpError,
};
use serde_json::json;
use std::{future::Future, sync::Arc};
use tracing::{debug, error, info};

use super::{
    ChatWithAgentRequest, GetAgentMemoryRequest, ScheduleEventRequest, SendGroupMessageRequest,
    UpdateAgentMemoryRequest, UpdateAgentModelRequest,
};

/// Core MCP tools handler
#[derive(Clone)]
pub struct CoreTools {
    pub multi_agent_system: Arc<MultiAgentSystem>,
}

// Core MCP tool implementations
impl CoreTools {
    pub async fn chat_with_agent(
        &self,
        params: Parameters<ChatWithAgentRequest>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let params = params.0;
        info!(user_id = params.user_id, agent_id = ?params.agent_id, "Sending message to agent");
        debug!(message = %params.message, "Message content");

        match self
            .chat_with_agent_internal(
                UserId(params.user_id),
                &params.message,
                params.agent_id.as_deref(),
            )
            .await
        {
            Ok(response) => {
                info!("Agent responded successfully");
                Ok(CallToolResult::success(vec![Content::text(response)]))
            }
            Err(e) => {
                error!("Agent error: {}", e);
                Err(McpError::internal_error(
                    "Error communicating with agent",
                    Some(json!({
                        "agent_id": params.agent_id,
                        "user_id": params.user_id,
                        "error_type": "agent_communication_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    pub async fn get_agent_memory(
        &self,
        params: Parameters<GetAgentMemoryRequest>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let params = params.0;
        info!(user_id = params.user_id, agent_id = ?params.agent_id, "Getting agent memory");

        match self
            .get_agent_memory_internal(UserId(params.user_id), params.agent_id.as_deref())
            .await
        {
            Ok(memory) => {
                info!("Retrieved agent memory successfully");
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&memory).unwrap_or_else(|_| "Error".to_string()),
                )]))
            }
            Err(e) => {
                error!("Error getting agent memory: {}", e);
                Err(McpError::internal_error(
                    "Error retrieving agent memory",
                    Some(json!({
                        "agent_id": params.agent_id,
                        "user_id": params.user_id,
                        "error_type": "memory_retrieval_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    pub async fn update_agent_memory(
        &self,
        params: Parameters<UpdateAgentMemoryRequest>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let params = params.0;
        info!(user_id = params.user_id, agent_id = ?params.agent_id, "Updating agent memory");

        let updates: serde_json::Value = match serde_json::from_str(&params.memory_json) {
            Ok(json) => json,
            Err(e) => {
                error!("Invalid JSON in memory update: {}", e);
                return Err(McpError::invalid_params(
                    "Invalid JSON in memory_json parameter",
                    Some(json!({
                        "parameter": "memory_json",
                        "error_type": "json_parse_error",
                        "details": e.to_string(),
                        "line": e.line(),
                        "column": e.column()
                    })),
                ));
            }
        };

        match self
            .update_agent_memory_internal(
                UserId(params.user_id),
                params.agent_id.as_deref(),
                &updates,
            )
            .await
        {
            Ok(()) => {
                info!("Agent memory updated successfully");
                Ok(CallToolResult::success(vec![Content::text(
                    "Memory updated successfully",
                )]))
            }
            Err(e) => {
                error!("Error updating agent memory: {}", e);
                Err(McpError::internal_error(
                    "Error updating agent memory",
                    Some(json!({
                        "agent_id": params.agent_id,
                        "user_id": params.user_id,
                        "error_type": "memory_update_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    pub async fn update_agent_model(
        &self,
        params: Parameters<UpdateAgentModelRequest>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let params = params.0;
        info!(
            user_id = params.user_id,
            agent_id = %params.agent_id,
            capability = ?params.capability,
            "Updating agent model capability"
        );

        // Parse agent ID
        let agent_id = match params.agent_id.parse::<crate::agent::AgentId>() {
            Ok(id) => id,
            Err(e) => {
                error!("Invalid agent ID: {}", e);
                return Err(McpError::invalid_params(
                    "Invalid agent_id parameter",
                    Some(json!({
                        "parameter": "agent_id",
                        "error_type": "invalid_agent_id",
                        "details": e.to_string(),
                        "allowed_values": ["pattern", "entropy", "flux", "archive", "momentum", "anchor"]
                    })),
                ));
            }
        };

        match self
            .multi_agent_system
            .update_agent_model_capability(UserId(params.user_id), &agent_id, params.capability)
            .await
        {
            Ok(()) => {
                info!("Agent model capability updated successfully");
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Successfully updated {} to {} capability level",
                    params.agent_id, params.capability
                ))]))
            }
            Err(e) => {
                error!("Error updating agent model capability: {}", e);
                Err(McpError::internal_error(
                    "Error updating agent model capability",
                    Some(json!({
                        "agent_id": params.agent_id,
                        "user_id": params.user_id,
                        "capability": format!("{:?}", params.capability),
                        "error_type": "model_update_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    pub async fn schedule_event(
        &self,
        params: Parameters<ScheduleEventRequest>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let params = params.0;
        info!(
            user_id = params.user_id,
            title = params.title,
            "Scheduling event"
        );

        // TODO: Implement event scheduling
        Ok(CallToolResult::success(vec![Content::text(
            "Event scheduling not yet implemented",
        )]))
    }

    pub async fn check_activity_state(&self) -> std::result::Result<CallToolResult, McpError> {
        info!("Checking activity state");

        // TODO: Implement activity monitoring
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&json!({
                "interruptible": true,
                "current_focus": null,
                "last_activity": "idle"
            }))
            .unwrap_or_else(|_| "Error".to_string()),
        )]))
    }

    #[rmcp::tool(description = "Send a message to a group of agents")]
    pub async fn send_group_message(
        &self,
        params: Parameters<SendGroupMessageRequest>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let params = params.0;
        info!(
            user_id = params.user_id,
            group_name = params.group_name,
            "Sending message to group"
        );
        debug!(message = %params.message, "Message content");

        match self
            .multi_agent_system
            .send_message_to_group(UserId(params.user_id), &params.group_name, &params.message)
            .await
        {
            Ok(response) => {
                info!("Group responded successfully");
                // Extract assistant messages from the response
                let messages = response
                    .messages
                    .iter()
                    .filter_map(|msg| match msg {
                        letta::types::LettaMessageUnion::AssistantMessage(m) => {
                            Some(m.content.clone())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();

                let response_text = if messages.is_empty() {
                    "No response from group".to_string()
                } else {
                    messages.join("\n\n")
                };

                Ok(CallToolResult::success(vec![Content::text(response_text)]))
            }
            Err(e) => {
                error!("Group error: {}", e);
                Err(McpError::internal_error(
                    "Error communicating with group",
                    Some(json!({
                        "group_name": params.group_name,
                        "user_id": params.user_id,
                        "error_type": "group_communication_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    // Internal helper methods
    pub(crate) async fn chat_with_agent_internal(
        &self,
        user_id: UserId,
        message: &str,
        agent_id: Option<&str>,
    ) -> miette::Result<String> {
        // Initialize user if needed
        self.multi_agent_system.initialize_user(user_id).await?;

        // Update active context
        let context = format!("task: chat | message: {} | agent: {:?}", message, agent_id);
        self.multi_agent_system
            .update_shared_memory(user_id, "active_context", &context)
            .await?;

        // Route to specific agent and get response
        let response = self
            .multi_agent_system
            .send_message_to_agent(user_id, agent_id, message)
            .await?;

        Ok(response)
    }

    pub(crate) async fn get_agent_memory_internal(
        &self,
        user_id: UserId,
        agent_id: Option<&str>,
    ) -> miette::Result<serde_json::Value> {
        // Initialize user if needed
        self.multi_agent_system.initialize_user(user_id).await?;

        // Get memory state
        let block_name = agent_id.unwrap_or("current_state");
        let memory_content = self
            .multi_agent_system
            .get_shared_memory(user_id, block_name)
            .await?;

        Ok(serde_json::json!({
            "block_name": block_name,
            "content": memory_content
        }))
    }

    pub(crate) async fn update_agent_memory_internal(
        &self,
        user_id: UserId,
        _agent_id: Option<&str>,
        updates: &serde_json::Value,
    ) -> miette::Result<()> {
        // Initialize user if needed
        self.multi_agent_system.initialize_user(user_id).await?;

        // Extract memory block updates
        if let Some(updates_obj) = updates.as_object() {
            for (block_name, value) in updates_obj {
                if let Some(value_str) = value.as_str() {
                    self.multi_agent_system
                        .update_shared_memory(user_id, block_name, value_str)
                        .await?;
                }
            }
        }

        Ok(())
    }
}
