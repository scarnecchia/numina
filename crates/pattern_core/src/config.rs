//! Configuration system for Pattern
//!
//! This module provides configuration structures and utilities for persisting
//! Pattern settings across sessions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    Result,
    db::DatabaseConfig,
    id::{AgentId, GroupId, UserId},
    memory::{MemoryPermission, MemoryType},
};

/// Top-level configuration for Pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternConfig {
    /// User configuration
    pub user: UserConfig,

    /// Agent configuration
    pub agent: AgentConfig,

    /// Model provider configuration
    pub model: ModelConfig,

    /// Database configuration
    #[serde(default)]
    pub database: DatabaseConfig,

    /// Agent groups configuration
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GroupConfig>,
}

/// User configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    /// User ID (persisted across sessions)
    #[serde(default)]
    pub id: UserId,

    /// Optional user name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// User-specific settings
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub settings: HashMap<String, serde_json::Value>,
}

/// Agent configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent ID (persisted once created)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<AgentId>,

    /// Agent name
    pub name: String,

    /// System prompt/base instructions for the agent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,

    /// Agent persona (creates a core memory block)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,

    /// Additional instructions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,

    /// Initial memory blocks
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub memory: HashMap<String, MemoryBlockConfig>,
}

/// Configuration for a memory block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBlockConfig {
    /// Content of the memory block
    pub content: String,

    /// Permission level for this block
    #[serde(default)]
    pub permission: MemoryPermission,

    /// Type of memory (core, working, archival)
    #[serde(default)]
    pub memory_type: MemoryType,

    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Configuration for an agent group
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupConfig {
    /// Optional ID (generated if not provided)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<GroupId>,

    /// Name of the group
    pub name: String,

    /// Description of the group's purpose
    pub description: String,

    /// Coordination pattern to use
    pub pattern: GroupPatternConfig,

    /// Members of this group
    pub members: Vec<GroupMemberConfig>,
}

/// Configuration for a group member
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMemberConfig {
    /// Friendly name for this agent in the group
    pub name: String,

    /// Optional agent ID (if referencing existing agent)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,

    /// Role in the group
    #[serde(default)]
    pub role: GroupMemberRoleConfig,

    /// Capabilities this agent brings
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

/// Member role configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GroupMemberRoleConfig {
    #[default]
    Regular,
    Supervisor,
    Specialist {
        domain: String,
    },
}

/// Configuration for coordination patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GroupPatternConfig {
    /// One agent leads, others follow
    Supervisor {
        /// The agent that leads (by member name)
        leader: String,
    },
    /// Agents take turns in order
    RoundRobin {
        /// Whether to skip unavailable agents
        #[serde(default = "default_skip_unavailable")]
        skip_unavailable: bool,
    },
    /// Sequential processing pipeline
    Pipeline {
        /// Ordered list of member names for each stage
        stages: Vec<String>,
    },
    /// Dynamic selection based on context
    Dynamic {
        /// Selector strategy name
        selector: String,
    },
    /// Background monitoring
    Sleeptime {
        /// Check interval in seconds
        interval_seconds: u64,
        /// Member name to activate on triggers
        intervention_agent: String,
    },
}

fn default_skip_unavailable() -> bool {
    true
}

/// Model provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Provider name (e.g., "anthropic", "openai")
    pub provider: String,

    /// Optional specific model to use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Optional temperature setting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Additional provider-specific settings
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub settings: HashMap<String, toml::Value>,
}

// Default implementations
impl Default for PatternConfig {
    fn default() -> Self {
        Self {
            user: UserConfig::default(),
            agent: AgentConfig::default(),
            model: ModelConfig::default(),
            database: DatabaseConfig::default(),
            groups: Vec::new(),
        }
    }
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            id: UserId::generate(),
            name: None,
            settings: HashMap::new(),
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            id: None,
            name: "Assistant".to_string(),
            system_prompt: None,
            persona: None,
            instructions: None,
            memory: HashMap::new(),
        }
    }
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: "Gemini".to_string(),
            model: None,
            temperature: None,
            settings: HashMap::new(),
        }
    }
}

// MemoryPermission already has Default derived

// Utility functions

/// Load configuration from a TOML file
pub async fn load_config(path: &Path) -> Result<PatternConfig> {
    let content = tokio::fs::read_to_string(path).await.map_err(|e| {
        crate::CoreError::ConfigurationError {
            config_path: path.display().to_string(),
            field: "file".to_string(),
            expected: "readable TOML file".to_string(),
            cause: Box::new(e),
        }
    })?;

    let config: PatternConfig =
        toml::from_str(&content).map_err(|e| crate::CoreError::ConfigurationError {
            config_path: path.display().to_string(),
            field: "content".to_string(),
            expected: "valid TOML configuration".to_string(),
            cause: Box::new(e),
        })?;

    Ok(config)
}

