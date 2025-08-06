//! Configuration system for Pattern
//!
//! This module provides configuration structures and utilities for persisting
//! Pattern settings across sessions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    Result,
    agent::tool_rules::ToolRule,
    context::compression::CompressionStrategy,
    data_source::bluesky::BlueskyFilter,
    db::DatabaseConfig,
    id::{AgentId, GroupId, MemoryId, UserId},
    memory::{MemoryPermission, MemoryType},
};

/// Resolve a path relative to a base directory
/// If the path is absolute, return it as-is
/// If the path is relative, resolve it relative to the base directory
fn resolve_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

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

    /// Bluesky configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bluesky: Option<BlueskyConfig>,
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

    /// Optional Bluesky handle for this agent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bluesky_handle: Option<String>,

    /// Tool execution rules for this agent
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_rules: Vec<ToolRuleConfig>,

    /// Available tools for this agent
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,

    /// Optional model configuration (overrides global model config)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelConfig>,

    /// Optional context configuration (overrides defaults)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextConfigOptions>,
}

/// Configuration for tool execution rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRuleConfig {
    /// Name of the tool this rule applies to
    pub tool_name: String,

    /// Type of rule
    pub rule_type: ToolRuleTypeConfig,

    /// Conditions for this rule (tool names, parameters, etc.)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<String>,

    /// Priority of this rule (higher numbers = higher priority)
    #[serde(default = "default_rule_priority")]
    pub priority: u8,

    /// Optional metadata for this rule
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Configuration for tool rule types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum ToolRuleTypeConfig {
    /// Continue the conversation loop after this tool is called (no heartbeat required)
    ContinueLoop,

    /// Exit conversation loop after this tool is called
    ExitLoop,

    /// This tool must be called after specified tools (ordering dependency)
    RequiresPrecedingTools,

    /// This tool must be called before specified tools
    RequiresFollowingTools,

    /// Multiple exclusive groups - only one tool from each group can be called per conversation
    ExclusiveGroups(Vec<Vec<String>>),

    /// Call this tool at conversation start
    StartConstraint,

    /// This tool must be called before conversation ends
    RequiredBeforeExit,

    /// Required for exit if condition is met
    RequiredBeforeExitIf,

    /// Maximum number of times this tool can be called
    MaxCalls(u32),

    /// Minimum cooldown period between calls (in seconds)
    Cooldown(u64),

    /// Call this tool periodically during long conversations (in seconds)
    Periodic(u64),
}

fn default_rule_priority() -> u8 {
    5
}

impl ToolRuleConfig {
    /// Convert configuration to runtime ToolRule
    pub fn to_tool_rule(&self) -> Result<ToolRule> {
        let rule_type = self.rule_type.to_runtime_type()?;
        let mut tool_rule = ToolRule::new(self.tool_name.clone(), rule_type);

        if !self.conditions.is_empty() {
            tool_rule = tool_rule.with_conditions(self.conditions.clone());
        }

        tool_rule = tool_rule.with_priority(self.priority);

        if let Some(metadata) = &self.metadata {
            tool_rule = tool_rule.with_metadata(metadata.clone());
        }

        Ok(tool_rule)
    }

    /// Create configuration from runtime ToolRule
    pub fn from_tool_rule(rule: &ToolRule) -> Self {
        Self {
            tool_name: rule.tool_name.clone(),
            rule_type: ToolRuleTypeConfig::from_runtime_type(&rule.rule_type),
            conditions: rule.conditions.clone(),
            priority: rule.priority,
            metadata: rule.metadata.clone(),
        }
    }
}

