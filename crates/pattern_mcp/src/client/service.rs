//! MCP Client Service - Simplified stub implementation

use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

use super::{ClientTransport, ToolRequest, ToolResponse};
use crate::Result;
use pattern_core::tool::DynamicTool;

/// Configuration for an MCP server connection
#[derive(Debug, Clone, Default)]
pub struct McpServerConfig {
    /// Name to identify this server
    pub name: String,
    /// Command to run the MCP server
    pub command: String,
    /// Arguments for the command
    pub args: Vec<String>,
}

/// MCP Client Service for a single MCP server connection
pub struct McpClientService {
    /// Server configuration
    server_config: McpServerConfig,
    /// Channel for receiving tool requests
    request_rx: mpsc::Receiver<ToolRequest>,
    /// Channel for sending tool responses
    response_tx: broadcast::Sender<ToolResponse>,
    /// Sender for requests (to clone for tool wrappers)
    request_tx: mpsc::Sender<ToolRequest>,
    /// Persistent connection to the MCP server
    connection: Option<ClientTransport>,
}

impl McpClientService {
    /// Create a new MCP client service
    pub fn new(server_config: McpServerConfig) -> Self {
        let (request_tx, request_rx) = mpsc::channel(100);
        let (response_tx, _) = broadcast::channel(100);

        Self {
            server_config,
            request_rx,
            response_tx,
            request_tx,
            connection: None,
        }
    }

    /// Get the request sender for creating tool wrappers
    pub fn request_sender(&self) -> mpsc::Sender<ToolRequest> {
        self.request_tx.clone()
    }

    /// Get a response receiver for tool wrappers
    pub fn response_receiver(&self) -> broadcast::Receiver<ToolResponse> {
        self.response_tx.subscribe()
    }

    /// Initialize connection to the MCP server
    pub async fn initialize(&mut self) -> Result<()> {
        info!(
            "MCP client service connecting to: {}",
            self.server_config.name
        );

        match ClientTransport::stdio(
            self.server_config.command.clone(),
            self.server_config.args.clone(),
        )
        .await
        {
            Ok(transport) => {
                info!(
                    "Successfully connected to MCP server: {}",
                    self.server_config.name
                );
                self.connection = Some(transport);
            }
            Err(e) => {
                warn!(
                    "Failed to connect to MCP server {}: {}",
                    self.server_config.name, e
                );
                return Err(e);
            }
        }

        info!("MCP client service initialized");
        Ok(())
    }

    /// Get all available tools as DynamicTool instances
    pub async fn get_tools(&self) -> Result<Vec<Box<dyn DynamicTool>>> {
        use super::{McpToolWrapper, ToolDiscovery};

        let mut tools: Vec<Box<dyn DynamicTool>> = vec![];

        // Use persistent connection to discover tools
        if let Some(transport) = &self.connection {
            info!(
                "Discovering tools from connected MCP server: {}",
                self.server_config.name
            );

            match ToolDiscovery::discover_tools(transport.peer()).await {
                Ok(discovered_tools) => {
                    info!(
                        "Discovered {} tools from {}",
                        discovered_tools.len(),
                        self.server_config.name
                    );

                    for tool_info in discovered_tools {
                        let tool_wrapper = McpToolWrapper::new(
                            tool_info.name,
                            self.server_config.name.clone(),
                            tool_info.description,
                            tool_info.input_schema,
                            self.request_tx.clone(),
                            self.response_tx.clone(),
                        );
                        tools.push(Box::new(tool_wrapper));
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to discover tools from {}: {}",
                        self.server_config.name, e
                    );
                }
            }
        }

        // Fallback to mock tools if no server connected
        if tools.is_empty() {
            info!("No MCP server connected, creating mock tools for testing");
            tools.extend(self.create_mock_tools());
        }

        info!("MCP client service: returning {} tools total", tools.len());
        Ok(tools)
    }

    /// Create mock tools for testing when no MCP servers are available
    fn create_mock_tools(&self) -> Vec<Box<dyn DynamicTool>> {
        use super::McpToolWrapper;
        use serde_json::json;

        vec![
            Box::new(McpToolWrapper::new(
                "echo".to_string(),
                "mock_server".to_string(),
                "Echo the input back".to_string(),
                json!({
                    "type": "object",
                    "properties": {
                        "text": {
                            "type": "string",
                            "description": "Text to echo back"
                        }
                    },
                    "required": ["text"]
                }),
                self.request_tx.clone(),
                self.response_tx.clone(),
            )),
            Box::new(McpToolWrapper::new(
                "current_time".to_string(),
                "mock_server".to_string(),
                "Get the current time".to_string(),
                json!({
                    "type": "object",
                    "properties": {}
                }),
                self.request_tx.clone(),
                self.response_tx.clone(),
            )),
        ]
    }

    /// Run the service, processing tool requests
    pub async fn run(mut self) -> tokio::task::JoinHandle<()> {
        let is_connected = self.connection.is_some();
        info!(
            "Starting MCP client service with connection: {}",
            is_connected
        );

        tokio::spawn(async move {
            while let Some(request) = self.request_rx.recv().await {
                info!(
                    "MCP client service: Processing request '{}' for tool '{}'",
                    request.id, request.tool
                );

                let response = Self::handle_mock_tool_request(request.clone()).await;

                if let Err(e) = self.response_tx.send(response) {
                    warn!("Failed to send tool response: {}", e);
                }
            }
        })
    }

    /// Handle a mock tool request for testing
    async fn handle_mock_tool_request(request: ToolRequest) -> ToolResponse {
        match request.tool.as_str() {
            "echo" => {
                if let Some(text) = request.params.get("text") {
                    ToolResponse::success(
                        request.id,
                        serde_json::json!({
                            "echoed": text,
                            "original_params": request.params,
                            "_mock": true
                        }),
                    )
                } else {
                    ToolResponse::error(request.id, "Missing required parameter 'text'".to_string())
                }
            }
            "current_time" => {
                let now = chrono::Utc::now();
                ToolResponse::success(
                    request.id,
                    serde_json::json!({
                        "timestamp": now.to_rfc3339(),
                        "unix_timestamp": now.timestamp(),
                        "formatted": now.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                        "_mock": true
                    }),
                )
            }
            _ => ToolResponse::error(
                request.id,
                format!("Tool '{}' not found (mock implementation)", request.tool),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_service_creation() {
        let server_config = McpServerConfig {
            name: "test_server".to_string(),
            command: "echo".to_string(),
            args: vec![],
        };

        let service = McpClientService::new(server_config);
        let tools = service.get_tools().await.unwrap();
        assert_eq!(tools.len(), 2); // echo + current_time (mock tools)
    }

    #[tokio::test]
    async fn test_mock_tool_execution() {
        use serde_json::json;
        use tokio::time::{Duration, timeout};

        let server_config = McpServerConfig::default();
        let service = McpClientService::new(server_config);

        // Get tools first
        let tools = service.get_tools().await.unwrap();
        let echo_tool = tools
            .iter()
            .find(|t| t.name() == "echo")
            .expect("Echo tool should exist");

        // Start the service in background BEFORE calling execute
        let service_handle = service.run().await;

        // Give the service a moment to start
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Test with timeout to prevent hanging
        let result = timeout(
            Duration::from_secs(5),
            echo_tool.execute(json!({
                "text": "Hello, MCP!"
            })),
        )
        .await;

        // Check that we got a response within timeout
        assert!(result.is_ok(), "Tool execution timed out");
        let response = result.unwrap().unwrap();
        assert_eq!(response["echoed"], "Hello, MCP!");

        // Cleanup
        service_handle.abort();
    }
}
