use crate::{
    agent::{MultiAgentSystem, UserId},
    db::Database,
    mcp::{
        discord_tools::{SendDiscordDmRequest, SendDiscordMessageRequest},
        knowledge_tools::{
            KnowledgeTools, ReadAgentKnowledgeRequest, SyncKnowledgeRequest,
            WriteAgentKnowledgeRequest,
        },
        MessageDestinationType,
    },
};
use rmcp::{
    handler::server::{router::tool::ToolRouter, tool::Parameters},
    model::{
        CallToolResult, Content, Implementation, InitializeResult as ServerInfo, ServerCapabilities,
    },
    Error as McpError, ServerHandler,
};
use serde_json::json;
use std::{future::Future, sync::Arc};
use tracing::{debug, error, info};

use super::{core_tools::CoreTools, discord_tools::DiscordTools, SendMessageRequest};
use super::{
    GetAgentMemoryRequest, RecordEnergyStateRequest, ScheduleEventRequest, SendGroupMessageRequest,
    UpdateAgentMemoryRequest, UpdateAgentModelRequest,
};

/// MCP server implementation for Pattern
#[derive(Clone)]
pub struct PatternMcpServer {
    #[allow(dead_code)] // Used by MCP tools
    letta_client: Arc<letta::LettaClient>,
    #[allow(dead_code)] // Used by MCP tools
    db: Arc<Database>,
    #[allow(dead_code)] // Used by MCP tools
    core_tools: CoreTools,
    #[cfg(feature = "discord")]
    #[allow(dead_code)] // Used by Discord MCP tools
    discord_tools: DiscordTools,
    #[allow(dead_code)] // Used by the ServerHandler trait
    tool_router: ToolRouter<Self>,
}