impl ToolRuleTypeConfig {
    /// Convert configuration type to runtime type
    pub fn to_runtime_type(&self) -> Result<crate::agent::tool_rules::ToolRuleType> {
        use crate::agent::tool_rules::ToolRuleType;
        use std::time::Duration;

        let runtime_type = match self {
            ToolRuleTypeConfig::ContinueLoop => ToolRuleType::ContinueLoop,
            ToolRuleTypeConfig::ExitLoop => ToolRuleType::ExitLoop,
            ToolRuleTypeConfig::RequiresPrecedingTools => ToolRuleType::RequiresPrecedingTools,
            ToolRuleTypeConfig::RequiresFollowingTools => ToolRuleType::RequiresFollowingTools,
            ToolRuleTypeConfig::ExclusiveGroups(groups) => {
                ToolRuleType::ExclusiveGroups(groups.clone())
            }
            ToolRuleTypeConfig::StartConstraint => ToolRuleType::StartConstraint,
            ToolRuleTypeConfig::RequiredBeforeExit => ToolRuleType::RequiredBeforeExit,
            ToolRuleTypeConfig::RequiredBeforeExitIf => ToolRuleType::RequiredBeforeExitIf,
            ToolRuleTypeConfig::MaxCalls(max) => ToolRuleType::MaxCalls(*max),
            ToolRuleTypeConfig::Cooldown(seconds) => {
                ToolRuleType::Cooldown(Duration::from_secs(*seconds))
            }
            ToolRuleTypeConfig::Periodic(seconds) => {
                ToolRuleType::Periodic(Duration::from_secs(*seconds))
            }
        };

        Ok(runtime_type)
    }

    /// Create configuration type from runtime type
    pub fn from_runtime_type(runtime_type: &crate::agent::tool_rules::ToolRuleType) -> Self {
        use crate::agent::tool_rules::ToolRuleType;

        match runtime_type {
            ToolRuleType::ContinueLoop => ToolRuleTypeConfig::ContinueLoop,
            ToolRuleType::ExitLoop => ToolRuleTypeConfig::ExitLoop,
            ToolRuleType::RequiresPrecedingTools => ToolRuleTypeConfig::RequiresPrecedingTools,
            ToolRuleType::RequiresFollowingTools => ToolRuleTypeConfig::RequiresFollowingTools,
            ToolRuleType::ExclusiveGroups(groups) => {
                ToolRuleTypeConfig::ExclusiveGroups(groups.clone())
            }
            ToolRuleType::StartConstraint => ToolRuleTypeConfig::StartConstraint,
            ToolRuleType::RequiredBeforeExit => ToolRuleTypeConfig::RequiredBeforeExit,
            ToolRuleType::RequiredBeforeExitIf => ToolRuleTypeConfig::RequiredBeforeExitIf,
            ToolRuleType::MaxCalls(max) => ToolRuleTypeConfig::MaxCalls(*max),
            ToolRuleType::Cooldown(duration) => ToolRuleTypeConfig::Cooldown(duration.as_secs()),
            ToolRuleType::Periodic(duration) => ToolRuleTypeConfig::Periodic(duration.as_secs()),
        }
    }
}

impl AgentConfig {
    /// Convert tool rule configurations to runtime tool rules
    pub fn get_tool_rules(&self) -> Result<Vec<ToolRule>> {
        self.tool_rules
            .iter()
            .map(|config| config.to_tool_rule())
            .collect()
    }

    /// Set tool rules from runtime types
    pub fn set_tool_rules(&mut self, rules: &[ToolRule]) {
        self.tool_rules = rules.iter().map(ToolRuleConfig::from_tool_rule).collect();
    }
}

impl AgentConfig {
    /// Load agent configuration from a file
    pub async fn load_from_file(path: &Path) -> Result<Self> {
        let content = tokio::fs::read_to_string(path).await.map_err(|e| {
            crate::CoreError::ConfigurationError {
                field: "agent config file".to_string(),
                config_path: path.display().to_string(),
                expected: "valid TOML file".to_string(),
                cause: crate::error::ConfigError::Io(e.to_string()),
            }
        })?;

        let mut config: AgentConfig =
            toml::from_str(&content).map_err(|e| crate::CoreError::ConfigurationError {
                field: "agent config".to_string(),
                config_path: path.display().to_string(),
                expected: "valid agent configuration".to_string(),
                cause: crate::error::ConfigError::TomlParse(e.to_string()),
            })?;

        // Resolve paths relative to the config file's directory
        let base_dir = path.parent().unwrap_or(Path::new("."));

        // Resolve memory block content_paths
        for (_, memory_block) in config.memory.iter_mut() {
            if let Some(ref content_path) = memory_block.content_path {
                memory_block.content_path = Some(resolve_path(base_dir, content_path));
            }
        }

        Ok(config)
    }
}

