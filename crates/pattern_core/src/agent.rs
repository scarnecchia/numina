use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::str::FromStr;
use std::sync::Arc;

use crate::{AgentId, Memory, Result, tool::DynamicTool};

/// The base trait that all agents must implement
#[async_trait]
pub trait Agent: Send + Sync + Debug {
    /// Get the agent's unique identifier
    fn id(&self) -> AgentId;

    /// Get the agent's name
    fn name(&self) -> &str;

    /// Get the agent's type
    fn agent_type(&self) -> AgentType;

    /// Process an incoming message and generate a response
    async fn process_message(&self, message: Message) -> Result<Response>;

    /// Get a memory block by key
    async fn get_memory(&self, key: &str) -> Result<Option<Memory>>;

    /// Update a memory block
    async fn update_memory(&self, key: &str, memory: Memory) -> Result<()>;

    /// Execute a tool with the given parameters
    async fn execute_tool(
        &self,
        tool_name: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value>;

    /// Get the agent's system prompt
    fn system_prompt(&self) -> &str;

    /// Get the list of tools available to this agent
    fn available_tools(&self) -> Vec<String>;

    /// Get the agent's current state
    fn state(&self) -> AgentState;

    /// Update the agent's state
    async fn set_state(&mut self, state: AgentState) -> Result<()>;
}

/// Types of agents in the system
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentType {
    /// Generic agent without specific personality
    Generic,

    /// ADHD-specific agent types
    #[cfg(feature = "nd")]
    Pattern,
    #[cfg(feature = "nd")]
    Entropy,
    #[cfg(feature = "nd")]
    Flux,
    #[cfg(feature = "nd")]
    Archive,
    #[cfg(feature = "nd")]
    Momentum,
    #[cfg(feature = "nd")]
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
            Self::Custom(name) => serializer.serialize_str(&format!("custom:{}", name)),
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
        if let Some(name) = s.strip_prefix("custom:") {
            Ok(Self::Custom(name.to_string()))
        } else {
            Ok(Self::from_str(&s).unwrap_or_else(|_| Self::Custom(s)))
        }
    }
}

impl AgentType {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// A message to be processed by an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub metadata: MessageMetadata,
    pub attachments: Vec<Attachment>,
}

/// The role of a message sender
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

/// Metadata associated with a message
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MessageMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(flatten)]
    pub custom: serde_json::Value,
}

/// A tool invocation within a message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub parameters: serde_json::Value,
}

/// An attachment to a message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub filename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    pub size_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<u8>>,
}

/// A response generated by an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub metadata: ResponseMetadata,
    pub suggested_actions: Vec<SuggestedAction>,
}

/// Metadata for a response
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResponseMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing_time: Option<chrono::Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_used: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    pub custom: serde_json::Value,
}

/// Token usage statistics
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

/// A suggested follow-up action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedAction {
    pub action_type: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub priority: ActionPriority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ActionPriority {
    Low,
    Medium,
    High,
    Critical,
}

/// Builder for creating new agents
#[derive(Debug, Clone)]
pub struct AgentBuilder {
    id: Option<AgentId>,
    name: Option<String>,
    agent_type: AgentType,
    system_prompt: Option<String>,
    tools: Vec<Box<dyn DynamicTool>>,
    memory_blocks: Vec<(String, Memory)>,
}

impl AgentBuilder {
    pub fn new(agent_type: AgentType) -> Self {
        Self {
            id: None,
            name: None,
            agent_type,
            system_prompt: None,
            tools: Vec::new(),
            memory_blocks: Vec::new(),
        }
    }

    pub fn with_id(mut self, id: AgentId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_tool(mut self, tool: Box<dyn DynamicTool>) -> Self {
        self.tools.push(tool);
        self
    }

    pub fn with_memory(mut self, key: impl Into<String>, memory: Memory) -> Self {
        self.memory_blocks.push((key.into(), memory));
        self
    }

    pub async fn build(self) -> Result<Arc<dyn Agent>> {
        // This would create the actual agent implementation
        // The agent_type field will be used here to create the appropriate agent type
        let _ = self.agent_type; // TODO: Use this when implementing
        todo!("Implement agent building")
    }
}
