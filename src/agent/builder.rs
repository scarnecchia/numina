//! Builder pattern for constellation configuration

use super::{constellation::*, AgentId, MemoryBlockId};
use crate::{
    config::ModelConfig,
    db::Database,
    error::{ConfigError, Result},
};
use letta::LettaClient;
// Serde imports will be needed when we implement configuration loading
// use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

/// Builder pattern for easier configuration
pub struct MultiAgentSystemBuilder {
    letta: Arc<LettaClient>,
    db: Arc<Database>,
    agent_configs: Vec<AgentConfig>,
    memory_configs: Vec<MemoryBlockConfig>,
    system_prompt_template: Option<String>,
    pattern_specific_prompt: Option<String>,
    model_config: Option<ModelConfig>,
}

impl MultiAgentSystemBuilder {
    pub fn new(letta: Arc<LettaClient>, db: Arc<Database>) -> Self {
        Self {
            letta,
            db,
            agent_configs: Vec::new(),
            memory_configs: Vec::new(),
            system_prompt_template: None,
            pattern_specific_prompt: None,
            model_config: None,
        }
    }

    /// Load agent configurations from a TOML file
    pub async fn with_config_file<P: AsRef<std::path::Path>>(mut self, path: P) -> Result<Self> {
        let path = path.as_ref();
        info!("Loading agent configuration from: {}", path.display());

        let contents =
            tokio::fs::read_to_string(path)
                .await
                .map_err(|_| ConfigError::NotFound {
                    path: path.display().to_string(),
                })?;

        // Parse the TOML structure with agents and memory sections
        let config_data: toml::Value =
            toml::from_str(&contents).map_err(|e| ConfigError::ParseFailed { source: e })?;

        // Extract system prompt template if present
        if let Some(template) = config_data.get("system_prompt_template") {
            if let Some(content) = template.get("content").and_then(|v| v.as_str()) {
                self.system_prompt_template = Some(content.to_string());
            }
        }

        // Extract pattern-specific prompt if present
        if let Some(pattern) = config_data.get("pattern_specific_prompt") {
            if let Some(content) = pattern.get("content").and_then(|v| v.as_str()) {
                self.pattern_specific_prompt = Some(content.to_string());
            }
        }

        // Extract agents section
        if let Some(agents) = config_data.get("agents").and_then(|v| v.as_table()) {
            for (_key, agent_value) in agents {
                if let Ok(agent_toml) = agent_value.clone().try_into::<AgentConfigToml>() {
                    let agent_id = AgentId::new(&agent_toml.id)?;

                    // Build agent config with persona
                    let config = AgentConfig {
                        id: agent_id,
                        name: agent_toml.name,
                        description: agent_toml.description,
                        system_prompt: agent_toml.persona, // Store persona in system_prompt for now
                        is_sleeptime: agent_toml.is_sleeptime,
                        sleeptime_interval: agent_toml.sleeptime_interval,
                    };

                    self.agent_configs.push(config);
                }
            }
        }

        // Extract memory blocks section
        if let Some(memory) = config_data.get("memory").and_then(|v| v.as_table()) {
            for (_key, memory_value) in memory {
                if let Ok(memory_toml) = memory_value.clone().try_into::<MemoryBlockConfigToml>() {
                    let memory_id = MemoryBlockId::new(&memory_toml.name)?;

                    let config = MemoryBlockConfig {
                        name: memory_id,
                        max_length: memory_toml.max_length,
                        default_value: memory_toml.default_value,
                        description: memory_toml.description,
                    };

                    self.memory_configs.push(config);
                }
            }
        }

        info!(
            "Loaded {} agents and {} memory blocks from config",
            self.agent_configs.len(),
            self.memory_configs.len()
        );

        Ok(self)
    }

    pub fn with_agent(mut self, config: AgentConfig) -> Self {
        self.agent_configs.push(config);
        self
    }

    pub fn with_memory_block(mut self, config: MemoryBlockConfig) -> Self {
        self.memory_configs.push(config);
        self
    }

    pub fn with_system_prompt_template(mut self, template: &str) -> Self {
        self.system_prompt_template = Some(template.to_string());
        self
    }

    pub fn with_pattern_specific_prompt(mut self, prompt: &str) -> Self {
        self.pattern_specific_prompt = Some(prompt.to_string());
        self
    }

    pub fn with_model_config(mut self, config: ModelConfig) -> Self {
        self.model_config = Some(config);
        self
    }

    pub fn build(self) -> MultiAgentSystem {
        let mut system = MultiAgentSystem::new(self.letta, self.db);
        system.system_prompt_template = self.system_prompt_template;
        system.pattern_specific_prompt = self.pattern_specific_prompt;

        // Apply model config if provided
        if let Some(model_config) = self.model_config {
            system.model_config = Arc::new(model_config);
        }

        for config in self.agent_configs {
            system.add_agent_config(config);
        }

        for config in self.memory_configs {
            system.add_memory_config(config);
        }

        system
    }
}