/// Configuration for a memory block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBlockConfig {
    /// Content of the memory block (inline)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    /// Path to file containing the content
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_path: Option<PathBuf>,

    /// Permission level for this block
    #[serde(default)]
    pub permission: MemoryPermission,

    /// Type of memory (core, working, archival)
    #[serde(default)]
    pub memory_type: MemoryType,

    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Optional ID for shared memory blocks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<MemoryId>,

    /// Whether this memory should be shared with other agents
    #[serde(default)]
    pub shared: bool,
}

impl MemoryBlockConfig {
    /// Load content from either inline or file path
    pub async fn load_content(&self) -> Result<String> {
        if let Some(content) = &self.content {
            Ok(content.clone())
        } else if let Some(path) = &self.content_path {
            tokio::fs::read_to_string(path).await.map_err(|e| {
                crate::CoreError::ConfigurationError {
                    field: "content_path".to_string(),
                    config_path: path.display().to_string(),
                    expected: "valid file path".to_string(),
                    cause: crate::error::ConfigError::Io(e.to_string()),
                }
            })
        } else {
            Err(crate::CoreError::ConfigurationError {
                field: "memory block".to_string(),
                config_path: "unknown".to_string(),
                expected: "either 'content' or 'content_path'".to_string(),
                cause: crate::error::ConfigError::MissingField(
                    "content or content_path".to_string(),
                ),
            })
        }
    }
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

    /// Optional path to agent configuration file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<PathBuf>,

    /// Optional inline agent configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_config: Option<AgentConfig>,

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

/// Configuration for a sleeptime trigger
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SleeptimeTriggerConfig {
    /// Name of the trigger
    pub name: String,
    /// Condition that activates this trigger
    pub condition: TriggerConditionConfig,
    /// Priority of this trigger
    pub priority: TriggerPriorityConfig,
}

/// Configuration for trigger conditions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerConditionConfig {
    /// Time-based trigger
    TimeElapsed {
        /// Duration in seconds
        duration: u64,
    },
    /// Metric-based trigger
    MetricThreshold {
        /// Metric name
        metric: String,
        /// Threshold value
        threshold: f64,
    },
    /// Constellation activity trigger
    ConstellationActivity {
        /// Number of messages to trigger
        message_threshold: u32,
        /// Time window in seconds
        time_threshold: u64,
    },
    /// Custom evaluator
    Custom {
        /// Custom evaluator name
        evaluator: String,
    },
}

/// Configuration for trigger priority
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerPriorityConfig {
    Critical,
    High,
    Medium,
    Low,
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
        /// Optional configuration for the selector
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        selector_config: HashMap<String, String>,
    },
    /// Background monitoring
    Sleeptime {
        /// Check interval in seconds
        check_interval: u64,
        /// Triggers that can activate intervention
        triggers: Vec<SleeptimeTriggerConfig>,
        /// Optional member name to activate on triggers (uses least recently active if not specified)
        #[serde(skip_serializing_if = "Option::is_none")]
        intervention_agent: Option<String>,
    },
}

fn default_skip_unavailable() -> bool {
    true
}

/// Bluesky/ATProto configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueskyConfig {
    /// Default filter for the firehose
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_filter: Option<BlueskyFilter>,

    /// Whether to automatically connect to firehose on startup
    #[serde(default)]
    pub auto_connect_firehose: bool,

    /// Jetstream endpoint URL (defaults to public endpoint)
    #[serde(default = "default_jetstream_endpoint")]
    pub jetstream_endpoint: String,
}

