//! Context management tool following Letta/MemGPT patterns

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    Result,
    context::AgentHandle,
    memory::{MemoryPermission, MemoryType},
    tool::AiTool,
};

/// Operation types for context management
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum CoreMemoryOperationType {
    Append,
    Replace,
    Archive,
    LoadFromArchival,
    Swap,
}

/// Input for managing context
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ContextInput {
    /// The operation to perform
    pub operation: CoreMemoryOperationType,

    /// The name/label of the context section (required for append/replace)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Content to append or new content for replace
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    /// For replace: text to search for (must match exactly)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_content: Option<String>,

    /// For replace: replacement text (use empty string to delete)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_content: Option<String>,

    /// For archive/load_from_archival/swap: label of the recall memory
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archival_label: Option<String>,

    /// For swap: name of the context to archive
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_name: Option<String>,

    /// Request another turn after this tool executes
    #[serde(default)]
    pub request_heartbeat: bool,
}

/// Output from context operations
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ContextOutput {
    /// Whether the operation was successful
    pub success: bool,

    /// Message about the operation
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// For read operations, the memory content
    #[serde(default)]
    pub content: serde_json::Value,
}

/// Unified tool for managing context
#[derive(Debug, Clone)]
pub struct ContextTool {
    pub(crate) handle: AgentHandle,
}

#[async_trait]
impl AiTool for ContextTool {
    type Input = ContextInput;
    type Output = ContextOutput;

    fn name(&self) -> &str {
        "context"
    }

    fn description(&self) -> &str {
        "Manage context sections (persona, human, etc). Context is always visible and shapes agent behavior. No need to read - it's already in your messages. Operations: append, replace, archive, load_from_archival, swap.
 - 'append' adds a new chunk of text to the block. avoid duplicate append operations.
 - 'replace' replaces a section of text (old_content is matched and replaced with new content) within a block. this can be used to delete sections.
 - 'archive' swaps an entire block to recall memory (only works on 'working' memory, not 'core', requires permissions)
 - 'load_from_archival' is the reverse, pulling a block from recall memory into working memory for editing/reading
 - 'swap' replaces a working memory with the requested recall memory, by label"
    }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        match params.operation {
            CoreMemoryOperationType::Append => {
                let name = params.name.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "context",
                        "append operation requires 'name' field",
                    )
                })?;
                let content = params.content.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "context",
                        "append operation requires 'content' field",
                    )
                })?;
                self.execute_append(name, content).await
            }
            CoreMemoryOperationType::Replace => {
                let name = params.name.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "context",
                        "replace operation requires 'name' field",
                    )
                })?;
                let old_content = params.old_content.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "context",
                        "replace operation requires 'old_content' field",
                    )
                })?;
                let new_content = params.new_content.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "context",
                        "replace operation requires 'new_content' field",
                    )
                })?;
                self.execute_replace(name, old_content, new_content).await
            }
            CoreMemoryOperationType::Archive => {
                let name = params.name.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "context",
                        "archive operation requires 'name' field",
                    )
                })?;
                self.execute_archive(name, params.archival_label).await
            }
            CoreMemoryOperationType::LoadFromArchival => {
                let archival_label = params.archival_label.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "context",
                        "load_from_archival operation requires 'archival_label' field",
                    )
                })?;
                let name = params.name.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "context",
                        "load_from_archival operation requires 'name' field for destination",
                    )
                })?;
                self.execute_load_from_archival(archival_label, name).await
            }
            CoreMemoryOperationType::Swap => {
                let archive_name = params.archive_name.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "context",
                        "swap operation requires 'archive_name' field",
                    )
                })?;
                let archival_label = params.archival_label.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "context",
                        "swap operation requires 'archival_label' field",
                    )
                })?;
                self.execute_swap(archive_name, archival_label).await
            }
        }
    }

    fn usage_rule(&self) -> Option<&'static str> {
        Some("the conversation will be continued when called")
    }

    fn examples(&self) -> Vec<crate::tool::ToolExample<Self::Input, Self::Output>> {
        vec![
            crate::tool::ToolExample {
                description: "Remember the user's name".to_string(),
                parameters: ContextInput {
                    operation: CoreMemoryOperationType::Append,
                    name: Some("human".to_string()),
                    content: Some("User's name is Alice, prefers to be called Ali.".to_string()),
                    old_content: None,
                    new_content: None,
                    archival_label: None,
                    archive_name: None,
                    request_heartbeat: false,
                },
                expected_output: Some(ContextOutput {
                    success: true,
                    message: Some("Appended 44 characters to context section 'human'".to_string()),
                    content: json!({}),
                }),
            },
            crate::tool::ToolExample {
                description: "Update agent personality".to_string(),
                parameters: ContextInput {
                    operation: CoreMemoryOperationType::Replace,
                    name: Some("persona".to_string()),
                    content: None,
                    old_content: Some("helpful AI assistant".to_string()),
                    new_content: Some("knowledgeable AI companion".to_string()),
                    archival_label: None,
                    archive_name: None,
                    request_heartbeat: false,
                },
                expected_output: Some(ContextOutput {
                    success: true,
                    message: Some("Replaced content in context section 'persona'".to_string()),
                    content: json!({}),
                }),
            },
        ]
    }
}

