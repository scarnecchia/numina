//! Agent exporter implementation

use chrono::Utc;
use cid::Cid;
use iroh_car::{CarHeader, CarWriter};
use multihash_codetable::Code;
use multihash_codetable::MultihashDigest;
use serde_ipld_dagcbor::to_vec as encode_dag_cbor;
use surrealdb::Surreal;
use tokio::io::AsyncWrite;

use crate::{
    AgentId, CoreError, Result,
    agent::AgentRecord,
    coordination::groups::{AgentGroup, Constellation, ConstellationMembership, GroupMembership},
    db::entity::DbEntity,
    export::{
        DEFAULT_CHUNK_SIZE, EXPORT_VERSION,
        types::{
            ConstellationExport, ExportManifest, ExportStats, GroupExport, MemoryChunk,
            MessageChunk,
        },
    },
    id::{ConstellationId, GroupId},
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

    /// Helper to create a CID from serialized data
    fn create_cid(data: &[u8]) -> Result<Cid> {
        // Use Blake3-256 hash and DAG-CBOR codec
        const DAG_CBOR_CODEC: u64 = 0x71;
        let hash = Code::Blake3_256.digest(data);
        Ok(Cid::new_v1(DAG_CBOR_CODEC, hash))
    }

    /// Export an agent to a CAR file
    pub async fn export_to_car(
        &self,
        agent_id: AgentId,
        mut output: impl AsyncWrite + Unpin + Send,
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
        let agent = AgentRecord::load_with_relations(&self.db, &agent_id)
            .await?
            .ok_or_else(|| CoreError::agent_not_found(agent_id.to_string()))?;

        // Serialize agent to DAG-CBOR
        let agent_data = encode_dag_cbor(&agent).map_err(|e| CoreError::DagCborEncodingError {
            data_type: "AgentRecord".to_string(),
            cause: e,
        })?;

        // Create CID for agent
        let agent_cid = Self::create_cid(&agent_data)?;
        stats.total_blocks += 1;
        stats.uncompressed_size += agent_data.len() as u64;

        // Create CAR writer with agent as root
        let header = CarHeader::new_v1(vec![agent_cid]);
        let mut car_writer = CarWriter::new(header, &mut output);

        // Write agent block
        car_writer
            .write(agent_cid, &agent_data)
            .await
            .map_err(|e| CoreError::CarError {
                operation: "writing agent to CAR".to_string(),
                cause: e,
            })?;

        // Export memories
        let memories_cid = self
            .export_memories(&agent, &mut car_writer, &mut stats)
            .await?;

        // Export messages if requested
        let messages_cid = if options.include_messages {
            self.export_messages(&agent, &mut car_writer, &mut stats, &options)
                .await?
        } else {
            None
        };

        // Flush the writer
        car_writer.finish().await.map_err(|e| CoreError::CarError {
            operation: "finishing CAR write".to_string(),
            cause: e,
        })?;

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

    /// Export memory blocks with their relations as a single chunk
    async fn export_memories(
        &self,
        agent: &AgentRecord,
        writer: &mut CarWriter<impl AsyncWrite + Unpin + Send>,
        stats: &mut ExportStats,
    ) -> Result<Option<Cid>> {
        use crate::export::types::MemoryChunk;

        if agent.memories.is_empty() {
            return Ok(None);
        }

        // Create a single memory chunk with all memories and their relations
        let memory_chunk = MemoryChunk {
            chunk_id: 0,
            memories: agent.memories.clone(),
            next_chunk: None,
        };

        // Serialize the chunk to DAG-CBOR
        let chunk_data =
            encode_dag_cbor(&memory_chunk).map_err(|e| CoreError::DagCborEncodingError {
                data_type: "MemoryChunk".to_string(),
                cause: e,
            })?;

        // Create CID
        let chunk_cid = Self::create_cid(&chunk_data)?;

        // Write to CAR
        writer
            .write(chunk_cid, &chunk_data)
            .await
            .map_err(|e| CoreError::CarError {
                operation: "writing memory chunk to CAR".to_string(),
                cause: e,
            })?;

        stats.memory_count = agent.memories.len() as u64;
        stats.total_blocks += 1;
        stats.uncompressed_size += chunk_data.len() as u64;

        Ok(Some(chunk_cid))
    }

    /// Export messages in chunks using the messages from the agent record
    async fn export_messages(
        &self,
        agent: &AgentRecord,
        writer: &mut CarWriter<impl AsyncWrite + Unpin + Send>,
        stats: &mut ExportStats,
        options: &ExportOptions,
    ) -> Result<Option<Cid>> {
        // Filter messages based on time if needed
        let messages_with_positions: Vec<_> = if let Some(since) = options.messages_since {
            agent
                .messages
                .iter()
                .filter(|(msg, _)| msg.created_at >= since)
                .collect()
        } else {
            agent.messages.iter().collect()
        };

        if messages_with_positions.is_empty() {
            return Ok(None);
        }

        // Process messages in chunks
        let mut first_chunk_cid = None;
        let mut prev_chunk_cid: Option<Cid> = None;

        for (chunk_id, chunk) in messages_with_positions
            .chunks(options.chunk_size)
            .enumerate()
        {
            let chunk_cid = self
                .write_message_chunk_with_positions(
                    chunk_id as u32,
                    chunk,
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

    /// Write a single message chunk with positions from relations
    async fn write_message_chunk_with_positions(
        &self,
        chunk_id: u32,
        messages: &[&(Message, crate::message::AgentMessageRelation)],
        next_chunk: Option<Cid>,
        writer: &mut CarWriter<impl AsyncWrite + Unpin + Send>,
        stats: &mut ExportStats,
    ) -> Result<Cid> {
        // Clone the messages with their relations
        let messages_with_relations: Vec<(Message, crate::message::AgentMessageRelation)> =
            messages
                .iter()
                .map(|&(msg, rel)| (msg.clone(), rel.clone()))
                .collect();

        // Use positions from the relations
        let chunk = MessageChunk {
            chunk_id,
            start_position: messages.first().unwrap().1.position.clone(),
            end_position: messages.last().unwrap().1.position.clone(),
            messages: messages_with_relations,
            next_chunk,
        };

        stats.message_count += messages.len() as u64;

        // Serialize chunk to DAG-CBOR
        let chunk_data = encode_dag_cbor(&chunk).map_err(|e| CoreError::DagCborEncodingError {
            data_type: "MessageChunk".to_string(),
            cause: e,
        })?;

        // Create CID
        let chunk_cid = Self::create_cid(&chunk_data)?;

        // Write to CAR
        writer
            .write(chunk_cid, &chunk_data)
            .await
            .map_err(|e| CoreError::CarError {
                operation: "writing message chunk to CAR".to_string(),
                cause: e,
            })?;
        stats.total_blocks += 1;
        stats.uncompressed_size += chunk_data.len() as u64;

        Ok(chunk_cid)
    }

    /// Export an agent to blocks without writing to CAR file
    async fn export_agent_to_blocks(
        &self,
        agent: &AgentRecord,
        options: &ExportOptions,
    ) -> Result<(Cid, Vec<(Cid, Vec<u8>)>, ExportStats)> {
        let mut blocks = Vec::new();
        let mut stats = ExportStats {
            memory_count: 0,
            message_count: 0,
            chunk_count: 0,
            total_blocks: 0,
            uncompressed_size: 0,
            compressed_size: None,
        };

        // Serialize agent to DAG-CBOR
        let agent_data = encode_dag_cbor(agent).map_err(|e| CoreError::DagCborEncodingError {
            data_type: "AgentRecord".to_string(),
            cause: e,
        })?;

        // Create CID for agent
        let agent_cid = Self::create_cid(&agent_data)?;
        stats.total_blocks += 1;
        stats.uncompressed_size += agent_data.len() as u64;

        blocks.push((agent_cid, agent_data));

        // Export memories if any
        if !agent.memories.is_empty() {
            let memory_chunk = MemoryChunk {
                chunk_id: 0,
                memories: agent.memories.clone(),
                next_chunk: None,
            };

            let chunk_data =
                encode_dag_cbor(&memory_chunk).map_err(|e| CoreError::DagCborEncodingError {
                    data_type: "MemoryChunk".to_string(),
                    cause: e,
                })?;

            let chunk_cid = Self::create_cid(&chunk_data)?;
            stats.memory_count = agent.memories.len() as u64;
            stats.total_blocks += 1;
            stats.uncompressed_size += chunk_data.len() as u64;

            blocks.push((chunk_cid, chunk_data));
        }

        // Export messages if requested
        if options.include_messages {
            let messages_with_positions: Vec<_> = if let Some(since) = options.messages_since {
                agent
                    .messages
                    .iter()
                    .filter(|(msg, _)| msg.created_at >= since)
                    .collect()
            } else {
                agent.messages.iter().collect()
            };

            if !messages_with_positions.is_empty() {
                // Process messages in chunks
                for (chunk_id, chunk) in messages_with_positions
                    .chunks(options.chunk_size)
                    .enumerate()
                {
                    let messages_with_relations: Vec<(
                        Message,
                        crate::message::AgentMessageRelation,
                    )> = chunk
                        .iter()
                        .map(|&(msg, rel)| (msg.clone(), rel.clone()))
                        .collect();

                    let message_chunk = MessageChunk {
                        chunk_id: chunk_id as u32,
                        start_position: chunk.first().unwrap().1.position.clone(),
                        end_position: chunk.last().unwrap().1.position.clone(),
                        messages: messages_with_relations,
                        next_chunk: None, // We'll fix this up later if needed
                    };

                    stats.message_count += chunk.len() as u64;

                    let chunk_data = encode_dag_cbor(&message_chunk).map_err(|e| {
                        CoreError::DagCborEncodingError {
                            data_type: "MessageChunk".to_string(),
                            cause: e,
                        }
                    })?;

                    let chunk_cid = Self::create_cid(&chunk_data)?;
                    blocks.push((chunk_cid, chunk_data));

                    stats.chunk_count += 1;
                    stats.total_blocks += 1;
                    stats.uncompressed_size += blocks.last().unwrap().1.len() as u64;
                }
            }
        }

        Ok((agent_cid, blocks, stats))
    }

    /// Export a group with all its member agents to a CAR file
    pub async fn export_group_to_car(
        &self,
        group_id: GroupId,
        mut output: impl AsyncWrite + Unpin + Send,
        options: ExportOptions,
    ) -> Result<Cid> {
        // Load the group with all members
        let group = self.load_group_with_members(&group_id).await?;

        // Export all member agents first
        let mut agent_export_cids = Vec::new();
        let mut all_blocks = Vec::new();

        for (agent, _membership) in &group.members {
            let (agent_cid, agent_blocks, _stats) =
                self.export_agent_to_blocks(agent, &options).await?;

            agent_export_cids.push((agent.id.clone(), agent_cid));
            all_blocks.extend(agent_blocks);
        }

        // Create the group export
        let group_export = self.export_group(&group, &agent_export_cids).await?;

        // Serialize group export
        let group_data =
            encode_dag_cbor(&group_export).map_err(|e| CoreError::DagCborEncodingError {
                data_type: "GroupExport".to_string(),
                cause: e,
            })?;
        let group_cid = Self::create_cid(&group_data)?;

        // Create CAR file with group as root
        let header = CarHeader::new_v1(vec![group_cid]);
        let mut car_writer = CarWriter::new(header, &mut output);

        // Write group block first
        car_writer
            .write(group_cid, &group_data)
            .await
            .map_err(|e| CoreError::CarError {
                operation: "writing group to CAR".to_string(),
                cause: e,
            })?;

        // Write all agent blocks
        for (cid, data) in all_blocks {
            car_writer
                .write(cid, &data)
                .await
                .map_err(|e| CoreError::CarError {
                    operation: "writing agent block to CAR".to_string(),
                    cause: e,
                })?;
        }

        car_writer.finish().await.map_err(|e| CoreError::CarError {
            operation: "finishing group CAR write".to_string(),
            cause: e,
        })?;

        Ok(group_cid)
    }

    /// Export a constellation with all its agents and groups
    pub async fn export_constellation_to_car(
        &self,
        constellation_id: ConstellationId,
        mut output: impl AsyncWrite + Unpin + Send,
        options: ExportOptions,
    ) -> Result<Cid> {
        // Load the constellation - load_with_relations might not work properly
        let constellation = self
            .load_constellation_with_members(&constellation_id)
            .await?;

        // We'll use the constellation as the root of our CAR file
        let mut agent_export_cids = Vec::new();
        let mut all_blocks = Vec::new();
        let mut total_stats = ExportStats {
            memory_count: 0,
            message_count: 0,
            chunk_count: 0,
            total_blocks: 0,
            uncompressed_size: 0,
            compressed_size: None,
        };

        // First, export all agents in the constellation and collect their blocks
        for (agent, _membership) in &constellation.agents {
            let (agent_cid, agent_blocks, stats) =
                self.export_agent_to_blocks(agent, &options).await?;

            agent_export_cids.push((agent.id.clone(), agent_cid));
            all_blocks.extend(agent_blocks);

            // Accumulate stats
            total_stats.memory_count += stats.memory_count;
            total_stats.message_count += stats.message_count;
            total_stats.chunk_count += stats.chunk_count;
            total_stats.total_blocks += stats.total_blocks;
            total_stats.uncompressed_size += stats.uncompressed_size;
        }

        // Export all groups in the constellation
        let mut group_exports = Vec::new();
        for group_id in &constellation.groups {
            let group = self.load_group_with_members(group_id).await?;

            let group_export = self.export_group(&group, &agent_export_cids).await?;

            // Serialize group export
            let group_data =
                encode_dag_cbor(&group_export).map_err(|e| CoreError::DagCborEncodingError {
                    data_type: "GroupExport".to_string(),
                    cause: e,
                })?;
            let group_cid = Self::create_cid(&group_data)?;

            all_blocks.push((group_cid, group_data));
            total_stats.total_blocks += 1;

            group_exports.push(group_export);
        }

        // Create constellation export
        let constellation_export = ConstellationExport {
            constellation: constellation.clone(),
            groups: group_exports,
            agent_export_cids,
        };

        // Serialize constellation export
        let constellation_data = encode_dag_cbor(&constellation_export).map_err(|e| {
            CoreError::DagCborEncodingError {
                data_type: "ConstellationExport".to_string(),
                cause: e,
            }
        })?;
        let constellation_cid = Self::create_cid(&constellation_data)?;

        // Create CAR file with constellation as root
        let header = CarHeader::new_v1(vec![constellation_cid]);
        let mut car_writer = CarWriter::new(header, &mut output);

        // Write constellation block first
        car_writer
            .write(constellation_cid, &constellation_data)
            .await
            .map_err(|e| CoreError::CarError {
                operation: "writing constellation to CAR".to_string(),
                cause: e,
            })?;

        // Write all collected blocks
        for (cid, data) in all_blocks {
            car_writer
                .write(cid, &data)
                .await
                .map_err(|e| CoreError::CarError {
                    operation: "writing block to CAR".to_string(),
                    cause: e,
                })?;
        }

        car_writer.finish().await.map_err(|e| CoreError::CarError {
            operation: "finishing constellation CAR write".to_string(),
            cause: e,
        })?;

        Ok(constellation_cid)
    }

    /// Export a group with references to its member agents
    async fn export_group(
        &self,
        group: &AgentGroup,
        agent_cids: &[(AgentId, Cid)],
    ) -> Result<GroupExport> {
        // Map member agent IDs to their export CIDs
        let member_agent_cids: Vec<(AgentId, Cid)> = group
            .members
            .iter()
            .filter_map(|(agent, _membership)| {
                agent_cids.iter().find(|(id, _)| id == &agent.id).cloned()
            })
            .collect();

        Ok(GroupExport {
            group: group.clone(),
            member_agent_cids,
        })
    }

    /// Load constellation with all members and relations
    async fn load_constellation_with_members(
        &self,
        constellation_id: &ConstellationId,
    ) -> Result<Constellation> {
        use crate::db::ops::get_entity;

        // First get the constellation entity
        let constellation = get_entity::<Constellation, _>(&self.db, constellation_id)
            .await?
            .ok_or_else(|| CoreError::agent_not_found(constellation_id.to_string()))?;

        // Load the agents via constellation_agents edge
        let agent_query = r#"
            SELECT * FROM constellation_agents
            WHERE in = $constellation_id
            ORDER BY joined_at ASC
        "#;

        let mut result = self
            .db
            .query(agent_query)
            .bind((
                "constellation_id",
                surrealdb::RecordId::from(constellation_id),
            ))
            .await
            .map_err(|e| CoreError::DatabaseQueryFailed {
                query: agent_query.to_string(),
                table: "constellation_agents".to_string(),
                cause: e.into(),
            })?;

        let membership_db_models: Vec<<ConstellationMembership as DbEntity>::DbModel> =
            result.take(0).map_err(|e| CoreError::DatabaseQueryFailed {
                query: agent_query.to_string(),
                table: "constellation_agents".to_string(),
                cause: e.into(),
            })?;

        let memberships: Vec<ConstellationMembership> = membership_db_models
            .into_iter()
            .map(|db_model| {
                ConstellationMembership::from_db_model(db_model)
                    .map_err(|e| CoreError::from(crate::db::DatabaseError::from(e)))
            })
            .collect::<Result<Vec<_>>>()?;

        // Load the agents for each membership
        let mut agents = Vec::new();
        for membership in memberships {
            if let Some(agent) = AgentRecord::load_with_relations(&self.db, &membership.out_id)
                .await
                .map_err(|e| CoreError::from(e))?
            {
                agents.push((agent, membership));
            }
        }

        // Load the group IDs via composed_of relation
        let groups_query = r#"
            SELECT out FROM $constellation_id->composed_of
        "#;

        let mut result = self
            .db
            .query(groups_query)
            .bind((
                "constellation_id",
                surrealdb::RecordId::from(constellation_id),
            ))
            .await
            .map_err(|e| CoreError::DatabaseQueryFailed {
                query: groups_query.to_string(),
                table: "composed_of".to_string(),
                cause: e.into(),
            })?;

        let group_record_ids: Vec<surrealdb::RecordId> =
            result
                .take("out")
                .map_err(|e| CoreError::DatabaseQueryFailed {
                    query: groups_query.to_string(),
                    table: "composed_of".to_string(),
                    cause: e.into(),
                })?;

        let group_ids: Vec<GroupId> = group_record_ids
            .into_iter()
            .map(|rid| GroupId::from_record(rid))
            .collect();

        Ok(Constellation {
            id: constellation.id,
            owner_id: constellation.owner_id,
            name: constellation.name,
            description: constellation.description,
            created_at: constellation.created_at,
            updated_at: constellation.updated_at,
            is_active: constellation.is_active,
            agents,
            groups: group_ids,
        })
    }

    /// Load group with all members
    async fn load_group_with_members(&self, group_id: &GroupId) -> Result<AgentGroup> {
        use crate::db::ops::get_entity;

        // Get the base group
        let mut group = get_entity::<AgentGroup, _>(&self.db, group_id)
            .await?
            .ok_or_else(|| CoreError::agent_not_found(group_id.to_string()))?;

        // Load members via group_members edge
        let query = r#"
            SELECT * FROM group_members
            WHERE out = $group_id
            ORDER BY joined_at ASC
        "#;

        let mut result = self
            .db
            .query(query)
            .bind(("group_id", surrealdb::RecordId::from(group_id)))
            .await
            .map_err(|e| CoreError::DatabaseQueryFailed {
                query: query.to_string(),
                table: "group_members".to_string(),
                cause: e.into(),
            })?;

        let membership_db_models: Vec<<GroupMembership as DbEntity>::DbModel> =
            result.take(0).map_err(|e| CoreError::DatabaseQueryFailed {
                query: query.to_string(),
                table: "group_members".to_string(),
                cause: e.into(),
            })?;

        let memberships: Vec<GroupMembership> = membership_db_models
            .into_iter()
            .map(|db_model| {
                GroupMembership::from_db_model(db_model)
                    .map_err(|e| CoreError::from(crate::db::DatabaseError::from(e)))
            })
            .collect::<Result<Vec<_>>>()?;

        // Load the agents for each membership
        let mut members = Vec::new();
        for membership in memberships {
            if let Some(agent) = AgentRecord::load_with_relations(&self.db, &membership.in_id)
                .await
                .map_err(|e| CoreError::from(e))?
            {
                members.push((agent, membership));
            }
        }

        group.members = members;
        Ok(group)
    }
}
