//! Unified core memory management tool following Letta/MemGPT patterns

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

/// Operation types for core memory management
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum CoreMemoryOperationType {
    Append,
    Replace,
}

/// Input for managing core memory
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ManageCoreMemoryInput {
    /// The operation to perform
    pub operation: CoreMemoryOperationType,

    /// The name/label of the core memory section (required for append/replace)
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

    /// For append: separator between existing and new content (default: "\n")
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub separator: Option<String>,
}

/// Output from core memory operations
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ManageCoreMemoryOutput {
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

/// Unified tool for managing core memory
#[derive(Debug, Clone)]
pub struct ManageCoreMemoryTool<C: surrealdb::Connection + Clone> {
    pub(crate) handle: AgentHandle<C>,
}

#[async_trait]
impl<C: surrealdb::Connection + Clone + std::fmt::Debug> AiTool for ManageCoreMemoryTool<C> {
    type Input = ManageCoreMemoryInput;
    type Output = ManageCoreMemoryOutput;

    fn name(&self) -> &str {
        "manage_core_memory"
    }

    fn description(&self) -> &str {
        "Manage core memory sections (persona, human, etc). Core memory is always in context and shapes agent behavior. No need to read - it's already visible. Operations: append, replace."
    }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        match params.operation {
            CoreMemoryOperationType::Append => {
                let name = params.name.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "manage_core_memory",
                        "append operation requires 'name' field",
                    )
                })?;
                let content = params.content.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "manage_core_memory",
                        "append operation requires 'content' field",
                    )
                })?;
                self.execute_append(name, content, params.separator).await
            }
            CoreMemoryOperationType::Replace => {
                let name = params.name.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "manage_core_memory",
                        "replace operation requires 'name' field",
                    )
                })?;
                let old_content = params.old_content.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "manage_core_memory",
                        "replace operation requires 'old_content' field",
                    )
                })?;
                let new_content = params.new_content.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "manage_core_memory",
                        "replace operation requires 'new_content' field",
                    )
                })?;
                self.execute_replace(name, old_content, new_content).await
            }
        }
    }

    fn usage_rule(&self) -> Option<&'static str> {
        Some("requires continuing your response when called")
    }

    fn examples(&self) -> Vec<crate::tool::ToolExample<Self::Input, Self::Output>> {
        vec![
            crate::tool::ToolExample {
                description: "Remember the user's name".to_string(),
                parameters: ManageCoreMemoryInput {
                    operation: CoreMemoryOperationType::Append,
                    name: Some("human".to_string()),
                    content: Some("User's name is Alice, prefers to be called Ali.".to_string()),
                    separator: None,
                    old_content: None,
                    new_content: None,
                },
                expected_output: Some(ManageCoreMemoryOutput {
                    success: true,
                    message: Some(
                        "Appended 44 characters to core memory section 'human'".to_string(),
                    ),
                    content: json!({}),
                }),
            },
            crate::tool::ToolExample {
                description: "Update agent personality".to_string(),
                parameters: ManageCoreMemoryInput {
                    operation: CoreMemoryOperationType::Replace,
                    name: Some("persona".to_string()),
                    content: None,
                    separator: None,
                    old_content: Some("helpful AI assistant".to_string()),
                    new_content: Some("knowledgeable AI companion".to_string()),
                },
                expected_output: Some(ManageCoreMemoryOutput {
                    success: true,
                    message: Some("Replaced content in core memory section 'persona'".to_string()),
                    content: json!({}),
                }),
            },
        ]
    }
}

impl<C: surrealdb::Connection + Clone> ManageCoreMemoryTool<C> {
    async fn execute_append(
        &self,
        name: String,
        content: String,
        separator: Option<String>,
    ) -> Result<ManageCoreMemoryOutput> {
        let separator = separator.as_deref().unwrap_or("\n");

        // Check if the block exists first
        if !self.handle.memory.contains_block(&name) {
            return Err(crate::CoreError::memory_not_found(
                &self.handle.agent_id,
                &name,
                self.handle.memory.list_blocks(),
            ));
        }

        // Use alter for atomic update with validation
        let mut validation_result: Option<ManageCoreMemoryOutput> = None;

        self.handle.memory.alter_block(&name, |_key, mut block| {
            // Check if this is core memory
            if block.memory_type != MemoryType::Core {
                validation_result = Some(ManageCoreMemoryOutput {
                    success: false,
                    message: Some(format!(
                        "Block '{}' is not core memory (type: {:?}). Use archival_memory_insert for non-core memories.",
                        name, block.memory_type
                    )),
                    content: json!({}),
                });
                return block;
            }

            // Check permission
            if block.permission < MemoryPermission::Append {
                validation_result = Some(ManageCoreMemoryOutput {
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
            block.value.push_str(separator);
            block.value.push_str(&content);
            block.updated_at = chrono::Utc::now();
            block
        });

        // If validation failed, return the error
        if let Some(error_result) = validation_result {
            return Ok(error_result);
        }

        Ok(ManageCoreMemoryOutput {
            success: true,
            message: Some(format!(
                "Appended {} characters to core memory section '{}'",
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
    ) -> Result<ManageCoreMemoryOutput> {
        // Check if the block exists first
        if !self.handle.memory.contains_block(&name) {
            return Err(crate::CoreError::memory_not_found(
                &self.handle.agent_id,
                &name,
                self.handle.memory.list_blocks(),
            ));
        }

        // Use alter for atomic update with validation
        let mut validation_result: Option<ManageCoreMemoryOutput> = None;

        self.handle.memory.alter_block(&name, |_key, mut block| {
            // Check if this is core memory
            if block.memory_type != MemoryType::Core {
                validation_result = Some(ManageCoreMemoryOutput {
                    success: false,
                    message: Some(format!(
                        "Block '{}' is not core memory (type: {:?})",
                        name, block.memory_type
                    )),
                    content: json!({}),
                });
                return block;
            }

            // Check permission
            if block.permission < MemoryPermission::ReadWrite {
                validation_result = Some(ManageCoreMemoryOutput {
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
                validation_result = Some(ManageCoreMemoryOutput {
                    success: false,
                    message: Some(format!(
                        "Content '{}' not found in core memory section '{}'",
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

        Ok(ManageCoreMemoryOutput {
            success: true,
            message: Some(format!(
                "Replaced content in core memory section '{}'",
                name
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
    async fn test_manage_core_memory_append() {
        let memory = Memory::with_owner(UserId::generate());

        // Create a core memory block
        memory
            .create_block("human", "The user is interested in AI.")
            .unwrap();
        if let Some(mut block) = memory.get_block_mut("human") {
            block.memory_type = MemoryType::Core;
            block.permission = MemoryPermission::ReadWrite;
        }

        let handle = AgentHandle::test_with_memory(memory.clone());

        let tool = ManageCoreMemoryTool { handle };

        // Test appending
        let result = tool
            .execute(ManageCoreMemoryInput {
                operation: CoreMemoryOperationType::Append,
                name: Some("human".to_string()),
                content: Some("They work in healthcare.".to_string()),
                separator: Some(" ".to_string()),
                old_content: None,
                new_content: None,
            })
            .await
            .unwrap();

        assert!(result.success);

        // Verify the append
        let block = memory.get_block("human").unwrap();
        assert_eq!(
            block.value,
            "The user is interested in AI. They work in healthcare."
        );
    }
}
