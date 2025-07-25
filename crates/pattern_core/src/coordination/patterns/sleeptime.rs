//! Sleeptime coordination pattern implementation

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use std::{sync::Arc, time::Duration};

use crate::{
    CoreError, Result,
    agent::Agent,
    coordination::{
        groups::{AgentResponse, AgentWithMembership, GroupManager, GroupResponse},
        types::{
            CoordinationPattern, GroupState, SleeptimeTrigger, TriggerCondition, TriggerEvent,
            TriggerPriority,
        },
        utils::text_response,
    },
    message::{ChatRole, Message},
};

pub struct SleeptimeManager;

#[async_trait]
impl GroupManager for SleeptimeManager {
    async fn route_message(
        &self,
        group: &crate::coordination::groups::AgentGroup,
        agents: &[AgentWithMembership<Arc<dyn Agent>>],
        message: Message,
    ) -> Result<GroupResponse> {
        let start_time = std::time::Instant::now();

        // Extract sleeptime config
        let (check_interval, triggers, intervention_agent_id) = match &group.coordination_pattern {
            CoordinationPattern::Sleeptime {
                check_interval,
                triggers,
                intervention_agent_id,
            } => (check_interval, triggers, intervention_agent_id),
            _ => {
                return Err(CoreError::AgentGroupError {
                    group_name: group.name.clone(),
                    operation: "route_message".to_string(),
                    cause: "Invalid pattern for SleeptimeManager".to_string(),
                });
            }
        };

        // Get current state
        let (last_check, mut trigger_history) = match &group.state {
            GroupState::Sleeptime {
                last_check,
                trigger_history,
            } => (*last_check, trigger_history.clone()),
            _ => (Utc::now() - ChronoDuration::hours(1), Vec::new()),
        };

        // Check if it's time to run checks
        let time_since_last_check = Utc::now() - last_check;
        let should_check = time_since_last_check
            > ChronoDuration::from_std(*check_interval).unwrap_or(ChronoDuration::minutes(20));

        let mut responses = Vec::new();

        if should_check {
            // Evaluate all triggers
            let mut fired_triggers = Vec::new();

            for trigger in triggers {
                if self
                    .evaluate_trigger(trigger, &message, &trigger_history)
                    .await?
                {
                    fired_triggers.push(trigger);
                }
            }

            // Sort by priority (highest first)
            fired_triggers.sort_by(|a, b| b.priority.cmp(&a.priority));

            if !fired_triggers.is_empty() {
                // Find intervention agent
                let intervention_agent = agents
                    .iter()
                    .find(|awm| &awm.agent.as_ref().id() == intervention_agent_id)
                    .ok_or_else(|| CoreError::agent_not_found(intervention_agent_id.to_string()))?;

                // Create intervention response - process the message with context about triggers
                let trigger_names: Vec<_> =
                    fired_triggers.iter().map(|t| t.name.as_str()).collect();
                let mut intervention_context = format!(
                    "[Sleeptime Intervention] Triggers fired: {}. {}",
                    trigger_names.join(", "),
                    self.get_intervention_message(&fired_triggers)
                );

                let text = message
                    .content
                    .text()
                    .map(String::from)
                    .unwrap_or(intervention_context.clone());
                intervention_context.push_str(&text);

                // A bit dirty, we need more elegant helper methods here.
                let intervention_message = match message.role {
                    ChatRole::System => Message::system(intervention_context),
                    ChatRole::User => Message::user(intervention_context),
                    ChatRole::Assistant => Message::agent(intervention_context),
                    ChatRole::Tool => Message::system(intervention_context),
                };
                let agent_response = intervention_agent
                    .agent
                    .as_ref()
                    .process_message(intervention_message)
                    .await?;

                responses.push(AgentResponse {
                    agent_id: intervention_agent.agent.as_ref().id(),
                    response: agent_response,
                    responded_at: Utc::now(),
                });

                // Record trigger events
                for trigger in fired_triggers {
                    trigger_history.push(TriggerEvent {
                        trigger_name: trigger.name.clone(),
                        timestamp: Utc::now(),
                        intervention_activated: true,
                        metadata: Default::default(),
                    });
                }
            } else {
                // No triggers fired, just log the check
                responses.push(AgentResponse {
                    agent_id: intervention_agent_id.clone(),
                    response: text_response(
                        "[Sleeptime] Routine check complete. All systems nominal.",
                    ),
                    responded_at: Utc::now(),
                });
            }

            // Keep trigger history to reasonable size (last 1000 events)
            if trigger_history.len() > 1000 {
                trigger_history = trigger_history.into_iter().rev().take(1000).rev().collect();
            }
        } else {
            // Not time to check yet, pass through message
            // In real implementation, this might route to a default handler
            responses.push(AgentResponse {
                agent_id: agents
                    .first()
                    .map(|awm| awm.agent.as_ref().id())
                    .unwrap_or_else(|| intervention_agent_id.clone()),
                response: text_response(format!(
                    "[Sleeptime] Next check in: {}",
                    self.format_duration(
                        *check_interval - time_since_last_check.to_std().unwrap_or_default()
                    )
                )),
                responded_at: Utc::now(),
            });
        }

        // Update state
        let new_state = GroupState::Sleeptime {
            last_check: if should_check { Utc::now() } else { last_check },
            trigger_history,
        };

        Ok(GroupResponse {
            group_id: group.id.clone(),
            pattern: "sleeptime".to_string(),
            responses,
            execution_time: start_time.elapsed(),
            state_changes: Some(new_state),
        })
    }

