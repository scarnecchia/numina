pub mod agent;
pub mod agents;
pub mod config;
pub mod db;
pub mod error;
pub mod service;
pub mod types;

#[cfg(feature = "discord")]
pub mod discord;

#[cfg(feature = "mcp")]
pub mod server;

#[cfg(feature = "mcp")]
pub mod mcp_tools;
use miette::Result;
use std::sync::Arc;

/// Core Pattern service that manages agents, tasks, and scheduling
#[derive(Debug, Clone)]
pub struct PatternService {
    db: Arc<db::Database>,
    letta_client: Option<Arc<letta::LettaClient>>,
    agent_manager: Option<Arc<agent::AgentManager>>,
    multi_agent_system: Option<Arc<agents::MultiAgentSystem>>,
}

impl PatternService {
    /// Create a new Pattern service with database connection
    pub async fn new(db_path: &str) -> Result<Self> {
        let db = db::Database::new(db_path).await?;
        db.migrate().await?;

        Ok(Self {
            db: Arc::new(db),
            letta_client: None,
            agent_manager: None,
            multi_agent_system: None,
        })
    }

    /// Create service with existing database connection
    pub fn with_database(db: db::Database) -> Self {
        Self {
            db: Arc::new(db),
            letta_client: None,
            agent_manager: None,
            multi_agent_system: None,
        }
    }

    /// Set the Letta client for agent operations
    pub fn with_letta_client(mut self, client: letta::LettaClient) -> Self {
        let letta_client = Arc::new(client);
        let agent_manager = Arc::new(agent::AgentManager::new(
            Arc::clone(&letta_client),
            Arc::clone(&self.db),
        ));
        let multi_agent_system = Arc::new(agents::MultiAgentSystem::new(
            Arc::clone(&letta_client),
            Arc::clone(&self.db),
        ));

        self.letta_client = Some(letta_client);
        self.agent_manager = Some(agent_manager);
        self.multi_agent_system = Some(multi_agent_system);
        self
    }

    /// Get a reference to the database
    pub fn db(&self) -> &db::Database {
        &self.db
    }

    /// Get a reference to the Letta client if available
    pub fn letta_client(&self) -> Option<&letta::LettaClient> {
        self.letta_client.as_ref().map(|c| c.as_ref())
    }

    /// Get a reference to the agent manager if available
    pub fn agent_manager(&self) -> Option<&agent::AgentManager> {
        self.agent_manager.as_ref().map(|m| m.as_ref())
    }

    /// Get a reference to the multi-agent system if available
    pub fn multi_agent_system(&self) -> Option<&agents::MultiAgentSystem> {
        self.multi_agent_system.as_ref().map(|m| m.as_ref())
    }
}

// Re-export commonly used types
pub use agent::{AgentInstance, AgentManager, UserId};
pub use agents::{AgentConfig, MemoryBlockConfig, MultiAgentSystem, MultiAgentSystemBuilder};
pub use db::{Agent, Database, Event, SharedMemory, Task, User};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_service_creation() {
        let service = PatternService::new(":memory:").await.unwrap();
        assert!(service.letta_client().is_none());
    }
}
