//! Recall storage management tool following Letta/MemGPT patterns

use std::fmt::Debug;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Result,
    context::AgentHandle,
    memory::{MemoryPermission, MemoryType},
    tool::{AiTool, ExecutionMeta},
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
    // request_heartbeat handled via ExecutionMeta injection; field removed
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
pub struct RecallTool {
    pub(crate) handle: AgentHandle,
}

#[async_trait]
impl AiTool for RecallTool {
    type Input = RecallInput;
    type Output = RecallOutput;

    fn name(&self) -> &str {
        "recall"
    }

    fn description(&self) -> &str {
        "Manage long-term recall storage. Recall memories are not always visible in context. Operations: insert, append, read (by label), delete.
 - 'insert' creates a new recall memory with the provided content
 - 'append' appends the provided content to the recall memory with the specified label
 - 'read' reads out the contents of the recall block with the specified label
 - 'delete' removes the recall memory with the specified label"
    }

    async fn execute(
        &self,
        params: Self::Input,
        _meta: &crate::tool::ExecutionMeta,
    ) -> Result<Self::Output> {
        match params.operation {
            ArchivalMemoryOperationType::Insert => {
                let content = params.content.ok_or_else(|| {
                    crate::CoreError::tool_exec_msg(
                        "recall",
                        serde_json::json!({"operation":"insert"}),
                        "insert operation requires 'content' field",
                    )
                })?;
                self.execute_insert(content, params.label).await
            }
            ArchivalMemoryOperationType::Append => {
                let label = params.label.ok_or_else(|| {
                    crate::CoreError::tool_exec_msg(
                        "recall",
                        serde_json::json!({"operation":"append"}),
                        "append operation requires 'label' field",
                    )
                })?;
                let content = params.content.ok_or_else(|| {
                    crate::CoreError::tool_exec_msg(
                        "recall",
                        serde_json::json!({"operation":"append"}),
                        "append operation requires 'content' field",
                    )
                })?;
                self.execute_append(label, content).await
            }
            ArchivalMemoryOperationType::Read => {
                let label = params.label.ok_or_else(|| {
                    crate::CoreError::tool_exec_msg(
                        "recall",
                        serde_json::json!({"operation":"read"}),
                        "read operation requires 'label' field",
                    )
                })?;
                self.execute_read(label).await
            }
            ArchivalMemoryOperationType::Delete => {
                let label = params.label.ok_or_else(|| {
                    crate::CoreError::tool_exec_msg(
                        "recall",
                        serde_json::json!({"operation":"delete"}),
                        "delete operation requires 'label' field",
                    )
                })?;
                self.execute_delete(label).await
            }
        }
    }

    fn usage_rule(&self) -> Option<&'static str> {
        Some("the conversation will be continued when called")
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

impl RecallTool {
    async fn execute_insert(&self, content: String, label: Option<String>) -> Result<RecallOutput> {
        // Generate label if not provided
        let label = label.unwrap_or_else(|| format!("archival_{}", chrono::Utc::now().timestamp()));

        tracing::info!(
            "Recall insert operation for label '{}' (content: {} chars)",
            label,
            content.len()
        );

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
        tracing::info!("Using in-memory insert for recall label '{}'", label);

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
        tracing::info!("Created memory block '{}' in memory", label);

        // Update it to be archival type with appropriate permissions using alter_block
        self.handle.memory.alter_block(&label, |_key, mut block| {
            block.memory_type = MemoryType::Archival;
            block.permission = MemoryPermission::ReadWrite;
            block
        });
        tracing::info!("Updated block '{}' to Archival type", label);

        Ok(RecallOutput {
            success: true,
            message: Some(format!("Created recall memory '{}'", label)),
            results: vec![],
        })
    }

    async fn execute_read(&self, label: String) -> Result<RecallOutput> {
        if let Ok(Some(memory)) = self.handle.get_archival_memory_by_label(&label).await {
            Ok(RecallOutput {
                success: true,
                message: Some(format!("Found recall memory '{}'", label)),
                results: vec![ArchivalSearchResult {
                    label,
                    content: memory.value,
                    created_at: memory.created_at,
                    updated_at: memory.updated_at,
                }],
            })
        } else {
            Ok(RecallOutput {
                success: false,
                message: Some(format!("Couldn't find recall memory '{}'", label)),
                results: vec![],
            })
        }
    }

