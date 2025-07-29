use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::data_source::{BufferConfig, DataIngestionCoordinator, FileDataSource, FileStorageMode};
use crate::error::Result;
use crate::tool::{AiTool, ToolRegistry};

/// Tool for managing data sources that feed into agents
#[derive(Debug, Clone)]
pub struct DataSourceTool {
    coordinator: Arc<tokio::sync::RwLock<DataIngestionCoordinator>>,
}

impl DataSourceTool {
    pub fn new(coordinator: Arc<tokio::sync::RwLock<DataIngestionCoordinator>>) -> Self {
        Self { coordinator }
    }
}

/// Operation types for data source management
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum DataSourceOperation {
    ReadFile,
    IndexFile,
    WatchFile,
    SearchBuffer,
    ListSources,
    PauseSource,
    ResumeSource,
    GetBufferStats,
}

/// Input for data source operations
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DataSourceInput {
    /// The operation to perform
    pub operation: DataSourceOperation,

    /// File path (for read_file, index_file, watch_file)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// Source identifier (for search_buffer, pause_source, resume_source, get_buffer_stats)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,

    /// Search query (for search_buffer)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,

    /// Template name for notifications (for watch_file)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_name: Option<String>,

    /// Line range for reading files (format: "start-end", e.g., "10-20")
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_range: Option<String>,

    /// Chunk size for indexing files
    #[schemars(default, with = "i64")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_size: Option<i64>,

    /// Maximum number of results
    #[schemars(default, with = "i64")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,

    /// DEPRECATED: watch_file always enables notifications
    #[serde(default)]
    pub notify: bool,

    /// Request another turn after this tool executes
    #[serde(default)]
    pub request_heartbeat: bool,
}

/// Output from data source operations
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DataSourceOutput {
    /// Whether the operation was successful
    pub success: bool,

    /// Message about the operation
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// Content returned by the operation (file content, source list, etc.)
    #[serde(default)]
    pub content: serde_json::Value,
}

#[async_trait]
impl AiTool for DataSourceTool {
    type Input = DataSourceInput;
    type Output = DataSourceOutput;

