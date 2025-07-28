use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Result;

/// Core trait for data sources that agents can consume from
#[async_trait]
pub trait DataSource: Send + Sync {
    type Item: Serialize + for<'de> Deserialize<'de> + Send;
    type Filter: Send;
    type Cursor: Serialize + for<'de> Deserialize<'de> + Send + Clone;

    /// Unique identifier for this source
    fn source_id(&self) -> &str;

    /// Pull next items with optional cursor
    async fn pull(&mut self, limit: usize, after: Option<Self::Cursor>) -> Result<Vec<Self::Item>>;

    /// Subscribe from a specific cursor position
    async fn subscribe(
        &mut self,
        from: Option<Self::Cursor>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamEvent<Self::Item, Self::Cursor>>> + Send + Unpin>>;

    /// Apply filtering/rate limiting
    fn set_filter(&mut self, filter: Self::Filter);

    /// Get current cursor position
    fn current_cursor(&self) -> Option<Self::Cursor>;

    /// Get source metadata (type, status, stats)
    fn metadata(&self) -> DataSourceMetadata;
}

/// Event from a streaming data source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent<T, C> {
    pub item: T,
    pub cursor: C,
    pub timestamp: DateTime<Utc>,
}

/// Metadata about a data source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceMetadata {
    pub source_type: String,
    pub status: DataSourceStatus,
    pub items_processed: u64,
    pub last_item_time: Option<DateTime<Utc>>,
    pub error_count: u64,
    pub custom: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DataSourceStatus {
    Active,
    Paused,
    Error(String),
    Disconnected,
}
