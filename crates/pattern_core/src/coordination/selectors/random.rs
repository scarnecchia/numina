//! Random agent selection

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use super::SelectionContext;
use crate::coordination::AgentSelector;
use crate::coordination::groups::AgentWithMembership;
use crate::{Result, agent::Agent};

/// Selects agents randomly
#[derive(Debug, Clone)]
pub struct RandomSelector;

#[async_trait]
impl AgentSelector for RandomSelector {
    async fn select_agents<'a>(
        &'a self,
        agents: &'a [AgentWithMembership<Arc<dyn Agent>>],
        _context: &SelectionContext,
        config: &HashMap<String, String>,
    ) -> Result<Vec<&'a AgentWithMembership<Arc<dyn Agent>>>> {
        let mut rng = rand::rng();

        // Get number of agents to select (default 1)
        let count = config
            .get("count")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1);

        // Filter active agents
        let available: Vec<_> = agents
            .iter()
            .filter(|awm| awm.membership.is_active)
            .collect();

        if available.is_empty() {
            return Ok(vec![]);
        }

        // Randomly select up to 'count' agents
        let selected_count = count.min(available.len());
        // Manually select random indices
        let mut indices: Vec<usize> = (0..available.len()).collect();
        use rand::seq::SliceRandom;
        indices.shuffle(&mut rng);
        let selected: Vec<_> = indices
            .into_iter()
            .take(selected_count)
            .map(|i| available[i])
            .collect();

        Ok(selected)
    }

    fn name(&self) -> &str {
        "random"
    }

    fn description(&self) -> &str {
        "Randomly selects one or more agents from the available pool"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Agent, AgentState, AgentType};
    use crate::coordination::groups::GroupMembership;
    use crate::coordination::types::GroupMemberRole;
    use crate::memory::MemoryPermission;
    use crate::{
        AgentId, UserId,
        message::{ChatRole, Message, MessageContent, MessageMetadata, MessageOptions, Response},
    };
    use crate::{MemoryBlock, tool::DynamicTool};
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
            _access_level: MemoryPermission,
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
    async fn test_random_selector() {
        let selector = RandomSelector;

        // Create mock agents with membership
        let agents: Vec<AgentWithMembership<Arc<dyn Agent>>> = vec![
            AgentWithMembership {
                agent: Arc::new(TestAgent {
                    id: AgentId::generate(),
                    name: "agent1".to_string(),
                }),
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: true,
                    capabilities: vec![],
                },
            },
            AgentWithMembership {
                agent: Arc::new(TestAgent {
                    id: AgentId::generate(),
                    name: "agent2".to_string(),
                }),
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: true,
                    capabilities: vec![],
                },
            },
            AgentWithMembership {
                agent: Arc::new(TestAgent {
                    id: AgentId::generate(),
                    name: "agent3".to_string(),
                }),
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: true,
                    capabilities: vec![],
                },
            },
        ];

        let context = SelectionContext {
            message: create_test_message("Test"),
            recent_selections: vec![],
            available_agents: vec![], // Not used in new implementation
            agent_capabilities: Default::default(),
        };

        // Select default (1 agent)
        let selected = selector
            .select_agents(&agents, &context, &HashMap::new())
            .await
            .unwrap();
        assert_eq!(selected.len(), 1);

        // Select multiple
        let mut config = HashMap::new();
        config.insert("count".to_string(), "2".to_string());
        let selected = selector
            .select_agents(&agents, &context, &config)
            .await
            .unwrap();
        assert_eq!(selected.len(), 2);

        // Request more than available
        config.insert("count".to_string(), "10".to_string());
        let selected = selector
            .select_agents(&agents, &context, &config)
            .await
            .unwrap();
        assert_eq!(selected.len(), 3); // Only 3 available
    }
}
