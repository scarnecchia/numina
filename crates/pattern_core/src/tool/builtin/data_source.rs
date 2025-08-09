use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::data_source::DataIngestionCoordinator;
use crate::embeddings::EmbeddingProvider;
use crate::error::Result;
use crate::tool::{AiTool, ToolRegistry};

fn default_limit() -> i64 {
    10
}

/// Tool for managing data sources that feed into agents
#[derive(Debug, Clone)]
pub struct DataSourceTool<E: EmbeddingProvider + Clone> {
    coordinator: Arc<tokio::sync::RwLock<DataIngestionCoordinator<E>>>,
}

impl<E: EmbeddingProvider + Clone> DataSourceTool<E> {
    pub fn new(coordinator: Arc<tokio::sync::RwLock<DataIngestionCoordinator<E>>>) -> Self {
        Self { coordinator }
    }
}

/// Operation types for data source management
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceOperation {
    Read,
    Search,
    Monitor,
    Pause,
    Resume,
    List,
}

/// Input for data source operations
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DataSourceInput {
    /// The operation to perform
    pub operation: DataSourceOperation,

    /// Source ID to operate on (not needed for list)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,

    /// Query for read/search operations
    /// For read: can be "lines 10-20", "last 50", "chunk 1024"
    /// For search: the search query
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,

    /// Maximum number of results (for read/search operations)
    #[schemars(default, with = "i64")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,

    /// Request another turn after this tool executes
    #[serde(default)]
    pub request_heartbeat: bool,
}

/// Output from data source operations
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DataSourceOutput {
    /// Whether the operation succeeded
    pub success: bool,

    /// Human-readable message about the result
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// The actual content returned (for read/search/list operations)
    pub content: serde_json::Value,

    /// Whether another turn was requested
    #[serde(default)]
    pub request_heartbeat: bool,
}

