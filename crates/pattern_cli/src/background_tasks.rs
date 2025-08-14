//! Background tasks for Pattern agents
//!
//! This module provides background monitoring and periodic activation
//! for agent groups, particularly those using the sleeptime coordination pattern.

use std::sync::Arc;
use miette::Result;
use pattern_core::{
    agent::Agent,
    coordination::{
        groups::{AgentGroup, AgentWithMembership, GroupManager},
        types::CoordinationPattern,
    },
    message::{Message, MessageContent},
};
use crate::{chat::print_group_response_event, output::Output};

/// Start a background monitoring task for a sleeptime group
///
/// This spawns a task that periodically sends trigger messages to the group
/// to check if any sleeptime triggers should fire.
pub async fn start_context_sync_monitoring<M: GroupManager + Clone + 'static>(
    group: AgentGroup,
    agents: Vec<AgentWithMembership<Arc<dyn Agent>>>,
    manager: M,
    output: Output,
) -> Result<tokio::task::JoinHandle<()>> {
    // Extract check interval from the group's coordination pattern
    let check_interval = match &group.coordination_pattern {
        CoordinationPattern::Sleeptime {
            check_interval,
            ..
        } => *check_interval,
        _ => {
            return Err(miette::miette!(
                "Context sync monitoring requires a sleeptime coordination pattern"
            ));
        }
    };

    let group_name = group.name.clone();
    let mut group = group; // Make group mutable so we can update its state

    // Spawn the background monitoring task
    let handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(check_interval);

        // Skip the first tick so we don't immediately fire on startup
        interval.tick().await;

        output.info(
            "Background monitoring started",
            &format!(
                "Context sync for group '{}' checking every {:?}",
                group_name, check_interval
            ),
        );

        loop {
            interval.tick().await;

            // Create a generic trigger check message
            // The sleeptime manager will customize it for the specific agent being activated
            let trigger_message = Message::user(MessageContent::from_text(
                "Context sync check: Review your domain and report any notable patterns or concerns. Provide brief status updates only if intervention is needed."
            ));

            // Route the message through the group manager
            match manager.route_message(&group, &agents, trigger_message).await {
                Ok(mut stream) => {
                    // Process the stream and capture state updates
                    let agents_clone = agents.clone();
                    let output_clone = output.clone();
                    let group_name_clone = group_name.clone();

                    use futures::StreamExt;
                    use pattern_core::coordination::groups::GroupResponseEvent;

                    // Show which group this is from at the start
                    output_clone.section(&format!("[Background] {}", group_name_clone));

                    // Process the response stream
                    while let Some(event) = stream.next().await {
                        // Check for state updates in Complete event
                        if let GroupResponseEvent::Complete { state_changes, .. } = &event {
                            if let Some(new_state) = state_changes {
                                // Update the group's state for next iteration
                                group.state = new_state.clone();
                                tracing::debug!("Updated group state for next iteration: {:?}", new_state);
                            }
                        }

                        print_group_response_event(
                            event,
                            &output_clone,
                            &agents_clone,
                            Some("Background")
                        ).await;
                    }
                }
                Err(e) => {
                    output.error(&format!("Failed to route context sync message: {}", e));
                }
            }
        }
    });

    Ok(handle)
}
