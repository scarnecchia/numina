use crate::error::{ConfigError, PatternError, Result};
use foyer::{DirectFsDeviceOptions, Engine, HybridCache, HybridCacheBuilder};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

/// Lightweight cache entry for agent ID mapping
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedAgentId {
    pub letta_id: String,
    pub system_prompt_hash: String,
}

/// Pattern's multi-tier cache system
#[derive(Clone, Debug)]
pub struct PatternCache {
    // user_id + agent_type -> Letta agent ID with metadata
    agents: HybridCache<String, Arc<CachedAgentId>>,

    // user_id + group_name -> Letta Group (already serializable)
    groups: HybridCache<String, Arc<letta::types::Group>>,

    // user_id + block_name -> just the block value string
    memory_blocks: HybridCache<String, Arc<String>>,

    // Optional: agent_id -> AgentState for full agent caching
    agent_states: HybridCache<String, Arc<letta::types::AgentState>>,
}

impl PatternCache {
    /// Create a new cache instance
    pub async fn new(cache_dir: &Path) -> Result<Self> {
        // Create cache directories
        std::fs::create_dir_all(cache_dir).map_err(|e| {
            PatternError::Config(ConfigError::Invalid {
                field: "cache_dir".to_string(),
                reason: format!("Failed to create cache directory: {}", e),
            })
        })?;
        std::fs::create_dir_all(cache_dir.join("agents")).map_err(|e| {
            PatternError::Config(ConfigError::Invalid {
                field: "cache_dir".to_string(),
                reason: format!("Failed to create agents cache directory: {}", e),
            })
        })?;
        std::fs::create_dir_all(cache_dir.join("groups")).map_err(|e| {
            PatternError::Config(ConfigError::Invalid {
                field: "cache_dir".to_string(),
                reason: format!("Failed to create groups cache directory: {}", e),
            })
        })?;
        std::fs::create_dir_all(cache_dir.join("memory")).map_err(|e| {
            PatternError::Config(ConfigError::Invalid {
                field: "cache_dir".to_string(),
                reason: format!("Failed to create memory cache directory: {}", e),
            })
        })?;
        std::fs::create_dir_all(cache_dir.join("agent_states")).map_err(|e| {
            PatternError::Config(ConfigError::Invalid {
                field: "cache_dir".to_string(),
                reason: format!("Failed to create agent_states cache directory: {}", e),
            })
        })?;

        info!("Initializing Pattern cache at {:?}", cache_dir);

        // Agents: Hot cache, accessed on every message
        let agents = HybridCacheBuilder::new()
            .memory(64 * 1024 * 1024) // 64MB in-memory
            .storage(Engine::large()) // Optimized for larger values
            .with_device_options(
                DirectFsDeviceOptions::new(cache_dir.join("agents"))
                    .with_capacity(256 * 1024 * 1024), // 256MB on disk
            )
            .build()
            .await
            .map_err(|e| {
                PatternError::Config(ConfigError::Invalid {
                    field: "cache".to_string(),
                    reason: format!("Failed to create agents cache: {}", e),
                })
            })?;

        // Groups: Medium access frequency
        let groups = HybridCacheBuilder::new()
            .memory(32 * 1024 * 1024) // 32MB in-memory
            .storage(Engine::small()) // Smaller values
            .with_device_options(
                DirectFsDeviceOptions::new(cache_dir.join("groups"))
                    .with_capacity(128 * 1024 * 1024), // 128MB on disk
            )
            .build()
            .await
            .map_err(|e| {
                PatternError::Config(ConfigError::Invalid {
                    field: "cache".to_string(),
                    reason: format!("Failed to create groups cache: {}", e),
                })
            })?;

        // Memory blocks: Frequently updated
        let memory_blocks = HybridCacheBuilder::new()
            .memory(128 * 1024 * 1024) // 128MB in-memory
            .storage(Engine::large())
            .with_device_options(
                DirectFsDeviceOptions::new(cache_dir.join("memory"))
                    .with_capacity(512 * 1024 * 1024), // 512MB on disk
            )
            .build()
            .await
            .map_err(|e| {
                PatternError::Config(ConfigError::Invalid {
                    field: "cache".to_string(),
                    reason: format!("Failed to create memory blocks cache: {}", e),
                })
            })?;

        // Agent states: Full agent state caching
        let agent_states = HybridCacheBuilder::new()
            .memory(256 * 1024 * 1024) // 256MB in-memory
            .storage(Engine::large())
            .with_device_options(
                DirectFsDeviceOptions::new(cache_dir.join("agent_states"))
                    .with_capacity(1024 * 1024 * 1024), // 1GB on disk
            )
            .build()
            .await
            .map_err(|e| {
                PatternError::Config(ConfigError::Invalid {
                    field: "cache".to_string(),
                    reason: format!("Failed to create agent states cache: {}", e),
                })
            })?;

        Ok(Self {
            agents,
            groups,
            memory_blocks,
            agent_states,
        })
    }