impl ContextTool {
    async fn execute_append(&self, name: String, content: String) -> Result<ContextOutput> {
        // Check if the block exists first
        if !self.handle.memory.contains_block(&name) {
            return Err(crate::CoreError::memory_not_found(
                &self.handle.agent_id,
                &name,
                self.handle.memory.list_blocks(),
            ));
        }

        // Use alter for atomic update with validation
        let mut validation_result: Option<ContextOutput> = None;

        self.handle.memory.alter_block(&name, |_key, mut block| {
            // Check if this is context
            if block.memory_type == MemoryType::Archival {
                validation_result = Some(ContextOutput {
                    success: false,
                    message: Some(format!(
                        "Block '{}' is not context (type: {:?}). Use `recall` with the insert operation for non-core memories.",
                        name, block.memory_type
                    )),
                    content: json!({}),
                });
                return block;
            }

            // Check permission
            if block.permission < MemoryPermission::Append {
                validation_result = Some(ContextOutput {
                    success: false,
                    message: Some(format!(
                        "Insufficient permission to modify block '{}' (requires Append or higher, has {:?})",
                        name, block.permission
                    )),
                    content: json!({}),
                });
                return block;
            }

            // All checks passed, update the block
            block.value.push_str("\n\n");
            block.value.push_str(&content);
            block.updated_at = chrono::Utc::now();
            block
        });

        // If validation failed, return the error
        if let Some(error_result) = validation_result {
            return Ok(error_result);
        }

        Ok(ContextOutput {
            success: true,
            message: Some(format!(
                "Appended {} characters to context section '{}'",
                content.len(),
                name
            )),
            content: json!({}),
        })
    }

    async fn execute_replace(
        &self,
        name: String,
        old_content: String,
        new_content: String,
    ) -> Result<ContextOutput> {
        // Check if the block exists first
        if !self.handle.memory.contains_block(&name) {
            return Ok(ContextOutput {
                success: false,
                message: Some(format!(
                    "Memory '{}' not found, available blocks follow",
                    name
                )),
                content: serde_json::to_value(self.handle.memory.list_blocks())
                    .unwrap_or(json!({})),
            });
        }

        // Use alter for atomic update with validation
        let mut validation_result: Option<ContextOutput> = None;

        self.handle.memory.alter_block(&name, |_key, mut block| {
            // Check if this is context
            if block.memory_type == MemoryType::Archival {
                validation_result = Some(ContextOutput {
                    success: false,
                    message: Some(format!(
                        "Block '{}' is not context (type: {:?})",
                        name, block.memory_type
                    )),
                    content: json!({}),
                });
                return block;
            }

            // Check permission
            if block.permission < MemoryPermission::ReadWrite {
                validation_result = Some(ContextOutput {
                    success: false,
                    message: Some(format!(
                        "Insufficient permission to replace content in block '{}' (requires ReadWrite or higher)",
                        name
                    )),
                    content: json!({}),
                });
                return block;
            }

            // Check if old content exists
            if !block.value.contains(&old_content) {
                validation_result = Some(ContextOutput {
                    success: false,
                    message: Some(format!(
                        "Content '{}' not found in context section '{}'",
                        old_content, name
                    )),
                    content: json!({}),
                });
                return block;
            }

            // All checks passed, update the block
            block.value = block.value.replace(&old_content, &new_content);
            block.updated_at = chrono::Utc::now();
            block
        });

        // If validation failed, return the error
        if let Some(error_result) = validation_result {
            return Ok(error_result);
        }

        Ok(ContextOutput {
            success: true,
            message: Some(format!("Replaced content in context section '{}'", name)),
            content: json!({}),
        })
    }

