use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Agent identifier type with proper validation
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(String);

impl AgentId {
    pub fn new(id: impl Into<String>) -> Result<Self, ValidationError> {
        let id = id.into();

        // Validate agent ID format
        if id.is_empty() {
            return Err(ValidationError::EmptyAgentId);
        }

        if !id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(ValidationError::InvalidAgentId(id));
        }

        Ok(Self(id.to_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for AgentId {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// Memory block identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MemoryBlockId(String);

impl MemoryBlockId {
    pub fn new(id: impl Into<String>) -> Result<Self, ValidationError> {
        let id = id.into();

        if id.is_empty() {
            return Err(ValidationError::EmptyMemoryBlockId);
        }

        if !id.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(ValidationError::InvalidMemoryBlockId(id));
        }

        Ok(Self(id.to_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MemoryBlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// MCP transport type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Sse,
    #[serde(rename = "http")]
    Http,
}

impl fmt::Display for McpTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            McpTransport::Stdio => write!(f, "stdio"),
            McpTransport::Sse => write!(f, "sse"),
            McpTransport::Http => write!(f, "http"),
        }
    }
}

impl FromStr for McpTransport {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stdio" => Ok(McpTransport::Stdio),
            "sse" => Ok(McpTransport::Sse),
            "http" => Ok(McpTransport::Http),
            _ => Err(ValidationError::InvalidTransport(s.to_string())),
        }
    }
}

/// Validation errors for types
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Agent ID cannot be empty")]
    EmptyAgentId,

    #[error("Invalid agent ID format: {0}")]
    InvalidAgentId(String),

    #[error("Memory block ID cannot be empty")]
    EmptyMemoryBlockId,

    #[error("Invalid memory block ID format: {0}")]
    InvalidMemoryBlockId(String),

    #[error("Unknown MCP transport: {0}")]
    InvalidTransport(String),
}

/// Standard memory blocks used by all agents
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardMemoryBlock {
    CurrentState,
    ActiveContext,
    BondEvolution,
}

impl StandardMemoryBlock {
    pub fn id(&self) -> MemoryBlockId {
        match self {
            Self::CurrentState => MemoryBlockId("current_state".to_string()),
            Self::ActiveContext => MemoryBlockId("active_context".to_string()),
            Self::BondEvolution => MemoryBlockId("bond_evolution".to_string()),
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::CurrentState => "Real-time energy/attention/mood tracking",
            Self::ActiveContext => "What they're doing NOW, including blockers",
            Self::BondEvolution => "Growing understanding of this human",
        }
    }

    pub fn max_length(&self) -> usize {
        match self {
            Self::CurrentState => 200,
            Self::ActiveContext => 400,
            Self::BondEvolution => 600,
        }
    }

    pub fn default_value(&self) -> &'static str {
        match self {
            Self::CurrentState => "energy: unknown | attention: unknown | mood: unknown",
            Self::ActiveContext => "task: initializing | progress: 0%",
            Self::BondEvolution => "trust: building | formality: professional",
        }
    }
}

impl fmt::Display for StandardMemoryBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id())
    }
}

/// Standard agent IDs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardAgent {
    Pattern,
    Entropy,
    Flux,
    Archive,
    Momentum,
    Anchor,
}

impl StandardAgent {
    pub fn id(&self) -> AgentId {
        match self {
            Self::Pattern => AgentId("pattern".to_string()),
            Self::Entropy => AgentId("entropy".to_string()),
            Self::Flux => AgentId("flux".to_string()),
            Self::Archive => AgentId("archive".to_string()),
            Self::Momentum => AgentId("momentum".to_string()),
            Self::Anchor => AgentId("anchor".to_string()),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Pattern => "Pattern",
            Self::Entropy => "Entropy",
            Self::Flux => "Flux",
            Self::Archive => "Archive",
            Self::Momentum => "Momentum",
            Self::Anchor => "Anchor",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Pattern => "Sleeptime orchestrator and main coordinator",
            Self::Entropy => "Task complexity and breakdown specialist",
            Self::Flux => "Time management and scheduling expert",
            Self::Archive => "Memory and knowledge retrieval system",
            Self::Momentum => "Energy and flow state tracker",
            Self::Anchor => "Habits and routine builder",
        }
    }

    pub fn is_sleeptime(&self) -> bool {
        matches!(self, Self::Pattern)
    }
}

impl fmt::Display for StandardAgent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Discord channel reference
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub u64);

impl fmt::Display for ChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
