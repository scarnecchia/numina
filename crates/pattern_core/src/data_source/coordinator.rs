use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;

use crate::context::message_router::AgentMessageRouter;
use crate::embeddings::EmbeddingProvider;
use crate::error::Result;
use crate::prompt_template::TemplateRegistry;

use super::buffer::{BufferConfig, StreamBuffer};
use super::traits::{DataSource, StreamEvent};

use async_trait::async_trait;
use futures::Stream;

/// Event that prompts an agent with new data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataIngestionEvent {
    pub source_id: String,
    pub items: Vec<Value>,
    pub cursor: Option<Value>,
    pub metadata: HashMap<String, Value>,
    pub template_name: String,
}

/// Manages multiple data sources and routes their output to agents
#[derive(Debug)]
pub struct DataIngestionCoordinator {
    sources: Arc<RwLock<HashMap<String, SourceHandle>>>,
    agent_router: AgentMessageRouter,
    template_registry: Arc<TemplateRegistry>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

/// Type-erased wrapper for concrete data sources
struct TypeErasedSource<S>
where
    S: DataSource + 'static,
{
    inner: S,
}

impl<S> TypeErasedSource<S>
where
    S: DataSource + 'static,
    S::Item: Serialize + for<'de> Deserialize<'de> + Send,
    S::Filter: Serialize + for<'de> Deserialize<'de> + Send,
    S::Cursor: Serialize + for<'de> Deserialize<'de> + Send,
{
    fn new(source: S) -> Self {
        Self { inner: source }
    }
}

#[async_trait]
impl<S> DataSource for TypeErasedSource<S>
where
    S: DataSource + 'static,
    S::Item: Serialize + for<'de> Deserialize<'de> + Send,
    S::Filter: Serialize + for<'de> Deserialize<'de> + Send,
    S::Cursor: Serialize + for<'de> Deserialize<'de> + Send,
{
    type Item = Value;
    type Filter = Value;
    type Cursor = Value;

    fn source_id(&self) -> &str {
        self.inner.source_id()
    }

    fn metadata(&self) -> super::traits::DataSourceMetadata {
        self.inner.metadata()
    }

    async fn pull(&mut self, limit: usize, after: Option<Self::Cursor>) -> Result<Vec<Self::Item>> {
        // Convert Value cursor to concrete type
        let cursor = if let Some(v) = after {
            Some(
                serde_json::from_value(v).map_err(|e| crate::CoreError::SerializationError {
                    data_type: "cursor".to_string(),
                    cause: e,
                })?,
            )
        } else {
            None
        };

        // Pull from inner source
        let items = self.inner.pull(limit, cursor).await?;

        // Convert concrete items to Values
        items
            .into_iter()
            .map(|item| {
                serde_json::to_value(item).map_err(|e| crate::CoreError::SerializationError {
                    data_type: "item".to_string(),
                    cause: e,
                })
            })
            .collect()
    }

    async fn subscribe(
        &mut self,
        from: Option<Self::Cursor>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamEvent<Self::Item, Self::Cursor>>> + Send + Unpin>>
    {
        use futures::stream::StreamExt;

        // Convert Value cursor to concrete type
        let cursor = if let Some(v) = from {
            Some(
                serde_json::from_value(v).map_err(|e| crate::CoreError::SerializationError {
                    data_type: "cursor".to_string(),
                    cause: e,
                })?,
            )
        } else {
            None
        };

        // Get stream from inner source
        let inner_stream = self.inner.subscribe(cursor).await?;

        // Map concrete events to Value events
        let mapped_stream = inner_stream.map(|event_result| {
            event_result.and_then(|event| {
                let value_item = serde_json::to_value(event.item).map_err(|e| {
                    crate::CoreError::SerializationError {
                        data_type: "item".to_string(),
                        cause: e,
                    }
                })?;
                let value_cursor = serde_json::to_value(event.cursor).map_err(|e| {
                    crate::CoreError::SerializationError {
                        data_type: "cursor".to_string(),
                        cause: e,
                    }
                })?;
                Ok(StreamEvent {
                    item: value_item,
                    cursor: value_cursor,
                    timestamp: event.timestamp,
                })
            })
        });

        Ok(Box::new(mapped_stream))
    }

    fn set_filter(&mut self, filter: Self::Filter) {
        // Convert Value filter to concrete type if possible
        if let Ok(concrete_filter) = serde_json::from_value::<S::Filter>(filter) {
            self.inner.set_filter(concrete_filter);
        }
        // Otherwise ignore - best effort
    }

    fn current_cursor(&self) -> Option<Self::Cursor> {
        self.inner
            .current_cursor()
            .map(|cursor| serde_json::to_value(cursor).unwrap_or(Value::Null))
    }
}

struct SourceHandle {
    source: Box<dyn DataSource<Item = Value, Filter = Value, Cursor = Value>>,
    buffer_config: BufferConfig,
    buffer: Box<dyn std::any::Any + Send + Sync>,
    last_cursor: Option<Value>,
    template_name: String,
}

impl std::fmt::Debug for SourceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceHandle")
            .field("source_id", &self.source.source_id())
            .field("buffer_config", &self.buffer_config)
            .field("last_cursor", &self.last_cursor)
            .field("template_name", &self.template_name)
            .finish()
    }
}

