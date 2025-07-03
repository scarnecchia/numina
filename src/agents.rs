use crate::{
    agent::UserId,
    db::Database,
    error::{AgentError, Result},
    types::{AgentId, MemoryBlockId},
};
use letta::{
    types::LettaMessageUnion, Block, CreateAgentRequest, CreateMessagesRequest, LettaClient,
    MessageCreate,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info};
/// Configuration for an agent in the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Unique identifier for this agent type
    pub id: AgentId,
    pub name: String,
    pub description: String,
    /// System prompt for the agent
    pub system_prompt: String,
    /// Whether this agent runs as a sleeptime agent
    pub is_sleeptime: bool,
    /// Interval for sleeptime checks (in seconds)
    pub sleeptime_interval: Option<u64>,
}

/// Configuration for a shared memory block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBlockConfig {
    /// Unique identifier for this memory block
    pub name: MemoryBlockId,
    /// Maximum length for the block value
    pub max_length: usize,
    /// Default value when initializing
    pub default_value: String,
    /// Description of what this block stores
    pub description: String,
}

/// Multi-agent system that can be configured with different agent types
#[derive(Debug, Clone)]
pub struct MultiAgentSystem {
    letta: Arc<LettaClient>,
    db: Arc<Database>,
    agent_configs: HashMap<AgentId, AgentConfig>,
    memory_configs: HashMap<MemoryBlockId, MemoryBlockConfig>,
}

impl MultiAgentSystem {
    /// Configure Letta to use our MCP server for Discord tools
    pub async fn configure_mcp_server(&self, mcp_port: u16) -> Result<()> {
        use letta::types::tool::{McpServerConfig, McpServerType, StreamableHttpServerConfig};

        info!("Configuring Pattern MCP server in Letta");

        // First, try to list existing tools to see if server is already configured
        match self
            .letta
            .tools()
            .list_mcp_tools_by_server("pattern-discord")
            .await
        {
            Ok(tools) => {
                info!(
                    "MCP server 'pattern-discord' already configured with {} tools",
                    tools.len()
                );
                return Ok(());
            }
            Err(_) => {
                // Server doesn't exist yet, continue with configuration
                debug!("MCP server not found, creating new configuration");
            }
        }

        // Create MCP server config for streamable HTTP transport
        let server_config = McpServerConfig::StreamableHttp(StreamableHttpServerConfig {
            server_name: "pattern-discord".to_string(),
            server_type: Some(McpServerType::StreamableHttp),
            server_url: format!("http://localhost:{}/mcp", mcp_port),
            auth_header: None, // Could add auth later if needed
            auth_token: None,
            custom_headers: None,
        });

        // Add the MCP server to Letta
        match self.letta.tools().add_mcp_server(server_config).await {
            Ok(_) => {
                info!("Successfully added MCP server to Letta");
            }
            Err(e) => {
                // Check if it's a "already exists" error
                match &e {
                    letta::LettaError::Api {
                        status, message, ..
                    } => {
                        if *status == 409 || message.contains("already exists") {
                            info!("MCP server already exists in Letta, continuing...");
                        } else {
                            return Err(crate::error::PatternError::Agent(
                                AgentError::CreationFailed {
                                    name: "MCP server config".to_string(),
                                    source: e,
                                },
                            ));
                        }
                    }
                    _ => {
                        return Err(crate::error::PatternError::Agent(
                            AgentError::CreationFailed {
                                name: "MCP server config".to_string(),
                                source: e,
                            },
                        ));
                    }
                }
            }
        }

        // Wait a moment for the server to be fully ready
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // List available tools from the MCP server
        info!("Listing tools from MCP server 'pattern-discord'...");
        let tools = match self
            .letta
            .tools()
            .list_mcp_tools_by_server("pattern-discord")
            .await
        {
            Ok(tools) => tools,
            Err(e) => {
                error!("Failed to list MCP tools: {:?}", e);
                // Try to get more information about the error
                match &e {
                    letta::LettaError::Api {
                        status,
                        message,
                        body,
                        ..
                    } => {
                        error!(
                            "API Error - Status: {}, Message: {}, Body: {:?}",
                            status, message, body
                        );
                    }
                    letta::LettaError::Http(http_err) => {
                        error!("HTTP Error: {:?}", http_err);
                    }
                    _ => {
                        error!("Other error type: {:?}", e);
                    }
                }
                return Err(crate::error::PatternError::Agent(
                    AgentError::CreationFailed {
                        name: "MCP tools list".to_string(),
                        source: e,
                    },
                ));
            }
        };

        info!("MCP server configured with {} tools", tools.len());
        for tool in &tools {
            debug!(
                "  - {}: {}",
                tool.name,
                tool.description.as_deref().unwrap_or("No description")
            );
        }

        Ok(())
    }
    /// Create a new multi-agent system
    pub fn new(letta: Arc<LettaClient>, db: Arc<Database>) -> Self {
        Self {
            letta,
            db,
            agent_configs: HashMap::new(),
            memory_configs: HashMap::new(),
        }
    }

