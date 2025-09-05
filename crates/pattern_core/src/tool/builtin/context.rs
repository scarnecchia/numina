//! Context management tool following Letta/MemGPT patterns

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    Result,
    context::AgentHandle,
    memory::{MemoryPermission, MemoryType},
    memory_acl::{MemoryGate, MemoryOp, check as acl_check, consent_reason},
    permission::PermissionScope,
    tool::{AiTool, ExecutionMeta},
};

/// Operation types for context management
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CoreMemoryOperationType {
    Append,
    Replace,
    Archive,
    Load,
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

    /// For archive/load/swap: label of the recall memory
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archival_label: Option<String>,

    /// For swap: name of the context to archive
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_name: Option<String>,
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
        "Manage context sections (persona, human, etc). Context is always visible and shapes agent behavior. No need to read - it's already in your messages. Operations: append, replace, archive, load, swap.
 - 'append' adds a new chunk of text to the block. avoid duplicate append operations.
 - 'replace' replaces a section of text (old_content is matched and replaced with new content) within a block. this can be used to delete sections.
 - 'archive' swaps an entire block to recall memory (only works on 'working' memory, not 'core', requires permissions)
 - 'load' pulls a block from recall memory into working memory (destination name optional - defaults to same label)
 - 'swap' replaces a working memory with the requested recall memory, by label"
    }

    async fn execute(&self, params: Self::Input, meta: &ExecutionMeta) -> Result<Self::Output> {
        match params.operation {
            CoreMemoryOperationType::Append => {
                let name = params.name.ok_or_else(|| {
                    crate::CoreError::tool_exec_msg(
                        "context",
                        serde_json::json!({"operation":"append"}),
                        "append operation requires 'name' field",
                    )
                })?;
                let content = params.content.ok_or_else(|| {
                    crate::CoreError::tool_exec_msg(
                        "context",
                        serde_json::json!({"operation":"append"}),
                        "append operation requires 'content' field",
                    )
                })?;
                self.execute_append(name, content, meta).await
            }
            CoreMemoryOperationType::Replace => {
                let name = params.name.ok_or_else(|| {
                    crate::CoreError::tool_exec_msg(
                        "context",
                        serde_json::json!({"operation":"replace"}),
                        "replace operation requires 'name' field",
                    )
                })?;
                let old_content = params.old_content.ok_or_else(|| {
                    crate::CoreError::tool_exec_msg(
                        "context",
                        serde_json::json!({"operation":"replace"}),
                        "replace operation requires 'old_content' field",
                    )
                })?;
                let new_content = params.new_content.ok_or_else(|| {
                    crate::CoreError::tool_exec_msg(
                        "context",
                        serde_json::json!({"operation":"replace"}),
                        "replace operation requires 'new_content' field",
                    )
                })?;
                self.execute_replace(name, old_content, new_content, meta)
                    .await
            }
            CoreMemoryOperationType::Archive => {
                let name = params.name.ok_or_else(|| {
                    crate::CoreError::tool_exec_msg(
                        "context",
                        serde_json::json!({"operation":"archive"}),
                        "archive operation requires 'name' field",
                    )
                })?;
                self.execute_archive(name, params.archival_label, meta)
                    .await
            }
            CoreMemoryOperationType::Load => {
                let archival_label = params.archival_label.ok_or_else(|| {
                    crate::CoreError::tool_exec_msg(
                        "context",
                        serde_json::json!({"operation":"load"}),
                        "load operation requires 'archival_label' field",
                    )
                })?;
                // Name is optional - defaults to archival_label
                let name = params.name;
                self.execute_load(archival_label, name, meta).await
            }
            CoreMemoryOperationType::Swap => {
                let archive_name = params.archive_name.ok_or_else(|| {
                    crate::CoreError::tool_exec_msg(
                        "context",
                        serde_json::json!({"operation":"swap"}),
                        "swap operation requires 'archive_name' field",
                    )
                })?;
                let archival_label = params.archival_label.ok_or_else(|| {
                    crate::CoreError::tool_exec_msg(
                        "context",
                        serde_json::json!({"operation":"swap"}),
                        "swap operation requires 'archival_label' field",
                    )
                })?;
                self.execute_swap(archive_name, archival_label, meta).await
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
    pub fn new(handle: AgentHandle) -> Self {
        Self { handle }
    }
    fn can_bypass(&self, meta: &ExecutionMeta, key: &str) -> bool {
        if let Some(grant) = &meta.permission_grant {
            match &grant.scope {
                PermissionScope::MemoryEdit { key: gk } => gk == key,
                PermissionScope::MemoryBatch { prefix } => key.starts_with(prefix),
                _ => false,
            }
        } else {
            false
        }
    }

    async fn execute_append(
        &self,
        name: String,
        content: String,
        meta: &ExecutionMeta,
    ) -> Result<ContextOutput> {
        // Fetch and validate target block (copy needed fields, then drop guard)
        let (current_type, current_perm) = {
            let guard = self.handle.memory.get_block(&name).ok_or_else(|| {
                crate::CoreError::memory_not_found(
                    &self.handle.agent_id,
                    &name,
                    self.handle.memory.list_blocks(),
                )
            })?;
            (guard.memory_type, guard.permission)
        };

        if current_type == MemoryType::Archival {
            return Ok(ContextOutput {
                success: false,
                message: Some(format!(
                    "Block '{}' is not context (type: {:?}). Use `recall` for archival memories.",
                    name, current_type
                )),
                content: json!({}),
            });
        }

        // Gate by permission, offering consent path when applicable
        if !self.can_bypass(meta, &name) {
            match acl_check(MemoryOp::Append, current_perm) {
                MemoryGate::Allow => {}
                MemoryGate::Deny { reason } => {
                    return Ok(ContextOutput {
                        success: false,
                        message: Some(format!("{} — cannot append to '{}'", reason, name)),
                        content: json!({}),
                    });
                }
                MemoryGate::RequireConsent { .. } => {
                    let grant = crate::permission::broker()
                        .request(
                            self.handle.agent_id.clone(),
                            "context".to_string(),
                            PermissionScope::MemoryEdit { key: name.clone() },
                            Some(consent_reason(&name, MemoryOp::Append, current_perm)),
                            meta.route_metadata.clone(),
                            std::time::Duration::from_secs(90),
                        )
                        .await;
                    if grant.is_none() {
                        return Ok(ContextOutput {
                            success: false,
                            message: Some(format!(
                                "Append to '{}' requires approval; request timed out or was denied",
                                name
                            )),
                            content: json!({}),
                        });
                    }
                }
            }
        }

        // Perform the append
        self.handle.memory.alter_block(&name, |_k, mut block| {
            block.value.push_str("\n\n");
            block.value.push_str(&content);
            block.updated_at = chrono::Utc::now();
            block
        });

        // Get the updated block to show the new state
        let updated_block = self
            .handle
            .memory
            .get_block(&name)
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

                json!({
                    "content_preview": content_preview,
                    "total_chars": char_count,
                })
            })
            .unwrap_or_else(|| json!({}));

        Ok(ContextOutput {
            success: true,
            message: Some(format!(
                "Successfully appended {} characters to context section '{}'. The section now contains {} total characters.",
                content.len(),
                name,
                updated_block
                    .get("total_chars")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
            )),
            content: updated_block,
        })
    }

    async fn execute_replace(
        &self,
        name: String,
        old_content: String,
        new_content: String,
        meta: &ExecutionMeta,
    ) -> Result<ContextOutput> {
        // Fetch and validate target block (copy fields, drop guard)
        let (current_type, current_perm) = match self.handle.memory.get_block(&name) {
            Some(b) => (b.memory_type, b.permission),
            None => {
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
        };

        if current_type == MemoryType::Archival {
            return Ok(ContextOutput {
                success: false,
                message: Some(format!(
                    "Block '{}' is not context (type: {:?})",
                    name, current_type
                )),
                content: json!({}),
            });
        }

        // Gate by permission, offering consent path when applicable
        if !self.can_bypass(meta, &name) {
            match acl_check(MemoryOp::Overwrite, current_perm) {
                MemoryGate::Allow => {}
                MemoryGate::Deny { reason } => {
                    return Ok(ContextOutput {
                        success: false,
                        message: Some(format!("{} — cannot replace in '{}'", reason, name)),
                        content: json!({}),
                    });
                }
                MemoryGate::RequireConsent { .. } => {
                    let grant = crate::permission::broker()
                        .request(
                            self.handle.agent_id.clone(),
                            "context".to_string(),
                            PermissionScope::MemoryEdit { key: name.clone() },
                            Some(consent_reason(&name, MemoryOp::Overwrite, current_perm)),
                            meta.route_metadata.clone(),
                            std::time::Duration::from_secs(90),
                        )
                        .await;
                    if grant.is_none() {
                        return Ok(ContextOutput {
                            success: false,
                            message: Some(format!(
                                "Replace in '{}' requires approval; request timed out or was denied",
                                name
                            )),
                            content: json!({}),
                        });
                    }
                }
            }
        }

        // Perform the replacement (and validate old_content presence)
        if let Some(mut block) = self.handle.memory.get_block_mut(&name) {
            if !block.value.contains(&old_content) {
                return Ok(ContextOutput {
                    success: false,
                    message: Some(format!(
                        "Content '{}' not found in context section '{}'",
                        old_content, name
                    )),
                    content: json!({}),
                });
            }
            block.value = block.value.replace(&old_content, &new_content);
            block.updated_at = chrono::Utc::now();
        }

        // Get the updated block to show the new state
        let updated_block = self
            .handle
            .memory
            .get_block(&name)
            .map(|block| {
                let char_count = block.value.chars().count();

                // Find where the replacement happened and show context around it
                let preview_chars = 100; // Show 100 chars before and after
                let content_preview = if let Some(pos) = block.value.find(&new_content) {
                    let start = pos.saturating_sub(preview_chars);
                    let end = (pos + new_content.len() + preview_chars).min(block.value.len());

                    let prefix = if start > 0 { "..." } else { "" };
                    let suffix = if end < block.value.len() { "..." } else { "" };

                    format!("{}{}{}", prefix, &block.value[start..end], suffix)
                } else {
                    // Fallback to showing the end if we can't find the replacement
                    if block.value.len() > preview_chars * 2 {
                        format!(
                            "...{}",
                            &block.value[block.value.len().saturating_sub(preview_chars * 2)..]
                        )
                    } else {
                        block.value.clone()
                    }
                };

                json!({
                    "content_preview": content_preview,
                    "total_chars": char_count,
                })
            })
            .unwrap_or_else(|| json!({}));

        Ok(ContextOutput {
            success: true,
            message: Some(format!(
                "Successfully replaced content in context section '{}'. The section now contains {} total characters.",
                name,
                updated_block
                    .get("total_chars")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
            )),
            content: updated_block,
        })
    }

    async fn execute_archive(
        &self,
        name: String,
        archival_label: Option<String>,
        meta: &ExecutionMeta,
    ) -> Result<ContextOutput> {
        // Check if the block exists and is context
        let block = match self.handle.memory.get_block(&name) {
            Some(block) => {
                // can't archive blocks you don't have admin access for
                if block.memory_type != MemoryType::Working
                    && block.memory_type != MemoryType::Core
                    && !block.pinned
                {
                    return Ok(ContextOutput {
                        success: false,
                        message: Some(format!(
                            "Block '{}' is not context (type: {:?})",
                            name, block.memory_type
                        )),
                        content: json!({}),
                    });
                } else if block.permission < MemoryPermission::Append
                    && !self.can_bypass(meta, &name)
                {
                    return Ok(ContextOutput {
                        success: false,
                        message: Some(format!(
                            "Not enough permissions to swap out block '{}', requires read_write",
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

        // If the archival label already exists, enforce Overwrite ACL (with consent path)
        if self.handle.memory.contains_block(&archival_label) {
            if let Some(existing) = self.handle.memory.get_block(&archival_label) {
                if !self.can_bypass(meta, &archival_label) {
                    match acl_check(MemoryOp::Overwrite, existing.permission) {
                        MemoryGate::Allow => {}
                        MemoryGate::Deny { reason } => {
                            return Ok(ContextOutput {
                                success: false,
                                message: Some(format!(
                                    "{} — cannot overwrite existing recall memory '{}'",
                                    reason, archival_label
                                )),
                                content: json!({}),
                            });
                        }
                        MemoryGate::RequireConsent { .. } => {
                            let grant = crate::permission::broker()
                                .request(
                                    self.handle.agent_id.clone(),
                                    "context".to_string(),
                                    PermissionScope::MemoryEdit {
                                        key: archival_label.clone(),
                                    },
                                    Some(consent_reason(
                                        &archival_label,
                                        MemoryOp::Overwrite,
                                        existing.permission,
                                    )),
                                    meta.route_metadata.clone(),
                                    std::time::Duration::from_secs(90),
                                )
                                .await;
                            if grant.is_none() {
                                return Ok(ContextOutput {
                                    success: false,
                                    message: Some(format!(
                                        "Overwriting recall memory '{}' requires approval; request timed out or was denied",
                                        archival_label
                                    )),
                                    content: json!({}),
                                });
                            }
                        }
                    }
                }
            }
        }

        // Create the recall memory
        self.handle
            .memory
            .create_block(&archival_label, &block.value)?;

        // Update it to be archival type
        if let Some(mut archival_block) = self.handle.memory.get_block_mut(&archival_label) {
            archival_block.memory_type = MemoryType::Archival;
            archival_block.permission = MemoryPermission::ReadWrite;
            archival_block.description = Some(format!("Archived from context '{}'", name));
        }

        // Enforce delete ACL for the original context block
        match acl_check(MemoryOp::Delete, block.permission) {
            MemoryGate::Allow => {
                self.handle.memory.remove_block(&name);
            }
            MemoryGate::RequireConsent { .. } | MemoryGate::Deny { .. } => {
                return Ok(ContextOutput {
                    success: false,
                    message: Some(format!(
                        "Insufficient permission to delete context block '{}' (requires Admin)",
                        name
                    )),
                    content: json!({}),
                });
            }
        }

        Ok(ContextOutput {
            success: true,
            message: Some(format!(
                "Archived context '{}' to recall memory '{}'",
                name, archival_label
            )),
            content: json!({}),
        })
    }

    async fn execute_load(
        &self,
        archival_label: String,
        name: Option<String>,
        _meta: &ExecutionMeta,
    ) -> Result<ContextOutput> {
        // Use archival_label as destination if name not provided
        let destination_name = name.unwrap_or_else(|| archival_label.clone());

        // Support same-label load by converting archival -> working in-memory
        if destination_name == archival_label {
            // If present in memory, flip the type
            if let Some(mut existing_block) = self.handle.memory.get_block_mut(&archival_label) {
                if existing_block.memory_type == MemoryType::Archival {
                    existing_block.memory_type = MemoryType::Working;
                    return Ok(ContextOutput {
                        success: true,
                        message: Some(format!(
                            "Changed recall memory '{}' to working memory",
                            archival_label
                        )),
                        content: json!({}),
                    });
                } else {
                    return Ok(ContextOutput {
                        success: true,
                        message: Some(format!(
                            "Memory '{}' is already working memory",
                            archival_label
                        )),
                        content: json!({}),
                    });
                }
            }

            // Not present in memory: attempt to fetch archival from DB and add as working
            if let Some(src) = self
                .handle
                .get_archival_memory_by_label(&archival_label)
                .await?
            {
                self.handle
                    .memory
                    .create_block(&archival_label, &src.value)?;
                if let Some(mut blk) = self.handle.memory.get_block_mut(&archival_label) {
                    blk.memory_type = MemoryType::Working;
                    blk.permission = MemoryPermission::ReadWrite;
                }
                return Ok(ContextOutput {
                    success: true,
                    message: Some(format!(
                        "Loaded recall memory '{}' into working memory (same label)",
                        archival_label
                    )),
                    content: json!({}),
                });
            } else {
                return Ok(ContextOutput {
                    success: false,
                    message: Some(format!(
                        "Archival memory '{}' not found for same-label load",
                        archival_label
                    )),
                    content: json!({}),
                });
            }
        }

        // If not in memory, check if it exists as archival
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

        // If destination exists already, block to avoid unintentional overwrite
        if self.handle.memory.contains_block(&destination_name) {
            return Ok(ContextOutput {
                success: false,
                message: Some(format!(
                    "Working memory '{}' already exists. Use swap or choose a different destination name.",
                    destination_name
                )),
                content: json!({}),
            });
        }

        // Create the context block at destination
        self.handle
            .memory
            .create_block(&destination_name, &archival_block.value)?;

        // Update it to be working type
        if let Some(mut working_block) = self.handle.memory.get_block_mut(&destination_name) {
            working_block.memory_type = MemoryType::Working;
            working_block.permission = MemoryPermission::ReadWrite;
            working_block.description =
                Some(format!("Loaded from recall memory '{}'", archival_label));
        }

        // Important: keep the original archival block intact

        Ok(ContextOutput {
            success: true,
            message: Some(format!(
                "Loaded recall memory '{}' into working memory '{}' (archival retained)",
                archival_label, destination_name
            )),
            content: json!({}),
        })
    }

    async fn execute_swap(
        &self,
        archive_name: String,
        archival_label: String,
        meta: &ExecutionMeta,
    ) -> Result<ContextOutput> {
        // First check both blocks exist and have correct types
        let core_block = match self.handle.memory.get_block(&archive_name) {
            Some(block) => {
                if block.memory_type != MemoryType::Working || block.pinned {
                    return Ok(ContextOutput {
                        success: false,
                        message: Some(format!(
                            "Block '{}' is not context (type: {:?})",
                            archive_name, block.memory_type
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

        // Overwrite requires ACL; allow consent path if configured
        if !self.can_bypass(meta, &archive_name) {
            match acl_check(MemoryOp::Overwrite, core_block.permission) {
                MemoryGate::Allow => {}
                MemoryGate::Deny { reason } => {
                    return Ok(ContextOutput {
                        success: false,
                        message: Some(format!("{} — cannot swap into '{}'", reason, archive_name)),
                        content: json!({}),
                    });
                }
                MemoryGate::RequireConsent { .. } => {
                    let grant = crate::permission::broker()
                        .request(
                            self.handle.agent_id.clone(),
                            "context".to_string(),
                            PermissionScope::MemoryEdit {
                                key: archive_name.clone(),
                            },
                            Some(consent_reason(
                                &archive_name,
                                MemoryOp::Overwrite,
                                core_block.permission,
                            )),
                            meta.route_metadata.clone(),
                            std::time::Duration::from_secs(90),
                        )
                        .await;
                    if grant.is_none() {
                        return Ok(ContextOutput {
                            success: false,
                            message: Some(format!(
                                "Swap into '{}' requires approval; request timed out or was denied",
                                archive_name
                            )),
                            content: json!({}),
                        });
                    }
                }
            }
        }

        // Update the context with archival content
        self.handle
            .memory
            .update_block_value(&archive_name, archival_block.value.clone())?;

        // Remove the original archival block (enforce delete ACL)
        match acl_check(MemoryOp::Delete, archival_block.permission) {
            MemoryGate::Allow => {
                self.handle.memory.remove_block(&archival_label);
            }
            _ => {
                return Ok(ContextOutput {
                    success: false,
                    message: Some(format!(
                        "Insufficient permission to delete recall memory '{}' (requires Admin)",
                        archival_label
                    )),
                    content: json!({}),
                });
            }
        }

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
        // Best-effort cleanup of temp block
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
            .execute(
                ContextInput {
                    operation: CoreMemoryOperationType::Append,
                    name: Some("human".to_string()),
                    content: Some("They work in healthcare.".to_string()),
                    old_content: None,
                    new_content: None,
                    archival_label: None,
                    archive_name: None,
                },
                &crate::tool::ExecutionMeta::default(),
            )
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
