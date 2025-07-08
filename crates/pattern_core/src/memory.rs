use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;

use crate::Result;

/// A memory block following the MemGPT pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBlock {
    /// Label/name of the memory block (e.g., "persona", "human")
    pub label: String,

    /// The actual content of the memory block
    pub value: String,

    /// Optional description of what this block is for
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// When this block was last modified
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<DateTime<Utc>>,
}

/// Core memory system for agents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Memory blocks by label
    blocks: HashMap<String, MemoryBlock>,

    /// Maximum characters per block (soft limit)
    char_limit: usize,
}

impl Memory {
    /// Create a new memory system
    pub fn new() -> Self {
        Self {
            blocks: HashMap::new(),
            char_limit: 5000,
        }
    }

    /// Create with a specific character limit
    pub fn with_char_limit(mut self, limit: usize) -> Self {
        self.char_limit = limit;
        self
    }

    /// Create a new memory block
    pub fn create_block(
        &mut self,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<()> {
        let label = label.into();
        let value = value.into();

        let block = MemoryBlock {
            label: label.clone(),
            value,
            description: None,
            last_modified: Some(Utc::now()),
        };

        self.blocks.insert(label, block);
        Ok(())
    }

    /// Builder method to add a block
    pub fn with_block(
        mut self,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self> {
        self.create_block(label, value)?;
        Ok(self)
    }

    /// Get a memory block by label
    pub fn get_block(&self, label: &str) -> Option<&MemoryBlock> {
        self.blocks.get(label)
    }

    /// Get a mutable reference to a memory block
    pub fn get_block_mut(&mut self, label: &str) -> Option<&mut MemoryBlock> {
        self.blocks.get_mut(label)
    }

    /// Update the value of a memory block
    pub fn update_block_value(&mut self, label: &str, value: impl Into<String>) -> Result<()> {
        if let Some(block) = self.blocks.get_mut(label) {
            block.value = value.into();
            block.last_modified = Some(Utc::now());
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
        self.blocks.values().cloned().collect()
    }

    /// List all block labels
    pub fn list_blocks(&self) -> Vec<String> {
        self.blocks.keys().cloned().collect()
    }

    /// Remove a memory block
    pub fn remove_block(&mut self, label: &str) -> Option<MemoryBlock> {
        self.blocks.remove(label)
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryBlock {
    /// Create a new memory block
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            description: None,
            last_modified: Some(Utc::now()),
        }
    }

    /// Set the description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_creation() {
        let mut memory = Memory::new();
        assert_eq!(memory.list_blocks().len(), 0);

        memory.create_block("test", "test content").unwrap();
        assert_eq!(memory.list_blocks().len(), 1);

        let block = memory.get_block("test").unwrap();
        assert_eq!(block.value, "test content");
    }

    #[test]
    fn test_memory_block_versioning() {
        let mut memory = Memory::new();
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
}