    async fn execute_delete(&self, label: String) -> Result<RecallOutput> {
        // Check if block exists and get type
        let block_type = if let Some(block) = self.handle.memory.get_block(&label) {
            let memory_type = block.memory_type;
            if block.permission != MemoryPermission::Admin {
                return Ok(RecallOutput {
                    success: false,
                    message: Some(format!(
                        "Insufficient permission to delete block '{}' (requires Admin, has {:?})",
                        label, block.permission
                    )),
                    results: vec![],
                });
            }
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
        tracing::info!(
            "Recall append operation for label '{}' (content: {} chars)",
            label,
            content.len()
        );

        // Check if the block exists first
        if !self.handle.memory.contains_block(&label) {
            tracing::warn!("Append failed: block '{}' not found", label);
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

            if block.permission < MemoryPermission::Append {
                validation_result = Some(RecallOutput {
                    success: false,
                    message: Some(format!(
                        "Insufficient permission to append to block '{}' (requires Append or higher, has {:?})",
                        label, block.permission
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

        // Get the updated block to show a preview
        let preview_info = self
            .handle
            .memory
            .get_block(&label)
            .map(|block| {
                let char_count = block.value.chars().count();

                // Show the last part of the content (where the append happened)
                let preview_chars = 200; // Show last 200 chars
                let content_preview = if block.value.len() > preview_chars {
                    format!(
                        "...{}",
                        &block.value[block.value.len().saturating_sub(preview_chars)..]
                    )
                } else {
                    block.value.clone()
                };

                (char_count, content_preview)
            })
            .unwrap_or((0, String::new()));

        Ok(RecallOutput {
            success: true,
            message: Some(format!(
                "Successfully appended {} characters to recall memory '{}'. The memory now contains {} total characters. Preview: {}",
                content.len(),
                label,
                preview_info.0,
                preview_info.1
            )),
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
        let memory = Memory::with_owner(&UserId::generate());
        let handle = AgentHandle::test_with_memory(memory.clone());

        let tool = RecallTool { handle };

        // Test inserting
        let result = tool
            .execute(RecallInput {
                operation: ArchivalMemoryOperationType::Insert,
                content: Some("The user's favorite color is blue.".to_string()),
                label: Some("user_preferences".to_string()),
                request_heartbeat: false,
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
                request_heartbeat: false,
            })
            .await
            .unwrap();

        assert!(result.success);
        // Message format has changed to include more details
        assert!(
            result
                .message
                .as_ref()
                .unwrap()
                .contains("Successfully appended")
        );
        assert!(
            result
                .message
                .as_ref()
                .unwrap()
                .contains("user_preferences")
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
        let memory = Memory::with_owner(&UserId::generate());

        // Create an archival block
        memory
            .create_block("to_delete", "Temporary information")
            .unwrap();
        if let Some(mut block) = memory.get_block_mut("to_delete") {
            block.memory_type = MemoryType::Archival;
            block.permission = MemoryPermission::Admin;
        }

        let handle = AgentHandle::test_with_memory(memory.clone());

        let tool = RecallTool { handle };

        // Test deleting
        let result = tool
            .execute(RecallInput {
                operation: ArchivalMemoryOperationType::Delete,
                content: None,
                label: Some("to_delete".to_string()),
                request_heartbeat: false,
            })
            .await
            .unwrap();

        assert!(result.success);
        assert!(memory.get_block("to_delete").is_none());
    }

    #[tokio::test]
    async fn test_cannot_delete_non_archival() {
        let memory = Memory::with_owner(&UserId::generate());

        // Create a core memory block with Admin permission
        memory
            .create_block("core_block", "Core information")
            .unwrap();
        if let Some(mut block) = memory.get_block_mut("core_block") {
            // Default type is Core, but set Admin permission so we test the right error
            block.permission = MemoryPermission::Admin;
        }

        let handle = AgentHandle::test_with_memory(memory.clone());

        let tool = RecallTool { handle };

        // Try to delete a core memory block
        let result = tool
            .execute(RecallInput {
                operation: ArchivalMemoryOperationType::Delete,
                content: None,
                label: Some("core_block".to_string()),
                request_heartbeat: false,
            })
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.message.unwrap().contains("not recall memory"));
        // Block should still exist
        assert!(memory.get_block("core_block").is_some());
    }
}