/// Save configuration to a TOML file
pub async fn save_config(config: &PatternConfig, path: &Path) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            crate::CoreError::ConfigurationError {
                config_path: parent.display().to_string(),
                field: "directory".to_string(),
                expected: "writable directory".to_string(),
                cause: Box::new(e),
            }
        })?;
    }

    let content =
        toml::to_string_pretty(config).map_err(|e| crate::CoreError::ConfigurationError {
            config_path: path.display().to_string(),
            field: "serialization".to_string(),
            expected: "serializable config structure".to_string(),
            cause: Box::new(e),
        })?;

    tokio::fs::write(path, content)
        .await
        .map_err(|e| crate::CoreError::ConfigurationError {
            config_path: path.display().to_string(),
            field: "file".to_string(),
            expected: "writable file location".to_string(),
            cause: Box::new(e),
        })?;

    Ok(())
}

/// Merge two configurations, with the overlay taking precedence
pub fn merge_configs(base: PatternConfig, overlay: PartialConfig) -> PatternConfig {
    PatternConfig {
        user: overlay.user.unwrap_or(base.user),
        agent: if let Some(agent_overlay) = overlay.agent {
            merge_agent_configs(base.agent, agent_overlay)
        } else {
            base.agent
        },
        model: overlay.model.unwrap_or(base.model),
        database: overlay.database.unwrap_or(base.database),
        groups: overlay.groups.unwrap_or(base.groups),
    }
}

/// Partial configuration for overlaying
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PartialConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<PartialAgentConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<DatabaseConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<GroupConfig>>,
}

/// Partial agent configuration for overlaying
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PartialAgentConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<AgentId>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<HashMap<String, MemoryBlockConfig>>,
}

fn merge_agent_configs(base: AgentConfig, overlay: PartialAgentConfig) -> AgentConfig {
    AgentConfig {
        id: overlay.id.or(base.id),
        name: overlay.name.unwrap_or(base.name),
        system_prompt: overlay.system_prompt.or(base.system_prompt),
        persona: overlay.persona.or(base.persona),
        instructions: overlay.instructions.or(base.instructions),
        memory: if let Some(overlay_memory) = overlay.memory {
            // Merge memory blocks, overlay takes precedence
            let mut merged = base.memory;
            merged.extend(overlay_memory);
            merged
        } else {
            base.memory
        },
    }
}

/// Standard config file locations
pub fn config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Project-specific config
    paths.push(PathBuf::from("pattern.toml"));

    // User config directory
    if let Some(config_dir) = dirs::config_dir() {
        paths.push(config_dir.join("pattern").join("config.toml"));
    }

    // Home directory fallback
    if let Some(home_dir) = dirs::home_dir() {
        paths.push(home_dir.join(".pattern").join("config.toml"));
    }

    paths
}

/// Load configuration from standard locations
pub async fn load_config_from_standard_locations() -> Result<PatternConfig> {
    for path in config_paths() {
        if path.exists() {
            return load_config(&path).await;
        }
    }

    // No config found, return default
    Ok(PatternConfig::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = PatternConfig::default();
        assert_eq!(config.agent.name, "Assistant");
        assert_eq!(config.model.provider, "Gemini");
        assert!(config.groups.is_empty());
    }

    #[test]
    fn test_config_serialization() {
        let config = PatternConfig::default();
        let toml = toml::to_string_pretty(&config).unwrap();
        assert!(toml.contains("[user]"));
        assert!(toml.contains("[agent]"));
        assert!(toml.contains("[model]"));
    }

    #[test]
    fn test_merge_configs() {
        let base = PatternConfig::default();
        let overlay = PartialConfig {
            agent: Some(PartialAgentConfig {
                name: Some("Custom Agent".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let merged = merge_configs(base, overlay);
        assert_eq!(merged.agent.name, "Custom Agent");
        // persona is None by default
        assert_eq!(merged.agent.persona, None);
    }

    #[test]
    fn test_group_config_serialization() {
        let group = GroupConfig {
            id: None,
            name: "Main Group".to_string(),
            description: "Primary ADHD support group".to_string(),
            pattern: GroupPatternConfig::RoundRobin {
                skip_unavailable: true,
            },
            members: vec![
                GroupMemberConfig {
                    name: "Executive".to_string(),
                    agent_id: None,
                    role: GroupMemberRoleConfig::Regular,
                    capabilities: vec!["planning".to_string(), "organization".to_string()],
                },
                GroupMemberConfig {
                    name: "Memory".to_string(),
                    agent_id: Some(AgentId::generate()),
                    role: GroupMemberRoleConfig::Specialist {
                        domain: "memory_management".to_string(),
                    },
                    capabilities: vec!["recall".to_string()],
                },
            ],
        };

        let toml = toml::to_string_pretty(&group).unwrap();
        assert!(toml.contains("name = \"Main Group\""));
        assert!(toml.contains("type = \"round_robin\""));
        assert!(toml.contains("[[members]]"));
        assert!(toml.contains("name = \"Executive\""));
    }
}
