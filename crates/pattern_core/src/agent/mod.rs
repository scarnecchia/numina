//! Agent framework for Pattern
//!
//! This module provides the core agent abstraction and implementations.

mod entity;
mod impls;
#[cfg(test)]
mod tests;
pub mod tool_rules;

use compact_str::CompactString;
pub use entity::{AgentMemoryRelation, AgentRecord, get_next_message_position};
pub use impls::{AgentDbExt, DatabaseAgent};
pub use tool_rules::{
    ExecutionPhase, ToolExecution, ToolExecutionState, ToolRule, ToolRuleEngine, ToolRuleType,
    ToolRuleViolation,
};

use async_trait::async_trait;
use chrono::Utc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::str::FromStr;
use std::sync::Arc;
use tokio_stream::Stream;

use crate::{
    AgentId, MemoryBlock, Result,
    message::{Message, MessageContent, Response, ToolCall, ToolResponse},
    tool::DynamicTool,
};

/// Events emitted during message processing for real-time streaming
#[derive(Debug, Clone)]
pub enum ResponseEvent {
    /// Tool execution is starting
    ToolCallStarted {
        call_id: String,
        fn_name: String,
        args: serde_json::Value,
    },
    /// Tool execution completed (success or error)
    ToolCallCompleted {
        call_id: String,
        result: std::result::Result<String, String>,
    },
    /// Partial text chunk from the LLM response
    TextChunk {
        text: String,
        /// Whether this is a final chunk for this text block
        is_final: bool,
    },
    /// Partial reasoning/thinking content from the model
    ReasoningChunk {
        text: String,
        /// Whether this is a final chunk for this reasoning block
        is_final: bool,
    },
    /// Tool calls the agent is about to make
    ToolCalls { calls: Vec<ToolCall> },
    /// Tool responses received
    ToolResponses { responses: Vec<ToolResponse> },
    /// Processing complete with final metadata
    Complete {
        /// The ID of the incoming message that triggered this response
        message_id: crate::MessageId,
        /// Metadata about the complete response (usage, timing, etc)
        metadata: crate::message::ResponseMetadata,
    },
    /// An error occurred during processing
    Error { message: String, recoverable: bool },
}

/// The base trait that all agents must implement
#[async_trait]
pub trait Agent: Send + Sync + Debug {
    /// Get the agent's unique identifier
    fn id(&self) -> AgentId;

    /// Get the agent's name
    fn name(&self) -> String;

    /// Get the agent's type
    fn agent_type(&self) -> AgentType;

    /// Get the agent's handle for controlled access to internals
    async fn handle(&self) -> crate::context::state::AgentHandle;

    /// Get the agent's last active timestamp
    async fn last_active(&self) -> Option<chrono::DateTime<chrono::Utc>>;

    /// Process an incoming message and generate a response
    async fn process_message(self: Arc<Self>, message: Message) -> Result<Response>;