    /// Configure an agent type
    pub fn add_agent_config(&mut self, config: AgentConfig) {
        self.agent_configs.insert(config.id.clone(), config);
    }

    /// Configure a shared memory block
    pub fn add_memory_config(&mut self, config: MemoryBlockConfig) {
        self.memory_configs.insert(config.name.clone(), config);
    }

    /// Initialize all configured agents for a user
    pub async fn initialize_user(&self, user_id: UserId) -> Result<()> {
        info!("Initializing multi-agent system for user {}", user_id.0);

        // Initialize shared memory blocks
        self.initialize_shared_memory(user_id).await?;

        // Create agents
        for (agent_id, config) in &self.agent_configs {
            self.create_or_get_agent(user_id, agent_id, config).await?;
        }

        info!("Multi-agent system initialized for user {}", user_id.0);
        Ok(())
    }

    /// Initialize shared memory blocks for a user
    async fn initialize_shared_memory(&self, user_id: UserId) -> Result<()> {
        let user = self
            .db
            .get_or_create_user(&format!("user_{}", user_id.0))
            .await?;

        for (block_name, config) in &self.memory_configs {
            self.db
                .upsert_shared_memory(
                    user.id,
                    block_name.as_str(),
                    &config.default_value,
                    config.max_length as i32,
                )
                .await?;
        }
        debug!(
            "Initialized {} shared memory blocks for user {}",
            self.memory_configs.len(),
            user_id.0
        );
        Ok(())
    }

    /// Create or get a specific agent for a user
    async fn create_or_get_agent(
        &self,
        user_id: UserId,
        agent_id: &AgentId,
        config: &AgentConfig,
    ) -> Result<letta::LettaId> {
        let user = self
            .db
            .get_or_create_user(&format!("user_{}", user_id.0))
            .await?;

        // Check if agent already exists
        // Use a simpler naming scheme to avoid validation issues
        // Hash the user_id to keep it shorter while still unique
        let user_hash = format!("{:x}", user_id.0 % 1000000); // Keep it to 6 hex chars max
        let agent_name = format!("{}_{}", agent_id.as_str(), user_hash);
        if let Some(existing) = self.db.get_agent_for_user(user.id).await? {
            if existing.name == agent_name {
                debug!("Agent {} already exists", agent_name);
                return existing.letta_agent_id.parse().map_err(|_| {
                    crate::error::AgentError::InvalidLettaId(existing.letta_agent_id).into()
                });
            }
        }

        // Create new agent with shared memory
        let shared_memory = self.db.get_shared_memory(user.id).await?;

        // Build memory blocks
        let mut memory_blocks = vec![Block::persona(&config.system_prompt)];

        // Add shared memory blocks as context
        for block in shared_memory {
            let content = format!("{}: {}", block.block_name, block.block_value);
            memory_blocks.push(Block::human(&content));
        }

        let request = CreateAgentRequest::builder()
            .name(&agent_name)
            .model("letta/letta-free")
            .embedding("letta/letta-free")
            .agent_type(letta::types::AgentType::MemGPT)
            .include_base_tools(true) // Include standard Letta tools
            .tools(vec![
                "send_message".to_string(),           // Basic messaging
                "archival_memory_insert".to_string(), // Store important info
                "archival_memory_search".to_string(), // Retrieve context
            ])
            .memory_blocks(memory_blocks) // Add the memory blocks
            .build();

        info!(
            "Creating agent '{}' for user {} with agent_id '{}'",
            agent_name,
            user_id.0,
            agent_id.as_str()
        );

        let agent = self.letta.agents().create(request).await.map_err(|e| {
            error!("Failed to create agent '{}': {:?}", agent_name, e);
            crate::error::AgentError::CreationFailed {
                name: agent_name.clone(),
                source: e,
            }
        })?;

        info!(
            "Successfully created agent '{}' with ID: {}",
            agent_name, agent.id
        );

        // Store agent in database
        self.db
            .create_agent(user.id, &agent.id.to_string(), &agent_name)
            .await?;

        Ok(agent.id)
    }

