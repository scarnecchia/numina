use async_trait::async_trait;
use chrono::{DateTime, Utc};
use compact_str::CompactString;
use futures::stream::Stream;
use pattern_core::{
    CoreError, Result,
    data_source::{
        BufferConfig, DataSource, DataSourceMetadata, StreamEvent, traits::DataSourceStatus,
    },
    memory::MemoryBlock,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serenity::{
    http::Http,
    model::{channel::Message, id::ChannelId},
};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Discord message event for data source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordMessage {
    pub message_id: String,
    pub channel_id: String,
    pub author_id: String,
    pub author_name: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub is_bot: bool,
    pub mentions: Vec<String>,
    pub reply_to: Option<String>,
}

impl From<Message> for DiscordMessage {
    fn from(msg: Message) -> Self {
        Self {
            message_id: msg.id.to_string(),
            channel_id: msg.channel_id.to_string(),
            author_id: msg.author.id.to_string(),
            author_name: msg.author.name.clone(),
            content: msg.content.clone(),
            timestamp: DateTime::<Utc>::from_timestamp(msg.timestamp.unix_timestamp(), 0)
                .unwrap_or_else(Utc::now),
            is_bot: msg.author.bot,
            mentions: msg.mentions.iter().map(|u| u.id.to_string()).collect(),
            reply_to: msg.referenced_message.as_ref().map(|m| m.id.to_string()),
        }
    }
}

/// Discord cursor for tracking position in message stream
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordCursor {
    pub channel_id: String,
    pub last_message_id: String,
    pub timestamp: DateTime<Utc>,
}

/// Discord filter for message filtering
#[derive(Debug, Clone)]
pub struct DiscordFilter {
    pub include_bots: bool,
    pub channel_ids: Vec<String>,
    pub author_ids: Vec<String>,
}

impl Default for DiscordFilter {
    fn default() -> Self {
        Self {
            include_bots: false,
            channel_ids: Vec::new(),
            author_ids: Vec::new(),
        }
    }
}

/// Configuration for Discord data source
#[derive(Debug, Clone)]
pub struct DiscordConfig {
    /// Maximum message history to fetch on startup
    pub scrollback_limit: usize,
    /// Include bot messages in history
    pub include_bots: bool,
    /// Filter to specific channel IDs (empty = all accessible channels)
    pub channel_filter: Vec<String>,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            scrollback_limit: 100,
            include_bots: false,
            channel_filter: Vec::new(),
        }
    }
}

/// Discord data source for message history and real-time events
pub struct DiscordDataSource {
    source_id: String,
    http: Arc<Http>,
    config: DiscordConfig,
    filter: DiscordFilter,
    receiver: Option<mpsc::UnboundedReceiver<DiscordMessage>>,
    current_cursor: Option<DiscordCursor>,
    items_processed: u64,
    error_count: u64,
    notifications_enabled: bool,
}

impl DiscordDataSource {
    /// Create a new Discord data source
    pub fn new(token: String, config: DiscordConfig) -> Self {
        let http = Arc::new(Http::new(&token));
        let filter = DiscordFilter {
            include_bots: config.include_bots,
            channel_ids: config.channel_filter.clone(),
            author_ids: Vec::new(),
        };

        Self {
            source_id: "discord".to_string(),
            http,
            config,
            filter,
            receiver: None,
            current_cursor: None,
            items_processed: 0,
            error_count: 0,
            notifications_enabled: true,
        }
    }

    /// Fetch message history for a channel
    pub async fn fetch_channel_history(
        &self,
        channel_id: ChannelId,
        limit: usize,
    ) -> Result<Vec<DiscordMessage>> {
        let messages = channel_id
            .messages(
                &self.http,
                serenity::builder::GetMessages::new().limit(limit as u8),
            )
            .await
            .map_err(|e| CoreError::DataSourceError {
                source_name: "discord".to_string(),
                operation: "fetch_channel_history".to_string(),
                cause: format!("Failed to fetch channel history: {}", e),
            })?;

        let mut history = Vec::new();
        for msg in messages {
            if !self.filter.include_bots && msg.author.bot {
                continue;
            }
            history.push(DiscordMessage::from(msg));
        }

        Ok(history)
    }

    /// Start streaming Discord messages
    pub fn start_stream(&mut self) -> mpsc::UnboundedSender<DiscordMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.receiver = Some(rx);
        tx
    }
}

#[async_trait]
impl DataSource for DiscordDataSource {
    type Item = DiscordMessage;
    type Filter = DiscordFilter;
    type Cursor = DiscordCursor;

    fn source_id(&self) -> &str {
        &self.source_id
    }

    async fn pull(&mut self, limit: usize, after: Option<Self::Cursor>) -> Result<Vec<Self::Item>> {
        // Extract channel ID from cursor or use first configured channel
        let channel_id = if let Some(cursor) = after.as_ref() {
            cursor.channel_id.parse::<u64>().ok().map(ChannelId::new)
        } else if !self.config.channel_filter.is_empty() {
            self.config.channel_filter[0]
                .parse::<u64>()
                .ok()
                .map(ChannelId::new)
        } else {
            None
        };

        let channel_id = channel_id.ok_or_else(|| CoreError::DataSourceError {
            source_name: "discord".to_string(),
            operation: "pull".to_string(),
            cause: "No channel ID specified for pull".to_string(),
        })?;

        let messages = self.fetch_channel_history(channel_id, limit).await?;

        // Update cursor if we got messages
        if let Some(last_msg) = messages.last() {
            self.current_cursor = Some(DiscordCursor {
                channel_id: last_msg.channel_id.clone(),
                last_message_id: last_msg.message_id.clone(),
                timestamp: last_msg.timestamp,
            });
        }

        self.items_processed += messages.len() as u64;
        Ok(messages)
    }