    fn name(&self) -> &'static str {
        "data_source"
    }

    fn description(&self) -> &'static str {
        "Manage data sources that feed information to agents. Read files, search buffers, manage streams."
    }

    fn usage_rule(&self) -> Option<&'static str> {
        Some(
            r#"Use data_source for file operations and stream management:
- read_file: Read file contents (supports line ranges like "10-20")
- index_file: Create indexed file source for semantic search
- watch_file: Monitor file changes (automatically sends notifications)
- list_sources: See all active data sources
- pause_source/resume_source: Control data flow
- get_buffer_stats: Check source buffer status
- search_buffer: Search buffered data (not yet implemented)

For simple file reads, use read_file. For monitoring changes, use watch_file."#,
        )
    }

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        // Extract values to avoid partial move issues
        let operation = input.operation.clone();
        let path = input.path;
        let source_id = input.source_id;
        let query = input.query;
        let template_name = input.template_name;
        let line_range = input.line_range;
        let chunk_size = input.chunk_size;
        let limit = input.limit;
        let _notify = input.notify; // deprecated field
        let _request_heartbeat = input.request_heartbeat;

        match operation {
            DataSourceOperation::ReadFile => {
                let path = path.ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                    tool_name: self.name().to_string(),
                    cause: "Missing required parameter 'path' for read_file".to_string(),
                    parameters: json!({}),
                })?;

                let path_buf = PathBuf::from(path);

                if !path_buf.exists() {
                    return Ok(DataSourceOutput {
                        success: false,
                        message: Some(format!("File not found: {}", path_buf.display())),
                        content: json!(null),
                    });
                }

                // Parse line range if provided
                let content = if let Some(range_str) = line_range {
                    // Parse "start-end" format
                    let parts: Vec<&str> = range_str.split('-').collect();
                    if parts.len() == 2 {
                        if let (Ok(start), Ok(end)) =
                            (parts[0].parse::<usize>(), parts[1].parse::<usize>())
                        {
                            let text = tokio::fs::read_to_string(&path_buf).await.map_err(|e| {
                                crate::CoreError::ToolExecutionFailed {
                                    tool_name: self.name().to_string(),
                                    cause: format!("Failed to read file: {}", e),
                                    parameters: json!({}),
                                }
                            })?;

                            let lines: Vec<&str> = text.lines().collect();
                            let start = start.saturating_sub(1).min(lines.len()); // Convert to 0-based
                            let end = end.min(lines.len());

                            lines[start..end].join("\n")
                        } else {
                            return Ok(DataSourceOutput {
                                success: false,
                                message: Some(
                                    "Invalid line range format. Use 'start-end' (e.g., '10-20')"
                                        .to_string(),
                                ),
                                content: json!(null),
                            });
                        }
                    } else {
                        return Ok(DataSourceOutput {
                            success: false,
                            message: Some(
                                "Invalid line range format. Use 'start-end' (e.g., '10-20')"
                                    .to_string(),
                            ),
                            content: json!(null),
                        });
                    }
                } else {
                    // Read entire file
                    tokio::fs::read_to_string(&path_buf).await.map_err(|e| {
                        crate::CoreError::ToolExecutionFailed {
                            tool_name: self.name().to_string(),
                            cause: format!("Failed to read file: {}", e),
                            parameters: json!({}),
                        }
                    })?
                };

                let metadata = tokio::fs::metadata(&path_buf).await.map_err(|e| {
                    crate::CoreError::ToolExecutionFailed {
                        tool_name: self.name().to_string(),
                        cause: format!("Failed to read metadata: {}", e),
                        parameters: json!({}),
                    }
                })?;

                Ok(DataSourceOutput {
                    success: true,
                    message: Some(format!(
                        "Read {} bytes from {}",
                        content.len(),
                        path_buf.display()
                    )),
                    content: json!({
                        "text": content,
                        "path": path_buf.display().to_string(),
                        "size_bytes": metadata.len(),
                        "is_dir": metadata.is_dir(),
                    }),
                })
            }

            DataSourceOperation::IndexFile => {
                let path = path.ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                    tool_name: self.name().to_string(),
                    cause: "Missing required parameter 'path' for index_file".to_string(),
                    parameters: json!({}),
                })?;

                let chunk_size = chunk_size.unwrap_or(512);

                // Get embedding provider from coordinator
                let coordinator_read = self.coordinator.read().await;
                let embedding_provider = coordinator_read.embedding_provider();
                drop(coordinator_read);

                if let Some(provider) = embedding_provider {
                    // Create an indexed file data source
                    let source = FileDataSource::new(
                        path.clone(),
                        FileStorageMode::Indexed {
                            embedding_provider: provider,
                            chunk_size,
                        },
                    );

                    let config = BufferConfig {
                        max_items: 1000,
                        max_age: std::time::Duration::from_secs(86400), // 24 hours
                        persist_to_db: true,
                        index_content: true,
                    };

                    let mut coordinator = self.coordinator.write().await;
                    coordinator
                        .add_source(source, config, "file_changed".to_string())
                        .await?;

                    Ok(DataSourceOutput {
                        success: true,
                        message: Some(format!("Indexed file source created for: {}", path)),
                        content: json!(null),
                    })
                } else {
                    Ok(DataSourceOutput {
                        success: false,
                        message: Some("No embedding provider available. Indexed file sources require embeddings.".to_string()),
                        content: json!(null),
                    })
                }
            }

            DataSourceOperation::WatchFile => {
                let path = path.ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                    tool_name: self.name().to_string(),
                    cause: "Missing required parameter 'path' for watch_file".to_string(),
                    parameters: json!({}),
                })?;

                let template_name = template_name.unwrap_or_else(|| "file_changed".to_string());

                // Create a file data source with watching enabled
                let source =
                    FileDataSource::new(path.clone(), FileStorageMode::Ephemeral).with_watch();

                let config = BufferConfig {
                    max_items: 100,
                    max_age: std::time::Duration::from_secs(3600), // 1 hour
                    persist_to_db: false,
                    index_content: false,
                };

                // watch_file always enables notifications
                let mut coordinator = self.coordinator.write().await;
                coordinator
                    .add_source(source, config, template_name)
                    .await?;

                Ok(DataSourceOutput {
                    success: true,
                    message: Some(format!("Watching file with notifications: {}", path)),
                    content: json!(null),
                })
            }

            DataSourceOperation::ListSources => {
                let coordinator = self.coordinator.read().await;
                let sources = coordinator.list_sources().await;

                let mut source_list: Vec<_> = sources
                    .into_iter()
                    .map(|(id, source_type)| {
                        json!({
                            "source_id": id,
                            "source_type": source_type,
                            "status": "active"
                        })
                    })
                    .collect();

                // Apply limit if specified
                if let Some(max_items) = limit {
                    source_list.truncate(max_items as usize);
                }

                Ok(DataSourceOutput {
                    success: true,
                    message: Some(format!("Found {} active sources", source_list.len())),
                    content: json!(source_list),
                })
            }

            DataSourceOperation::GetBufferStats => {
                let source_id = source_id.ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                    tool_name: self.name().to_string(),
                    cause: "Missing required parameter 'source_id' for get_buffer_stats"
                        .to_string(),
                    parameters: json!({}),
                })?;

                let coordinator = self.coordinator.read().await;
                let stats = coordinator.get_buffer_stats(&source_id).await?;

                Ok(DataSourceOutput {
                    success: true,
                    message: Some(format!("Buffer stats for source '{}'", source_id)),
                    content: stats,
                })
            }

            DataSourceOperation::PauseSource => {
                let source_id = source_id.ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                    tool_name: self.name().to_string(),
                    cause: "Missing required parameter 'source_id' for pause_source".to_string(),
                    parameters: json!({}),
                })?;

                let coordinator = self.coordinator.read().await;
                coordinator.pause_source(&source_id).await?;

                Ok(DataSourceOutput {
                    success: true,
                    message: Some(format!("Source '{}' paused", source_id)),
                    content: json!(null),
                })
            }

            DataSourceOperation::ResumeSource => {
                let source_id = source_id.ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                    tool_name: self.name().to_string(),
                    cause: "Missing required parameter 'source_id' for resume_source".to_string(),
                    parameters: json!({}),
                })?;

                let coordinator = self.coordinator.read().await;
                coordinator.resume_source(&source_id).await?;

                Ok(DataSourceOutput {
                    success: true,
                    message: Some(format!("Source '{}' resumed", source_id)),
                    content: json!(null),
                })
            }

            DataSourceOperation::SearchBuffer => {
                let _source_id =
                    source_id.ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                        tool_name: self.name().to_string(),
                        cause: "Missing required parameter 'source_id' for search_buffer"
                            .to_string(),
                        parameters: json!({}),
                    })?;

                let _query = query.ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                    tool_name: self.name().to_string(),
                    cause: "Missing required parameter 'query' for search_buffer".to_string(),
                    parameters: json!({}),
                })?;

                // TODO: Implement buffer search
                Ok(DataSourceOutput {
                    success: false,
                    message: Some("Buffer search not yet implemented".to_string()),
                    content: json!(null),
                })
            }
        }
    }
}

/// Register data source tool in the registry
pub fn register_data_source_tool(
    registry: &ToolRegistry,
    coordinator: Arc<tokio::sync::RwLock<DataIngestionCoordinator>>,
) {
    let tool = DataSourceTool::new(coordinator);
    registry.register(tool);
}
