use crate::db;
use letta::{
    types::{
        AgentMemory, Block, CreateAgentRequest, CreateMessagesRequest, LettaId, MessageCreate,
    },
    LettaClient,
};
use miette::{miette, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// User identifier
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub struct UserId(pub i64);

/// Agent instance with metadata
#[derive(Debug, Clone)]
pub struct AgentInstance {
    pub agent_id: LettaId,
    pub user_id: UserId,
    pub name: String,
}

/// Manages Letta agents for users
#[derive(Debug, Clone)]
pub struct AgentManager {
    letta: Arc<LettaClient>,
    db: Arc<db::Database>,
    /// Cache of user_id -> agent instance
    cache: Arc<RwLock<HashMap<UserId, AgentInstance>>>,
}

impl AgentManager {
    /// Create a new agent manager
    pub fn new(letta: Arc<LettaClient>, db: Arc<db::Database>) -> Self {
        Self {
            letta,
            db,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get or create an agent for a user
    pub async fn get_or_create_agent(&self, user_id: UserId) -> Result<AgentInstance> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(agent) = cache.get(&user_id) {
                debug!("Found agent in cache for user {}", user_id.0);
                return Ok(agent.clone());
            }
        }

        // Check database
        if let Some(db_agent) = self.db.get_agent_for_user(user_id.0).await? {
            info!("Found existing agent in database for user {}", user_id.0);

            // Parse the stored agent ID
            let agent_id: LettaId = db_agent.letta_agent_id.parse().map_err(|_| {
                miette!("Invalid agent ID in database: {}", db_agent.letta_agent_id)
            })?;

            let instance = AgentInstance {
                agent_id,
                user_id: user_id.clone(),
                name: db_agent.name,
            };

            // Update cache
            self.cache.write().await.insert(user_id, instance.clone());
            return Ok(instance);
        }

        // Create new agent
        info!("Creating new agent for user {}", user_id.0);
        self.create_agent_for_user(user_id).await
    }

    /// Create a new agent for a user
    async fn create_agent_for_user(&self, user_id: UserId) -> Result<AgentInstance> {
        let agent_name = format!("assistant_{}", user_id.0);

        // Build agent creation request with memory blocks
        let persona_block = Block::persona("You are a helpful assistant with context awareness. You help users manage their tasks, schedule, and provide personalized support based on their patterns and preferences.");
        let human_block = Block::human(&format!("User {}", user_id.0));

        let request = CreateAgentRequest::builder()
            .name(&agent_name)
            .memory_block(persona_block)
            .memory_block(human_block)
            .build();

        // Create agent via Letta API
        let agent = self
            .letta
            .agents()
            .create(request)
            .await
            .map_err(|e| miette!("Failed to create Letta agent: {}", e))?;

        // Store in database
        self.db
            .create_agent(user_id.0, &agent.id.to_string(), &agent_name)
            .await?;

        let instance = AgentInstance {
            agent_id: agent.id,
            user_id: user_id.clone(),
            name: agent_name,
        };

        // Update cache
        self.cache.write().await.insert(user_id, instance.clone());

        info!(
            "Created new agent {} for user {}",
            instance.agent_id, user_id.0
        );
        Ok(instance)
    }

    /// Send a message to a user's agent
    pub async fn send_message(&self, user_id: UserId, message: &str) -> Result<String> {
        let agent = self.get_or_create_agent(user_id).await?;

        debug!("Sending message to agent {}: {}", agent.agent_id, message);

        // Create a user message
        let user_message = MessageCreate::user(message);

        let request = CreateMessagesRequest::builder()
            .messages(vec![user_message])
            .build();

        let response = self
            .letta
            .messages()
            .create(&agent.agent_id, request)
            .await
            .map_err(|e| miette!("Failed to send message to agent: {}", e))?;

        // Extract the assistant's response from the messages
        let assistant_message = response
            .messages
            .iter()
            .find_map(|msg| {
                // Check if this is an assistant message and extract content
                if let letta::types::LettaMessageUnion::AssistantMessage(m) = msg {
                    Some(m.content.clone())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "No response from agent".to_string());

        Ok(assistant_message)
    }

    /// Get the current memory state of a user's agent
    pub async fn get_agent_memory(&self, user_id: UserId) -> Result<AgentMemory> {
        let agent = self.get_or_create_agent(user_id).await?;

        let agent_details = self
            .letta
            .agents()
            .get(&agent.agent_id)
            .await
            .map_err(|e| miette!("Failed to get agent details: {}", e))?;

        agent_details
            .memory
            .ok_or_else(|| miette!("Agent has no memory configured"))
    }

    /// Update the memory of a user's agent
    pub async fn update_agent_memory(&self, user_id: UserId, memory: AgentMemory) -> Result<()> {
        let agent = self.get_or_create_agent(user_id).await?;

        // Workaround: Since Letta doesn't have a direct memory update API,
        // we need to update individual blocks
        for block in memory.blocks {
            // Each block has an ID we can use to update it
            if let Some(block_id) = &block.id {
                // First try to update the block directly using the blocks API
                self.letta
                    .blocks()
                    .update(
                        block_id,
                        letta::types::UpdateBlockRequest {
                            value: Some(block.value.clone()),
                            label: Some(block.label.clone()),
                            limit: block.limit,
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(|e| miette!("Failed to update memory block: {}", e))?;
            } else {
                // If the block doesn't have an ID, we can't update it
                return Err(miette!("Cannot update memory block without ID"));
            }
        }

        info!("Updated memory for agent {}", agent.agent_id);
        Ok(())
    }

    /// Clear the cache (useful for testing or memory management)
    pub async fn clear_cache(&self) {
        self.cache.write().await.clear();
        debug!("Agent cache cleared");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_user_id() {
        let user_id = UserId(123);
        assert_eq!(user_id.0, 123);
    }
}
