use async_trait::async_trait;
use pattern_core::{
    Result,
    agent::Agent,
    context::message_router::{MessageEndpoint, MessageOrigin},
    coordination::groups::{AgentGroup, AgentWithMembership, GroupManager},
    message::{ContentBlock, ContentPart, Message, MessageContent},
};
use serde_json::Value;
use std::sync::Arc;
use tokio_stream::StreamExt;

use crate::{agent_ops::print_group_response_event, output::Output};

/// CLI endpoint that formats messages using Output
pub struct CliEndpoint {
    output: Output,
}

impl CliEndpoint {
    pub fn new(output: Output) -> Self {
        Self { output }
    }
}

#[async_trait::async_trait]
impl MessageEndpoint for CliEndpoint {
    async fn send(
        &self,
        message: Message,
        _metadata: Option<Value>,
        origin: Option<&MessageOrigin>,
    ) -> Result<Option<String>> {
        // Extract text content from the message
        let text = match &message.content {
            MessageContent::Text(text) => text.as_str(),
            MessageContent::Parts(parts) => {
                // Find first text part
                parts
                    .iter()
                    .find_map(|part| match part {
                        ContentPart::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .unwrap_or("")
            }
            MessageContent::Blocks(blocks) => {
                // Extract text from blocks, skipping thinking blocks
                blocks
                    .iter()
                    .find_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .unwrap_or("")
            }
            _ => "",
        };

        // Use Output to format the message nicely
        // Format based on origin and extract sender name
        let sender_name = if let Some(origin) = origin {
            self.output
                .status(&format!("📤 Message from {}", origin.description()));
            
            // Extract the agent name from the origin if it's an agent
            match origin {
                MessageOrigin::Agent { name, .. } => name.clone(),
                _ => "Pattern".to_string(),
            }
        } else {
            self.output.status("📤 Sending message to user:");
            "Pattern".to_string()
        };
        
        // Add a tiny delay to let reasoning chunks finish printing
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        
        self.output.agent_message(&sender_name, text);

        Ok(None)
    }

    fn endpoint_type(&self) -> &'static str {
        "cli"
    }
}

/// CLI endpoint for routing messages through agent groups with nice formatting
pub struct GroupCliEndpoint {
    pub group: AgentGroup,
    pub agents: Vec<AgentWithMembership<Arc<dyn Agent>>>,
    pub manager: Arc<dyn GroupManager>,
    pub output: Output,
}

#[async_trait]
impl MessageEndpoint for GroupCliEndpoint {
    async fn send(
        &self,
        mut message: Message,
        metadata: Option<Value>,
        origin: Option<&MessageOrigin>,
    ) -> Result<Option<String>> {
        // Show origin info if provided
        if let Some(origin) = origin {
            self.output.info("Message from:", &origin.description());
            self.output.list_item(message.content.text().unwrap_or("")); // temporarily to see formatting
        }

        // Merge any provided metadata into the message
        if let Some(meta) = metadata {
            if let Some(obj) = meta.as_object() {
                // Merge with existing custom metadata
                if let Some(existing_obj) = message.metadata.custom.as_object_mut() {
                    for (key, value) in obj {
                        existing_obj.insert(key.clone(), value.clone());
                    }
                } else {
                    message.metadata.custom = meta;
                }
            }
        }

        let mut stream = self
            .manager
            .route_message(&self.group, &self.agents, message)
            .await?;

        // Show which source this is from at the beginning
        self.output.section("[Jetstream] Processing incoming data");
        
        while let Some(event) = stream.next().await {
            print_group_response_event(
                event,
                &self.output,
                &self.agents,
                Some("Jetstream")
            ).await;
        }

        Ok(None)
    }

    fn endpoint_type(&self) -> &'static str {
        "group"
    }
}
