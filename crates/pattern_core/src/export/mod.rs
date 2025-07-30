//! Agent export/import functionality using DAG-CBOR CAR archives
//!
//! This module provides tools for exporting agents to portable CAR files
//! and importing them back, preserving all relationships and data.

mod exporter;
mod importer;
mod types;

pub use exporter::{AgentExporter, ExportOptions};
pub use importer::{AgentImporter, ImportOptions, ImportResult};
pub use types::{
    AgentExport, ConstellationExport, ExportManifest, ExportStats, ExportType, GroupExport,
    MemoryChunk, MessageChunk,
};

/// Current export format version
pub const EXPORT_VERSION: u32 = 1;

/// Default chunk size for message batching
pub const DEFAULT_CHUNK_SIZE: usize = 1000;
