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

/// Task priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskPriority {
    Low,
    Medium,
    High,
    Urgent,
}

impl TaskPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Urgent => "urgent",
        }
    }
}

impl FromStr for TaskPriority {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "urgent" => Ok(Self::Urgent),
            _ => Ok(Self::Medium), // Default to medium
        }
    }
}

/// Task status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            _ => Self::Pending, // Default
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_id_validation() {
        // Valid IDs
        assert!(AgentId::new("pattern").is_ok());
        assert!(AgentId::new("test_agent").is_ok());
        assert!(AgentId::new("agent-123").is_ok());
        assert!(AgentId::new("UPPERCASE").is_ok());

        // Invalid IDs
        assert!(matches!(
            AgentId::new(""),
            Err(ValidationError::EmptyAgentId)
        ));
        assert!(matches!(
            AgentId::new("agent with spaces"),
            Err(ValidationError::InvalidAgentId(_))
        ));
        assert!(matches!(
            AgentId::new("agent@invalid"),
            Err(ValidationError::InvalidAgentId(_))
        ));

        // Case normalization
        let id = AgentId::new("MixedCase").unwrap();
        assert_eq!(id.as_str(), "mixedcase");
    }

    #[test]
    fn test_memory_block_id_validation() {
        // Valid IDs
        assert!(MemoryBlockId::new("current_state").is_ok());
        assert!(MemoryBlockId::new("block123").is_ok());

        // Invalid IDs
        assert!(matches!(
            MemoryBlockId::new(""),
            Err(ValidationError::EmptyMemoryBlockId)
        ));
        assert!(matches!(
            MemoryBlockId::new("block-with-dash"),
            Err(ValidationError::InvalidMemoryBlockId(_))
        ));
        assert!(matches!(
            MemoryBlockId::new("block.period"),
            Err(ValidationError::InvalidMemoryBlockId(_))
        ));
    }

    #[test]
    fn test_mcp_transport_parsing() {
        assert_eq!(
            McpTransport::from_str("stdio").unwrap(),
            McpTransport::Stdio
        );
        assert_eq!(McpTransport::from_str("SSE").unwrap(), McpTransport::Sse);
        assert_eq!(McpTransport::from_str("HTTP").unwrap(), McpTransport::Http);

        assert!(matches!(
            McpTransport::from_str("websocket"),
            Err(ValidationError::InvalidTransport(_))
        ));
    }

    #[test]
    fn test_standard_memory_blocks() {
        let current = StandardMemoryBlock::CurrentState;
        assert_eq!(current.max_length(), 200);
        assert!(current.default_value().contains("unknown"));

        let context = StandardMemoryBlock::ActiveContext;
        assert_eq!(context.max_length(), 400);

        let bond = StandardMemoryBlock::BondEvolution;
        assert_eq!(bond.max_length(), 600);
        assert!(bond.default_value().contains("trust"));
    }

    #[test]
    fn test_standard_agents() {
        let pattern = StandardAgent::Pattern;
        assert!(pattern.is_sleeptime());
        assert_eq!(pattern.name(), "Pattern");

        let entropy = StandardAgent::Entropy;
        assert!(!entropy.is_sleeptime());
        assert!(entropy.description().contains("complexity"));

        // Verify all agents have unique IDs
        let agents = vec![
            StandardAgent::Pattern,
            StandardAgent::Entropy,
            StandardAgent::Flux,
            StandardAgent::Archive,
            StandardAgent::Momentum,
            StandardAgent::Anchor,
        ];

        let ids: Vec<String> = agents.iter().map(|a| a.id().as_str().to_string()).collect();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort();
        sorted_ids.dedup();
        assert_eq!(sorted_ids.len(), agents.len());
    }

    #[test]
    fn test_task_priority() {
        assert_eq!(TaskPriority::from_str("high").unwrap(), TaskPriority::High);
        assert_eq!(
            TaskPriority::from_str("URGENT").unwrap(),
            TaskPriority::Urgent
        );
        assert_eq!(
            TaskPriority::from_str("invalid").unwrap(),
            TaskPriority::Medium
        ); // default

        assert_eq!(TaskPriority::Low.as_str(), "low");
    }

    #[test]
    fn test_serialization() {
        // Test AgentId serialization
        let agent_id = AgentId::new("test_agent").unwrap();
        let json = serde_json::to_string(&agent_id).unwrap();
        assert_eq!(json, "\"test_agent\"");

        let deserialized: AgentId = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, agent_id);

        // Test McpTransport serialization
        let transport = McpTransport::Sse;
        let json = serde_json::to_string(&transport).unwrap();
        assert_eq!(json, "\"sse\"");

        // Test TaskStatus serialization
        let status = TaskStatus::InProgress;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"in_progress\"");
    }
}
