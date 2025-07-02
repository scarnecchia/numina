use crate::{agent::UserId, db::Database};
use letta::{Block, CreateAgentRequest, LettaClient};
use miette::{IntoDiagnostic, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

/// Configuration for an agent in the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Unique identifier for this agent type
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Description of the agent's role
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
    pub name: String,
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
    agent_configs: HashMap<String, AgentConfig>,
    memory_configs: HashMap<String, MemoryBlockConfig>,
}

impl MultiAgentSystem {
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
                    block_name,
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
        agent_id: &str,
        config: &AgentConfig,
    ) -> Result<String> {
        let user = self
            .db
            .get_or_create_user(&format!("user_{}", user_id.0))
            .await?;

        // Check if agent already exists
        let agent_name = format!("{}_u{}", agent_id, user_id.0);
        if let Some(existing) = self.db.get_agent_for_user(user.id).await? {
            if existing.name == agent_name {
                debug!("Agent {} already exists", agent_name);
                return Ok(existing.letta_agent_id);
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
            .memory_blocks(memory_blocks)
            .build();

        let agent = self
            .letta
            .agents()
            .create(request)
            .await
            .into_diagnostic()?;

        // Store agent in database
        self.db
            .create_agent(user.id, &agent.id.to_string(), &agent_name)
            .await?;

        info!("Created {} agent for user {}", config.name, user_id.0);
        Ok(agent.id.to_string())
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
            .get(block_name)
            .ok_or_else(|| miette::miette!("Unknown memory block: {}", block_name))?;

        // Validate length
        if block_value.len() > config.max_length {
            return Err(miette::miette!(
                "Value exceeds max length of {} for block {}",
                config.max_length,
                block_name
            ));
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
            .ok_or_else(|| miette::miette!("Memory block '{}' not found", block_name))?;

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
    pub fn agent_configs(&self) -> &HashMap<String, AgentConfig> {
        &self.agent_configs
    }

    /// Get configured memory blocks
    pub fn memory_configs(&self) -> &HashMap<String, MemoryBlockConfig> {
        &self.memory_configs
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