fn default_jetstream_endpoint() -> String {
    "wss://jetstream2.us-east.bsky.network/subscribe".to_string()
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
            bluesky: None,
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
            bluesky_handle: None,
            tool_rules: Vec::new(),
            tools: Vec::new(),
            model: None,
            context: None,
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
            cause: crate::error::ConfigError::Io(e.to_string()),
        }
    })?;

    let mut config: PatternConfig =
        toml::from_str(&content).map_err(|e| crate::CoreError::ConfigurationError {
            config_path: path.display().to_string(),
            field: "content".to_string(),
            expected: "valid TOML configuration".to_string(),
            cause: crate::error::ConfigError::TomlParse(e.to_string()),
        })?;

    // Resolve paths relative to the config file's directory
    let base_dir = path.parent().unwrap_or(Path::new("."));

    // Resolve paths in main agent memory blocks
    for (_, memory_block) in config.agent.memory.iter_mut() {
        if let Some(ref content_path) = memory_block.content_path {
            memory_block.content_path = Some(resolve_path(base_dir, content_path));
        }
    }

    // Resolve paths in group members
    for group in config.groups.iter_mut() {
        for member in group.members.iter_mut() {
            if let Some(ref config_path) = member.config_path {
                member.config_path = Some(resolve_path(base_dir, config_path));
            }
        }
    }

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
                cause: crate::error::ConfigError::Io(e.to_string()),
            }
        })?;
    }

    let content =
        toml::to_string_pretty(config).map_err(|e| crate::CoreError::ConfigurationError {
            config_path: path.display().to_string(),
            field: "serialization".to_string(),
            expected: "serializable config structure".to_string(),
            cause: crate::error::ConfigError::TomlSerialize(e.to_string()),
        })?;

    tokio::fs::write(path, content)
        .await
        .map_err(|e| crate::CoreError::ConfigurationError {
            config_path: path.display().to_string(),
            field: "file".to_string(),
            expected: "writable file location".to_string(),
            cause: crate::error::ConfigError::Io(e.to_string()),
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
        bluesky: overlay.bluesky.or(base.bluesky),
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

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bluesky: Option<BlueskyConfig>,
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

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bluesky_handle: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_rules: Option<Vec<ToolRuleConfig>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelConfig>,
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
        bluesky_handle: overlay.bluesky_handle.or(base.bluesky_handle),
        tool_rules: overlay.tool_rules.unwrap_or(base.tool_rules),
        tools: overlay.tools.unwrap_or(base.tools),
        model: overlay.model.or(base.model),
        context: base.context, // Keep base context config for now (no overlay field yet)
    }
}

/// Optional context configuration for agents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfigOptions {
    /// Maximum messages to keep before compression
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_messages: Option<usize>,

    /// Maximum age of messages in hours before archival
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_message_age_hours: Option<i64>,

    /// Number of messages that triggers compression
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression_threshold: Option<usize>,

    /// Compression strategy to use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression_strategy: Option<CompressionStrategy>,

    /// Characters limit per memory block
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_char_limit: Option<usize>,

    /// Whether to enable thinking/reasoning
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<bool>,
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

impl PatternConfig {
    /// Load configuration from standard locations
    pub async fn load() -> Result<Self> {
        load_config_from_standard_locations().await
    }

    /// Load configuration from a specific file
    pub async fn load_from(path: &Path) -> Result<Self> {
        load_config(path).await
    }

    /// Save configuration to a specific file
    pub async fn save_to(&self, path: &Path) -> Result<()> {
        save_config(self, path).await
    }

    /// Save configuration to standard location
    pub async fn save(&self) -> Result<()> {
        let config_path = config_paths()
            .into_iter()
            .find(|p| p.parent().map_or(false, |parent| parent.exists()))
            .unwrap_or_else(|| {
                dirs::config_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("pattern")
                    .join("config.toml")
            });

        self.save_to(&config_path).await
    }

    /// Get tool rules for a specific agent by name
    pub fn get_agent_tool_rules(&self, agent_name: &str) -> Result<Vec<ToolRule>> {
        if self.agent.name == agent_name {
            return self.agent.get_tool_rules();
        }

        // Look in groups for agents with matching names
        for group in &self.groups {
            for member in &group.members {
                if member.name == agent_name {
                    // For now, group members don't have individual tool rules
                    // This could be extended in the future
                    return Ok(Vec::new());
                }
            }
        }

        // Agent not found, return empty rules
        Ok(Vec::new())
    }