    /// Process a message and stream responses as they happen
    ///
    /// Default implementation collects all events and returns the final response.
    /// Implementations should override this to provide real streaming.
    async fn process_message_stream(
        self: Arc<Self>,
        message: Message,
    ) -> Result<Box<dyn Stream<Item = ResponseEvent> + Send + Unpin>>
    where
        Self: 'static,
    {
        use tokio_stream::wrappers::ReceiverStream;

        let (tx, rx) = tokio::sync::mpsc::channel(100);

        // Clone for the spawned task
        let self_clone = self.clone();
        let message_id = message.id.clone();

        tokio::spawn(async move {
            match self_clone.process_message(message).await {
                Ok(response) => {
                    // Emit any text content
                    for content in &response.content {
                        match content {
                            MessageContent::Text(text) => {
                                let _ = tx
                                    .send(ResponseEvent::TextChunk {
                                        text: text.clone(),
                                        is_final: true,
                                    })
                                    .await;
                            }
                            MessageContent::ToolCalls(calls) => {
                                let _ = tx
                                    .send(ResponseEvent::ToolCalls {
                                        calls: calls.clone(),
                                    })
                                    .await;
                            }
                            MessageContent::ToolResponses(responses) => {
                                let _ = tx
                                    .send(ResponseEvent::ToolResponses {
                                        responses: responses.clone(),
                                    })
                                    .await;
                            }
                            _ => {}
                        }
                    }

                    // Emit reasoning if present
                    if let Some(reasoning) = &response.reasoning {
                        let _ = tx
                            .send(ResponseEvent::ReasoningChunk {
                                text: reasoning.clone(),
                                is_final: true,
                            })
                            .await;
                    }

                    // Send completion
                    let _ = tx
                        .send(ResponseEvent::Complete {
                            message_id,
                            metadata: response.metadata,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(ResponseEvent::Error {
                            message: e.to_string(),
                            recoverable: false,
                        })
                        .await;
                }
            }
        });

        Ok(Box::new(ReceiverStream::new(rx)))
    }

    /// Get a memory block by key
    async fn get_memory(&self, key: &str) -> Result<Option<MemoryBlock>>;

    /// Update a memory block
    async fn update_memory(&self, key: &str, memory: MemoryBlock) -> Result<()>;

    /// Execute a tool with the given parameters
    async fn execute_tool(
        &self,
        tool_name: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value>;

    /// Create or update a memory block with just the value
    async fn set_memory(&self, key: &str, value: String) -> Result<()> {
        self.update_memory(key, MemoryBlock::new(key, value)).await
    }

    /// List all available memory block keys
    async fn list_memory_keys(&self) -> Result<Vec<CompactString>>;

    /// Share a memory block with another agent
    async fn share_memory_with(
        &self,
        memory_key: &str,
        target_agent_id: AgentId,
        access_level: crate::memory::MemoryPermission,
    ) -> Result<()>;

    /// Get all memory blocks shared with this agent
    async fn get_shared_memories(&self) -> Result<Vec<(AgentId, CompactString, MemoryBlock)>>;

    /// Get the agent's system prompt components
    async fn system_prompt(&self) -> Vec<String>;

    /// Get the list of tools available to this agent
    async fn available_tools(&self) -> Vec<Box<dyn DynamicTool>>;

    /// Get the agent's current state and a watch receiver for changes
    async fn state(&self) -> (AgentState, Option<tokio::sync::watch::Receiver<AgentState>>);

    /// Update the agent's state
    async fn set_state(&self, state: AgentState) -> Result<()>;

    /// Register a message endpoint for a specific channel
    async fn register_endpoint(
        &self,
        name: String,
        endpoint: Arc<dyn crate::context::message_router::MessageEndpoint>,
    ) -> Result<()>;

    /// Set the default user endpoint
    async fn set_default_user_endpoint(
        &self,
        endpoint: Arc<dyn crate::context::message_router::MessageEndpoint>,
    ) -> Result<()>;
}

/// Types of agents in the system
#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
pub enum AgentType {
    /// Generic agent without specific personality
    Generic,

    /// ADHD-specific agent types
    #[cfg(feature = "nd")]
    /// Orchestrator agent - coordinates other agents and runs background checks
    Pattern,
    #[cfg(feature = "nd")]
    /// Task specialist - breaks down overwhelming tasks into atomic units
    Entropy,
    #[cfg(feature = "nd")]
    /// Time translator - converts between ADHD time and clock time
    Flux,
    #[cfg(feature = "nd")]
    /// Memory bank - external memory for context recovery and pattern finding
    Archive,
    #[cfg(feature = "nd")]
    /// Energy tracker - monitors energy patterns and protects flow states
    Momentum,
    #[cfg(feature = "nd")]
    /// Habit manager - manages routines and basic needs without nagging
    Anchor,

    /// Custom agent type
    Custom(String),
}

impl Serialize for AgentType {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Generic => serializer.serialize_str("generic"),
            #[cfg(feature = "nd")]
            Self::Pattern => serializer.serialize_str("pattern"),
            #[cfg(feature = "nd")]
            Self::Entropy => serializer.serialize_str("entropy"),
            #[cfg(feature = "nd")]
            Self::Flux => serializer.serialize_str("flux"),
            #[cfg(feature = "nd")]
            Self::Archive => serializer.serialize_str("archive"),
            #[cfg(feature = "nd")]
            Self::Momentum => serializer.serialize_str("momentum"),
            #[cfg(feature = "nd")]
            Self::Anchor => serializer.serialize_str("anchor"),
            Self::Custom(name) => serializer.serialize_str(&format!("custom_{}", name)),
        }
    }
}

impl<'de> Deserialize<'de> for AgentType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        // Check if it starts with custom: prefix
        if let Some(name) = s.strip_prefix("custom_") {
            Ok(Self::Custom(name.to_string()))
        } else {
            Ok(Self::from_str(&s).unwrap_or_else(|_| Self::Custom(s)))
        }
    }
}

