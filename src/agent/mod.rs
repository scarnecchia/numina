//! Agent-related types and functionality

use crate::error::ValidationError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
// Re-export submodules
pub mod builder;
pub mod constellation;
pub mod coordination;
pub mod human;

// Re-export commonly used types
pub use constellation::MultiAgentSystem;
pub use human::UserId;

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

/// Agent types - standard agents plus support for custom agents
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    Pattern,
    Entropy,
    Flux,
    Archive,
    Momentum,
    Anchor,
    #[serde(rename = "custom")]
    Custom(String),
}

impl AgentType {
    pub fn id(&self) -> AgentId {
        match self {
            Self::Pattern => AgentId("pattern".to_string()),
            Self::Entropy => AgentId("entropy".to_string()),
            Self::Flux => AgentId("flux".to_string()),
            Self::Archive => AgentId("archive".to_string()),
            Self::Momentum => AgentId("momentum".to_string()),
            Self::Anchor => AgentId("anchor".to_string()),
            Self::Custom(name) => AgentId(name.clone()),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Pattern => "Pattern",
            Self::Entropy => "Entropy",
            Self::Flux => "Flux",
            Self::Archive => "Archive",
            Self::Momentum => "Momentum",
            Self::Anchor => "Anchor",
            Self::Custom(name) => name,
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Self::Pattern => "Sleeptime orchestrator and main coordinator",
            Self::Entropy => "Task complexity and breakdown specialist",
            Self::Flux => "Time management and scheduling expert",
            Self::Archive => "Memory and knowledge retrieval system",
            Self::Momentum => "Energy and flow state tracker",
            Self::Anchor => "Habits and routine builder",
            Self::Custom(_) => "Custom agent",
        }
    }

    pub fn is_sleeptime(&self) -> bool {
        matches!(self, Self::Pattern)
    }

    pub fn knowledge_file(&self) -> String {
        match self {
            Self::Pattern => "coordination_patterns.md".to_string(),
            Self::Entropy => "task_patterns.md".to_string(),
            Self::Flux => "time_patterns.md".to_string(),
            Self::Archive => "memory_patterns.md".to_string(),
            Self::Momentum => "energy_patterns.md".to_string(),
            Self::Anchor => "routine_patterns.md".to_string(),
            Self::Custom(name) => format!("{}_patterns.md", name.to_lowercase()),
        }
    }
}

impl fmt::Display for AgentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl FromStr for AgentType {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pattern" => Ok(AgentType::Pattern),
            "entropy" => Ok(AgentType::Entropy),
            "flux" => Ok(AgentType::Flux),
            "archive" => Ok(AgentType::Archive),
            "momentum" => Ok(AgentType::Momentum),
            "anchor" => Ok(AgentType::Anchor),
            _ => Ok(AgentType::Custom(s.to_string())),
        }
    }
}

/// Model capability levels that agents can request
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapability {
    /// Everyday tasks, quick checks, simple queries
    Routine,
    /// Normal conversations, standard dialogue
    Interactive,
    /// Research, debugging, analysis, problem solving
    Investigative,
    /// High-stakes decisions, complex planning, critical tasks
    Critical,
}

impl ModelCapability {
    pub fn description(&self) -> &'static str {
        match self {
            Self::Routine => "Everyday tasks, quick checks, simple queries",
            Self::Interactive => "Normal conversations, standard dialogue",
            Self::Investigative => "Research, debugging, analysis, problem solving",
            Self::Critical => "High-stakes decisions, complex planning, critical tasks",
        }
    }
}

impl fmt::Display for ModelCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelCapability::Routine => write!(f, "routine"),
            ModelCapability::Interactive => write!(f, "interactive"),
            ModelCapability::Investigative => write!(f, "investigative"),
            ModelCapability::Critical => write!(f, "critical"),
        }
    }
}

impl FromStr for ModelCapability {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "routine" => Ok(ModelCapability::Routine),
            "interactive" => Ok(ModelCapability::Interactive),
            "investigative" => Ok(ModelCapability::Investigative),
            "critical" => Ok(ModelCapability::Critical),
            _ => Err(ValidationError::InvalidModelCapability(s.to_string())),
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
    fn test_agent_types() {
        let pattern = AgentType::Pattern;
        assert!(pattern.is_sleeptime());
        assert_eq!(pattern.name(), "Pattern");

        let entropy = AgentType::Entropy;
        assert!(!entropy.is_sleeptime());
        assert!(entropy.description().contains("complexity"));

        // Test custom agent
        let custom = AgentType::Custom("MyAgent".to_string());
        assert_eq!(custom.name(), "MyAgent");
        assert_eq!(custom.description(), "Custom agent");
        assert_eq!(custom.knowledge_file(), "myagent_patterns.md");

        // Verify all standard agents have unique IDs
        let agents = vec![
            AgentType::Pattern,
            AgentType::Entropy,
            AgentType::Flux,
            AgentType::Archive,
            AgentType::Momentum,
            AgentType::Anchor,
        ];

        let ids: Vec<String> = agents.iter().map(|a| a.id().as_str().to_string()).collect();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort();
        sorted_ids.dedup();
        assert_eq!(sorted_ids.len(), agents.len());
    }

    #[test]
    fn test_model_capability() {
        assert_eq!(
            ModelCapability::from_str("routine").unwrap(),
            ModelCapability::Routine
        );
        assert_eq!(
            ModelCapability::from_str("CRITICAL").unwrap(),
            ModelCapability::Critical
        );
        assert!(matches!(
            ModelCapability::from_str("invalid"),
            Err(ValidationError::InvalidModelCapability(_))
        ));
    }

    #[test]
    fn test_serialization() {
        // Test AgentId serialization
        let agent_id = AgentId::new("test_agent").unwrap();
        let json = serde_json::to_string(&agent_id).unwrap();
        assert_eq!(json, "\"test_agent\"");

        let deserialized: AgentId = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, agent_id);

        // Test ModelCapability serialization
        let capability = ModelCapability::Interactive;
        let json = serde_json::to_string(&capability).unwrap();
        assert_eq!(json, "\"interactive\"");

        // Test AgentType serialization
        let standard = AgentType::Pattern;
        let json = serde_json::to_string(&standard).unwrap();
        assert_eq!(json, "\"pattern\"");

        let custom = AgentType::Custom("CustomBot".to_string());
        let json = serde_json::to_string(&custom).unwrap();
        assert_eq!(json, "{\"custom\":\"CustomBot\"}");
    }

    #[test]
    fn test_agent_type_from_str() {
        // Standard agents
        assert_eq!(AgentType::from_str("pattern").unwrap(), AgentType::Pattern);
        assert_eq!(AgentType::from_str("ENTROPY").unwrap(), AgentType::Entropy);
        assert_eq!(AgentType::from_str("Flux").unwrap(), AgentType::Flux);

        // Custom agents - any unrecognized string becomes Custom
        assert_eq!(
            AgentType::from_str("my_custom_agent").unwrap(),
            AgentType::Custom("my_custom_agent".to_string())
        );
        assert_eq!(
            AgentType::from_str("NotAStandardAgent").unwrap(),
            AgentType::Custom("NotAStandardAgent".to_string())
        );
    }
}
