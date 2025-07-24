//! Agent exporter implementation

use std::io::Write;

use chrono::Utc;
use cid::Cid;
use futures::StreamExt;
use iroh_car::{CarHeader, CarWriter};
use libipld::{Block, Ipld};
use multihash::{Code, MultihashDigest};
use surrealdb::Surreal;

use crate::{
    AgentId, CoreError, Result,
    agent::AgentRecord,
    db::{DbEntity, ops},
    export::{
        DEFAULT_CHUNK_SIZE, EXPORT_VERSION,
        types::{ExportManifest, ExportStats, MessageBlock, MessageChunk},
    },
    message::Message,
};

/// Options for exporting an agent
#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// Whether to include message history
    pub include_messages: bool,

    /// Maximum messages per chunk
    pub chunk_size: usize,

    /// Optional time filter for messages
    pub messages_since: Option<chrono::DateTime<chrono::Utc>>,

    /// Whether to compress the output
    #[cfg(feature = "export")]
    pub compress: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_messages: true,
            chunk_size: DEFAULT_CHUNK_SIZE,
            messages_since: None,
            #[cfg(feature = "export")]
            compress: false,
        }
    }
}

/// Agent exporter
pub struct AgentExporter<C>
where
    C: surrealdb::Connection + Clone,
{
    db: Surreal<C>,
}

