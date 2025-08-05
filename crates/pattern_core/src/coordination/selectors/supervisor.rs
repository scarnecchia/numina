//! Supervisor-based agent selection
//!
//! Uses a supervisor agent to decide which agents should handle a message.
//!
//! Behavior varies by role:
//! - Supervisor: Can select any agent including itself
//! - Specialist (routing): Cannot select itself, only routes to other agents
//! - Specialist (other): Can select any agent including itself

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use super::SelectionContext;
use crate::coordination::AgentSelector;
use crate::coordination::groups::AgentWithMembership;
use crate::coordination::types::GroupMemberRole;
use crate::{
    CoreError, Result,
    agent::Agent,
    message::{ChatRole, Message, MessageContent},
};

/// Selects agents by asking a supervisor to decide
#[derive(Debug, Clone)]
pub struct SupervisorSelector;

#[async_trait]
impl AgentSelector for SupervisorSelector {
    async fn select_agents<'a>(
        &'a self,
        agents: &'a [AgentWithMembership<Arc<dyn Agent>>],
        context: &SelectionContext,
        config: &HashMap<String, String>,
    ) -> Result<super::SelectionResult<'a>> {
        // Find supervisor agents or specified specialist
        let specialist_domain = config.get("specialist_domain");

        let supervisors: Vec<_> = agents
            .iter()
            .filter(|awm| {
                awm.membership.is_active
                    && match &awm.membership.role {
                        GroupMemberRole::Specialist { domain } => {
                            specialist_domain.map_or(false, |d| d == domain)
                        }
                        GroupMemberRole::Supervisor => true,

                        GroupMemberRole::Regular => false,
                    }
            })
            .collect();

        if supervisors.is_empty() {
            return Err(CoreError::CoordinationFailed {
                group: "unknown".to_string(),
                pattern: "supervisor".to_string(),
                participating_agents: agents.iter().map(|a| a.agent.name()).collect(),
                cause: "No supervisor or matching specialist found in group".to_string(),
            });
        }

        // Pick first available supervisor (could be enhanced with load balancing)
        let supervisor = supervisors[0];

        // Build prompt for supervisor
        let prompt = build_selection_prompt(&context.message, agents, config);

        // Create a message for the supervisor
        let supervisor_message = Message {
            id: crate::MessageId::generate(),
            role: ChatRole::User,
            owner_id: None,
            content: MessageContent::from_text(prompt),
            metadata: Default::default(),
            options: Default::default(),
            has_tool_calls: false,
            word_count: 0,
            created_at: chrono::Utc::now(),
            embedding: None,
            embedding_model: None,
        };

        // Ask supervisor to decide using streaming
        let mut response_stream = supervisor
            .agent
            .clone()
            .process_message_stream(supervisor_message)
            .await?;

        // Buffer initial events to determine if this is agent selection or direct response
        use tokio_stream::StreamExt;
        let mut buffered_events = Vec::new();
        let mut response_text = String::new();
        let mut _has_tool_calls = false;
        let mut is_complete = false;

        // Collect up to 10 events or until we see a Complete event
        while buffered_events.len() < 10 && !is_complete {
            if let Some(event) = response_stream.next().await {
                match &event {
                    crate::agent::ResponseEvent::TextChunk { text, .. } => {
                        response_text.push_str(text);
                    }
                    crate::agent::ResponseEvent::ToolCallStarted { .. } => {
                        _has_tool_calls = true;
                    }
                    crate::agent::ResponseEvent::Complete { .. } => {
                        is_complete = true;
                    }
                    _ => {}
                }
                buffered_events.push(event);
            } else {
                break;
            }
        }

        // Parse supervisor's response to get selected agent names
        let selected_names = parse_supervisor_response(&response_text);

        // Check if the response looks like agent names or a direct response
        let is_direct_response = if selected_names.is_empty() {
            // No valid agent names found, check if it's a substantive response
            response_text.len() > 50 || response_text.contains('.') || response_text.contains('?')
        } else {
            // Check if all "names" are actually just single words (likely agent names)
            let all_single_words = selected_names.iter().all(|name| !name.contains(' '));
            let all_match_agents = selected_names
                .iter()
                .all(|name| agents.iter().any(|a| a.agent.name() == name.as_str()));
            !all_single_words || !all_match_agents
        };

        // Check if this selector can select itself
        let can_select_self = match &supervisor.membership.role {
            GroupMemberRole::Specialist { domain } if domain == "routing" => false,
            _ => true,
        };

        // If supervisor provided a direct response and can select self, return self
        if is_direct_response && can_select_self {
            // Create a stream that includes the buffered events plus the rest
            use tokio_stream::wrappers::ReceiverStream;
            let (tx, rx) = tokio::sync::mpsc::channel(100);

            // Spawn task to forward buffered events and remaining stream
            tokio::spawn(async move {
                // Send buffered events first
                for event in buffered_events {
                    let _ = tx.send(event).await;
                }

                // Forward remaining events
                while let Some(event) = response_stream.next().await {
                    let _ = tx.send(event).await;
                }
            });

            // Return supervisor as selected with their response stream
            return Ok(super::SelectionResult {
                agents: vec![supervisor],
                selector_response: Some(Box::new(ReceiverStream::new(rx))),
            });
        }

        // Find the selected agents
        let mut selected = Vec::new();
        for name in selected_names {
            if let Some(awm) = agents.iter().find(|a| a.agent.name() == name) {
                // Skip self-selection for routing specialists
                if !can_select_self && awm.agent.id() == supervisor.agent.id() {
                    continue;
                }
                selected.push(awm);
            }
        }

        // If supervisor didn't select anyone valid, handle fallback
        if selected.is_empty() {
            if can_select_self {
                // Supervisors and non-routing specialists can fall back to themselves
                selected.push(supervisor);
            } else {
                // Routing specialists broadcast to all other agents
                for awm in agents {
                    if awm.membership.is_active && awm.agent.id() != supervisor.agent.id() {
                        selected.push(awm);
                    }
                }
            }
        }

        Ok(super::SelectionResult {
            agents: selected,
            selector_response: None,
        })
    }

    fn name(&self) -> &str {
        "supervisor"
    }

    fn description(&self) -> &str {
        "Supervisor agent decides which agents should handle the message"
    }
}

