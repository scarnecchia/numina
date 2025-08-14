//! Types for agent export/import

use chrono::{DateTime, Utc};
use cid::Cid;
use serde::{Deserialize, Serialize};

use crate::{AgentId, message::Message};

/// Manifest describing any export - this is always the root of a CAR file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportManifest {
    /// Export format version
    pub version: u32,

    /// When this export was created
    pub exported_at: DateTime<Utc>,

    /// Type of export
    pub export_type: ExportType,

    /// Export statistics
    pub stats: ExportStats,

    /// CID of the actual export data
    pub data_cid: Cid,
}

/// Type of data being exported
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportType {
    Agent,
    Group,
    Constellation,
}

/// Agent export with all related data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExport {
    /// The agent record
    pub agent: crate::agent::AgentRecord,

    /// CIDs of message chunks
    pub message_chunk_cids: Vec<Cid>,

    /// CIDs of memory chunks
    pub memory_chunk_cids: Vec<Cid>,
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

    /// Messages in this chunk with their relations (includes position)
    pub messages: Vec<(Message, crate::message::AgentMessageRelation)>,

    /// CID of next chunk if any
    pub next_chunk: Option<Cid>,
}

/// A chunk of memories for streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryChunk {
    /// Sequential chunk ID
    pub chunk_id: u32,

    /// Memories in this chunk with their agent relations (includes access_level)
    pub memories: Vec<(
        crate::memory::MemoryBlock,
        crate::agent::AgentMemoryRelation,
    )>,

    /// CID of next chunk if any
    pub next_chunk: Option<Cid>,
}

/// A complete constellation export with all relationships
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstellationExport {
    /// The constellation record itself
    pub constellation: crate::coordination::groups::Constellation,

    /// All groups in this constellation with their full membership data
    pub groups: Vec<GroupExport>,

    /// CIDs of all agent exports in this constellation
    pub agent_export_cids: Vec<(AgentId, Cid)>,
}

/// A complete group export with all relationships
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupExport {
    /// The group record itself
    pub group: crate::coordination::groups::AgentGroup,

    /// CIDs of member agent exports (agents are exported separately)
    pub member_agent_cids: Vec<(AgentId, Cid)>,
}

/// Compression settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct CompressionSettings {
    pub algorithm: String,
    pub level: i32,
}

/// Options for chunking messages
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ChunkingStrategy {
    /// Maximum messages per chunk
    pub chunk_size: usize,

    /// Whether to compress individual chunks
    pub compress_chunks: bool,
}
