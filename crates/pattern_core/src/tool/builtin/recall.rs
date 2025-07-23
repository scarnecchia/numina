//! Recall storage management tool following Letta/MemGPT patterns

use std::fmt::Debug;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Result,
    context::AgentHandle,
    memory::{MemoryPermission, MemoryType},
    tool::AiTool,
};

/// Operation types for recall storage management
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum ArchivalMemoryOperationType {
    Insert,
    Append,
    Read,
    Delete,
}

/// Input for managing recall storage
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct RecallInput {
    /// The operation to perform
    pub operation: ArchivalMemoryOperationType,

    /// For insert/read/delete: label for the memory (insert defaults to "archival_<timestamp>")
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// For insert: content to store in recall storage
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Output from recall storage operations
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RecallOutput {
    /// Whether the operation was successful
    pub success: bool,

    /// Message about the operation
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// For search operations, the matching entries
    #[schemars(default)]
    pub results: Vec<ArchivalSearchResult>,
}

/// A single search result from recall storage
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ArchivalSearchResult {
    /// Label of the memory block
    pub label: String,
    /// Content of the memory
    pub content: String,
    /// When the memory was created
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When the memory was last updated
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Unified tool for managing recall storage
#[derive(Debug, Clone)]
pub struct RecallTool<C: surrealdb::Connection + Clone> {
    pub(crate) handle: AgentHandle<C>,
}

#[async_trait]
impl<C: surrealdb::Connection + Clone + Debug> AiTool for RecallTool<C> {
    type Input = RecallInput;
    type Output = RecallOutput;

    fn name(&self) -> &str {
        "recall"
    }

    fn description(&self) -> &str {
        "Manage long-term recall storage. Recall memories are not always visible in context. Operations: insert, append, read (by label), delete."
    }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        match params.operation {
            ArchivalMemoryOperationType::Insert => {
                let content = params.content.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "recall",
                        "insert operation requires 'content' field",
                    )
                })?;
                self.execute_insert(content, params.label).await
            }
            ArchivalMemoryOperationType::Append => {
                let label = params.label.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "recall",
                        "append operation requires 'label' field",
                    )
                })?;
                let content = params.content.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "recall",
                        "append operation requires 'content' field",
                    )
                })?;
                self.execute_append(label, content).await
            }
            ArchivalMemoryOperationType::Read => {
                let label = params.label.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "recall",
                        "read operation requires 'label' field",
                    )
                })?;
                self.execute_read(label).await
            }
            ArchivalMemoryOperationType::Delete => {
                let label = params.label.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "recall",
                        "delete operation requires 'label' field",
                    )
                })?;
                self.execute_delete(label).await
            }
        }
    }

    fn usage_rule(&self) -> Option<&'static str> {
        Some("requires continuing your response when called")
    }

    fn examples(&self) -> Vec<crate::tool::ToolExample<Self::Input, Self::Output>> {
        vec![
            crate::tool::ToolExample {
                description: "Store important information for later".to_string(),
                parameters: RecallInput {
                    operation: ArchivalMemoryOperationType::Insert,
                    content: Some(
                        "User mentioned they have a dog named Max who likes to play fetch."
                            .to_string(),
                    ),
                    label: None,
                },
                expected_output: Some(RecallOutput {
                    success: true,
                    message: Some("Created recall memory 'archival_1234567890'".to_string()),
                    results: vec![],
                }),
            },
            crate::tool::ToolExample {
                description: "Add more information to existing recall memory".to_string(),
                parameters: RecallInput {
                    operation: ArchivalMemoryOperationType::Append,
                    content: Some("Max is a golden retriever.".to_string()),
                    label: Some("archival_1234567890".to_string()),
                },
                expected_output: Some(RecallOutput {
                    success: true,
                    message: Some("Appended to recall memory 'archival_1234567890'".to_string()),
                    results: vec![],
                }),
            },
        ]
    }
}