impl DataIngestionCoordinator {
    pub fn new(
        agent_router: AgentMessageRouter,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Result<Self> {
        let template_registry = TemplateRegistry::new().with_defaults()?;

        Ok(Self {
            sources: Arc::new(RwLock::new(HashMap::new())),
            agent_router,
            template_registry: Arc::new(template_registry),
            embedding_provider,
        })
    }

    /// Get the embedding provider if available
    pub fn embedding_provider(&self) -> Option<Arc<dyn EmbeddingProvider>> {
        self.embedding_provider.clone()
    }

    /// Register a source and start monitoring
    pub async fn add_source<S>(
        &mut self,
        source: S,
        config: BufferConfig,
        template_name: String,
    ) -> Result<()>
    where
        S: DataSource + 'static,
        S::Item: Serialize + for<'de> Deserialize<'de> + Send,
        S::Filter: Serialize + for<'de> Deserialize<'de> + Send,
        S::Cursor: Serialize + for<'de> Deserialize<'de> + Send,
    {
        let source_id = source.source_id().to_string();

        // Create a type-erased wrapper
        let erased_source = Box::new(TypeErasedSource::new(source));

        // Create buffer based on config
        let buffer: Box<dyn std::any::Any + Send + Sync> = Box::new(
            StreamBuffer::<Value, Value>::new(config.max_items, config.max_age),
        );

        let handle = SourceHandle {
            source: erased_source,
            buffer_config: config,
            buffer,
            last_cursor: None,
            template_name,
        };

        let mut sources = self.sources.write().await;
        sources.insert(source_id.clone(), handle);

        // TODO: Start monitoring task for this source

        Ok(())
    }

    /// Handle incoming data and prompt agent if needed
    pub async fn handle_ingestion_event(&self, event: DataIngestionEvent) -> Result<()> {
        // Get template
        let template = self
            .template_registry
            .get(&event.template_name)
            .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                tool_name: "data_ingestion".to_string(),
                cause: format!("Template '{}' not found", event.template_name),
                parameters: serde_json::json!({ "template": event.template_name }),
            })?;

        // Build context for template
        let mut context = event.metadata.clone();
        context.insert("source_id".to_string(), serde_json::json!(event.source_id));
        context.insert(
            "item_count".to_string(),
            serde_json::json!(event.items.len()),
        );

        // If single item with common fields, extract them
        if event.items.len() == 1 {
            if let Some(item) = event.items.first() {
                // Extract common fields like author, content, text
                if let Some(obj) = item.as_object() {
                    for (key, value) in obj {
                        if matches!(
                            key.as_str(),
                            "author" | "content" | "text" | "title" | "uri"
                        ) {
                            context.insert(key.clone(), value.clone());
                        }
                    }
                }
            }
        }

        // Render prompt
        let prompt = template.render(&context)?;

        // Send to agent via router
        let target = crate::tool::builtin::MessageTarget {
            target_type: crate::tool::builtin::TargetType::User,
            target_id: None, // Will go to default user endpoint
        };

        self.agent_router
            .send_message(
                target,
                prompt,
                Some(serde_json::json!({
                    "source": "data_ingestion",
                    "source_id": event.source_id,
                    "items": event.items,
                })),
            )
            .await?;

        Ok(())
    }

    /// List all registered sources
    pub async fn list_sources(&self) -> Vec<(String, String)> {
        let sources = self.sources.read().await;
        sources
            .iter()
            .map(|(id, handle)| (id.clone(), handle.source.metadata().source_type))
            .collect()
    }

    /// Pause a source
    pub async fn pause_source(&self, source_id: &str) -> Result<()> {
        let mut sources = self.sources.write().await;

        if let Some(_handle) = sources.get_mut(source_id) {
            // TODO: Implement pause logic
            Ok(())
        } else {
            Err(crate::CoreError::ToolExecutionFailed {
                tool_name: "data_ingestion".to_string(),
                cause: format!("Source '{}' not found", source_id),
                parameters: serde_json::json!({ "source_id": source_id }),
            })
        }
    }

    /// Resume a source
    pub async fn resume_source(&self, source_id: &str) -> Result<()> {
        let mut sources = self.sources.write().await;

        if let Some(_handle) = sources.get_mut(source_id) {
            // TODO: Implement resume logic
            Ok(())
        } else {
            Err(crate::CoreError::ToolExecutionFailed {
                tool_name: "data_ingestion".to_string(),
                cause: format!("Source '{}' not found", source_id),
                parameters: serde_json::json!({ "source_id": source_id }),
            })
        }
    }

    /// Get buffer stats for a source
    pub async fn get_buffer_stats(&self, source_id: &str) -> Result<Value> {
        let sources = self.sources.read().await;

        if let Some(handle) = sources.get(source_id) {
            // Downcast buffer to get stats
            if let Some(buffer) = handle.buffer.downcast_ref::<StreamBuffer<Value, Value>>() {
                Ok(serde_json::to_value(buffer.stats()).map_err(|e| {
                    crate::CoreError::SerializationError {
                        data_type: "buffer stats".to_string(),
                        cause: e,
                    }
                })?)
            } else {
                Ok(serde_json::json!({
                    "error": "Buffer type mismatch"
                }))
            }
        } else {
            Err(crate::CoreError::ToolExecutionFailed {
                tool_name: "data_ingestion".to_string(),
                cause: format!("Source '{}' not found", source_id),
                parameters: serde_json::json!({ "source_id": source_id }),
            })
        }
    }
}
