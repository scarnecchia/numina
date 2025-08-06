use async_trait::async_trait;
use owo_colors::OwoColorize;
use pattern_core::{
    Result,
    agent::Agent,
    context::message_router::{MessageEndpoint, MessageOrigin},
    coordination::groups::{AgentGroup, AgentWithMembership, GroupManager, GroupResponseEvent},
    message::{ContentBlock, ContentPart, Message, MessageContent},
};
use serde_json::Value;
use std::sync::Arc;
use tokio_stream::StreamExt;

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
    async fn send(
        &self,
        message: Message,
        _metadata: Option<Value>,
        origin: Option<&MessageOrigin>,
    ) -> Result<()> {
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
        // Format based on origin
        if let Some(origin) = origin {
            self.output
                .status(&format!("📤 Message from {}", origin.description()));
        } else {
            self.output.status("📤 Sending message to user:");
        }
        self.output.agent_message("Pattern", text);

        Ok(())
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
        message: Message,
        _metadata: Option<Value>,
        origin: Option<&MessageOrigin>,
    ) -> Result<()> {
        let mut stream = self
            .manager
            .route_message(&self.group, &self.agents, message)
            .await?;

        // Show origin info if provided
        if let Some(origin) = origin {
            self.output.info("Message from:", &origin.description());
        }

        while let Some(event) = stream.next().await {
            match event {
                GroupResponseEvent::Started {
                    pattern,
                    agent_count,
                    ..
                } => {
                    self.output
                        .info("Pattern:", &pattern.bright_cyan().to_string());
                    self.output
                        .info("Agents:", &format!("{} responding", agent_count));
                }
                GroupResponseEvent::AgentStarted {
                    agent_name, role, ..
                } => {
                    self.output
                        .info(&format!("{} ({:?})", agent_name, role), "starting response");
                }
                GroupResponseEvent::TextChunk {
                    agent_id: _,
                    text,
                    is_final: _,
                } => {
                    // Stream the text directly (output handles formatting)
                    print!("{}", text);
                    use std::io::{self, Write};
                    let _ = io::stdout().flush();
                }
                GroupResponseEvent::ReasoningChunk {
                    agent_id: _,
                    text,
                    is_final: _,
                } => {
                    // Show thinking in dimmed style
                    print!("{}", text.dimmed());
                    use std::io::{self, Write};
                    let _ = io::stdout().flush();
                }
                GroupResponseEvent::ToolCallStarted {
                    agent_id: _,
                    fn_name,
                    args,
                    ..
                } => {
                    let args_str =
                        serde_json::to_string_pretty(&args).unwrap_or_else(|_| args.to_string());
                    self.output.tool_call(&fn_name, &args_str);
                }
                GroupResponseEvent::ToolCallCompleted {
                    agent_id: _,
                    call_id: _,
                    result,
                } => match result {
                    Ok(result) => self.output.tool_result(&result),
                    Err(error) => self.output.error(&format!("Tool error: {}", error)),
                },
                GroupResponseEvent::AgentCompleted {
                    agent_name,
                    message_id: _,
                    ..
                } => {
                    self.output
                        .info("Completed:", &agent_name.bright_green().to_string());
                }
                GroupResponseEvent::Error {
                    agent_id: _,
                    message,
                    recoverable,
                } => {
                    if recoverable {
                        self.output.warning(&message);
                    } else {
                        self.output.error(&message);
                    }
                }
                GroupResponseEvent::Complete {
                    agent_responses,
                    execution_time,
                    ..
                } => {
                    println!(); // Ensure we're on a new line
                    self.output.info(
                        "Group complete:",
                        &format!(
                            "{} agents responded in {:.2}s",
                            agent_responses.len(),
                            execution_time.as_secs_f64()
                        ),
                    );
                }
                _ => {} // Other events we don't need to display
            }
        }

        Ok(())
    }

    fn endpoint_type(&self) -> &'static str {
        "group_cli"
    }
}
