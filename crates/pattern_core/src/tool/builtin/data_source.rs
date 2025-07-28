use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DataSourceInput {
    pub operation: DataSourceOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum DataSourceOperation {
    // File operations
    ReadFile {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        lines: Option<std::ops::Range<usize>>,
    },
    IndexFile {
        path: String,
        chunk_size: usize,
    },
    WatchFile {
        path: String,
        notify: bool,
        template_name: String,
    },

    // Stream operations
    SearchBuffer {
        source_id: String,
        query: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        time_range: Option<TimeRange>,
        limit: usize,
    },
    ReplayFrom {
        source_id: String,
        cursor: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        filter: Option<Value>,
    },
    GetBufferStats {
        source_id: String,
    },

    // Source management
    ListSources,
    PauseSource {
        source_id: String,
    },
    ResumeSource {
        source_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TimeRange {
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DataSourceOutput {
    pub success: bool,
    pub result: DataSourceResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum DataSourceResult {
    Text(String),
    FileContent {
        path: String,
        content: String,
        metadata: FileMetadata,
    },
    Sources(Vec<SourceInfo>),
    BufferStats(Value),
    Success {
        message: String,
    },
    Error {
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileMetadata {
    pub size_bytes: u64,
    pub modified: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SourceInfo {
    pub source_id: String,
    pub source_type: String,
    pub status: String,
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

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        match input.operation {
            DataSourceOperation::ReadFile { path, lines } => {
                // Simple file read without creating a data source
                let path = PathBuf::from(path);

                if !path.exists() {
                    return Ok(DataSourceOutput {
                        success: false,
                        result: DataSourceResult::Error {
                            error: format!("File not found: {}", path.display()),
                        },
                    });
                }

                let content = if let Some(range) = lines {
                    // Read specific lines
                    let text = tokio::fs::read_to_string(&path).await.map_err(|e| {
                        crate::CoreError::ToolExecutionFailed {
                            tool_name: AiTool::name(self).to_string(),
                            cause: format!("Failed to read file: {}", e),
                            parameters: serde_json::json!({ "path": path }),
                        }
                    })?;

                    let lines: Vec<&str> = text.lines().collect();
                    let start = range.start.min(lines.len());
                    let end = range.end.min(lines.len());

                    lines[start..end].join("\n")
                } else {
                    // Read entire file
                    tokio::fs::read_to_string(&path).await.map_err(|e| {
                        crate::CoreError::ToolExecutionFailed {
                            tool_name: AiTool::name(self).to_string(),
                            cause: format!("Failed to read file: {}", e),
                            parameters: serde_json::json!({ "path": path }),
                        }
                    })?
                };

                let metadata = tokio::fs::metadata(&path).await.map_err(|e| {
                    crate::CoreError::ToolExecutionFailed {
                        tool_name: AiTool::name(self).to_string(),
                        cause: format!("Failed to read metadata: {}", e),
                        parameters: serde_json::json!({ "path": path }),
                    }
                })?;

                Ok(DataSourceOutput {
                    success: true,
                    result: DataSourceResult::FileContent {
                        path: path.display().to_string(),
                        content,
                        metadata: FileMetadata {
                            size_bytes: metadata.len(),
                            modified: metadata
                                .modified()
                                .map(|t| format!("{:?}", t))
                                .unwrap_or_else(|_| "unknown".to_string()),
                            is_dir: metadata.is_dir(),
                        },
                    },
                })
            }

            DataSourceOperation::IndexFile { path, chunk_size } => {
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
                        result: DataSourceResult::Success {
                            message: format!("Indexed file source created for: {}", path),
                        },
                    })
                } else {
                    Ok(DataSourceOutput {
                        success: false,
                        result: DataSourceResult::Error {
                            error: "No embedding provider available. Indexed file sources require embeddings.".to_string(),
                        },
                    })
                }
            }

            DataSourceOperation::WatchFile {
                path,
                notify,
                template_name,
            } => {
                // Create a file data source with watching enabled
                let source =
                    FileDataSource::new(path.clone(), FileStorageMode::Ephemeral).with_watch();

                let config = BufferConfig {
                    max_items: 100,
                    max_age: std::time::Duration::from_secs(3600), // 1 hour
                    persist_to_db: false,
                    index_content: false,
                };

                if notify {
                    let mut coordinator = self.coordinator.write().await;
                    coordinator
                        .add_source(source, config, template_name)
                        .await?;

                    Ok(DataSourceOutput {
                        success: true,
                        result: DataSourceResult::Success {
                            message: format!("Watching file with notifications: {}", path),
                        },
                    })
                } else {
                    Ok(DataSourceOutput {
                        success: true,
                        result: DataSourceResult::Success {
                            message: format!("File watching not yet implemented: {}", path),
                        },
                    })
                }
            }

            DataSourceOperation::ListSources => {
                let coordinator = self.coordinator.read().await;
                let sources = coordinator.list_sources().await;

                let source_infos: Vec<SourceInfo> = sources
                    .into_iter()
                    .map(|(id, source_type)| SourceInfo {
                        source_id: id,
                        source_type,
                        status: "active".to_string(), // TODO: Get actual status
                    })
                    .collect();

                Ok(DataSourceOutput {
                    success: true,
                    result: DataSourceResult::Sources(source_infos),
                })
            }

            DataSourceOperation::GetBufferStats { source_id } => {
                let coordinator = self.coordinator.read().await;
                let stats = coordinator.get_buffer_stats(&source_id).await?;

                Ok(DataSourceOutput {
                    success: true,
                    result: DataSourceResult::BufferStats(stats),
                })
            }

            DataSourceOperation::PauseSource { source_id } => {
                let coordinator = self.coordinator.read().await;
                coordinator.pause_source(&source_id).await?;

                Ok(DataSourceOutput {
                    success: true,
                    result: DataSourceResult::Success {
                        message: format!("Source '{}' paused", source_id),
                    },
                })
            }

            DataSourceOperation::ResumeSource { source_id } => {
                let coordinator = self.coordinator.read().await;
                coordinator.resume_source(&source_id).await?;

                Ok(DataSourceOutput {
                    success: true,
                    result: DataSourceResult::Success {
                        message: format!("Source '{}' resumed", source_id),
                    },
                })
            }

            _ => Ok(DataSourceOutput {
                success: false,
                result: DataSourceResult::Error {
                    error: "Operation not implemented yet".to_string(),
                },
            }),
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
