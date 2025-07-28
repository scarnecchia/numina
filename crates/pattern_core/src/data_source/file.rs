use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::embeddings::EmbeddingProvider;
use crate::error::Result;

use super::traits::{DataSource, DataSourceMetadata, DataSourceStatus, StreamEvent};

/// File-specific implementation
pub struct FileDataSource {
    pub path: PathBuf,
    pub storage_mode: FileStorageMode,
    pub watch: bool,
    source_id: String,
    current_cursor: Option<FileCursor>,
    filter: FileFilter,
    metadata: DataSourceMetadata,
}

#[derive(Debug, Clone)]
pub enum FileStorageMode {
    Ephemeral,
    Indexed {
        embedding_provider: Arc<dyn EmbeddingProvider>,
        chunk_size: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileCursor {
    ModTime(SystemTime),
    LineNumber(usize),
    ByteOffset(u64),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileFilter {
    pub extensions: Option<Vec<String>>,
    pub pattern: Option<String>,
    pub max_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileItem {
    pub path: PathBuf,
    pub content: FileContent,
    pub metadata: FileMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileContent {
    Text(String),
    Lines(Vec<String>),
    Chunk {
        text: String,
        start_line: usize,
        end_line: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub size_bytes: u64,
    pub modified: SystemTime,
    pub created: Option<SystemTime>,
    pub is_dir: bool,
}

impl FileDataSource {
    pub fn new(path: impl AsRef<Path>, storage_mode: FileStorageMode) -> Self {
        let path = path.as_ref().to_path_buf();
        let source_id = format!("file:{}", path.display());

        let metadata = DataSourceMetadata {
            source_type: "file".to_string(),
            status: DataSourceStatus::Active,
            items_processed: 0,
            last_item_time: None,
            error_count: 0,
            custom: Default::default(),
        };

        Self {
            path,
            storage_mode,
            watch: false,
            source_id,
            current_cursor: None,
            filter: FileFilter::default(),
            metadata,
        }
    }

    pub fn with_watch(mut self) -> Self {
        self.watch = true;
        self
    }

    async fn read_file(&self, path: &Path) -> Result<FileItem> {
        let metadata =
            fs::metadata(path)
                .await
                .map_err(|e| crate::CoreError::ToolExecutionFailed {
                    tool_name: "file_data_source".to_string(),
                    cause: format!("Failed to read metadata: {}", e),
                    parameters: serde_json::json!({ "path": path }),
                })?;

        let file_metadata = FileMetadata {
            size_bytes: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            created: metadata.created().ok(),
            is_dir: metadata.is_dir(),
        };

        if metadata.is_dir() {
            return Ok(FileItem {
                path: path.to_path_buf(),
                content: FileContent::Text(String::new()),
                metadata: file_metadata,
            });
        }

        // Check filter
        if let Some(max_size) = self.filter.max_size_bytes {
            if metadata.len() > max_size {
                return Err(crate::CoreError::ToolExecutionFailed {
                    tool_name: "file_data_source".to_string(),
                    cause: format!("File too large: {} bytes", metadata.len()),
                    parameters: serde_json::json!({ "path": path, "size": metadata.len() }),
                });
            }
        }

        // Read content
        let content =
            match &self.storage_mode {
                FileStorageMode::Ephemeral => {
                    let text = fs::read_to_string(path).await.map_err(|e| {
                        crate::CoreError::ToolExecutionFailed {
                            tool_name: "file_data_source".to_string(),
                            cause: format!("Failed to read file: {}", e),
                            parameters: serde_json::json!({ "path": path }),
                        }
                    })?;
                    FileContent::Text(text)
                }
                FileStorageMode::Indexed { chunk_size, .. } => {
                    // Read in chunks for indexing
                    let file = fs::File::open(path).await.map_err(|e| {
                        crate::CoreError::ToolExecutionFailed {
                            tool_name: "file_data_source".to_string(),
                            cause: format!("Failed to open file: {}", e),
                            parameters: serde_json::json!({ "path": path }),
                        }
                    })?;

                    let reader = BufReader::new(file);
                    let mut lines = reader.lines();
                    let mut chunks = Vec::new();
                    let mut current_chunk = Vec::new();
                    let mut line_num = 0;

                    while let Some(line) = lines.next_line().await.map_err(|e| {
                        crate::CoreError::ToolExecutionFailed {
                            tool_name: "file_data_source".to_string(),
                            cause: format!("Failed to read line: {}", e),
                            parameters: serde_json::json!({ "path": path, "line": line_num }),
                        }
                    })? {
                        current_chunk.push(line);
                        line_num += 1;

                        if current_chunk.len() >= *chunk_size {
                            let text = current_chunk.join("\n");
                            chunks.push(FileContent::Chunk {
                                text,
                                start_line: line_num - current_chunk.len(),
                                end_line: line_num - 1,
                            });
                            current_chunk.clear();
                        }
                    }

                    // Last chunk
                    if !current_chunk.is_empty() {
                        let text = current_chunk.join("\n");
                        chunks.push(FileContent::Chunk {
                            text,
                            start_line: line_num - current_chunk.len(),
                            end_line: line_num - 1,
                        });
                    }

                    // For now, return as lines
                    FileContent::Lines(
                        chunks
                            .into_iter()
                            .map(|c| match c {
                                FileContent::Chunk { text, .. } => text,
                                _ => String::new(),
                            })
                            .collect(),
                    )
                }
            };

        Ok(FileItem {
            path: path.to_path_buf(),
            content,
            metadata: file_metadata,
        })
    }
}

#[async_trait]
impl DataSource for FileDataSource {
    type Item = FileItem;
    type Filter = FileFilter;
    type Cursor = FileCursor;

    fn source_id(&self) -> &str {
        &self.source_id
    }

    async fn pull(&mut self, limit: usize, after: Option<Self::Cursor>) -> Result<Vec<Self::Item>> {
        let mut items = Vec::new();

        // For single file
        if self.path.is_file() {
            let item = self.read_file(&self.path).await?;

            // Check cursor
            if let Some(cursor) = after {
                match cursor {
                    FileCursor::ModTime(mod_time) => {
                        if item.metadata.modified <= mod_time {
                            return Ok(vec![]);
                        }
                    }
                    _ => {} // Other cursor types not applicable for single file
                }
            }

            items.push(item);
        } else {
            // Directory listing
            let mut entries = fs::read_dir(&self.path).await.map_err(|e| {
                crate::CoreError::ToolExecutionFailed {
                    tool_name: "file_data_source".to_string(),
                    cause: format!("Failed to read directory: {}", e),
                    parameters: serde_json::json!({ "path": &self.path }),
                }
            })?;

            while let Some(entry) =
                entries
                    .next_entry()
                    .await
                    .map_err(|e| crate::CoreError::ToolExecutionFailed {
                        tool_name: "file_data_source".to_string(),
                        cause: format!("Failed to read entry: {}", e),
                        parameters: serde_json::json!({ "path": &self.path }),
                    })?
            {
                if items.len() >= limit {
                    break;
                }

                let path = entry.path();

                // Apply filter
                if let Some(extensions) = &self.filter.extensions {
                    if let Some(ext) = path.extension() {
                        if !extensions.contains(&ext.to_string_lossy().to_string()) {
                            continue;
                        }
                    }
                }

                match self.read_file(&path).await {
                    Ok(item) => items.push(item),
                    Err(e) => {
                        tracing::warn!("Failed to read file {:?}: {}", path, e);
                        self.metadata.error_count += 1;
                    }
                }
            }
        }

        self.metadata.items_processed += items.len() as u64;
        if !items.is_empty() {
            self.metadata.last_item_time = Some(chrono::Utc::now());
            self.current_cursor = Some(FileCursor::ModTime(SystemTime::now()));
        }

        Ok(items)
    }

    async fn subscribe(
        &mut self,
        _from: Option<Self::Cursor>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamEvent<Self::Item, Self::Cursor>>> + Send + Unpin>>
    {
        if !self.watch {
            return Err(crate::CoreError::ToolExecutionFailed {
                tool_name: "file_data_source".to_string(),
                cause: "File watching not enabled".to_string(),
                parameters: serde_json::json!({ "path": &self.path }),
            });
        }

        // TODO: Implement file watching with notify crate
        // For now, return empty stream
        let stream = futures::stream::empty();
        Ok(Box::new(stream))
    }

    fn set_filter(&mut self, filter: Self::Filter) {
        self.filter = filter;
    }

    fn current_cursor(&self) -> Option<Self::Cursor> {
        self.current_cursor.clone()
    }

    fn metadata(&self) -> DataSourceMetadata {
        self.metadata.clone()
    }
}