fn build_selection_prompt(
    message: &Message,
    agents: &[AgentWithMembership<Arc<dyn Agent>>],
    config: &HashMap<String, String>,
) -> String {
    // Extract text content from the message
    let message_text = match &message.content {
        MessageContent::Text(text) => text.clone(),
        MessageContent::Parts(parts) => parts
            .iter()
            .filter_map(|p| match p {
                crate::message::ContentPart::Text(text) => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => "[non-text content]".to_string(),
    };

    let mut prompt = format!(
        "You are coordinating agent selection. A message needs to be handled:\n\n\
        Message: {}\n\n\
        Available agents:\n",
        message_text
    );

    for awm in agents {
        if awm.membership.is_active {
            prompt.push_str(&format!(
                "- {} (capabilities: {})\n",
                awm.agent.name(),
                awm.membership.capabilities.join(", ")
            ));
        }
    }

    prompt.push_str(
        "\nBased on the message content and agent capabilities, \
                     which agent(s) should handle this message?\n\n",
    );

    // Note: The supervisor/specialist's own name may be included in the available agents list.
    // The dynamic manager will handle self-selection appropriately.

    if let Some(max_agents) = config.get("max_agents") {
        prompt.push_str(&format!("Select up to {} agents.\n", max_agents));
    }

    prompt.push_str(
        "Respond with ONLY the agent names, one per line. \
                     If multiple agents should collaborate, list all of them.",
    );

    prompt
}

fn parse_supervisor_response(response: &str) -> Vec<String> {
    response
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                // Remove any bullet points or numbering
                let name = trimmed
                    .trim_start_matches(|c: char| {
                        c.is_numeric() || c == '.' || c == '-' || c == '*'
                    })
                    .trim();
                if !name.is_empty() {
                    Some(name.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_supervisor_response() {
        let response = "Entropy\nFlux\n";
        let names = parse_supervisor_response(response);
        assert_eq!(names, vec!["Entropy", "Flux"]);

        let response_with_bullets = "- Pattern\n* Archive\n1. Anchor\n";
        let names = parse_supervisor_response(response_with_bullets);
        assert_eq!(names, vec!["Pattern", "Archive", "Anchor"]);

        let response_with_comments = "# These agents should handle it:\nMomentum\n# Done\n";
        let names = parse_supervisor_response(response_with_comments);
        assert_eq!(names, vec!["Momentum"]);
    }
}
