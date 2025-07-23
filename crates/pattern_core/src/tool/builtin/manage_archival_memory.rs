//! Archival memory management tool following Letta/MemGPT patterns

use std::{cmp::max, fmt::Debug};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Result,
    context::AgentHandle,
    memory::{MemoryPermission, MemoryType},
    tool::AiTool,
};

/// Operation types for archival memory management
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum ArchivalMemoryOperationType {
    Insert,
    Read,
    Search,
    Delete,
}

/// Input for managing archival memory
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ManageArchivalMemoryInput {
    /// The operation to perform
    pub operation: ArchivalMemoryOperationType,

    /// For insert/read/delete: label for the memory (insert defaults to "archival_<timestamp>")
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// For insert: content to store in archival memory
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    /// For search: search query (currently text-based substring matching)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,

    /// For search: maximum number of results to return (default: 10)
    #[schemars(default, with = "i64")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
}

/// Output from archival memory operations
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ManageArchivalMemoryOutput {
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

/// A single search result from archival memory
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

/// Unified tool for managing archival memory
#[derive(Debug, Clone)]
pub struct ManageArchivalMemoryTool<C: surrealdb::Connection + Clone> {
    pub(crate) handle: AgentHandle<C>,
}

#[async_trait]
impl<C: surrealdb::Connection + Clone + Debug> AiTool for ManageArchivalMemoryTool<C> {
    type Input = ManageArchivalMemoryInput;
    type Output = ManageArchivalMemoryOutput;

    fn name(&self) -> &str {
        "manage_archival_memory"
    }

    fn description(&self) -> &str {
        "Manage archival (long-term) memory storage. Archival memory is searchable on-demand and not always in context. Operations: insert, read (by label), search (by content), delete."
    }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        match params.operation {
            ArchivalMemoryOperationType::Insert => {
                let content = params.content.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "manage_archival_memory",
                        "insert operation requires 'content' field",
                    )
                })?;
                self.execute_insert(content, params.label).await
            }
            ArchivalMemoryOperationType::Read => {
                let label = params.label.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "manage_archival_memory",
                        "read operation requires 'label' field",
                    )
                })?;
                self.execute_read(label).await
            }
            ArchivalMemoryOperationType::Search => {
                let query = params.query.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "manage_archival_memory",
                        "search operation requires 'query' field",
                    )
                })?;
                self.execute_search(
                    query,
                    params.limit.map(|limit| {
                        usize::try_from(max(limit, 0))
                            .expect("should be zero or greater after this")
                    }),
                )
                .await
            }
            ArchivalMemoryOperationType::Delete => {
                let label = params.label.ok_or_else(|| {
                    crate::CoreError::tool_execution_error(
                        "manage_archival_memory",
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
                parameters: ManageArchivalMemoryInput {
                    operation: ArchivalMemoryOperationType::Insert,
                    content: Some(
                        "User mentioned they have a dog named Max who likes to play fetch."
                            .to_string(),
                    ),
                    label: None,
                    query: None,
                    limit: None,
                },
                expected_output: Some(ManageArchivalMemoryOutput {
                    success: true,
                    message: Some("Created archival memory 'archival_1234567890'".to_string()),
                    results: vec![],
                }),
            },
            crate::tool::ToolExample {
                description: "Search for information about pets".to_string(),
                parameters: ManageArchivalMemoryInput {
                    operation: ArchivalMemoryOperationType::Search,
                    content: None,
                    label: None,
                    query: Some("dog".to_string()),
                    limit: Some(5),
                },
                expected_output: Some(ManageArchivalMemoryOutput {
                    success: true,
                    message: Some("Found 1 matching archival memories".to_string()),
                    results: vec![ArchivalSearchResult {
                        label: "archival_1234567890".to_string(),
                        content:
                            "User mentioned they have a dog named Max who likes to play fetch."
                                .to_string(),
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                    }],
                }),
            },
        ]
    }
}

