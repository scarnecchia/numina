use chrono::Utc;
use compact_str::CompactString;
use dashmap::DashMap;
use pattern_macros::Entity;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::Debug;
use std::ops::Deref;
use std::ops::DerefMut;
use std::sync::Arc;

use crate::id::MemoryIdType;
use crate::{MemoryId, Result, UserId};

/// A memory block following the MemGPT pattern
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "mem")]
pub struct MemoryBlock {
    /// Unique identifier for this memory block
    pub id: MemoryId,

    /// The user (human) who owns this memory block
    pub owner_id: UserId,

    /// Label identifying this memory block (e.g., "persona", "human", "context")
    pub label: CompactString,

    /// The actual value of the memory block
    pub value: String,

    /// Optional description of what this memory block contains
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Additional metadata for this block
    #[serde(default)]
    pub metadata: serde_json::Value,

    /// The embedding model used to generate embeddings for this block (if any)
    pub embedding_model: Option<String>,

    pub embedding: Option<Vec<f32>>,
    /// When this memory block was created
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// When this memory block was last updated
    pub updated_at: chrono::DateTime<chrono::Utc>,

    /// Whether this memory block is active (false = soft deleted, hidden from agent)
    pub is_active: bool,
}

impl Default for MemoryBlock {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: MemoryId::generate(),
            owner_id: UserId::nil(),
            label: CompactString::new(""),
            value: String::new(),
            description: None,
            metadata: json!({}),
            embedding_model: None,
            embedding: None,
            created_at: now,
            updated_at: now,
            is_active: true,
        }
    }
}

/// Core memory system for agents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Memory blocks by label
    #[serde(skip)]
    blocks: Arc<DashMap<CompactString, MemoryBlock>>,

    /// Maximum characters per block (soft limit)
    char_limit: usize,
    /// The user (human) who owns this memory collection
    pub owner_id: UserId,
}

impl Memory {
    /// Create a new memory system
    pub fn new() -> Self {
        Self {
            blocks: Arc::new(DashMap::new()),
            char_limit: 5000,
            owner_id: UserId::generate(),
        }
    }

    /// Create a new memory system owned by a specific user
    pub fn with_owner(owner_id: UserId) -> Self {
        Self {
            blocks: Arc::new(DashMap::new()),
            char_limit: 5000,
            owner_id,
        }
    }

    /// Create with a specific character limit
    pub fn with_char_limit(mut self, limit: usize) -> Self {
        self.char_limit = limit;
        self
    }

    /// Create a new memory block
    pub fn create_block(
        &self,
        label: impl Into<CompactString>,
        value: impl Into<String>,
    ) -> Result<()> {
        let label = label.into();
        let value = value.into();

        let block = MemoryBlock {
            id: MemoryId::generate(),
            owner_id: self.owner_id.clone(),
            label: label.clone(),
            value,
            description: None,
            embedding_model: None,
            updated_at: Utc::now(),
            is_active: true,
            metadata: json!({}),
            created_at: Utc::now(),
            ..Default::default()
        };

        self.blocks.insert(label, block);
        Ok(())
    }

    /// Builder method to add a block
    pub fn with_block(
        self,
        label: impl Into<CompactString>,
        value: impl Into<String>,
    ) -> Result<Self> {
        self.create_block(label, value)?;
        Ok(self)
    }