    async fn execute_archive(
        &self,
        name: String,
        archival_label: Option<String>,
    ) -> Result<ContextOutput> {
        // Check if the block exists and is context
        let block = match self.handle.memory.get_block(&name) {
            Some(block) => {
                // can't archive blocks you don't have admin access for
                if block.memory_type != MemoryType::Core {
                    return Ok(ContextOutput {
                        success: false,
                        message: Some(format!(
                            "Block '{}' is not context (type: {:?})",
                            name, block.memory_type
                        )),
                        content: json!({}),
                    });
                } else if block.permission < MemoryPermission::Admin {
                    return Ok(ContextOutput {
                        success: false,
                        message: Some(format!(
                            "Not enough permissions to swap out block '{}', requires Admin",
                            name
                        )),
                        content: json!({}),
                    });
                }
                block.clone()
            }
            None => {
                return Ok(ContextOutput {
                    success: false,
                    message: Some(format!("Memory '{}' not found", name)),
                    content: serde_json::to_value(self.handle.memory.list_blocks())
                        .unwrap_or(json!({})),
                });
            }
        };

        // Generate archival label if not provided
        let archival_label = archival_label
            .unwrap_or_else(|| format!("{}_archived_{}", name, chrono::Utc::now().timestamp()));

        // Create the recall memory
        self.handle
            .memory
            .create_block(&archival_label, &block.value)?;

        // Update it to be archival type
        if let Some(mut archival_block) = self.handle.memory.get_block_mut(&archival_label) {
            archival_block.memory_type = MemoryType::Archival;
            archival_block.permission = MemoryPermission::Admin;
            archival_block.description = Some(format!("Archived from context '{}'", name));
        }

        // Remove the context block
        self.handle.memory.remove_block(&name);

        Ok(ContextOutput {
            success: true,
            message: Some(format!(
                "Archived context '{}' to recall memory '{}'",
                name, archival_label
            )),
            content: json!({}),
        })
    }

    async fn execute_load_from_archival(
        &self,
        archival_label: String,
        name: String,
    ) -> Result<ContextOutput> {
        // Check if recall memory exists
        let archival_block = match self.handle.memory.get_block(&archival_label) {
            Some(block) => {
                if block.memory_type != MemoryType::Archival {
                    return Ok(ContextOutput {
                        success: false,
                        message: Some(format!(
                            "Block '{}' is not recall memory (type: {:?})",
                            archival_label, block.memory_type
                        )),
                        content: json!({}),
                    });
                }
                block.clone()
            }
            None => {
                return Ok(ContextOutput {
                    success: false,
                    message: Some(format!(
                        "Archival memory '{}' not found, available blocks follow",
                        archival_label
                    )),
                    // Note: should filter for the right block type
                    content: serde_json::to_value(self.handle.memory.list_blocks().truncate(20))
                        .unwrap_or(json!({})),
                });
            }
        };

        // Check if context slot already exists
        if self.handle.memory.contains_block(&name) {
            return Ok(ContextOutput {
                success: false,
                message: Some(format!(
                    "Core memory '{}' already exists. Use swap operation instead.",
                    name
                )),
                content: json!({}),
            });
        }

        // Create the context block
        self.handle
            .memory
            .create_block(&name, &archival_block.value)?;

        // Update it to be core type
        if let Some(mut core_block) = self.handle.memory.get_block_mut(&name) {
            core_block.memory_type = MemoryType::Core;
            core_block.permission = MemoryPermission::ReadWrite;
            core_block.description =
                Some(format!("Loaded from recall memory '{}'", archival_label));
        }

        // Remove the recall memory block
        self.handle.memory.remove_block(&archival_label);

        Ok(ContextOutput {
            success: true,
            message: Some(format!(
                "Loaded recall memory '{}' into context '{}'",
                archival_label, name
            )),
            content: json!({}),
        })
    }

