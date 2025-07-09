//! Memory management tools for agents

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Result, context::AgentHandle, tool::AiTool};

/// Input parameters for updating memory
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct UpdateMemoryInput {
    /// The label of the memory block to update
    pub label: String,

    /// The new value for the memory block
    pub value: String,

    /// Optional description for the memory block
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Output from memory update operation
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct UpdateMemoryOutput {
    /// Whether the update was successful
    pub success: bool,

    /// The previous value of the memory block (if it existed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_value: Option<String>,

    /// Any warnings or notes about the update
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Tool for updating agent memory blocks
#[derive(Debug, Clone)]
pub struct UpdateMemoryTool {
    pub(crate) handle: AgentHandle,
}

#[async_trait]
impl AiTool for UpdateMemoryTool {
    type Input = UpdateMemoryInput;
    type Output = UpdateMemoryOutput;

    fn name(&self) -> &str {
        "update_memory"
    }

    fn description(&self) -> &str {
        "Update or create a memory block with a new value. Memory blocks are persistent storage areas that maintain information between conversations."
    }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        // Get the previous value if it exists
        let previous_value = self
            .handle
            .memory
            .get_block(&params.label)
            .map(|block| block.content.clone());

        // Update or create the memory block
        if previous_value.is_some() {
            // Update existing block
            self.handle
                .memory
                .update_block_value(&params.label, params.value)?;
        } else {
            // Create new block
            self.handle
                .memory
                .create_block(&params.label, params.value)?;
        }

        // Update description if provided
        if let Some(description) = params.description {
            if let Some(mut block) = self.handle.memory.get_block_mut(&params.label) {
                block.description = Some(description);
            }
        }

        Ok(UpdateMemoryOutput {
            success: true,
            previous_value: previous_value.clone(),
            message: if previous_value.is_some() {
                None
            } else {
                Some(format!("Created new memory block '{}'", params.label))
            },
        })
    }

    fn examples(&self) -> Vec<crate::tool::ToolExample<Self::Input, Self::Output>> {
        vec![
            crate::tool::ToolExample {
                description: "Update the user's name in memory".to_string(),
                parameters: UpdateMemoryInput {
                    label: "human".to_string(),
                    value: "The user's name is Alice. She prefers to be called Ali.".to_string(),
                    description: Some("Information about the user".to_string()),
                },
                expected_output: Some(UpdateMemoryOutput {
                    success: true,
                    previous_value: Some("The user's name is unknown.".to_string()),
                    message: None,
                }),
            },
            crate::tool::ToolExample {
                description: "Create a new memory block for tracking preferences".to_string(),
                parameters: UpdateMemoryInput {
                    label: "preferences".to_string(),
                    value: "User prefers dark mode and uses VS Code.".to_string(),
                    description: Some("User preferences and settings".to_string()),
                },
                expected_output: Some(UpdateMemoryOutput {
                    success: true,
                    previous_value: None,
                    message: Some("Created new memory block 'preferences'".to_string()),
                }),
            },
        ]
    }
}

/// Tool for appending to memory blocks
#[derive(Debug, Clone)]
pub struct AppendMemoryTool {
    pub(crate) handle: AgentHandle,
}

/// Input parameters for appending to memory
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct AppendMemoryInput {
    /// The label of the memory block to append to
    pub label: String,

    /// The content to append
    pub content: String,

    /// Optional separator (defaults to newline)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub separator: Option<String>,
}

#[async_trait]
impl AiTool for AppendMemoryTool {
    type Input = AppendMemoryInput;
    type Output = UpdateMemoryOutput;

    fn name(&self) -> &str {
        "append_memory"
    }

    fn description(&self) -> &str {
        "Append content to an existing memory block. Useful for adding to lists or accumulating information over time."
    }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        let separator = params.separator.as_deref().unwrap_or("\n");

        // Get current value
        let current = self.handle.memory.get_block(&params.label).ok_or_else(|| {
            crate::CoreError::memory_not_found(
                &self.handle.agent_id,
                &params.label,
                self.handle.memory.list_blocks(),
            )
        })?;

        let previous_value = Some(current.content.clone());
        let new_value = format!("{}{}{}", current.content, separator, params.content);

        // Update the block
        self.handle
            .memory
            .update_block_value(&params.label, new_value)?;

        Ok(UpdateMemoryOutput {
            success: true,
            previous_value,
            message: Some(format!("Appended to memory block '{}'", params.label)),
        })
    }
}

/// Tool for replacing content in memory blocks
#[derive(Debug, Clone)]
pub struct ReplaceInMemoryTool {
    pub(crate) handle: AgentHandle,
}

/// Input parameters for replacing memory content
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ReplaceInMemoryInput {
    /// The label of the memory block to update
    pub label: String,

    /// The text to search for
    pub search: String,

    /// The replacement text
    pub replace: String,

    /// Whether to replace all occurrences (default: true)
    #[serde(default = "default_replace_all")]
    pub replace_all: bool,
}

fn default_replace_all() -> bool {
    true
}

#[async_trait]
impl AiTool for ReplaceInMemoryTool {
    type Input = ReplaceInMemoryInput;
    type Output = UpdateMemoryOutput;

    fn name(&self) -> &str {
        "replace_in_memory"
    }

    fn description(&self) -> &str {
        "Replace specific text within a memory block. Useful for updating outdated information."
    }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        // Get current value
        let current = self.handle.memory.get_block(&params.label).ok_or_else(|| {
            crate::CoreError::memory_not_found(
                &self.handle.agent_id,
                &params.label,
                self.handle.memory.list_blocks(),
            )
        })?;

        let previous_value = Some(current.content.clone());

        let new_value = if params.replace_all {
            current.content.replace(&params.search, &params.replace)
        } else {
            current.content.replacen(&params.search, &params.replace, 1)
        };

        // Check if anything was replaced
        let replacements = if params.replace_all {
            current.content.matches(&params.search).count()
        } else {
            if current.content.contains(&params.search) {
                1
            } else {
                0
            }
        };

        // Update the block
        self.handle
            .memory
            .update_block_value(&params.label, new_value)?;

        Ok(UpdateMemoryOutput {
            success: true,
            previous_value,
            message: Some(format!(
                "Replaced {} occurrence(s) in memory block '{}'",
                replacements, params.label
            )),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentId, UserId, memory::Memory};

    #[tokio::test]
    async fn test_update_memory_tool() {
        let memory = Memory::with_owner(UserId::generate());
        memory.create_block("test", "initial value").unwrap();

        let handle = AgentHandle {
            agent_id: AgentId::generate(),
            memory: memory.clone(),
        };

        let tool = UpdateMemoryTool {
            handle: handle.clone(),
        };

        // Test updating existing block
        let result = tool
            .execute(UpdateMemoryInput {
                label: "test".to_string(),
                value: "updated value".to_string(),
                description: None,
            })
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.previous_value, Some("initial value".to_string()));

        // Verify the update
        let block = handle.memory.get_block("test").unwrap();
        assert_eq!(block.content, "updated value");

        // Test creating new block
        let result = tool
            .execute(UpdateMemoryInput {
                label: "new_block".to_string(),
                value: "new value".to_string(),
                description: Some("A test block".to_string()),
            })
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.previous_value, None);
        assert!(result.message.is_some());

        // Verify the new block
        let block = handle.memory.get_block("new_block").unwrap();
        assert_eq!(block.content, "new value");
        assert_eq!(block.description, Some("A test block".to_string()));
    }
}