    /// Get a memory block by label
    pub fn get_block(&self, label: &str) -> Option<impl Deref<Target = MemoryBlock> + use<'_>> {
        self.blocks.get(label)
    }

    /// Get a mutable reference to a memory block
    pub fn get_block_mut(&self, label: &str) -> Option<impl DerefMut<Target = MemoryBlock>> {
        self.blocks.get_mut(label)
    }

    /// Update the value of a memory block
    pub fn update_block_value(&self, label: &str, value: impl Into<String>) -> Result<()> {
        if let Some(mut block) = self.blocks.get_mut(label) {
            block.value = value.into();
            block.updated_at = Utc::now();
            Ok(())
        } else {
            Err(crate::CoreError::MemoryNotFound {
                agent_id: "unknown".to_string(),
                block_name: label.to_string(),
                available_blocks: self.list_blocks(),
            })
        }
    }

    /// Get all memory blocks
    pub fn get_all_blocks(&self) -> Vec<MemoryBlock> {
        self.blocks.iter().map(|e| e.value().clone()).collect()
    }

    /// List all block labels
    pub fn list_blocks(&self) -> Vec<CompactString> {
        self.blocks.iter().map(|e| e.key().clone()).collect()
    }

    /// Remove a memory block
    pub fn remove_block(&self, label: &str) -> Option<MemoryBlock> {
        self.blocks.remove(label).map(|e| e.1)
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryBlock {
    /// Create a new memory block with auto-generated ID and owner
    pub fn new(label: impl Into<CompactString>, value: impl Into<String>) -> Self {
        Self {
            id: MemoryId::generate(),
            owner_id: UserId::generate(),
            label: label.into(),
            value: value.into(),
            description: None,

            embedding_model: None,
            updated_at: Utc::now(),
            metadata: json!({}),
            created_at: Utc::now(),
            is_active: true,
            ..Default::default()
        }
    }

    /// Create a new memory block with specific ID and owner
    pub fn owned_with_id(
        id: MemoryId,
        owner_id: UserId,
        label: impl Into<CompactString>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            id,
            owner_id,
            label: label.into(),
            value: value.into(),
            description: None,

            embedding_model: None,
            updated_at: Utc::now(),
            metadata: json!({}),
            created_at: Utc::now(),
            is_active: true,
            ..Default::default()
        }
    }

    /// Create a new memory block owned by a specific user
    pub fn owned(
        owner_id: UserId,
        label: impl Into<CompactString>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            id: MemoryId::generate(),
            owner_id,
            label: label.into(),
            value: value.into(),
            description: None,
            updated_at: Utc::now(),

            embedding_model: None,
            metadata: json!({}),
            created_at: Utc::now(),
            is_active: true,
            ..Default::default()
        }
    }

    /// Set the description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the embedding model name for this block
    pub fn with_embedding_model(mut self, embedding_model: impl Into<String>) -> Self {
        self.embedding_model = Some(embedding_model.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_creation() {
        let memory = Memory::new();
        assert_eq!(memory.list_blocks().len(), 0);

        memory.create_block("test", "test content").unwrap();
        assert_eq!(memory.list_blocks().len(), 1);

        let block = memory.get_block("test").unwrap();
        assert_eq!(block.value, "test content");
    }

    #[test]
    fn test_memory_block_versioning() {
        let memory = Memory::new();
        memory.create_block("persona", "I am a helpful AI").unwrap();
        memory
            .create_block("human", "The user's name is Alice")
            .unwrap();

        assert_eq!(memory.list_blocks().len(), 2);

        memory
            .update_block_value("persona", "I am a very helpful AI assistant")
            .unwrap();

        let persona_block = memory.get_block("persona").unwrap();
        assert_eq!(persona_block.value, "I am a very helpful AI assistant");
    }

    #[test]
    fn test_memory_block_with_description() {
        let block = MemoryBlock::new("test", "content").with_description("Test block");

        assert_eq!(block.label, "test");
        assert_eq!(block.value, "content");
        assert_eq!(block.description, Some("Test block".to_string()));
    }

    #[tokio::test]
    async fn test_memory_block_entity_operations() {
        use crate::db::client;

        // Initialize test database
        let db = client::create_test_db().await.unwrap();

        // Create a memory block
        let block = MemoryBlock::new("persona", "I am a helpful AI assistant")
            .with_description("Agent persona")
            .with_embedding_model("text-embedding-3-small");

        // Store it using entity system
        let stored = block.store_with_relations(&db).await.unwrap();
        assert_eq!(stored.label.as_str(), "persona");
        assert_eq!(stored.value, "I am a helpful AI assistant");
        assert_eq!(stored.description, Some("Agent persona".to_string()));
        assert_eq!(
            stored.embedding_model,
            Some("text-embedding-3-small".to_string())
        );

        // Load it back
        let loaded = MemoryBlock::load_with_relations(&db, stored.id)
            .await
            .unwrap()
            .expect("Memory block should exist");

        assert_eq!(loaded.label.as_str(), "persona");
        assert_eq!(loaded.value, "I am a helpful AI assistant");
        assert_eq!(loaded.description, Some("Agent persona".to_string()));

        // Test that CompactString conversion works correctly
        assert_eq!(loaded.label, stored.label);
    }
}
