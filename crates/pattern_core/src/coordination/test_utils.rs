//! Shared test utilities for coordination pattern tests

#[cfg(test)]
pub(crate) mod test {
    use std::sync::Arc;

    use chrono::Utc;

    use crate::{
        AgentId, MemoryBlock, Result, UserId,
        agent::{Agent, AgentState, AgentType},
        coordination::groups::GroupResponseEvent,
        memory::MemoryPermission,
        message::{ChatRole, Message, MessageContent, MessageMetadata, MessageOptions, Response},
        tool::DynamicTool,
    };

    /// Test agent implementation for coordination pattern tests
    #[derive(Debug)]
    pub struct TestAgent {
        pub id: AgentId,
        pub name: String,
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
                content: vec![MessageContent::Text(format!("{} test response", self.name))],
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

        async fn set_default_user_endpoint(
            &self,
            _endpoint: Arc<dyn crate::context::message_router::MessageEndpoint>,
        ) -> Result<()> {
            unimplemented!("Test agent")
        }
    }

    /// Create a test agent with the given name
    pub fn create_test_agent(name: &str) -> TestAgent {
        TestAgent {
            id: AgentId::generate(),
            name: name.to_string(),
        }
    }

    /// Create a test message with the given content
    pub fn create_test_message(content: &str) -> Message {
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

    /// Helper to collect events from stream and extract Complete event
    pub async fn collect_complete_event(
        mut stream: Box<dyn futures::Stream<Item = GroupResponseEvent> + Send + Unpin>,
    ) -> (
        Vec<crate::coordination::groups::AgentResponse>,
        Option<crate::coordination::types::GroupState>,
    ) {
        use tokio_stream::StreamExt;
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event);
        }

        events
            .iter()
            .find_map(|event| {
                if let GroupResponseEvent::Complete {
                    agent_responses,
                    state_changes,
                    ..
                } = event
                {
                    Some((agent_responses.clone(), state_changes.clone()))
                } else {
                    None
                }
            })
            .expect("Should have Complete event")
    }

    /// Helper to collect all agent responses from a streaming round-robin implementation
    pub async fn collect_agent_responses(
        mut stream: Box<dyn futures::Stream<Item = GroupResponseEvent> + Send + Unpin>,
    ) -> Vec<crate::coordination::groups::AgentResponse> {
        use crate::message::ResponseMetadata;
        use tokio_stream::StreamExt;

        let mut responses = Vec::new();
        let mut current_agent_id = None;
        let mut text_chunks = Vec::new();

        while let Some(event) = stream.next().await {
            match event {
                GroupResponseEvent::AgentStarted { agent_id, .. } => {
                    current_agent_id = Some(agent_id);
                    text_chunks.clear();
                }
                GroupResponseEvent::TextChunk { text, .. } => {
                    text_chunks.push(text);
                }
                GroupResponseEvent::AgentCompleted { agent_id, .. } => {
                    if let Some(ref current_id) = current_agent_id {
                        if *current_id == agent_id {
                            // Create a response from the collected chunks
                            let content = if text_chunks.is_empty() {
                                vec![MessageContent::Text("Test response".to_string())]
                            } else {
                                vec![MessageContent::Text(text_chunks.join(""))]
                            };

                            responses.push(crate::coordination::groups::AgentResponse {
                                agent_id,
                                response: Response {
                                    content,
                                    reasoning: None,
                                    metadata: ResponseMetadata::default(),
                                },
                                responded_at: Utc::now(),
                            });

                            current_agent_id = None;
                            text_chunks.clear();
                        }
                    }
                }
                _ => {}
            }
        }

        responses
    }
}
