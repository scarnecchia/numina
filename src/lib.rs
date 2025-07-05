pub mod agent;
pub mod cache;
pub mod config;
pub mod db;
pub mod error;
pub mod service;

#[cfg(feature = "discord")]
pub mod discord;

#[cfg(feature = "mcp")]
pub mod mcp;
use miette::Result;
use std::sync::Arc;

/// Core Pattern service that manages agents, tasks, and scheduling
#[derive(Debug, Clone)]
pub struct PatternService {
    db: Arc<db::Database>,
    letta_client: Option<Arc<letta::LettaClient>>,
    agent_manager: Option<Arc<agent::coordination::AgentCoordinator>>,
    multi_agent_system: Option<Arc<agent::constellation::MultiAgentSystem>>,
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
        let multi_agent_system = Arc::new(agent::constellation::MultiAgentSystem::new(
            Arc::clone(&letta_client),
            Arc::clone(&self.db),
        ));
        let agent_coordinator = Arc::new(agent::coordination::AgentCoordinator::new(Arc::clone(
            &multi_agent_system,
        )));

        self.letta_client = Some(letta_client);
        self.agent_manager = Some(agent_coordinator);
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

    /// Get a reference to the agent coordinator if available
    pub fn agent_coordinator(&self) -> Option<&agent::coordination::AgentCoordinator> {
        self.agent_manager.as_ref().map(|m| m.as_ref())
    }

    /// Get a reference to the multi-agent system if available
    pub fn multi_agent_system(&self) -> Option<&agent::MultiAgentSystem> {
        self.multi_agent_system.as_ref().map(|m| m.as_ref())
    }
}

// Re-export commonly used types
pub use agent::builder::MultiAgentSystemBuilder;
pub use agent::constellation::{AgentConfig, MemoryBlockConfig, MultiAgentSystem};
pub use agent::UserId;
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