    async fn update_state(
        &self,
        _current_state: &GroupState,
        response: &GroupResponse,
    ) -> Result<Option<GroupState>> {
        // State is already updated in route_message for sleeptime
        Ok(response.state_changes.clone())
    }
}

impl SleeptimeManager {
    async fn evaluate_trigger(
        &self,
        trigger: &SleeptimeTrigger,
        _message: &Message,
        history: &[TriggerEvent],
    ) -> Result<bool> {
        match &trigger.condition {
            TriggerCondition::TimeElapsed { duration } => {
                // Check if enough time has passed since last trigger
                let last_fired = history
                    .iter()
                    .filter(|e| e.trigger_name == trigger.name && e.intervention_activated)
                    .max_by_key(|e| e.timestamp);

                if let Some(last) = last_fired {
                    let elapsed = Utc::now() - last.timestamp;
                    Ok(elapsed > ChronoDuration::from_std(*duration).unwrap_or_default())
                } else {
                    // Never fired before, so it's elapsed
                    Ok(true)
                }
            }
            TriggerCondition::PatternDetected { pattern_name } => {
                // In real implementation, would check for specific patterns
                // For now, simulate pattern detection
                Ok(pattern_name.contains("hyperfocus") && rand::random::<f32>() > 0.7)
            }
            TriggerCondition::ThresholdExceeded { metric, threshold } => {
                // In real implementation, would check actual metrics
                // For now, simulate threshold check
                Ok(metric.contains("sedentary") && *threshold < 60.0)
            }
            TriggerCondition::Custom { evaluator } => {
                // Would call custom evaluator function
                Ok(evaluator.contains("custom") && rand::random::<f32>() > 0.8)
            }
        }
    }