#[async_trait]
impl<E: EmbeddingProvider + Clone + 'static> AiTool for DataSourceTool<E> {
    type Input = DataSourceInput;
    type Output = DataSourceOutput;

    fn name(&self) -> &str {
        "data_source"
    }

    fn description(&self) -> &str {
        "Manage data sources that feed information to the agent"
    }

    fn usage_rule(&self) -> Option<&'static str> {
        Some(
            r#"Use this tool to interact with configured data sources.

Operations:
- read: Read from a source. Use query for filtering (e.g., "lines 10-20", "last 50")
- search: Search within a source (if supported)
- monitor: Start receiving notifications from a source when new data arrives
- pause: Stop notifications temporarily
- resume: Resume notifications
- list: Show all configured sources

Sources must be configured separately before they can be used."#,
        )
    }

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        match input.operation {
            DataSourceOperation::Read => {
                let source_id =
                    input
                        .source_id
                        .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                            tool_name: self.name().to_string(),
                            cause: "Missing required parameter 'source_id' for read".to_string(),
                            parameters: json!({}),
                        })?;

                // Parse query for read modifiers
                let mut limit = input.limit.unwrap_or(default_limit()) as usize;
                let mut offset = 0;
                let mut cursor = None;

                if let Some(query) = &input.query {
                    if query.starts_with("last ") {
                        if let Ok(n) = query[5..].trim().parse::<usize>() {
                            limit = n;
                        }
                    } else if query.starts_with("lines ") {
                        // Parse line range like "lines 10-20"
                        if let Some(range) = query[6..].trim().split_once('-') {
                            if let (Ok(start), Ok(end)) =
                                (range.0.parse::<usize>(), range.1.parse::<usize>())
                            {
                                offset = start.saturating_sub(1); // Convert to 0-based
                                limit = end - start + 1;
                            }
                        }
                    } else if query.starts_with("after ") {
                        // Parse cursor like "after <cursor_value>"
                        let cursor_str = query[6..].trim();
                        cursor = Some(json!(cursor_str));
                    }
                }

                // Read from source
                let coordinator = self.coordinator.read().await;
                let items = coordinator.read_source(&source_id, limit, cursor).await?;

                // Apply offset if needed
                let items = if offset > 0 {
                    items.into_iter().skip(offset).collect()
                } else {
                    items
                };

                Ok(DataSourceOutput {
                    success: true,
                    message: Some(format!(
                        "Read {} items from source '{}'",
                        items.len(),
                        source_id
                    )),
                    content: json!(items),
                    request_heartbeat: input.request_heartbeat,
                })
            }

            DataSourceOperation::Search => {
                let source_id =
                    input
                        .source_id
                        .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                            tool_name: self.name().to_string(),
                            cause: "Missing required parameter 'source_id' for search".to_string(),
                            parameters: json!({}),
                        })?;

                let query = input
                    .query
                    .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                        tool_name: self.name().to_string(),
                        cause: "Missing required parameter 'query' for search".to_string(),
                        parameters: json!({}),
                    })?;

                // Search in source
                let coordinator = self.coordinator.read().await;
                let results = coordinator
                    .search_source(
                        &source_id,
                        &query,
                        input.limit.unwrap_or(default_limit()) as usize,
                    )
                    .await?;

                Ok(DataSourceOutput {
                    success: true,
                    message: Some(format!(
                        "Found {} results for '{}' in source '{}'",
                        results.len(),
                        query,
                        source_id
                    )),
                    content: json!(results),
                    request_heartbeat: input.request_heartbeat,
                })
            }

            DataSourceOperation::Monitor => {
                let source_id =
                    input
                        .source_id
                        .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                            tool_name: self.name().to_string(),
                            cause: "Missing required parameter 'source_id' for monitor".to_string(),
                            parameters: json!({}),
                        })?;

                // Start monitoring
                let mut coordinator = self.coordinator.write().await;
                coordinator.start_monitoring(&source_id).await?;

                Ok(DataSourceOutput {
                    success: true,
                    message: Some(format!("Started monitoring source '{}'", source_id)),
                    content: json!(null),
                    request_heartbeat: input.request_heartbeat,
                })
            }

            DataSourceOperation::Pause => {
                let source_id =
                    input
                        .source_id
                        .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                            tool_name: self.name().to_string(),
                            cause: "Missing required parameter 'source_id' for pause".to_string(),
                            parameters: json!({}),
                        })?;

                let coordinator = self.coordinator.read().await;
                coordinator.pause_source(&source_id).await?;

                Ok(DataSourceOutput {
                    success: true,
                    message: Some(format!("Paused notifications from source '{}'", source_id)),
                    content: json!(null),
                    request_heartbeat: input.request_heartbeat,
                })
            }

            DataSourceOperation::Resume => {
                let source_id =
                    input
                        .source_id
                        .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                            tool_name: self.name().to_string(),
                            cause: "Missing required parameter 'source_id' for resume".to_string(),
                            parameters: json!({}),
                        })?;

                let coordinator = self.coordinator.read().await;
                coordinator.resume_source(&source_id).await?;

                Ok(DataSourceOutput {
                    success: true,
                    message: Some(format!("Resumed notifications from source '{}'", source_id)),
                    content: json!(null),
                    request_heartbeat: input.request_heartbeat,
                })
            }

            DataSourceOperation::List => {
                let coordinator = self.coordinator.read().await;
                let sources = coordinator.list_sources().await;
                let source_list: Vec<_> = sources
                    .into_iter()
                    .map(|(id, source_type)| {
                        json!({
                            "id": id,
                            "type": source_type,
                        })
                    })
                    .collect();

                Ok(DataSourceOutput {
                    success: true,
                    message: Some(format!("Found {} configured sources", source_list.len())),
                    content: json!(source_list),
                    request_heartbeat: input.request_heartbeat,
                })
            }
        }
    }
}

/// Register the data source tool with the registry
pub fn register_data_source_tool<E: EmbeddingProvider + Clone + 'static>(
    registry: &mut ToolRegistry,
    coordinator: Arc<tokio::sync::RwLock<DataIngestionCoordinator<E>>>,
) -> Result<()> {
    registry.register(DataSourceTool::new(coordinator));
    Ok(())
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_query_parsing() {
        // Test "last N" parsing
        let query = "last 50";
        assert!(query.starts_with("last "));
        assert_eq!(query[5..].trim().parse::<i64>().unwrap(), 50);

        // Test "lines X-Y" parsing
        let query = "lines 10-20";
        assert!(query.starts_with("lines "));
        let range = query[6..].trim().split_once('-').unwrap();
        assert_eq!(range.0.parse::<i64>().unwrap(), 10);
        assert_eq!(range.1.parse::<i64>().unwrap(), 20);
    }
}
