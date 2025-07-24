//! Types for agent export/import

use chrono::{DateTime, Utc};
use cid::Cid;
use libipld::Ipld;
use serde::{Deserialize, Serialize};

use crate::{AgentId, MessageId};

/// Manifest describing an agent export
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportManifest {
    /// Export format version
    pub version: u32,
    
    /// When this export was created
    pub exported_at: DateTime<Utc>,
    
    /// Agent being exported
    pub agent_id: AgentId,
    
    /// Export statistics
    pub stats: ExportStats,
    
    /// Root CID of the agent block
    pub agent_cid: Cid,
    
    /// Root CID of the memory collection
    pub memories_cid: Option<Cid>,
    
    /// Root CID of the first message chunk
    pub messages_cid: Option<Cid>,
    
    /// Compression settings if any
    pub compression: Option<CompressionSettings>,
}

/// Statistics about an export
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportStats {
    /// Number of memory blocks exported
    pub memory_count: u64,
    
    /// Total number of messages exported
    pub message_count: u64,
    
    /// Number of message chunks
    pub chunk_count: u64,
    
    /// Total blocks in the CAR file
    pub total_blocks: u64,
    
    /// Uncompressed size in bytes
    pub uncompressed_size: u64,
    
    /// Compressed size if compression was used
    pub compressed_size: Option<u64>,
}

/// A chunk of messages for streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageChunk {
    /// Sequential chunk ID
    pub chunk_id: u32,
    
    /// Snowflake ID of first message
    pub start_position: String,
    
    /// Snowflake ID of last message
    pub end_position: String,
    
    /// Messages in this chunk
    pub messages: Vec<MessageBlock>,
    
    /// CID of next chunk if any
    pub next_chunk: Option<Cid>,
}

/// Simplified message for export
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageBlock {
    pub id: MessageId,
    pub content: Ipld,
    pub metadata: Ipld,
}

/// Compression settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionSettings {
    pub algorithm: String,
    pub level: i32,
}

/// Options for chunking messages
#[derive(Debug, Clone)]
pub struct ChunkingStrategy {
    /// Maximum messages per chunk
    pub chunk_size: usize,
    
    /// Whether to compress individual chunks
    pub compress_chunks: bool,
}