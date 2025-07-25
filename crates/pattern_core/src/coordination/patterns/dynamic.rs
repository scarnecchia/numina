//! Dynamic coordination pattern implementation

use async_trait::async_trait;
use chrono::Utc;
use std::sync::Arc;

use crate::{
    CoreError, Result,
    agent::Agent,
    coordination::{
        groups::{AgentResponse, AgentWithMembership, GroupManager, GroupResponse},
        types::{CoordinationPattern, GroupState, SelectionContext},
    },
    message::Message,
};

pub struct DynamicManager {
    selectors: Arc<dyn crate::coordination::selectors::SelectorRegistry>,
}

impl DynamicManager {
    pub fn new(selectors: Arc<dyn crate::coordination::selectors::SelectorRegistry>) -> Self {
        Self { selectors }
    }
}

#[async_trait]
impl GroupManager for DynamicManager {
    async fn route_message(
        &self,
        group: &crate::coordination::groups::AgentGroup,
        agents: &[AgentWithMembership<Arc<dyn Agent>>],
        message: Message,
    ) -> Result<GroupResponse> {
        let start_time = std::time::Instant::now();

        // Extract dynamic config
        let (selector_name, selector_config) = match &group.coordination_pattern {
            CoordinationPattern::Dynamic {
                selector_name,
                selector_config,
            } => (selector_name, selector_config),
            _ => {
                return Err(CoreError::AgentGroupError {
                    group_name: group.name.clone(),
                    operation: "route_message".to_string(),
                    cause: "Invalid pattern for DynamicManager".to_string(),
                });
            }
        };

        // Get recent selections from state
        let recent_selections = match &group.state {
            GroupState::Dynamic { recent_selections } => recent_selections.clone(),
            _ => Vec::new(),
        };

        // Get the selector
        let selector =
            self.selectors
                .get(selector_name)
                .ok_or_else(|| CoreError::AgentGroupError {
                    group_name: group.name.clone(),
                    operation: "get_selector".to_string(),
                    cause: format!("Selector '{}' not found", selector_name),
                })?;

        // Build selection context
        let available_agents = agents
            .iter()
            .filter(|awm| awm.membership.is_active)
            .map(|awm| (awm.agent.as_ref().id(), crate::AgentState::Ready))
            .collect();

        let agent_capabilities = agents
            .iter()
            .filter(|awm| awm.membership.is_active)
            .map(|awm| (awm.agent.as_ref().id(), awm.membership.capabilities.clone()))
            .collect();

        let context = SelectionContext {
            message: message.clone(),
            recent_selections: recent_selections.clone(),
            available_agents,
            agent_capabilities,
        };

        // Use the actual selector to select agents
        let selected_agents = selector
            .select_agents(agents, &context, selector_config)
            .await?;

        if selected_agents.is_empty() {
            return Err(CoreError::AgentGroupError {
                group_name: group.name.clone(),
                operation: "select_agents".to_string(),
                cause: "No agents selected by dynamic selector".to_string(),
            });
        }

        // Create responses for selected agents
        let mut responses = Vec::new();
        for awm in selected_agents.iter() {
            let agent_response = awm.agent.as_ref().process_message(message.clone()).await?;
            responses.push(AgentResponse {
                agent_id: awm.agent.as_ref().id(),
                response: agent_response,
                responded_at: Utc::now(),
            });
        }

        // Update recent selections
        let mut new_recent_selections = recent_selections;
        for awm in selected_agents {
            new_recent_selections.push((Utc::now(), awm.agent.as_ref().id()));
        }

        // Keep only recent selections (last 100 or from last hour)
        let one_hour_ago = Utc::now() - chrono::Duration::hours(1);
        new_recent_selections.retain(|(timestamp, _)| *timestamp > one_hour_ago);
        if new_recent_selections.len() > 100 {
            new_recent_selections = new_recent_selections
                .into_iter()
                .rev()
                .take(100)
                .rev()
                .collect();
        }

        let new_state = GroupState::Dynamic {
            recent_selections: new_recent_selections,
        };

        Ok(GroupResponse {
            group_id: group.id.clone(),
            pattern: format!("dynamic:{}", selector_name),
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
        // State is already updated in route_message for dynamic
        Ok(response.state_changes.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AgentId, MemoryBlock, UserId,
        agent::{Agent, AgentState, AgentType},
        coordination::{
            AgentGroup, AgentSelector,
            groups::{AgentWithMembership, GroupMembership},
            selectors::SelectorRegistry,
            types::GroupMemberRole,
        },
        id::GroupId,
        message::{ChatRole, Message, MessageContent, MessageMetadata, MessageOptions, Response},
    };
    use chrono::Utc;
    use std::collections::HashMap;

    // Mock selector registry for testing
    struct MockSelectorRegistry {
        selectors: HashMap<String, Arc<dyn AgentSelector>>,
    }

    impl MockSelectorRegistry {
        fn new() -> Self {
            let mut registry = Self {
                selectors: HashMap::new(),
            };
            // Add a default random selector for testing
            registry.register(
                "random".to_string(),
                Arc::new(crate::coordination::selectors::RandomSelector),
            );
            registry
        }
    }

    impl crate::coordination::selectors::SelectorRegistry for MockSelectorRegistry {
        fn get(&self, name: &str) -> Option<Arc<dyn AgentSelector>> {
            self.selectors.get(name).map(|s| s.clone())
        }

        fn register(&mut self, name: String, selector: Arc<dyn AgentSelector>) {
            self.selectors.insert(name, selector);
        }

        fn list(&self) -> Vec<String> {
            self.selectors.keys().map(|s| s.clone()).collect()
        }
    }

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
                content: vec![MessageContent::Text("Test response".to_string())],
                reasoning: None,
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
            _access_level: crate::memory::MemoryPermission,
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

        fn available_tools(&self) -> Vec<Box<dyn crate::tool::DynamicTool>> {
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
    async fn test_dynamic_with_random_selector() {
        let registry = Arc::new(MockSelectorRegistry::new());
        let manager = DynamicManager::new(registry);

        let agents: Vec<AgentWithMembership<Arc<dyn Agent>>> = vec![
            AgentWithMembership {
                agent: Arc::new(create_test_agent("Agent1")),
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: true,
                    capabilities: vec!["general".to_string()],
                },
            },
            AgentWithMembership {
                agent: Arc::new(create_test_agent("Agent2")),
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: true,
                    capabilities: vec!["technical".to_string()],
                },
            },
            AgentWithMembership {
                agent: Arc::new(create_test_agent("Agent3")),
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: false, // Inactive - should not be selected
                    capabilities: vec!["creative".to_string()],
                },
            },
        ];

        let group = AgentGroup {
            id: GroupId::generate(),
            name: "TestGroup".to_string(),
            description: "Test dynamic group".to_string(),
            coordination_pattern: CoordinationPattern::Dynamic {
                selector_name: "random".to_string(),
                selector_config: HashMap::new(),
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
            is_active: true,
            state: GroupState::Dynamic {
                recent_selections: vec![],
            },
            members: vec![], // Empty members for test
        };

        let message = create_test_message("Test message");

        let response = manager
            .route_message(&group, &agents, message)
            .await
            .unwrap();

        // Should have selected at least one agent
        assert!(!response.responses.is_empty());

        // Selected agent should be active (not Agent3)
        let selected_id = &response.responses[0].agent_id;
        assert!(selected_id != &agents[2].agent.id());

        // State should be updated with recent selection
        if let Some(GroupState::Dynamic { recent_selections }) = response.state_changes {
            assert_eq!(recent_selections.len(), response.responses.len());
        } else {
            panic!("Expected Dynamic state");
        }
    }
}
