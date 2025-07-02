//! Pattern - A library for managing Letta agents with task and calendar functionality
//!
//! This crate provides the core functionality for:
//! - Managing Letta AI agents
//! - Task management with ADHD-aware time estimation
//! - Calendar scheduling
//! - Activity monitoring

pub mod db;

#[cfg(feature = "mcp")]
pub mod mcp_server;

use miette::Result;
use std::sync::Arc;

/// Core Pattern service that manages agents, tasks, and scheduling
#[derive(Debug, Clone)]
pub struct PatternService {
    db: Arc<db::Database>,
    letta_client: Option<Arc<letta::LettaClient>>,
}

impl PatternService {
    /// Create a new Pattern service with database connection
    pub async fn new(db_path: &str) -> Result<Self> {
        let db = db::Database::new(db_path).await?;
        db.migrate().await?;

        Ok(Self {
            db: Arc::new(db),
            letta_client: None,
        })
    }

    /// Create service with existing database connection
    pub fn with_database(db: db::Database) -> Self {
        Self {
            db: Arc::new(db),
            letta_client: None,
        }
    }

    /// Set the Letta client for agent operations
    pub fn with_letta_client(mut self, client: letta::LettaClient) -> Self {
        self.letta_client = Some(Arc::new(client));
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
}

// Re-export commonly used types
pub use db::{Agent, Database, Event, Task, User};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_service_creation() {
        let service = PatternService::new(":memory:").await.unwrap();
        assert!(service.letta_client().is_none());
    }
}
