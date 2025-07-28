//! Round-robin coordination pattern implementation

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use crate::{
    CoreError, Result,
    agent::Agent,
    coordination::{
        groups::{AgentResponse, AgentWithMembership, GroupManager, GroupResponse},
        types::{CoordinationPattern, GroupState},
    },
    message::Message,
};

pub struct RoundRobinManager;

#[async_trait]
impl GroupManager for RoundRobinManager {
    async fn route_message(
        &self,
        group: &crate::coordination::groups::AgentGroup,
        agents: &[AgentWithMembership<Arc<dyn Agent>>],
        message: Message,
    ) -> Result<GroupResponse> {
        let start_time = std::time::Instant::now();

        // Extract round-robin config
        let (mut current_index, skip_unavailable) = match &group.coordination_pattern {
            CoordinationPattern::RoundRobin {
                current_index,
                skip_unavailable,
            } => (*current_index, *skip_unavailable),
            _ => {
                return Err(CoreError::AgentGroupError {
                    group_name: group.name.clone(),
                    operation: "route_message".to_string(),
                    cause: "Invalid pattern for RoundRobinManager".to_string(),
                });
            }
        };

        // Get active agents if skip_unavailable is true
        let available_agents: Vec<_> = if skip_unavailable {
            agents
                .iter()
                .enumerate()
                .filter(|(_, awm)| awm.membership.is_active)
                .collect()
        } else {
            agents.iter().enumerate().collect()
        };

        if available_agents.is_empty() {
            return Err(CoreError::AgentGroupError {
                group_name: group.name.clone(),
                operation: "route_message".to_string(),
                cause: "No available agents in group".to_string(),
            });
        }

        // Ensure current_index is within bounds of available agents
        current_index = current_index % available_agents.len();

        // Get the agent at the current index
        let (_original_index, awm) = &available_agents[current_index];

        // Route to selected agent
        let agent_response = awm.agent.clone().process_message(message.clone()).await?;
        let response = AgentResponse {
            agent_id: awm.agent.as_ref().id(),
            response: agent_response,
            responded_at: Utc::now(),
        };

        // Calculate next index
        let next_index = if skip_unavailable {
            // Move to next available agent index
            (current_index + 1) % available_agents.len()
        } else {
            // Simple increment in full agent array
            (current_index + 1) % agents.len()
        };

        // Update state
        let new_state = GroupState::RoundRobin {
            current_index: next_index,
            last_rotation: Utc::now(),
        };

        Ok(GroupResponse {
            group_id: group.id.clone(),
            pattern: "round_robin".to_string(),
            responses: vec![response],
            execution_time: start_time.elapsed(),
            state_changes: Some(new_state),
        })
    }

    async fn update_state(
        &self,
        _current_state: &GroupState,
        response: &GroupResponse,
    ) -> Result<Option<GroupState>> {
        // State is already updated in route_message for round-robin
        Ok(response.state_changes.clone())
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
    use chrono::Utc;

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

        async fn process_message(self: Arc<Self>, _message: Message) -> Result<Response> {
            use crate::message::ResponseMetadata;
            Ok(Response {
                content: vec![MessageContent::Text(
                    "Round robin test response".to_string(),
                )],
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

        async fn available_tools(&self) -> Vec<Box<dyn DynamicTool>> {
            vec![]
        }

        async fn state(&self) -> AgentState {
            AgentState::Ready
        }

        async fn set_state(&self, _state: AgentState) -> Result<()> {
            unimplemented!("Test agent")
        }
        async fn register_endpoint(
            &self,
            _name: String,
            _endpoint: Arc<dyn crate::context::message_router::MessageEndpoint>,
        ) -> Result<()> {
            unimplemented!("Test agent")
        }

        /// Set the default user endpoint
        async fn set_default_user_endpoint(
            &self,
            _endpoint: Arc<dyn crate::context::message_router::MessageEndpoint>,
        ) -> Result<()> {
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
    async fn test_round_robin_basic() {
        let manager = RoundRobinManager;

        let agents: Vec<AgentWithMembership<Arc<dyn Agent>>> = vec![
            AgentWithMembership {
                agent: Arc::new(create_test_agent("Agent1")),
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: true,
                    capabilities: vec![],
                },
            },
            AgentWithMembership {
                agent: Arc::new(create_test_agent("Agent2")),
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: true,
                    capabilities: vec![],
                },
            },
            AgentWithMembership {
                agent: Arc::new(create_test_agent("Agent3")),
                membership: GroupMembership {
                    joined_at: Utc::now(),
                    role: GroupMemberRole::Regular,
                    is_active: true,
                    capabilities: vec![],
                },
            },
        ];

        let group = AgentGroup {
            id: GroupId::generate(),
            name: "TestGroup".to_string(),
            description: "Test round-robin group".to_string(),
            coordination_pattern: CoordinationPattern::RoundRobin {
                current_index: 0,
                skip_unavailable: false,
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
            is_active: true,
            state: GroupState::RoundRobin {
                current_index: 0,
                last_rotation: Utc::now(),
            },
            members: vec![], // Empty for test
        };

        let message = create_test_message("Test message");

        // First call should route to agent 0
        let response = manager
            .route_message(&group, &agents, message.clone())
            .await
            .unwrap();
        assert_eq!(response.responses.len(), 1);
        assert_eq!(response.responses[0].agent_id, agents[0].agent.id());

        // Check state was updated
        if let Some(GroupState::RoundRobin { current_index, .. }) = response.state_changes {
            assert_eq!(current_index, 1);
        } else {
            panic!("Expected RoundRobin state");
        }
    }

    #[tokio::test]
    async fn test_round_robin_skip_inactive() {
        let manager = RoundRobinManager;

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
            description: "Test round-robin with skip".to_string(),
            coordination_pattern: CoordinationPattern::RoundRobin {
                current_index: 0,
                skip_unavailable: true,
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
            is_active: true,
            state: GroupState::RoundRobin {
                current_index: 0,
                last_rotation: Utc::now(),
            },
            members: vec![], // Empty for test
        };

        let message = create_test_message("Test message");

        // Should route to agent1 first
        let response1 = manager
            .route_message(&group, &agents, message.clone())
            .await
            .unwrap();
        assert_eq!(response1.responses[0].agent_id, agents[0].agent.id());

        // Update group state for next call
        let mut group2 = group.clone();
        if let Some(new_state) = response1.state_changes {
            group2.state = new_state.clone();
            if let CoordinationPattern::RoundRobin { current_index, .. } =
                &mut group2.coordination_pattern
            {
                if let GroupState::RoundRobin {
                    current_index: state_idx,
                    ..
                } = &new_state
                {
                    *current_index = *state_idx;
                }
            }
        }

        // Next call should go to agent2 (skipping inactive agent3)
        let response2 = manager
            .route_message(&group2, &agents, message)
            .await
            .unwrap();
        assert_eq!(response2.responses[0].agent_id, agents[1].agent.id());
    }
}