impl AgentType {
    /// Convert the agent type to its string representation
    ///
    /// For custom agents, returns the raw name without any prefix
    pub fn as_str(&self) -> &str {
        match self {
            Self::Generic => "generic",
            #[cfg(feature = "nd")]
            Self::Pattern => "pattern",
            #[cfg(feature = "nd")]
            Self::Entropy => "entropy",
            #[cfg(feature = "nd")]
            Self::Flux => "flux",
            #[cfg(feature = "nd")]
            Self::Archive => "archive",
            #[cfg(feature = "nd")]
            Self::Momentum => "momentum",
            #[cfg(feature = "nd")]
            Self::Anchor => "anchor",
            Self::Custom(name) => name, // Note: this returns the raw name without prefix
        }
    }
}

impl FromStr for AgentType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "generic" => Ok(Self::Generic),
            #[cfg(feature = "nd")]
            "pattern" => Ok(Self::Pattern),
            #[cfg(feature = "nd")]
            "entropy" => Ok(Self::Entropy),
            #[cfg(feature = "nd")]
            "flux" => Ok(Self::Flux),
            #[cfg(feature = "nd")]
            "archive" => Ok(Self::Archive),
            #[cfg(feature = "nd")]
            "momentum" => Ok(Self::Momentum),
            #[cfg(feature = "nd")]
            "anchor" => Ok(Self::Anchor),
            // Check for custom: prefix
            other if other.starts_with("custom:") => Ok(Self::Custom(
                other.strip_prefix("custom:").unwrap().to_string(),
            )),
            // For backward compatibility, also accept without prefix
            other => Ok(Self::Custom(other.to_string())),
        }
    }
}

/// The current state of an agent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    /// Agent is ready to process messages
    Ready,

    /// Agent is currently processing a message
    Processing,

    /// Agent is in a cooldown period
    Cooldown { until: chrono::DateTime<Utc> },

    /// Agent is suspended
    Suspended,

    /// Agent has encountered an error
    Error,
}

impl Default for AgentState {
    fn default() -> Self {
        Self::Ready
    }
}

impl FromStr for AgentState {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "ready" => Ok(Self::Ready),
            "processing" => Ok(Self::Processing),
            "suspended" => Ok(Self::Suspended),
            "error" => Ok(Self::Error),
            other => {
                // Try to parse as cooldown with timestamp
                if other.starts_with("cooldown:") {
                    let timestamp_str = &other[9..];
                    chrono::DateTime::parse_from_rfc3339(timestamp_str)
                        .map(|dt| Self::Cooldown {
                            until: dt.with_timezone(&Utc),
                        })
                        .map_err(|e| format!("Invalid cooldown timestamp: {}", e))
                } else {
                    Err(format!("Unknown agent state: {}", other))
                }
            }
        }
    }
}

/// Priority levels for agent actions and tasks
///
/// Used to determine the urgency and ordering of agent actions.
/// The variants are ordered from lowest to highest priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ActionPriority {
    /// Low priority - can be deferred or batched
    Low,
    /// Medium priority - normal operations
    Medium,
    /// High priority - should be handled soon
    High,
    /// Critical priority - requires immediate attention
    Critical,
}