impl<C> AgentExporter<C>
where
    C: surrealdb::Connection + Clone,
{
    /// Create a new exporter
    pub fn new(db: Surreal<C>) -> Self {
        Self { db }
    }

    /// Export an agent to a CAR file
    pub async fn export_to_car(
        &self,
        agent_id: AgentId,
        mut output: impl Write,
        options: ExportOptions,
    ) -> Result<ExportManifest> {
        let start_time = Utc::now();
        let mut stats = ExportStats {
            memory_count: 0,
            message_count: 0,
            chunk_count: 0,
            total_blocks: 0,
            uncompressed_size: 0,
            compressed_size: None,
        };

        // Load the agent record
        let agent = AgentRecord::load_with_relations(&self.db, agent_id.clone())
            .await?
            .ok_or_else(|| CoreError::NotFound {
                message: format!("Agent {} not found", agent_id),
            })?;

        // Convert agent to IPLD
        let agent_ipld =
            serde_json::to_value(&agent).map_err(|e| CoreError::SerializationError {
                type_name: "AgentRecord".to_string(),
                reason: e.to_string(),
            })?;
        let agent_ipld = serde_json::from_value::<Ipld>(agent_ipld).map_err(|e| {
            CoreError::SerializationError {
                type_name: "IPLD".to_string(),
                reason: e.to_string(),
            }
        })?;

        // Create agent block
        let agent_block = Block::<libipld::DefaultParams>::encode(
            libipld::cbor::DagCborCodec,
            Code::Blake3_256,
            &agent_ipld,
        )?;
        let agent_cid = agent_block.cid().clone();
        stats.total_blocks += 1;
        stats.uncompressed_size += agent_block.data().len() as u64;

        // Create CAR writer with agent as root
        let header = CarHeader::new_v1(vec![agent_cid.clone()]);
        let mut car_writer = CarWriter::new(header, &mut output);

        // Write agent block
        car_writer
            .write(agent_cid.clone(), agent_block.data())
            .await?;

        // Export memories
        let memories_cid = if !agent.memories.is_empty() {
            let memory_cids = self
                .export_memories(&agent, &mut car_writer, &mut stats)
                .await?;

            // Create memories collection block
            let memories_ipld = Ipld::List(memory_cids.into_iter().map(Ipld::Link).collect());

            let memories_block = Block::<libipld::DefaultParams>::encode(
                libipld::cbor::DagCborCodec,
                Code::Blake3_256,
                &memories_ipld,
            )?;
            let memories_cid = memories_block.cid().clone();

            car_writer
                .write(memories_cid.clone(), memories_block.data())
                .await?;
            stats.total_blocks += 1;
            stats.uncompressed_size += memories_block.data().len() as u64;

            Some(memories_cid)
        } else {
            None
        };

        // Export messages if requested
        let messages_cid = if options.include_messages {
            self.export_messages(agent_id.clone(), &mut car_writer, &mut stats, &options)
                .await?
        } else {
            None
        };

        // Flush the writer
        car_writer.finish().await?;

        // Create and return manifest
        let manifest = ExportManifest {
            version: EXPORT_VERSION,
            exported_at: start_time,
            agent_id,
            stats,
            agent_cid,
            memories_cid,
            messages_cid,
            compression: None,
        };

        Ok(manifest)
    }

    /// Export memory blocks
    async fn export_memories(
        &self,
        agent: &AgentRecord,
        writer: &mut CarWriter<impl Write>,
        stats: &mut ExportStats,
    ) -> Result<Vec<Cid>> {
        let mut memory_cids = Vec::new();

        for (memory, _relation) in &agent.memories {
            // Convert memory to IPLD
            let memory_ipld =
                serde_json::to_value(memory).map_err(|e| CoreError::SerializationError {
                    type_name: "MemoryBlock".to_string(),
                    reason: e.to_string(),
                })?;
            let memory_ipld = serde_json::from_value::<Ipld>(memory_ipld).map_err(|e| {
                CoreError::SerializationError {
                    type_name: "IPLD".to_string(),
                    reason: e.to_string(),
                }
            })?;

            // Create memory block
            let memory_block = Block::<libipld::DefaultParams>::encode(
                libipld::cbor::DagCborCodec,
                Code::Blake3_256,
                &memory_ipld,
            )?;
            let memory_cid = memory_block.cid().clone();

            // Write to CAR
            writer
                .write(memory_cid.clone(), memory_block.data())
                .await?;
            memory_cids.push(memory_cid);

            stats.memory_count += 1;
            stats.total_blocks += 1;
            stats.uncompressed_size += memory_block.data().len() as u64;
        }

        Ok(memory_cids)
    }

    /// Export messages in chunks
    async fn export_messages(
        &self,
        agent_id: AgentId,
        writer: &mut CarWriter<impl Write>,
        stats: &mut ExportStats,
        options: &ExportOptions,
    ) -> Result<Option<Cid>> {
        // Query messages with optional time filter
        let query = if let Some(since) = options.messages_since {
            format!(
                "SELECT * FROM message WHERE agent_id = $agent_id AND created_at >= $since ORDER BY position"
            )
        } else {
            format!("SELECT * FROM message WHERE agent_id = $agent_id ORDER BY position")
        };

        let mut response = if let Some(since) = options.messages_since {
            self.db
                .query(&query)
                .bind(("agent_id", agent_id.to_string()))
                .bind(("since", since))
                .await?
        } else {
            self.db
                .query(&query)
                .bind(("agent_id", agent_id.to_string()))
                .await?
        };

        let messages: Vec<Message> = response.take(0)?;

        if messages.is_empty() {
            return Ok(None);
        }

        // Process messages in chunks
        let mut first_chunk_cid = None;
        let mut prev_chunk_cid: Option<Cid> = None;

        for (chunk_id, chunk_messages) in messages.chunks(options.chunk_size).enumerate() {
            let chunk_cid = self
                .write_message_chunk(
                    chunk_id as u32,
                    chunk_messages,
                    prev_chunk_cid,
                    writer,
                    stats,
                )
                .await?;

            if first_chunk_cid.is_none() {
                first_chunk_cid = Some(chunk_cid.clone());
            }

            prev_chunk_cid = Some(chunk_cid);
            stats.chunk_count += 1;
        }

        Ok(first_chunk_cid)
    }

    /// Write a single message chunk
    async fn write_message_chunk(
        &self,
        chunk_id: u32,
        messages: &[Message],
        next_chunk: Option<Cid>,
        writer: &mut CarWriter<impl Write>,
        stats: &mut ExportStats,
    ) -> Result<Cid> {
        let mut message_blocks = Vec::new();

        for msg in messages {
            // Simplified message for export
            let msg_block = MessageBlock {
                id: msg.id.clone(),
                content: serde_json::from_value(serde_json::to_value(&msg.content)?)?,
                metadata: serde_json::from_value(serde_json::to_value(&msg.metadata)?)?,
            };
            message_blocks.push(msg_block);
            stats.message_count += 1;
        }

        let chunk = MessageChunk {
            chunk_id,
            start_position: messages.first().unwrap().position.clone(),
            end_position: messages.last().unwrap().position.clone(),
            messages: message_blocks,
            next_chunk,
        };

        // Convert chunk to IPLD
        let chunk_ipld = serde_json::to_value(&chunk)?;
        let chunk_ipld = serde_json::from_value::<Ipld>(chunk_ipld)?;

        // Create chunk block
        let chunk_block = Block::<libipld::DefaultParams>::encode(
            libipld::cbor::DagCborCodec,
            Code::Blake3_256,
            &chunk_ipld,
        )?;
        let chunk_cid = chunk_block.cid().clone();

        // Write to CAR
        writer.write(chunk_cid.clone(), chunk_block.data()).await?;
        stats.total_blocks += 1;
        stats.uncompressed_size += chunk_block.data().len() as u64;

        Ok(chunk_cid)
    }
}