impl<C: surrealdb::Connection + Clone> ManageArchivalMemoryTool<C> {
    async fn execute_insert(
        &self,
        content: String,
        label: Option<String>,
    ) -> Result<ManageArchivalMemoryOutput> {
        // Generate label if not provided
        let label = label.unwrap_or_else(|| format!("archival_{}", chrono::Utc::now().timestamp()));

        // Try to use database if available, fall back to in-memory
        if self.handle.has_db_connection() {
            match self.handle.insert_archival_memory(&label, &content).await {
                Ok(_) => Ok(ManageArchivalMemoryOutput {
                    success: true,
                    message: Some(format!("Created archival memory '{}' in database", label)),
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

    fn insert_in_memory(
        &self,
        label: String,
        content: String,
    ) -> Result<ManageArchivalMemoryOutput> {
        // Check if label already exists
        if self.handle.memory.contains_block(&label) {
            return Ok(ManageArchivalMemoryOutput {
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

        // Update it to be archival type with appropriate permissions
        if let Some(mut block) = self.handle.memory.get_block_mut(&label) {
            block.memory_type = MemoryType::Archival;
            block.permission = MemoryPermission::ReadWrite;
        }

        Ok(ManageArchivalMemoryOutput {
            success: true,
            message: Some(format!("Created archival memory '{}'", label)),
            results: vec![],
        })
    }

    async fn execute_search(
        &self,
        query: String,
        limit: Option<usize>,
    ) -> Result<ManageArchivalMemoryOutput> {
        let limit = limit.unwrap_or(10);

        // Try to search using database if available, fall back to in-memory search
        let results = if self.handle.has_db_connection() {
            // Use database search for better performance and to avoid loading all into RAM
            match self.handle.search_archival_memories(&query, limit).await {
                Ok(blocks) => blocks
                    .into_iter()
                    .map(|block| ArchivalSearchResult {
                        label: block.label.to_string(),
                        content: block.value.clone(),
                        created_at: block.created_at,
                        updated_at: block.updated_at,
                    })
                    .collect(),
                Err(e) => {
                    tracing::warn!("Database search failed, falling back to in-memory: {}", e);
                    self.search_in_memory(&query, limit)
                }
            }
        } else {
            // Fall back to in-memory search
            self.search_in_memory(&query, limit)
        };

        let count = results.len();

        Ok(ManageArchivalMemoryOutput {
            success: true,
            message: Some(format!("Found {} matching archival memories", count)),
            results: results,
        })
    }

    fn search_in_memory(&self, query: &str, limit: usize) -> Vec<ArchivalSearchResult> {
        let query_lower = query.to_lowercase();

        let mut results: Vec<ArchivalSearchResult> = self
            .handle
            .memory
            .get_all_blocks()
            .into_iter()
            .filter(|block| {
                block.memory_type == MemoryType::Archival
                    && block.value.to_lowercase().contains(&query_lower)
            })
            .take(limit)
            .map(|block| ArchivalSearchResult {
                label: block.label.to_string(),
                content: block.value.clone(),
                created_at: block.created_at,
                updated_at: block.updated_at,
            })
            .collect();

        // Sort by most recently updated first
        results.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        results
    }

    async fn execute_read(&self, label: String) -> Result<ManageArchivalMemoryOutput> {
        // Fall back to in-memory
        match self.handle.memory.get_block(&label) {
            Some(block) => {
                // Verify it's archival memory
                if block.memory_type == MemoryType::Archival {
                    Ok(ManageArchivalMemoryOutput {
                        success: true,
                        message: Some(format!("Found archival memory '{}'", label)),
                        results: vec![ArchivalSearchResult {
                            label: block.label.to_string(),
                            content: block.value.clone(),
                            created_at: block.created_at,
                            updated_at: block.updated_at,
                        }],
                    })
                } else {
                    Ok(ManageArchivalMemoryOutput {
                        success: false,
                        message: Some(format!(
                            "Block '{}' exists but is not archival memory (type: {:?})",
                            label, block.memory_type
                        )),
                        results: vec![],
                    })
                }
            }
            None => {
                // Try database first if available
                if self.handle.has_db_connection() {
                    // For now, we'll search for the exact label
                    match self.handle.search_archival_memories(&label, 10).await {
                        Ok(blocks) => {
                            // Find exact match
                            if let Some(block) = blocks.iter().find(|b| b.label == label) {
                                return Ok(ManageArchivalMemoryOutput {
                                    success: true,
                                    message: Some(format!("Found archival memory '{}'", label)),
                                    results: vec![ArchivalSearchResult {
                                        label: block.label.to_string(),
                                        content: block.value.clone(),
                                        created_at: block.created_at,
                                        updated_at: block.updated_at,
                                    }],
                                });
                            } else {
                                Ok(ManageArchivalMemoryOutput {
                                    success: false,
                                    message: Some(format!(
                                        "No archival memory found with label '{}'",
                                        label
                                    )),
                                    results: vec![],
                                })
                            }
                        }
                        Err(e) => Ok(ManageArchivalMemoryOutput {
                            success: false,
                            message: Some(format!(
                                "No archival memory found with label '{}' due to database error {}",
                                label, e
                            )),
                            results: vec![],
                        }),
                    }
                } else {
                    Ok(ManageArchivalMemoryOutput {
                        success: false,
                        message: Some(format!("No archival memory found with label '{}'", label)),
                        results: vec![],
                    })
                }
            }
        }
    }

    async fn execute_delete(&self, label: String) -> Result<ManageArchivalMemoryOutput> {
        // Check if block exists
        match self.handle.memory.get_block(&label) {
            Some(block) => {
                // Verify it's archival memory
                if block.memory_type != MemoryType::Archival {
                    return Ok(ManageArchivalMemoryOutput {
                        success: false,
                        message: Some(format!(
                            "Block '{}' is not archival memory (type: {:?})",
                            label, block.memory_type
                        )),
                        results: vec![],
                    });
                }
            }
            None => {
                return Ok(ManageArchivalMemoryOutput {
                    success: false,
                    message: Some(format!("Archival memory '{}' not found", label)),
                    results: vec![],
                });
            }
        }

        // Remove the block from memory
        self.handle.memory.remove_block(&label);

        Ok(ManageArchivalMemoryOutput {
            success: true,
            message: Some(format!("Deleted archival memory '{}'", label)),
            results: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UserId, memory::Memory};

    #[tokio::test]
    async fn test_archival_insert_and_search() {
        let memory = Memory::with_owner(UserId::generate());
        let handle = AgentHandle::test_with_memory(memory.clone());

        let tool = ManageArchivalMemoryTool { handle };

        // Test inserting
        let result = tool
            .execute(ManageArchivalMemoryInput {
                operation: ArchivalMemoryOperationType::Insert,
                content: Some("The user's favorite color is blue.".to_string()),
                label: Some("user_preferences".to_string()),
                query: None,
                limit: None,
            })
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(
            result.message.as_ref().unwrap(),
            "Created archival memory 'user_preferences'"
        );

        // Verify the block was created with correct type
        let block = memory.get_block("user_preferences").unwrap();
        assert_eq!(block.memory_type, MemoryType::Archival);
        assert_eq!(block.value, "The user's favorite color is blue.");

        // Test searching
        let result = tool
            .execute(ManageArchivalMemoryInput {
                operation: ArchivalMemoryOperationType::Search,
                content: None,
                label: None,
                query: Some("color".to_string()),
                limit: None,
            })
            .await
            .unwrap();

        assert!(result.success);
        let results = result.results;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].label, "user_preferences");
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

        let tool = ManageArchivalMemoryTool { handle };

        // Test deleting
        let result = tool
            .execute(ManageArchivalMemoryInput {
                operation: ArchivalMemoryOperationType::Delete,
                content: None,
                label: Some("to_delete".to_string()),
                query: None,
                limit: None,
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

        let tool = ManageArchivalMemoryTool { handle };

        // Try to delete a core memory block
        let result = tool
            .execute(ManageArchivalMemoryInput {
                operation: ArchivalMemoryOperationType::Delete,
                content: None,
                label: Some("core_block".to_string()),
                query: None,
                limit: None,
            })
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.message.unwrap().contains("not archival memory"));
        // Block should still exist
        assert!(memory.get_block("core_block").is_some());
    }
}
