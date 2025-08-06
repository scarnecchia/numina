use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio_stream::StreamExt;

use crate::{
    Result,
    agent::Agent,
    coordination::groups::{AgentGroup, AgentWithMembership, GroupManager, GroupResponseEvent},
    message::Message,
};

use super::{MessageEndpoint, MessageOrigin};

/// Endpoint for routing messages through agent groups
pub struct GroupEndpoint {
    pub group: AgentGroup,
    pub agents: Vec<AgentWithMembership<Arc<dyn Agent>>>,
    pub manager: Arc<dyn GroupManager>,
}

#[async_trait]
impl MessageEndpoint for GroupEndpoint {
    async fn send(
        &self,
        message: Message,
        _metadata: Option<Value>,
        _origin: Option<&MessageOrigin>,
    ) -> Result<()> {
        let mut stream = self
            .manager
            .route_message(&self.group, &self.agents, message)
            .await?;

        // Process to completion, logging key events
        while let Some(event) = stream.next().await {
            match event {
                GroupResponseEvent::Error {
                    message, agent_id, ..
                } => {
                    tracing::error!(
                        "Group {} routing error from {:?}: {}",
                        self.group.name,
                        agent_id,
                        message
                    );
                }
                GroupResponseEvent::Complete {
                    agent_responses, ..
                } => {
                    tracing::info!(
                        "Group {} processed message, {} agents responded",
                        self.group.name,
                        agent_responses.len()
                    );
                }
                _ => {} // Other events handled silently for now
            }
        }

        Ok(())
    }

    fn endpoint_type(&self) -> &'static str {
        "group"
    }
}