    /// Update shared memory and sync to agents
    pub async fn update_shared_memory(
        &self,
        user_id: UserId,
        block_name: &str,
        block_value: &str,
    ) -> Result<()> {
        let user = self
            .db
            .get_or_create_user(&format!("user_{}", user_id.0))
            .await?;

        // Validate block exists in config
        let config = self
            .memory_configs
            .get(&MemoryBlockId::new(block_name)?)
            .ok_or_else(|| crate::error::DatabaseError::MemoryBlockNotFound {
                user_id: user.id,
                block: block_name.to_string(),
            })?;

        // Validate length
        if block_value.len() > config.max_length {
            return Err(AgentError::MemoryTooLong {
                block: MemoryBlockId::new(block_name)?,
                max: config.max_length,
                actual: block_value.len(),
            }
            .into());
        }

        // Update in database
        self.db
            .upsert_shared_memory(user.id, block_name, block_value, config.max_length as i32)
            .await?;

        // TODO: Sync to agents via Letta blocks API

        Ok(())
    }

    /// Get current state from shared memory
    pub async fn get_shared_memory(&self, user_id: UserId, block_name: &str) -> Result<String> {
        let user = self
            .db
            .get_or_create_user(&format!("user_{}", user_id.0))
            .await?;
        let block = self
            .db
            .get_shared_memory_block(user.id, block_name)
            .await?
            .ok_or_else(|| crate::error::DatabaseError::MemoryBlockNotFound {
                user_id: user.id,
                block: block_name.to_string(),
            })?;

        Ok(block.block_value)
    }

    /// Get all shared memory for a user
    pub async fn get_all_shared_memory(&self, user_id: UserId) -> Result<HashMap<String, String>> {
        let user = self
            .db
            .get_or_create_user(&format!("user_{}", user_id.0))
            .await?;

        let blocks = self.db.get_shared_memory(user.id).await?;

        Ok(blocks
            .into_iter()
            .map(|b| (b.block_name, b.block_value))
            .collect())
    }

    /// Get configured agent types
    pub fn agent_configs(&self) -> &HashMap<AgentId, AgentConfig> {
        &self.agent_configs
    }

    /// Get configured memory blocks
    pub fn memory_configs(&self) -> &HashMap<MemoryBlockId, MemoryBlockConfig> {
        &self.memory_configs
    }

