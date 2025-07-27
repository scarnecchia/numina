use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use surrealdb::Surreal;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::db::client;
use crate::error::Result;
use crate::id::{AgentId, GroupId, UserId};
use crate::message::Message;
use crate::message_queue::QueuedMessage;
use crate::tool::builtin::{MessageTarget, TargetType};

/// Trait for message delivery endpoints
#[async_trait::async_trait]
pub trait MessageEndpoint: Send + Sync {
    /// Send a message to this endpoint
    async fn send(&self, message: Message, metadata: Option<Value>) -> Result<()>;

    /// Get the endpoint type name
    fn endpoint_type(&self) -> &'static str;
}

/// Routes messages from agents to their destinations
#[derive(Clone)]
pub struct AgentMessageRouter<C: surrealdb::Connection = surrealdb::engine::any::Any> {
    /// The agent this router belongs to
    agent_id: AgentId,

    /// Database connection for queuing messages
    db: Surreal<C>,

    /// Map of endpoint types to their implementations
    endpoints: Arc<RwLock<HashMap<String, Arc<dyn MessageEndpoint>>>>,

    /// Default endpoint for user messages
    default_user_endpoint: Arc<RwLock<Option<Arc<dyn MessageEndpoint>>>>,
}

impl<C: surrealdb::Connection> AgentMessageRouter<C> {
    /// Create a new message router for an agent
    pub fn new(agent_id: AgentId, db: Surreal<C>) -> Self {
        Self {
            agent_id,
            db,
            endpoints: Arc::new(RwLock::new(HashMap::new())),
            default_user_endpoint: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the default endpoint for user messages (builder pattern)
    pub fn with_default_user_endpoint(self, endpoint: Arc<dyn MessageEndpoint>) -> Self {
        // Can't modify self here since we moved it, so we'll lock and set
        *self.default_user_endpoint.blocking_write() = Some(endpoint);
        self
    }

    /// Set the default user endpoint at runtime
    pub async fn set_default_user_endpoint(&self, endpoint: Arc<dyn MessageEndpoint>) {
        let mut default_endpoint = self.default_user_endpoint.write().await;
        *default_endpoint = Some(endpoint);
    }

    /// Register an endpoint
    pub async fn register_endpoint(&self, name: String, endpoint: Arc<dyn MessageEndpoint>) {
        let mut endpoints = self.endpoints.write().await;
        endpoints.insert(name, endpoint);
    }

    /// Send a message to the specified target
    pub async fn send_message(
        &self,
        target: MessageTarget,
        content: String,
        metadata: Option<Value>,
    ) -> Result<()> {
        match target.target_type {
            TargetType::User => {
                let user_id = target
                    .target_id
                    .as_ref()
                    .and_then(|id| id.parse::<UserId>().ok())
                    .unwrap_or_else(UserId::nil);
                self.send_to_user(user_id, content, metadata).await
            }
            TargetType::Agent => {
                let agent_id = target
                    .target_id
                    .as_ref()
                    .and_then(|id| id.parse::<AgentId>().ok())
                    .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                        tool_name: "send_message".to_string(),
                        cause: "Agent ID required for agent target".to_string(),
                        parameters: serde_json::json!({ "target": target }),
                    })?;
                self.send_to_agent(agent_id, content, metadata).await
            }
            TargetType::Group => {
                let group_id = target
                    .target_id
                    .as_ref()
                    .and_then(|id| id.parse::<GroupId>().ok())
                    .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                        tool_name: "send_message".to_string(),
                        cause: "Group ID required for group target".to_string(),
                        parameters: serde_json::json!({ "target": target }),
                    })?;
                self.send_to_group(group_id, content, metadata).await
            }
            TargetType::Channel => {
                let channel_info = metadata
                    .clone()
                    .unwrap_or_else(|| Value::Object(Default::default()));
                self.send_to_channel(channel_info, content, metadata).await
            }
        }
    }

    /// Send a message to a user
    async fn send_to_user(
        &self,
        user_id: UserId,
        content: String,
        metadata: Option<Value>,
    ) -> Result<()> {
        debug!(
            "Routing message from agent {} to user {}",
            self.agent_id, user_id
        );

        // If we have a default user endpoint, use it
        let default_endpoint = self.default_user_endpoint.read().await;
        if let Some(endpoint) = default_endpoint.as_ref() {
            let message = Message::agent(content);
            endpoint.send(message, metadata).await?;
        } else {
            // Queue the message for later delivery
            let queued = QueuedMessage::agent_to_agent(
                self.agent_id.clone(),
                // TODO: We need to look up the user's primary agent or notification agent
                // For now, just log it
                AgentId::nil(),
                content,
                metadata,
            );

            warn!(
                "No user endpoint configured, would queue message: {:?}",
                queued
            );
        }

        Ok(())
    }

    /// Send a message to another agent
    async fn send_to_agent(
        &self,
        target_agent_id: AgentId,
        content: String,
        metadata: Option<Value>,
    ) -> Result<()> {
        debug!(
            "Routing message from agent {} to agent {}",
            self.agent_id, target_agent_id
        );

        // Create the queued message
        let queued = QueuedMessage::agent_to_agent(
            self.agent_id.clone(),
            target_agent_id.clone(),
            content,
            metadata,
        );

        // Check for loops - if this agent is already in the call chain, prevent sending
        if queued.is_in_call_chain(&target_agent_id) {
            warn!(
                "Preventing message loop: agent {} is already in call chain {:?}",
                target_agent_id, queued.call_chain
            );
            return Ok(());
        }

        // Store the message in the database
        self.store_queued_message(queued).await?;

        Ok(())
    }

    /// Send a message to a group
    async fn send_to_group(
        &self,
        group_id: GroupId,
        _content: String,
        _metadata: Option<Value>,
    ) -> Result<()> {
        debug!(
            "Routing message from agent {} to group {}",
            self.agent_id, group_id
        );

        // TODO: Look up group members and send to each based on group pattern
        warn!("Group messaging not yet implemented");

        Ok(())
    }

    /// Send a message to a channel (Discord, etc)
    async fn send_to_channel(
        &self,
        channel_info: Value,
        content: String,
        metadata: Option<Value>,
    ) -> Result<()> {
        debug!("Routing message from agent {} to channel", self.agent_id);

        // Extract channel type from the info
        let channel_type = channel_info
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        // Look for appropriate endpoint
        let endpoints = self.endpoints.read().await;
        if let Some(endpoint) = endpoints.get(channel_type) {
            let message = Message::agent(content);
            endpoint.send(message, metadata).await?;
        } else {
            warn!("No endpoint registered for channel type: {}", channel_type);
        }

        Ok(())
    }

    /// Store a queued message in the database
    async fn store_queued_message(&self, message: QueuedMessage) -> Result<()> {
        info!(
            "Storing queued message from {} to {}",
            message.from_agent.as_ref().unwrap_or(&AgentId::nil()),
            message.to_agent
        );

        // Store the message
        message.store_with_relations(&self.db).await?;

        Ok(())
    }
}

