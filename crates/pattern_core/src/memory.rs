use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;

use crate::Result;

/// A memory block that can be stored and retrieved
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub content: String,
    pub metadata: MemoryMetadata,
    pub embedding: Option<Vec<f32>>,
}

/// Metadata associated with a memory block
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryMetadata {
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub access_count: usize,
    pub last_accessed: Option<chrono::DateTime<chrono::Utc>>,
    pub tags: Vec<String>,
    pub source: Option<String>,
    pub confidence: Option<f32>,
    pub custom: serde_json::Value,
}

/// A single memory block with versioning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBlock {
    pub key: String,
    pub current: Memory,
    pub history: Vec<MemoryVersion>,
    pub max_versions: usize,
}

/// A historical version of a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryVersion {
    pub version: usize,
    pub memory: Memory,
    pub changed_at: chrono::DateTime<chrono::Utc>,
    pub changed_by: Option<String>,
}

/// A store for managing agent memories
#[derive(Debug, Clone)]
pub struct MemoryStore {
    blocks: HashMap<String, MemoryBlock>,
    capacity: Option<usize>,
    eviction_policy: EvictionPolicy,
}

/// Policy for evicting memories when capacity is reached
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvictionPolicy {
    /// Least Recently Used
    LRU,
    /// Least Frequently Used
    LFU,
    /// First In First Out
    FIFO,
    /// Keep high confidence memories
    ConfidenceBased,
}

impl Memory {
    pub fn new(content: impl Into<String>) -> Self {
        let now = chrono::Utc::now();
        Self {
            content: content.into(),
            metadata: MemoryMetadata {
                created_at: now,
                updated_at: now,
                ..Default::default()
            },
            embedding: None,
        }
    }

    pub fn with_embedding(mut self, embedding: Vec<f32>) -> Self {
        self.embedding = Some(embedding);
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.metadata.tags = tags;
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.metadata.source = Some(source.into());
        self
    }

    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.metadata.confidence = Some(confidence);
        self
    }
}

impl MemoryBlock {
    pub fn new(key: impl Into<String>, memory: Memory) -> Self {
        Self {
            key: key.into(),
            current: memory,
            history: Vec::new(),
            max_versions: 10,
        }
    }

    pub fn update(&mut self, new_memory: Memory, changed_by: Option<String>) {
        // Move current to history
        let version = MemoryVersion {
            version: self.history.len() + 1,
            memory: self.current.clone(),
            changed_at: chrono::Utc::now(),
            changed_by,
        };

        self.history.push(version);

        // Trim history if needed
        if self.history.len() > self.max_versions {
            self.history.remove(0);
        }

        self.current = new_memory;
    }

    pub fn get_version(&self, version: usize) -> Option<&Memory> {
        if version == 0 {
            Some(&self.current)
        } else {
            self.history
                .iter()
                .find(|v| v.version == version)
                .map(|v| &v.memory)
        }
    }
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            blocks: HashMap::new(),
            capacity: None,
            eviction_policy: EvictionPolicy::LRU,
        }
    }

    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = Some(capacity);
        self
    }

    pub fn with_eviction_policy(mut self, policy: EvictionPolicy) -> Self {
        self.eviction_policy = policy;
        self
    }

    pub fn get(&mut self, key: &str) -> Option<&Memory> {
        self.blocks.get_mut(key).map(|block| {
            block.current.metadata.access_count += 1;
            block.current.metadata.last_accessed = Some(chrono::Utc::now());
            &block.current
        })
    }

    pub fn set(&mut self, key: impl Into<String>, memory: Memory) -> Result<()> {
        let key = key.into();

        // Check capacity and evict if needed
        if let Some(capacity) = self.capacity {
            if self.blocks.len() >= capacity && !self.blocks.contains_key(&key) {
                self.evict()?;
            }
        }

        if let Some(block) = self.blocks.get_mut(&key) {
            block.update(memory, None);
        } else {
            self.blocks
                .insert(key.clone(), MemoryBlock::new(key, memory));
        }

        Ok(())
    }

    pub fn remove(&mut self, key: &str) -> Option<MemoryBlock> {
        self.blocks.remove(key)
    }

    pub fn keys(&self) -> Vec<&String> {
        self.blocks.keys().collect()
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    fn evict(&mut self) -> Result<()> {
        match self.eviction_policy {
            EvictionPolicy::LRU => {
                // Find least recently accessed
                let key_to_remove = self
                    .blocks
                    .iter()
                    .min_by_key(|(_, block)| block.current.metadata.last_accessed)
                    .map(|(k, _)| k.clone());

                if let Some(key) = key_to_remove {
                    self.blocks.remove(&key);
                }
            }
            EvictionPolicy::LFU => {
                // Find least frequently accessed
                let key_to_remove = self
                    .blocks
                    .iter()
                    .min_by_key(|(_, block)| block.current.metadata.access_count)
                    .map(|(k, _)| k.clone());

                if let Some(key) = key_to_remove {
                    self.blocks.remove(&key);
                }
            }
            EvictionPolicy::FIFO => {
                // Find oldest
                let key_to_remove = self
                    .blocks
                    .iter()
                    .min_by_key(|(_, block)| block.current.metadata.created_at)
                    .map(|(k, _)| k.clone());

                if let Some(key) = key_to_remove {
                    self.blocks.remove(&key);
                }
            }
            EvictionPolicy::ConfidenceBased => {
                // Remove lowest confidence
                let key_to_remove = self
                    .blocks
                    .iter()
                    .filter(|(_, block)| block.current.metadata.confidence.is_some())
                    .min_by(|(_, a), (_, b)| {
                        a.current
                            .metadata
                            .confidence
                            .partial_cmp(&b.current.metadata.confidence)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(k, _)| k.clone());

                if let Some(key) = key_to_remove {
                    self.blocks.remove(&key);
                }
            }
        }

        Ok(())
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_creation() {
        let memory = Memory::new("test content")
            .with_tags(vec!["test".to_string(), "example".to_string()])
            .with_confidence(0.9);

        assert_eq!(memory.content, "test content");
        assert_eq!(memory.metadata.tags, vec!["test", "example"]);
        assert_eq!(memory.metadata.confidence, Some(0.9));
    }

    #[test]
    fn test_memory_block_versioning() {
        let memory1 = Memory::new("version 1");
        let mut block = MemoryBlock::new("test_key", memory1);

        let memory2 = Memory::new("version 2");
        block.update(memory2.clone(), Some("test_user".to_string()));

        assert_eq!(block.current.content, "version 2");
        assert_eq!(block.history.len(), 1);
        assert_eq!(block.history[0].memory.content, "version 1");
    }

    #[test]
    fn test_memory_store() {
        let mut store = MemoryStore::new().with_capacity(2);

        store.set("key1", Memory::new("content 1")).unwrap();
        store.set("key2", Memory::new("content 2")).unwrap();

        assert_eq!(store.len(), 2);

        // This should trigger eviction
        store.set("key3", Memory::new("content 3")).unwrap();
        assert_eq!(store.len(), 2);
    }
}