    /// Send a message to a specific agent
    pub async fn send_message_to_agent(
        &self,
        user_id: UserId,
        agent_id: Option<&str>,
        message: &str,
    ) -> Result<String> {
        // Determine which agent to use
        let target_agent = agent_id
            .and_then(|id| AgentId::new(id).ok())
            .unwrap_or_else(|| AgentId::new("pattern").unwrap());

        // Get the agent config
        let config =
            self.agent_configs
                .get(&target_agent)
                .ok_or_else(|| AgentError::UnknownAgent {
                    agent: target_agent.clone(),
                })?;

        // Create or get the agent for this user
        let letta_agent_id = self
            .create_or_get_agent(user_id, &target_agent, config)
            .await?;

        // Send message to the agent
        debug!(
            "Sending message to agent {} (ID: {})",
            target_agent.as_str(),
            letta_agent_id
        );
        let user_message = MessageCreate::user(message);

        let request = CreateMessagesRequest::builder()
            .messages(vec![user_message])
            .build();

        debug!("Calling Letta API to send message...");
        let response = self
            .letta
            .messages()
            .create(&letta_agent_id, request)
            .await
            .map_err(|e| {
                error!("Message send failed after timeout: {:?}", e);
                AgentError::MessageFailed {
                    agent: target_agent.clone(),
                    source: e,
                }
            })?;

        debug!(
            "Received response from Letta with {} messages",
            response.messages.len()
        );

        // Extract the assistant's response
        let assistant_message = response
            .messages
            .iter()
            .filter_map(|msg| match msg {
                LettaMessageUnion::AssistantMessage(assistant) => Some(assistant.content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        if assistant_message.is_empty() {
            Ok("I'm still processing that thought...".to_string())
        } else {
            Ok(assistant_message)
        }
    }

    /// Get reference to the old agent manager for compatibility
    pub fn agent_manager(&self) -> Option<&crate::agent::AgentManager> {
        None // Multi-agent system doesn't use the old agent manager
    }
}

/// Builder pattern for easier configuration
pub struct MultiAgentSystemBuilder {
    letta: Arc<LettaClient>,
    db: Arc<Database>,
    agent_configs: Vec<AgentConfig>,
    memory_configs: Vec<MemoryBlockConfig>,
}

impl MultiAgentSystemBuilder {
    pub fn new(letta: Arc<LettaClient>, db: Arc<Database>) -> Self {
        Self {
            letta,
            db,
            agent_configs: Vec::new(),
            memory_configs: Vec::new(),
        }
    }

    pub fn with_agent(mut self, config: AgentConfig) -> Self {
        self.agent_configs.push(config);
        self
    }

    pub fn with_memory_block(mut self, config: MemoryBlockConfig) -> Self {
        self.memory_configs.push(config);
        self
    }

    pub fn build(self) -> MultiAgentSystem {
        let mut system = MultiAgentSystem::new(self.letta, self.db);

        for config in self.agent_configs {
            system.add_agent_config(config);
        }

        for config in self.memory_configs {
            system.add_memory_config(config);
        }

        system
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{StandardAgent, StandardMemoryBlock};

    #[tokio::test]
    async fn test_memory_block_validation() {
        let db = Arc::new(Database::new(":memory:").await.unwrap());
        db.migrate().await.unwrap();
        let letta = Arc::new(letta::LettaClient::local().unwrap());

        let mut system = MultiAgentSystem::new(letta, db);

        // Add memory config
        system.add_memory_config(MemoryBlockConfig {
            name: StandardMemoryBlock::CurrentState.id(),
            max_length: 50,
            default_value: "test".to_string(),
            description: "Test block".to_string(),
        });

        let user_id = UserId(1);
        system.initialize_user(user_id).await.unwrap();

        // Test length validation
        let long_value = "a".repeat(100);
        let result = system
            .update_shared_memory(user_id, "current_state", &long_value)
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::PatternError::Agent(AgentError::MemoryTooLong {
                max, actual, ..
            }) => {
                assert_eq!(max, 50);
                assert_eq!(actual, 100);
            }
            _ => panic!("Expected MemoryTooLong error"),
        }

        // Valid update should work
        let result = system
            .update_shared_memory(user_id, "current_state", "valid value")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_agent_config_uniqueness() {
        let db = Arc::new(Database::new(":memory:").await.unwrap());
        db.migrate().await.unwrap();
        let letta = Arc::new(letta::LettaClient::local().unwrap());

        let mut system = MultiAgentSystem::new(letta, db);

        // Add two agents with same ID (second should override)
        let config1 = AgentConfig {
            id: StandardAgent::Pattern.id(),
            name: "Pattern v1".to_string(),
            description: "First version".to_string(),
            system_prompt: "Prompt 1".to_string(),
            is_sleeptime: true,
            sleeptime_interval: Some(1800),
        };

        let config2 = AgentConfig {
            id: StandardAgent::Pattern.id(),
            name: "Pattern v2".to_string(),
            description: "Second version".to_string(),
            system_prompt: "Prompt 2".to_string(),
            is_sleeptime: false,
            sleeptime_interval: None,
        };

        system.add_agent_config(config1);
        system.add_agent_config(config2);

        assert_eq!(system.agent_configs().len(), 1);
        let stored = system
            .agent_configs()
            .get(&StandardAgent::Pattern.id())
            .unwrap();
        assert_eq!(stored.name, "Pattern v2");
        assert!(!stored.is_sleeptime);
    }

    #[tokio::test]
    async fn test_builder_pattern() {
        let db = Arc::new(Database::new(":memory:").await.unwrap());
        db.migrate().await.unwrap();
        let letta = Arc::new(letta::LettaClient::local().unwrap());

        let system = MultiAgentSystemBuilder::new(letta, db)
            .with_agent(AgentConfig {
                id: StandardAgent::Entropy.id(),
                name: "Entropy".to_string(),
                description: "Task agent".to_string(),
                system_prompt: "Break down tasks".to_string(),
                is_sleeptime: false,
                sleeptime_interval: None,
            })
            .with_memory_block(MemoryBlockConfig {
                name: StandardMemoryBlock::ActiveContext.id(),
                max_length: 400,
                default_value: "idle".to_string(),
                description: "Current context".to_string(),
            })
            .build();

        assert_eq!(system.agent_configs().len(), 1);
        assert_eq!(system.memory_configs().len(), 1);
        assert!(system
            .agent_configs()
            .contains_key(&StandardAgent::Entropy.id()));
    }

    #[tokio::test]
    async fn test_unknown_agent_error() {
        let db = Arc::new(Database::new(":memory:").await.unwrap());
        db.migrate().await.unwrap();
        let letta = Arc::new(letta::LettaClient::local().unwrap());

        let system = MultiAgentSystem::new(letta, db);
        let user_id = UserId(1);

        // Try to send message to non-existent agent
        let result = system
            .send_message_to_agent(user_id, Some("nonexistent"), "hello")
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::PatternError::Agent(AgentError::UnknownAgent { agent }) => {
                assert_eq!(agent.as_str(), "nonexistent");
            }
            _ => panic!("Expected UnknownAgent error"),
        }
    }

    #[tokio::test]
    async fn test_memory_block_not_found() {
        let db = Arc::new(Database::new(":memory:").await.unwrap());
        db.migrate().await.unwrap();
        let letta = Arc::new(letta::LettaClient::local().unwrap());

        let system = MultiAgentSystem::new(letta, db);
        let user_id = UserId(1);

        // Try to update non-configured memory block
        let result = system
            .update_shared_memory(user_id, "unknown_block", "value")
            .await;

        assert!(result.is_err());
        // Should fail because block isn't configured
    }

    #[test]
    fn test_agent_config_serialization() {
        let config = AgentConfig {
            id: StandardAgent::Flux.id(),
            name: "Flux".to_string(),
            description: "Time agent".to_string(),
            system_prompt: "Manage time".to_string(),
            is_sleeptime: false,
            sleeptime_interval: None,
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"id\":\"flux\""));
        assert!(json.contains("\"is_sleeptime\":false"));

        let deserialized: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, config.id);
        assert_eq!(deserialized.is_sleeptime, false);
    }
}