impl<C: surrealdb::Connection> std::fmt::Debug for AgentMessageRouter<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentMessageRouter")
            .field("agent_id", &self.agent_id)
            .field(
                "has_default_endpoint",
                &self.default_user_endpoint.blocking_read().is_some(),
            )
            .finish()
    }
}

// CLI endpoint moved to pattern_cli crate for better separation of concerns

/// An endpoint that queues messages to the database
pub struct QueueEndpoint {
    pub db: Surreal<surrealdb::engine::any::Any>,
    pub from_agent: AgentId,
}

impl QueueEndpoint {
    pub fn new(db: Surreal<surrealdb::engine::any::Any>, from_agent: AgentId) -> Self {
        Self { db, from_agent }
    }
}

#[async_trait::async_trait]
impl MessageEndpoint for QueueEndpoint {
    async fn send(&self, _message: Message, _metadata: Option<Value>) -> Result<()> {
        // For now, we'll need to extract the target from metadata
        // This is a fallback endpoint
        warn!("QueueEndpoint used - this should be replaced with proper routing");
        Ok(())
    }

    fn endpoint_type(&self) -> &'static str {
        "queue"
    }
}

/// Create a message router from the global database connection
pub async fn create_router_with_global_db(agent_id: AgentId) -> Result<AgentMessageRouter> {
    // Clone the global DB instance
    let db = client::DB.clone();
    Ok(AgentMessageRouter::new(agent_id, db))
}