    fn get_intervention_message(&self, triggers: &[&SleeptimeTrigger]) -> &'static str {
        // Determine intervention based on highest priority trigger
        if let Some(trigger) = triggers.first() {
            match trigger.priority {
                TriggerPriority::Critical => {
                    "CRITICAL: Immediate intervention required. Please take a break NOW."
                }
                TriggerPriority::High => {
                    "Important: It's time for a break. Your wellbeing depends on it."
                }
                TriggerPriority::Medium => "Reminder: Consider taking a short break soon.",
                TriggerPriority::Low => {
                    "Gentle nudge: A break might be beneficial when convenient."
                }
            }
        } else {
            "Routine check complete."
        }
    }

    fn format_duration(&self, duration: Duration) -> String {
        let total_secs = duration.as_secs();
        let hours = total_secs / 3600;
        let minutes = (total_secs % 3600) / 60;
        let seconds = total_secs % 60;

        if hours > 0 {
            format!("{}h {}m", hours, minutes)
        } else if minutes > 0 {
            format!("{}m {}s", minutes, seconds)
        } else {
            format!("{}s", seconds)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        AgentId, MemoryBlock, UserId,
        agent::{Agent, AgentState, AgentType},
        coordination::{
            AgentGroup,
            groups::{AgentWithMembership, GroupMembership},
            types::GroupMemberRole,
        },
        id::GroupId,
        memory::MemoryPermission,
        message::{ChatRole, Message, MessageContent, MessageMetadata, MessageOptions, Response},
        tool::DynamicTool,
    };

    // Test agent implementation
    #[derive(Debug)]
    struct TestAgent {
        id: AgentId,
        name: String,
    }

    impl AsRef<TestAgent> for TestAgent {
        fn as_ref(&self) -> &TestAgent {
            self
        }
    }

    #[async_trait::async_trait]
    impl Agent for TestAgent {
        fn id(&self) -> AgentId {
            self.id.clone()
        }
        fn name(&self) -> String {
            self.name.to_string()
        }
        fn agent_type(&self) -> AgentType {
            AgentType::Generic
        }

        async fn process_message(&self, _message: Message) -> Result<Response> {
            use crate::message::ResponseMetadata;
            Ok(Response {
                content: vec![MessageContent::Text("Sleeptime test response".to_string())],
                reasoning: None,
                tool_calls: vec![],
                metadata: ResponseMetadata::default(),
            })
        }

        async fn get_memory(&self, _key: &str) -> Result<Option<MemoryBlock>> {
            unimplemented!("Test agent")
        }

        async fn update_memory(&self, _key: &str, _memory: MemoryBlock) -> Result<()> {
            unimplemented!("Test agent")
        }

        async fn execute_tool(
            &self,
            _tool_name: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value> {
            unimplemented!("Test agent")
        }

        async fn list_memory_keys(&self) -> Result<Vec<compact_str::CompactString>> {
            unimplemented!("Test agent")
        }

        async fn search_memory(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<(compact_str::CompactString, MemoryBlock, f32)>> {
            unimplemented!("Test agent")
        }

        async fn share_memory_with(
            &self,
            _memory_key: &str,
            _target_agent_id: AgentId,
            _access_level: MemoryPermission,
        ) -> Result<()> {
            unimplemented!("Test agent")
        }

        async fn get_shared_memories(
            &self,
        ) -> Result<Vec<(AgentId, compact_str::CompactString, MemoryBlock)>> {
            unimplemented!("Test agent")
        }

        async fn system_prompt(&self) -> Vec<String> {
            vec![]
        }

        fn available_tools(&self) -> Vec<Box<dyn DynamicTool>> {
            vec![]
        }

        fn state(&self) -> AgentState {
            AgentState::Ready
        }

        async fn set_state(&self, _state: AgentState) -> Result<()> {
            unimplemented!("Test agent")
        }
    }

    fn create_test_agent(name: &str) -> TestAgent {
        TestAgent {
            id: AgentId::generate(),
            name: name.to_string(),
        }
    }

    fn create_test_message(content: &str) -> Message {
        Message {
            id: crate::id::MessageId::generate(),
            role: ChatRole::User,
            owner_id: Some(UserId::generate()),
            content: MessageContent::Text(content.to_string()),
            metadata: MessageMetadata::default(),
            options: MessageOptions::default(),
            has_tool_calls: false,
            word_count: content.split_whitespace().count() as u32,
            created_at: Utc::now(),
            embedding: None,
            embedding_model: None,
        }
    }

    #[tokio::test]
    async fn test_sleeptime_trigger_check() {
        let manager = SleeptimeManager;
        let intervention_agent = create_test_agent("Pattern");
        let intervention_id = intervention_agent.id.clone();

        let agents: Vec<AgentWithMembership<Arc<dyn Agent>>> = vec![AgentWithMembership {
            agent: Arc::new(intervention_agent),
            membership: GroupMembership {
                joined_at: Utc::now(),
                role: GroupMemberRole::Supervisor,
                is_active: true,
                capabilities: vec!["intervention".to_string()],
            },
        }];

        let triggers = vec![
            SleeptimeTrigger {
                name: "hyperfocus_check".to_string(),
                condition: TriggerCondition::TimeElapsed {
                    duration: Duration::from_secs(1), // 1 second for testing
                },
                priority: TriggerPriority::High,
            },
            SleeptimeTrigger {
                name: "hydration_reminder".to_string(),
                condition: TriggerCondition::ThresholdExceeded {
                    metric: "minutes_since_water".to_string(),
                    threshold: 45.0,
                },
                priority: TriggerPriority::Medium,
            },
        ];

        let group = AgentGroup {
            id: GroupId::generate(),
            name: "SleeptimeGroup".to_string(),
            description: "Background monitoring group".to_string(),
            coordination_pattern: CoordinationPattern::Sleeptime {
                check_interval: Duration::from_secs(1), // 1 second for testing
                triggers,
                intervention_agent_id: intervention_id,
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
            is_active: true,
            state: GroupState::Sleeptime {
                last_check: Utc::now() - ChronoDuration::hours(1), // Force check
                trigger_history: vec![],
            },
            members: vec![], // Empty for test
        };

        let message = create_test_message("Working on code");

        let response = manager
            .route_message(&group, &agents, message)
            .await
            .unwrap();

        // Should have at least one response
        assert!(!response.responses.is_empty());

        // Response should be from intervention agent
        assert_eq!(response.responses[0].agent_id, intervention_id);

        // State should be updated with new last_check time
        if let Some(GroupState::Sleeptime { last_check, .. }) = response.state_changes {
            assert!(last_check > group.created_at);
        } else {
            panic!("Expected Sleeptime state");
        }
    }
}
