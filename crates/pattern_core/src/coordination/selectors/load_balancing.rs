//! Load-balancing agent selection

use chrono::{Duration, Utc};
use std::collections::HashMap;

use super::SelectionContext;
use crate::coordination::groups::AgentWithMembership;
use crate::{Result, agent::Agent};

/// Selects agents based on load balancing (least recently used)
#[derive(Debug, Clone)]
pub struct LoadBalancingSelector;

impl LoadBalancingSelector {
    pub async fn select_agents<'a, A, T>(
        &self,
        agents: &'a [AgentWithMembership<T>],
        context: &SelectionContext,
        config: &HashMap<String, String>,
    ) -> Result<Vec<&'a AgentWithMembership<T>>>
    where
        A: Agent,
        T: AsRef<A>,
    {
        // Get window for considering recent selections (default 5 minutes)
        let window_minutes = config
            .get("window_minutes")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(5);

        let cutoff_time = Utc::now() - Duration::minutes(window_minutes);

        // Only consider active agents
        let active_agents: Vec<_> = agents
            .iter()
            .filter(|awm| awm.membership.is_active)
            .collect();

        // Count recent uses per agent
        let mut usage_counts = HashMap::new();

        // Initialize all active agents with 0 count
        for awm in &active_agents {
            let agent_id = awm.agent.as_ref().id();
            usage_counts.insert(agent_id, (0, awm));
        }

        // Count recent selections
        for (timestamp, agent_id) in &context.recent_selections {
            if *timestamp > cutoff_time {
                if let Some((count, _)) = usage_counts.get_mut(agent_id) {
                    *count += 1;
                }
            }
        }

        // Sort agents by usage (least used first)
        let mut sorted_agents: Vec<_> = usage_counts
            .into_iter()
            .map(|(_, (count, awm))| (count, awm))
            .collect();
        sorted_agents.sort_by_key(|(count, _)| *count);

        // Get number of agents to select
        let select_count = config
            .get("count")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1);

        // Select the least used agents
        let selected: Vec<_> = sorted_agents
            .into_iter()
            .take(select_count)
            .map(|(_, awm)| *awm)
            .collect();

        Ok(selected)
    }

    pub fn name(&self) -> &str {
        "load_balancing"
    }

    pub fn description(&self) -> &str {
        "Selects least recently used agents to balance load"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Agent, AgentState, AgentType, MemoryAccessLevel};
    use crate::coordination::groups::GroupMembership;
    use crate::coordination::types::GroupMemberRole;
    use crate::{AgentId, MemoryBlock, tool::DynamicTool};
    use crate::{
        UserId,
        message::{ChatRole, Message, MessageContent, MessageMetadata, MessageOptions, Response},
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use compact_str::CompactString;

    // Simple test agent implementation
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

    #[async_trait]
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
            unimplemented!("Test agent")
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

        async fn list_memory_keys(&self) -> Result<Vec<CompactString>> {
            unimplemented!("Test agent")
        }

        async fn search_memory(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<(CompactString, MemoryBlock, f32)>> {
            unimplemented!("Test agent")
        }

        async fn share_memory_with(
            &self,
            _memory_key: &str,
            _target_agent_id: AgentId,
            _access_level: MemoryAccessLevel,
        ) -> Result<()> {
            unimplemented!("Test agent")
        }

        async fn get_shared_memories(&self) -> Result<Vec<(AgentId, CompactString, MemoryBlock)>> {
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
    async fn test_load_balancing_selector() {
        let selector = LoadBalancingSelector;

        let agent1_id = AgentId::generate();
        let agent2_id = AgentId::generate();
        let agent3_id = AgentId::generate();

        // Create agents
        let agents = vec![
            AgentWithMembership {
                agent: TestAgent {
                    id: agent1_id.clone(),
                    name: "agent1".to_string(),
                },
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: true,
                    capabilities: vec![],
                },
            },
            AgentWithMembership {
                agent: TestAgent {
                    id: agent2_id.clone(),
                    name: "agent2".to_string(),
                },
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: true,
                    capabilities: vec![],
                },
            },
            AgentWithMembership {
                agent: TestAgent {
                    id: agent3_id.clone(),
                    name: "agent3".to_string(),
                },
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: true,
                    capabilities: vec![],
                },
            },
        ];

        // Create recent selections showing agent1 used twice, agent2 once, agent3 never
        let recent_selections = vec![
            (Utc::now() - Duration::minutes(2), agent1_id.clone()),
            (Utc::now() - Duration::minutes(3), agent1_id.clone()),
            (Utc::now() - Duration::minutes(4), agent2_id.clone()),
        ];

        let context = SelectionContext {
            message: create_test_message("Test"),
            recent_selections,
            available_agents: vec![], // Not used in new implementation
            agent_capabilities: Default::default(), // Not used in new implementation
        };

        // Should select agent3 (never used)
        let selected = selector
            .select_agents(&agents, &context, &HashMap::new())
            .await
            .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].agent.id(), agent3_id);

        // Select multiple - should get agent3 and agent2 (least used)
        let mut config = HashMap::new();
        config.insert("count".to_string(), "2".to_string());

        let selected = selector
            .select_agents(&agents, &context, &config)
            .await
            .unwrap();
        assert_eq!(selected.len(), 2);
        let selected_ids: Vec<_> = selected.iter().map(|awm| awm.agent.id()).collect();
        assert!(selected_ids.contains(&agent3_id));
        assert!(selected_ids.contains(&agent2_id));
        assert!(!selected_ids.contains(&agent1_id)); // Most used

        // Test with different time window
        config.insert("window_minutes".to_string(), "1".to_string());

        // Now only selections from last minute count - all agents equal
        let selected = selector
            .select_agents(&agents, &context, &config)
            .await
            .unwrap();
        assert_eq!(selected.len(), 2);
    }
}