    async fn subscribe(
        &mut self,
        _from: Option<Self::Cursor>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamEvent<Self::Item, Self::Cursor>>> + Send + Unpin>>
    {
        if self.receiver.is_none() {
            warn!("Discord stream not started. Call start_stream() first.");
            // Return empty stream
            return Ok(Box::new(futures::stream::empty()));
        }

        let receiver = self.receiver.take().unwrap();
        Ok(Box::new(DiscordMessageStream { receiver }))
    }

    fn set_filter(&mut self, filter: Self::Filter) {
        self.filter = filter;
    }

    fn current_cursor(&self) -> Option<Self::Cursor> {
        self.current_cursor.clone()
    }

    fn metadata(&self) -> DataSourceMetadata {
        let mut custom = HashMap::new();
        custom.insert(
            "channel_count".to_string(),
            Value::Number(self.config.channel_filter.len().into()),
        );
        custom.insert(
            "include_bots".to_string(),
            Value::Bool(self.config.include_bots),
        );

        DataSourceMetadata {
            source_type: "discord".to_string(),
            status: if self.receiver.is_some() {
                DataSourceStatus::Active
            } else {
                DataSourceStatus::Disconnected
            },
            items_processed: self.items_processed,
            last_item_time: self.current_cursor.as_ref().map(|c| c.timestamp),
            error_count: self.error_count,
            custom,
        }
    }

    fn buffer_config(&self) -> BufferConfig {
        BufferConfig {
            max_items: 1000,
            max_age: std::time::Duration::from_secs(3600), // 1 hour
            persist_to_db: false,
            index_content: false,
            notify_changes: true,
        }
    }

    async fn format_notification(
        &self,
        item: &Self::Item,
    ) -> Option<(String, Vec<(CompactString, MemoryBlock)>)> {
        let mut notification = format!(
            "Discord message from {} in channel {}:\n",
            item.author_name, item.channel_id
        );

        if let Some(reply_to) = &item.reply_to {
            notification.push_str(&format!("(Reply to message {})\n", reply_to));
        }

        notification.push_str(&item.content);

        if !item.mentions.is_empty() {
            notification.push_str(&format!("\nMentions: {:?}", item.mentions));
        }

        // Create memory block for this message
        let memory_block = discord_message_to_memory_block(item);
        let memory_blocks = vec![(
            CompactString::new(format!("discord_msg_{}", item.message_id)),
            memory_block,
        )];

        Some((notification, memory_blocks))
    }

    fn set_notifications_enabled(&mut self, enabled: bool) {
        self.notifications_enabled = enabled;
    }

    fn notifications_enabled(&self) -> bool {
        self.notifications_enabled
    }
}

/// Stream wrapper for Discord messages
struct DiscordMessageStream {
    receiver: mpsc::UnboundedReceiver<DiscordMessage>,
}

impl Stream for DiscordMessageStream {
    type Item = Result<StreamEvent<DiscordMessage, DiscordCursor>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.receiver.poll_recv(cx) {
            Poll::Ready(Some(msg)) => {
                let cursor = DiscordCursor {
                    channel_id: msg.channel_id.clone(),
                    last_message_id: msg.message_id.clone(),
                    timestamp: msg.timestamp,
                };

                let event = StreamEvent {
                    item: msg,
                    cursor,
                    timestamp: Utc::now(),
                };

                Poll::Ready(Some(Ok(event)))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Helper to create memory blocks from Discord messages
pub fn discord_message_to_memory_block(msg: &DiscordMessage) -> MemoryBlock {
    let content = format!(
        "[{}] {}: {}",
        msg.timestamp.format("%Y-%m-%d %H:%M:%S"),
        msg.author_name,
        msg.content
    );

    MemoryBlock::new(format!("discord_msg_{}", msg.message_id), content)
}

/// Builder for Discord data source with scrollback buffer
pub struct DiscordDataSourceBuilder {
    token: String,
    config: DiscordConfig,
    initial_channels: Vec<u64>,
}

impl DiscordDataSourceBuilder {
    pub fn new(token: String) -> Self {
        Self {
            token,
            config: DiscordConfig::default(),
            initial_channels: Vec::new(),
        }
    }

    pub fn with_scrollback(mut self, limit: usize) -> Self {
        self.config.scrollback_limit = limit;
        self
    }

    pub fn include_bots(mut self, include: bool) -> Self {
        self.config.include_bots = include;
        self
    }

    pub fn with_channels(mut self, channel_ids: Vec<u64>) -> Self {
        self.initial_channels = channel_ids.clone();
        self.config.channel_filter = channel_ids.iter().map(|id| id.to_string()).collect();
        self
    }

    pub async fn build(self) -> Result<DiscordDataSource> {
        let source = DiscordDataSource::new(self.token, self.config);

        // Pre-fetch history for initial channels
        for channel_id in self.initial_channels {
            let channel = ChannelId::new(channel_id);
            match source
                .fetch_channel_history(channel, source.config.scrollback_limit)
                .await
            {
                Ok(messages) => {
                    info!(
                        "Fetched {} messages from Discord channel {}",
                        messages.len(),
                        channel_id
                    );
                }
                Err(e) => {
                    warn!("Failed to fetch history for channel {}: {}", channel_id, e);
                }
            }
        }

        Ok(source)
    }
}
