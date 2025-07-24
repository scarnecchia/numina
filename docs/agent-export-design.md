# Agent Export/Import Design

## Overview

Pattern agents can be exported to and imported from CAR (Content Addressable aRchive) files using DAG-CBOR encoding. This provides an efficient, portable format for backing up, sharing, and migrating agents between systems.

## Why DAG-CBOR CAR?

- **Efficient**: Binary format, much smaller than JSON
- **Graph-native**: Handles our agent→memory→message relationships naturally
- **Content-addressed**: Each block has a CID (Content Identifier) for integrity
- **Streaming**: CAR files can be processed incrementally
- **AT Protocol compatible**: Same format used by Bluesky/ATProto ecosystem

## Archive Structure

```
AgentArchive (root)
├── manifest (metadata about the export)
├── agent (AgentRecord)
├── memories/ (MemoryBlock collection)
│   ├── memory_1
│   ├── memory_2
│   └── ...
└── messages/ (Message collection, chunked)
    ├── chunk_1 (1000 messages)
    ├── chunk_2 (1000 messages)
    └── ...
```

## Data Model

### Manifest Block
```rust
struct ExportManifest {
    version: u32,                    // Archive format version
    exported_at: DateTime<Utc>,      // When exported
    agent_id: AgentId,               // Agent being exported
    stats: ExportStats,              // Counts and sizes
    chunking: ChunkingStrategy,      // How messages are chunked
}

struct ExportStats {
    memory_count: u64,
    message_count: u64,
    total_blocks: u64,
    compressed_size: Option<u64>,
}
```

### Agent Block
The existing `AgentRecord` struct, but with CID links instead of IDs:
- `memories` → CIDs of memory blocks
- `messages` → CIDs of message chunks

### Memory Blocks
Individual `MemoryBlock` structs with their relationships

### Message Chunks
Messages grouped into chunks for efficient streaming:
```rust
struct MessageChunk {
    chunk_id: u32,
    start_position: String,  // Snowflake ID
    end_position: String,    // Snowflake ID
    messages: Vec<Message>,
    next_chunk: Option<Cid>, // Link to next chunk
}
```

## Implementation Plan

### 1. Dependencies
Add to `pattern-core/Cargo.toml`:
```toml
# DAG-CBOR encoding
ipld = { version = "0.16", features = ["dag-cbor"] }
libipld = "0.16"

# CAR file support
iroh-car = "0.4"  # or ipfs-car

# CID handling
cid = "0.11"
multihash = "0.19"
```

### 2. Export Module (`pattern-core/src/export/`)

```rust
pub struct AgentExporter {
    db: Surreal<C>,
    chunk_size: usize,
}

impl AgentExporter {
    /// Export an agent to a CAR file
    pub async fn export_to_car(
        &self,
        agent_id: AgentId,
        output: impl Write,
    ) -> Result<ExportManifest> {
        // 1. Load agent record
        // 2. Stream memory blocks
        // 3. Stream message chunks
        // 4. Build manifest
        // 5. Write CAR file
    }
}
```

### 3. Import Module

```rust
pub struct AgentImporter {
    db: Surreal<C>,
}

impl AgentImporter {
    /// Import an agent from a CAR file
    pub async fn import_from_car(
        &self,
        input: impl Read,
        options: ImportOptions,
    ) -> Result<AgentRecord> {
        // 1. Read and validate manifest
        // 2. Import agent record
        // 3. Stream memory blocks
        // 4. Stream message chunks
        // 5. Rebuild relationships
    }
}
```

### 4. CLI Commands

```bash
# Export agent to CAR file
pattern-cli agent export <name> -o agent.car

# Import agent from CAR file
pattern-cli agent import agent.car --name "NewName"

# Inspect CAR file without importing
pattern-cli agent inspect agent.car
```

## Advanced Features

### Compression
- Optional zstd compression for the CAR file
- Compress individual blocks or entire archive

### Selective Export
```bash
# Export without message history
pattern-cli agent export <name> --no-messages

# Export only recent messages
pattern-cli agent export <name> --messages-since "7 days ago"
```

### Streaming Support
- Export/import large agents without loading everything into memory
- Progress reporting for long operations

### Migration Support
- Version detection and upgrade paths
- Schema evolution handling

## Security Considerations

1. **Encryption**: Option to encrypt CAR files with age or similar
2. **Signing**: Include signature block for authenticity
3. **Redaction**: Strip sensitive data during export
4. **Access Control**: Verify permissions during import

## Example Usage

```rust
// Export
let exporter = AgentExporter::new(db.clone());
let mut file = File::create("my-agent.car")?;
let manifest = exporter.export_to_car(agent_id, &mut file).await?;
println!("Exported {} memories, {} messages", 
    manifest.stats.memory_count, 
    manifest.stats.message_count
);

// Import
let importer = AgentImporter::new(db.clone());
let file = File::open("my-agent.car")?;
let agent = importer.import_from_car(file, ImportOptions {
    rename_to: Some("ImportedAgent".to_string()),
    merge_existing: false,
}).await?;
```

## Benefits Over JSON

1. **Size**: ~70% smaller than equivalent JSON
2. **Streaming**: Process multi-GB exports without loading into memory  
3. **Integrity**: Content addressing ensures data hasn't been tampered with
4. **Efficiency**: Binary format parses much faster
5. **Compatibility**: Can be used with IPFS tooling

## Future Extensions

1. **Incremental Backups**: Export only changes since last backup
2. **Multi-agent Archives**: Export entire constellations
3. **IPFS Integration**: Store archives on IPFS
4. **ATProto Sync**: Share agents via AT Protocol repositories