    /// Set tool rules for the main agent
    pub fn set_agent_tool_rules(&mut self, rules: &[ToolRule]) {
        self.agent.set_tool_rules(rules);
    }
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
    fn test_tool_rules_configuration() {
        use crate::agent::tool_rules::{ToolRule, ToolRuleType};
        use std::time::Duration;

        // Create tool rules
        let rules = vec![
            ToolRule::start_constraint("setup".to_string()),
            ToolRule::continue_loop("fast_search".to_string()),
            ToolRule::max_calls("api_call".to_string(), 3),
            ToolRule::cooldown("slow_tool".to_string(), Duration::from_secs(5)),
        ];

        // Create agent config with tool rules
        let mut agent_config = AgentConfig::default();
        agent_config.set_tool_rules(&rules);

        // Test conversion
        let loaded_rules = agent_config.get_tool_rules().unwrap();
        assert_eq!(loaded_rules.len(), 4);

        // Test individual rule types
        assert_eq!(loaded_rules[0].tool_name, "setup");
        assert!(matches!(
            loaded_rules[0].rule_type,
            ToolRuleType::StartConstraint
        ));

        assert_eq!(loaded_rules[1].tool_name, "fast_search");
        assert!(matches!(
            loaded_rules[1].rule_type,
            ToolRuleType::ContinueLoop
        ));

        assert_eq!(loaded_rules[2].tool_name, "api_call");
        assert!(matches!(
            loaded_rules[2].rule_type,
            ToolRuleType::MaxCalls(3)
        ));

        assert_eq!(loaded_rules[3].tool_name, "slow_tool");
        assert!(matches!(
            loaded_rules[3].rule_type,
            ToolRuleType::Cooldown(_)
        ));
    }

    #[test]
    fn test_tool_rule_config_serialization() {
        use crate::agent::tool_rules::ToolRule;
        use std::time::Duration;

        let rule = ToolRule::cooldown("test_tool".to_string(), Duration::from_secs(30));
        let config_rule = ToolRuleConfig::from_tool_rule(&rule);

        // Test serialization
        let serialized = toml::to_string(&config_rule).unwrap();
        assert!(serialized.contains("tool_name"));
        assert!(serialized.contains("rule_type"));

        // Test deserialization
        let deserialized: ToolRuleConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.tool_name, "test_tool");

        // Convert back to runtime type
        let runtime_rule = deserialized.to_tool_rule().unwrap();
        assert_eq!(runtime_rule.tool_name, "test_tool");
        assert!(matches!(
            runtime_rule.rule_type,
            crate::agent::tool_rules::ToolRuleType::Cooldown(_)
        ));
    }

    #[tokio::test]
    async fn test_pattern_config_with_tool_rules() {
        use crate::agent::tool_rules::ToolRule;

        // Create a config with tool rules
        let mut config = PatternConfig::default();
        let rules = vec![
            ToolRule::start_constraint("init".to_string()),
            ToolRule::continue_loop("search".to_string()),
        ];
        config.set_agent_tool_rules(&rules);

        // Test getting rules back
        let loaded_rules = config.get_agent_tool_rules(&config.agent.name).unwrap();
        assert_eq!(loaded_rules.len(), 2);
        assert_eq!(loaded_rules[0].tool_name, "init");
        assert_eq!(loaded_rules[1].tool_name, "search");

        // Test serialization roundtrip
        let toml_content = toml::to_string_pretty(&config).unwrap();
        let deserialized_config: PatternConfig = toml::from_str(&toml_content).unwrap();

        let reloaded_rules = deserialized_config
            .get_agent_tool_rules(&config.agent.name)
            .unwrap();
        assert_eq!(reloaded_rules.len(), 2);
        assert_eq!(reloaded_rules[0].tool_name, "init");
        assert_eq!(reloaded_rules[1].tool_name, "search");
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
                    config_path: None,
                    agent_config: None,
                    role: GroupMemberRoleConfig::Regular,
                    capabilities: vec!["planning".to_string(), "organization".to_string()],
                },
                GroupMemberConfig {
                    name: "Memory".to_string(),
                    agent_id: Some(AgentId::generate()),
                    config_path: None,
                    agent_config: None,
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
