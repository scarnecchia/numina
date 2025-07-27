use pattern_core::{
    Result,
    context::message_router::MessageEndpoint,
    message::{ContentPart, Message, MessageContent},
};
use serde_json::Value;

use crate::output::Output;

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
    async fn send(&self, message: Message, _metadata: Option<Value>) -> Result<()> {
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
            _ => "",
        };

        // Use Output to format the message nicely
        // Since this is coming from send_message tool, show it as agent output
        self.output.status("📤 Sending message to user:");
        self.output.agent_message("Pattern", text);

        Ok(())
    }

    fn endpoint_type(&self) -> &'static str {
        "cli"
    }
}