    // Agent cache operations

    /// Get a cached agent ID
    pub async fn get_agent(
        &self,
        user_id: i64,
        agent_type: &str,
    ) -> Result<Option<Arc<CachedAgentId>>> {
        let key = agent_key(user_id, agent_type);
        match self.agents.get(&key).await {
            Ok(Some(entry)) => Ok(Some(entry.value().clone())),
            Ok(None) => Ok(None),
            Err(e) => Err(PatternError::Config(ConfigError::Invalid {
                field: "cache".to_string(),
                reason: format!("Failed to get agent from cache: {}", e),
            })),
        }
    }

    /// Cache an agent ID
    pub async fn insert_agent(
        &self,
        user_id: i64,
        agent_type: &str,
        letta_id: String,
        system_prompt_hash: String,
    ) -> Result<()> {
        let key = agent_key(user_id, agent_type);
        let agent = CachedAgentId {
            letta_id,
            system_prompt_hash,
        };
        self.agents.insert(key, Arc::new(agent));
        Ok(())
    }

    /// Remove an agent from cache
    pub async fn remove_agent(
        &self,
        user_id: i64,
        agent_type: &str,
    ) -> Result<Option<Arc<CachedAgentId>>> {
        let key = agent_key(user_id, agent_type);
        // Note: foyer's remove doesn't return the old value, so we need to get it first if needed
        let old_value = self.get_agent(user_id, agent_type).await?;
        self.agents.remove(&key);
        Ok(old_value)
    }

    // Group cache operations

    /// Get a cached group
    pub async fn get_group(
        &self,
        user_id: i64,
        group_name: &str,
    ) -> Result<Option<Arc<letta::types::Group>>> {
        let key = group_key(user_id, group_name);
        match self.groups.get(&key).await {
            Ok(Some(entry)) => Ok(Some(entry.value().clone())),
            Ok(None) => Ok(None),
            Err(e) => Err(PatternError::Config(ConfigError::Invalid {
                field: "cache".to_string(),
                reason: format!("Failed to get group from cache: {}", e),
            })),
        }
    }

    /// Cache a group
    pub async fn insert_group(
        &self,
        user_id: i64,
        group_name: &str,
        group: letta::types::Group,
    ) -> Result<()> {
        let key = group_key(user_id, group_name);
        self.groups.insert(key, Arc::new(group));
        Ok(())
    }

    /// Remove a group from cache
    pub async fn remove_group(
        &self,
        user_id: i64,
        group_name: &str,
    ) -> Result<Option<Arc<letta::types::Group>>> {
        let key = group_key(user_id, group_name);
        let old_value = self.get_group(user_id, group_name).await?;
        self.groups.remove(&key);
        Ok(old_value)
    }

    // Memory block cache operations

    /// Get a cached memory block value
    pub async fn get_memory_block(
        &self,
        user_id: i64,
        block_name: &str,
    ) -> Result<Option<Arc<String>>> {
        let key = memory_key(user_id, block_name);
        match self.memory_blocks.get(&key).await {
            Ok(Some(entry)) => Ok(Some(entry.value().clone())),
            Ok(None) => Ok(None),
            Err(e) => Err(PatternError::Config(ConfigError::Invalid {
                field: "cache".to_string(),
                reason: format!("Failed to get memory block from cache: {}", e),
            })),
        }
    }

    /// Cache a memory block value
    pub async fn insert_memory_block(
        &self,
        user_id: i64,
        block_name: &str,
        block_value: String,
    ) -> Result<()> {
        let key = memory_key(user_id, block_name);
        self.memory_blocks.insert(key, Arc::new(block_value));
        Ok(())
    }

    /// Update a memory block value (write-through cache)
    pub async fn update_memory_block_value(
        &self,
        user_id: i64,
        block_name: &str,
        new_value: String,
    ) -> Result<()> {
        let key = memory_key(user_id, block_name);
        self.memory_blocks.insert(key, Arc::new(new_value));
        Ok(())
    }

    /// Remove a memory block from cache
    pub async fn remove_memory_block(
        &self,
        user_id: i64,
        block_name: &str,
    ) -> Result<Option<Arc<String>>> {
        let key = memory_key(user_id, block_name);
        let old_value = self.get_memory_block(user_id, block_name).await?;
        self.memory_blocks.remove(&key);
        Ok(old_value)
    }