    async fn execute_swap(
        &self,
        archive_name: String,
        archival_label: String,
    ) -> Result<ContextOutput> {
        // First check both blocks exist and have correct types
        let core_block = match self.handle.memory.get_block(&archive_name) {
            Some(block) => {
                if block.memory_type != MemoryType::Core {
                    return Ok(ContextOutput {
                        success: false,
                        message: Some(format!(
                            "Block '{}' is not context (type: {:?})",
                            archive_name, block.memory_type
                        )),
                        content: json!({}),
                    });
                } else if block.permission <= MemoryPermission::ReadWrite {
                    return Ok(ContextOutput {
                        success: false,
                        message: Some(format!(
                            "Not enough permissions to swap out block '{}', requires at least Read/Write",
                            archive_name
                        )),
                        content: json!({}),
                    });
                }
                block.clone()
            }
            None => {
                return Ok(ContextOutput {
                    success: false,
                    message: Some(format!(
                        "Memory '{}' not found, available blocks follow",
                        archive_name
                    )),
                    // Note: should filter for the right block type
                    content: serde_json::to_value(self.handle.memory.list_blocks().truncate(20))
                        .unwrap_or(json!({})),
                });
            }
        };

        let archival_block = match self.handle.memory.get_block(&archival_label) {
            Some(block) => {
                if block.memory_type == MemoryType::Archival {
                    return Ok(ContextOutput {
                        success: false,
                        message: Some(format!(
                            "Block '{}' is not recall memory (type: {:?})",
                            archival_label, block.memory_type
                        )),
                        content: json!({}),
                    });
                }
                block.clone()
            }
            None => {
                return Ok(ContextOutput {
                    success: false,
                    message: Some(format!("Archival memory '{}' not found", archival_label)),
                    content: serde_json::to_value(self.handle.memory.list_blocks().truncate(20))
                        .unwrap_or(json!({})),
                });
            }
        };

        // Perform the swap atomically
        // First, create a temporary archival block for the context
        let temp_label = format!(
            "{}_swap_temp_{}",
            archive_name,
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        self.handle
            .memory
            .create_block(&temp_label, &core_block.value)?;

        if let Some(mut temp_block) = self.handle.memory.get_block_mut(&temp_label) {
            temp_block.memory_type = MemoryType::Archival;
            temp_block.permission = MemoryPermission::ReadWrite;
            temp_block.description = Some(format!("Swapped out from context '{}'", archive_name));
        }

        // Update the context with archival content
        self.handle
            .memory
            .update_block_value(&archive_name, archival_block.value.clone())?;

        // Remove the original archival block
        self.handle.memory.remove_block(&archival_label);

        // Rename the temporary archival block to the original archival label
        // Since we can't rename directly, create new and remove temp
        self.handle
            .memory
            .create_block(&archival_label, &core_block.value)?;
        if let Some(mut new_archival) = self.handle.memory.get_block_mut(&archival_label) {
            new_archival.memory_type = MemoryType::Archival;
            new_archival.permission = MemoryPermission::ReadWrite;
            new_archival.description = Some(format!("Swapped out from context '{}'", archive_name));
        }
        self.handle.memory.remove_block(&temp_label);

        Ok(ContextOutput {
            success: true,
            message: Some(format!(
                "Swapped context '{}' with recall memory '{}'",
                archive_name, archival_label
            )),
            content: json!({}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UserId, memory::Memory};

    #[tokio::test]
    async fn test_context_append() {
        let memory = Memory::with_owner(&UserId::generate());

        // Create a context block
        memory
            .create_block("human", "The user is interested in AI.")
            .unwrap();
        if let Some(mut block) = memory.get_block_mut("human") {
            block.memory_type = MemoryType::Core;
            block.permission = MemoryPermission::ReadWrite;
        }

        let handle = AgentHandle::test_with_memory(memory.clone());

        let tool = ContextTool { handle };

        // Test appending
        let result = tool
            .execute(ContextInput {
                operation: CoreMemoryOperationType::Append,
                name: Some("human".to_string()),
                content: Some("They work in healthcare.".to_string()),
                old_content: None,
                new_content: None,
                archival_label: None,
                archive_name: None,
                request_heartbeat: false,
            })
            .await
            .unwrap();

        assert!(result.success);

        // Verify the append
        let block = memory.get_block("human").unwrap();
        assert_eq!(
            block.value,
            "The user is interested in AI.\n\nThey work in healthcare."
        );
    }
}
