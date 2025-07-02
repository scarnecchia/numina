use crate::{agent::UserId, agents::MultiAgentSystem, db::Database};
use rmcp::{
    handler::server::tool::Parameters,
    model::{CallToolResult, Content, Implementation, InitializeResult as ServerInfo},
    Error as McpError, ServerHandler,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{future::Future, sync::Arc};
use tracing::{debug, error, info};

// Request structs for tools
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct ChatWithAgentRequest {
    user_id: i64,
    message: String,
    agent_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct GetAgentMemoryRequest {
    user_id: i64,
    agent_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct UpdateAgentMemoryRequest {
    user_id: i64,
    memory_json: String,
    agent_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct ScheduleEventRequest {
    user_id: i64,
    title: String,
    start_time: String,
    description: Option<String>,
    duration_minutes: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct SendMessageRequest {
    channel_id: u64,
    message: String,
}

/// MCP server implementation for Pattern
#[derive(Debug, Clone)]
pub struct PatternMcpServer {
    letta_client: Arc<letta::LettaClient>,
    db: Arc<Database>,
    multi_agent_system: Arc<MultiAgentSystem>,
}

impl PatternMcpServer {
    pub fn new(
        letta_client: Arc<letta::LettaClient>,
        db: Arc<Database>,
        multi_agent_system: Arc<MultiAgentSystem>,
    ) -> Self {
        Self {
            letta_client,
            db,
            multi_agent_system,
        }
    }

    /// Run the MCP server on stdio transport
    pub async fn run_stdio(self) -> miette::Result<()> {
        info!("Starting MCP server on stdio transport");

        use tokio::io::{stdin, stdout};

        let transport = (stdin(), stdout());
        let server = rmcp::ServiceExt::serve(self, transport)
            .await
            .map_err(|e| miette::miette!("Failed to start server: {}", e))?;

        let quit_reason = server
            .waiting()
            .await
            .map_err(|e| miette::miette!("Server error: {}", e))?;

        info!("Server stopped: {:?}", quit_reason);
        Ok(())
    }

    /// Run the MCP server on SSE transport
    #[cfg(feature = "mcp-sse")]
    pub async fn run_sse(self, port: u16) -> miette::Result<()> {
        info!("Starting MCP server on SSE transport at port {}", port);

        // TODO: Implement SSE transport when available in rmcp
        Err(miette::miette!("SSE transport not yet implemented"))
    }
}

impl ServerHandler for PatternMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "pattern".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(
                "Pattern MCP server for multi-agent ADHD support system with Discord integration"
                    .to_string(),
            ),
            ..Default::default()
        }
    }
}

// Tool implementations
#[rmcp::tool_router]
impl PatternMcpServer {
    #[rmcp::tool(description = "Send a message to a Letta agent and get a response")]
    async fn chat_with_agent(
        &self,
        params: Parameters<ChatWithAgentRequest>,
    ) -> Result<CallToolResult, McpError> {
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

    #[rmcp::tool(description = "Get the current memory state of an agent")]
    async fn get_agent_memory(
        &self,
        params: Parameters<GetAgentMemoryRequest>,
    ) -> Result<CallToolResult, McpError> {
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

    #[rmcp::tool(description = "Update agent memory blocks")]
    async fn update_agent_memory(
        &self,
        params: Parameters<UpdateAgentMemoryRequest>,
    ) -> Result<CallToolResult, McpError> {
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

    #[rmcp::tool(description = "Schedule an event with smart time estimation")]
    async fn schedule_event(
        &self,
        params: Parameters<ScheduleEventRequest>,
    ) -> Result<CallToolResult, McpError> {
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

    #[rmcp::tool(description = "Send message to user via Discord")]
    async fn send_message(
        &self,
        params: Parameters<SendMessageRequest>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        info!(channel_id = params.channel_id, "Sending Discord message");

        // TODO: Implement Discord message sending
        // This will need access to the Discord bot instance
        Ok(CallToolResult::success(vec![Content::text(
            "Discord messaging not yet implemented",
        )]))
    }

    #[rmcp::tool(description = "Check activity state for interruption timing")]
    async fn check_activity_state(&self) -> Result<CallToolResult, McpError> {
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
}

// Internal implementation methods
impl PatternMcpServer {
    async fn chat_with_agent_internal(
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

    async fn get_agent_memory_internal(
        &self,
        user_id: UserId,
        _agent_id: Option<&str>,
    ) -> miette::Result<serde_json::Value> {
        // Get shared memory
        let memory = self
            .multi_agent_system
            .get_all_shared_memory(user_id)
            .await?;

        Ok(serde_json::json!({
            "shared_memory": memory,
            // TODO: Add agent-specific memory when we implement per-agent memory
        }))
    }

    async fn update_agent_memory_internal(
        &self,
        user_id: UserId,
        _agent_id: Option<&str>,
        updates: &serde_json::Value,
    ) -> miette::Result<()> {
        // Update shared memory blocks
        if let Some(obj) = updates.as_object() {
            for (key, value) in obj {
                if let Some(value_str) = value.as_str() {
                    self.multi_agent_system
                        .update_shared_memory(user_id, key, value_str)
                        .await?;
                }
            }
        }

        Ok(())
    }
}