impl<C: surrealdb::Connection + Clone> RecallTool<C> {
    async fn execute_insert(&self, content: String, label: Option<String>) -> Result<RecallOutput> {
        // Generate label if not provided
        let label = label.unwrap_or_else(|| format!("archival_{}", chrono::Utc::now().timestamp()));

        // Try to use database if available, fall back to in-memory
        if self.handle.has_db_connection() {
            match self.handle.insert_archival_memory(&label, &content).await {
                Ok(_) => Ok(RecallOutput {
                    success: true,
                    message: Some(format!("Created recall memory '{}' in database", label)),
                    results: vec![],
                }),
                Err(e) => {
                    tracing::warn!("Database insert failed, falling back to in-memory: {}", e);
                    self.insert_in_memory(label, content)
                }
            }
        } else {
            self.insert_in_memory(label, content)
        }
    }

    fn insert_in_memory(&self, label: String, content: String) -> Result<RecallOutput> {
        // Check if label already exists
        if self.handle.memory.contains_block(&label) {
            return Ok(RecallOutput {
                success: false,
                message: Some(format!(
                    "Memory block with label '{}' already exists",
                    label
                )),
                results: vec![],
            });
        }

        // Create the archival memory block
        self.handle.memory.create_block(&label, &content)?;

        // Update it to be archival type with appropriate permissions using alter_block
        self.handle.memory.alter_block(&label, |_key, mut block| {
            block.memory_type = MemoryType::Archival;
            block.permission = MemoryPermission::ReadWrite;
            block
        });

        Ok(RecallOutput {
            success: true,
            message: Some(format!("Created recall memory '{}'", label)),
            results: vec![],
        })
    }

    async fn execute_read(&self, label: String) -> Result<RecallOutput> {
        // Fall back to in-memory
        if let Some(block) = self.handle.memory.get_block(&label) {
            // Verify it's archival memory
            if block.memory_type == MemoryType::Archival {
                // Clone what we need and drop the ref immediately
                let result = ArchivalSearchResult {
                    label: block.label.to_string(),
                    content: block.value.clone(),
                    created_at: block.created_at,
                    updated_at: block.updated_at,
                };
                drop(block); // Explicitly drop to release lock

                Ok(RecallOutput {
                    success: true,
                    message: Some(format!("Found recall memory '{}'", label)),
                    results: vec![result],
                })
            } else {
                let memory_type = block.memory_type;
                drop(block); // Explicitly drop to release lock

                Ok(RecallOutput {
                    success: false,
                    message: Some(format!(
                        "Block '{}' exists but is not recall memory (type: {:?})",
                        label, memory_type
                    )),
                    results: vec![],
                })
            }
        } else {
            // Try database first if available
            if self.handle.has_db_connection() {
                // For now, we'll search for the exact label
                match self.handle.search_archival_memories(&label, 10).await {
                    Ok(blocks) => {
                        // Find exact match
                        if let Some(block) = blocks.iter().find(|b| b.label == label) {
                            return Ok(RecallOutput {
                                success: true,
                                message: Some(format!("Found recall memory '{}'", label)),
                                results: vec![ArchivalSearchResult {
                                    label: block.label.to_string(),
                                    content: block.value.clone(),
                                    created_at: block.created_at,
                                    updated_at: block.updated_at,
                                }],
                            });
                        } else {
                            Ok(RecallOutput {
                                success: false,
                                message: Some(format!(
                                    "No recall memory found with label '{}'",
                                    label
                                )),
                                results: vec![],
                            })
                        }
                    }
                    Err(e) => Ok(RecallOutput {
                        success: false,
                        message: Some(format!(
                            "No recall memory found with label '{}' due to database error {}",
                            label, e
                        )),
                        results: vec![],
                    }),
                }
            } else {
                Ok(RecallOutput {
                    success: false,
                    message: Some(format!("No recall memory found with label '{}'", label)),
                    results: vec![],
                })
            }
        }
    }

    async fn execute_delete(&self, label: String) -> Result<RecallOutput> {
        // Check if block exists and get type
        let block_type = if let Some(block) = self.handle.memory.get_block(&label) {
            let memory_type = block.memory_type;
            drop(block); // Release lock immediately
            Some(memory_type)
        } else {
            None
        };

        match block_type {
            Some(MemoryType::Archival) => {
                // Remove the block from memory
                self.handle.memory.remove_block(&label);

                Ok(RecallOutput {
                    success: true,
                    message: Some(format!("Deleted recall memory '{}'", label)),
                    results: vec![],
                })
            }
            Some(memory_type) => Ok(RecallOutput {
                success: false,
                message: Some(format!(
                    "Block '{}' is not recall memory (type: {:?})",
                    label, memory_type
                )),
                results: vec![],
            }),
            None => Ok(RecallOutput {
                success: false,
                message: Some(format!("Archival memory '{}' not found", label)),
                results: vec![],
            }),
        }
    }