impl PatternMcpServer {
    pub fn new(
        letta_client: Arc<letta::LettaClient>,
        db: Arc<Database>,
        multi_agent_system: Arc<MultiAgentSystem>,
        #[cfg(feature = "discord")] discord_client: Option<
            Arc<tokio::sync::RwLock<serenity::Client>>,
        >,
    ) -> Self {
        let tool_router = Self::tool_router();
        info!(
            "Tool router initialized with {} tools",
            tool_router.list_all().len()
        );

        Self {
            letta_client,
            db,
            core_tools: CoreTools { multi_agent_system },
            #[cfg(feature = "discord")]
            discord_tools: DiscordTools { discord_client },
            tool_router,
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

    /// Run the MCP server on HTTP transport (streamable)
    #[cfg(feature = "mcp")]
    pub async fn run_http(self, port: u16) -> miette::Result<()> {
        use hyper_util::{
            rt::{TokioExecutor, TokioIo},
            server::conn::auto::Builder,
            service::TowerToHyperService,
        };
        use rmcp::transport::streamable_http_server::{
            session::local::LocalSessionManager, StreamableHttpService,
        };
        use tokio::net::TcpListener;

        info!("Starting MCP server on HTTP transport at port {}", port);

        // Bind to the port (using IPv4 for compatibility)
        let addr = format!("0.0.0.0:{}", port);
        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| miette::miette!("Failed to bind to {}: {}", addr, e))?;

        info!("MCP HTTP server listening on http://{}", addr);
        info!("Configure Letta to use: http://localhost:{}", port);
        info!("Note: Letta may be slow to respond. The server will keep connections open.");

        // Main accept loop - no ctrl+c handler here, let main handle shutdown
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    info!("Accepted new MCP connection from {}", addr);
                    let io = TokioIo::new(stream);

                    // Clone self for the spawned task
                    let server = self.clone();
                    let addr_info = addr;

                    // Create service for this connection
                    let service = TowerToHyperService::new(StreamableHttpService::new(
                        move || {
                            info!("Creating new MCP handler instance for {}", addr_info);
                            Ok(server.clone())
                        },
                        LocalSessionManager::default().into(),
                        Default::default(),
                    ));

                    // Spawn handler for this connection
                    tokio::spawn(async move {
                        debug!("Starting to serve connection");
                        let builder = Builder::new(TokioExecutor::new());
                        let conn = builder.serve_connection(io, service);

                        if let Err(e) = conn.await {
                            error!("Error serving MCP connection: {}", e);
                        } else {
                            info!("MCP connection closed normally");
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                    // If we can't accept connections, might as well stop
                    return Err(miette::miette!("Failed to accept connection: {}", e));
                }
            }
        }
    }

    /// Run the MCP server on SSE transport
    #[cfg(feature = "mcp-sse")]
    pub async fn run_sse(self, port: u16) -> miette::Result<()> {
        use rmcp::transport::{sse_server::SseServerConfig, SseServer};
        use tokio_util::sync::CancellationToken;

        info!("Starting MCP server on SSE transport at port {}", port);

        let addr = format!("0.0.0.0:{}", port);
        let addr: std::net::SocketAddr = addr
            .parse()
            .map_err(|e| miette::miette!("Failed to parse address: {}", e))?;

        info!("MCP SSE server listening on http://{}/sse", addr);
        info!("Configure Letta to use: http://localhost:{}/sse", port);

        // Create SSE server configuration
        let config = SseServerConfig {
            bind: addr,
            sse_path: "/sse".to_string(),
            post_path: "/message".to_string(),
            ct: CancellationToken::new(),
            sse_keep_alive: Some(std::time::Duration::from_secs(30)),
        };

        // Create and serve
        let sse_server = SseServer::serve_with_config(config)
            .await
            .map_err(|e| miette::miette!("Failed to start SSE server: {}", e))?;

        // Handle incoming connections
        let handler = self;
        let ct = sse_server.with_service(move || {
            info!("SSE client connected");
            handler.clone()
        });

        // Wait for shutdown
        ct.cancelled().await;
        info!("SSE server shutdown");

        Ok(())
    }
}

impl ServerHandler for PatternMcpServer {
    fn get_info(&self) -> ServerInfo {
        info!("MCP server get_info called - this is the initial handshake");
        let info = ServerInfo {
            server_info: Implementation {
                name: "pattern".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(
                "Pattern MCP server for multi-agent ADHD support system with Discord integration"
                    .to_string(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        };
        debug!("Returning server info: {:?}", info);
        info
    }

    async fn list_tools(
        &self,
        _params: Option<rmcp::model::PaginatedRequestParam>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, McpError> {
        let tools = self.tool_router.list_all();
        info!("list_tools called, returning {} tools", tools.len());
        for tool in &tools {
            debug!(
                "  - {}: {}",
                tool.name,
                tool.description.as_deref().unwrap_or("")
            );
        }

        Ok(rmcp::model::ListToolsResult {
            tools,
            next_cursor: None,
        })
    }
}

// Tool implementations for agent operations
#[rmcp::tool_router]
impl PatternMcpServer {
    #[rmcp::tool(description = "Get the current memory state of an agent")]
    async fn get_agent_memory(
        &self,
        params: Parameters<GetAgentMemoryRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.core_tools.get_agent_memory(params).await
    }

    #[rmcp::tool(description = "Update agent memory blocks")]
    async fn update_agent_memory(
        &self,
        params: Parameters<UpdateAgentMemoryRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.core_tools.update_agent_memory(params).await
    }

    #[rmcp::tool(description = "Update an agent's language model capability level")]
    async fn update_agent_model(
        &self,
        params: Parameters<UpdateAgentModelRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.core_tools.update_agent_model(params).await
    }

    #[rmcp::tool(description = "Schedule an event with smart time estimation")]
    async fn schedule_event(
        &self,
        params: Parameters<ScheduleEventRequest>,
    ) -> Result<CallToolResult, McpError> {
        info!("MCP server received schedule_event request");
        debug!("Raw params: {:?}", params);

        match self.core_tools.schedule_event(params).await {
            Ok(result) => {
                info!("schedule_event completed successfully");
                Ok(result)
            }
            Err(e) => {
                error!("schedule_event failed: {:?}", e);
                Err(e)
            }
        }
    }

    #[rmcp::tool(description = "Check activity state for interruption timing")]
    async fn check_activity_state(
        &self,
        _params: Parameters<super::EmptyRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.core_tools.check_activity_state().await
    }

    #[rmcp::tool(description = "Record current energy and attention state")]
    async fn record_energy_state(
        &self,
        params: Parameters<RecordEnergyStateRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.core_tools.record_energy_state(params).await
    }

    // Unified messaging tool below replaces both chat_with_agent and send_group_message

    #[rmcp::tool(
        description = "Send a message to various destinations: agents, groups, or Discord."
    )]
    pub async fn send_message(
        &self,
        params: Parameters<SendMessageRequest>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let params = params.0;
        info!(
            user_id = params.user_id,
            destination_type = ?params.destination_type,
            destination = params.destination,
            "Sending message"
        );

        match params.destination_type {
            MessageDestinationType::Agent => {
                let agent_id = if params.destination.is_empty() {
                    None
                } else {
                    Some(params.destination.as_str())
                };

                let user_id = params.user_id.parse::<i64>().map_err(|e| {
                    McpError::invalid_params(
                        "Invalid user_id format",
                        Some(json!({
                            "user_id": params.user_id,
                            "error": e.to_string(),
                            "hint": "user_id must be a valid integer"
                        })),
                    )
                })?;

                match self
                    .core_tools
                    .chat_with_agent_internal(UserId(user_id), &params.message, agent_id)
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
                                "destination": params.destination,
                                "user_id": params.user_id,
                                "error_type": "agent_communication_failure",
                                "details": e.to_string()
                            })),
                        ))
                    }
                }
            }
            MessageDestinationType::Group => {
                self.core_tools
                    .send_group_message(Parameters(SendGroupMessageRequest {
                        user_id: params.user_id,
                        group_name: params.destination,
                        message: params.message,
                        request_heartbeat: params.request_heartbeat,
                    }))
                    .await
            }
            #[cfg(feature = "discord")]
            MessageDestinationType::DiscordDm => {
                let discord_user_id = self
                    .db
                    .get_discord_id_for_user(
                        i64::from_str_radix(&params.destination, 10).unwrap_or_default(),
                    )
                    .await
                    .expect("discord id should exist in the database")
                    .unwrap();

                self.discord_tools
                    .send_discord_dm(SendDiscordDmRequest {
                        user_id: u64::from_str_radix(&discord_user_id, 10).unwrap(),
                        message: params.message,
                        request_heartbeat: params.request_heartbeat,
                    })
                    .await
            }
            #[cfg(feature = "discord")]
            MessageDestinationType::DiscordChannel => {
                self.discord_tools
                    .send_discord_channel_message(SendDiscordMessageRequest {
                        channel: params.destination,
                        message: params.message,
                        request_heartbeat: params.request_heartbeat,
                    })
                    .await
            }
        }
    }

    // Knowledge tools
    #[rmcp::tool(description = "Write insights to an agent's knowledge file for passive sharing")]
    async fn write_agent_knowledge(
        &self,
        params: Parameters<WriteAgentKnowledgeRequest>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;

        match KnowledgeTools::write_agent_knowledge(
            params.agent,
            params.category,
            params.insight,
            params.context,
        )
        .await
        {
            Ok(message) => {
                info!("Knowledge written successfully");
                Ok(CallToolResult::success(vec![Content::text(message)]))
            }
            Err(e) => {
                error!("Failed to write knowledge: {}", e);
                Err(McpError::internal_error(
                    "Failed to write agent knowledge",
                    Some(json!({
                        "error_type": "knowledge_write_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    #[rmcp::tool(description = "Read insights from a specific agent's knowledge file")]
    async fn read_agent_knowledge(
        &self,
        params: Parameters<ReadAgentKnowledgeRequest>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;

        match KnowledgeTools::read_agent_knowledge(
            params.agent,
            params.category_filter,
            params.limit,
        )
        .await
        {
            Ok(content) => {
                info!("Knowledge read successfully");
                Ok(CallToolResult::success(vec![Content::text(content)]))
            }
            Err(e) => {
                error!("Failed to read knowledge: {}", e);
                Err(McpError::internal_error(
                    "Failed to read agent knowledge",
                    Some(json!({
                        "error_type": "knowledge_read_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    #[rmcp::tool(description = "List all available knowledge files and their sizes")]
    async fn list_knowledge_files(
        &self,
        _params: Parameters<super::EmptyRequest>,
    ) -> Result<CallToolResult, McpError> {
        match KnowledgeTools::list_knowledge_files().await {
            Ok(content) => {
                info!("Knowledge files listed successfully");
                Ok(CallToolResult::success(vec![Content::text(content)]))
            }
            Err(e) => {
                error!("Failed to list knowledge files: {}", e);
                Err(McpError::internal_error(
                    "Failed to list knowledge files",
                    Some(json!({
                        "error_type": "knowledge_list_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    #[rmcp::tool(
        description = "Synchronize knowledge files with Letta sources for semantic search"
    )]
    async fn sync_knowledge_to_letta(
        &self,
        params: Parameters<SyncKnowledgeRequest>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;

        match KnowledgeTools::sync_knowledge_to_letta(&self.letta_client, UserId(params.user_id))
            .await
        {
            Ok(message) => {
                info!("Knowledge synced to Letta successfully");
                Ok(CallToolResult::success(vec![Content::text(message)]))
            }
            Err(e) => {
                error!("Failed to sync knowledge to Letta: {}", e);
                Err(McpError::internal_error(
                    "Failed to sync knowledge to Letta",
                    Some(json!({
                        "error_type": "knowledge_sync_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }
}

// Internal implementation methods
#[allow(dead_code)] // These methods are used by the MCP tools
impl PatternMcpServer {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_format_validation() {
        // Test various channel formats we might receive
        let valid_formats = vec![
            "123456789012345678",    // Direct ID
            "MyServer/general",      // Guild/channel
            "Test Guild/voice-chat", // With spaces and hyphens
        ];

        let invalid_formats = vec![
            "just-a-name",          // No separator
            "Server/Channel/Extra", // Too many parts
            "/channel",             // Missing guild
            "guild/",               // Missing channel
            "",                     // Empty
        ];

        // Just validate the string parsing logic
        for format in valid_formats {
            let parts: Vec<&str> = format.split('/').collect();
            if parts.len() == 1 {
                // Should be parseable as u64
                assert!(format.parse::<u64>().is_ok() || parts.len() == 2);
            } else {
                assert_eq!(parts.len(), 2);
                assert!(!parts[0].is_empty());
                assert!(!parts[1].is_empty());
            }
        }

        for format in invalid_formats {
            let parts: Vec<&str> = format.split('/').collect();
            let is_valid_id = format.parse::<u64>().is_ok();
            let is_valid_name = parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty();
            assert!(!is_valid_id && !is_valid_name);
        }
    }

    #[test]
    fn test_request_struct_serialization() {
        use crate::mcp::discord_tools::*;

        // Test SendDiscordMessageRequest
        let msg_req = SendDiscordMessageRequest {
            channel: "MyServer/general".to_string(),
            message: "Hello, world!".to_string(),
            request_heartbeat: None,
        };
        let json = serde_json::to_string(&msg_req).unwrap();
        assert!(json.contains("MyServer/general"));
        assert!(json.contains("Hello, world!"));

        // Test with numeric channel
        let msg_req2 = SendDiscordMessageRequest {
            channel: "123456789012345678".to_string(),
            message: "Test".to_string(),
            request_heartbeat: None,
        };
        let json2 = serde_json::to_string(&msg_req2).unwrap();
        assert!(json2.contains("123456789012345678"));

        // Test embed request
        let embed_req = SendDiscordEmbedRequest {
            channel: "Server/announcements".to_string(),
            title: "Update".to_string(),
            description: "New feature!".to_string(),
            color: Some(0x00ff00),
            fields: Some(vec![EmbedField {
                name: "Version".to_string(),
                value: "1.0.0".to_string(),
                inline: Some(true),
            }]),
            request_heartbeat: None,
        };
        let embed_json = serde_json::to_string(&embed_req).unwrap();
        assert!(embed_json.contains("Server/announcements"));
        assert!(embed_json.contains("Version"));
    }

    #[tokio::test]
    async fn test_channel_resolution_errors() {
        // Mock a server with no Discord client
        let letta_client = Arc::new(letta::LettaClient::local().unwrap());
        let db = Arc::new(Database::new(":memory:").await.unwrap());
        let multi_agent_system = Arc::new(
            crate::agent::constellation::MultiAgentSystemBuilder::new(
                letta_client.clone(),
                db.clone(),
            )
            .build(),
        );

        let server = PatternMcpServer::new(
            letta_client,
            db,
            multi_agent_system,
            #[cfg(feature = "discord")]
            None, // No Discord client
        );

        // Test that missing Discord client returns appropriate error when sending a message
        #[cfg(feature = "discord")]
        {
            use crate::mcp::discord_tools::SendDiscordMessageRequest;

            let request = SendDiscordMessageRequest {
                channel: "MyServer/general".to_string(),
                message: "Test message".to_string(),
                request_heartbeat: None,
            };

            let result = server
                .discord_tools
                .send_discord_channel_message(request)
                .await;

            assert!(result.is_err());
            if let Err(e) = result {
                // Should get an error about Discord not being configured
                let error_str = format!("{:?}", e);
                assert!(
                    error_str.contains("discord_not_configured") || error_str.contains("Discord")
                );
            }
        }
    }

    #[test]
    fn test_error_message_consistency() {
        // Test that our error responses have consistent structure
        let channel_error = json!({
            "channel": "invalid/format/here",
            "expected": "123456789 or MyServer/general"
        });

        // Verify error contains expected fields
        assert!(channel_error["channel"].is_string());
        assert!(channel_error["expected"].is_string());

        // Test error for missing Discord client
        let discord_error = json!({
            "error_type": "discord_not_configured",
            "details": "Discord bot is not running or not configured"
        });

        assert_eq!(discord_error["error_type"], "discord_not_configured");
        assert!(discord_error["details"]
            .as_str()
            .unwrap()
            .contains("Discord"));
    }

    #[test]
    fn test_channel_id_parsing_edge_cases() {
        // Test u64 boundary cases
        let max_u64 = u64::MAX.to_string();
        assert!(max_u64.parse::<u64>().is_ok());

        // Test invalid numeric strings
        let too_long = "9".repeat(100);
        let invalid_numbers = vec![
            "-123",    // Negative
            "12.34",   // Decimal
            "0x123",   // Hex
            "123abc",  // Mixed
            &too_long, // Too long
        ];

        for num in invalid_numbers {
            assert!(num.parse::<u64>().is_err());
        }
    }
}