    // Agent state cache operations

    /// Get a cached agent state
    pub async fn get_agent_state(
        &self,
        agent_id: &str,
    ) -> Result<Option<Arc<letta::types::AgentState>>> {
        match self.agent_states.get(&agent_id.to_string()).await {
            Ok(Some(entry)) => Ok(Some(entry.value().clone())),
            Ok(None) => Ok(None),
            Err(e) => Err(PatternError::Config(ConfigError::Invalid {
                field: "cache".to_string(),
                reason: format!("Failed to get agent state from cache: {}", e),
            })),
        }
    }

    /// Cache an agent state
    pub async fn insert_agent_state(
        &self,
        agent_id: String,
        agent_state: letta::types::AgentState,
    ) -> Result<()> {
        self.agent_states.insert(agent_id, Arc::new(agent_state));
        Ok(())
    }

    /// Remove an agent state from cache
    pub async fn remove_agent_state(
        &self,
        agent_id: &str,
    ) -> Result<Option<Arc<letta::types::AgentState>>> {
        let old_value = self.get_agent_state(agent_id).await?;
        self.agent_states.remove(agent_id);
        Ok(old_value)
    }

    // Cache management

    /// Clear all caches for a specific user
    pub async fn clear_user_cache(&self, _user_id: i64) -> Result<()> {
        // Note: Foyer doesn't provide a way to clear by prefix,
        // so we'd need to track keys separately if we need this functionality
        warn!("clear_user_cache not fully implemented - foyer doesn't support prefix deletion");
        Ok(())
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        // Note: These are placeholder values - foyer's actual metrics API may differ
        CacheStats {
            agents_in_memory: 0,
            agents_on_disk: 0,
            groups_in_memory: 0,
            groups_on_disk: 0,
            memory_blocks_in_memory: 0,
            memory_blocks_on_disk: 0,
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub agents_in_memory: usize,
    pub agents_on_disk: usize,
    pub groups_in_memory: usize,
    pub groups_on_disk: usize,
    pub memory_blocks_in_memory: usize,
    pub memory_blocks_on_disk: usize,
}

// Key generation functions

/// Generate a consistent key for agent cache
fn agent_key(user_id: i64, agent_type: &str) -> String {
    format!("u{}:a:{}", user_id, agent_type)
}

/// Generate a consistent key for group cache
fn group_key(user_id: i64, group_name: &str) -> String {
    format!("u{}:g:{}", user_id, group_name)
}

/// Generate a consistent key for memory block cache
fn memory_key(user_id: i64, block_name: &str) -> String {
    format!("u{}:m:{}", user_id, block_name)
}

/// Generate a consistent key for message history cache
#[allow(dead_code)]
fn message_history_key(agent_id: &str) -> String {
    format!("msg:{}", agent_id)
}

/// Calculate a hash for a string (for comparing system prompts)
pub fn hash_string(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_cache_creation() {
        let temp_dir = TempDir::new().unwrap();
        let cache = PatternCache::new(temp_dir.path()).await.unwrap();

        // Should create subdirectories
        assert!(temp_dir.path().join("agents").exists());
        assert!(temp_dir.path().join("groups").exists());
        assert!(temp_dir.path().join("memory").exists());
        assert!(temp_dir.path().join("agent_states").exists());
    }

    #[tokio::test]
    async fn test_agent_cache_operations() {
        let temp_dir = TempDir::new().unwrap();
        let cache = PatternCache::new(temp_dir.path()).await.unwrap();

        // Insert
        cache
            .insert_agent(1, "pattern", "test-id".to_string(), "hash123".to_string())
            .await
            .unwrap();

        // Get
        let cached = cache.get_agent(1, "pattern").await.unwrap();
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().letta_id, "test-id");

        // Remove
        let removed = cache.remove_agent(1, "pattern").await.unwrap();
        assert!(removed.is_some());

        // Should be gone
        let cached = cache.get_agent(1, "pattern").await.unwrap();
        assert!(cached.is_none());
    }

    #[test]
    fn test_key_generation() {
        assert_eq!(agent_key(123, "pattern"), "u123:a:pattern");
        assert_eq!(group_key(456, "main"), "u456:g:main");
        assert_eq!(memory_key(789, "state"), "u789:m:state");
        assert_eq!(message_history_key("agent-123"), "msg:agent-123");
    }

    #[test]
    fn test_hash_string() {
        let hash1 = hash_string("test prompt");
        let hash2 = hash_string("test prompt");
        let hash3 = hash_string("different prompt");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }
}