    async fn execute_append(&self, label: String, content: String) -> Result<RecallOutput> {
        // Check if the block exists first
        if !self.handle.memory.contains_block(&label) {
            return Ok(RecallOutput {
                success: false,
                message: Some(format!("Archival memory '{}' not found", label)),
                results: vec![],
            });
        }

        // Use alter for atomic update with validation
        let mut validation_result: Option<RecallOutput> = None;

        self.handle.memory.alter_block(&label, |_key, mut block| {
            // Check if this is recall memory
            if block.memory_type != MemoryType::Archival {
                validation_result = Some(RecallOutput {
                    success: false,
                    message: Some(format!(
                        "Block '{}' is not recall memory (type: {:?})",
                        label, block.memory_type
                    )),
                    results: vec![],
                });
                return block;
            }

            // All checks passed, update the block
            block.value.push_str("\n");
            block.value.push_str(&content);
            block.updated_at = chrono::Utc::now();
            block
        });

        // If validation failed, return the error
        if let Some(error_result) = validation_result {
            return Ok(error_result);
        }

        Ok(RecallOutput {
            success: true,
            message: Some(format!("Appended to recall memory '{}'", label)),
            results: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UserId, memory::Memory};

    #[tokio::test]
    async fn test_archival_insert_and_append() {
        let memory = Memory::with_owner(UserId::generate());
        let handle = AgentHandle::test_with_memory(memory.clone());

        let tool = RecallTool { handle };

        // Test inserting
        let result = tool
            .execute(RecallInput {
                operation: ArchivalMemoryOperationType::Insert,
                content: Some("The user's favorite color is blue.".to_string()),
                label: Some("user_preferences".to_string()),
            })
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(
            result.message.as_ref().unwrap(),
            "Created recall memory 'user_preferences'"
        );

        // Verify the block was created with correct type
        {
            let block = memory.get_block("user_preferences").unwrap();
            assert_eq!(block.memory_type, MemoryType::Archival);
            assert_eq!(block.value, "The user's favorite color is blue.");
        } // Block ref dropped here

        // Test appending
        let result = tool
            .execute(RecallInput {
                operation: ArchivalMemoryOperationType::Append,
                content: Some(" They also like the color green.".to_string()),
                label: Some("user_preferences".to_string()),
            })
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(
            result.message.as_ref().unwrap(),
            "Appended to recall memory 'user_preferences'"
        );

        // Verify the append
        {
            let block = memory.get_block("user_preferences").unwrap();
            assert_eq!(
                block.value,
                "The user's favorite color is blue.\n They also like the color green."
            );
        }
    }

    #[tokio::test]
    async fn test_archival_delete() {
        let memory = Memory::with_owner(UserId::generate());

        // Create an archival block
        memory
            .create_block("to_delete", "Temporary information")
            .unwrap();
        if let Some(mut block) = memory.get_block_mut("to_delete") {
            block.memory_type = MemoryType::Archival;
        }

        let handle = AgentHandle::test_with_memory(memory.clone());

        let tool = RecallTool { handle };

        // Test deleting
        let result = tool
            .execute(RecallInput {
                operation: ArchivalMemoryOperationType::Delete,
                content: None,
                label: Some("to_delete".to_string()),
            })
            .await
            .unwrap();

        assert!(result.success);
        assert!(memory.get_block("to_delete").is_none());
    }

    #[tokio::test]
    async fn test_cannot_delete_non_archival() {
        let memory = Memory::with_owner(UserId::generate());

        // Create a core memory block
        memory
            .create_block("core_block", "Core information")
            .unwrap();
        // Default type is Core

        let handle = AgentHandle::test_with_memory(memory.clone());

        let tool = RecallTool { handle };

        // Try to delete a core memory block
        let result = tool
            .execute(RecallInput {
                operation: ArchivalMemoryOperationType::Delete,
                content: None,
                label: Some("core_block".to_string()),
            })
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.message.unwrap().contains("not recall memory"));
        // Block should still exist
        assert!(memory.get_block("core_block").is_some());
    }
}
